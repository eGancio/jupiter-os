// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

//! Read-only bridge for the GUI to display received emails.
//!
//! Moon Io already indexes mail into a shared Qdrant collection ("emails"); the
//! full body lives in the payload. We read it straight from Qdrant's REST API
//! (no MCP roundtrip, no embeddings) and hand a list to the frontend.

use serde::Serialize;
use serde_json::json;

/// One received email, read from the Moon Io Qdrant "emails" collection payload.
#[derive(Serialize)]
pub struct EmailItem {
    pub uid: i64,
    pub subject: String,
    pub sender: String,
    pub recipient: String,
    pub date: String,
    pub body: String,
    pub folder: String,
    pub account: String,
}

/// Qdrant REST base URL. Moon Io talks gRPC on :6334; REST is :6333. We read
/// QDRANT_URL from `.mcp.json` best-effort to honor a custom host, but always
/// force the REST port. Defaults to http://127.0.0.1:6333.
fn qdrant_rest_base() -> String {
    let host = read_qdrant_host().unwrap_or_else(|| "127.0.0.1".to_string());
    format!("http://{host}:6333")
}

fn read_qdrant_host() -> Option<String> {
    let mcp_path = crate::config::mcp_json_path();
    let content = std::fs::read_to_string(&mcp_path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&content).ok()?;
    // The real .mcp.json may use "mcpServers" or "servers" as the root.
    let url = ["mcpServers", "servers"].iter().find_map(|root| {
        v.get(root)
            .and_then(|s| s.get("io"))
            .and_then(|io| io.get("env"))
            .and_then(|e| e.get("QDRANT_URL"))
            .and_then(|u| u.as_str())
    })?;
    // Extract host from "http://HOST:PORT".
    let host = url
        .split("://")
        .nth(1)
        .unwrap_or(url)
        .split(':')
        .next()
        .unwrap_or("")
        .to_string();
    (!host.is_empty()).then_some(host)
}

fn payload_str(p: &serde_json::Value, key: &str) -> String {
    p.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string()
}

/// List received emails (folder = INBOX), newest first, straight from Qdrant.
/// Read-only — the Moon Io daemon keeps the collection indexed; we just display it.
#[tauri::command]
pub async fn list_emails_io(limit: Option<u32>) -> Result<Vec<EmailItem>, String> {
    let limit = limit.unwrap_or(50).clamp(1, 200);
    let url = format!("{}/collections/emails/points/scroll", qdrant_rest_base());

    let body = json!({
        "limit": limit,
        "with_payload": true,
        "with_vector": false,
        // Newest first (imap_uid is indexed); good enough for an INBOX view.
        "order_by": { "key": "imap_uid", "direction": "desc" },
        "filter": { "must": [ { "key": "folder", "match": { "value": "INBOX" } } ] }
    });

    let resp = reqwest::Client::new()
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Qdrant non raggiungibile ({e}). Avvia Moon Io."))?;

    if !resp.status().is_success() {
        return Err(format!(
            "Qdrant ha risposto {} — la collezione 'emails' è indicizzata?",
            resp.status()
        ));
    }

    let v: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Risposta Qdrant non valida: {e}"))?;

    let points = v
        .get("result")
        .and_then(|r| r.get("points"))
        .and_then(|p| p.as_array())
        .cloned()
        .unwrap_or_default();

    let items = points
        .iter()
        .filter_map(|pt| pt.get("payload"))
        .map(|p| EmailItem {
            uid: p.get("imap_uid").and_then(|v| v.as_i64()).unwrap_or(0),
            subject: payload_str(p, "subject"),
            sender: payload_str(p, "sender"),
            recipient: payload_str(p, "recipient"),
            date: payload_str(p, "date"),
            body: payload_str(p, "text"),
            folder: payload_str(p, "folder"),
            account: payload_str(p, "account"),
        })
        .collect();

    Ok(items)
}
