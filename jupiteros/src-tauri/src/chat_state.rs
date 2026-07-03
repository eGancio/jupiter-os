// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

use std::sync::{Arc, Mutex};

use tauri::AppHandle;

use crate::claude::{AgentSidecar, ChatSession, load_sessions_from_disk, save_sessions_to_disk};
use crate::config;

pub struct ChatState {
    pub sessions: Arc<Mutex<Vec<ChatSession>>>,
    pub model: Arc<Mutex<String>>,
    pub engine: Arc<Mutex<String>>,
    /// Permission mode for the chat ("auto" | "plan" | "bypass"). Claude only;
    /// other engines ignore it. Default "auto".
    pub permission_mode: Arc<Mutex<String>>,
    pub sidecar: Arc<Mutex<Option<AgentSidecar>>>,
    /// Kept so we can respawn the sidecar after a crash (broken pipe).
    pub app_handle: Arc<Mutex<Option<AppHandle>>>,
}

impl ChatState {
    pub fn new() -> Self {
        // Load saved sessions from disk
        let saved = load_sessions_from_disk();
        Self {
            sessions: Arc::new(Mutex::new(saved)),
            model: Arc::new(Mutex::new("opus".to_string())),
            engine: Arc::new(Mutex::new("claude".to_string())),
            permission_mode: Arc::new(Mutex::new("auto".to_string())),
            sidecar: Arc::new(Mutex::new(None)),
            app_handle: Arc::new(Mutex::new(None)),
        }
    }

    /// Initialize the sidecar process. Call once during app setup.
    pub fn init_sidecar(&self, app_handle: AppHandle) {
        // Remember the handle so send_to_sidecar can respawn after a crash.
        if let Ok(mut guard) = self.app_handle.lock() {
            *guard = Some(app_handle.clone());
        }
        let sessions = Arc::clone(&self.sessions);
        match AgentSidecar::spawn(app_handle, sessions) {
            Ok(sc) => {
                if let Ok(mut guard) = self.sidecar.lock() {
                    *guard = Some(sc);
                }
                // Register restored sessions with the sidecar
                self.register_restored_sessions();
            }
            Err(e) => {
                eprintln!("Failed to init sidecar: {}", e);
                let log_path = config::get_base_dir().join(".claude-gui-debug.log");
                if let Ok(mut log) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&log_path)
                {
                    use std::io::Write;
                    let _ = writeln!(log, "[SIDECAR] Failed to spawn: {}", e);
                }
            }
        }
    }

    /// Register sessions loaded from disk with the sidecar so they can resume.
    fn register_restored_sessions(&self) {
        let mcp_config = config::mcp_json_path();
        let cwd = config::get_base_dir();
        let permission_mode = self.permission_mode.lock().map(|m| m.clone()).unwrap_or_else(|_| "auto".to_string());
        if let Ok(sessions) = self.sessions.lock() {
            for s in sessions.iter() {
                let mut cmd = serde_json::json!({
                    "cmd": "create_session",
                    "id": s.id,
                    "model": s.model,
                    "engine": s.engine,
                    "cwd": cwd.to_string_lossy().to_string(),
                    "mcp_config": mcp_config.to_string_lossy().to_string(),
                    "permission_mode": permission_mode,
                });
                // If we have a saved SDK session ID, include it for resume
                if let Some(ref sdk_id) = s.sdk_session_id {
                    cmd["sdk_session_id"] = serde_json::json!(sdk_id);
                }
                // send_once (not send_to_sidecar): respawn re-registers sessions,
                // so going through the respawn path here would recurse.
                let _ = self.send_once(cmd);
            }
        }
    }

    /// Stop the sidecar process (on app exit).
    pub fn stop_sidecar(&self) {
        if let Ok(mut guard) = self.sidecar.lock() {
            if let Some(ref mut sc) = *guard {
                sc.stop();
            }
            *guard = None;
        }
    }

    /// Save all sessions to disk.
    pub fn save_sessions(&self) {
        if let Ok(sessions) = self.sessions.lock() {
            save_sessions_to_disk(&sessions);
        }
    }

    /// Send a JSON command to the sidecar, transparently respawning it once if
    /// the pipe is dead. A long-lived chat used to die permanently the moment
    /// the sidecar crashed (broken pipe, os error 32) because nothing ever
    /// restarted it. Now the first failure triggers a respawn + retry, so the
    /// chat recovers on the next message instead of being stuck forever.
    pub fn send_to_sidecar(&self, cmd: serde_json::Value) -> Result<(), String> {
        // First attempt against the current sidecar.
        let first = self.send_once(cmd.clone());
        if first.is_ok() {
            return Ok(());
        }

        // The send failed (dead pipe or no sidecar): respawn once and retry.
        self.respawn_sidecar()
            .map_err(|e| format!("{} (after send error: {})", e, first.unwrap_err()))?;
        self.send_once(cmd)
    }

    /// Single send attempt against the current sidecar, with no respawn. Used by
    /// the retry path and by session re-registration to avoid respawn recursion.
    fn send_once(&self, cmd: serde_json::Value) -> Result<(), String> {
        let guard = self.sidecar.lock().map_err(|e| e.to_string())?;
        match guard.as_ref() {
            Some(sc) => sc.send_command(cmd),
            None => Err("Sidecar not running".to_string()),
        }
    }

    /// Kill any dead sidecar and start a fresh one, then re-register every known
    /// session so they resume with their saved SDK token (context preserved).
    fn respawn_sidecar(&self) -> Result<(), String> {
        let app_handle = {
            let guard = self.app_handle.lock().map_err(|e| e.to_string())?;
            guard.clone().ok_or("No app handle to respawn sidecar")?
        };
        let sessions = Arc::clone(&self.sessions);
        let new_sc = AgentSidecar::spawn(app_handle, sessions)
            .map_err(|e| format!("Sidecar respawn failed: {}", e))?;

        {
            let mut guard = self.sidecar.lock().map_err(|e| e.to_string())?;
            if let Some(ref mut old) = *guard {
                old.stop();
            }
            *guard = Some(new_sc);
        } // release the lock before re-registering (it calls send_to_sidecar)

        let log_path = config::get_base_dir().join(".claude-gui-debug.log");
        if let Ok(mut log) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
        {
            use std::io::Write;
            let _ = writeln!(log, "[SIDECAR] respawned after pipe failure");
        }

        // Re-register restored sessions with the fresh sidecar so the pending
        // "send" we're about to retry finds its session (and resumes context).
        self.register_restored_sessions();
        Ok(())
    }

    /// Public: restart the sidecar so freshly-saved environment (e.g. Ads API
    /// keys entered in the GUI) is picked up. Env is injected at spawn time, so
    /// keys added while the sidecar is running need a respawn to take effect.
    /// Sessions are re-registered and resume with their saved SDK token.
    pub fn restart_sidecar(&self) -> Result<(), String> {
        self.respawn_sidecar()
    }

    /// Create a new session and tell the sidecar about it.
    pub fn create_session(&self) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let model = self.model.lock().unwrap().clone();
        let engine = self.engine.lock().unwrap().clone();
        let mut session = ChatSession::new(id.clone());
        session.model = model.clone();
        session.engine = engine.clone();

        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.push(session);
            save_sessions_to_disk(&sessions);
        }

        // Tell sidecar to create an SDK session
        let mcp_config = config::mcp_json_path();
        let cwd = config::get_base_dir();
        let permission_mode = self.get_permission_mode();
        let _ = self.send_to_sidecar(serde_json::json!({
            "cmd": "create_session",
            "id": id,
            "model": model,
            "engine": engine,
            "cwd": cwd.to_string_lossy(),
            "mcp_config": mcp_config.to_string_lossy(),
            "permission_mode": permission_mode,
        }));

        id
    }

    pub fn get_model(&self) -> String {
        self.model.lock().unwrap().clone()
    }

    pub fn set_model(&self, model: String) {
        *self.model.lock().unwrap() = model;
    }

    pub fn get_engine(&self) -> String {
        self.engine.lock().unwrap().clone()
    }

    pub fn set_engine(&self, engine: String) {
        *self.engine.lock().unwrap() = engine;
    }

    pub fn get_permission_mode(&self) -> String {
        self.permission_mode.lock().unwrap().clone()
    }

    pub fn set_permission_mode(&self, mode: String) {
        *self.permission_mode.lock().unwrap() = mode;
    }
}
