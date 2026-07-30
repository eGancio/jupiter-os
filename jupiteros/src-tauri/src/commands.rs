// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

use serde::Serialize;
use tauri::{AppHandle, State};

use crate::config::{mcp_json_path, ProcessKind, ServerDef};
use crate::services::ServiceState;
use crate::state::AppState;

#[derive(Serialize)]
pub struct ServiceInfo {
    pub name: String,
    pub kind: String,
    pub running: bool,
    pub command: String,
    pub args: Vec<String>,
    pub cwd: Option<String>,
}

#[derive(Serialize)]
pub struct LogLines {
    pub lines: Vec<String>,
}

fn kind_str(kind: ProcessKind) -> String {
    match kind {
        ProcessKind::McpServer => "McpServer".to_string(),
        ProcessKind::Daemon => "Daemon".to_string(),
    }
}

#[tauri::command]
pub fn get_services(state: State<AppState>) -> Vec<ServiceInfo> {
    let services = state.services.lock().unwrap();
    services
        .values()
        .map(|s| ServiceInfo {
            name: s.name.clone(),
            kind: kind_str(s.kind),
            running: s.running,
            command: s.def.command.clone(),
            args: s.def.args.clone(),
            cwd: s.def.cwd.clone(),
        })
        .collect()
}

/// Returns MCP servers declared in `.mcp.json` `mcpServers` block that are
/// NOT also present in the local `servers` block. These are "virtual moons":
/// either remote (sse/http) or stdio servers spawned on-demand by the chat
/// sidecar — the GUI cannot start/stop them but should still list them.
#[tauri::command]
pub fn get_virtual_moons(state: State<AppState>) -> Vec<ServiceInfo> {
    let local_names: std::collections::BTreeSet<String> = {
        let services = state.services.lock().unwrap();
        services.keys().cloned().collect()
    };

    let path = mcp_json_path();
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return Vec::new();
    };
    let Some(map) = value.get("mcpServers").and_then(|v| v.as_object()) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for (name, def) in map {
        if local_names.contains(name) {
            continue;
        }
        let transport = def.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let (command, args) = match transport {
            "sse" | "http" => {
                let url = def
                    .get("url")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                (url, Vec::new())
            }
            "stdio" => {
                let cmd = def
                    .get("command")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let args: Vec<String> = def
                    .get("args")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|x| x.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default();
                (cmd, args)
            }
            _ => (String::new(), Vec::new()),
        };
        out.push(ServiceInfo {
            name: name.clone(),
            kind: "McpServer".to_string(),
            running: true,
            command,
            args,
            cwd: None,
        });
    }
    out
}

#[tauri::command]
pub fn start_service(name: String, state: State<AppState>, app: AppHandle) -> Result<(), String> {
    let mut services = state.services.lock().unwrap();
    let svc = services.get_mut(&name).ok_or("Service not found")?;
    svc.start(Some(app));
    Ok(())
}

#[tauri::command]
pub fn stop_service(name: String, state: State<AppState>) -> Result<(), String> {
    let mut services = state.services.lock().unwrap();
    let svc = services.get_mut(&name).ok_or("Service not found")?;
    svc.stop();
    Ok(())
}

#[tauri::command]
pub fn restart_service(name: String, state: State<AppState>, app: AppHandle) -> Result<(), String> {
    let mut services = state.services.lock().unwrap();
    let svc = services.get_mut(&name).ok_or("Service not found")?;
    svc.stop();
    svc.start(Some(app));
    Ok(())
}

#[tauri::command]
pub fn start_all(state: State<AppState>, app: AppHandle) -> Result<(), String> {
    let mut services = state.services.lock().unwrap();
    for svc in services.values_mut() {
        if !svc.running {
            svc.start(Some(app.clone()));
        }
    }
    Ok(())
}

#[tauri::command]
pub fn stop_all(state: State<AppState>) -> Result<(), String> {
    let mut services = state.services.lock().unwrap();
    for svc in services.values_mut() {
        if svc.running {
            svc.stop();
        }
    }
    Ok(())
}

#[tauri::command]
pub fn get_logs(name: String, state: State<AppState>) -> Result<LogLines, String> {
    let services = state.services.lock().unwrap();
    let svc = services.get(&name).ok_or("Service not found")?;
    let logs = svc.logs.lock().unwrap();
    Ok(LogLines {
        lines: logs.iter().cloned().collect(),
    })
}

#[tauri::command]
pub fn clear_logs(name: String, state: State<AppState>) -> Result<(), String> {
    let services = state.services.lock().unwrap();
    let svc = services.get(&name).ok_or("Service not found")?;
    svc.logs.lock().unwrap().clear();
    Ok(())
}

#[tauri::command]
pub fn add_local_moon(
    name: String,
    command: String,
    args: Vec<String>,
    cwd: Option<String>,
    env_vars: Vec<String>,
    port: u16,
    state: State<AppState>,
) -> Result<(), String> {
    use std::collections::BTreeMap;

    let path = mcp_json_path();
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|_| "{}".to_string());
    let mut json: serde_json::Value =
        serde_json::from_str(&raw).map_err(|e| format!("Invalid .mcp.json: {}", e))?;

    // Ensure required sections exist
    if json.get("servers").is_none() {
        json["servers"] = serde_json::json!({});
    }
    if json.get("mcpServers").is_none() {
        json["mcpServers"] = serde_json::json!({});
    }

    // Guard: name must be unique
    if json["servers"].get(&name).is_some() {
        return Err(format!("Moon '{}' esiste già", name));
    }

    // Build env map
    let mut env: BTreeMap<String, String> = BTreeMap::new();
    env.insert("MCP_TRANSPORT".to_string(), "sse".to_string());
    env.insert("MCP_PORT".to_string(), port.to_string());
    for kv in &env_vars {
        if let Some((k, v)) = kv.split_once('=') {
            env.insert(k.trim().to_string(), v.trim().to_string());
        }
    }

    // Write into "servers"
    let mut entry = serde_json::json!({
        "command": command,
        "args": args,
        "env": env,
    });
    if let Some(ref c) = cwd {
        entry["cwd"] = serde_json::json!(c);
    }
    json["servers"][&name] = entry;

    // Write into "mcpServers"
    json["mcpServers"][&name] = serde_json::json!({
        "type": "sse",
        "url": format!("http://127.0.0.1:{}/sse", port),
    });

    // Persist
    let pretty = serde_json::to_string_pretty(&json).map_err(|e| e.to_string())?;
    std::fs::write(&path, pretty)
        .map_err(|e| format!("Impossibile scrivere .mcp.json: {}", e))?;

    // Hot-reload: inject into live AppState (no restart needed)
    let def = ServerDef { command, args, cwd, env, autostart: true };
    state
        .services
        .lock()
        .unwrap()
        .insert(name.clone(), ServiceState::new(name, def, ProcessKind::McpServer));

    Ok(())
}

#[tauri::command]
pub fn add_remote_moon(name: String, url: String, moon_type: String) -> Result<(), String> {
    let path = mcp_json_path();
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|_| "{}".to_string());
    let mut json: serde_json::Value =
        serde_json::from_str(&raw).map_err(|e| format!("Invalid .mcp.json: {}", e))?;

    if json.get("mcpServers").is_none() {
        json["mcpServers"] = serde_json::json!({});
    }
    if json["mcpServers"].get(&name).is_some() {
        return Err(format!("Moon '{}' esiste già", name));
    }

    json["mcpServers"][&name] = serde_json::json!({
        "type": moon_type,
        "url": url,
    });

    let pretty = serde_json::to_string_pretty(&json).map_err(|e| e.to_string())?;
    std::fs::write(&path, pretty)
        .map_err(|e| format!("Impossibile scrivere .mcp.json: {}", e))?;

    Ok(())
}

#[tauri::command]
pub fn remove_moon(name: String, state: State<AppState>) -> Result<(), String> {
    // Stop + remove from live state (no-op for remote moons)
    {
        let mut services = state.services.lock().unwrap();
        if let Some(svc) = services.get_mut(&name) {
            svc.stop();
        }
        services.remove(&name);
    }

    let path = mcp_json_path();
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| format!("Impossibile leggere .mcp.json: {}", e))?;
    let mut json: serde_json::Value =
        serde_json::from_str(&raw).map_err(|e| format!("Invalid .mcp.json: {}", e))?;

    if let Some(s) = json.get_mut("servers").and_then(|v| v.as_object_mut()) {
        s.remove(&name);
    }
    if let Some(m) = json.get_mut("mcpServers").and_then(|v| v.as_object_mut()) {
        m.remove(&name);
    }

    let pretty = serde_json::to_string_pretty(&json).map_err(|e| e.to_string())?;
    std::fs::write(&path, pretty)
        .map_err(|e| format!("Impossibile scrivere .mcp.json: {}", e))?;

    Ok(())
}

#[tauri::command]
pub fn remove_local_moon(name: String, state: State<AppState>) -> Result<(), String> {
    // Stop + remove from live state
    {
        let mut services = state.services.lock().unwrap();
        if let Some(svc) = services.get_mut(&name) {
            svc.stop();
        }
        services.remove(&name);
    }

    // Remove from .mcp.json
    let path = mcp_json_path();
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| format!("Impossibile leggere .mcp.json: {}", e))?;
    let mut json: serde_json::Value =
        serde_json::from_str(&raw).map_err(|e| format!("Invalid .mcp.json: {}", e))?;

    if let Some(s) = json.get_mut("servers").and_then(|v| v.as_object_mut()) {
        s.remove(&name);
    }
    if let Some(m) = json.get_mut("mcpServers").and_then(|v| v.as_object_mut()) {
        m.remove(&name);
    }

    let pretty = serde_json::to_string_pretty(&json).map_err(|e| e.to_string())?;
    std::fs::write(&path, pretty)
        .map_err(|e| format!("Impossibile scrivere .mcp.json: {}", e))?;

    Ok(())
}

#[tauri::command]
pub fn is_autostart_enabled() -> bool {
    crate::autostart::is_autostart_enabled()
}

#[tauri::command]
pub fn set_autostart(enabled: bool) -> Result<(), String> {
    crate::autostart::set_autostart(enabled);
    Ok(())
}
