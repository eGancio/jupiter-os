// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

//! Tauri commands for configuring the third-party Ads MCP servers (Google Ads,
//! Meta Ads) from the GUI.
//!
//! Unlike [`crate::europa_credentials`], secrets here are stored **only** in the
//! OS keyring (service "MoonAds"), never written to `.mcp.json` or any plaintext
//! file. The ads MCP servers are registered in the `mcpServers` block of
//! `.mcp.json` with `${VAR}` placeholders; at sidecar spawn time
//! [`load_ads_env_from_keyring`] materializes the real values into the sidecar's
//! environment (see [`crate::claude`]), where `expandEnv` in the sidecar resolves
//! the placeholders. So the tokens live encrypted at rest and only reach the
//! stdio MCP child process in memory.

use serde::Serialize;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::path::PathBuf;

use keyring::Entry;

const SERVICE: &str = "MoonAds";

/// Canonical env-var keys per platform, with whether each is required.
/// The `.mcp.json` `mcpServers.*.env` block maps the actual server's expected
/// variable to `${<one of these>}`, so these names are JupiterOS-internal.
const GOOGLE_KEYS: &[(&str, bool)] = &[
    ("GOOGLE_ADS_DEVELOPER_TOKEN", true),
    ("GOOGLE_ADS_CLIENT_ID", true),
    ("GOOGLE_ADS_CLIENT_SECRET", true),
    ("GOOGLE_ADS_REFRESH_TOKEN", true),
    ("GOOGLE_ADS_LOGIN_CUSTOMER_ID", false),
];

const META_KEYS: &[(&str, bool)] = &[
    ("META_ACCESS_TOKEN", true),
    ("META_AD_ACCOUNT_ID", false),
];

fn platform_keys(platform: &str) -> Option<&'static [(&'static str, bool)]> {
    match platform {
        "google-ads" => Some(GOOGLE_KEYS),
        "meta-ads" => Some(META_KEYS),
        _ => None,
    }
}

/// Read a secret from the keyring; `None` if missing or empty.
fn kget(key: &str) -> Option<String> {
    Entry::new(SERVICE, key)
        .ok()
        .and_then(|e| e.get_password().ok())
        .filter(|s| !s.is_empty())
}

/// Set (or, for an empty value, delete) a secret in the keyring.
fn kset(key: &str, val: &str) -> Result<(), String> {
    let entry = Entry::new(SERVICE, key).map_err(|e| format!("Keyring open ({key}): {e}"))?;
    if val.is_empty() {
        let _ = entry.delete_credential();
        Ok(())
    } else {
        entry
            .set_password(val)
            .map_err(|e| format!("Keyring set ({key}): {e}"))
    }
}

fn kdel(key: &str) {
    if let Ok(entry) = Entry::new(SERVICE, key) {
        let _ = entry.delete_credential();
    }
}

// ---------------------------------------------------------------------------
// Status (for the UI) — never returns secret values, only whether each is set
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct AdsKeyStatus {
    /// Canonical env-var name (also the form field id).
    pub key: String,
    pub set: bool,
    pub required: bool,
}

#[derive(Serialize)]
pub struct AdsPlatformStatus {
    /// "google-ads" | "meta-ads"
    pub platform: String,
    /// true when every REQUIRED key is set.
    pub configured: bool,
    pub keys: Vec<AdsKeyStatus>,
}

fn platform_status(platform: &str, keys: &'static [(&'static str, bool)]) -> AdsPlatformStatus {
    let key_status: Vec<AdsKeyStatus> = keys
        .iter()
        .map(|(k, req)| AdsKeyStatus {
            key: (*k).to_string(),
            set: kget(k).is_some(),
            required: *req,
        })
        .collect();
    let configured = keys
        .iter()
        .filter(|(_, req)| *req)
        .all(|(k, _)| kget(k).is_some());
    AdsPlatformStatus {
        platform: platform.to_string(),
        configured,
        keys: key_status,
    }
}

#[tauri::command]
pub fn ads_credentials_status() -> Vec<AdsPlatformStatus> {
    vec![
        platform_status("google-ads", GOOGLE_KEYS),
        platform_status("meta-ads", META_KEYS),
    ]
}

/// Save one platform's keys. `values` maps canonical key -> value; an empty
/// value clears that key. Keys not belonging to the platform are rejected.
#[tauri::command]
pub fn ads_credentials_set(
    platform: String,
    values: HashMap<String, String>,
) -> Result<(), String> {
    let keys = platform_keys(&platform)
        .ok_or_else(|| format!("Piattaforma sconosciuta: {platform}"))?;
    for k in values.keys() {
        if !keys.iter().any(|(kk, _)| kk == k) {
            return Err(format!("Chiave non valida per {platform}: {k}"));
        }
    }
    for (k, v) in &values {
        kset(k, v.trim())?;
    }
    Ok(())
}

/// Remove every stored key for a platform.
#[tauri::command]
pub fn ads_credentials_delete(platform: String) -> Result<(), String> {
    let keys = platform_keys(&platform)
        .ok_or_else(|| format!("Piattaforma sconosciuta: {platform}"))?;
    for (k, _) in keys {
        kdel(k);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Sidecar env injection (called from crate::claude at spawn time)
// ---------------------------------------------------------------------------

/// Collect every ads secret present in the keyring as env vars to inject into
/// the sidecar process. Missing keys are simply omitted.
pub fn load_ads_env_from_keyring() -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for (k, _) in GOOGLE_KEYS.iter().chain(META_KEYS.iter()) {
        if let Some(v) = kget(k) {
            map.insert((*k).to_string(), v);
        }
    }
    map
}

/// RAM-backed runtime dir for ephemeral secrets. On Linux `XDG_RUNTIME_DIR` is a
/// tmpfs (volatile, cleared on logout) — secrets written here never hit
/// persistent disk. Falls back to the temp dir if unset.
fn runtime_secrets_dir() -> PathBuf {
    if let Some(rt) = std::env::var_os("XDG_RUNTIME_DIR") {
        return PathBuf::from(rt).join("jupiteros");
    }
    std::env::temp_dir().join("jupiteros")
}

/// The official `google_ads_mcp` server reads credentials from a `google-ads.yaml`
/// file (path via env `GOOGLE_ADS_CREDENTIALS`), NOT from individual env vars.
/// Materialize that file from the keyring into the RAM-backed runtime dir with
/// `0600` perms and return its path. The keyring stays the source of truth; the
/// file is regenerated at every sidecar spawn and lives only in volatile tmpfs.
/// Returns `None` if the required Google keys aren't set.
pub fn materialize_google_ads_yaml() -> Option<PathBuf> {
    // YAML single-quoted scalars are literal; Google tokens never contain a
    // single quote, so this is safe without escaping.
    let dev = kget("GOOGLE_ADS_DEVELOPER_TOKEN")?;
    let client_id = kget("GOOGLE_ADS_CLIENT_ID")?;
    let client_secret = kget("GOOGLE_ADS_CLIENT_SECRET")?;
    let refresh = kget("GOOGLE_ADS_REFRESH_TOKEN")?;

    let mut yaml = String::new();
    yaml.push_str(&format!("developer_token: '{dev}'\n"));
    yaml.push_str(&format!("client_id: '{client_id}'\n"));
    yaml.push_str(&format!("client_secret: '{client_secret}'\n"));
    yaml.push_str(&format!("refresh_token: '{refresh}'\n"));
    if let Some(login) = kget("GOOGLE_ADS_LOGIN_CUSTOMER_ID") {
        yaml.push_str(&format!("login_customer_id: '{login}'\n"));
    }
    yaml.push_str("use_proto_plus: True\n");

    let dir = runtime_secrets_dir();
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join("google-ads.yaml");
    std::fs::write(&path, yaml).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Some(path)
}
