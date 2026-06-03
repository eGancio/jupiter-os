// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

//! Slack adapter using the Slack Web API (https://api.slack.com/web).
//!
//! Auth = a bot token (`xoxb-...`) created from https://api.slack.com/apps
//! (OAuth scopes: `channels:read`, `groups:read`, `chat:write`, `channels:history`,
//! `groups:history`, `users:read`). No interactive login — just paste the token.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use jupiteros_shared::store::{IndexResult, MessageData, MessageStore};
use serde::de::DeserializeOwned;
use serde_json::Value;
use tracing::{info, warn};

use super::{Contact, ContactType, Entity, MessagingAdapter};
use crate::config::Config;
use crate::error::{EuropaError, Result};

pub struct SlackAdapter {
    token: String,
    http: reqwest::Client,
    /// channel name -> id, resolved from conversations.list (for send_message).
    name_to_id: Mutex<HashMap<String, String>>,
    /// channel id -> last indexed message ts (for the polling listener).
    last_ts: Mutex<HashMap<String, String>>,
}

impl SlackAdapter {
    pub fn new(config: Config) -> Self {
        Self {
            token: config.slack_bot_token.clone().unwrap_or_default(),
            http: reqwest::Client::new(),
            name_to_id: Mutex::new(HashMap::new()),
            last_ts: Mutex::new(HashMap::new()),
        }
    }

    /// POST a Slack Web API method (form-encoded body) and return the envelope.
    async fn call<T: DeserializeOwned>(&self, method: &str, form: &[(&str, &str)]) -> Result<T> {
        let url = format!("https://slack.com/api/{method}");
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&self.token)
            .form(form)
            .send()
            .await
            .map_err(|e| EuropaError::Adapter(format!("Slack {method}: {e}")))?;
        let env: Value = resp
            .json()
            .await
            .map_err(|e| EuropaError::Adapter(format!("Slack {method} decode: {e}")))?;
        if env.get("ok").and_then(|v| v.as_bool()) != Some(true) {
            let err = env.get("error").and_then(|v| v.as_str()).unwrap_or("unknown");
            return Err(EuropaError::Adapter(format!("Slack {method}: {err}")));
        }
        serde_json::from_value(env)
            .map_err(|e| EuropaError::Adapter(format!("Slack {method} parse: {e}")))
    }

    /// List all conversations (channels) the bot can see.
    async fn conversations(&self) -> Result<Vec<(String, String)>> {
        let env: Value = self
            .call(
                "conversations.list",
                &[("types", "public_channel,private_channel"), ("limit", "200")],
            )
            .await?;
        let mut out = Vec::new();
        if let Some(arr) = env.get("channels").and_then(|v| v.as_array()) {
            let mut map = self.name_to_id.lock().unwrap();
            for ch in arr {
                let id = ch.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let name = ch.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                if !id.is_empty() {
                    map.insert(name.to_lowercase(), id.clone());
                    out.push((id, name));
                }
            }
        }
        Ok(out)
    }

    /// Resolve a recipient (channel id like "C123", or "#name"/"name") to an id.
    async fn resolve_channel(&self, to: &str) -> Result<String> {
        let t = to.trim().trim_start_matches('#');
        // Looks like a Slack id already (C/G/D + uppercase).
        if t.len() > 1 && t.chars().next().map(|c| matches!(c, 'C' | 'G' | 'D')).unwrap_or(false)
            && t.chars().all(|c| c.is_ascii_alphanumeric())
        {
            return Ok(t.to_string());
        }
        if let Some(id) = self.name_to_id.lock().unwrap().get(&t.to_lowercase()).cloned() {
            return Ok(id);
        }
        // Refresh and retry.
        self.conversations().await?;
        self.name_to_id
            .lock()
            .unwrap()
            .get(&t.to_lowercase())
            .cloned()
            .ok_or_else(|| EuropaError::Adapter(format!("Slack channel '{to}' not found")))
    }

    fn history_to_data(channel: &str, msg: &Value) -> Option<MessageData> {
        let text = msg.get("text").and_then(|v| v.as_str()).unwrap_or("");
        if text.is_empty() {
            return None;
        }
        let ts = msg.get("ts").and_then(|v| v.as_str()).unwrap_or("");
        let sender = msg
            .get("user")
            .or_else(|| msg.get("bot_id"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        let date = ts
            .split('.')
            .next()
            .and_then(|s| s.parse::<i64>().ok())
            .and_then(|secs| chrono::DateTime::from_timestamp(secs, 0))
            .map(|dt| dt.to_rfc3339())
            .unwrap_or_else(|| Utc::now().to_rfc3339());
        Some(MessageData {
            id: format!("slack_{channel}_{ts}"),
            text: text.to_string(),
            sender,
            recipient: channel.to_string(),
            subject: String::new(),
            date,
            channel: "slack".to_string(),
            folder: String::new(),
            imap_uid: None,
            account: String::new(),
        })
    }

    /// Fetch history for one channel since the last seen ts, index, advance cursor.
    async fn index_channel(&self, store: &MessageStore, channel: &str) -> Result<usize> {
        let oldest = self.last_ts.lock().unwrap().get(channel).cloned().unwrap_or_default();
        let env: Value = self
            .call(
                "conversations.history",
                &[("channel", channel), ("oldest", &oldest), ("limit", "200")],
            )
            .await?;
        let mut indexed = 0;
        let mut newest = oldest.clone();
        if let Some(arr) = env.get("messages").and_then(|v| v.as_array()) {
            for m in arr {
                if let Some(ts) = m.get("ts").and_then(|v| v.as_str()) {
                    if ts > newest.as_str() {
                        newest = ts.to_string();
                    }
                }
                if let Some(data) = Self::history_to_data(channel, m) {
                    match store.index_message(data).await {
                        Ok(true) => indexed += 1,
                        Ok(false) => {}
                        Err(e) => warn!("Slack index failed: {e}"),
                    }
                }
            }
        }
        if !newest.is_empty() {
            self.last_ts.lock().unwrap().insert(channel.to_string(), newest);
        }
        Ok(indexed)
    }
}

#[async_trait]
impl MessagingAdapter for SlackAdapter {
    fn channel_name(&self) -> &str {
        "slack"
    }

    async fn connect(&mut self) -> Result<()> {
        if self.token.is_empty() {
            return Err(EuropaError::Config("SLACK_BOT_TOKEN not set".into()));
        }
        let env: Value = self.call("auth.test", &[]).await?;
        info!(
            "Connected to Slack workspace '{}' as '{}'",
            env.get("team").and_then(|v| v.as_str()).unwrap_or("?"),
            env.get("user").and_then(|v| v.as_str()).unwrap_or("?")
        );
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<()> {
        Ok(())
    }

    async fn list_contacts(&self) -> Result<Vec<Contact>> {
        let channels = self.conversations().await?;
        Ok(channels
            .into_iter()
            .map(|(id, name)| Contact {
                name: format!("#{name}"),
                // Slack ids are alphanumeric; expose a stable hash-free placeholder id.
                id: 0,
                contact_type: ContactType::Channel,
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
        let channel = self.resolve_channel(to).await?;
        let env: Value = self
            .call("chat.postMessage", &[("channel", &channel), ("text", message)])
            .await?;
        let ts = env.get("ts").and_then(|v| v.as_str()).unwrap_or("");
        Ok(format!("Sent to {to} (ts: {ts})"))
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
            id: format!("slack_sent_{}", Utc::now().timestamp_millis()),
            text: message.to_string(),
            sender: "me".to_string(),
            recipient: to.to_string(),
            subject: String::new(),
            date: Utc::now().to_rfc3339(),
            channel: "slack".to_string(),
            folder: String::new(),
            imap_uid: None,
            account: String::new(),
        };
        if let Err(e) = store.index_message(msg).await {
            warn!("Failed to index sent Slack message: {e}");
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
        Err(EuropaError::Adapter("save_media not yet supported for Slack".into()))
    }

    async fn resolve_entities(&self) -> Result<Vec<Entity>> {
        let channels = self.conversations().await?;
        Ok(channels
            .into_iter()
            .map(|(id, name)| Entity {
                id: 0,
                name: format!("{name}|{id}"),
                entity_type: ContactType::Channel,
            })
            .collect())
    }

    async fn bulk_index(&self, store: &MessageStore, entity: &Entity) -> Result<IndexResult> {
        // Entity name is "name|id"; extract the id.
        let channel_id = entity.name.rsplit('|').next().unwrap_or(&entity.name);
        let indexed = self.index_channel(store, channel_id).await?;
        Ok(IndexResult { indexed, skipped: 0, errors: 0 })
    }

    async fn start_listener(&self, store: &MessageStore, entities: &[Entity]) -> Result<()> {
        info!("Slack listener started (polling conversations.history)");
        loop {
            for entity in entities {
                let channel_id = entity.name.rsplit('|').next().unwrap_or(&entity.name);
                if let Err(e) = self.index_channel(store, channel_id).await {
                    warn!("Slack poll '{}' failed: {e}", entity.name);
                }
            }
            tokio::time::sleep(Duration::from_secs(30)).await;
        }
    }
}
