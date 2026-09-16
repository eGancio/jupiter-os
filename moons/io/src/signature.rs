// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use base64::Engine;
use tracing::{info, warn};

use crate::error::{IoError, Result};

const SIGNATURE_FILENAME: &str = "signature.json";

// ---------------------------------------------------------------------------
// Signature storage (JSON file)
// ---------------------------------------------------------------------------

/// Get the signature file path, ensuring the directory exists.
fn signature_file(dir: &Path) -> PathBuf {
    if !dir.exists() {
        std::fs::create_dir_all(dir).ok();
    }
    dir.join(SIGNATURE_FILENAME)
}

/// Load all signatures from the JSON file.
fn load_all(dir: &Path) -> HashMap<String, String> {
    let path = signature_file(dir);
    match std::fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => HashMap::new(),
    }
}

/// Save all signatures to the JSON file.
fn save_all(dir: &Path, sigs: &HashMap<String, String>) -> Result<()> {
    let path = signature_file(dir);
    let content = serde_json::to_string_pretty(sigs)
        .map_err(|e| IoError::Signature(format!("Serialize: {e}")))?;
    std::fs::write(&path, content)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Save a signature with a given name.
pub fn save_signature(dir: &Path, name: &str, content: &str) -> Result<()> {
    let mut sigs = load_all(dir);
    sigs.insert(name.to_string(), content.to_string());
    save_all(dir, &sigs)?;
    info!("Saved signature '{name}'");
    Ok(())
}

/// Get a signature by name. Returns None if not found.
pub fn get_signature(dir: &Path, name: &str) -> Option<String> {
    let sigs = load_all(dir);
    sigs.get(name).cloned()
}

/// List all available signature names.
pub fn list_signatures(dir: &Path) -> Vec<String> {
    let sigs = load_all(dir);
    sigs.keys().cloned().collect()
}

/// Delete a signature by name. Returns true if found and deleted.
pub fn delete_signature(dir: &Path, name: &str) -> Result<bool> {
    let mut sigs = load_all(dir);
    let removed = sigs.remove(name).is_some();
    if removed {
        save_all(dir, &sigs)?;
        info!("Deleted signature '{name}'");
    }
    Ok(removed)
}

/// Create a default signature (HTML or plain text).
///
/// If `photo_url` or `color` is provided, generates HTML.
/// Otherwise, generates plain text.
pub fn create_default_signature(
    name: &str,
    email: &str,
    phone: Option<&str>,
    company: Option<&str>,
    role: Option<&str>,
    photo_url: Option<&str>,
    color: Option<&str>,
    style: Option<&str>,
) -> String {
    if photo_url.is_some() || color.is_some() {
        create_html_signature(name, email, phone, company, role, photo_url, color, style)
    } else {
        create_plain_signature(name, email, phone, company, role)
    }
}

// ---------------------------------------------------------------------------
// Plain text signature
// ---------------------------------------------------------------------------

fn create_plain_signature(
    name: &str,
    email: &str,
    phone: Option<&str>,
    company: Option<&str>,
    role: Option<&str>,
) -> String {
    let mut lines = Vec::new();
    lines.push(format!("-- "));
    lines.push(name.to_string());

    if let Some(role) = role {
        lines.push(role.to_string());
    }
    if let Some(company) = company {
        lines.push(company.to_string());
    }
    if let Some(phone) = phone {
        lines.push(format!("Tel: {phone}"));
    }
    lines.push(email.to_string());

    lines.join("\n")
}

// ---------------------------------------------------------------------------
// HTML signature
// ---------------------------------------------------------------------------

fn create_html_signature(
    name: &str,
    email: &str,
    phone: Option<&str>,
    company: Option<&str>,
    role: Option<&str>,
    photo_url: Option<&str>,
    color: Option<&str>,
    style: Option<&str>,
) -> String {
    let accent = match style.unwrap_or("professional") {
        "colorful" => color.unwrap_or("#4CAF50"),
        "minimal" => color.unwrap_or("#333333"),
        _ => color.unwrap_or("#0066cc"), // professional
    };

    let photo_html = if let Some(url) = photo_url {
        format!(
            r#"<td style="padding-right:15px;vertical-align:top">
                <img src="{url}" width="80" height="80"
                     style="border-radius:50%;object-fit:cover;object-position:center 30%" />
            </td>"#
        )
    } else {
        String::new()
    };

    let role_html = role
        .map(|r| format!(r#"<div style="color:{accent};font-size:13px;margin-bottom:4px">{r}</div>"#))
        .unwrap_or_default();

    let company_html = company
        .map(|c| format!(r#"<div style="color:#666;font-size:12px;margin-bottom:8px">{c}</div>"#))
        .unwrap_or_default();

    let phone_html = phone
        .map(|p| format!(r#"<div style="font-size:12px;color:#555">Tel: {p}</div>"#))
        .unwrap_or_default();

    format!(
        r#"<table cellpadding="0" cellspacing="0" style="font-family:Arial,sans-serif">
<tr>
{photo_html}
<td style="vertical-align:top">
    <div style="font-size:15px;font-weight:bold;color:#222;margin-bottom:2px">{name}</div>
    {role_html}
    {company_html}
    <div style="border-top:2px solid {accent};padding-top:8px;margin-top:4px">
        <div style="font-size:12px;color:#555">{email}</div>
        {phone_html}
    </div>
</td>
</tr>
</table>"#
    )
}

// ---------------------------------------------------------------------------
// Photo upload (Imgur anonymous)
// ---------------------------------------------------------------------------

/// Upload a photo to Imgur (anonymous) and return the URL.
/// Returns None on failure.
pub async fn upload_photo_to_imgur(image_path: &Path) -> Option<String> {
    let client_id = "546c25a59c58ad7"; // Public demo key (same as Python)

    if !image_path.exists() || !image_path.is_file() {
        warn!("Photo not found: {}", image_path.display());
        return None;
    }

    // Validate extension
    let ext = image_path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();
    if !["jpg", "jpeg", "png", "gif", "webp"].contains(&ext.as_str()) {
        warn!("Unsupported image format: .{ext}");
        return None;
    }

    let data = match std::fs::read(image_path) {
        Ok(d) => d,
        Err(e) => {
            warn!("Failed to read image: {e}");
            return None;
        }
    };

    let b64 = base64::engine::general_purpose::STANDARD.encode(&data);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .ok()?;

    let resp = client
        .post("https://api.imgur.com/3/image")
        .header("Authorization", format!("Client-ID {client_id}"))
        .form(&[("image", &b64)])
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() {
        warn!("Imgur upload failed: {}", resp.status());
        return None;
    }

    let json: serde_json::Value = resp.json().await.ok()?;
    json.get("data")
        .and_then(|d| d.get("link"))
        .and_then(|l| l.as_str())
        .map(|s| s.to_string())
}
