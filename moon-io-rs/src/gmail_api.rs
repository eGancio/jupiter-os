//! Gmail REST API backend (`https://gmail.googleapis.com/gmail/v1/users/me`).
//!
//! Used in place of IMAP+XOAUTH2 for Gmail Workspace accounts, because
//! `async-imap` 0.10 deadlocks on `AUTHENTICATE XOAUTH2` against Gmail.
//!
//! Synthetic UID strategy: Gmail message IDs are 16-char hex strings. We
//! map them deterministically to `u32` by parsing the first 8 hex digits
//! (with `wrapping_add(1)` collision resolution), and persist the bidirectional
//! map in `data_dir/gmail_state_<account>.json`. This keeps the rest of the
//! pipeline (Qdrant payload, MCP tool params) unchanged.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine as _;
use mail_parser::MimeHeaders;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::config::AccountConfig;
use crate::email_client::{
    self, AttachmentData, AttachmentInfo, BounceInfo, EmailData, SendEmailParams, SendResult,
};
use crate::error::{IoError, Result};
use crate::mail_backend::{MailBackend, NewMailCallback};
use crate::oauth::{self, AccessTokenCache};
use crate::signature;

const GMAIL_API: &str = "https://gmail.googleapis.com/gmail/v1/users/me";

// ---------------------------------------------------------------------------
// State (persisted on disk: synthetic UID map + last historyId for polling)
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Serialize, Deserialize, Clone)]
struct GmailState {
    /// Last seen Gmail `historyId` per folder (used by watch_once for
    /// incremental sync via /history endpoint).
    #[serde(default)]
    pub last_history_id: HashMap<String, String>,
    /// synthetic_uid -> gmail message_id. Reverse map (msgid -> uid) is built
    /// in-memory on load to avoid storing it twice.
    #[serde(default)]
    pub uid_to_msgid: HashMap<u32, String>,
}

impl GmailState {
    fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    fn save(&self, path: &Path) -> Result<()> {
        let tmp = path.with_extension("json.tmp");
        let content = serde_json::to_string_pretty(self)
            .map_err(|e| IoError::Other(format!("Serialize gmail_state: {e}")))?;
        std::fs::write(&tmp, &content)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Gmail API response models (only what we use)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct GmailError {
    error: GmailErrorInner,
}

#[derive(Debug, Deserialize)]
struct GmailErrorInner {
    #[serde(default)]
    code: i32,
    #[serde(default)]
    message: String,
    #[serde(default)]
    status: String,
}

#[derive(Debug, Deserialize)]
struct LabelsList {
    #[serde(default)]
    labels: Vec<Label>,
}

#[derive(Debug, Deserialize, Clone)]
struct Label {
    id: String,
    name: String,
    #[serde(rename = "type", default)]
    label_type: String, // "system" | "user"
}

#[derive(Debug, Deserialize)]
struct MessagesList {
    #[serde(default)]
    messages: Vec<MessageRef>,
    #[serde(rename = "nextPageToken", default)]
    next_page_token: Option<String>,
    // resultSizeEstimate is also returned, not needed.
}

#[derive(Debug, Deserialize)]
struct MessageRef {
    id: String,
    // threadId also returned, not needed for now.
}

#[derive(Debug, Deserialize)]
struct MessageFull {
    id: String,
    #[serde(rename = "threadId", default)]
    thread_id: String,
    #[serde(rename = "internalDate", default)]
    internal_date: String, // ms since epoch as string
    #[serde(default)]
    payload: Option<MessagePart>,
    /// Only present when format=raw — base64url-encoded full RFC 822 message.
    #[serde(default)]
    raw: Option<String>,
    #[serde(rename = "historyId", default)]
    history_id: Option<String>,
    #[serde(rename = "labelIds", default)]
    label_ids: Vec<String>,
}

#[derive(Debug, Deserialize, Clone)]
struct MessagePart {
    #[serde(rename = "partId", default)]
    part_id: String,
    #[serde(rename = "mimeType", default)]
    mime_type: String,
    #[serde(default)]
    filename: String,
    #[serde(default)]
    headers: Vec<Header>,
    #[serde(default)]
    body: Option<MessagePartBody>,
    #[serde(default)]
    parts: Vec<MessagePart>,
}

#[derive(Debug, Deserialize, Clone)]
struct Header {
    name: String,
    value: String,
}

#[derive(Debug, Deserialize, Clone)]
struct MessagePartBody {
    #[serde(rename = "attachmentId", default)]
    attachment_id: Option<String>,
    #[serde(default)]
    size: u64,
    /// base64url-encoded body
    #[serde(default)]
    data: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AttachmentResp {
    #[serde(default)]
    size: u64,
    /// base64url-encoded attachment body
    #[serde(default)]
    data: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ProfileResp {
    #[serde(rename = "emailAddress", default)]
    _email_address: String,
    #[serde(rename = "historyId", default)]
    history_id: String,
}

#[derive(Debug, Deserialize)]
struct HistoryResp {
    #[serde(default)]
    history: Vec<HistoryRecord>,
    #[serde(rename = "historyId", default)]
    history_id: Option<String>,
    #[serde(rename = "nextPageToken", default)]
    _next_page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct HistoryRecord {
    #[serde(rename = "messagesAdded", default)]
    messages_added: Vec<HistoryMessageAdded>,
}

#[derive(Debug, Deserialize)]
struct HistoryMessageAdded {
    message: MessageRef,
}

// ---------------------------------------------------------------------------
// GmailApiBackend
// ---------------------------------------------------------------------------

pub struct GmailApiBackend {
    account: AccountConfig,
    signature_dir: PathBuf,
    oauth_cache: AccessTokenCache,
    http: reqwest::Client,
    state_path: PathBuf,
    state: Arc<RwLock<GmailState>>,
    msgid_to_uid: Arc<RwLock<HashMap<String, u32>>>,
    /// Cached labels list (id ↔ name) — refreshed on demand.
    labels_cache: Arc<RwLock<Option<Vec<Label>>>>,
}

impl GmailApiBackend {
    pub fn new(
        account: AccountConfig,
        signature_dir: PathBuf,
        oauth_cache: AccessTokenCache,
    ) -> Self {
        // Pick a data_dir for state. Heuristic: derive from a path next to
        // signature_dir which is `<data_dir>/signatures`. So data_dir =
        // signature_dir.parent(). Fallback to CWD.
        let data_dir = signature_dir
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

        let state_path = data_dir.join(format!("gmail_state_{}.json", account.name));
        let state = GmailState::load(&state_path);

        // Build reverse map
        let mut msgid_to_uid = HashMap::with_capacity(state.uid_to_msgid.len());
        for (uid, mid) in &state.uid_to_msgid {
            msgid_to_uid.insert(mid.clone(), *uid);
        }

        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        Self {
            account,
            signature_dir,
            oauth_cache,
            http,
            state_path,
            state: Arc::new(RwLock::new(state)),
            msgid_to_uid: Arc::new(RwLock::new(msgid_to_uid)),
            labels_cache: Arc::new(RwLock::new(None)),
        }
    }

    // -----------------------------------------------------------------------
    // Auth + HTTP plumbing
    // -----------------------------------------------------------------------

    async fn access_token(&self) -> Result<String> {
        oauth::get_access_token(&self.account.name, &self.oauth_cache)
            .await
            .map_err(|e| IoError::Imap(format!("OAuth: {e}")))
    }

    /// GET request with bearer auth, 401-retry-after-refresh, basic 429 backoff.
    async fn get_json<T: for<'de> Deserialize<'de>>(&self, url: &str) -> Result<T> {
        for attempt in 0..3 {
            let token = self.access_token().await?;
            let resp = self
                .http
                .get(url)
                .bearer_auth(&token)
                .send()
                .await
                .map_err(|e| IoError::Imap(format!("GET {url}: {e}")))?;

            let status = resp.status();
            if status.is_success() {
                let body = resp.bytes().await.map_err(|e| IoError::Imap(e.to_string()))?;
                return serde_json::from_slice::<T>(&body).map_err(|e| {
                    IoError::Parse(format!("Gmail parse: {e} (body: {})", String::from_utf8_lossy(&body)))
                });
            }

            // 401: token expired mid-flight; clear cache and try again
            if status == reqwest::StatusCode::UNAUTHORIZED && attempt == 0 {
                debug!("[{}] gmail 401 → refreshing token", self.account.name);
                self.oauth_cache.clear(&self.account.name).await;
                continue;
            }

            // 429/503: rate limit / server temporary; honor Retry-After or back off
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS
                || status == reqwest::StatusCode::SERVICE_UNAVAILABLE
            {
                let delay = resp
                    .headers()
                    .get("retry-after")
                    .and_then(|h| h.to_str().ok()?.parse::<u64>().ok())
                    .unwrap_or(2u64.pow(attempt as u32 + 1));
                warn!(
                    "[{}] gmail {status} on {url} → sleeping {delay}s (attempt {})",
                    self.account.name,
                    attempt + 1
                );
                tokio::time::sleep(Duration::from_secs(delay)).await;
                continue;
            }

            let body = resp.text().await.unwrap_or_default();
            return Err(IoError::Imap(format!(
                "Gmail {status} on {url}: {body}"
            )));
        }
        Err(IoError::Imap(format!("Gmail: too many retries for {url}")))
    }

    async fn post_json<B: Serialize, T: for<'de> Deserialize<'de>>(
        &self,
        url: &str,
        body: &B,
    ) -> Result<T> {
        for attempt in 0..3 {
            let token = self.access_token().await?;
            let resp = self
                .http
                .post(url)
                .bearer_auth(&token)
                .json(body)
                .send()
                .await
                .map_err(|e| IoError::Imap(format!("POST {url}: {e}")))?;

            let status = resp.status();
            if status.is_success() {
                let body_bytes = resp.bytes().await.map_err(|e| IoError::Imap(e.to_string()))?;
                if body_bytes.is_empty() {
                    // Empty 2xx response → try to deserialize "null"
                    return serde_json::from_slice::<T>(b"null")
                        .map_err(|e| IoError::Parse(format!("Gmail parse empty: {e}")));
                }
                return serde_json::from_slice::<T>(&body_bytes).map_err(|e| {
                    IoError::Parse(format!(
                        "Gmail parse: {e} (body: {})",
                        String::from_utf8_lossy(&body_bytes)
                    ))
                });
            }

            if status == reqwest::StatusCode::UNAUTHORIZED && attempt == 0 {
                self.oauth_cache.clear(&self.account.name).await;
                continue;
            }
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS
                || status == reqwest::StatusCode::SERVICE_UNAVAILABLE
            {
                let delay = resp
                    .headers()
                    .get("retry-after")
                    .and_then(|h| h.to_str().ok()?.parse::<u64>().ok())
                    .unwrap_or(2u64.pow(attempt as u32 + 1));
                tokio::time::sleep(Duration::from_secs(delay)).await;
                continue;
            }

            let body_text = resp.text().await.unwrap_or_default();
            return Err(IoError::Imap(format!(
                "Gmail POST {status} on {url}: {body_text}"
            )));
        }
        Err(IoError::Imap(format!("Gmail: too many retries for POST {url}")))
    }

    // -----------------------------------------------------------------------
    // Synthetic UID ↔ Gmail message ID mapping
    // -----------------------------------------------------------------------

    async fn uid_for_msgid(&self, gmail_id: &str) -> u32 {
        // Fast path: already mapped
        if let Some(uid) = self.msgid_to_uid.read().await.get(gmail_id).copied() {
            return uid;
        }

        // Derive candidate from hex prefix
        let candidate = u32::from_str_radix(&gmail_id[..8.min(gmail_id.len())], 16)
            .unwrap_or_else(|_| {
                use std::hash::{Hash, Hasher};
                let mut h = std::collections::hash_map::DefaultHasher::new();
                gmail_id.hash(&mut h);
                h.finish() as u32
            });

        // Resolve collisions
        let mut state = self.state.write().await;
        let mut reverse = self.msgid_to_uid.write().await;
        let mut uid = candidate;
        loop {
            match state.uid_to_msgid.get(&uid) {
                Some(existing) if existing == gmail_id => break,
                Some(_) => {
                    uid = uid.wrapping_add(1);
                    continue;
                }
                None => {
                    state.uid_to_msgid.insert(uid, gmail_id.to_string());
                    reverse.insert(gmail_id.to_string(), uid);
                    let _ = state.save(&self.state_path); // best-effort
                    break;
                }
            }
        }
        uid
    }

    async fn msgid_for_uid(&self, uid: u32) -> Result<String> {
        self.state
            .read()
            .await
            .uid_to_msgid
            .get(&uid)
            .cloned()
            .ok_or_else(|| {
                IoError::NotFound(format!(
                    "Gmail UID {uid} non in mappa locale — è stato indicizzato in questa sessione?"
                ))
            })
    }

    // -----------------------------------------------------------------------
    // Labels cache + folder mapping
    // -----------------------------------------------------------------------

    /// Map IMAP-style folder name → Gmail label ID.
    async fn folder_to_label_id(&self, folder: &str) -> Result<String> {
        // System label shortcuts
        match folder {
            "INBOX" => return Ok("INBOX".into()),
            "[Gmail]/Sent Mail" => return Ok("SENT".into()),
            "[Gmail]/Trash" => return Ok("TRASH".into()),
            "[Gmail]/Drafts" => return Ok("DRAFT".into()),
            "[Gmail]/Spam" => return Ok("SPAM".into()),
            "[Gmail]/Starred" => return Ok("STARRED".into()),
            "[Gmail]/Important" => return Ok("IMPORTANT".into()),
            "[Gmail]/All Mail" => return Ok("".into()), // sentinel: no label filter
            _ => {}
        }
        // User label: strip prefix and look up by name
        let needle = folder.strip_prefix("[Gmail]/").unwrap_or(folder);
        let labels = self.labels_cached().await?;
        labels
            .iter()
            .find(|l| l.name.eq_ignore_ascii_case(needle))
            .map(|l| l.id.clone())
            .ok_or_else(|| IoError::NotFound(format!("Gmail folder '{folder}' non trovato")))
    }

    async fn labels_cached(&self) -> Result<Vec<Label>> {
        if let Some(c) = self.labels_cache.read().await.clone() {
            return Ok(c);
        }
        let url = format!("{GMAIL_API}/labels");
        let resp: LabelsList = self.get_json(&url).await?;
        let labels = resp.labels;
        *self.labels_cache.write().await = Some(labels.clone());
        Ok(labels)
    }

    async fn fetch_current_history_id(&self) -> Result<String> {
        let url = format!("{GMAIL_API}/profile");
        let p: ProfileResp = self.get_json(&url).await?;
        Ok(p.history_id)
    }

    // -----------------------------------------------------------------------
    // Email parsing (Gmail message → EmailData)
    // -----------------------------------------------------------------------

    fn parse_full_message(&self, msg: &MessageFull, synthetic_uid: u32) -> EmailData {
        // Prefer raw if available — pass it straight to mail-parser.
        if let Some(raw_b64) = &msg.raw {
            let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(raw_b64.as_bytes())
                .or_else(|_| {
                    base64::engine::general_purpose::URL_SAFE.decode(raw_b64.as_bytes())
                })
                .unwrap_or_default();
            if let Some(parsed) = mail_parser::MessageParser::default().parse(&raw[..]) {
                return self.parsed_to_emaildata(&parsed, synthetic_uid);
            }
        }
        // Otherwise reconstruct minimal EmailData from payload headers + body
        self.payload_to_emaildata(msg, synthetic_uid)
    }

    fn parsed_to_emaildata(
        &self,
        parsed: &mail_parser::Message<'_>,
        uid: u32,
    ) -> EmailData {
        let from = parsed
            .from()
            .and_then(|f| f.first())
            .map(|a| {
                let name = a.name().unwrap_or("");
                let addr = a.address().unwrap_or("");
                if name.is_empty() {
                    addr.to_string()
                } else {
                    format!("{name} <{addr}>")
                }
            })
            .unwrap_or_default();
        let to = parsed
            .to()
            .map(|l| l.iter().map(|a| a.address().unwrap_or("").to_string()).collect())
            .unwrap_or_default();
        let cc = parsed
            .cc()
            .map(|l| l.iter().map(|a| a.address().unwrap_or("").to_string()).collect())
            .unwrap_or_default();
        let subject = parsed.subject().unwrap_or("(nessun oggetto)").to_string();
        let date = parsed.date().map(|d| d.to_rfc3339()).unwrap_or_default();
        let message_id = parsed.message_id().unwrap_or("").to_string();
        let in_reply_to = parsed.in_reply_to().as_text().map(|s| s.to_string());
        let references = parsed.references().as_text_list().map(|l| {
            l.iter()
                .map(|s| s.as_ref() as &str)
                .collect::<Vec<&str>>()
                .join(" ")
        });
        let body = parsed
            .body_text(0)
            .map(|s| s.to_string())
            .or_else(|| parsed.body_html(0).map(|h| html_to_text(&h)))
            .unwrap_or_default();
        let body: String = body.chars().take(5000).collect();

        let attachments: Vec<AttachmentInfo> = parsed
            .attachments
            .iter()
            .enumerate()
            .map(|(idx, &pidx)| {
                let part = &parsed.parts[pidx];
                let filename = part
                    .attachment_name()
                    .unwrap_or("attachment")
                    .to_string();
                let content_type = part
                    .content_type()
                    .map(|ct| {
                        let main = ct.c_type.as_ref();
                        let sub = ct.c_subtype.as_deref().unwrap_or("octet-stream");
                        format!("{main}/{sub}")
                    })
                    .unwrap_or_else(|| "application/octet-stream".into());
                AttachmentInfo {
                    index: idx,
                    filename,
                    content_type,
                    size: part.contents().len(),
                }
            })
            .collect();

        EmailData {
            message_id,
            from,
            to,
            cc,
            subject,
            date,
            body,
            in_reply_to,
            references,
            attachments,
            uid: Some(uid),
        }
    }

    fn payload_to_emaildata(&self, msg: &MessageFull, uid: u32) -> EmailData {
        let payload = msg.payload.as_ref();
        let header = |name: &str| -> String {
            payload
                .and_then(|p| {
                    p.headers
                        .iter()
                        .find(|h| h.name.eq_ignore_ascii_case(name))
                        .map(|h| h.value.clone())
                })
                .unwrap_or_default()
        };

        let from = header("From");
        let to_raw = header("To");
        let cc_raw = header("Cc");
        let subject = header("Subject");
        let date = header("Date");
        let message_id = header("Message-ID");

        let to: Vec<String> = to_raw
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let cc: Vec<String> = cc_raw
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        // Extract plain text body + attachments from parts tree
        let mut body = String::new();
        let mut attachments = Vec::new();
        if let Some(p) = payload {
            collect_body_and_attachments(p, &mut body, &mut attachments);
        }
        let body: String = body.chars().take(5000).collect();

        EmailData {
            message_id,
            from,
            to,
            cc,
            subject,
            date,
            body,
            in_reply_to: None,
            references: None,
            attachments,
            uid: Some(uid),
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers: HTML → text, payload parts walker
// ---------------------------------------------------------------------------

fn html_to_text(html: &str) -> String {
    use regex::Regex;
    let tag_re = Regex::new(r"<[^>]+>").unwrap();
    let text = tag_re.replace_all(html, " ");
    let text = text
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"");
    let ws_re = Regex::new(r"\s+").unwrap();
    ws_re.replace_all(&text, " ").trim().to_string()
}

/// Recursively walk Gmail payload parts: extract text body + attachment info.
fn collect_body_and_attachments(
    part: &MessagePart,
    body: &mut String,
    attachments: &mut Vec<AttachmentInfo>,
) {
    if !part.filename.is_empty() {
        let size = part.body.as_ref().map(|b| b.size).unwrap_or(0) as usize;
        let idx = attachments.len();
        attachments.push(AttachmentInfo {
            index: idx,
            filename: part.filename.clone(),
            content_type: if part.mime_type.is_empty() {
                "application/octet-stream".into()
            } else {
                part.mime_type.clone()
            },
            size,
        });
        return;
    }
    if part.mime_type == "text/plain" {
        if let Some(b) = &part.body {
            if let Some(data) = &b.data {
                if let Ok(decoded) = base64::engine::general_purpose::URL_SAFE_NO_PAD
                    .decode(data.as_bytes())
                    .or_else(|_| {
                        base64::engine::general_purpose::URL_SAFE.decode(data.as_bytes())
                    })
                {
                    if let Ok(s) = std::str::from_utf8(&decoded) {
                        if !body.is_empty() {
                            body.push('\n');
                        }
                        body.push_str(s);
                    }
                }
            }
        }
        return;
    }
    if part.mime_type == "text/html" && body.is_empty() {
        if let Some(b) = &part.body {
            if let Some(data) = &b.data {
                if let Ok(decoded) = base64::engine::general_purpose::URL_SAFE_NO_PAD
                    .decode(data.as_bytes())
                    .or_else(|_| {
                        base64::engine::general_purpose::URL_SAFE.decode(data.as_bytes())
                    })
                {
                    if let Ok(html) = std::str::from_utf8(&decoded) {
                        body.push_str(&html_to_text(html));
                    }
                }
            }
        }
        return;
    }
    for sub in &part.parts {
        collect_body_and_attachments(sub, body, attachments);
    }
}

// ---------------------------------------------------------------------------
// MailBackend impl
// ---------------------------------------------------------------------------

#[async_trait]
impl MailBackend for GmailApiBackend {
    fn account_name(&self) -> &str {
        &self.account.name
    }

    fn sent_folder(&self) -> &str {
        &self.account.sent_folder
    }

    fn backend_kind(&self) -> &'static str {
        "gmail-api"
    }

    // ----- list_folders -----

    async fn list_folders(&self) -> Result<Vec<String>> {
        let labels = self.labels_cached().await?;
        let mut out = Vec::new();
        for l in &labels {
            match (l.label_type.as_str(), l.id.as_str()) {
                ("system", "INBOX") => out.push("INBOX".into()),
                ("system", "SENT") => out.push("[Gmail]/Sent Mail".into()),
                ("system", "DRAFT") => out.push("[Gmail]/Drafts".into()),
                ("system", "TRASH") => out.push("[Gmail]/Trash".into()),
                ("system", "SPAM") => out.push("[Gmail]/Spam".into()),
                ("system", "STARRED") => out.push("[Gmail]/Starred".into()),
                ("system", "IMPORTANT") => out.push("[Gmail]/Important".into()),
                ("system", _) => { /* CHAT, CATEGORY_* — ignored */ }
                ("user", _) => out.push(format!("[Gmail]/{}", l.name)),
                _ => {}
            }
        }
        info!(
            "[{}] gmail-api list_folders: {} folders",
            self.account.name,
            out.len()
        );
        Ok(out)
    }

    // ----- fetch_uids_since -----

    async fn fetch_uids_since(&self, folder: &str, since_uid: u32) -> Result<Vec<u32>> {
        let label_id = self.folder_to_label_id(folder).await?;

        // Incremental path: history.list (requires previous historyId for this folder)
        if since_uid > 0 {
            if let Some(start) = self.state.read().await.last_history_id.get(folder).cloned() {
                match self.fetch_via_history(&label_id, folder, &start).await {
                    Ok(uids) => return Ok(uids),
                    Err(e) => {
                        warn!(
                            "[{}] gmail history.list failed ({e}) → fallback messages.list",
                            self.account.name
                        );
                    }
                }
            }
        }

        // Initial / fallback path: messages.list with date filter
        let q = if since_uid == 0 {
            "newer_than:365d"
        } else {
            "newer_than:30d"
        };
        let uids = self.fetch_via_messages_list(&label_id, q).await?;

        // Snapshot current historyId so subsequent runs can use history.list
        if let Ok(hist) = self.fetch_current_history_id().await {
            let mut st = self.state.write().await;
            st.last_history_id.insert(folder.to_string(), hist);
            let _ = st.save(&self.state_path);
        }

        info!(
            "[{}] gmail-api fetch_uids_since {folder} (q={q}): {} ids",
            self.account.name,
            uids.len()
        );
        Ok(uids)
    }

    async fn fetch_emails_by_uids(
        &self,
        _folder: &str,
        uids: &[u32],
    ) -> Result<Vec<EmailData>> {
        // Sequential messages.get?format=raw with bounded concurrency.
        // (Multipart batchGet is more efficient but trickier to parse — start
        // simple with bounded parallel single calls; can be optimised later.)
        if uids.is_empty() {
            return Ok(vec![]);
        }
        use futures_util::stream::{self, StreamExt};

        let concurrency = 8;
        let stream = stream::iter(uids.to_vec()).map(|uid| async move {
            let gmail_id = self.msgid_for_uid(uid).await.ok()?;
            let url = format!("{GMAIL_API}/messages/{gmail_id}?format=raw");
            let msg = match self.get_json::<MessageFull>(&url).await {
                Ok(m) => m,
                Err(e) => {
                    warn!("[{}] gmail get {gmail_id}: {e}", self.account.name);
                    return None;
                }
            };
            Some(self.parse_full_message(&msg, uid))
        });

        let results: Vec<EmailData> = stream
            .buffer_unordered(concurrency)
            .filter_map(|x| async move { x })
            .collect()
            .await;

        Ok(results)
    }

    async fn search_uids_filtered(
        &self,
        folder: &str,
        sender: Option<&str>,
        subject_contains: Option<&str>,
        before: Option<&str>,
    ) -> Result<Vec<u32>> {
        let label_id = self.folder_to_label_id(folder).await?;
        let mut q_parts: Vec<String> = Vec::new();
        if let Some(s) = sender {
            let safe = s.replace('"', "");
            q_parts.push(format!("from:{safe}"));
        }
        if let Some(s) = subject_contains {
            let safe = s.replace('"', "");
            q_parts.push(format!("subject:{safe}"));
        }
        if let Some(d) = before {
            // YYYY-MM-DD → YYYY/MM/DD
            let gmail_date = d.replace('-', "/");
            q_parts.push(format!("before:{gmail_date}"));
        }
        let q = q_parts.join(" ");
        self.fetch_via_messages_list(&label_id, &q).await
    }

    // ----- attachments -----

    async fn get_attachments(
        &self,
        email_uid: u32,
        _folder: &str,
    ) -> Result<Vec<AttachmentInfo>> {
        let gmail_id = self.msgid_for_uid(email_uid).await?;
        let url = format!("{GMAIL_API}/messages/{gmail_id}?format=full");
        let msg: MessageFull = self.get_json(&url).await?;
        let mut body = String::new();
        let mut attachments = Vec::new();
        if let Some(p) = &msg.payload {
            collect_body_and_attachments(p, &mut body, &mut attachments);
        }
        Ok(attachments)
    }

    async fn download_attachment(
        &self,
        email_uid: u32,
        attachment_index: usize,
        _folder: &str,
    ) -> Result<AttachmentData> {
        let gmail_id = self.msgid_for_uid(email_uid).await?;
        let url = format!("{GMAIL_API}/messages/{gmail_id}?format=full");
        let msg: MessageFull = self.get_json(&url).await?;

        // Walk payload parts in the same order used by `get_attachments` and
        // pick the one at `attachment_index`.
        let mut parts_flat: Vec<MessagePart> = Vec::new();
        if let Some(p) = &msg.payload {
            flatten_attachment_parts(p, &mut parts_flat);
        }
        let part = parts_flat.get(attachment_index).ok_or_else(|| {
            IoError::NotFound(format!(
                "Attachment index {attachment_index} non trovato (totali: {})",
                parts_flat.len()
            ))
        })?;

        let filename = part.filename.clone();
        let content_type = if part.mime_type.is_empty() {
            "application/octet-stream".into()
        } else {
            part.mime_type.clone()
        };

        // Body data may be inline (small) or require a separate fetch via attachmentId
        let data: Vec<u8> = if let Some(b) = &part.body {
            if let Some(inline) = &b.data {
                decode_b64url(inline)
            } else if let Some(att_id) = &b.attachment_id {
                let url = format!(
                    "{GMAIL_API}/messages/{gmail_id}/attachments/{att_id}"
                );
                let a: AttachmentResp = self.get_json(&url).await?;
                a.data.as_deref().map(decode_b64url).unwrap_or_default()
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };

        Ok(AttachmentData {
            filename,
            content_type,
            data,
        })
    }

    // ----- delete -----

    async fn move_to_trash(&self, _source_folder: &str, uid: u32) -> Result<String> {
        let gmail_id = self.msgid_for_uid(uid).await?;
        let url = format!("{GMAIL_API}/messages/{gmail_id}/trash");
        // POST with empty body
        let _: serde_json::Value = self.post_json(&url, &serde_json::json!({})).await?;
        info!(
            "[{}] gmail-api trashed {gmail_id} (uid={uid})",
            self.account.name
        );
        Ok("[Gmail]/Trash".to_string())
    }

    async fn bulk_move_to_trash(
        &self,
        _source_folder: &str,
        uids: &[u32],
    ) -> Result<(usize, String)> {
        if uids.is_empty() {
            return Ok((0, "[Gmail]/Trash".into()));
        }
        // Resolve all gmail message ids
        let mut ids: Vec<String> = Vec::with_capacity(uids.len());
        for u in uids {
            if let Ok(id) = self.msgid_for_uid(*u).await {
                ids.push(id);
            }
        }
        // batchModify supports up to 1000 IDs per request
        let mut moved = 0usize;
        for chunk in ids.chunks(1000) {
            let url = format!("{GMAIL_API}/messages/batchModify");
            let body = serde_json::json!({
                "ids": chunk,
                "addLabelIds": ["TRASH"],
                "removeLabelIds": ["INBOX"],
            });
            let _: serde_json::Value = self.post_json(&url, &body).await?;
            moved += chunk.len();
        }
        info!(
            "[{}] gmail-api bulk_move_to_trash: {} moved",
            self.account.name, moved
        );
        Ok((moved, "[Gmail]/Trash".into()))
    }

    // ----- send -----

    async fn send_email(&self, params: SendEmailParams) -> Result<SendResult> {
        let raw_b64 = self.build_raw_b64url(&params).await?;
        self.post_send_raw(raw_b64, None).await?;
        info!(
            "[{}] gmail-api send to {:?}, subject: {}",
            self.account.name, params.to, params.subject
        );
        Ok(SendResult {
            success: true,
            to: params.to,
            subject: params.subject,
            saved_to_sent: true, // Gmail auto-puts sent mail in [Gmail]/Sent
            error: None,
        })
    }

    async fn reply_email(
        &self,
        email_uid: u32,
        _folder: &str,
        body: &str,
        cc: Vec<String>,
        signature_name: &str,
        attachments: Vec<PathBuf>,
    ) -> Result<SendResult> {
        // Fetch original (raw) to get threadId + headers in one call.
        let gmail_id = self.msgid_for_uid(email_uid).await?;
        let url = format!("{GMAIL_API}/messages/{gmail_id}?format=raw");
        let msg: MessageFull = self.get_json(&url).await?;
        let thread_id = msg.thread_id.clone();
        let original = self.parse_full_message(&msg, email_uid);

        // Build References chain: existing + original Message-ID.
        let references = {
            let mut refs = original.references.unwrap_or_default();
            if !original.message_id.is_empty() {
                if !refs.is_empty() {
                    refs.push(' ');
                }
                refs.push_str(&original.message_id);
            }
            refs
        };

        let re_regex = regex::Regex::new(r"(?i)^re\s*:\s*").unwrap();
        let subject = if re_regex.is_match(&original.subject) {
            original.subject.clone()
        } else {
            format!("RE: {}", original.subject)
        };

        let reply_to = email_client::extract_email_address(&original.from);

        let params = SendEmailParams {
            to: vec![reply_to],
            subject,
            body: body.to_string(),
            cc,
            from_name: Some(" ".to_string()),
            attachments,
            signature_name: signature_name.to_string(),
            in_reply_to: Some(original.message_id.clone()),
            references: Some(references),
        };

        let raw_b64 = self.build_raw_b64url(&params).await?;
        self.post_send_raw(raw_b64, Some(thread_id.clone())).await?;
        info!(
            "[{}] gmail-api reply to {:?} (thread={})",
            self.account.name, params.to, thread_id
        );

        Ok(SendResult {
            success: true,
            to: params.to,
            subject: params.subject,
            saved_to_sent: true,
            error: None,
        })
    }

    async fn check_bounced(
        &self,
        folder: &str,
        limit: usize,
    ) -> Result<Vec<BounceInfo>> {
        let label_id = self.folder_to_label_id(folder).await?;

        // Gmail search query covering common bounce sources.
        let q = r#"from:mailer-daemon OR from:postmaster OR subject:"Delivery Status Notification" OR subject:"Undelivered Mail Returned" OR subject:"Mail Delivery Failed" OR subject:"Failure Notice""#;
        let q_enc = urlencoding::encode(q).to_string();

        let max_results = limit.clamp(1, 500);
        let mut url = format!("{GMAIL_API}/messages?maxResults={max_results}&q={q_enc}");
        if !label_id.is_empty() {
            url.push_str(&format!("&labelIds={label_id}"));
        }

        let resp: MessagesList = self.get_json(&url).await?;
        let ids: Vec<String> = resp.messages.into_iter().map(|m| m.id).collect();
        if ids.is_empty() {
            return Ok(vec![]);
        }

        let email_regex = regex::Regex::new(r"[\w\.\-]+@[\w\.\-]+\.\w+").unwrap();
        let self_addr_lower = self.account.imap_username.to_lowercase();
        let mut bounces = Vec::new();
        let mut seen_ids = std::collections::HashSet::new();

        for id in ids.iter().take(limit) {
            let url = format!("{GMAIL_API}/messages/{id}?format=raw");
            let msg: MessageFull = match self.get_json(&url).await {
                Ok(m) => m,
                Err(e) => {
                    warn!(
                        "[{}] gmail bounce fetch {id}: {e}",
                        self.account.name
                    );
                    continue;
                }
            };

            let Some(raw_b64) = msg.raw.as_ref() else {
                continue;
            };
            let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(raw_b64.as_bytes())
                .or_else(|_| {
                    base64::engine::general_purpose::URL_SAFE.decode(raw_b64.as_bytes())
                })
                .unwrap_or_default();

            let parsed = match mail_parser::MessageParser::default().parse(&raw[..]) {
                Some(p) => p,
                None => continue,
            };

            let message_id = parsed.message_id().unwrap_or("").to_string();
            if !seen_ids.insert(message_id.clone()) {
                continue;
            }

            let body_text = parsed.body_text(0).unwrap_or_default().to_string();
            if let Some(failed_match) = email_regex.find(&body_text) {
                let failed = failed_match.as_str().to_string();
                if failed.to_lowercase() == self_addr_lower {
                    continue;
                }
                let reason = email_client::classify_bounce_reason(&body_text);
                let date = parsed
                    .date()
                    .map(|d| d.to_rfc3339())
                    .unwrap_or_default();
                bounces.push(BounceInfo {
                    message_id,
                    failed_email: failed,
                    reason,
                    date,
                });
            }
        }

        info!(
            "[{}] gmail-api check_bounced({folder}): {} bounces in {} scanned",
            self.account.name,
            bounces.len(),
            ids.len().min(limit)
        );
        Ok(bounces)
    }

    // ----- watcher (polling history.list every 60s) -----

    async fn watch_once(
        &self,
        folder: &str,
        timeout_secs: u64,
        on_new_mail: NewMailCallback,
    ) -> Result<()> {
        let label_id = self.folder_to_label_id(folder).await?;
        let start = std::time::Instant::now();

        loop {
            if start.elapsed().as_secs() >= timeout_secs {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_secs(60)).await;

            let last_hist = self.state.read().await.last_history_id.get(folder).cloned();
            let Some(hist) = last_hist else {
                // No baseline yet — bail out so daemon does a normal bulk pass.
                return Ok(());
            };

            let mut url = format!(
                "{GMAIL_API}/history?startHistoryId={hist}&historyTypes=messageAdded"
            );
            if !label_id.is_empty() {
                url.push_str(&format!("&labelId={label_id}"));
            }

            match self.get_json::<HistoryResp>(&url).await {
                Ok(r) if !r.history.is_empty() => {
                    info!(
                        "[{}] gmail polling: {} history records on {folder}",
                        self.account.name,
                        r.history.len()
                    );
                    if let Some(new_hist) = r.history_id {
                        let mut st = self.state.write().await;
                        st.last_history_id.insert(folder.to_string(), new_hist);
                        let _ = st.save(&self.state_path);
                    }
                    on_new_mail().await;
                    return Ok(()); // let the daemon loop re-call us
                }
                Ok(r) => {
                    // Nothing new — update history baseline anyway
                    if let Some(new_hist) = r.history_id {
                        let mut st = self.state.write().await;
                        st.last_history_id.insert(folder.to_string(), new_hist);
                        let _ = st.save(&self.state_path);
                    }
                }
                Err(e) => {
                    let es = e.to_string();
                    if es.contains("404") {
                        // historyId expired (>7d). Reset baseline, ask daemon to do a bulk pass.
                        warn!(
                            "[{}] gmail history expired on {folder} — full re-sync next cycle",
                            self.account.name
                        );
                        self.state
                            .write()
                            .await
                            .last_history_id
                            .remove(folder);
                        let _ = self.state.read().await.save(&self.state_path);
                        on_new_mail().await;
                        return Ok(());
                    }
                    warn!("[{}] gmail polling error on {folder}: {e}", self.account.name);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Internal helpers for GmailApiBackend
// ---------------------------------------------------------------------------

impl GmailApiBackend {
    async fn fetch_via_messages_list(&self, label_id: &str, q: &str) -> Result<Vec<u32>> {
        let mut uids = Vec::new();
        let mut page_token: Option<String> = None;
        let q_enc = urlencoding::encode(q).to_string();

        loop {
            let mut url = format!("{GMAIL_API}/messages?maxResults=500");
            if !q.is_empty() {
                url.push_str(&format!("&q={q_enc}"));
            }
            if !label_id.is_empty() {
                url.push_str(&format!("&labelIds={label_id}"));
            }
            if let Some(t) = &page_token {
                url.push_str(&format!("&pageToken={t}"));
            }

            let resp: MessagesList = self.get_json(&url).await?;
            for m in resp.messages {
                let uid = self.uid_for_msgid(&m.id).await;
                uids.push(uid);
            }
            page_token = resp.next_page_token;
            if page_token.is_none() {
                break;
            }
        }
        Ok(uids)
    }

    /// Build an RFC 822 message (with auto-appended signature + attachments)
    /// via lettre, then base64url-encode it for Gmail's `messages.send`.
    async fn build_raw_b64url(&self, params: &SendEmailParams) -> Result<String> {
        use lettre::message::header::ContentType;
        use lettre::message::{Attachment, MultiPart, SinglePart};
        use lettre::Message;

        // Load signature (HTML or plain) and assemble body parts.
        // Per-account first: a signature stored under the account's name wins
        // over the generic `signature_name`. An empty entry under the account
        // name is an explicit opt-out (no signature at all), NOT a fallback
        // trigger — lets the user keep "default" for Aruba but no signature
        // on Gmail.
        let sig_content =
            match signature::get_signature(&self.signature_dir, &self.account.name) {
                Some(s) if s.trim().is_empty() => None,
                Some(s) => Some(s),
                None => signature::get_signature(&self.signature_dir, &params.signature_name),
            };
        let has_html_sig = sig_content
            .as_ref()
            .map(|s| s.trim_start().starts_with('<'))
            .unwrap_or(false);

        let (plain_body, html_body) = if let Some(ref sig) = sig_content {
            if has_html_sig {
                let plain = format!(
                    "{}\n\n-- \n(firma HTML non visualizzabile in testo)",
                    params.body
                );
                let html = format!(
                    "<div style=\"font-family: Roboto, Arial, sans-serif; font-size: 12pt; line-height: 1.5;\">{}</div><br>{sig}",
                    params.body.replace('\n', "<br>")
                );
                (plain, Some(html))
            } else {
                let plain = format!("{}\n\n-- \n{sig}", params.body);
                (plain, None)
            }
        } else {
            (params.body.clone(), None)
        };

        let from_name = params
            .from_name
            .clone()
            .unwrap_or_else(|| " ".to_string());
        let from_addr = format!("{} <{}>", from_name, self.account.imap_username);

        let mut builder = Message::builder()
            .from(
                from_addr
                    .parse()
                    .map_err(|e| IoError::Smtp(format!("From: {e}")))?,
            )
            .subject(&params.subject);

        for to in &params.to {
            builder =
                builder.to(to.parse().map_err(|e| IoError::Smtp(format!("To: {e}")))?);
        }
        for cc in &params.cc {
            builder =
                builder.cc(cc.parse().map_err(|e| IoError::Smtp(format!("Cc: {e}")))?);
        }
        if let Some(ref rt) = params.in_reply_to {
            builder = builder.in_reply_to(rt.to_string());
        }
        if let Some(ref r) = params.references {
            builder = builder.references(r.to_string());
        }

        let text_part = SinglePart::builder()
            .content_type(ContentType::TEXT_PLAIN)
            .body(plain_body.clone());

        let body_multipart = if let Some(ref html) = html_body {
            let html_part = SinglePart::builder()
                .content_type(ContentType::TEXT_HTML)
                .body(html.clone());
            MultiPart::alternative()
                .singlepart(text_part)
                .singlepart(html_part)
        } else {
            MultiPart::alternative().singlepart(text_part)
        };

        let final_multipart = if params.attachments.is_empty() {
            body_multipart
        } else {
            let mut mixed = MultiPart::mixed().multipart(body_multipart);
            for attach_path in &params.attachments {
                if !attach_path.exists() {
                    warn!("Attachment not found: {}", attach_path.display());
                    continue;
                }
                let filename = attach_path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                let data = tokio::fs::read(attach_path).await.map_err(IoError::Io)?;
                let content_type = ContentType::parse("application/octet-stream")
                    .unwrap_or(ContentType::TEXT_PLAIN);
                let attachment = Attachment::new(filename).body(data, content_type);
                mixed = mixed.singlepart(attachment);
            }
            mixed
        };

        let email_msg = builder
            .multipart(final_multipart)
            .map_err(|e| IoError::Smtp(format!("Build message: {e}")))?;

        let raw = email_msg.formatted();
        Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&raw))
    }

    /// POST the encoded message to `messages.send`, optionally pinning a thread.
    async fn post_send_raw(
        &self,
        raw_b64: String,
        thread_id: Option<String>,
    ) -> Result<()> {
        let url = format!("{GMAIL_API}/messages/send");
        let body = match thread_id {
            Some(t) if !t.is_empty() => {
                serde_json::json!({ "raw": raw_b64, "threadId": t })
            }
            _ => serde_json::json!({ "raw": raw_b64 }),
        };
        let _: serde_json::Value = self.post_json(&url, &body).await?;
        Ok(())
    }

    async fn fetch_via_history(
        &self,
        label_id: &str,
        _folder: &str,
        start_history: &str,
    ) -> Result<Vec<u32>> {
        let mut url = format!(
            "{GMAIL_API}/history?startHistoryId={start_history}&historyTypes=messageAdded"
        );
        if !label_id.is_empty() {
            url.push_str(&format!("&labelId={label_id}"));
        }
        let resp: HistoryResp = self.get_json(&url).await?;
        let mut uids = Vec::new();
        for rec in resp.history {
            for added in rec.messages_added {
                uids.push(self.uid_for_msgid(&added.message.id).await);
            }
        }
        Ok(uids)
    }
}

fn flatten_attachment_parts(part: &MessagePart, out: &mut Vec<MessagePart>) {
    if !part.filename.is_empty() {
        out.push(part.clone());
        return;
    }
    for sub in &part.parts {
        flatten_attachment_parts(sub, out);
    }
}

fn decode_b64url(s: &str) -> Vec<u8> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(s.as_bytes())
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(s.as_bytes()))
        .unwrap_or_default()
}
