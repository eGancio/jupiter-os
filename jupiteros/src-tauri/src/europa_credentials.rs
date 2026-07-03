// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

//! Tauri commands for configuring Moon Europa's messaging channels from the GUI.
//!
//! Mirrors the email setup in [`crate::credentials`] but for messaging:
//! - **Telegram (account)** — full OTP login (phone → code → 2FA) runs in-process
//!   here via grammers, writing the session to the same SQLite file moon-europa
//!   reads at startup. To make the path unambiguous we store an **absolute**
//!   `DATA_DIR` in `.mcp.json`.
//! - **Telegram (bot)** — just stores a bot token (`TELEGRAM_BOT_TOKEN`).
//! - **Slack** — stores a bot token (`SLACK_BOT_TOKEN`).
//! - **Teams** — OAuth2 device-code flow; stores `TEAMS_*` + a refresh token.
//!
//! Secrets are written into `servers.europa.env` of `.mcp.json` (moon-europa only
//! reads env) and mirrored into the OS keyring (service "MoonEuropa") for parity.

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

use grammers_client::client::{LoginToken, PasswordToken};
use grammers_client::{Client, SignInError};
use grammers_mtsender::SenderPool;
use grammers_session::storages::SqliteSession;
use keyring::Entry;
use serde::Serialize;
use serde_json::Value;
use std::sync::Arc;
use tauri::State;

const SERVICE: &str = "MoonEuropa";

// ---------------------------------------------------------------------------
// .mcp.json (servers.europa.env) helpers
// ---------------------------------------------------------------------------

fn read_doc() -> Result<(PathBuf, Value), String> {
    let path = crate::config::mcp_json_path();
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("Cannot read {path:?}: {e}"))?;
    let doc: Value = serde_json::from_str(&content).map_err(|e| format!("Parse mcp.json: {e}"))?;
    Ok((path, doc))
}

fn write_doc(path: &PathBuf, doc: &Value) -> Result<(), String> {
    let serialized = serde_json::to_string_pretty(doc).map_err(|e| format!("Serialize: {e}"))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, &serialized).map_err(|e| format!("Write tmp: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("Rename tmp: {e}"))
}

/// Read the current `servers.europa.env` map (empty if missing).
fn europa_env() -> serde_json::Map<String, Value> {
    read_doc()
        .ok()
        .and_then(|(_, doc)| {
            doc.pointer("/servers/europa/env")
                .and_then(|v| v.as_object())
                .cloned()
        })
        .unwrap_or_default()
}

fn env_str(env: &serde_json::Map<String, Value>, key: &str) -> String {
    env.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string()
}

/// Apply a set of env updates to `servers.europa.env` (creating it if needed).
fn set_europa_env(updates: &[(&str, String)]) -> Result<(), String> {
    let (path, mut doc) = read_doc()?;
    let env = doc
        .pointer_mut("/servers/europa/env")
        .and_then(|v| v.as_object_mut())
        .ok_or_else(|| "servers.europa.env mancante in .mcp.json".to_string())?;
    for (k, v) in updates {
        env.insert((*k).to_string(), Value::String(v.clone()));
    }
    write_doc(&path, &doc)
}

fn remove_europa_env(keys: &[&str]) -> Result<(), String> {
    let (path, mut doc) = read_doc()?;
    if let Some(env) = doc
        .pointer_mut("/servers/europa/env")
        .and_then(|v| v.as_object_mut())
    {
        for k in keys {
            env.remove(*k);
        }
    }
    write_doc(&path, &doc)
}

fn keyring_set(account: &str, secret: &str) {
    // Best-effort mirror into the OS keyring (env is the source of truth for the daemon).
    if let Ok(e) = Entry::new(SERVICE, account) {
        let _ = e.set_password(secret);
    }
}

fn keyring_delete(account: &str) {
    if let Ok(e) = Entry::new(SERVICE, account) {
        let _ = e.delete_credential();
    }
}

/// Absolute Europa data dir (where moon-europa keeps the model + telegram session).
fn europa_data_dir() -> PathBuf {
    let raw = env_str(&europa_env(), "DATA_DIR");
    let raw = if raw.is_empty() { "moon-europa-rs/data".to_string() } else { raw };
    let p = PathBuf::from(&raw);
    if p.is_absolute() {
        p
    } else {
        crate::config::get_base_dir().join(p)
    }
}

fn telegram_session_path() -> PathBuf {
    europa_data_dir().join("telegram")
}

/// Baileys session dir for WhatsApp (where the companion-device creds live).
fn whatsapp_session_dir() -> PathBuf {
    europa_data_dir().join("whatsapp")
}

/// True only if the Baileys `creds.json` is a USABLE companion session, not just
/// present. A SIGKILL mid-write (or an aborted pairing) can leave a 0-byte or
/// half-written `creds.json` that still "exists" but never links — showing it as
/// "Sessione salvata" misleads the user into waiting instead of re-pairing. We
/// require: present, non-empty, valid JSON, and a linked identity (`me.id`).
/// Mirrors moon-europa's `config::whatsapp_session_valid` — and like it must NOT
/// gate on `creds.registered`, which Baileys never sets for QR linking.
fn whatsapp_session_valid() -> bool {
    let creds = whatsapp_session_dir().join("creds.json");
    match std::fs::read_to_string(&creds) {
        Ok(s) if !s.trim().is_empty() => serde_json::from_str::<Value>(&s)
            .ok()
            .and_then(|v| {
                v.get("me")
                    .and_then(|me| me.get("id"))
                    .and_then(|id| id.as_str())
                    .map(|id| !id.is_empty())
            })
            .unwrap_or(false),
        _ => false,
    }
}

/// Absolute path of the WhatsApp Baileys helper script. Derived from the europa
/// DATA_DIR (`.../moon-europa-rs/data`) whose parent is the crate root, so it
/// matches moon-europa's own default and works regardless of where the GUI binary
/// lives (get_base_dir points at the binary dir, NOT the repo root).
fn whatsapp_helper_path() -> PathBuf {
    let data = europa_data_dir();
    let root = data.parent().map(|p| p.to_path_buf()).unwrap_or(data);
    root.join("whatsapp-helper").join("helper.mjs")
}

/// Path of the `<channel>.connected` marker that moon-europa writes when it
/// actually connects that channel (and removes on failure). The panel reads
/// this to show the REAL connection state, not just "credentials saved".
fn channel_connected_marker(channel: &str) -> PathBuf {
    europa_data_dir().join(format!("{channel}.connected"))
}

// ---------------------------------------------------------------------------
// Channel status (for the UI list)
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct EuropaChannel {
    pub channel: String,
    /// "user" | "bot" | "token" | "oauth" | "none"
    pub mode: String,
    pub configured: bool,
    pub authorized: bool,
    pub detail: String,
}

#[tauri::command]
pub fn europa_channels_list() -> Result<Vec<EuropaChannel>, String> {
    let env = europa_env();
    let mut out = Vec::new();

    // Telegram — bot token takes precedence over user account.
    let bot = env_str(&env, "TELEGRAM_BOT_TOKEN");
    let api_id = env_str(&env, "TELEGRAM_API_ID");
    let api_hash = env_str(&env, "TELEGRAM_API_HASH");
    if !bot.is_empty() {
        out.push(EuropaChannel {
            channel: "telegram".into(),
            mode: "bot".into(),
            configured: true,
            authorized: true,
            detail: "Bot token configurato".into(),
        });
    } else {
        let configured = api_id != "YOUR_API_ID"
            && !api_id.is_empty()
            && api_id != "0"
            && !api_hash.is_empty()
            && api_hash != "YOUR_API_HASH";
        let authorized = channel_connected_marker("telegram").exists();
        out.push(EuropaChannel {
            channel: "telegram".into(),
            mode: "user".into(),
            configured,
            authorized,
            detail: if !configured {
                "API id/hash da inserire".into()
            } else if authorized {
                "Account collegato".into()
            } else {
                "API salvate — fai il login (telefono + codice)".into()
            },
        });
    }

    // Slack
    let slack = env_str(&env, "SLACK_BOT_TOKEN");
    let slack_connected = channel_connected_marker("slack").exists();
    out.push(EuropaChannel {
        channel: "slack".into(),
        mode: "token".into(),
        configured: !slack.is_empty(),
        authorized: slack_connected,
        detail: if slack.is_empty() {
            "Bot token (xoxb-…) da inserire".into()
        } else if slack_connected {
            "Workspace collegato".into()
        } else {
            "Token salvato — connessione non confermata".into()
        },
    });

    // Teams
    let teams_refresh = env_str(&env, "TEAMS_REFRESH_TOKEN");
    let teams_client = env_str(&env, "TEAMS_CLIENT_ID");
    let teams_connected = channel_connected_marker("teams").exists();
    out.push(EuropaChannel {
        channel: "teams".into(),
        mode: "oauth".into(),
        configured: !teams_refresh.is_empty() && !teams_client.is_empty(),
        authorized: teams_connected,
        detail: if teams_refresh.is_empty() {
            "Login Microsoft da completare".into()
        } else if teams_connected {
            "Account Microsoft collegato".into()
        } else {
            "Login fatto — connessione non confermata".into()
        },
    });

    // WhatsApp (personal, Baileys companion device)
    let wa_valid = whatsapp_session_valid();
    let wa_enabled = env_str(&env, "WHATSAPP_ENABLED") == "true" || wa_valid;
    // A session dir/creds file present but NOT valid = corrupt/aborted pairing;
    // the user must re-pair, not wait for a connection that can never happen.
    let wa_corrupt = !wa_valid && whatsapp_session_dir().join("creds.json").exists();
    let wa_connected = channel_connected_marker("whatsapp").exists();
    out.push(EuropaChannel {
        channel: "whatsapp".into(),
        mode: "user".into(),
        configured: wa_enabled && !wa_corrupt,
        authorized: wa_connected,
        detail: if wa_corrupt {
            "Sessione non valida — rimuovi e ri-accoppia col QR".into()
        } else if !wa_enabled {
            "Collega il tuo WhatsApp con il QR".into()
        } else if wa_connected {
            "WhatsApp collegato".into()
        } else {
            "Sessione salvata — connessione non confermata".into()
        },
    });

    Ok(out)
}

// ---------------------------------------------------------------------------
// Telegram — user account (OTP login in-process)
// ---------------------------------------------------------------------------

pub struct PendingLogin {
    client: Client,
    runner: tokio::task::JoinHandle<()>,
    token: Option<LoginToken>,
    password_token: Option<PasswordToken>,
}

#[derive(Default)]
pub struct TelegramAuth(pub Mutex<Option<PendingLogin>>);

/// Finalize a successful login: force a round-trip with `get_me()` so grammers
/// fully persists the authorized session to SQLite (this is what
/// `auth_interactive` does), then disconnect and give the runner a brief moment
/// to flush its writes before aborting it. Without this, moon-europa reloads the
/// session and reports "not authorized".
async fn finalize_session(pending: PendingLogin) {
    let PendingLogin { client, runner, .. } = pending;
    let _ = client.get_me().await;
    client.disconnect();
    tokio::time::sleep(Duration::from_millis(800)).await;
    runner.abort();
    // Note: we do NOT mark the channel "connected" here — that is owned by
    // moon-europa, which writes the marker only if it can actually connect with
    // this session. So the GUI never claims "connected" on a login that the
    // server can't use.
}

/// Save the Telegram API id/hash and pin an absolute DATA_DIR (so the session
/// path is identical for moon-europa and for the login below).
#[tauri::command]
pub fn telegram_save_api(api_id: String, api_hash: String) -> Result<(), String> {
    let api_id = api_id.trim().to_string();
    let api_hash = api_hash.trim().to_string();
    if api_id.parse::<i32>().map(|n| n == 0).unwrap_or(true) {
        return Err("API ID non valido (deve essere un numero).".into());
    }
    if api_hash.is_empty() {
        return Err("API hash vuoto.".into());
    }
    let abs_data = europa_data_dir().to_string_lossy().to_string();
    set_europa_env(&[
        ("TELEGRAM_API_ID", api_id),
        ("TELEGRAM_API_HASH", api_hash.clone()),
        ("DATA_DIR", abs_data),
    ])?;
    keyring_set("telegram_api_hash", &api_hash);
    Ok(())
}

/// Open a fresh grammers session and request a login code for `phone`.
/// IMPORTANT: stop the `europa` service first (the GUI does this) so two
/// processes don't open the same SQLite session.
#[tauri::command]
pub async fn telegram_request_code(
    phone: String,
    state: State<'_, TelegramAuth>,
) -> Result<String, String> {
    let env = europa_env();
    let api_id: i32 = env_str(&env, "TELEGRAM_API_ID")
        .parse()
        .map_err(|_| "API ID mancante o non valido. Salva prima le API.".to_string())?;
    let api_hash = env_str(&env, "TELEGRAM_API_HASH");
    if api_hash.is_empty() {
        return Err("API hash mancante. Salva prima le API.".into());
    }
    let phone = phone.trim().to_string();
    if phone.is_empty() {
        return Err("Numero di telefono vuoto.".into());
    }

    let session_path = telegram_session_path();
    std::fs::create_dir_all(session_path.parent().unwrap_or(&PathBuf::from(".")))
        .map_err(|e| format!("Create data dir: {e}"))?;

    let session = SqliteSession::open(&session_path)
        .await
        .map_err(|e| format!("Open session: {e}"))?;
    let session = Arc::new(session);

    let SenderPool { runner, handle, updates: _ } = SenderPool::new(session, api_id);
    let runner_handle = tokio::spawn(async move { runner.run().await });
    let client = Client::new(handle);

    if client
        .is_authorized()
        .await
        .map_err(|e| format!("Check auth: {e}"))?
    {
        client.disconnect();
        runner_handle.abort();
        return Ok("already_authorized".into());
    }

    let token = client
        .request_login_code(&phone, &api_hash)
        .await
        .map_err(|e| {
            runner_handle.abort();
            format!("Richiesta codice fallita: {e}")
        })?;

    *state.0.lock().unwrap() = Some(PendingLogin {
        client,
        runner: runner_handle,
        token: Some(token),
        password_token: None,
    });
    Ok("code_sent".into())
}

/// Submit the OTP code. Returns "done" or "password_required" (2FA).
#[tauri::command]
pub async fn telegram_submit_code(
    code: String,
    state: State<'_, TelegramAuth>,
) -> Result<String, String> {
    let mut pending = state
        .0
        .lock()
        .unwrap()
        .take()
        .ok_or("Nessun login in corso. Richiedi prima il codice.")?;
    let code = code.trim().to_string();

    // sign_in borrows the token, so a wrong code is retryable — keep the
    // pending login alive instead of tearing it down.
    let outcome = match pending.token.as_ref() {
        Some(token) => pending.client.sign_in(token, &code).await,
        None => return Err("Token di login mancante. Riavvia il login.".into()),
    };

    match outcome {
        Ok(_user) => {
            finalize_session(pending).await;
            Ok("done".into())
        }
        Err(SignInError::PasswordRequired(pt)) => {
            pending.password_token = Some(pt);
            *state.0.lock().unwrap() = Some(pending);
            Ok("password_required".into())
        }
        Err(SignInError::InvalidCode) => {
            // Wrong code: keep the session so the user can re-enter it.
            *state.0.lock().unwrap() = Some(pending);
            Err("Codice non corretto. Riprova.".into())
        }
        Err(e) => {
            pending.client.disconnect();
            pending.runner.abort();
            Err(format!("Login fallito: {e}"))
        }
    }
}

/// Submit the 2FA password (only when telegram_submit_code returned "password_required").
#[tauri::command]
pub async fn telegram_submit_password(
    password: String,
    state: State<'_, TelegramAuth>,
) -> Result<String, String> {
    let mut pending = state
        .0
        .lock()
        .unwrap()
        .take()
        .ok_or("Nessun login in corso. Riavvia il login.")?;
    let pt = pending
        .password_token
        .take()
        .ok_or("Password 2FA non richiesta in questo flusso.")?;

    match pending.client.check_password(pt, password.trim()).await {
        Ok(_user) => {
            finalize_session(pending).await;
            Ok("done".into())
        }
        Err(SignInError::InvalidPassword(pt2)) => {
            // Wrong password: Telegram returns a fresh token, so keep the
            // session alive and let the user retry.
            pending.password_token = Some(pt2);
            *state.0.lock().unwrap() = Some(pending);
            Err("Password 2FA non corretta. Riprova.".into())
        }
        Err(e) => {
            pending.client.disconnect();
            pending.runner.abort();
            Err(format!("Login 2FA fallito: {e}"))
        }
    }
}

// ---------------------------------------------------------------------------
// Telegram — bot token
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn telegram_save_bot(token: String) -> Result<(), String> {
    let token = token.trim().to_string();
    if token.is_empty() || !token.contains(':') {
        return Err("Token bot non valido (formato 123456:ABC...).".into());
    }
    set_europa_env(&[("TELEGRAM_BOT_TOKEN", token.clone())])?;
    keyring_set("telegram_bot_token", &token);
    Ok(())
}

// ---------------------------------------------------------------------------
// Slack — bot token
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn slack_save(token: String) -> Result<(), String> {
    let token = token.trim().to_string();
    if !token.starts_with("xoxb-") {
        return Err("Il bot token Slack deve iniziare con 'xoxb-'.".into());
    }
    set_europa_env(&[("SLACK_BOT_TOKEN", token.clone())])?;
    keyring_set("slack_bot_token", &token);
    Ok(())
}

// ---------------------------------------------------------------------------
// Teams — OAuth2 device-code flow (Microsoft Graph)
// ---------------------------------------------------------------------------

pub struct PendingDevice {
    client_id: String,
    tenant_id: String,
    device_code: String,
    interval: u64,
    last_poll: Instant,
    http: reqwest::Client,
}

#[derive(Default)]
pub struct TeamsAuth(pub Mutex<Option<PendingDevice>>);

#[derive(Serialize)]
pub struct DeviceCodeInfo {
    pub user_code: String,
    pub verification_uri: String,
    pub message: String,
}

/// Start the Microsoft device-code flow. Returns the code/URL to show the user.
#[tauri::command]
pub async fn teams_start_device_code(
    client_id: String,
    tenant_id: String,
    state: State<'_, TeamsAuth>,
) -> Result<DeviceCodeInfo, String> {
    let client_id = client_id.trim().to_string();
    let tenant_id = {
        let t = tenant_id.trim();
        if t.is_empty() { "common".to_string() } else { t.to_string() }
    };
    if client_id.is_empty() {
        return Err("Client ID (Azure app) mancante.".into());
    }

    let http = reqwest::Client::new();
    let url = format!("https://login.microsoftonline.com/{tenant_id}/oauth2/v2.0/devicecode");
    let resp = http
        .post(&url)
        .form(&[
            ("client_id", client_id.as_str()),
            ("scope", "offline_access Chat.ReadWrite User.Read"),
        ])
        .send()
        .await
        .map_err(|e| format!("Device code: {e}"))?;
    let body: Value = resp.json().await.map_err(|e| format!("Device code decode: {e}"))?;

    let device_code = body
        .get("device_code")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            body.get("error_description")
                .and_then(|v| v.as_str())
                .unwrap_or("device_code mancante")
                .to_string()
        })?
        .to_string();
    let user_code = body.get("user_code").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let verification_uri = body
        .get("verification_uri")
        .and_then(|v| v.as_str())
        .unwrap_or("https://microsoft.com/devicelogin")
        .to_string();
    let message = body
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let interval = body.get("interval").and_then(|v| v.as_u64()).unwrap_or(5);

    *state.0.lock().unwrap() = Some(PendingDevice {
        client_id,
        tenant_id,
        device_code,
        interval,
        last_poll: Instant::now(),
        http,
    });

    Ok(DeviceCodeInfo { user_code, verification_uri, message })
}

/// Poll the token endpoint once. Returns "pending" | "done". On "done", the
/// refresh token + client/tenant are persisted to .mcp.json + keyring.
#[tauri::command]
pub async fn teams_poll_device_code(state: State<'_, TeamsAuth>) -> Result<String, String> {
    // Snapshot the pending data without holding the lock across awaits.
    let (client_id, tenant_id, device_code, interval, http) = {
        let guard = state.0.lock().unwrap();
        let p = guard.as_ref().ok_or("Nessun login Teams in corso.")?;
        if p.last_poll.elapsed() < Duration::from_secs(p.interval) {
            return Ok("pending".into());
        }
        (
            p.client_id.clone(),
            p.tenant_id.clone(),
            p.device_code.clone(),
            p.interval,
            p.http.clone(),
        )
    };

    let url = format!("https://login.microsoftonline.com/{tenant_id}/oauth2/v2.0/token");
    let resp = http
        .post(&url)
        .form(&[
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ("client_id", client_id.as_str()),
            ("device_code", device_code.as_str()),
        ])
        .send()
        .await
        .map_err(|e| format!("Token poll: {e}"))?;
    let body: Value = resp.json().await.map_err(|e| format!("Token decode: {e}"))?;

    if let Some(refresh) = body.get("refresh_token").and_then(|v| v.as_str()) {
        set_europa_env(&[
            ("TEAMS_CLIENT_ID", client_id),
            ("TEAMS_TENANT_ID", tenant_id),
            ("TEAMS_REFRESH_TOKEN", refresh.to_string()),
        ])?;
        keyring_set("teams_refresh_token", refresh);
        *state.0.lock().unwrap() = None;
        return Ok("done".into());
    }

    // Update last_poll and interpret the pending/expired error.
    let error = body.get("error").and_then(|v| v.as_str()).unwrap_or("");
    {
        let mut guard = state.0.lock().unwrap();
        if let Some(p) = guard.as_mut() {
            p.last_poll = Instant::now();
            if error == "slow_down" {
                p.interval += 5;
            }
        }
    }
    let _ = interval;
    match error {
        "authorization_pending" | "slow_down" | "" => Ok("pending".into()),
        "authorization_declined" => Err("Accesso rifiutato dall'utente.".into()),
        "expired_token" => {
            *state.0.lock().unwrap() = None;
            Err("Codice scaduto, riprova.".into())
        }
        other => Err(format!("Errore login Teams: {other}")),
    }
}

// ---------------------------------------------------------------------------
// WhatsApp — personal account via the Baileys helper (QR pairing)
// ---------------------------------------------------------------------------

#[derive(Default, Clone, Serialize)]
pub struct WhatsappStatus {
    /// "idle" | "waiting" (QR shown, not yet scanned) | "connected" | "error"
    pub status: String,
    /// Current QR string to render (rotates ~every 20s until scanned).
    pub qr: Option<String>,
    pub error: Option<String>,
}

struct WaPairing {
    child: tokio::process::Child,
    shared: Arc<Mutex<WhatsappStatus>>,
}

#[derive(Default)]
pub struct WhatsappPairing(Mutex<Option<WaPairing>>);

/// Start the WhatsApp pairing: spawn the helper in `pair` mode. The QR string
/// is then read via `whatsapp_pairing_status`. The GUI stops `europa` first (so
/// two processes don't open the same session), and restarts it once connected.
#[tauri::command]
pub async fn whatsapp_start_pairing(state: State<'_, WhatsappPairing>) -> Result<(), String> {
    // Kill any previous pairing attempt.
    if let Some(mut prev) = state.0.lock().unwrap().take() {
        let _ = prev.child.start_kill();
    }

    let helper = whatsapp_helper_path();
    if !helper.exists() {
        return Err(format!(
            "Helper WhatsApp non trovato in {helper:?}. Esegui `npm ci` in moon-europa-rs/whatsapp-helper."
        ));
    }
    let session = whatsapp_session_dir();
    std::fs::create_dir_all(&session).map_err(|e| format!("Create session dir: {e}"))?;

    let mut child = Command::new("node")
        .arg(&helper)
        .arg("pair")
        .arg("--session")
        .arg(&session)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Avvio helper (node) fallito: {e}"))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "helper senza stdout".to_string())?;

    let shared = Arc::new(Mutex::new(WhatsappStatus {
        status: "waiting".into(),
        qr: None,
        error: None,
    }));
    let reader_shared = Arc::clone(&shared);

    // Reader task: update the shared status as the helper emits QR/connected/error.
    tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        loop {
            let line = match lines.next_line().await {
                Ok(Some(l)) => l,
                _ => break,
            };
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let v: serde_json::Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(_) => continue,
            };
            match v.get("type").and_then(|t| t.as_str()) {
                Some("qr") => {
                    if let Some(qr) = v.get("qr").and_then(|q| q.as_str()) {
                        let mut s = reader_shared.lock().unwrap();
                        s.status = "waiting".into();
                        s.qr = Some(qr.to_string());
                    }
                }
                Some("connected") => {
                    let mut s = reader_shared.lock().unwrap();
                    s.status = "connected".into();
                    s.qr = None;
                }
                Some("error") => {
                    let mut s = reader_shared.lock().unwrap();
                    s.status = "error".into();
                    s.error = v.get("error").and_then(|e| e.as_str()).map(|x| x.to_string());
                }
                _ => {}
            }
        }
    });

    *state.0.lock().unwrap() = Some(WaPairing { child, shared });
    Ok(())
}

/// Poll the current pairing status (QR string / connected / error). On the first
/// "connected" we persist the WhatsApp config so moon-europa starts the channel.
#[tauri::command]
pub fn whatsapp_pairing_status(state: State<'_, WhatsappPairing>) -> Result<WhatsappStatus, String> {
    let snapshot = {
        let guard = state.0.lock().unwrap();
        match guard.as_ref() {
            Some(p) => p.shared.lock().unwrap().clone(),
            None => WhatsappStatus {
                status: "idle".into(),
                qr: None,
                error: None,
            },
        }
    };

    if snapshot.status == "connected" {
        // Persist config so moon-europa runs the WhatsApp channel on restart.
        let abs_data = europa_data_dir().to_string_lossy().to_string();
        let helper = whatsapp_helper_path().to_string_lossy().to_string();
        set_europa_env(&[
            ("WHATSAPP_ENABLED", "true".to_string()),
            ("WHATSAPP_HELPER", helper),
            ("DATA_DIR", abs_data),
        ])?;
        // The pair-mode helper exits on its own after connecting; drop our handle.
        if let Some(mut p) = state.0.lock().unwrap().take() {
            let _ = p.child.start_kill();
        }
    }

    Ok(snapshot)
}

/// Cancel an in-progress pairing (kills the helper).
#[tauri::command]
pub fn whatsapp_cancel_pairing(state: State<'_, WhatsappPairing>) -> Result<(), String> {
    if let Some(mut p) = state.0.lock().unwrap().take() {
        let _ = p.child.start_kill();
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Channel removal
// ---------------------------------------------------------------------------

/// Remove a channel's credentials from .mcp.json + keyring.
#[tauri::command]
pub fn europa_channel_remove(channel: String) -> Result<(), String> {
    match channel.as_str() {
        "telegram" => {
            remove_europa_env(&["TELEGRAM_BOT_TOKEN", "TELEGRAM_API_ID", "TELEGRAM_API_HASH"])?;
            keyring_delete("telegram_api_hash");
            keyring_delete("telegram_bot_token");
            // Remove the saved session so the account is fully disconnected.
            let _ = std::fs::remove_file(telegram_session_path());
            let _ = std::fs::remove_file(channel_connected_marker("telegram"));
        }
        "slack" => {
            remove_europa_env(&["SLACK_BOT_TOKEN"])?;
            keyring_delete("slack_bot_token");
            let _ = std::fs::remove_file(channel_connected_marker("slack"));
        }
        "teams" => {
            remove_europa_env(&["TEAMS_CLIENT_ID", "TEAMS_TENANT_ID", "TEAMS_REFRESH_TOKEN"])?;
            keyring_delete("teams_refresh_token");
            let _ = std::fs::remove_file(channel_connected_marker("teams"));
        }
        "whatsapp" => {
            remove_europa_env(&["WHATSAPP_ENABLED"])?;
            // Remove the Baileys session so the companion device is unlinked.
            let _ = std::fs::remove_dir_all(whatsapp_session_dir());
            let _ = std::fs::remove_file(channel_connected_marker("whatsapp"));
        }
        other => return Err(format!("Canale sconosciuto: {other}")),
    }
    Ok(())
}
