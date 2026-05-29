//! Microsoft Teams adapter using the Microsoft Graph API.
//!
//! Auth = OAuth2 (public desktop client). The GUI runs the consent flow and stores
//! a `refresh_token` in the keyring; this adapter exchanges it for short-lived
//! access tokens at runtime. Requires an Azure app registration (client id +
//! delegated Graph permissions `Chat.ReadWrite`, `offline_access`, `User.Read`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::Utc;
use jupiteros_shared::store::{IndexResult, MessageData, MessageStore};
use serde_json::{json, Value};
use tracing::{info, warn};

use super::{Contact, ContactType, Entity, MessagingAdapter};
use crate::config::Config;
use crate::error::{EuropaError, Result};

const GRAPH: &str = "https://graph.microsoft.com/v1.0";

pub struct TeamsAdapter {
    client_id: String,
    tenant: String,
    refresh_token: String,
    http: reqwest::Client,
    /// Cached (access_token, expiry).
    token: Mutex<Option<(String, Instant)>>,
    /// chat id -> last indexed createdDateTime (for the polling listener).
    last_seen: Mutex<HashMap<String, String>>,
}

impl TeamsAdapter {
    pub fn new(config: Config) -> Self {
        Self {
            client_id: config.teams_client_id.clone().unwrap_or_default(),
            tenant: config.teams_tenant_id.clone().unwrap_or_else(|| "common".into()),
            refresh_token: config.teams_refresh_token.clone().unwrap_or_default(),
            http: reqwest::Client::new(),
            token: Mutex::new(None),
            last_seen: Mutex::new(HashMap::new()),
        }
    }

    /// Return a valid access token, refreshing via the refresh_token when needed.
    async fn access_token(&self) -> Result<String> {
        if let Some((tok, exp)) = self.token.lock().unwrap().as_ref() {
            if *exp > Instant::now() {
                return Ok(tok.clone());
            }
        }
        let url = format!(
            "https://login.microsoftonline.com/{}/oauth2/v2.0/token",
            self.tenant
        );
        let resp = self
            .http
            .post(&url)
            .form(&[
                ("client_id", self.client_id.as_str()),
                ("grant_type", "refresh_token"),
                ("refresh_token", self.refresh_token.as_str()),
                ("scope", "offline_access Chat.ReadWrite User.Read"),
            ])
            .send()
            .await
            .map_err(|e| EuropaError::Adapter(format!("Teams token: {e}")))?;
        let body: Value = resp
            .json()
            .await
            .map_err(|e| EuropaError::Adapter(format!("Teams token decode: {e}")))?;
        let access = body
            .get("access_token")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                let err = body
                    .get("error_description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("no access_token");
                EuropaError::Adapter(format!("Teams token refresh failed: {err}"))
            })?
            .to_string();
        let expires_in = body.get("expires_in").and_then(|v| v.as_u64()).unwrap_or(3600);
        let exp = Instant::now() + Duration::from_secs(expires_in.saturating_sub(60));
        *self.token.lock().unwrap() = Some((access.clone(), exp));
        Ok(access)
    }

    async fn graph_get(&self, path: &str) -> Result<Value> {
        let token = self.access_token().await?;
        let resp = self
            .http
            .get(format!("{GRAPH}{path}"))
            .bearer_auth(token)
            .send()
            .await
            .map_err(|e| EuropaError::Adapter(format!("Graph GET {path}: {e}")))?;
        resp.json()
            .await
            .map_err(|e| EuropaError::Adapter(format!("Graph GET {path} decode: {e}")))
    }

    fn chat_label(chat: &Value) -> String {
        chat.get("topic")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                let id = chat.get("id").and_then(|v| v.as_str()).unwrap_or("chat");
                format!("chat:{id}")
            })
    }

    async fn list_chats(&self) -> Result<Vec<(String, String)>> {
        let body = self.graph_get("/me/chats?$top=50").await?;
        let mut out = Vec::new();
        if let Some(arr) = body.get("value").and_then(|v| v.as_array()) {
            for chat in arr {
                if let Some(id) = chat.get("id").and_then(|v| v.as_str()) {
                    out.push((id.to_string(), Self::chat_label(chat)));
                }
            }
        }
        Ok(out)
    }

    fn message_to_data(chat_id: &str, chat_label: &str, msg: &Value) -> Option<MessageData> {
        let text = msg
            .get("body")
            .and_then(|b| b.get("content"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        // Strip basic HTML tags Graph returns for html content.
        let text = strip_html(text);
        if text.trim().is_empty() {
            return None;
        }
        let id = msg.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let sender = msg
            .get("from")
            .and_then(|f| f.get("user"))
            .and_then(|u| u.get("displayName"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        let date = msg
            .get("createdDateTime")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        Some(MessageData {
            id: format!("teams_{chat_id}_{id}"),
            text,
            sender,
            recipient: chat_label.to_string(),
            subject: String::new(),
            date: if date.is_empty() { Utc::now().to_rfc3339() } else { date },
            channel: "teams".to_string(),
            folder: String::new(),
            imap_uid: None,
            account: String::new(),
        })
    }

    async fn index_chat(&self, store: &MessageStore, chat_id: &str, label: &str) -> Result<usize> {
        let body = self
            .graph_get(&format!("/chats/{chat_id}/messages?$top=50"))
            .await?;
        let last = self.last_seen.lock().unwrap().get(chat_id).cloned().unwrap_or_default();
        let mut indexed = 0;
        let mut newest = last.clone();
        if let Some(arr) = body.get("value").and_then(|v| v.as_array()) {
            for m in arr {
                let created = m.get("createdDateTime").and_then(|v| v.as_str()).unwrap_or("");
                if !last.is_empty() && created <= last.as_str() {
                    continue;
                }
                if created > newest.as_str() {
                    newest = created.to_string();
                }
                if let Some(data) = Self::message_to_data(chat_id, label, m) {
                    match store.index_message(data).await {
                        Ok(true) => indexed += 1,
                        Ok(false) => {}
                        Err(e) => warn!("Teams index failed: {e}"),
                    }
                }
            }
        }
        if !newest.is_empty() {
            self.last_seen.lock().unwrap().insert(chat_id.to_string(), newest);
        }
        Ok(indexed)
    }
}

/// Very small HTML-to-text helper (Graph message bodies are often html).
fn strip_html(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut in_tag = false;
    for c in input.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&nbsp;", " ").replace("&amp;", "&").trim().to_string()
}

#[async_trait]
impl MessagingAdapter for TeamsAdapter {
    fn channel_name(&self) -> &str {
        "teams"
    }

    async fn connect(&mut self) -> Result<()> {
        if self.client_id.is_empty() || self.refresh_token.is_empty() {
            return Err(EuropaError::Config(
                "Teams not configured (need client_id + refresh_token)".into(),
            ));
        }
        let _ = self.access_token().await?;
        info!("Connected to Microsoft Teams (Graph)");
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<()> {
        Ok(())
    }

    async fn list_contacts(&self) -> Result<Vec<Contact>> {
        let chats = self.list_chats().await?;
        Ok(chats
            .into_iter()
            .map(|(id, label)| Contact {
                name: label,
                id: 0,
                contact_type: ContactType::Group,
                username: Some(id),
            })
            .collect())
    }

    async fn send_message(
        &self,
        to: &str,
        message: &str,
        _attachments: &[PathBuf],
    ) -> Result<String> {
        if message.is_empty() {
            return Err(EuropaError::Adapter("Message text is empty".into()));
        }
        let token = self.access_token().await?;
        let resp = self
            .http
            .post(format!("{GRAPH}/chats/{to}/messages"))
            .bearer_auth(token)
            .json(&json!({ "body": { "content": message } }))
            .send()
            .await
            .map_err(|e| EuropaError::Adapter(format!("Teams send: {e}")))?;
        let body: Value = resp
            .json()
            .await
            .map_err(|e| EuropaError::Adapter(format!("Teams send decode: {e}")))?;
        match body.get("id").and_then(|v| v.as_str()) {
            Some(id) => Ok(format!("Sent to chat {to} (msg_id: {id})")),
            None => {
                let err = body
                    .get("error")
                    .and_then(|e| e.get("message"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown error");
                Err(EuropaError::Adapter(format!("Teams send failed: {err}")))
            }
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
        let msg = MessageData {
            id: format!("teams_sent_{}", Utc::now().timestamp_millis()),
            text: message.to_string(),
            sender: "me".to_string(),
            recipient: to.to_string(),
            subject: String::new(),
            date: Utc::now().to_rfc3339(),
            channel: "teams".to_string(),
            folder: String::new(),
            imap_uid: None,
            account: String::new(),
        };
        if let Err(e) = store.index_message(msg).await {
            warn!("Failed to index sent Teams message: {e}");
        }
        Ok(result)
    }

    async fn save_media(
        &self,
        _chat: &str,
        _message_id: Option<i64>,
        _last_n: usize,
        _save_dir: &Path,
    ) -> Result<Vec<PathBuf>> {
        Err(EuropaError::Adapter("save_media not yet supported for Teams".into()))
    }

    async fn resolve_entities(&self) -> Result<Vec<Entity>> {
        let chats = self.list_chats().await?;
        Ok(chats
            .into_iter()
            .map(|(id, label)| Entity {
                id: 0,
                name: format!("{label}|{id}"),
                entity_type: ContactType::Group,
            })
            .collect())
    }

    async fn bulk_index(&self, store: &MessageStore, entity: &Entity) -> Result<IndexResult> {
        let (label, id) = entity.name.rsplit_once('|').unwrap_or((entity.name.as_str(), ""));
        let indexed = self.index_chat(store, id, label).await?;
        Ok(IndexResult { indexed, skipped: 0, errors: 0 })
    }

    async fn start_listener(&self, store: &MessageStore, entities: &[Entity]) -> Result<()> {
        info!("Teams listener started (polling Graph messages)");
        loop {
            for entity in entities {
                let (label, id) = entity.name.rsplit_once('|').unwrap_or((entity.name.as_str(), ""));
                if let Err(e) = self.index_chat(store, id, label).await {
                    warn!("Teams poll '{}' failed: {e}", entity.name);
                }
            }
            tokio::time::sleep(Duration::from_secs(60)).await;
        }
    }
}
