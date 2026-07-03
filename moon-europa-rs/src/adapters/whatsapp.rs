// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

//! WhatsApp adapter (personal account) backed by the Baileys Node helper.
//!
//! WhatsApp has no official personal API, so we link the account as a companion
//! device (multi-device, same as WhatsApp Web) via the Baileys library. Baileys
//! is Node, moon-europa is Rust — so this adapter spawns the helper script
//! (`whatsapp-helper/helper.mjs run`) and talks to it over **stdio JSON-lines**,
//! the same shape as the JupiterOS chat sidecar.
//!
//! Pairing (the QR scan) is driven separately by the GUI/Tauri backend in the
//! helper's `pair` mode; here we only `run` an already-paired session.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use jupiteros_shared::store::{IndexResult, MessageData, MessageStore};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{mpsc, oneshot, Mutex as AsyncMutex};
use tracing::{info, warn};

use super::{Contact, ContactType, Entity, MessagingAdapter};
use crate::config::Config;
use crate::error::{EuropaError, Result};

/// Deterministic positive i64 id for a WhatsApp jid (FNV-1a, top bit cleared).
/// The trait's Contact/Entity use i64 ids; WhatsApp's natural key is the jid
/// string, so we hash it and keep a reverse map for bulk_index/send.
fn jid_id(jid: &str) -> i64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in jid.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    (h >> 1) as i64
}

/// Convert a helper "message"/history JSON node into our MessageData.
fn message_to_data(v: &Value) -> Option<MessageData> {
    let chat = v.get("chat").and_then(|x| x.as_str()).unwrap_or("");
    if chat.is_empty() {
        return None;
    }
    let id = v.get("id").and_then(|x| x.as_str()).unwrap_or("");
    let from_me = v.get("fromMe").and_then(|x| x.as_bool()).unwrap_or(false);
    let sender = if from_me {
        "me".to_string()
    } else {
        v.get("sender").and_then(|x| x.as_str()).unwrap_or("").to_string()
    };
    Some(MessageData {
        id: format!("wa_{id}"),
        text: v.get("text").and_then(|x| x.as_str()).unwrap_or("").to_string(),
        sender,
        recipient: chat.to_string(),
        subject: String::new(),
        date: v.get("date").and_then(|x| x.as_str()).unwrap_or("").to_string(),
        channel: "whatsapp".to_string(),
        folder: String::new(),
        imap_uid: None,
        account: String::new(),
    })
}

pub struct WhatsappAdapter {
    config: Config,
    /// Writer to the helper's stdin (commands).
    stdin: AsyncMutex<Option<ChildStdin>>,
    /// Helper process handle (kept so we can kill it on disconnect).
    child: AsyncMutex<Option<Child>>,
    /// reqId -> reply channel for request/response commands.
    pending: Arc<StdMutex<HashMap<u64, oneshot::Sender<Value>>>>,
    /// Entity/Contact i64 id -> jid (filled by list_contacts/resolve_entities).
    jid_by_id: Arc<StdMutex<HashMap<i64, String>>>,
    /// Live incoming messages, consumed by start_listener.
    msg_rx: AsyncMutex<Option<mpsc::UnboundedReceiver<MessageData>>>,
    req_counter: AtomicU64,
    connected: Arc<AtomicBool>,
}

impl WhatsappAdapter {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            stdin: AsyncMutex::new(None),
            child: AsyncMutex::new(None),
            pending: Arc::new(StdMutex::new(HashMap::new())),
            jid_by_id: Arc::new(StdMutex::new(HashMap::new())),
            msg_rx: AsyncMutex::new(None),
            req_counter: AtomicU64::new(1),
            connected: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Send a command to the helper and await its reply (matched by reqId).
    async fn request(&self, mut cmd: Value) -> Result<Value> {
        let id = self.req_counter.fetch_add(1, Ordering::Relaxed);
        cmd["reqId"] = json!(id);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);

        {
            let mut guard = self.stdin.lock().await;
            let stdin = guard
                .as_mut()
                .ok_or_else(|| EuropaError::Adapter("WhatsApp helper not running".into()))?;
            let line = format!("{}\n", serde_json::to_string(&cmd).unwrap_or_default());
            stdin
                .write_all(line.as_bytes())
                .await
                .map_err(|e| EuropaError::Adapter(format!("write helper stdin: {e}")))?;
            let _ = stdin.flush().await;
        }

        match tokio::time::timeout(Duration::from_secs(30), rx).await {
            Ok(Ok(v)) => {
                if let Some(err) = v.get("error").and_then(|e| e.as_str()) {
                    return Err(EuropaError::Adapter(format!("WhatsApp helper: {err}")));
                }
                Ok(v)
            }
            Ok(Err(_)) => Err(EuropaError::Adapter("WhatsApp helper reply dropped".into())),
            Err(_) => {
                self.pending.lock().unwrap().remove(&id);
                Err(EuropaError::Adapter("WhatsApp helper timeout".into()))
            }
        }
    }

    /// Stop the helper GRACEFULLY, then SIGKILL only as a fallback.
    ///
    /// A bare `start_kill()` (SIGKILL) while Baileys is mid-write of `creds.json`
    /// truncates it to 0 bytes, which permanently bricks the companion session
    /// (every later link attempt then fails and re-truncates — a death spiral).
    /// Instead we close the helper's stdin: its readline sees EOF and exits after
    /// ending the socket cleanly. We wait a beat for that, then kill if it lingers.
    async fn stop_helper(&self) {
        // Drop stdin → close the pipe → helper's `rl.on("close")` fires.
        drop(self.stdin.lock().await.take());
        if let Some(mut c) = self.child.lock().await.take() {
            for _ in 0..14 {
                if let Ok(Some(_)) = c.try_wait() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            let _ = c.start_kill();
        }
    }
}

#[async_trait]
impl MessagingAdapter for WhatsappAdapter {
    fn channel_name(&self) -> &str {
        "whatsapp"
    }

    async fn connect(&mut self) -> Result<()> {
        let _ = std::fs::create_dir_all(&self.config.whatsapp_session_path);

        if !self.config.whatsapp_helper_path.exists() {
            return Err(EuropaError::Adapter(format!(
                "WhatsApp helper not found at {:?} (set WHATSAPP_HELPER or run npm ci in whatsapp-helper)",
                self.config.whatsapp_helper_path
            )));
        }

        let mut child = Command::new(&self.config.whatsapp_node)
            .arg(&self.config.whatsapp_helper_path)
            .arg("run")
            .arg("--session")
            .arg(&self.config.whatsapp_session_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // CRUCIAL: kill the helper if this adapter is dropped (e.g. connect
            // times out). Otherwise orphaned helpers pile up and fight over the
            // single-socket WhatsApp companion session, so none ever connects.
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| EuropaError::Adapter(format!("spawn WhatsApp helper (node): {e}")))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| EuropaError::Adapter("helper has no stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| EuropaError::Adapter("helper has no stdout".into()))?;

        // Surface helper stderr into our logs (was /dev/null → blind debugging).
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(l)) = lines.next_line().await {
                    warn!("[wa-helper] {l}");
                }
            });
        }

        let (msg_tx, msg_rx) = mpsc::unbounded_channel();
        let (conn_tx, conn_rx) = oneshot::channel::<()>();
        let pending = Arc::clone(&self.pending);
        let connected = Arc::clone(&self.connected);

        // Reader task: routes JSON-lines from the helper.
        tokio::spawn(async move {
            let mut conn_tx = Some(conn_tx);
            let mut lines = BufReader::new(stdout).lines();
            loop {
                let line = match lines.next_line().await {
                    Ok(Some(l)) => l,
                    _ => break,
                };
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let v: Value = match serde_json::from_str(line) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                // Command replies carry a reqId.
                if let Some(id) = v.get("reqId").and_then(|x| x.as_u64()) {
                    if let Some(tx) = pending.lock().unwrap().remove(&id) {
                        let _ = tx.send(v);
                    }
                    continue;
                }
                match v.get("type").and_then(|t| t.as_str()) {
                    Some("connected") => {
                        connected.store(true, Ordering::Relaxed);
                        if let Some(tx) = conn_tx.take() {
                            let _ = tx.send(());
                        }
                    }
                    Some("message") => {
                        if let Some(md) = message_to_data(&v) {
                            let _ = msg_tx.send(md);
                        }
                    }
                    Some("error") => {
                        let err = v.get("error").and_then(|e| e.as_str()).unwrap_or("");
                        warn!("WhatsApp helper error: {err}");
                        if v.get("fatal").and_then(|b| b.as_bool()) == Some(true) {
                            connected.store(false, Ordering::Relaxed);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            connected.store(false, Ordering::Relaxed);
        });

        *self.stdin.lock().await = Some(stdin);
        *self.child.lock().await = Some(child);
        *self.msg_rx.lock().await = Some(msg_rx);

        match tokio::time::timeout(Duration::from_secs(45), conn_rx).await {
            Ok(Ok(())) => {
                info!("WhatsApp connected");
                Ok(())
            }
            _ => {
                // Stop the helper gracefully so it doesn't linger and fight the
                // session on the next attempt (single-socket companion) — and so
                // a kill can't truncate creds.json.
                self.stop_helper().await;
                Err(EuropaError::NotAuthorized(
                    "WhatsApp session not connected — pair it from the Chats panel.".into(),
                ))
            }
        }
    }

    async fn disconnect(&mut self) -> Result<()> {
        self.stop_helper().await;
        self.connected.store(false, Ordering::Relaxed);
        Ok(())
    }

    async fn list_contacts(&self) -> Result<Vec<Contact>> {
        let v = self.request(json!({"cmd": "list_contacts"})).await?;
        let arr = v
            .get("contacts")
            .and_then(|c| c.as_array())
            .cloned()
            .unwrap_or_default();
        let mut map = self.jid_by_id.lock().unwrap();
        let mut out = Vec::with_capacity(arr.len());
        for c in arr {
            let jid = c.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string();
            if jid.is_empty() {
                continue;
            }
            let name = c.get("name").and_then(|x| x.as_str()).unwrap_or(&jid).to_string();
            let contact_type = match c.get("type").and_then(|x| x.as_str()) {
                Some("group") => ContactType::Group,
                Some("channel") => ContactType::Channel,
                _ => ContactType::User,
            };
            let id = jid_id(&jid);
            map.insert(id, jid.clone());
            out.push(Contact {
                name,
                id,
                contact_type,
                username: Some(jid),
            });
        }
        Ok(out)
    }

    async fn send_message(
        &self,
        to: &str,
        message: &str,
        _attachments: &[PathBuf],
    ) -> Result<String> {
        let v = self
            .request(json!({"cmd": "send", "to": to, "text": message}))
            .await?;
        Ok(v.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string())
    }

    async fn send_message_and_index(
        &self,
        to: &str,
        message: &str,
        attachments: &[PathBuf],
        store: &MessageStore,
    ) -> Result<String> {
        let id = self.send_message(to, message, attachments).await?;
        let now = Utc::now().to_rfc3339();
        let key = if id.is_empty() { now.clone() } else { id.clone() };
        let md = MessageData {
            id: format!("wa_{key}"),
            text: message.to_string(),
            sender: "me".to_string(),
            recipient: to.to_string(),
            subject: String::new(),
            date: now,
            channel: "whatsapp".to_string(),
            folder: String::new(),
            imap_uid: None,
            account: String::new(),
        };
        if let Err(e) = store.index_message(md).await {
            warn!("index sent WhatsApp message failed: {e}");
        }
        Ok(id)
    }

    async fn save_media(
        &self,
        _chat: &str,
        _message_id: Option<i64>,
        _last_n: usize,
        _save_dir: &Path,
    ) -> Result<Vec<PathBuf>> {
        warn!("WhatsApp save_media not implemented yet");
        Ok(vec![])
    }

    async fn resolve_entities(&self) -> Result<Vec<Entity>> {
        let contacts = self.list_contacts().await?;
        Ok(contacts
            .into_iter()
            .map(|c| Entity {
                id: c.id,
                name: c.name,
                entity_type: c.contact_type,
            })
            .collect())
    }

    async fn bulk_index(&self, store: &MessageStore, entity: &Entity) -> Result<IndexResult> {
        let jid = self.jid_by_id.lock().unwrap().get(&entity.id).cloned();
        let jid = match jid {
            Some(j) => j,
            None => {
                return Ok(IndexResult {
                    indexed: 0,
                    skipped: 0,
                    errors: 0,
                })
            }
        };
        let v = self
            .request(json!({"cmd": "fetch_history", "chat": jid, "limit": 500}))
            .await?;
        let msgs = v
            .get("messages")
            .and_then(|m| m.as_array())
            .cloned()
            .unwrap_or_default();
        let mut batch = Vec::new();
        for m in &msgs {
            if let Some(md) = message_to_data(m) {
                if !md.text.is_empty() {
                    batch.push(md);
                }
            }
        }
        if batch.is_empty() {
            return Ok(IndexResult {
                indexed: 0,
                skipped: 0,
                errors: 0,
            });
        }
        Ok(store.index_batch(batch, true).await?)
    }

    async fn start_listener(&self, store: &MessageStore, _entities: &[Entity]) -> Result<()> {
        let mut rx = self
            .msg_rx
            .lock()
            .await
            .take()
            .ok_or_else(|| EuropaError::Adapter("WhatsApp listener already running".into()))?;
        info!("WhatsApp listener started (event-driven)");
        while let Some(md) = rx.recv().await {
            if let Err(e) = store.index_message(md).await {
                warn!("WhatsApp index failed: {e}");
            }
        }
        // Channel closed → helper stdout ended. Return Err so the daemon restarts us.
        Err(EuropaError::Adapter("WhatsApp helper stream closed".into()))
    }
}
