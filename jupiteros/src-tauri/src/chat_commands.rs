// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::chat_state::ChatState;
use crate::claude::{self, ChatMsg, ToolCallInfo};
use crate::config;

#[derive(Serialize)]
pub struct ChatSessionInfo {
    pub id: String,
    pub title: String,
    pub message_count: usize,
}

#[derive(Serialize)]
pub struct ClaudeMdStatus {
    pub exists: bool,
    pub path: String,
    pub size_bytes: u64,
}

#[tauri::command]
pub fn create_chat_session(state: State<'_, ChatState>) -> String {
    state.create_session()
}

#[tauri::command]
pub fn send_chat_message(
    session_id: String,
    message: String,
    images: Vec<String>,
    state: State<'_, ChatState>,
) -> Result<(), String> {
    // Record user message and mark busy
    {
        let mut sessions = state.sessions.lock().map_err(|e| e.to_string())?;
        let session = sessions
            .iter_mut()
            .find(|s| s.id == session_id)
            .ok_or_else(|| format!("Session {} not found", session_id))?;

        if session.is_busy() {
            return Err("Chat is busy".to_string());
        }
        session.set_busy(true);
        session.add_user_message(message.clone());
    }

    // Send to sidecar
    let mut cmd = serde_json::json!({
        "cmd": "send",
        "session_id": session_id,
        "message": message,
    });

    // Include images if any (base64 encoded)
    if !images.is_empty() {
        cmd["images"] = serde_json::json!(images);
    }

    state.send_to_sidecar(cmd).map_err(|e| {
        // Log the failure so it appears in the debug file
        let log_path = crate::config::get_base_dir().join(".claude-gui-debug.log");
        if let Ok(mut log) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
        {
            use std::io::Write;
            let _ = writeln!(log, "[SEND-ERROR] session={} error={}", session_id, e);
        }
        // Unmark busy on send failure
        if let Ok(mut sessions) = state.sessions.lock() {
            if let Some(s) = sessions.iter_mut().find(|s| s.id == session_id) {
                s.set_busy(false);
            }
        }
        e
    })
}

#[tauri::command]
pub fn stop_chat(session_id: String, state: State<'_, ChatState>) -> Result<(), String> {
    // Always unmark busy so the session is usable again, even if the sidecar
    // is stuck on a hanging MCP tool call and doesn't respond in time.
    if let Ok(mut sessions) = state.sessions.lock() {
        if let Some(s) = sessions.iter_mut().find(|s| s.id == session_id) {
            s.set_busy(false);
        }
    }

    state.send_to_sidecar(serde_json::json!({
        "cmd": "stop",
        "session_id": session_id,
    }))
}

#[tauri::command]
pub fn get_chat_messages(
    session_id: String,
    state: State<'_, ChatState>,
) -> Result<Vec<ChatMsg>, String> {
    let sessions = state.sessions.lock().map_err(|e| e.to_string())?;
    let session = sessions
        .iter()
        .find(|s| s.id == session_id)
        .ok_or_else(|| format!("Session {} not found", session_id))?;
    Ok(session.messages.clone())
}

#[tauri::command]
pub fn list_chat_sessions(state: State<'_, ChatState>) -> Vec<ChatSessionInfo> {
    let sessions = match state.sessions.lock() {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    sessions
        .iter()
        .map(|s| ChatSessionInfo {
            id: s.id.clone(),
            title: s.title.clone(),
            message_count: s.messages.len(),
        })
        .collect()
}

#[tauri::command]
pub fn set_chat_model(model: String, state: State<'_, ChatState>) {
    state.set_model(model.clone());

    // Also notify all active sessions in the sidecar
    if let Ok(sessions) = state.sessions.lock() {
        for s in sessions.iter() {
            let _ = state.send_to_sidecar(serde_json::json!({
                "cmd": "set_model",
                "session_id": s.id,
                "model": model,
            }));
        }
    }
}

#[tauri::command]
pub fn delete_chat_session(
    session_id: String,
    state: State<'_, ChatState>,
) -> Result<(), String> {
    // Tell sidecar to dispose
    let _ = state.send_to_sidecar(serde_json::json!({
        "cmd": "dispose",
        "session_id": session_id,
    }));

    // Remove from local state and save
    let mut sessions = state.sessions.lock().map_err(|e| e.to_string())?;
    sessions.retain(|s| s.id != session_id);
    crate::claude::save_sessions_to_disk(&sessions);
    Ok(())
}

#[tauri::command]
pub fn compact_chat_session(
    session_id: String,
    state: State<'_, ChatState>,
) -> Result<(), String> {
    // Reset the SDK session ID so next query starts with a fresh context window,
    // but keep the same UI session (no new chat in sidebar).
    {
        let mut sessions = state.sessions.lock().map_err(|e| e.to_string())?;
        let session = sessions
            .iter_mut()
            .find(|s| s.id == session_id)
            .ok_or_else(|| format!("Session {} not found", session_id))?;
        session.sdk_session_id = None;
        crate::claude::save_sessions_to_disk(&sessions);
    }

    // Tell sidecar to reset the SDK session
    state.send_to_sidecar(serde_json::json!({
        "cmd": "compact_session",
        "session_id": session_id,
    }))
}

#[tauri::command]
pub fn rename_chat_session(
    session_id: String,
    title: String,
    state: State<'_, ChatState>,
) -> Result<(), String> {
    let mut sessions = state.sessions.lock().map_err(|e| e.to_string())?;
    let session = sessions
        .iter_mut()
        .find(|s| s.id == session_id)
        .ok_or_else(|| format!("Session {} not found", session_id))?;
    session.title = title;
    crate::claude::save_sessions_to_disk(&sessions);
    Ok(())
}

#[tauri::command]
pub fn get_chat_model(state: State<'_, ChatState>) -> String {
    state.get_model()
}

#[tauri::command]
pub fn read_chart_file(path: String) -> Result<String, String> {
    std::fs::read_to_string(&path).map_err(|e| format!("Cannot read chart file: {}", e))
}

/// Frontend-friendly payload for `flush_session_messages`.
/// Mirrors `ChatMsg` plus a `streaming` flag for in-progress assistant rows.
#[derive(Deserialize)]
pub struct FlushChatMsg {
    pub id: String,
    pub role: String,
    pub content: String,
    #[serde(default)]
    pub thinking: String,
    #[serde(default)]
    pub tool_calls: Vec<ToolCallInfo>,
    #[serde(default)]
    pub timestamp: u64,
    #[serde(default)]
    pub streaming: bool,
}

/// Persist a debounced snapshot of the React-side message list for a session.
/// Called every ~500ms during streaming so a sidecar crash never loses more
/// than half a second of accumulated deltas (text + tool calls).
///
/// Replaces `session.messages` with the snapshot. The `done` handler in
/// `claude.rs` pops the trailing streaming placeholder before pushing the
/// finalized assistant message, so no duplicates accumulate on normal flow.
#[tauri::command]
pub fn flush_session_messages(
    session_id: String,
    messages: Vec<FlushChatMsg>,
    state: State<'_, ChatState>,
) -> Result<(), String> {
    let mut sessions = state.sessions.lock().map_err(|e| e.to_string())?;
    let session = sessions
        .iter_mut()
        .find(|s| s.id == session_id)
        .ok_or_else(|| format!("Session {} not found", session_id))?;

    session.messages = messages
        .into_iter()
        .map(|m| ChatMsg {
            id: m.id,
            role: m.role,
            content: m.content,
            thinking: m.thinking,
            tool_calls: m.tool_calls,
            timestamp: m.timestamp,
            streaming: m.streaming,
        })
        .collect();
    session.updated_at = claude::now_secs();

    claude::save_sessions_to_disk(&sessions);
    Ok(())
}

#[tauri::command]
pub fn get_claude_md_status() -> ClaudeMdStatus {
    let path = config::get_base_dir().join("CLAUDE.md");
    let (exists, size) = if path.exists() {
        let size = std::fs::metadata(&path)
            .map(|m| m.len())
            .unwrap_or(0);
        (true, size)
    } else {
        (false, 0)
    };
    ClaudeMdStatus {
        exists,
        path: path.to_string_lossy().to_string(),
        size_bytes: size,
    }
}
