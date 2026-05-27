use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;
use tracing::{error, info, warn};

use jupiteros_shared::store::MessageStore;

use crate::config::{AccountConfig, Config};
use crate::email_indexer::{EmailIndexer, IndexerState};
use crate::mail_backend::{build_backend, MailBackend, NewMailCallback};
use crate::oauth::AccessTokenCache;

const IDLE_TIMEOUT_SECS: u64 = 1680; // 28 min (RFC 2177)
const RECONNECT_DELAY_SECS: u64 = 30;
const ATTACH_RECENT_COUNT: usize = 50;

// Circuit breaker: stop hammering the IMAP server when auth keeps failing.
// Prevents IP lockout cascades when a password is missing/wrong/expired.
const AUTH_FAIL_THRESHOLD: u32 = 3;
const AUTH_FAIL_COOLDOWN_SECS: u64 = 3600; // 1 hour

/// Returns true if an error message looks like an IMAP AUTHENTICATIONFAILED.
fn is_auth_failure(err: &(impl std::fmt::Display + ?Sized)) -> bool {
    let s = err.to_string().to_uppercase();
    s.contains("AUTHENTICATIONFAILED") || s.contains("AUTHENTICATION FAILED")
}

/// Per-account auth-failure tracker. Once `AUTH_FAIL_THRESHOLD` consecutive
/// auth failures are recorded, the breaker trips for `AUTH_FAIL_COOLDOWN_SECS`.
#[derive(Default)]
struct AuthBreaker {
    consecutive_fails: u32,
    tripped_until: Option<Instant>,
}

impl AuthBreaker {
    fn allow_attempt(&mut self) -> bool {
        if let Some(until) = self.tripped_until {
            if Instant::now() < until {
                return false;
            }
            // Cooldown expired: half-open the breaker, allow one probe.
            self.tripped_until = None;
            self.consecutive_fails = 0;
        }
        true
    }

    fn record_success(&mut self) {
        self.consecutive_fails = 0;
        self.tripped_until = None;
    }

    fn record_auth_failure(&mut self) -> bool {
        self.consecutive_fails += 1;
        if self.consecutive_fails >= AUTH_FAIL_THRESHOLD {
            self.tripped_until =
                Some(Instant::now() + Duration::from_secs(AUTH_FAIL_COOLDOWN_SECS));
            return true; // breaker just tripped
        }
        false
    }

    /// Non-auth errors (network glitches) don't count toward the breaker.
    fn record_other_error(&self) {}
}

/// Folders to skip during auto-discovery (trash, spam, drafts).
const SKIP_FOLDER_PATTERNS: &[&str] = &[
    "trash", "cestino", "junk", "spam", "drafts", "bozze",
];

/// Email daemon: runs bulk indexing + IMAP IDLE watchers + fallback polling
/// for ALL configured accounts.
pub struct EmailDaemon {
    config: Config,
    store: Arc<MessageStore>,
    index_lock: Arc<Mutex<()>>,
    oauth_cache: AccessTokenCache,
}

impl EmailDaemon {
    pub fn new(config: Config, store: Arc<MessageStore>) -> Self {
        Self {
            config,
            store,
            index_lock: Arc::new(Mutex::new(())),
            oauth_cache: AccessTokenCache::new(),
        }
    }

    /// Build the right MailBackend for a given account (IMAP or Gmail API).
    fn client_for(&self, account: &AccountConfig) -> Box<dyn MailBackend> {
        build_backend(
            account.clone(),
            self.config.signature_dir.clone(),
            self.oauth_cache.clone(),
        )
    }

    /// Discover all indexable IMAP folders for a single account.
    /// If the account has an explicit folders list (different from the default
    /// `INBOX, INBOX.Sent`), use those. Otherwise auto-discover.
    async fn discover_folders(&self, account: &AccountConfig) -> Vec<String> {
        let default_folders = vec!["INBOX".to_string(), "INBOX.Sent".to_string()];
        if !account.folders.is_empty() && account.folders != default_folders {
            info!(
                "[{}] Using configured folders: {:?}",
                account.name, account.folders
            );
            return account.folders.clone();
        }

        info!("[{}] Auto-discovering IMAP folders...", account.name);
        let client = self.client_for(account);
        match client.list_folders().await {
            Ok(all_folders) => {
                info!(
                    "[{}] Found {} IMAP folders total",
                    account.name,
                    all_folders.len()
                );

                let folders: Vec<String> = all_folders
                    .into_iter()
                    .filter(|f| {
                        let lower = f.to_lowercase();
                        !SKIP_FOLDER_PATTERNS.iter().any(|p| lower.contains(p))
                    })
                    .collect();

                info!(
                    "[{}] Indexing {} folders (excluded trash/spam/drafts): {:?}",
                    account.name,
                    folders.len(),
                    folders
                );
                folders
            }
            Err(e) => {
                warn!(
                    "[{}] Failed to list IMAP folders: {e}. Falling back to config: {:?}",
                    account.name, account.folders
                );
                account.folders.clone()
            }
        }
    }

    /// Run the full daemon lifecycle:
    /// 1. For each account: discover folders, bulk index, attachment extraction
    /// 2. For each (account, folder): spawn IDLE watcher
    /// 3. Spawn one fallback poller per account
    pub async fn run(&self) {
        info!(
            "Starting email daemon — {} account(s)",
            self.config.accounts.len()
        );

        if self.config.accounts.is_empty() {
            error!("No email accounts configured!");
            return;
        }

        // Per-account folder map + per-account auth circuit breaker.
        // The breaker is created here so it covers Phase 1 (bulk), Phase 2
        // (attachments), AND Phase 3 (IDLE+poller) — preventing the startup
        // burst of failures from rebuilding an IP lockout.
        let mut account_folders: Vec<(AccountConfig, Vec<String>, Arc<Mutex<AuthBreaker>>)> =
            Vec::with_capacity(self.config.accounts.len());

        for account in &self.config.accounts {
            let folders = self.discover_folders(account).await;
            if folders.is_empty() {
                warn!("[{}] No folders to index, skipping", account.name);
                continue;
            }

            // Sanity check on indexer state (data loss detection)
            self.check_state_integrity(account).await;

            let breaker = Arc::new(Mutex::new(AuthBreaker::default()));
            account_folders.push((account.clone(), folders, breaker));
        }

        // Phase 1: Bulk indexing
        for (account, folders, breaker) in &account_folders {
            if !breaker.lock().await.allow_attempt() {
                warn!(
                    "[{}] Phase 1 SKIPPED — auth circuit breaker is tripped.",
                    account.name
                );
                continue;
            }
            info!(
                "[{}] Phase 1: Bulk indexing {} folders...",
                account.name,
                folders.len()
            );
            for folder in folders {
                if !breaker.lock().await.allow_attempt() {
                    warn!(
                        "[{}] Phase 1 aborted on {folder} — breaker tripped.",
                        account.name
                    );
                    break;
                }
                let client = self.client_for(account);
                let indexer =
                    EmailIndexer::new(self.store.clone(), client, &self.config.data_dir);

                match indexer.index_folder(folder, 0, false, false).await {
                    Ok(result) => {
                        breaker.lock().await.record_success();
                        info!(
                            "[{}] Bulk index {folder}: indexed={}, skipped={}, errors={}",
                            account.name, result.indexed, result.skipped, result.errors
                        );
                    }
                    Err(e) => {
                        if is_auth_failure(&e) {
                            let tripped = breaker.lock().await.record_auth_failure();
                            if tripped {
                                error!(
                                    "[{}] AUTH FAILURE × {AUTH_FAIL_THRESHOLD} consecutivi in bulk indexing — \
                                     circuit breaker attivato per {} minuti. Verifica la password (GUI → Update) e poi Restart.",
                                    account.name,
                                    AUTH_FAIL_COOLDOWN_SECS / 60
                                );
                            } else {
                                error!("[{}] Bulk index {folder} auth-fail: {e}", account.name);
                            }
                        } else {
                            error!("[{}] Bulk index {folder} failed: {e}", account.name);
                        }
                    }
                }
            }
        }

        // Phase 2: Attachment extraction
        for (account, folders, breaker) in &account_folders {
            if !breaker.lock().await.allow_attempt() {
                warn!(
                    "[{}] Phase 2 SKIPPED — breaker tripped.",
                    account.name
                );
                continue;
            }
            info!("[{}] Phase 2: Attachment text extraction...", account.name);
            for folder in folders {
                if !breaker.lock().await.allow_attempt() {
                    break;
                }
                let client = self.client_for(account);
                let indexer =
                    EmailIndexer::new(self.store.clone(), client, &self.config.data_dir);

                match indexer
                    .index_folder(folder, ATTACH_RECENT_COUNT, true, true)
                    .await
                {
                    Ok(result) => {
                        breaker.lock().await.record_success();
                        info!(
                            "[{}] Attachment extraction {folder}: indexed={}, skipped={}",
                            account.name, result.indexed, result.skipped
                        );
                    }
                    Err(e) => {
                        if is_auth_failure(&e) {
                            let tripped = breaker.lock().await.record_auth_failure();
                            if tripped {
                                error!(
                                    "[{}] AUTH FAILURE × {AUTH_FAIL_THRESHOLD} consecutivi in attachment extraction — \
                                     circuit breaker attivato per {} minuti.",
                                    account.name,
                                    AUTH_FAIL_COOLDOWN_SECS / 60
                                );
                            } else {
                                warn!(
                                    "[{}] Attachment extraction {folder} auth-fail: {e}",
                                    account.name
                                );
                            }
                        } else {
                            warn!(
                                "[{}] Attachment extraction {folder} failed: {e}",
                                account.name
                            );
                        }
                    }
                }
            }
        }

        // Phase 3: IDLE/polling watchers + fallback pollers
        info!("Phase 3: Starting IDLE/polling watchers and fallback pollers");
        let mut handles = Vec::new();

        for (account, folders, breaker) in &account_folders {

            for folder in folders.clone() {
                let account = account.clone();
                let signature_dir = self.config.signature_dir.clone();
                let data_dir = self.config.data_dir.clone();
                let store = self.store.clone();
                let lock = self.index_lock.clone();
                let breaker = breaker.clone();
                let oauth_cache = self.oauth_cache.clone();

                handles.push(tokio::spawn(async move {
                    idle_watcher_loop(
                        account,
                        signature_dir,
                        data_dir,
                        store,
                        lock,
                        breaker,
                        oauth_cache,
                        &folder,
                    )
                    .await;
                }));
            }

            // One poller per account
            let account = account.clone();
            let signature_dir = self.config.signature_dir.clone();
            let data_dir = self.config.data_dir.clone();
            let store = self.store.clone();
            let lock = self.index_lock.clone();
            let poller_folders = folders.clone();
            let interval = self.config.index_interval_secs;
            let breaker = breaker.clone();
            let oauth_cache = self.oauth_cache.clone();

            handles.push(tokio::spawn(async move {
                fallback_poller_loop(
                    account,
                    signature_dir,
                    data_dir,
                    store,
                    lock,
                    breaker,
                    oauth_cache,
                    poller_folders,
                    interval,
                )
                .await;
            }));
        }

        for handle in handles {
            if let Err(e) = handle.await {
                error!("Daemon task panicked: {e}");
            }
        }
    }

    /// Detect Qdrant data loss vs stale indexer state.
    async fn check_state_integrity(&self, account: &AccountConfig) {
        let state_file = if account.name.is_empty() || account.name == "primary" {
            "indexer_state.json".to_string()
        } else {
            format!("indexer_state_{}.json", account.name)
        };
        let state_path = self.config.data_dir.join(&state_file);
        let state = IndexerState::load(&state_path);
        let max_claimed_uid: u32 = state
            .folders
            .values()
            .map(|f| f.last_uid)
            .max()
            .unwrap_or(0);

        if max_claimed_uid == 0 {
            return;
        }

        match self.store.get_stats().await {
            Ok(stats) => {
                let expected_min = (max_claimed_uid as usize) / 20;
                if stats.total < expected_min && expected_min > 50 {
                    warn!(
                        "[{}] DATA LOSS DETECTED: indexer state claims UID up to {} \
                         but Qdrant only has {} points (expected >= {}). \
                         Resetting indexer state for full re-index.",
                        account.name, max_claimed_uid, stats.total, expected_min
                    );
                    let empty = IndexerState::default();
                    if let Err(e) = empty.save(&state_path) {
                        error!("[{}] Failed to reset indexer state: {e}", account.name);
                    }
                } else {
                    info!(
                        "[{}] Qdrant sanity check OK: {} points, state max UID={}",
                        account.name, stats.total, max_claimed_uid
                    );
                }
            }
            Err(e) => {
                warn!(
                    "[{}] Could not verify Qdrant data integrity: {e}",
                    account.name
                );
            }
        }
    }
}

/// IDLE watcher loop for a single (account, folder). Reconnects on error.
async fn idle_watcher_loop(
    account: AccountConfig,
    signature_dir: std::path::PathBuf,
    data_dir: std::path::PathBuf,
    store: Arc<MessageStore>,
    lock: Arc<Mutex<()>>,
    breaker: Arc<Mutex<AuthBreaker>>,
    oauth_cache: AccessTokenCache,
    folder: &str,
) {
    loop {
        // Circuit breaker check
        if !breaker.lock().await.allow_attempt() {
            // Tripped: sleep a long while before re-checking
            tokio::time::sleep(Duration::from_secs(300)).await;
            continue;
        }

        let client = build_backend(
            account.clone(),
            signature_dir.clone(),
            oauth_cache.clone(),
        );
        info!(
            "[{}] Starting watcher for {folder} (backend={})",
            account.name,
            client.backend_kind()
        );

        // Build the new-mail callback. Capture clones for the inner index task.
        let store_ref = store.clone();
        let lock_ref = lock.clone();
        let account_ref = account.clone();
        let signature_dir_ref = signature_dir.clone();
        let data_dir_ref = data_dir.clone();
        let oauth_cache_ref = oauth_cache.clone();
        let folder_owned = folder.to_string();

        let on_new: NewMailCallback = Arc::new(move || {
            let store = store_ref.clone();
            let lock = lock_ref.clone();
            let account = account_ref.clone();
            let signature_dir = signature_dir_ref.clone();
            let data_dir = data_dir_ref.clone();
            let oauth_cache = oauth_cache_ref.clone();
            let folder = folder_owned.clone();

            Box::pin(async move {
                let _guard = lock.lock().await;
                let client = build_backend(account.clone(), signature_dir, oauth_cache);
                let indexer = EmailIndexer::new(store, client, &data_dir);
                match indexer.index_folder(&folder, 0, false, false).await {
                    Ok(result) => {
                        if result.indexed > 0 {
                            info!(
                                "[{}] watcher {folder}: indexed {} new emails",
                                account.name, result.indexed
                            );
                        }
                    }
                    Err(e) => {
                        warn!("[{}] watcher index {folder} failed: {e}", account.name);
                    }
                }
            })
        });

        let result = client
            .watch_once(folder, IDLE_TIMEOUT_SECS, on_new)
            .await;

        match result {
            Ok(_) => {
                breaker.lock().await.record_success();
                info!(
                    "[{}] IDLE watcher for {folder} ended normally",
                    account.name
                );
            }
            Err(e) => {
                if is_auth_failure(&e) {
                    let tripped = breaker.lock().await.record_auth_failure();
                    if tripped {
                        error!(
                            "[{}] AUTH FAILURE × {AUTH_FAIL_THRESHOLD} consecutivi su {folder} — \
                             circuit breaker attivato: niente retry per {} minuti. \
                             Verifica la password (GUI → Update) e poi Restart del server.",
                            account.name,
                            AUTH_FAIL_COOLDOWN_SECS / 60
                        );
                    } else {
                        warn!(
                            "[{}] IDLE auth-fail on {folder}: {e}. Retry in {RECONNECT_DELAY_SECS}s",
                            account.name
                        );
                    }
                } else {
                    breaker.lock().await.record_other_error();
                    warn!(
                        "[{}] IDLE watcher for {folder} failed: {e}. Reconnecting in {RECONNECT_DELAY_SECS}s",
                        account.name
                    );
                }
            }
        }

        tokio::time::sleep(Duration::from_secs(RECONNECT_DELAY_SECS)).await;
    }
}

/// Fallback poller loop for a single account.
async fn fallback_poller_loop(
    account: AccountConfig,
    signature_dir: std::path::PathBuf,
    data_dir: std::path::PathBuf,
    store: Arc<MessageStore>,
    lock: Arc<Mutex<()>>,
    breaker: Arc<Mutex<AuthBreaker>>,
    oauth_cache: AccessTokenCache,
    folders: Vec<String>,
    interval_secs: u64,
) {
    let interval = Duration::from_secs(interval_secs);
    info!(
        "[{}] Fallback poller: interval={}s, folders={}",
        account.name,
        interval_secs,
        folders.len()
    );

    loop {
        tokio::time::sleep(interval).await;

        // Skip the cycle entirely if the breaker is tripped.
        if !breaker.lock().await.allow_attempt() {
            continue;
        }

        let _guard = lock.lock().await;

        for folder in &folders {
            let client = build_backend(account.clone(), signature_dir.clone(), oauth_cache.clone());
            let indexer = EmailIndexer::new(store.clone(), client, &data_dir);

            match indexer.index_folder(folder, 0, false, false).await {
                Ok(result) => {
                    breaker.lock().await.record_success();
                    if result.indexed > 0 {
                        info!(
                            "[{}] Poller {folder}: indexed {} new emails",
                            account.name, result.indexed
                        );
                    }
                }
                Err(e) => {
                    if is_auth_failure(&e) {
                        let tripped = breaker.lock().await.record_auth_failure();
                        if tripped {
                            error!(
                                "[{}] AUTH FAILURE × {AUTH_FAIL_THRESHOLD} consecutivi sul poller — \
                                 circuit breaker attivato: niente retry per {} minuti. \
                                 Verifica la password (GUI → Update) e poi Restart del server.",
                                account.name,
                                AUTH_FAIL_COOLDOWN_SECS / 60
                            );
                            break; // also skip remaining folders this cycle
                        } else {
                            warn!("[{}] Poller auth-fail {folder}: {e}", account.name);
                        }
                    } else {
                        breaker.lock().await.record_other_error();
                        warn!("[{}] Poller {folder} failed: {e}", account.name);
                    }
                }
            }
        }
    }
}

// Suppress unused warning if `_` placeholder is needed in callsites; ATTACH_RECENT_COUNT
// is referenced above directly so this isn't necessary.
#[allow(dead_code)]
const _: u32 = 0;
