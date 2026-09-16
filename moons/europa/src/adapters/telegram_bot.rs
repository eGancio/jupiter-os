// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

//! Telegram **Bot** adapter using the HTTP Bot API (https://core.telegram.org/bots/api).
//!
//! Unlike the user-account adapter ([`super::telegram::TelegramAdapter`], MTProto
//! via grammers), this only needs a bot token from @BotFather — no api_id/api_hash,
//! no phone/OTP login. Bots cannot read arbitrary chat history; they only see chats
//! they were added to or users who messaged them, surfaced via `getUpdates`.
//!
//! Serves the `telegram` channel whenever `TELEGRAM_BOT_TOKEN` is configured.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use jupiteros_shared::store::{IndexResult, MessageData, MessageStore};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use tracing::{info, warn};

use super::{Contact, ContactType, Entity, MessagingAdapter};
use crate::config::Config;
use crate::error::{EuropaError, Result};

pub struct TelegramBotAdapter {
    token: String,
    http: reqwest::Client,
    /// getUpdates long-poll offset (last update_id + 1).
    offset: Mutex<i64>,
    /// Chats seen via getUpdates: chat_id -> display name (for list_contacts).
    seen_chats: Mutex<HashMap<i64, String>>,
    bot_username: Mutex<Option<String>>,
}

impl TelegramBotAdapter {
    pub fn new(config: Config) -> Self {
        Self {
            token: config.telegram_bot_token.clone().unwrap_or_default(),
            http: reqwest::Client::new(),
            offset: Mutex::new(0),
            seen_chats: Mutex::new(HashMap::new()),
            bot_username: Mutex::new(None),
        }
    }

    fn base(&self) -> String {
        format!("https://api.telegram.org/bot{}", self.token)
    }

    /// Call a Bot API method with a JSON body, returning the `result` field.
    async fn call<T: DeserializeOwned>(&self, method: &str, body: Value) -> Result<T> {
        let url = format!("{}/{}", self.base(), method);
        let resp = self
            .http
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| EuropaError::Adapter(format!("Bot API {method}: {e}")))?;

        let envelope: Value = resp
            .json()
            .await
            .map_err(|e| EuropaError::Adapter(format!("Bot API {method} decode: {e}")))?;

        if envelope.get("ok").and_then(|v| v.as_bool()) != Some(true) {
            let desc = envelope
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            return Err(EuropaError::Adapter(format!("Bot API {method}: {desc}")));
        }

        let result = envelope.get("result").cloned().unwrap_or(Value::Null);
        serde_json::from_value(result)
            .map_err(|e| EuropaError::Adapter(format!("Bot API {method} parse: {e}")))
    }

    /// Resolve a recipient string into a chat_id JSON value (number or @username).
    fn chat_id_value(to: &str) -> Value {
        let t = to.trim();
        if let Ok(id) = t.parse::<i64>() {
            json!(id)
        } else if t.starts_with('@') {
            json!(t)
        } else {
            json!(format!("@{t}"))
        }
    }

    /// Convert a Bot API `message` object into our MessageData (or None if no text).
    fn message_to_data(&self, msg: &Value) -> Option<MessageData> {
        let text = msg
            .get("text")
            .or_else(|| msg.get("caption"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if text.is_empty() {
            return None;
        }
        let msg_id = msg.get("message_id").and_then(|v| v.as_i64()).unwrap_or(0);
        let chat = msg.get("chat");
        let chat_id = chat.and_then(|c| c.get("id")).and_then(|v| v.as_i64()).unwrap_or(0);
        let chat_name = chat
            .map(Self::entity_title)
            .unwrap_or_default();
        let sender = msg
            .get("from")
            .map(Self::entity_title)
            .unwrap_or_default();
        let date = msg
            .get("date")
            .and_then(|v| v.as_i64())
            .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0))
            .map(|dt| dt.to_rfc3339())
            .unwrap_or_else(|| Utc::now().to_rfc3339());

        // Remember the chat for list_contacts
        if chat_id != 0 {
            self.seen_chats
                .lock()
                .unwrap()
                .insert(chat_id, chat_name.clone());
        }

        Some(MessageData {
            id: format!("tgbot_{chat_id}_{msg_id}"),
            text,
            sender,
            recipient: chat_name,
            subject: String::new(),
            date,
            channel: "telegram".to_string(),
            folder: String::new(),
            imap_uid: None,
            account: String::new(),
        })
    }

    /// Best-effort display name from a Bot API chat/user object.
    fn entity_title(obj: &Value) -> String {
        if let Some(title) = obj.get("title").and_then(|v| v.as_str()) {
            return title.to_string();
        }
        let first = obj.get("first_name").and_then(|v| v.as_str()).unwrap_or("");
        let last = obj.get("last_name").and_then(|v| v.as_str()).unwrap_or("");
        let name = format!("{first} {last}").trim().to_string();
        if !name.is_empty() {
            return name;
        }
        obj.get("username")
            .and_then(|v| v.as_str())
            .map(|u| format!("@{u}"))
            .unwrap_or_else(|| {
                obj.get("id")
                    .and_then(|v| v.as_i64())
                    .map(|i| i.to_string())
                    .unwrap_or_default()
            })
    }

    /// Poll getUpdates once, indexing any text messages. Returns number indexed.
    async fn poll_updates(&self, store: &MessageStore, timeout_secs: u64) -> Result<usize> {
        let offset = *self.offset.lock().unwrap();
        let updates: Vec<Value> = self
            .call(
                "getUpdates",
                json!({ "offset": offset, "timeout": timeout_secs, "allowed_updates": ["message", "channel_post"] }),
            )
            .await?;

        let mut indexed = 0;
        let mut max_update_id = offset - 1;
        for upd in &updates {
            if let Some(uid) = upd.get("update_id").and_then(|v| v.as_i64()) {
                max_update_id = max_update_id.max(uid);
            }
            let msg = upd
                .get("message")
                .or_else(|| upd.get("channel_post"));
            if let Some(msg) = msg {
                if let Some(data) = self.message_to_data(msg) {
                    match store.index_message(data).await {
                        Ok(true) => indexed += 1,
                        Ok(false) => {}
                        Err(e) => warn!("Bot index failed: {e}"),
                    }
                }
            }
        }
        if max_update_id >= offset {
            *self.offset.lock().unwrap() = max_update_id + 1;
        }
        Ok(indexed)
    }
}

#[async_trait]
impl MessagingAdapter for TelegramBotAdapter {
    fn channel_name(&self) -> &str {
        "telegram"
    }

    async fn connect(&mut self) -> Result<()> {
        if self.token.is_empty() {
            return Err(EuropaError::Config("TELEGRAM_BOT_TOKEN not set".into()));
        }
        let me: Value = self.call("getMe", json!({})).await?;
        let username = me
            .get("username")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        info!(
            "Connected as Telegram bot: @{} (ID: {})",
            username.clone().unwrap_or_else(|| "?".into()),
            me.get("id").and_then(|v| v.as_i64()).unwrap_or(0)
        );
        *self.bot_username.lock().unwrap() = username;
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<()> {
        Ok(())
    }

    async fn list_contacts(&self) -> Result<Vec<Contact>> {
        // Bots can't enumerate dialogs. Surface chats seen via getUpdates.
        let chats = self.seen_chats.lock().unwrap().clone();
        let contacts = chats
            .into_iter()
            .map(|(id, name)| Contact {
                name,
                id,
                contact_type: if id < 0 { ContactType::Group } else { ContactType::User },
                username: None,
            })
            .collect::<Vec<_>>();
        Ok(contacts)
    }

    async fn send_message(
        &self,
        to: &str,
        message: &str,
        attachments: &[PathBuf],
    ) -> Result<String> {
        let chat_id = Self::chat_id_value(to);

        if attachments.is_empty() {
            if message.is_empty() {
                return Err(EuropaError::Adapter(
                    "Message text is empty and no attachments provided".into(),
                ));
            }
            let sent: Value = self
                .call("sendMessage", json!({ "chat_id": chat_id, "text": message }))
                .await?;
            let id = sent.get("message_id").and_then(|v| v.as_i64()).unwrap_or(0);
            return Ok(format!("Sent to {to} (msg_id: {id})"));
        }

        // Attachments → sendDocument (multipart), one per file.
        let mut last_id = 0i64;
        for (i, path) in attachments.iter().enumerate() {
            if !path.exists() {
                return Err(EuropaError::Adapter(format!("File not found: {}", path.display())));
            }
            let bytes = std::fs::read(path)?;
            let fname = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "file.bin".into());
            let mut form = reqwest::multipart::Form::new()
                .text("chat_id", chat_id.to_string().trim_matches('"').to_string())
                .part(
                    "document",
                    reqwest::multipart::Part::bytes(bytes).file_name(fname),
                );
            if i == 0 && !message.is_empty() {
                form = form.text("caption", message.to_string());
            }
            let url = format!("{}/sendDocument", self.base());
            let resp = self
                .http
                .post(&url)
                .multipart(form)
                .send()
                .await
                .map_err(|e| EuropaError::Adapter(format!("sendDocument: {e}")))?;
            let env: Value = resp
                .json()
                .await
                .map_err(|e| EuropaError::Adapter(format!("sendDocument decode: {e}")))?;
            if env.get("ok").and_then(|v| v.as_bool()) != Some(true) {
                let desc = env.get("description").and_then(|v| v.as_str()).unwrap_or("error");
                return Err(EuropaError::Adapter(format!("sendDocument: {desc}")));
            }
            last_id = env
                .get("result")
                .and_then(|r| r.get("message_id"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
        }
        let n = attachments.len();
        Ok(format!("Sent to {to} ({n} file{}, msg_id: {last_id})", if n == 1 { "" } else { "s" }))
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
            id: format!("tgbot_sent_{}", Utc::now().timestamp_millis()),
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
            warn!("Failed to index sent bot message: {e}");
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
        // Bot API exposes only files referenced by recent updates; arbitrary
        // history download is not possible. Not supported in this adapter.
        Err(EuropaError::Adapter(
            "save_media is not supported for Telegram bots (no history access). Use the user account channel.".into(),
        ))
    }

    async fn resolve_entities(&self) -> Result<Vec<Entity>> {
        // Bots index live via the listener; nothing to bulk-resolve.
        Ok(Vec::new())
    }

    async fn bulk_index(&self, _store: &MessageStore, _entity: &Entity) -> Result<IndexResult> {
        Ok(IndexResult { indexed: 0, skipped: 0, errors: 0 })
    }

    async fn start_listener(&self, store: &MessageStore, _entities: &[Entity]) -> Result<()> {
        info!("Telegram bot listener started (long-poll getUpdates)");
        loop {
            match self.poll_updates(store, 50).await {
                Ok(n) if n > 0 => info!("Bot indexed {n} new message(s)"),
                Ok(_) => {}
                Err(e) => {
                    warn!("Bot getUpdates error: {e}");
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
        }
    }
}
