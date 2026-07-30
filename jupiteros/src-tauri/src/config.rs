// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

pub const MAX_LOG_LINES: usize = 500;

pub fn get_base_dir() -> PathBuf {
    // Primary: directory containing the executable (works for both dev and release builds)
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            if dir.join(".mcp.json").exists() {
                return dir.to_path_buf();
            }
        }
    }
    // Secondary: current working directory (useful when running `cargo tauri dev`)
    if let Ok(cwd) = std::env::current_dir() {
        if cwd.join(".mcp.json").exists() {
            return cwd;
        }
    }
    // Final fallback: XDG config dir on Linux/macOS, AppData on Windows
    #[cfg(target_os = "windows")]
    {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            return PathBuf::from(appdata).join("jupiteros");
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(".config").join("jupiteros");
        }
    }
    std::env::temp_dir().join("jupiteros")
}

pub fn mcp_json_path() -> PathBuf {
    get_base_dir().join(".mcp.json")
}

pub fn logo_path() -> PathBuf {
    get_base_dir().join("logo-ZIO.png")
}

pub fn load_credentials_env() -> BTreeMap<String, String> {
    let path = get_base_dir().join("credentials.env");
    let mut map = BTreeMap::new();
    if let Ok(content) = std::fs::read_to_string(&path) {
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((key, value)) = line.split_once('=') {
                map.insert(key.trim().to_string(), value.trim().to_string());
            }
        }
    }
    map
}

#[derive(Deserialize, Clone)]
pub struct McpConfig {
    #[serde(rename = "mcpServers", default)]
    pub _mcp_servers: BTreeMap<String, serde_json::Value>,

    #[serde(default)]
    pub servers: BTreeMap<String, ServerDef>,

    #[serde(default)]
    pub daemons: BTreeMap<String, ServerDef>,
}

#[derive(Deserialize, Clone)]
pub struct ServerDef {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Start automatically at app launch. Default true; set false in
    /// .mcp.json to keep a server manual-start only.
    #[serde(default = "default_true")]
    pub autostart: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ProcessKind {
    McpServer,
    Daemon,
}
