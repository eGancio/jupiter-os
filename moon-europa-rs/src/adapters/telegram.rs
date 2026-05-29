use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use grammers_client::media::Media;
use grammers_client::peer::Peer;
use grammers_client::Client;
use grammers_mtsender::SenderPool;
use grammers_session::storages::SqliteSession;
use grammers_session::types::PeerRef;
use grammers_tl_types as tl;
use jupiteros_shared::store::{IndexResult, MessageData, MessageStore};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use super::{Contact, ContactType, Entity, MessagingAdapter};
use crate::config::Config;
use crate::error::{EuropaError, Result};

/// Persistent indexer state: tracks the last indexed message ID per chat.
#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct IndexerState {
    pub(crate) last_message_id: HashMap<i64, i32>,
}

impl IndexerState {
    pub(crate) fn load(path: &Path) -> Self {
        if path.exists() {
            std::fs::read_to_string(path)
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default()
        } else {
            Self::default()
        }
    }

    pub(crate) fn save(&self, path: &Path) {
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, json);
        }
    }
}

/// Telegram adapter using grammers (pure Rust MTProto client).
pub struct TelegramAdapter {
    config: Config,
    /// The high-level grammers client, initialized on connect().
    client: Mutex<Option<Client>>,
    /// Keep the runner task alive so TCP connections stay open.
    _runner_handle: Mutex<Option<JoinHandle<()>>>,
    /// Cached PeerRef for each resolved entity (by bot_api_dialog_id).
    /// Populated by resolve_entities(), used by bulk_index() to avoid
    /// resolve_username() calls (which fail on chats without usernames).
    peer_refs: Mutex<HashMap<i64, PeerRef>>,
    /// Resolved recipient cache: lowercase name/query → (Peer, PeerRef).
    /// Avoids re-iterating all dialogs on repeated sends to the same recipient.
    resolved_cache: Mutex<HashMap<String, (Peer, PeerRef)>>,
    /// Snapshot of all contacts, filled by resolve_entities(). Lets list_contacts
    /// return instantly instead of re-iterating ~hundreds of dialogs over the
    /// network (which can take >60s and trip the MCP client timeout).
    contacts_cache: Mutex<Vec<Contact>>,
}

impl TelegramAdapter {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            client: Mutex::new(None),
            _runner_handle: Mutex::new(None),
            peer_refs: Mutex::new(HashMap::new()),
            resolved_cache: Mutex::new(HashMap::new()),
            contacts_cache: Mutex::new(Vec::new()),
        }
    }

    /// Get a reference to the connected client, or error.
    async fn get_client(&self) -> Result<tokio::sync::MutexGuard<'_, Option<Client>>> {
        let guard = self.client.lock().await;
        if guard.is_none() {
            return Err(EuropaError::NotAuthorized(
                "Telegram not connected. Call connect() first.".into(),
            ));
        }
        Ok(guard)
    }

    /// Resolve a recipient by display name, @username, or numeric ID.
    /// Uses a local cache so repeated sends to the same recipient are instant.
    async fn resolve_recipient(&self, client: &Client, to: &str) -> Result<(Peer, PeerRef)> {
        let to_trimmed = to.trim();
        let to_lower = to_trimmed.to_lowercase();

        // 0. Check resolved cache first (instant, no API call). The daemon's
        // resolve_entities() pre-populates this with every dialog's display name,
        // so we can also do a partial match here and avoid the (very slow,
        // network-bound) full dialog iteration below.
        {
            let cache = self.resolved_cache.lock().await;
            if let Some((peer, peer_ref)) = cache.get(&to_lower) {
                debug!("Recipient '{}' found in cache (exact)", to_trimmed);
                return Ok((peer.clone(), *peer_ref));
            }
            if to_lower.len() >= 3 {
                if let Some((peer, peer_ref)) = cache.iter().find_map(|(name, v)| {
                    if name.contains(&to_lower) || to_lower.contains(name.as_str()) {
                        Some(v)
                    } else {
                        None
                    }
                }) {
                    debug!("Recipient '{}' found in cache (partial)", to_trimmed);
                    return Ok((peer.clone(), *peer_ref));
                }
            }
        }

        // 1. If it looks like @username, try resolve_username first
        if to_trimmed.starts_with('@') {
            let username = to_trimmed.trim_start_matches('@');
            if let Ok(Some(peer)) = client.resolve_username(username).await {
                if let Some(peer_ref) = peer.to_ref().await {
                    self.cache_recipient(&to_lower, &peer, peer_ref).await;
                    return Ok((peer, peer_ref));
                }
            }
        }

        // 2. Check cached peer_refs for numeric ID
        {
            let refs = self.peer_refs.lock().await;
            if let Ok(id) = to_trimmed.parse::<i64>() {
                if refs.contains_key(&id) {
                    drop(refs);
                    let mut dialogs = client.iter_dialogs();
                    while let Some(dialog) = dialogs.next().await.map_err(|e| {
                        EuropaError::Telegram(format!("Iter dialogs: {e}"))
                    })? {
                        if dialog.peer().id().bot_api_dialog_id() == id {
                            let peer = dialog.peer().clone();
                            let peer_ref = dialog.peer_ref();
                            self.cache_recipient(&to_lower, &peer, peer_ref).await;
                            return Ok((peer, peer_ref));
                        }
                    }
                }
            }
        }

        // 3. Search dialogs by display name (case-insensitive partial match)
        let mut dialogs = client.iter_dialogs();
        while let Some(dialog) = dialogs
            .next()
            .await
            .map_err(|e| EuropaError::Telegram(format!("Iter dialogs: {e}")))?
        {
            let name = dialog.peer().name().unwrap_or_default().to_lowercase();
            if name == to_lower || name.contains(&to_lower) {
                let peer = dialog.peer().clone();
                let peer_ref = dialog.peer_ref();
                self.cache_recipient(&to_lower, &peer, peer_ref).await;
                return Ok((peer, peer_ref));
            }
        }

        // 4. Last resort: try resolve_username (without @)
        let username = to_trimmed.trim_start_matches('@');
        let peer = client
            .resolve_username(username)
            .await
            .map_err(|e| EuropaError::Telegram(format!("Resolve '{}': {e}", to)))?
            .ok_or_else(|| {
                EuropaError::Telegram(format!(
                    "Recipient '{}' not found. Use display name, @username, or numeric ID.",
                    to
                ))
            })?;

        let peer_ref = peer
            .to_ref()
            .await
            .ok_or_else(|| EuropaError::Telegram("Could not get peer reference".into()))?;

        self.cache_recipient(&to_lower, &peer, peer_ref).await;
        Ok((peer, peer_ref))
    }

    /// Add a resolved recipient to the cache (keyed by query + display name).
    async fn cache_recipient(&self, query: &str, peer: &Peer, peer_ref: PeerRef) {
        let mut cache = self.resolved_cache.lock().await;
        cache.insert(query.to_string(), (peer.clone(), peer_ref));
        // Also cache by display name so "Mario Rossi" works after resolving "@mario"
        let name = peer.name().unwrap_or_default().to_lowercase();
        if !name.is_empty() && name != query {
            cache.insert(name, (peer.clone(), peer_ref));
        }
    }

    /// State file path for the indexer.
    fn state_path(&self) -> PathBuf {
        self.config.data_dir.join("indexer_state.json")
    }

    /// Interactive Telegram auth flow.
    /// Prompts for phone number and OTP code on stdin/stdout.
    /// Saves the session to SQLite so subsequent connects() work.
    pub async fn auth_interactive(config: &Config) -> std::result::Result<(), anyhow::Error> {
        use std::io::{self, BufRead, Write};
        use grammers_client::SignInError;

        println!("=== Moon Europa — Telegram Auth ===");
        println!("Session file: {}", config.session_path.display());

        let session_path = &config.session_path;
        std::fs::create_dir_all(session_path.parent().unwrap_or(Path::new("."))).ok();
        let session = SqliteSession::open(session_path).await?;
        let session = Arc::new(session);

        let pool = SenderPool::new(session, config.telegram_api_id);
        let SenderPool {
            runner,
            handle,
            updates: _updates,
        } = pool;

        let runner_handle = tokio::spawn(async move {
            runner.run().await;
        });

        let client = Client::new(handle);

        // Check if already authorized
        if client.is_authorized().await? {
            let me = client.get_me().await?;
            println!(
                "Already authorized as: {} (ID: {})",
                me.first_name().unwrap_or("?"),
                me.id()
            );
            client.disconnect();
            runner_handle.abort();
            return Ok(());
        }

        // Prompt for phone number
        print!("Phone number (with +country code, e.g. +39...): ");
        io::stdout().flush()?;
        let mut phone = String::new();
        io::stdin().lock().read_line(&mut phone)?;
        let phone = phone.trim().to_string();

        println!("Requesting login code...");
        let token = client.request_login_code(&phone, &config.telegram_api_hash).await?;

        // Prompt for OTP code
        print!("Enter the code you received: ");
        io::stdout().flush()?;
        let mut code = String::new();
        io::stdin().lock().read_line(&mut code)?;
        let code = code.trim().to_string();

        match client.sign_in(&token, &code).await {
            Ok(_user) => {
                println!("Signed in successfully!");
            }
            Err(SignInError::PasswordRequired(password_token)) => {
                // 2FA enabled
                print!("2FA password required. Enter password: ");
                io::stdout().flush()?;
                let mut password = String::new();
                io::stdin().lock().read_line(&mut password)?;
                let password = password.trim();
                client.check_password(password_token, password).await?;
                println!("Signed in with 2FA successfully!");
            }
            Err(e) => {
                return Err(anyhow::anyhow!("Sign in failed: {e}"));
            }
        }

        let me = client.get_me().await?;
        println!(
            "Logged in as: {} (ID: {})",
            me.first_name().unwrap_or("?"),
            me.id()
        );

        client.disconnect();
        runner_handle.abort();
        println!("Session saved. You can now start the server normally.");
        Ok(())
    }

    /// Get the set of peer IDs belonging to a Telegram folder (dialog filter) by name.
    async fn get_folder_peer_ids(
        &self,
        client: &Client,
        folder_name: &str,
    ) -> std::result::Result<HashSet<i64>, EuropaError> {
        let response = client
            .invoke(&tl::functions::messages::GetDialogFilters {})
            .await
            .map_err(|e| EuropaError::Telegram(format!("GetDialogFilters: {e}")))?;

        // Extract the filters list from the response
        let filters = match response {
            tl::enums::messages::DialogFilters::Filters(df) => df.filters,
        };

        // Find the filter matching the requested folder name
        for filter in filters {
            if let tl::enums::DialogFilter::Filter(f) = filter {
                // title is TextWithEntities — extract the plain text
                let title_text = match &f.title {
                    tl::enums::TextWithEntities::Entities(t) => &t.text,
                };
                if title_text.eq_ignore_ascii_case(folder_name) {
                    let mut ids = HashSet::new();
                    // Collect peers from both include_peers and pinned_peers
                    for peer in f.include_peers.iter().chain(f.pinned_peers.iter()) {
                        if let Some(id) = Self::input_peer_to_bot_api_id(peer) {
                            ids.insert(id);
                        }
                    }
                    return Ok(ids);
                }
            }
        }

        Err(EuropaError::Telegram(format!(
            "Folder '{}' not found in dialog filters",
            folder_name
        )))
    }

    /// Convert a TL InputPeer to bot API dialog ID format.
    fn input_peer_to_bot_api_id(peer: &tl::enums::InputPeer) -> Option<i64> {
        match peer {
            tl::enums::InputPeer::User(u) => Some(u.user_id),
            tl::enums::InputPeer::Chat(c) => Some(-c.chat_id),
            tl::enums::InputPeer::Channel(ch) => Some(-1_000_000_000_000 - ch.channel_id),
            _ => None, // Empty, Self, FromMessage variants
        }
    }

    /// Convert a grammers Message to our MessageData format.
    fn make_message_data(
        msg: &grammers_client::message::Message,
        chat_name: &str,
    ) -> MessageData {
        let text = msg.text().to_string();
        let sender_name = msg
            .sender()
            .and_then(|p| p.name().map(|s| s.to_string()))
            .unwrap_or_else(|| "Unknown".to_string());
        let date = msg.date().to_rfc3339();

        MessageData {
            id: format!("tg_{}", msg.id()),
            text,
            sender: sender_name,
            recipient: chat_name.to_string(),
            subject: String::new(),
            date,
            channel: "telegram".to_string(),
            folder: String::new(),
            imap_uid: None,
            account: String::new(),
        }
    }
}

#[async_trait]
impl MessagingAdapter for TelegramAdapter {
    fn channel_name(&self) -> &str {
        "telegram"
    }

    async fn connect(&mut self) -> Result<()> {
        if !self.config.has_telegram() {
            return Err(EuropaError::Config(
                "Telegram API credentials not configured".into(),
            ));
        }

        info!("Connecting to Telegram...");

        // Open persistent SQLite session (preserves auth keys across restarts)
        let session_path = &self.config.session_path;
        std::fs::create_dir_all(session_path.parent().unwrap_or(Path::new("."))).ok();
        let session = SqliteSession::open(session_path)
            .await
            .map_err(|e| EuropaError::Telegram(format!("Open session: {e}")))?;
        let session = Arc::new(session);

        // Create sender pool (manages TCP connections to Telegram)
        let pool = SenderPool::new(session, self.config.telegram_api_id);

        // Destructure pool into components
        let SenderPool {
            runner,
            handle,
            updates: _updates,
        } = pool;

        // Spawn the runner task — drives all TCP I/O
        let runner_handle = tokio::spawn(async move {
            runner.run().await;
        });

        // Create high-level client from the fat handle
        let client = Client::new(handle);

        // Check authorization
        let authorized = client
            .is_authorized()
            .await
            .map_err(|e| EuropaError::Telegram(format!("Check auth: {e}")))?;

        if !authorized {
            return Err(EuropaError::NotAuthorized(
                "Telegram session not authorized. Run: moon-europa --auth".into(),
            ));
        }

        let me = client
            .get_me()
            .await
            .map_err(|e| EuropaError::Telegram(format!("Get me: {e}")))?;

        info!(
            "Connected as: {} (ID: {})",
            me.first_name().unwrap_or("?"),
            me.id()
        );

        *self.client.lock().await = Some(client);
        *self._runner_handle.lock().await = Some(runner_handle);

        Ok(())
    }

    async fn disconnect(&mut self) -> Result<()> {
        if let Some(client) = self.client.lock().await.take() {
            client.disconnect();
        }
        if let Some(handle) = self._runner_handle.lock().await.take() {
            handle.abort();
        }
        info!("Telegram disconnected");
        Ok(())
    }

    async fn list_contacts(&self) -> Result<Vec<Contact>> {
        // Fast path: return the snapshot built by resolve_entities() (the daemon
        // populates it at startup). Iterating all dialogs live is network-bound
        // and can take >60s on large accounts, tripping the MCP client timeout.
        {
            let cached = self.contacts_cache.lock().await;
            if !cached.is_empty() {
                return Ok(cached.clone());
            }
        }

        let guard = self.get_client().await?;
        let client = guard.as_ref().unwrap();

        let mut contacts = Vec::new();
        let mut dialogs = client.iter_dialogs();

        while let Some(dialog) = dialogs
            .next()
            .await
            .map_err(|e| EuropaError::Telegram(format!("List dialogs: {e}")))?
        {
            let peer = dialog.peer();
            let (contact_type, username) = match peer {
                Peer::User(u) => (ContactType::User, u.username().map(|s| s.to_string())),
                Peer::Group(_) => (ContactType::Group, None),
                Peer::Channel(ch) => (
                    ContactType::Channel,
                    ch.username().map(|s| s.to_string()),
                ),
            };

            contacts.push(Contact {
                name: peer.name().unwrap_or_default().to_string(),
                id: peer.id().bot_api_dialog_id(),
                contact_type,
                username,
            });
        }

        *self.contacts_cache.lock().await = contacts.clone();
        Ok(contacts)
    }

    async fn send_message(
        &self,
        to: &str,
        message: &str,
        attachments: &[PathBuf],
    ) -> Result<String> {
        use grammers_client::message::InputMessage;

        let guard = self.get_client().await?;
        let client = guard.as_ref().unwrap();

        // Resolve recipient by name, @username, or ID
        let (peer, peer_ref) = self.resolve_recipient(client, to).await?;
        let display_name = peer.name().unwrap_or_default().to_string();

        if attachments.is_empty() {
            // Text-only message
            if message.is_empty() {
                return Err(EuropaError::Telegram(
                    "Message text is empty and no attachments provided".into(),
                ));
            }
            let sent = client
                .send_message(peer_ref, message)
                .await
                .map_err(|e| EuropaError::Telegram(format!("Send message: {e}")))?;

            Ok(format!("Sent to {} (msg_id: {})", display_name, sent.id()))
        } else {
            // Upload and send each attachment
            let mut last_msg_id = 0i32;

            for (i, path) in attachments.iter().enumerate() {
                if !path.exists() {
                    return Err(EuropaError::Telegram(format!(
                        "File not found: {}",
                        path.display()
                    )));
                }

                info!("Uploading file: {}", path.display());
                let uploaded = client
                    .upload_file(path)
                    .await
                    .map_err(|e| EuropaError::Telegram(format!("Upload '{}': {e}", path.display())))?;

                // Caption text only on the first attachment
                let caption = if i == 0 && !message.is_empty() {
                    message
                } else {
                    ""
                };

                let msg = InputMessage::new().text(caption).file(uploaded);

                let sent = client
                    .send_message(peer_ref, msg)
                    .await
                    .map_err(|e| {
                        EuropaError::Telegram(format!("Send file '{}': {e}", path.display()))
                    })?;

                last_msg_id = sent.id();
                info!("Sent file {} to {} (msg_id: {})", path.display(), display_name, last_msg_id);
            }

            let n = attachments.len();
            Ok(format!(
                "Sent to {} ({} file{}, msg_id: {})",
                display_name,
                n,
                if n > 1 { "s" } else { "" },
                last_msg_id
            ))
        }
    }

    async fn send_message_and_index(
        &self,
        to: &str,
        message: &str,
        attachments: &[PathBuf],
        store: &MessageStore,
    ) -> Result<String> {
        let result = self.send_message(to, message, attachments).await?;

        // Index the sent message (best effort)
        let msg = MessageData {
            id: format!("tg_sent_{}", Utc::now().timestamp_millis()),
            text: message.to_string(),
            sender: "me".to_string(),
            recipient: to.to_string(),
            subject: String::new(),
            date: Utc::now().to_rfc3339(),
            channel: "telegram".to_string(),
            folder: String::new(),
            imap_uid: None,
            account: String::new(),
        };

        if let Err(e) = store.index_message(msg).await {
            warn!("Failed to index sent message: {e}");
        }

        Ok(result)
    }

    async fn save_media(
        &self,
        chat: &str,
        message_id: Option<i64>,
        last_n: usize,
        save_dir: &Path,
    ) -> Result<Vec<PathBuf>> {
        let guard = self.get_client().await?;
        let client = guard.as_ref().unwrap();

        // Resolve chat by name, @username, or ID
        let (peer, peer_ref) = self.resolve_recipient(client, chat).await?;

        std::fs::create_dir_all(save_dir)?;

        let mut saved_files = Vec::new();
        let mut messages = client.iter_messages(peer_ref);
        let mut count = 0;
        let chat_name = peer.name().unwrap_or_default().to_string();

        while let Some(msg) = messages
            .next()
            .await
            .map_err(|e| EuropaError::Telegram(format!("Iter messages: {e}")))?
        {
            if count >= last_n {
                break;
            }

            // If specific message ID requested, skip others
            if let Some(target_id) = message_id {
                if msg.id() as i64 != target_id {
                    continue;
                }
            }

            let media = match msg.media() {
                Some(m) => m,
                None => continue,
            };

            // Only Photo, Document, Sticker are downloadable in grammers.
            // WebPage, Contact, Poll, Geo, Dice, Venue, GeoLive are NOT.
            let filename = match &media {
                Media::Document(doc) => {
                    // Use original filename if available (e.g. "brand_identity.pdf")
                    match doc.name() {
                        Some(name) if !name.is_empty() => name.to_string(),
                        _ => format!("tg_{}_{}.bin", chat_name, msg.id()),
                    }
                }
                Media::Photo(_) => format!("tg_{}_{}.jpg", chat_name, msg.id()),
                Media::Sticker(_) => format!("tg_{}_{}_sticker.webp", chat_name, msg.id()),
                _ => {
                    // Non-downloadable media type — skip silently
                    debug!("Skipping non-downloadable media type in msg {}", msg.id());
                    continue;
                }
            };

            let file_path = save_dir.join(&filename);

            // Download media — handle errors gracefully (skip and try next)
            let mut download = client.iter_download(&media);
            let mut file_bytes = Vec::new();
            let mut download_ok = true;

            while let Some(chunk_result) = {
                let r = download.next().await;
                Some(r)
            } {
                match chunk_result {
                    Ok(Some(chunk)) => file_bytes.extend(chunk),
                    Ok(None) => break,
                    Err(e) => {
                        warn!("Download error for msg {} ({}): {e} — skipping", msg.id(), filename);
                        download_ok = false;
                        break;
                    }
                }
            }

            if download_ok && !file_bytes.is_empty() {
                std::fs::write(&file_path, &file_bytes)?;
                info!(
                    "Saved media: {} ({} bytes)",
                    file_path.display(),
                    file_bytes.len()
                );
                saved_files.push(file_path);
            }

            count += 1;
        }

        Ok(saved_files)
    }

    async fn resolve_entities(&self) -> Result<Vec<Entity>> {
        let guard = self.get_client().await?;
        let client = guard.as_ref().unwrap();

        let mut entities = Vec::new();
        let mut contacts: Vec<Contact> = Vec::new();
        let mut refs_map = HashMap::new();
        let mut resolved_map: HashMap<String, (Peer, PeerRef)> = HashMap::new();

        // If a folder is configured, get the peer IDs in that folder
        let folder_filter: Option<HashSet<i64>> =
            if let Some(folder_name) = &self.config.telegram_folder {
                info!("Filtering chats by folder '{}'...", folder_name);
                match self.get_folder_peer_ids(client, folder_name).await {
                    Ok(ids) => {
                        info!("Folder '{}' contains {} peers", folder_name, ids.len());
                        Some(ids)
                    }
                    Err(e) => {
                        warn!(
                            "Could not get folder '{}': {e}. Falling back to all dialogs.",
                            folder_name
                        );
                        None
                    }
                }
            } else {
                None
            };

        // Iterate all dialogs, filtering by folder if configured
        let mut dialogs = client.iter_dialogs();
        while let Some(dialog) = dialogs
            .next()
            .await
            .map_err(|e| EuropaError::Telegram(format!("Iter dialogs: {e}")))?
        {
            let peer = dialog.peer();
            let peer_id = peer.id().bot_api_dialog_id();

            // Apply folder filter: skip chats not in the configured folder
            if let Some(ref allowed_ids) = folder_filter {
                if !allowed_ids.contains(&peer_id) {
                    continue;
                }
            }

            let (entity_type, username) = match peer {
                Peer::User(u) => (ContactType::User, u.username().map(|s| s.to_string())),
                Peer::Group(_) => (ContactType::Group, None),
                Peer::Channel(ch) => (ContactType::Channel, ch.username().map(|s| s.to_string())),
            };

            // Cache the PeerRef for use in bulk_index
            let peer_ref = dialog.peer_ref();
            refs_map.insert(peer_id, peer_ref);

            let display_name = peer.name().unwrap_or_default().to_string();

            // Pre-populate resolved cache so send_message is instant for monitored contacts
            let peer_clone = peer.clone();
            let name_lower = display_name.to_lowercase();
            if !name_lower.is_empty() {
                resolved_map.insert(name_lower, (peer_clone, peer_ref));
            }

            // Snapshot for list_contacts (avoids a slow live dialog iteration later)
            contacts.push(Contact {
                name: display_name.clone(),
                id: peer_id,
                contact_type: entity_type.clone(),
                username,
            });

            entities.push(Entity {
                id: peer_id,
                name: display_name,
                entity_type,
            });
        }

        // Store cached peer refs
        *self.peer_refs.lock().await = refs_map;
        // Store resolved recipient cache
        *self.resolved_cache.lock().await = resolved_map;
        // Store contacts snapshot
        *self.contacts_cache.lock().await = contacts;

        info!("Resolved {} entities for indexing", entities.len());
        Ok(entities)
    }

    async fn bulk_index(&self, store: &MessageStore, entity: &Entity) -> Result<IndexResult> {
        let guard = self.get_client().await?;
        let client = guard.as_ref().unwrap();

        // Look up the cached PeerRef (populated by resolve_entities)
        let peer_ref = {
            let refs = self.peer_refs.lock().await;
            refs.get(&entity.id).copied()
        };

        let peer_ref = match peer_ref {
            Some(r) => r,
            None => {
                warn!(
                    "No cached peer ref for '{}' (id={}), skipping",
                    entity.name, entity.id
                );
                return Ok(IndexResult::default());
            }
        };

        let mut state = IndexerState::load(&self.state_path());
        let last_id = state
            .last_message_id
            .get(&entity.id)
            .copied()
            .unwrap_or(0);

        let mut result = IndexResult::default();
        let mut batch = Vec::new();
        let mut max_id = last_id;
        let mut messages = client.iter_messages(peer_ref);

        while let Some(msg) = messages
            .next()
            .await
            .map_err(|e| EuropaError::Telegram(format!("Iter for bulk: {e}")))?
        {
            let msg_id = msg.id();

            // Stop if we've seen this message before
            if msg_id <= last_id {
                break;
            }

            if msg_id > max_id {
                max_id = msg_id;
            }

            let data = Self::make_message_data(&msg, &entity.name);
            batch.push(data);

            // Flush batch every N messages
            if batch.len() >= self.config.batch_size {
                let batch_result = store
                    .index_batch(std::mem::take(&mut batch), false)
                    .await?;
                result.indexed += batch_result.indexed;
                result.skipped += batch_result.skipped;
            }
        }

        // Flush remaining
        if !batch.is_empty() {
            let batch_result = store.index_batch(batch, false).await?;
            result.indexed += batch_result.indexed;
            result.skipped += batch_result.skipped;
        }

        // Update state
        if max_id > last_id {
            state.last_message_id.insert(entity.id, max_id);
            state.save(&self.state_path());
        }

        Ok(result)
    }

    async fn start_listener(
        &self,
        store: &MessageStore,
        entities: &[Entity],
    ) -> Result<()> {
        // Validate client is connected, then release the lock immediately
        // so MCP tool calls (send_message, etc.) can use the client concurrently.
        {
            let _guard = self.get_client().await?;
        }

        let entity_ids: HashSet<i64> = entities.iter().map(|e| e.id).collect();

        info!(
            "Listening for updates from {} entities...",
            entity_ids.len()
        );

        // TODO: Implement proper update streaming with grammers v0.9
        warn!("Real-time listener not yet implemented for grammers v0.9");
        warn!("Falling back to periodic polling...");

        // Simple polling fallback: re-run bulk_index periodically
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            debug!("Polling for new messages...");

            for entity in entities {
                match self.bulk_index(store, entity).await {
                    Ok(r) if r.indexed > 0 => {
                        info!("Polled '{}': {} new messages", entity.name, r.indexed);
                    }
                    Ok(_) => {}
                    Err(e) => {
                        warn!("Poll error for '{}': {e}", entity.name);
                    }
                }
                // Anti-flood: wait between entity polls
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
        }
    }
}
