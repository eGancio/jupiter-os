use std::sync::{Arc, Mutex};

use tauri::AppHandle;

use crate::claude::{AgentSidecar, ChatSession, load_sessions_from_disk, save_sessions_to_disk};
use crate::config;

pub struct ChatState {
    pub sessions: Arc<Mutex<Vec<ChatSession>>>,
    pub model: Arc<Mutex<String>>,
    pub sidecar: Arc<Mutex<Option<AgentSidecar>>>,
}

impl ChatState {
    pub fn new() -> Self {
        // Load saved sessions from disk
        let saved = load_sessions_from_disk();
        Self {
            sessions: Arc::new(Mutex::new(saved)),
            model: Arc::new(Mutex::new("sonnet".to_string())),
            sidecar: Arc::new(Mutex::new(None)),
        }
    }

    /// Initialize the sidecar process. Call once during app setup.
    pub fn init_sidecar(&self, app_handle: AppHandle) {
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
        if let Ok(sessions) = self.sessions.lock() {
            for s in sessions.iter() {
                let mut cmd = serde_json::json!({
                    "cmd": "create_session",
                    "id": s.id,
                    "model": s.model,
                    "cwd": cwd.to_string_lossy().to_string(),
                    "mcp_config": mcp_config.to_string_lossy().to_string(),
                });
                // If we have a saved SDK session ID, include it for resume
                if let Some(ref sdk_id) = s.sdk_session_id {
                    cmd["sdk_session_id"] = serde_json::json!(sdk_id);
                }
                let _ = self.send_to_sidecar(cmd);
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

    /// Send a JSON command to the sidecar.
    pub fn send_to_sidecar(&self, cmd: serde_json::Value) -> Result<(), String> {
        let guard = self.sidecar.lock().map_err(|e| e.to_string())?;
        match guard.as_ref() {
            Some(sc) => sc.send_command(cmd),
            None => Err("Sidecar not running".to_string()),
        }
    }

    /// Create a new session and tell the sidecar about it.
    pub fn create_session(&self) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let model = self.model.lock().unwrap().clone();
        let mut session = ChatSession::new(id.clone());
        session.model = model.clone();

        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.push(session);
            save_sessions_to_disk(&sessions);
        }

        // Tell sidecar to create an SDK session
        let mcp_config = config::mcp_json_path();
        let cwd = config::get_base_dir();
        let _ = self.send_to_sidecar(serde_json::json!({
            "cmd": "create_session",
            "id": id,
            "model": model,
            "cwd": cwd.to_string_lossy(),
            "mcp_config": mcp_config.to_string_lossy(),
        }));

        id
    }

    pub fn get_model(&self) -> String {
        self.model.lock().unwrap().clone()
    }

    pub fn set_model(&self, model: String) {
        *self.model.lock().unwrap() = model;
    }
}
