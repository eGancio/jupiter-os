// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

//! GUI bridge for ingesting files/folders into Moon Metis (the knowledge layer)
//! straight from the filesystem — deterministic, NO chat/LLM roundtrip.
//!
//! Mirrors the "spawn the moon binary" idea: we run `moon-metis ingest …` in its
//! one-shot CLI mode (which reuses the exact extract→chunk→embed→index pipeline),
//! passing the same command/cwd/env declared for `metis` in `.mcp.json`. The CLI
//! prints a JSON array of per-file results on stdout, which we hand to the GUI.

use std::collections::BTreeMap;
use std::process::Command;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// One per-file ingest outcome (mirrors moon-metis `ingest::FileResult`).
#[derive(Serialize, Deserialize, Clone)]
pub struct IngestResult {
    pub source_path: String,
    pub status: String, // "indexed" | "skipped" | "scanned" | "empty" | "error"
    pub doc_id: String,
    pub title: String,
    pub nature: String,
    pub pages: u32,
    pub chunks: usize,
    pub message: String,
}

struct MetisDef {
    command: String,
    cwd: Option<String>,
    env: BTreeMap<String, String>,
}

/// Read the `metis` server definition (command/cwd/env) from `.mcp.json`, the
/// same source the app uses to spawn the Moon.
fn read_metis_def() -> Result<MetisDef, String> {
    let path = crate::config::mcp_json_path();
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| format!("Lettura .mcp.json: {e}"))?;
    let v: Value = serde_json::from_str(&raw).map_err(|e| format!(".mcp.json non valido: {e}"))?;

    let m = v
        .get("servers")
        .and_then(|s| s.get("metis"))
        .ok_or_else(|| "servers.metis assente in .mcp.json — registra il Moon Metis".to_string())?;

    let command = m
        .get("command")
        .and_then(|c| c.as_str())
        .ok_or_else(|| "servers.metis.command assente in .mcp.json".to_string())?
        .to_string();
    let cwd = m.get("cwd").and_then(|c| c.as_str()).map(|s| s.to_string());

    let mut env = BTreeMap::new();
    if let Some(obj) = m.get("env").and_then(|e| e.as_object()) {
        for (k, val) in obj {
            if let Some(s) = val.as_str() {
                env.insert(k.clone(), s.to_string());
            }
        }
    }

    Ok(MetisDef { command, cwd, env })
}

/// Ingest the given file/folder paths into Metis. `recursive` applies to folders.
/// Returns one result per file processed.
#[tauri::command]
pub async fn metis_ingest_paths(
    paths: Vec<String>,
    nature: Option<String>,
    recursive: bool,
    force: bool,
) -> Result<Vec<IngestResult>, String> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    let def = read_metis_def()?;

    let output = tokio::task::spawn_blocking(move || {
        let mut cmd = Command::new(&def.command);
        cmd.arg("ingest");
        if let Some(n) = nature.as_ref().filter(|s| !s.is_empty()) {
            cmd.arg("--nature").arg(n);
        }
        if recursive {
            cmd.arg("--recursive");
        }
        if force {
            cmd.arg("--force");
        }
        for p in &paths {
            cmd.arg(p);
        }
        if let Some(cwd) = &def.cwd {
            cmd.current_dir(cwd);
        }
        cmd.envs(&def.env);
        cmd.output()
    })
    .await
    .map_err(|e| format!("spawn: {e}"))?
    .map_err(|e| format!("Esecuzione moon-metis: {e}"))?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        let tail: Vec<&str> = err.lines().rev().take(6).collect();
        let tail: String = tail.into_iter().rev().collect::<Vec<_>>().join("\n");
        return Err(format!("moon-metis ingest fallito ({}):\n{}", output.status, tail));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json = extract_json_array(&stdout).ok_or_else(|| {
        format!(
            "Output inatteso da moon-metis: {}",
            stdout.chars().take(300).collect::<String>()
        )
    })?;

    serde_json::from_str::<Vec<IngestResult>>(json)
        .map_err(|e| format!("Parsing risultati ingest: {e}"))
}

/// Extract the outermost `[ … ]` JSON array from possibly-noisy output.
fn extract_json_array(s: &str) -> Option<&str> {
    let start = s.find('[')?;
    let end = s.rfind(']')?;
    (end >= start).then(|| &s[start..=end])
}

// ─────────────────────────────────────────────────────────────────────────────
// Listing / deleting indexed documents — read straight from Qdrant REST (no MCP,
// no embedding model), the same pattern as emails.rs.
// ─────────────────────────────────────────────────────────────────────────────

/// One indexed document (aggregated from its chunks).
#[derive(Serialize)]
pub struct MetisDoc {
    pub doc_id: String,
    pub title: String,
    pub source_path: String,
    pub nature: String,
    pub pages: u32,
    pub chunks: usize,
}

/// Qdrant REST base URL (port 6333) + collection name, from the `metis` env.
fn metis_qdrant_base_and_collection() -> Result<(String, String), String> {
    let def = read_metis_def()?;
    let url = def
        .env
        .get("QDRANT_URL")
        .cloned()
        .unwrap_or_else(|| "http://127.0.0.1:6334".to_string());
    let host = url
        .split("://")
        .nth(1)
        .unwrap_or(&url)
        .split(':')
        .next()
        .unwrap_or("127.0.0.1")
        .to_string();
    let coll = def
        .env
        .get("METIS_COLLECTION")
        .cloned()
        .unwrap_or_else(|| "metis_docs".to_string());
    Ok((format!("http://{host}:6333"), coll))
}

/// List the documents currently in the knowledge base (grouped from chunks).
#[tauri::command]
pub async fn metis_list_documents() -> Result<Vec<MetisDoc>, String> {
    let (base, coll) = metis_qdrant_base_and_collection()?;
    let client = reqwest::Client::new();
    let url = format!("{base}/collections/{coll}/points/scroll");

    let mut docs: BTreeMap<String, MetisDoc> = BTreeMap::new();
    let mut offset: Option<Value> = None;

    loop {
        let mut body = json!({ "limit": 256, "with_payload": true, "with_vector": false });
        if let Some(off) = &offset {
            body["offset"] = off.clone();
        }
        let resp = client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Qdrant non raggiungibile ({e}). Avvia Moon Metis/Io."))?;
        if resp.status().as_u16() == 404 {
            return Ok(Vec::new()); // collection not created yet
        }
        if !resp.status().is_success() {
            return Err(format!("Qdrant HTTP {}", resp.status()));
        }
        let v: Value = resp.json().await.map_err(|e| format!("decode: {e}"))?;

        for p in v["result"]["points"].as_array().cloned().unwrap_or_default() {
            let pl = &p["payload"];
            let doc_id = pl["doc_id"].as_str().unwrap_or("").to_string();
            if doc_id.is_empty() {
                continue;
            }
            let page = pl["page"].as_i64().unwrap_or(0) as u32;
            let entry = docs.entry(doc_id.clone()).or_insert_with(|| MetisDoc {
                doc_id: doc_id.clone(),
                title: pl["title"].as_str().unwrap_or("").to_string(),
                source_path: pl["source_path"].as_str().unwrap_or("").to_string(),
                nature: pl["nature"].as_str().unwrap_or("").to_string(),
                pages: 0,
                chunks: 0,
            });
            entry.chunks += 1;
            if page > entry.pages {
                entry.pages = page;
            }
        }

        let next = v["result"]["next_page_offset"].clone();
        if next.is_null() {
            break;
        }
        offset = Some(next);
    }

    let mut out: Vec<MetisDoc> = docs.into_values().collect();
    out.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
    Ok(out)
}

/// Open a file with the OS default application (read its content).
#[tauri::command]
pub async fn metis_open_file(path: String) -> Result<(), String> {
    if !std::path::Path::new(&path).exists() {
        return Err(format!("File non trovato: {path}"));
    }
    tokio::task::spawn_blocking(move || Command::new("xdg-open").arg(&path).spawn())
        .await
        .map_err(|e| format!("spawn: {e}"))?
        .map_err(|e| format!("xdg-open: {e}"))?;
    Ok(())
}

/// Reveal a file in the OS file manager (highlighted), or open its folder.
#[tauri::command]
pub async fn metis_reveal_file(path: String) -> Result<(), String> {
    let p = std::path::Path::new(&path);
    if !p.exists() {
        return Err(format!("File non trovato: {path}"));
    }
    let uri = format!("file://{path}");
    let parent = p.parent().map(|x| x.to_string_lossy().to_string());

    tokio::task::spawn_blocking(move || {
        // Precise reveal (highlight) via the freedesktop FileManager1 interface.
        let ok = Command::new("dbus-send")
            .args([
                "--session",
                "--dest=org.freedesktop.FileManager1",
                "--type=method_call",
                "/org/freedesktop/FileManager1",
                "org.freedesktop.FileManager1.ShowItems",
                &format!("array:string:{uri}"),
                "string:",
            ])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        // Fallback: just open the containing folder.
        if !ok {
            if let Some(dir) = parent {
                let _ = Command::new("xdg-open").arg(dir).spawn();
            }
        }
    })
    .await
    .map_err(|e| format!("spawn: {e}"))?;
    Ok(())
}

/// Delete a document (all its chunks) from the knowledge base by `doc_id`.
#[tauri::command]
pub async fn metis_delete_document(doc_id: String) -> Result<(), String> {
    let (base, coll) = metis_qdrant_base_and_collection()?;
    let client = reqwest::Client::new();
    let url = format!("{base}/collections/{coll}/points/delete?wait=true");
    let body = json!({
        "filter": { "must": [ { "key": "doc_id", "match": { "value": doc_id } } ] }
    });
    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Qdrant non raggiungibile ({e})."))?;
    if !resp.status().is_success() {
        return Err(format!("Qdrant HTTP {}", resp.status()));
    }
    Ok(())
}
