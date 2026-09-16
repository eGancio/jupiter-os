// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

//! Tauri commands for managing email credentials in the Windows Credential
//! Manager (service "MoonIo"). Mirrors what `moon-io.exe credentials ...` does,
//! so the CLI and the GUI share the same vault.

use keyring::Entry;
use serde::Serialize;
use std::process::Command;

const SERVICE: &str = "MoonIo";

// ---------------------------------------------------------------------------
// Hardcoded OAuth client (Desktop app)
// ---------------------------------------------------------------------------
//
// These are the Google OAuth 2.0 Desktop app credentials for "Moon Io".
// For Desktop OAuth clients Google explicitly states the client_secret is NOT
// a real secret (it's distributed in every binary that ships the app). See:
// https://developers.google.com/identity/protocols/oauth2/native-app
// So we embed them here for a one-click UX. To override (e.g. enterprise
// deployments with their own Cloud project), set env vars
// MOONIO_OAUTH_CLIENT_ID and MOONIO_OAUTH_CLIENT_SECRET before launching.
const DEFAULT_GMAIL_CLIENT_ID: &str =
    "934393516366-ipr67p3c168ofabase8b5bcbsueuato0.apps.googleusercontent.com";
const DEFAULT_GMAIL_CLIENT_SECRET: &str = "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";

fn gmail_oauth_client() -> (String, String) {
    let id = std::env::var("MOONIO_OAUTH_CLIENT_ID")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_GMAIL_CLIENT_ID.to_string());
    let secret = std::env::var("MOONIO_OAUTH_CLIENT_SECRET")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_GMAIL_CLIENT_SECRET.to_string());
    (id, secret)
}

/// Account-to-username mapping derived from `.mcp.json` env block.
///
/// We expose which accounts are configured (so the UI can show them) but only
/// the booleans — the password is never read back into the UI process beyond
/// "is it set?".
#[derive(Serialize)]
pub struct CredentialEntry {
    pub account: String,
    pub username: String,
    pub has_password: bool,
    /// True if OAuth credentials (refresh_token) are stored.
    pub has_oauth: bool,
}

fn entry(account: &str) -> Result<Entry, String> {
    Entry::new(SERVICE, account).map_err(|e| format!("Keyring init: {e}"))
}

#[tauri::command]
pub fn credentials_list() -> Result<Vec<CredentialEntry>, String> {
    // Parse the .mcp.json env for the "io" server to learn which accounts
    // exist and what their usernames are. We DO NOT read passwords from there.
    let mcp_path = crate::config::mcp_json_path();
    let content = std::fs::read_to_string(&mcp_path)
        .map_err(|e| format!("Cannot read {mcp_path:?}: {e}"))?;
    let v: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| format!("Parse mcp.json: {e}"))?;

    let env = v
        .get("servers")
        .and_then(|s| s.get("io"))
        .and_then(|io| io.get("env"))
        .and_then(|e| e.as_object())
        .cloned()
        .unwrap_or_default();

    let primary_username = env
        .get("IMAP_USERNAME")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let primary_name = env
        .get("IMAP_ACCOUNT_NAME")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("primary")
        .to_string();

    let gmail_username = env
        .get("GMAIL_USERNAME")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let mut accounts: Vec<(String, String)> = Vec::new();
    if !primary_username.is_empty() {
        accounts.push((primary_name, primary_username));
    }
    if !gmail_username.is_empty() {
        accounts.push(("gmail".to_string(), gmail_username));
    }

    // Generic ACCOUNTS=foo,bar pattern
    if let Some(list) = env.get("ACCOUNTS").and_then(|v| v.as_str()) {
        for name in list.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
            if accounts.iter().any(|(n, _)| n == name) {
                continue;
            }
            let prefix: String = name.to_uppercase().chars().map(|c| if c.is_alphanumeric() { c } else { '_' }).collect();
            let user_key = format!("{prefix}_IMAP_USERNAME");
            let user = env
                .get(&user_key)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if !user.is_empty() {
                accounts.push((name.to_string(), user));
            }
        }
    }

    let result = accounts
        .into_iter()
        .map(|(name, user)| {
            let has_password = Entry::new(SERVICE, &name)
                .ok()
                .and_then(|e| e.get_password().ok())
                .is_some();
            let has_oauth = Entry::new(SERVICE, &format!("oauth:{name}"))
                .ok()
                .and_then(|e| e.get_password().ok())
                .is_some();
            CredentialEntry {
                account: name,
                username: user,
                has_password,
                has_oauth,
            }
        })
        .collect();

    Ok(result)
}

#[tauri::command]
pub fn credentials_set(account: String, password: String) -> Result<(), String> {
    if account.trim().is_empty() {
        return Err("Account name vuoto.".into());
    }
    if password.is_empty() {
        return Err("Password vuota.".into());
    }
    entry(&account)?
        .set_password(&password)
        .map_err(|e| format!("Keyring set: {e}"))
}

#[tauri::command]
pub fn credentials_delete(account: String) -> Result<bool, String> {
    if account.trim().is_empty() {
        return Err("Account name vuoto.".into());
    }
    let e = entry(&account)?;
    match e.delete_credential() {
        Ok(_) => Ok(true),
        Err(keyring::Error::NoEntry) => Ok(false),
        Err(e) => Err(format!("Keyring delete: {e}")),
    }
}

/// Add a new email account: writes the (non-sensitive) username into
/// `.mcp.json` under `servers.io.env`, and stores the password in the keyring.
///
/// `preset` accepts:
/// - `"gmail"` → IMAP `imap.gmail.com:993`, SMTP `smtp.gmail.com:465`,
///   stored as `GMAIL_USERNAME` env. Account name is forced to `"gmail"`.
/// - any other value (or `None`) → generic account stored via the
///   `ACCOUNTS=name,...` + `{NAME}_IMAP_USERNAME` / `{NAME}_IMAP_HOST` env pattern.
#[tauri::command]
pub fn credentials_add_account(
    account: String,
    email: String,
    password: String,
    preset: Option<String>,
    imap_host: Option<String>,
) -> Result<(), String> {
    let account = account.trim().to_string();
    let email = email.trim().to_string();
    if account.is_empty() {
        return Err("Account name vuoto.".into());
    }
    if email.is_empty() {
        return Err("Email vuota.".into());
    }
    if password.is_empty() {
        return Err("Password vuota.".into());
    }

    let mcp_path = crate::config::mcp_json_path();
    let content = std::fs::read_to_string(&mcp_path)
        .map_err(|e| format!("Cannot read {mcp_path:?}: {e}"))?;
    let mut doc: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| format!("Parse mcp.json: {e}"))?;

    // Navigate/create servers.io.env
    let env = doc
        .pointer_mut("/servers/io/env")
        .and_then(|v| v.as_object_mut())
        .ok_or_else(|| "servers.io.env mancante in .mcp.json".to_string())?;

    let preset_lc = preset.as_deref().map(|s| s.to_lowercase());
    match preset_lc.as_deref() {
        Some("gmail") => {
            // Gmail shortcut — forces account name to "gmail"
            env.insert("GMAIL_USERNAME".into(), serde_json::Value::String(email));
            // Remove any stale empty placeholders for consistency
        }
        _ => {
            // Generic account: add to ACCOUNTS list + {NAME}_IMAP_USERNAME
            // Sanitize: env var names must be alphanumeric+underscore only
            let prefix = account
                .to_uppercase()
                .chars()
                .map(|c| if c.is_alphanumeric() { c } else { '_' })
                .collect::<String>();

            // ACCOUNTS list
            let mut list: Vec<String> = env
                .get("ACCOUNTS")
                .and_then(|v| v.as_str())
                .map(|s| {
                    s.split(',')
                        .map(|x| x.trim().to_string())
                        .filter(|x| !x.is_empty())
                        .collect()
                })
                .unwrap_or_default();
            if !list.contains(&account) {
                list.push(account.clone());
            }
            env.insert(
                "ACCOUNTS".into(),
                serde_json::Value::String(list.join(",")),
            );

            env.insert(
                format!("{prefix}_IMAP_USERNAME"),
                serde_json::Value::String(email),
            );
            // Store password in .mcp.json as well — keyring can be unavailable
            env.insert(
                format!("{prefix}_IMAP_PASSWORD"),
                serde_json::Value::String(password.clone()),
            );
            if let Some(host) = imap_host.as_deref().filter(|s| !s.is_empty()) {
                env.insert(
                    format!("{prefix}_IMAP_HOST"),
                    serde_json::Value::String(host.to_string()),
                );
            }
        }
    }

    // Atomic write
    let serialized = serde_json::to_string_pretty(&doc)
        .map_err(|e| format!("Serialize mcp.json: {e}"))?;
    let tmp_path = mcp_path.with_extension("json.tmp");
    std::fs::write(&tmp_path, &serialized)
        .map_err(|e| format!("Write tmp: {e}"))?;
    std::fs::rename(&tmp_path, &mcp_path)
        .map_err(|e| format!("Rename tmp: {e}"))?;

    // Now save the password to the keyring (account name = the canonical one)
    let keyring_account = if preset_lc.as_deref() == Some("gmail") {
        "gmail"
    } else {
        account.as_str()
    };
    entry(keyring_account)?
        .set_password(&password)
        .map_err(|e| format!("Keyring set: {e}"))
}

// ---------------------------------------------------------------------------
// OAuth 2.0 — Gmail / Google Workspace
// ---------------------------------------------------------------------------

/// Path to the moon-io binary used for the OAuth subcommand.
/// Reads the command from .mcp.json servers.io so it's always consistent
/// with what the GUI uses to start the service.
fn moon_io_binary() -> std::path::PathBuf {
    if let Ok(raw) = std::fs::read_to_string(crate::config::mcp_json_path()) {
        if let Ok(cfg) = serde_json::from_str::<crate::config::McpConfig>(&raw) {
            if let Some(server) = cfg.servers.get("io") {
                let p = std::path::PathBuf::from(&server.command);
                if p.is_absolute() {
                    return p;
                }
                return crate::config::get_base_dir().join(&server.command);
            }
        }
    }
    // Fallback: platform-appropriate name relative to project root
    let bin = if cfg!(target_os = "windows") { "moon-io.exe" } else { "moon-io" };
    crate::config::get_base_dir()
        .join("target")
        .join("release")
        .join(bin)
}

#[derive(Serialize)]
pub struct OAuthStatus {
    pub account: String,
    pub configured: bool,
}

/// Check whether OAuth credentials are stored for an account.
#[tauri::command]
pub fn oauth_status(account: String) -> Result<OAuthStatus, String> {
    let target = format!("oauth:{}", account.trim());
    let configured = Entry::new(SERVICE, &target)
        .ok()
        .and_then(|e| e.get_password().ok())
        .is_some();
    Ok(OAuthStatus { account, configured })
}

/// Remove stored OAuth credentials for an account.
#[tauri::command]
pub fn oauth_disconnect(account: String) -> Result<bool, String> {
    if account.trim().is_empty() {
        return Err("Account vuoto.".into());
    }
    let target = format!("oauth:{}", account.trim());
    let e = Entry::new(SERVICE, &target).map_err(|e| format!("Keyring init: {e}"))?;
    match e.delete_credential() {
        Ok(_) => Ok(true),
        Err(keyring::Error::NoEntry) => Ok(false),
        Err(e) => Err(format!("Keyring delete: {e}")),
    }
}

/// Run the OAuth authorization flow for a Gmail account.
///
/// Uses the hardcoded Moon Io Desktop OAuth client (see DEFAULT_GMAIL_* above),
/// so the user only provides their email — no client_id / client_secret prompt.
///
/// Spawns `moon-io.exe oauth connect gmail` as a subprocess; the subprocess
/// opens the system browser for consent, listens on a local port for the
/// callback, exchanges the code for tokens, and stores the refresh_token in
/// the keyring.
///
/// Also writes `GMAIL_USERNAME` into `.mcp.json` so the daemon picks it up.
#[tauri::command]
pub async fn oauth_connect_gmail(email: String) -> Result<String, String> {
    let email = email.trim().to_string();
    if email.is_empty() {
        return Err("Email vuota.".into());
    }

    let (client_id, client_secret) = gmail_oauth_client();

    // First persist GMAIL_USERNAME in .mcp.json (account name == "gmail")
    let mcp_path = crate::config::mcp_json_path();
    let content = std::fs::read_to_string(&mcp_path)
        .map_err(|e| format!("Cannot read {mcp_path:?}: {e}"))?;
    let mut doc: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| format!("Parse mcp.json: {e}"))?;
    let env = doc
        .pointer_mut("/servers/io/env")
        .and_then(|v| v.as_object_mut())
        .ok_or_else(|| "servers.io.env mancante in .mcp.json".to_string())?;
    env.insert(
        "GMAIL_USERNAME".into(),
        serde_json::Value::String(email.clone()),
    );
    let serialized =
        serde_json::to_string_pretty(&doc).map_err(|e| format!("Serialize mcp.json: {e}"))?;
    let tmp_path = mcp_path.with_extension("json.tmp");
    std::fs::write(&tmp_path, &serialized).map_err(|e| format!("Write tmp: {e}"))?;
    std::fs::rename(&tmp_path, &mcp_path).map_err(|e| format!("Rename tmp: {e}"))?;

    // Spawn moon-io oauth connect — this opens a browser window for consent
    let binary = moon_io_binary();
    if !binary.exists() {
        return Err(format!(
            "moon-io binary non trovato: {}. Esegui un build di Moon Io prima.",
            binary.display()
        ));
    }

    let mut cmd = Command::new(&binary);
    cmd.arg("oauth")
        .arg("connect")
        .arg("gmail")
        .env("MOONIO_OAUTH_CLIENT_ID", &client_id)
        .env("MOONIO_OAUTH_CLIENT_SECRET", &client_secret);

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }

    // The subprocess waits up to 5 minutes for the Google callback. Run the
    // blocking wait on a dedicated blocking thread so the Tauri main thread
    // (which drives the webview) stays responsive — otherwise the whole UI
    // freezes for the duration of the consent flow.
    let output = tokio::task::spawn_blocking(move || cmd.output())
        .await
        .map_err(|e| format!("Join spawn_blocking: {e}"))?
        .map_err(|e| format!("Spawn moon-io oauth: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        return Err(format!(
            "OAuth flow fallito (exit {:?}):\nSTDOUT: {stdout}\nSTDERR: {stderr}",
            output.status.code()
        ));
    }

    Ok(format!(
        "OAuth Gmail collegato per {email}. Fai Restart del server `io` per attivare l'account."
    ))
}

// ---------------------------------------------------------------------------
// Full account removal (cleanup keyring + .mcp.json)
// ---------------------------------------------------------------------------

/// Remove an account completely: deletes both password and OAuth credentials
/// from the keyring AND removes the username + related env from `.mcp.json`
/// so the account disappears from the GUI.
#[tauri::command]
pub fn account_remove(account: String) -> Result<(), String> {
    let account = account.trim().to_string();
    if account.is_empty() {
        return Err("Account name vuoto.".into());
    }

    // 1) Delete password from keyring (best-effort)
    if let Ok(e) = Entry::new(SERVICE, &account) {
        let _ = e.delete_credential();
    }

    // 2) Delete OAuth credentials from keyring (best-effort)
    if let Ok(e) = Entry::new(SERVICE, &format!("oauth:{account}")) {
        let _ = e.delete_credential();
    }

    // 3) Remove env entries from .mcp.json for the io server
    let mcp_path = crate::config::mcp_json_path();
    let content = std::fs::read_to_string(&mcp_path)
        .map_err(|e| format!("Cannot read {mcp_path:?}: {e}"))?;
    let mut doc: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| format!("Parse mcp.json: {e}"))?;

    let env = doc
        .pointer_mut("/servers/io/env")
        .and_then(|v| v.as_object_mut())
        .ok_or_else(|| "servers.io.env mancante in .mcp.json".to_string())?;

    let lc = account.to_lowercase();
    if lc == "gmail" {
        env.remove("GMAIL_USERNAME");
    } else {
        // Primary (legacy IMAP_*) — only remove if IMAP_ACCOUNT_NAME matches.
        let is_primary = env
            .get("IMAP_ACCOUNT_NAME")
            .and_then(|v| v.as_str())
            .map(|n| n.eq_ignore_ascii_case(&account))
            .unwrap_or(false)
            || (account == "primary" && !env.contains_key("IMAP_ACCOUNT_NAME"));

        if is_primary {
            env.remove("IMAP_USERNAME");
            env.remove("IMAP_PASSWORD");
            env.remove("IMAP_ACCOUNT_NAME");
            env.remove("IMAP_HOST");
            env.remove("IMAP_PORT");
            env.remove("SMTP_HOST");
            env.remove("SMTP_PORT");
        } else {
            // Generic ACCOUNTS pattern
            let prefix: String = account.to_uppercase().chars().map(|c| if c.is_alphanumeric() { c } else { '_' }).collect();
            env.remove(&format!("{prefix}_IMAP_USERNAME"));
            env.remove(&format!("{prefix}_IMAP_PASSWORD"));
            env.remove(&format!("{prefix}_IMAP_HOST"));
            env.remove(&format!("{prefix}_IMAP_PORT"));
            env.remove(&format!("{prefix}_SMTP_HOST"));
            env.remove(&format!("{prefix}_SMTP_PORT"));

            // Remove from ACCOUNTS list
            if let Some(list_str) = env.get("ACCOUNTS").and_then(|v| v.as_str()) {
                let new_list: String = list_str
                    .split(',')
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case(&account))
                    .collect::<Vec<_>>()
                    .join(",");
                if new_list.is_empty() {
                    env.remove("ACCOUNTS");
                } else {
                    env.insert("ACCOUNTS".into(), serde_json::Value::String(new_list));
                }
            }
        }
    }

    // Atomic write
    let serialized = serde_json::to_string_pretty(&doc)
        .map_err(|e| format!("Serialize mcp.json: {e}"))?;
    let tmp_path = mcp_path.with_extension("json.tmp");
    std::fs::write(&tmp_path, &serialized).map_err(|e| format!("Write tmp: {e}"))?;
    std::fs::rename(&tmp_path, &mcp_path).map_err(|e| format!("Rename tmp: {e}"))?;

    Ok(())
}
