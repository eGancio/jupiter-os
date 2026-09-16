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

/// Secret generico per gli env dei Moon: service "MoonEnv", account = nome
/// variabile (es. TAVILY_API_KEY). Si popola con:
///   security add-generic-password -U -s MoonEnv -a <VAR> -w '<valore>'
/// `None` se assente o vuoto.
pub fn env_keyring_get(var: &str) -> Option<String> {
    keyring::Entry::new("MoonEnv", var)
        .ok()
        .and_then(|e| e.get_password().ok())
        .filter(|s| !s.is_empty())
}

/// Espande i placeholder `${VAR}` negli env della sezione `servers`/`daemons`
/// di .mcp.json: prima credentials.env, poi il keyring (service "MoonEnv").
/// Così i segreti possono uscire dal .mcp.json in chiaro (stesso spirito di
/// MoonAds, che però copre solo il blocco mcpServers via sidecar).
///
/// Un placeholder irrisolto viene RIMOSSO, non lasciato letterale: la stringa
/// "${...}" passata com'è è la trappola peggiore (moon-io la userebbe come
/// password IMAP reale), mentre una variabile assente fa scattare i fallback
/// nativi dei Moon (keyring MoonIo, feature disabilitata con errore chiaro).
pub fn expand_env_placeholders(
    env: &mut BTreeMap<String, String>,
    creds: &BTreeMap<String, String>,
) {
    let keys: Vec<String> = env.keys().cloned().collect();
    for k in keys {
        let Some(v) = env.get(&k) else { continue };
        let Some(var) = v.strip_prefix("${").and_then(|s| s.strip_suffix('}')) else {
            continue;
        };
        let var = var.to_string();
        let resolved = creds
            .get(&var)
            .cloned()
            .filter(|s| !s.is_empty())
            .or_else(|| env_keyring_get(&var));
        match resolved {
            Some(val) => {
                env.insert(k, val);
            }
            None => {
                env.remove(&k);
            }
        }
    }
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

// ── Risoluzione del binario `node` ───────────────────────────

/// Percorso assoluto di `node`, risolto una volta sola.
///
/// Non ci si può affidare al `PATH` ereditato: un'app macOS lanciata da
/// Finder/Dock/Launchpad riceve solo `/usr/bin:/bin:/usr/sbin:/sbin` (verificato:
/// `LSEnvironment` nell'Info.plist viene ignorato da LaunchServices), e lì `node`
/// non c'è quasi mai. Il risultato era un `Command::new("node")` che falliva con
/// un opaco "No such file or directory (os error 2)" — il sidecar chat non
/// partiva e la GUI restava senza motore.
///
/// Ordine: `JUPITEROS_NODE` esplicito → `PATH` (dev/terminale) → percorsi noti.
/// Se non si trova nulla si torna a `"node"`, così su Linux/Windows il
/// comportamento resta identico a prima.
pub fn node_binary() -> PathBuf {
    static CACHED: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    CACHED.get_or_init(resolve_node_binary).clone()
}

fn resolve_node_binary() -> PathBuf {
    // 1) Override esplicito, per installazioni non standard.
    if let Some(p) = std::env::var_os("JUPITEROS_NODE") {
        let p = PathBuf::from(p);
        if is_executable(&p) {
            return p;
        }
    }

    let exe_name = if cfg!(target_os = "windows") { "node.exe" } else { "node" };

    // 2) PATH ereditato: è quello giusto quando si lancia da terminale o in dev.
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            let cand = dir.join(exe_name);
            if is_executable(&cand) {
                return cand;
            }
        }
    }

    // 3) Percorsi noti, in ordine di preferenza.
    for cand in node_candidate_paths(exe_name) {
        if is_executable(&cand) {
            return cand;
        }
    }

    // 4) Ultima spiaggia: lascia decidere all'OS (e produrre l'errore).
    PathBuf::from(exe_name)
}

/// Directory in cui cercare `node` quando il `PATH` non aiuta. Include le
/// installazioni "utente" (tarball ufficiale scompattato in ~/.local/tools,
/// nvm, fnm, volta) oltre ai prefissi di sistema.
fn node_candidate_paths(exe_name: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let home = std::env::var_os("HOME").map(PathBuf::from);

    // Tarball ufficiale scompattato: ~/.local/tools/node-v22.../bin/node.
    // Più versioni possono convivere: si prende la maggiore in ordine di nome.
    if let Some(ref h) = home {
        for base in [h.join(".local/tools"), h.join(".nvm/versions/node"), h.join(".local/share/fnm/node-versions")] {
            if let Ok(entries) = std::fs::read_dir(&base) {
                let mut versions: Vec<PathBuf> = entries
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| {
                        p.file_name()
                            .and_then(|n| n.to_str())
                            .is_some_and(|n| n.starts_with("node") || n.starts_with('v'))
                    })
                    .collect();
                versions.sort();
                for v in versions.into_iter().rev() {
                    // fnm annida un ulteriore livello "installation/"
                    out.push(v.join("bin").join(exe_name));
                    out.push(v.join("installation").join("bin").join(exe_name));
                }
            }
        }
        out.push(h.join(".volta/bin").join(exe_name));
        out.push(h.join(".local/bin").join(exe_name));
    }

    out.push(PathBuf::from("/opt/homebrew/bin").join(exe_name)); // macOS arm64
    out.push(PathBuf::from("/usr/local/bin").join(exe_name)); // macOS Intel / Linux
    out.push(PathBuf::from("/usr/bin").join(exe_name));
    out
}

fn is_executable(p: &std::path::Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        p.is_file()
    }
}
