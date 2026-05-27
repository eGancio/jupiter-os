use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

const DEV_BASE: &str = r"c:\Users\Edoardo\PycharmProjects\Tool-AI";

pub const MAX_LOG_LINES: usize = 500;

pub fn get_base_dir() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join(".mcp.json");
            if candidate.exists() {
                return dir.to_path_buf();
            }
        }
    }
    PathBuf::from(DEV_BASE)
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
}

#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ProcessKind {
    McpServer,
    Daemon,
}
