// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::config::{ProcessKind, ServerDef, MAX_LOG_LINES};

#[derive(Clone, Serialize)]
pub struct LogLinePayload {
    pub service: String,
    pub line: String,
}

pub struct ServiceState {
    pub name: String,
    pub def: ServerDef,
    pub kind: ProcessKind,
    pub running: bool,
    child: Option<Child>,
    pub logs: Arc<Mutex<VecDeque<String>>>,
}

impl ServiceState {
    pub fn new(name: String, def: ServerDef, kind: ProcessKind) -> Self {
        Self {
            name,
            def,
            kind,
            running: false,
            child: None,
            logs: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    pub fn start(&mut self, app_handle: Option<AppHandle>) {
        if self.running {
            return;
        }

        // Kill any orphaned process already on our MCP port (e.g. from a
        // previous JupiterOS session that wasn't tracked in self.child).
        if let Some(port_str) = self.def.env.get("MCP_PORT") {
            if let Ok(port) = port_str.parse::<u16>() {
                kill_process_on_port(port, &self.logs);
                // Give the OS a moment to release the port after SIGKILL
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
        }

        // Re-read env from .mcp.json so credentials added after startup are picked up
        self.refresh_env_from_disk();

        let mut cmd = Command::new(&self.def.command);
        cmd.args(&self.def.args);
        cmd.envs(&self.def.env);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.stdin(Stdio::piped());

        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }

        if let Some(ref cwd) = self.def.cwd {
            cmd.current_dir(cwd);
        }

        match cmd.spawn() {
            Ok(mut child) => {
                // Spawn stderr reader thread
                if let Some(stderr) = child.stderr.take() {
                    let logs = Arc::clone(&self.logs);
                    let handle = app_handle.clone();
                    let svc_name = self.name.clone();
                    std::thread::spawn(move || {
                        let reader = BufReader::new(stderr);
                        for line in reader.lines() {
                            if let Ok(line) = line {
                                let formatted = format_log(&line);
                                push_log(&logs, &formatted);
                                if let Some(ref h) = handle {
                                    let _ = h.emit(
                                        "log-line",
                                        LogLinePayload {
                                            service: svc_name.clone(),
                                            line: formatted,
                                        },
                                    );
                                }
                            }
                        }
                    });
                }

                // Spawn stdout reader thread
                if let Some(stdout) = child.stdout.take() {
                    let logs = Arc::clone(&self.logs);
                    let handle = app_handle;
                    let svc_name = self.name.clone();
                    std::thread::spawn(move || {
                        let reader = BufReader::new(stdout);
                        for line in reader.lines() {
                            if let Ok(line) = line {
                                let formatted = format!("{} [stdout] {}", timestamp(), line);
                                push_log(&logs, &formatted);
                                if let Some(ref h) = handle {
                                    let _ = h.emit(
                                        "log-line",
                                        LogLinePayload {
                                            service: svc_name.clone(),
                                            line: formatted,
                                        },
                                    );
                                }
                            }
                        }
                    });
                }

                self.child = Some(child);
                self.running = true;

                let kind_str = match self.kind {
                    ProcessKind::McpServer => "Server",
                    ProcessKind::Daemon => "Daemon",
                };
                let msg = format!("{} {} started", timestamp(), kind_str);
                push_log(&self.logs, &msg);
            }
            Err(e) => {
                let msg = format!("{} Failed to start: {}", timestamp(), e);
                push_log(&self.logs, &msg);
            }
        }
    }

    /// Re-read this service's env vars from .mcp.json so that credentials
    /// added after JupiterOS startup are picked up on the next Start.
    fn refresh_env_from_disk(&mut self) {
        use crate::config::{load_credentials_env, mcp_json_path, McpConfig};
        let Ok(raw) = std::fs::read_to_string(mcp_json_path()) else { return };
        let Ok(cfg) = serde_json::from_str::<McpConfig>(&raw) else { return };
        let source = if cfg.servers.contains_key(&self.name) {
            cfg.servers.get(&self.name)
        } else {
            cfg.daemons.get(&self.name)
        };
        if let Some(server_def) = source {
            let creds = load_credentials_env();
            let mut env = server_def.env.clone();
            for (k, v) in &creds {
                env.entry(k.clone()).or_insert_with(|| v.clone());
            }
            crate::config::expand_env_placeholders(&mut env, &creds);
            self.def.env = env;
        }
    }

    pub fn stop(&mut self) {
        if let Some(ref mut child) = self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.child = None;
        self.running = false;

        let kind_str = match self.kind {
            ProcessKind::McpServer => "Server",
            ProcessKind::Daemon => "Daemon",
        };
        let msg = format!("{} {} stopped", timestamp(), kind_str);
        push_log(&self.logs, &msg);
    }

    /// Returns true if status changed (process died unexpectedly)
    pub fn check_alive(&mut self) -> bool {
        if !self.running {
            return false;
        }
        if let Some(ref mut child) = self.child {
            match child.try_wait() {
                Ok(Some(status)) => {
                    let msg = format!("{} Process exited: {}", timestamp(), status);
                    push_log(&self.logs, &msg);
                    self.child = None;
                    self.running = false;
                    return true;
                }
                Err(e) => {
                    let msg = format!("{} Error checking process: {}", timestamp(), e);
                    push_log(&self.logs, &msg);
                    self.child = None;
                    self.running = false;
                    return true;
                }
                Ok(None) => {} // still running
            }
        }
        false
    }
}

fn timestamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let h = (secs / 3600) % 24;
    let m = (secs / 60) % 60;
    let s = secs % 60;
    format!("[{:02}:{:02}:{:02}]", h, m, s)
}

fn format_log(line: &str) -> String {
    format!("{} {}", timestamp(), line)
}

fn push_log(logs: &Arc<Mutex<VecDeque<String>>>, msg: &str) {
    if let Ok(mut buf) = logs.lock() {
        if buf.len() >= MAX_LOG_LINES {
            buf.pop_front();
        }
        buf.push_back(msg.to_string());
    }
}

/// Kill any process occupying `port` (TCP) so a fresh start can bind it.
/// Handles orphaned processes from previous JupiterOS sessions.
fn kill_process_on_port(port: u16, logs: &Arc<Mutex<VecDeque<String>>>) {
    #[cfg(target_os = "linux")]
    {
        // `fuser -k PORT/tcp` sends SIGKILL to every process bound to the port
        let out = Command::new("fuser")
            .args(["-k", &format!("{}/tcp", port)])
            .output();
        match out {
            Ok(o) if o.status.success() => {
                push_log(logs, &format!("{} Killed orphaned process on port {}", timestamp(), port));
            }
            _ => {} // Nothing was using the port — that's fine
        }
    }
    #[cfg(target_os = "windows")]
    {
        // netstat to find PID, then taskkill
        if let Ok(out) = Command::new("netstat").args(["-ano"]).output() {
            let text = String::from_utf8_lossy(&out.stdout);
            for line in text.lines() {
                if line.contains(&format!(":{} ", port)) && line.contains("LISTENING") {
                    if let Some(pid) = line.split_whitespace().last() {
                        let _ = Command::new("taskkill")
                            .args(["/PID", pid, "/F"])
                            .output();
                        push_log(logs, &format!("{} Killed orphaned process (PID {}) on port {}", timestamp(), pid, port));
                    }
                }
            }
        }
    }
}
