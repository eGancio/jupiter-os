// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

//! Tauri commands to configure Moon Thebe's brand kit from the GUI.
//!
//! The brand kit (logo, colors, company identity, footer) is the set of CONSTANTS
//! of a company report. It lives in `moons/thebe/data/brands/<name>/` as a brand.json
//! plus a logo file — the same directory Thebe reads at render time (THEBE_BRANDS_DIR).

use std::path::PathBuf;

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config;

/// Resolve the brand kits root the SAME way Thebe does at render time: read
/// THEBE_BRANDS_DIR from the thebe server entry in .mcp.json (absolute in the runtime
/// config). This avoids the base_dir pitfall (at runtime base_dir is the binary dir,
/// not the repo root). Falls back to <base>/moons/thebe/data/brands.
fn brands_root() -> PathBuf {
    let raw = std::fs::read_to_string(config::mcp_json_path()).unwrap_or_default();
    if let Ok(v) = serde_json::from_str::<Value>(&raw) {
        if let Some(dir) = v
            .pointer("/servers/thebe/env/THEBE_BRANDS_DIR")
            .and_then(|x| x.as_str())
        {
            let p = PathBuf::from(dir);
            return if p.is_absolute() { p } else { config::get_base_dir().join(p) };
        }
    }
    config::get_base_dir().join("moons/thebe/data/brands")
}

fn brand_dir(brand: &str) -> PathBuf {
    // Defend against path traversal: keep only a simple folder name.
    let safe: String = brand
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    brands_root().join(if safe.is_empty() { "emotion".into() } else { safe })
}

/// What the GUI form edits (a subset of the full brand.json — other fields are preserved).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrandKitInput {
    pub name: String,
    pub primary: String,
    pub accent: String,
    pub vat: String,
    pub address: String,
    pub contacts: String,
    pub confidentiality: String,
    /// Logo as a data URL ("data:image/png;base64,...") or empty/None to leave unchanged.
    pub logo_data_url: Option<String>,
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct BrandKitOutput {
    pub name: String,
    pub primary: String,
    pub accent: String,
    pub vat: String,
    pub address: String,
    pub contacts: String,
    pub confidentiality: String,
    pub has_logo: bool,
}

fn json_str(v: &Value, path: &[&str]) -> String {
    let mut cur = v;
    for k in path {
        cur = match cur.get(k) {
            Some(x) => x,
            None => return String::new(),
        };
    }
    cur.as_str().unwrap_or("").to_string()
}

#[tauri::command]
pub fn get_brand_kit(brand: String) -> Result<BrandKitOutput, String> {
    let dir = brand_dir(&brand);
    let cfg_path = dir.join("brand.json");
    let v: Value = if cfg_path.exists() {
        let raw = std::fs::read_to_string(&cfg_path).map_err(|e| e.to_string())?;
        serde_json::from_str(&raw).map_err(|e| format!("brand.json non valido: {e}"))?
    } else {
        Value::Object(Default::default())
    };

    let logo_name = json_str(&v, &["logo"]);
    let has_logo = !logo_name.is_empty() && dir.join(&logo_name).exists();

    Ok(BrandKitOutput {
        name: {
            let n = json_str(&v, &["name"]);
            if n.is_empty() { json_str(&v, &["footer", "company"]) } else { n }
        },
        primary: json_str(&v, &["colors", "primary"]),
        accent: json_str(&v, &["colors", "accent"]),
        vat: json_str(&v, &["footer", "vat"]),
        address: json_str(&v, &["footer", "address"]),
        contacts: json_str(&v, &["footer", "contacts"]),
        confidentiality: json_str(&v, &["footer", "confidentiality"]),
        has_logo,
    })
}

fn ext_from_data_url(data_url: &str) -> &str {
    if data_url.contains("image/png") { "png" }
    else if data_url.contains("image/jpeg") || data_url.contains("image/jpg") { "jpg" }
    else if data_url.contains("image/svg") { "svg" }
    else if data_url.contains("image/webp") { "webp" }
    else { "png" }
}

#[tauri::command]
pub fn set_brand_kit(brand: String, kit: BrandKitInput) -> Result<(), String> {
    let dir = brand_dir(&brand);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let cfg_path = dir.join("brand.json");

    // Load existing config (or start fresh) so we preserve fields the form doesn't edit
    // (text/muted/rule colors, font, tagline).
    let mut v: Value = if cfg_path.exists() {
        let raw = std::fs::read_to_string(&cfg_path).map_err(|e| e.to_string())?;
        serde_json::from_str(&raw).unwrap_or_else(|_| Value::Object(Default::default()))
    } else {
        Value::Object(Default::default())
    };
    if !v.is_object() {
        v = Value::Object(Default::default());
    }

    // Save the logo first (if a new one was provided) so we can set the file name.
    if let Some(durl) = kit.logo_data_url.as_ref().filter(|s| s.starts_with("data:")) {
        let b64 = durl.split(",").nth(1).ok_or("data URL malformato")?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64.trim())
            .map_err(|e| format!("logo base64 non valido: {e}"))?;
        // Remove any previous logo.* so only one remains.
        for old in ["logo.png", "logo.jpg", "logo.svg", "logo.webp"] {
            let _ = std::fs::remove_file(dir.join(old));
        }
        let ext = ext_from_data_url(durl);
        let logo_name = format!("logo.{ext}");
        std::fs::write(dir.join(&logo_name), &bytes).map_err(|e| e.to_string())?;
        v["logo"] = Value::String(logo_name);
    }

    // Patch the edited fields.
    v["name"] = Value::String(kit.name.clone());
    let colors = v.get_mut("colors").and_then(|c| c.as_object_mut());
    if colors.is_none() {
        v["colors"] = serde_json::json!({});
    }
    v["colors"]["primary"] = Value::String(kit.primary);
    v["colors"]["accent"] = Value::String(kit.accent);
    if v.get("font").is_none() {
        v["font"] = serde_json::json!({ "family": "Helvetica, Arial, sans-serif" });
    }
    if v.get("footer").is_none() {
        v["footer"] = serde_json::json!({});
    }
    v["footer"]["company"] = Value::String(kit.name);
    v["footer"]["vat"] = Value::String(kit.vat);
    v["footer"]["address"] = Value::String(kit.address);
    v["footer"]["contacts"] = Value::String(kit.contacts);
    v["footer"]["confidentiality"] = Value::String(kit.confidentiality);

    // Atomic write (write to .tmp then rename), same pattern as .mcp.json edits.
    let serialized = serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?;
    let tmp = cfg_path.with_extension("json.tmp");
    std::fs::write(&tmp, serialized.as_bytes()).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &cfg_path).map_err(|e| e.to_string())?;
    Ok(())
}
