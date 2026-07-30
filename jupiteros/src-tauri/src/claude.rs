// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

use std::io::{BufRead, BufReader, Write as IoWrite};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::config;

// ── Payloads emitted to frontend ──────────────────────────────

#[derive(Clone, Serialize)]
pub struct TextDeltaPayload {
    pub session_id: String,
    pub text: String,
}

#[derive(Clone, Serialize)]
pub struct ToolStartPayload {
    pub session_id: String,
    pub tool_name: String,
    pub tool_id: String,
    /// tool_use id of the parent Agent/Task call when this tool belongs to a
    /// subagent (None for top-level tools and non-Claude engines).
    pub parent_tool_id: Option<String>,
}

#[derive(Clone, Serialize)]
pub struct ToolInputDeltaPayload {
    pub session_id: String,
    pub tool_id: String,
    pub partial_json: String,
    pub parent_tool_id: Option<String>,
}

#[derive(Clone, Serialize)]
pub struct ToolResultPayload {
    pub session_id: String,
    pub tool_id: String,
    pub result: String,
    pub parent_tool_id: Option<String>,
}

#[derive(Clone, Serialize)]
pub struct ThinkingDeltaPayload {
    pub session_id: String,
    pub thinking: String,
}

#[derive(Clone, Serialize)]
pub struct ChatDonePayload {
    pub session_id: String,
}

#[derive(Clone, Serialize)]
pub struct ChatErrorPayload {
    pub session_id: String,
    pub error: String,
}

#[derive(Clone, Serialize)]
pub struct UsagePayload {
    pub session_id: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub total_cost_usd: f64,
}

// ── Stored message data ───────────────────────────────────────

#[derive(Clone, Serialize, Deserialize)]
pub struct ToolCallInfo {
    pub name: String,
    #[serde(default)]
    pub id: String,
    pub input: String,
    pub result: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_tool_id: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ChatMsg {
    pub id: String,
    pub role: String,
    pub content: String,
    #[serde(default)]
    pub thinking: String,
    pub tool_calls: Vec<ToolCallInfo>,
    pub timestamp: u64,
    /// Set by the frontend's eager flush when persisting an in-progress
    /// assistant message. The `done` handler replaces such a placeholder
    /// with the finalized message, so duplicates can't accumulate.
    #[serde(default)]
    pub streaming: bool,
}

// ── Lightweight chat session (metadata only) ─────────────────

fn default_engine() -> String {
    "claude".to_string()
}

/// Serializable subset for disk persistence
#[derive(Clone, Serialize, Deserialize)]
pub struct ChatSessionPersist {
    pub id: String,
    pub sdk_session_id: Option<String>,
    pub messages: Vec<ChatMsg>,
    pub model: String,
    #[serde(default = "default_engine")]
    pub engine: String,
    pub title: String,
    pub created_at: u64,
    pub updated_at: u64,
    // Both #[serde(default)] so pre-existing jupiteros-sessions.json load fine.
    /// User renamed the title by hand → the auto-titler must NOT overwrite it.
    #[serde(default)]
    pub title_manual: bool,
    /// Unix secs of the last successful auto-title (None = never titled).
    #[serde(default)]
    pub classified_at: Option<u64>,
}

pub struct ChatSession {
    pub id: String,
    pub sdk_session_id: Option<String>,
    pub messages: Vec<ChatMsg>,
    pub model: String,
    pub engine: String,
    pub title: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub title_manual: bool,
    pub classified_at: Option<u64>,
    busy: Arc<AtomicBool>,
}

impl ChatSession {
    pub fn new(id: String) -> Self {
        let now = now_secs();
        Self {
            id,
            sdk_session_id: None,
            messages: Vec::new(),
            model: "opus".to_string(),
            engine: "claude".to_string(),
            title: "Nuova chat".to_string(),
            created_at: now,
            updated_at: now,
            title_manual: false,
            classified_at: None,
            busy: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn from_persist(p: ChatSessionPersist) -> Self {
        Self {
            id: p.id,
            sdk_session_id: p.sdk_session_id,
            messages: p.messages,
            model: p.model,
            engine: p.engine,
            title: p.title,
            created_at: p.created_at,
            updated_at: p.updated_at,
            title_manual: p.title_manual,
            classified_at: p.classified_at,
            busy: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn to_persist(&self) -> ChatSessionPersist {
        ChatSessionPersist {
            id: self.id.clone(),
            sdk_session_id: self.sdk_session_id.clone(),
            messages: self.messages.clone(),
            model: self.model.clone(),
            engine: self.engine.clone(),
            title: self.title.clone(),
            created_at: self.created_at,
            updated_at: self.updated_at,
            title_manual: self.title_manual,
            classified_at: self.classified_at,
        }
    }

    pub fn is_busy(&self) -> bool {
        self.busy.load(Ordering::Relaxed)
    }

    pub fn set_busy(&self, val: bool) {
        self.busy.store(val, Ordering::Relaxed);
    }

    /// Record a user message in the session history
    pub fn add_user_message(&mut self, content: String) {
        self.messages.push(ChatMsg {
            id: uuid::Uuid::new_v4().to_string(),
            role: "user".to_string(),
            content,
            thinking: String::new(),
            tool_calls: Vec::new(),
            timestamp: now_secs(),
            streaming: false,
        });
        self.updated_at = now_secs();

        // Auto-title from first user message. Char-safe truncation: byte-slicing
        // at 40 would panic on a multibyte char straddling the boundary. This is
        // only a placeholder — the Haiku classifier replaces it with a real title
        // once the chat has enough content (unless the user renamed it by hand).
        if self.messages.len() == 1 {
            let t = &self.messages[0].content;
            let truncated: String = t.chars().take(40).collect();
            self.title = if truncated.chars().count() < t.chars().count() {
                format!("{}...", truncated)
            } else {
                truncated
            };
        }
    }
}

// ── Agent Sidecar — persistent Node.js process ───────────────

pub struct AgentSidecar {
    child: Child,
    stdin: Mutex<std::io::BufWriter<std::process::ChildStdin>>,
}

impl AgentSidecar {
    /// Spawn the sidecar and start the stdout reader thread.
    /// The reader thread emits Tauri events as events arrive from the SDK.
    pub fn spawn(
        app_handle: AppHandle,
        sessions: Arc<Mutex<Vec<ChatSession>>>,
    ) -> Result<Self, String> {
        let base = config::get_base_dir();
        let sidecar_path = {
            let with_sub = base.join("jupiteros").join("sidecar").join("agent.mjs");
            let without_sub = base.join("sidecar").join("agent.mjs");
            if with_sub.exists() { with_sub } else { without_sub }
        };

        if !sidecar_path.exists() {
            return Err(format!("Sidecar not found: {:?}", sidecar_path));
        }

        // Spawn: node agent.mjs
        #[cfg(target_os = "windows")]
        let mut cmd = {
            let mut c = Command::new("cmd");
            c.arg("/C").arg("node");
            c.arg(sidecar_path.to_string_lossy().to_string());
            c
        };
        #[cfg(not(target_os = "windows"))]
        let mut cmd = {
            let mut c = Command::new("node");
            c.arg(&sidecar_path);
            c
        };

        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.current_dir(config::get_base_dir());

        // Inject credentials.env so stdio MCP servers (es. ClickUp) can resolve ${VAR}
        for (k, v) in config::load_credentials_env() {
            cmd.env(k, v);
        }

        // Inject Ads API keys from the OS keyring (Google Ads / Meta Ads stdio
        // MCP servers). Secrets stay encrypted at rest in the keyring and only
        // reach the child process in memory; .mcp.json holds only ${VAR}
        // placeholders that the sidecar's expandEnv resolves from these.
        for (k, v) in crate::ads_credentials::load_ads_env_from_keyring() {
            cmd.env(k, v);
        }

        // The official Google Ads MCP server reads creds from a google-ads.yaml
        // file, not env vars. Generate it from the keyring into a RAM-backed
        // tmpfs dir and point the server at it via GOOGLE_ADS_CREDENTIALS.
        if let Some(yaml_path) = crate::ads_credentials::materialize_google_ads_yaml() {
            cmd.env("GOOGLE_ADS_CREDENTIALS", yaml_path);
        }

        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }

        let mut child = cmd.spawn().map_err(|e| format!("Failed to spawn sidecar: {}", e))?;

        let stdin = child
            .stdin
            .take()
            .ok_or("Failed to capture sidecar stdin")?;

        // Stderr reader — log sidecar errors
        if let Some(stderr) = child.stderr.take() {
            let log_path = config::get_base_dir().join(".claude-gui-debug.log");
            std::thread::spawn(move || {
                let reader = BufReader::new(stderr);
                for line in reader.lines().flatten() {
                    if let Ok(mut log) =
                        std::fs::OpenOptions::new().create(true).append(true).open(&log_path)
                    {
                        let _ = writeln!(log, "[SIDECAR-STDERR] {}", line);
                    }
                }
            });
        }

        // Stdout reader — parse sidecar events and emit Tauri events
        if let Some(stdout) = child.stdout.take() {
            let handle = app_handle;
            let sess_ref = sessions;
            std::thread::spawn(move || {
                Self::stdout_reader(stdout, handle, sess_ref);
            });
        }

        Ok(Self {
            child,
            stdin: Mutex::new(std::io::BufWriter::new(stdin)),
        })
    }

    /// Send a JSON command to the sidecar via stdin.
    pub fn send_command(&self, cmd: serde_json::Value) -> Result<(), String> {
        let mut stdin = self.stdin.lock().map_err(|e| e.to_string())?;
        let line = serde_json::to_string(&cmd).map_err(|e| e.to_string())?;
        writeln!(*stdin, "{}", line).map_err(|e| format!("Failed to write to sidecar: {}", e))?;
        stdin
            .flush()
            .map_err(|e| format!("Failed to flush sidecar stdin: {}", e))?;
        Ok(())
    }

    /// Kill the sidecar process.
    pub fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    /// Background thread that reads sidecar stdout and emits Tauri events.
    fn stdout_reader(
        stdout: std::process::ChildStdout,
        handle: AppHandle,
        sessions: Arc<Mutex<Vec<ChatSession>>>,
    ) {
        let mut reader = BufReader::new(stdout);
        let log_path = config::get_base_dir().join(".claude-gui-debug.log");

        // Accumulate assistant response per session for persistence
        let mut pending_text: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        let mut pending_thinking: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        let mut pending_tools: std::collections::HashMap<String, Vec<ToolCallInfo>> =
            std::collections::HashMap::new();

        // Read raw BYTES, not lines(): lines() yields Err on invalid UTF-8 and
        // the old `Err => break` closed the read end, which made the sidecar's
        // next stdout write fail with EPIPE — the very crash we're fixing. With
        // read_until + lossy decode a stray byte can never tear down the pipe.
        let mut buf: Vec<u8> = Vec::new();
        loop {
            buf.clear();
            match reader.read_until(b'\n', &mut buf) {
                Ok(0) => break, // EOF: sidecar closed stdout (it exited)
                Ok(_) => {}
                Err(_) => break, // genuine I/O error on the pipe
            }
            let line = String::from_utf8_lossy(&buf);
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            // Log to debug file (char-safe truncation — byte-slicing a multibyte
            // char at 300 would panic and kill this thread, closing the pipe).
            if let Ok(mut log) =
                std::fs::OpenOptions::new().create(true).append(true).open(&log_path)
            {
                let preview: String = trimmed.chars().take(300).collect();
                let _ = writeln!(log, "[SIDECAR-OUT] {}", preview);
            }

            let json: serde_json::Value = match serde_json::from_str(trimmed) {
                Ok(v) => v,
                Err(_) => continue,
            };

            let event = match json.get("event").and_then(|v| v.as_str()) {
                Some(e) => e,
                None => continue,
            };
            let sid = json
                .get("session_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            match event {
                "ready" => {
                    if let Ok(mut log) =
                        std::fs::OpenOptions::new().create(true).append(true).open(&log_path)
                    {
                        let _ = writeln!(log, "[SIDECAR] Ready");
                    }
                }
                "session_created" => {
                    // Session is now active in the SDK
                }
                "text_delta" => {
                    let text = json.get("text").and_then(|v| v.as_str()).unwrap_or("");
                    // Accumulate for persistence
                    pending_text.entry(sid.clone()).or_default().push_str(text);
                    let _ = handle.emit(
                        "claude-text-delta",
                        TextDeltaPayload {
                            session_id: sid,
                            text: text.to_string(),
                        },
                    );
                }
                "thinking_start" => {
                    // Initialize thinking accumulator for this session
                    pending_thinking.entry(sid.clone()).or_default();
                }
                "thinking_delta" => {
                    let thinking = json.get("thinking").and_then(|v| v.as_str()).unwrap_or("");
                    pending_thinking.entry(sid.clone()).or_default().push_str(thinking);
                    let _ = handle.emit(
                        "claude-thinking-delta",
                        ThinkingDeltaPayload {
                            session_id: sid,
                            thinking: thinking.to_string(),
                        },
                    );
                }
                "tool_start" => {
                    let tool_name =
                        json.get("tool_name").and_then(|v| v.as_str()).unwrap_or("");
                    let tool_id = json.get("tool_id").and_then(|v| v.as_str()).unwrap_or("");
                    let parent_tool_id = json
                        .get("parent_tool_id")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty())
                        .map(String::from);
                    // Track tool call for persistence
                    pending_tools.entry(sid.clone()).or_default().push(ToolCallInfo {
                        name: tool_name.to_string(),
                        id: tool_id.to_string(),
                        input: String::new(),
                        result: None,
                        parent_tool_id: parent_tool_id.clone(),
                    });
                    let _ = handle.emit(
                        "claude-tool-start",
                        ToolStartPayload {
                            session_id: sid,
                            tool_name: tool_name.to_string(),
                            tool_id: tool_id.to_string(),
                            parent_tool_id,
                        },
                    );
                }
                "tool_input_delta" => {
                    let tool_id = json.get("tool_id").and_then(|v| v.as_str()).unwrap_or("");
                    let partial =
                        json.get("partial_json").and_then(|v| v.as_str()).unwrap_or("");
                    let parent_tool_id = json
                        .get("parent_tool_id")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty())
                        .map(String::from);
                    // Accumulate tool input for persistence: match by id first
                    // (parallel subagents interleave deltas), fallback to the
                    // last tool for engines that don't set tool_id on deltas.
                    if let Some(tools) = pending_tools.get_mut(&sid) {
                        let target = if tool_id.is_empty() {
                            tools.last_mut()
                        } else {
                            tools.iter_mut().rev().find(|tc| tc.id == tool_id)
                        };
                        if let Some(tc) = target {
                            tc.input.push_str(partial);
                        }
                    }
                    let _ = handle.emit(
                        "claude-tool-input-delta",
                        ToolInputDeltaPayload {
                            session_id: sid,
                            tool_id: tool_id.to_string(),
                            partial_json: partial.to_string(),
                            parent_tool_id,
                        },
                    );
                }
                "tool_result" => {
                    let tool_id = json.get("tool_id").and_then(|v| v.as_str()).unwrap_or("");
                    let result = json.get("result").and_then(|v| v.as_str()).unwrap_or("");
                    let parent_tool_id = json
                        .get("parent_tool_id")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty())
                        .map(String::from);
                    // Store tool result for persistence: match by id first —
                    // with parallel subagents "last without result" would pin
                    // the result on the wrong call.
                    if let Some(tools) = pending_tools.get_mut(&sid) {
                        let idx = if tool_id.is_empty() {
                            tools.iter().rposition(|tc| tc.result.is_none())
                        } else {
                            tools
                                .iter()
                                .rposition(|tc| tc.id == tool_id)
                                .or_else(|| tools.iter().rposition(|tc| tc.result.is_none()))
                        };
                        if let Some(i) = idx {
                            tools[i].result = Some(result.to_string());
                        }
                    }
                    let _ = handle.emit(
                        "claude-tool-result",
                        ToolResultPayload {
                            session_id: sid.clone(),
                            tool_id: tool_id.to_string(),
                            result: result.to_string(),
                            parent_tool_id,
                        },
                    );
                }
                "done" => {
                    // Save accumulated assistant message + mark not busy
                    let text = pending_text.remove(&sid).unwrap_or_default();
                    let thinking = pending_thinking.remove(&sid).unwrap_or_default();
                    let tools = pending_tools.remove(&sid).unwrap_or_default();
                    if let Ok(mut sessions) = sessions.lock() {
                        if let Some(s) = sessions.iter_mut().find(|s| s.id == sid) {
                            // Only save if there's content
                            if !text.is_empty() || !tools.is_empty() || !thinking.is_empty() {
                                // If the last message is a streaming placeholder
                                // (added by the frontend's eager flush), replace
                                // it with the finalized assistant message.
                                if let Some(last) = s.messages.last() {
                                    if last.role == "assistant" && last.streaming {
                                        s.messages.pop();
                                    }
                                }
                                s.messages.push(ChatMsg {
                                    id: uuid::Uuid::new_v4().to_string(),
                                    role: "assistant".to_string(),
                                    content: text,
                                    thinking,
                                    tool_calls: tools,
                                    timestamp: now_secs(),
                                    streaming: false,
                                });
                            }
                            s.set_busy(false);
                            s.updated_at = now_secs();
                        }
                        save_sessions_to_disk(&sessions);
                    }
                    let _ = handle.emit(
                        "claude-done",
                        ChatDonePayload {
                            session_id: sid,
                        },
                    );
                }
                "result" => {
                    // Usage information — emit for /cost tracking
                    let usage = json.get("usage");
                    let _ = handle.emit(
                        "claude-usage",
                        UsagePayload {
                            session_id: sid,
                            input_tokens: usage
                                .and_then(|u| u.get("input_tokens"))
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0),
                            output_tokens: usage
                                .and_then(|u| u.get("output_tokens"))
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0),
                            cache_read_tokens: usage
                                .and_then(|u| u.get("cache_read_tokens"))
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0),
                            total_cost_usd: json
                                .get("total_cost_usd")
                                .and_then(|v| v.as_f64())
                                .unwrap_or(0.0),
                        },
                    );
                }
                "error" => {
                    let error = json.get("error").and_then(|v| v.as_str()).unwrap_or("Unknown error");
                    // Mark session as not busy
                    if !sid.is_empty() {
                        if let Ok(mut sessions) = sessions.lock() {
                            if let Some(s) = sessions.iter_mut().find(|s| s.id == sid) {
                                s.set_busy(false);
                            }
                        }
                    }
                    let _ = handle.emit(
                        "claude-error",
                        ChatErrorPayload {
                            session_id: sid,
                            error: error.to_string(),
                        },
                    );
                }
                "system_init" => {
                    // Capture SDK session ID for resume
                    let sdk_sid = json
                        .get("sdk_session_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    if !sdk_sid.is_empty() && !sid.is_empty() {
                        if let Ok(mut sessions) = sessions.lock() {
                            if let Some(s) = sessions.iter_mut().find(|s| s.id == sid) {
                                s.sdk_session_id = Some(sdk_sid.clone());
                            }
                        }
                    }
                    // Forward the model id RESOLVED by the SDK (e.g. "claude-opus-4-8")
                    // so the frontend can show the real version, not just the alias.
                    let resolved_model = json
                        .get("model")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    if !resolved_model.is_empty() && !sid.is_empty() {
                        let _ = handle.emit(
                            "claude-model-resolved",
                            serde_json::json!({
                                "session_id": sid,
                                "model": resolved_model,
                            }),
                        );
                    }
                    // Log init info
                    if let Ok(mut log) =
                        std::fs::OpenOptions::new().create(true).append(true).open(&log_path)
                    {
                        let _ = writeln!(log, "[SIDECAR] Session init: sdk_sid={} {}", sdk_sid, &trimmed[..trimmed.len().min(200)]);
                    }
                }
                "sessions_list" => {
                    // Forward to frontend for chat management
                    let _ = handle.emit("claude-sessions-list", json.clone());
                }
                "classify_result" => {
                    // One-shot Haiku auto-title came back. Parse the JSON
                    // {"title": ...} and write it onto the session, respecting a
                    // hand-set title. classified_at is stamped only on success,
                    // so a transient failure (empty result, junk output) retries
                    // at the next idle instead of leaving the chat untitled.
                    let raw = json.get("result").and_then(|v| v.as_str()).unwrap_or("");
                    if sid.is_empty() {
                        continue;
                    }
                    if let Some(title) = parse_classify_result(raw) {
                        if let Ok(mut sessions) = sessions.lock() {
                            if let Some(s) = sessions.iter_mut().find(|s| s.id == sid) {
                                if !s.title_manual {
                                    s.title = title;
                                }
                                s.classified_at = Some(now_secs());
                            }
                            save_sessions_to_disk(&sessions);
                        }
                        let _ = handle.emit("chats-reclassified", serde_json::json!({ "session_id": sid }));
                    }
                }
                _ => {}
            }
        }

        // Sidecar stdout closed — process died
        if let Ok(mut log) =
            std::fs::OpenOptions::new().create(true).append(true).open(&log_path)
        {
            let _ = writeln!(log, "[SIDECAR] Process stdout closed — notifying frontend");
        }

        // Notify frontend for all busy sessions so they don't hang forever
        if let Ok(mut sessions_lock) = sessions.lock() {
            for session in sessions_lock.iter_mut() {
                if session.is_busy() {
                    session.set_busy(false);
                    let _ = handle.emit(
                        "claude-error",
                        ChatErrorPayload {
                            session_id: session.id.clone(),
                            error: "Sidecar process crashed — please try again".to_string(),
                        },
                    );
                }
            }
        }
    }
}

impl Drop for AgentSidecar {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

// ── Auto-title (Haiku one-shot) ─────────────────────────────

/// Parse the Haiku auto-titler's raw output into a title. The model is asked
/// for bare JSON `{"title": ...}` but may wrap it in a ```json fence or add
/// prose — strip to the outermost object and parse. Returns None if
/// unparseable or the title is empty/placeholder.
pub fn parse_classify_result(raw: &str) -> Option<String> {
    let start = raw.find('{')?;
    let end = raw.rfind('}')?;
    if end < start {
        return None;
    }
    let slice = &raw[start..=end];
    let v: serde_json::Value = serde_json::from_str(slice).ok()?;
    let s = v.get("title")?.as_str()?.trim().to_string();
    let low = s.to_lowercase();
    if s.is_empty() || low == "null" || low == "none" || low == "n/a" {
        None
    } else {
        Some(s)
    }
}

/// Build the user-side prompt for titling a session: its current title plus the
/// first couple of user messages (truncated) and a bit of assistant context.
pub fn build_classify_prompt(session: &ChatSession) -> String {
    let mut parts = Vec::new();
    parts.push(format!("Titolo attuale: {}", session.title));

    let mut shown = 0;
    for m in session.messages.iter() {
        if m.role == "user" {
            let snippet: String = m.content.chars().take(500).collect();
            parts.push(format!("Messaggio utente: {}", snippet));
            shown += 1;
            if shown >= 2 {
                break;
            }
        }
    }
    // A little assistant context helps disambiguate the topic.
    if let Some(a) = session.messages.iter().find(|m| m.role == "assistant") {
        let snippet: String = a.content.chars().take(400).collect();
        if !snippet.trim().is_empty() {
            parts.push(format!("Risposta assistente (estratto): {}", snippet));
        }
    }

    parts.join("\n")
}

// ── Session persistence ─────────────────────────────────────

fn sessions_file_path() -> std::path::PathBuf {
    config::get_base_dir().join("jupiteros-sessions.json")
}

fn debug_log(msg: &str) {
    if let Ok(mut log) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(config::get_base_dir().join(".claude-gui-debug.log"))
    {
        let _ = writeln!(log, "{}", msg);
    }
}

pub fn save_sessions_to_disk(sessions: &[ChatSession]) {
    let persist: Vec<ChatSessionPersist> = sessions.iter().map(|s| s.to_persist()).collect();
    if let Ok(json) = serde_json::to_string_pretty(&persist) {
        // Atomic replace: a crash mid-write must never leave a truncated
        // sessions file (a truncated file reads as "no sessions" and the next
        // save would make the loss permanent).
        let path = sessions_file_path();
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, json).is_ok() {
            // Windows can't rename over an existing file.
            #[cfg(windows)]
            let _ = std::fs::remove_file(&path);
            let _ = std::fs::rename(&tmp, &path);
        }
    }
}

/// Parse the sessions file leniently: a corrupt session is skipped (and
/// counted), the valid ones survive. Err only when the content isn't a JSON
/// array at all (e.g. truncated file) — the caller quarantines it.
fn parse_sessions(content: &str) -> Result<(Vec<ChatSessionPersist>, usize), ()> {
    let values: Vec<serde_json::Value> = serde_json::from_str(content).map_err(|_| ())?;
    let mut ok = Vec::new();
    let mut skipped = 0usize;
    for v in values {
        match serde_json::from_value::<ChatSessionPersist>(v) {
            Ok(p) => ok.push(p),
            Err(_) => skipped += 1,
        }
    }
    Ok((ok, skipped))
}

pub fn load_sessions_from_disk() -> Vec<ChatSession> {
    let path = sessions_file_path();
    if !path.exists() {
        return Vec::new();
    }

    // Never overwrite silently: an unreadable/unparseable file is renamed
    // aside (quarantine) so the data survives on disk for manual recovery.
    let quarantine = || {
        let dest = path.with_extension(format!("json.corrotto-{}", now_secs()));
        let _ = std::fs::rename(&path, &dest);
        debug_log(&format!(
            "[SESSIONS] file illeggibile — messo in quarantena come {:?}",
            dest.file_name().unwrap_or_default()
        ));
    };

    match std::fs::read_to_string(&path) {
        Ok(content) => match parse_sessions(&content) {
            Ok((persisted, skipped)) => {
                if skipped > 0 {
                    debug_log(&format!(
                        "[SESSIONS] {} sessioni corrotte scartate, {} recuperate",
                        skipped,
                        persisted.len()
                    ));
                }
                persisted.into_iter().map(ChatSession::from_persist).collect()
            }
            Err(()) => {
                quarantine();
                Vec::new()
            }
        },
        Err(_) => {
            quarantine();
            Vec::new()
        }
    }
}

/// Rotate startup backups of the sessions file: .bak2→.bak3, .bak1→.bak2,
/// current→.bak1. Called once at startup, before anything can write.
pub fn backup_sessions_file() {
    let path = sessions_file_path();
    let non_empty = std::fs::metadata(&path).map(|m| m.len() > 2).unwrap_or(false);
    if !non_empty {
        return;
    }
    let bak = |n: u8| path.with_extension(format!("json.bak{}", n));
    let _ = std::fs::rename(bak(2), bak(3));
    let _ = std::fs::rename(bak(1), bak(2));
    let _ = std::fs::copy(&path, bak(1));
}

#[cfg(test)]
mod sessions_parse_tests {
    use super::parse_sessions;

    const VALID: &str = r#"{"id":"a","sdk_session_id":null,"messages":[],"model":"opus",
        "engine":"claude","title":"t","created_at":1,"updated_at":1}"#;

    #[test]
    fn broken_session_is_skipped_valid_survives() {
        let content = format!(r#"[{},{{"id":"rotta","messages":"non-un-array"}}]"#, VALID);
        let (ok, skipped) = parse_sessions(&content).unwrap();
        assert_eq!(ok.len(), 1);
        assert_eq!(ok[0].id, "a");
        assert_eq!(skipped, 1);
    }

    #[test]
    fn truncated_file_is_err() {
        let content = format!("[{}", VALID); // troncato: niente ']' finale
        assert!(parse_sessions(&content).is_err());
    }

    #[test]
    fn unknown_fields_are_ignored() {
        // Vecchi campi (company/topic/pinned) non devono rompere il load.
        let content = format!(
            r#"[{}]"#,
            VALID.replace(r#""title":"t""#, r#""title":"t","company":"X","pinned":true"#)
        );
        let (ok, skipped) = parse_sessions(&content).unwrap();
        assert_eq!((ok.len(), skipped), (1, 0));
    }
}
