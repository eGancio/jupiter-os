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
}

#[derive(Clone, Serialize)]
pub struct ToolInputDeltaPayload {
    pub session_id: String,
    pub tool_id: String,
    pub partial_json: String,
}

#[derive(Clone, Serialize)]
pub struct ToolResultPayload {
    pub session_id: String,
    pub tool_id: String,
    pub result: String,
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
    pub input: String,
    pub result: Option<String>,
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

/// Serializable subset for disk persistence
#[derive(Clone, Serialize, Deserialize)]
pub struct ChatSessionPersist {
    pub id: String,
    pub sdk_session_id: Option<String>,
    pub messages: Vec<ChatMsg>,
    pub model: String,
    pub title: String,
    pub created_at: u64,
    pub updated_at: u64,
}

pub struct ChatSession {
    pub id: String,
    pub sdk_session_id: Option<String>,
    pub messages: Vec<ChatMsg>,
    pub model: String,
    pub title: String,
    pub created_at: u64,
    pub updated_at: u64,
    busy: Arc<AtomicBool>,
}

impl ChatSession {
    pub fn new(id: String) -> Self {
        let now = now_secs();
        Self {
            id,
            sdk_session_id: None,
            messages: Vec::new(),
            model: "sonnet".to_string(),
            title: "Nuova chat".to_string(),
            created_at: now,
            updated_at: now,
            busy: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn from_persist(p: ChatSessionPersist) -> Self {
        Self {
            id: p.id,
            sdk_session_id: p.sdk_session_id,
            messages: p.messages,
            model: p.model,
            title: p.title,
            created_at: p.created_at,
            updated_at: p.updated_at,
            busy: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn to_persist(&self) -> ChatSessionPersist {
        ChatSessionPersist {
            id: self.id.clone(),
            sdk_session_id: self.sdk_session_id.clone(),
            messages: self.messages.clone(),
            model: self.model.clone(),
            title: self.title.clone(),
            created_at: self.created_at,
            updated_at: self.updated_at,
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

        // Auto-title from first user message
        if self.messages.len() == 1 {
            let t = &self.messages[0].content;
            self.title = if t.len() > 40 {
                format!("{}...", &t[..40])
            } else {
                t.clone()
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
        let reader = BufReader::new(stdout);
        let log_path = config::get_base_dir().join(".claude-gui-debug.log");

        // Accumulate assistant response per session for persistence
        let mut pending_text: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        let mut pending_thinking: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        let mut pending_tools: std::collections::HashMap<String, Vec<ToolCallInfo>> =
            std::collections::HashMap::new();

        for line in reader.lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => break,
            };
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            // Log to debug file
            if let Ok(mut log) =
                std::fs::OpenOptions::new().create(true).append(true).open(&log_path)
            {
                let _ = writeln!(
                    log,
                    "[SIDECAR-OUT] {}",
                    &trimmed[..trimmed.len().min(300)]
                );
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
                    // Track tool call for persistence
                    pending_tools.entry(sid.clone()).or_default().push(ToolCallInfo {
                        name: tool_name.to_string(),
                        input: String::new(),
                        result: None,
                    });
                    let _ = handle.emit(
                        "claude-tool-start",
                        ToolStartPayload {
                            session_id: sid,
                            tool_name: tool_name.to_string(),
                            tool_id: tool_id.to_string(),
                        },
                    );
                }
                "tool_input_delta" => {
                    let tool_id = json.get("tool_id").and_then(|v| v.as_str()).unwrap_or("");
                    let partial =
                        json.get("partial_json").and_then(|v| v.as_str()).unwrap_or("");
                    // Accumulate tool input for persistence
                    if let Some(tools) = pending_tools.get_mut(&sid) {
                        if let Some(last) = tools.last_mut() {
                            last.input.push_str(partial);
                        }
                    }
                    let _ = handle.emit(
                        "claude-tool-input-delta",
                        ToolInputDeltaPayload {
                            session_id: sid,
                            tool_id: tool_id.to_string(),
                            partial_json: partial.to_string(),
                        },
                    );
                }
                "tool_result" => {
                    let tool_id = json.get("tool_id").and_then(|v| v.as_str()).unwrap_or("");
                    let result = json.get("result").and_then(|v| v.as_str()).unwrap_or("");
                    // Store tool result for persistence
                    if let Some(tools) = pending_tools.get_mut(&sid) {
                        // Find matching tool (by id or last without result)
                        if let Some(tc) = tools.iter_mut().rev().find(|tc| tc.result.is_none()) {
                            tc.result = Some(result.to_string());
                        }
                    }
                    let _ = handle.emit(
                        "claude-tool-result",
                        ToolResultPayload {
                            session_id: sid.clone(),
                            tool_id: tool_id.to_string(),
                            result: result.to_string(),
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

// ── Session persistence ─────────────────────────────────────

fn sessions_file_path() -> std::path::PathBuf {
    config::get_base_dir().join("jupiteros-sessions.json")
}

pub fn save_sessions_to_disk(sessions: &[ChatSession]) {
    let persist: Vec<ChatSessionPersist> = sessions.iter().map(|s| s.to_persist()).collect();
    if let Ok(json) = serde_json::to_string_pretty(&persist) {
        let _ = std::fs::write(sessions_file_path(), json);
    }
}

pub fn load_sessions_from_disk() -> Vec<ChatSession> {
    let path = sessions_file_path();
    if !path.exists() {
        return Vec::new();
    }
    match std::fs::read_to_string(&path) {
        Ok(content) => {
            match serde_json::from_str::<Vec<ChatSessionPersist>>(&content) {
                Ok(persisted) => persisted.into_iter().map(ChatSession::from_persist).collect(),
                Err(_) => Vec::new(),
            }
        }
        Err(_) => Vec::new(),
    }
}
