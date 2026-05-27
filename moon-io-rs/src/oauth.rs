//! OAuth 2.0 (Authorization Code Flow with PKCE) for Gmail IMAP/SMTP.
//!
//! Flow:
//! 1. Caller invokes `authorize_interactive(account, config)`.
//! 2. We spawn a one-shot HTTP listener on `http://127.0.0.1:<ephemeral_port>/oauth/callback`.
//! 3. We build an authorization URL and open the system browser. User logs in
//!    on Google and grants `https://mail.google.com/` scope.
//! 4. Google redirects to our local listener with `?code=...&state=...`.
//! 5. We exchange the code for `access_token` + `refresh_token` via POST to
//!    Google's token endpoint.
//! 6. `refresh_token` (+ client_id/secret) is stored encrypted in the OS
//!    keyring under target `MoonIo:oauth:<account>`. `access_token` is cached
//!    in-process and auto-refreshed when expired.
//!
//! Storage layout:
//! - Keyring service: `"MoonIo"`, target: `oauth:<account>` (e.g. `oauth:gmail`)
//! - Value: JSON `{ client_id, client_secret, refresh_token }`
//! - Access tokens are NEVER persisted — refreshed on demand from refresh_token.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use keyring::Entry;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};
use uuid::Uuid;

const SERVICE: &str = "MoonIo";
const GOOGLE_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const GMAIL_SCOPE: &str = "https://mail.google.com/";

/// Persistent OAuth credentials for one account.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthCredentials {
    pub client_id: String,
    pub client_secret: String,
    pub refresh_token: String,
}

/// In-memory cached access token (valid up to `expires_at`).
#[derive(Debug, Clone)]
struct CachedAccessToken {
    token: String,
    expires_at: Instant,
}

/// Process-wide cache of access tokens keyed by account name.
/// Avoids hammering Google's token endpoint with refresh requests.
#[derive(Default, Clone)]
pub struct AccessTokenCache {
    inner: Arc<Mutex<HashMap<String, CachedAccessToken>>>,
}

impl AccessTokenCache {
    pub fn new() -> Self {
        Self::default()
    }

    async fn get(&self, account: &str) -> Option<String> {
        let guard = self.inner.lock().await;
        guard.get(account).and_then(|t| {
            // 30-second safety margin
            if t.expires_at > Instant::now() + Duration::from_secs(30) {
                Some(t.token.clone())
            } else {
                None
            }
        })
    }

    async fn set(&self, account: &str, token: String, expires_in_secs: u64) {
        let mut guard = self.inner.lock().await;
        guard.insert(
            account.to_string(),
            CachedAccessToken {
                token,
                expires_at: Instant::now() + Duration::from_secs(expires_in_secs),
            },
        );
    }

    pub async fn clear(&self, account: &str) {
        let mut guard = self.inner.lock().await;
        guard.remove(account);
    }
}

// ---------------------------------------------------------------------------
// Keyring storage
// ---------------------------------------------------------------------------

fn entry(account: &str) -> Result<Entry, String> {
    Entry::new(SERVICE, &format!("oauth:{account}"))
        .map_err(|e| format!("Keyring init: {e}"))
}

/// Load OAuth credentials for an account from the keyring.
pub fn load_credentials(account: &str) -> Option<OAuthCredentials> {
    let e = entry(account).ok()?;
    let json = e.get_password().ok()?;
    serde_json::from_str(&json).ok()
}

/// Save OAuth credentials for an account.
pub fn save_credentials(account: &str, creds: &OAuthCredentials) -> Result<(), String> {
    let json = serde_json::to_string(creds)
        .map_err(|e| format!("Serialize: {e}"))?;
    entry(account)?
        .set_password(&json)
        .map_err(|e| format!("Keyring set: {e}"))
}

/// Delete OAuth credentials for an account.
pub fn delete_credentials(account: &str) -> Result<bool, String> {
    let e = entry(account)?;
    match e.delete_credential() {
        Ok(_) => Ok(true),
        Err(keyring::Error::NoEntry) => Ok(false),
        Err(e) => Err(format!("Keyring delete: {e}")),
    }
}

/// True if OAuth credentials are stored for this account.
pub fn has_credentials(account: &str) -> bool {
    load_credentials(account).is_some()
}

// ---------------------------------------------------------------------------
// PKCE helpers
// ---------------------------------------------------------------------------

/// Generate a PKCE code_verifier + code_challenge pair.
/// verifier: 64 random URL-safe chars; challenge: SHA256(verifier) base64url.
fn generate_pkce_pair() -> (String, String) {
    use sha2::{Digest, Sha256};
    // 64 random bytes → URL-safe base64 → ~86 chars, plenty of entropy.
    let mut bytes = [0u8; 64];
    getrandom::fill(&mut bytes).expect("getrandom");
    let verifier = URL_SAFE_NO_PAD.encode(bytes);

    let challenge = {
        let mut hasher = Sha256::new();
        hasher.update(verifier.as_bytes());
        URL_SAFE_NO_PAD.encode(hasher.finalize())
    };
    (verifier, challenge)
}

// ---------------------------------------------------------------------------
// Token endpoint responses
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    token_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TokenError {
    error: String,
    #[serde(default)]
    error_description: Option<String>,
}

// ---------------------------------------------------------------------------
// Interactive authorize: opens browser, waits for callback
// ---------------------------------------------------------------------------

/// Start the authorization flow for an account.
///
/// 1. Binds an ephemeral local TCP port to receive Google's redirect.
/// 2. Builds the consent URL with PKCE and opens the user's browser.
/// 3. Waits up to `timeout_secs` for the user to complete consent.
/// 4. Exchanges the returned code for tokens.
/// 5. Stores `OAuthCredentials` (client_id + secret + refresh_token) in keyring.
///
/// Returns the user's Google email address (from the `id_token`, if requested).
pub async fn authorize_interactive(
    account: &str,
    client_id: &str,
    client_secret: &str,
    timeout_secs: u64,
) -> Result<(), String> {
    // Bind to an ephemeral port on 127.0.0.1
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| format!("Cannot bind callback port: {e}"))?;
    let local_addr = listener
        .local_addr()
        .map_err(|e| format!("Local addr: {e}"))?;
    let redirect_uri = format!("http://127.0.0.1:{}/oauth/callback", local_addr.port());
    debug!("OAuth redirect_uri = {redirect_uri}");

    // PKCE + state
    let (verifier, challenge) = generate_pkce_pair();
    let state = Uuid::new_v4().to_string();

    // Build the consent URL
    let mut auth_url = url::Url::parse(GOOGLE_AUTH_URL).map_err(|e| e.to_string())?;
    auth_url
        .query_pairs_mut()
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("response_type", "code")
        .append_pair("scope", GMAIL_SCOPE)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("access_type", "offline") // required to get refresh_token
        .append_pair("prompt", "consent") // force consent so refresh_token is always returned
        .append_pair("state", &state);

    let auth_url_str = auth_url.to_string();
    info!("Opening browser for OAuth consent...");
    if let Err(e) = webbrowser::open(&auth_url_str) {
        warn!(
            "Could not auto-open browser ({e}). Open this URL manually:\n  {auth_url_str}"
        );
    }

    // Wait for callback (with timeout)
    let accept_fut = listener.accept();
    let (mut socket, _peer) =
        match tokio::time::timeout(Duration::from_secs(timeout_secs), accept_fut).await {
            Ok(Ok(s)) => s,
            Ok(Err(e)) => return Err(format!("Accept callback: {e}")),
            Err(_) => return Err("Timeout aspettando il consenso Google".into()),
        };

    // Read the HTTP request line (just need the URL)
    let mut buf = [0u8; 4096];
    let n = socket
        .read(&mut buf)
        .await
        .map_err(|e| format!("Read callback: {e}"))?;
    let request = String::from_utf8_lossy(&buf[..n]);
    let first_line = request.lines().next().unwrap_or("");
    // "GET /oauth/callback?code=...&state=... HTTP/1.1"
    let path_with_query = first_line.split_whitespace().nth(1).unwrap_or("");

    // Send a friendly HTML response to the browser
    let html = "HTTP/1.1 200 OK\r\n\
                Content-Type: text/html; charset=utf-8\r\n\
                Connection: close\r\n\r\n\
                <!doctype html><html><head><meta charset=\"utf-8\"><title>Moon Io</title>\
                <style>body{font-family:system-ui;background:#0e0a06;color:#e5b87a;\
                display:flex;align-items:center;justify-content:center;height:100vh;margin:0}\
                .box{text-align:center;padding:24px;border:1px solid #4a3110;border-radius:8px}\
                h1{margin:0 0 8px;font-size:18px}p{margin:0;color:#a0907a;font-size:13px}</style></head>\
                <body><div class=\"box\"><h1>✓ Moon Io connesso a Google</h1>\
                <p>Puoi chiudere questa scheda e tornare a JupiterOS.</p></div></body></html>";
    let _ = socket.write_all(html.as_bytes()).await;
    let _ = socket.shutdown().await;

    // Parse query string
    let qs_raw = path_with_query
        .split_once('?')
        .map(|(_, q)| q)
        .unwrap_or("");
    let qs: HashMap<String, String> =
        url::form_urlencoded::parse(qs_raw.as_bytes())
            .into_owned()
            .collect();

    if let Some(err) = qs.get("error") {
        return Err(format!(
            "Google ha negato il consenso: {err} ({})",
            qs.get("error_description").map(|s| s.as_str()).unwrap_or("")
        ));
    }

    let returned_state = qs.get("state").cloned().unwrap_or_default();
    if returned_state != state {
        return Err("State mismatch — possibile CSRF, annullato.".into());
    }

    let code = qs
        .get("code")
        .cloned()
        .ok_or_else(|| "Risposta Google senza 'code'".to_string())?;

    // Exchange code for tokens
    let resp = exchange_code(&code, client_id, client_secret, &redirect_uri, &verifier).await?;

    let refresh_token = resp.refresh_token.ok_or_else(|| {
        "Google non ha restituito un refresh_token. \
         Probabilmente avevi già autorizzato l'app prima e Google non lo ridà. \
         Vai su https://myaccount.google.com/permissions, revoca 'Moon Io' e riprova."
            .to_string()
    })?;

    let creds = OAuthCredentials {
        client_id: client_id.to_string(),
        client_secret: client_secret.to_string(),
        refresh_token,
    };
    save_credentials(account, &creds)?;
    info!("OAuth credentials saved for account '{account}'");
    Ok(())
}

async fn exchange_code(
    code: &str,
    client_id: &str,
    client_secret: &str,
    redirect_uri: &str,
    code_verifier: &str,
) -> Result<TokenResponse, String> {
    let client = reqwest::Client::new();
    let params = [
        ("client_id", client_id),
        ("client_secret", client_secret),
        ("code", code),
        ("code_verifier", code_verifier),
        ("grant_type", "authorization_code"),
        ("redirect_uri", redirect_uri),
    ];

    let resp = client
        .post(GOOGLE_TOKEN_URL)
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("Token endpoint: {e}"))?;

    let status = resp.status();
    let body = resp.text().await.map_err(|e| format!("Token body: {e}"))?;
    if status.is_success() {
        serde_json::from_str(&body).map_err(|e| format!("Parse token: {e} (body: {body})"))
    } else {
        let err: TokenError = serde_json::from_str(&body)
            .unwrap_or(TokenError {
                error: "unknown".into(),
                error_description: Some(body),
            });
        Err(format!(
            "Token exchange failed ({status}): {} — {}",
            err.error,
            err.error_description.unwrap_or_default()
        ))
    }
}

// ---------------------------------------------------------------------------
// Refresh + get valid access token
// ---------------------------------------------------------------------------

/// Get a valid access token for an account, refreshing it if needed.
/// Uses an in-process cache to avoid extra requests.
pub async fn get_access_token(
    account: &str,
    cache: &AccessTokenCache,
) -> Result<String, String> {
    if let Some(t) = cache.get(account).await {
        return Ok(t);
    }

    let creds = load_credentials(account)
        .ok_or_else(|| format!("Nessuna credenziale OAuth per account '{account}'"))?;

    let resp = refresh_access_token(&creds).await?;
    cache.set(account, resp.access_token.clone(), resp.expires_in).await;
    Ok(resp.access_token)
}

async fn refresh_access_token(creds: &OAuthCredentials) -> Result<TokenResponse, String> {
    let client = reqwest::Client::new();
    let params = [
        ("client_id", creds.client_id.as_str()),
        ("client_secret", creds.client_secret.as_str()),
        ("refresh_token", creds.refresh_token.as_str()),
        ("grant_type", "refresh_token"),
    ];

    let resp = client
        .post(GOOGLE_TOKEN_URL)
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("Refresh endpoint: {e}"))?;

    let status = resp.status();
    let body = resp.text().await.map_err(|e| format!("Refresh body: {e}"))?;
    if status.is_success() {
        serde_json::from_str(&body).map_err(|e| format!("Parse refresh: {e}"))
    } else {
        let err: TokenError = serde_json::from_str(&body)
            .unwrap_or(TokenError {
                error: "unknown".into(),
                error_description: Some(body),
            });
        Err(format!(
            "Refresh failed ({status}): {} — {}",
            err.error,
            err.error_description.unwrap_or_default()
        ))
    }
}

// ---------------------------------------------------------------------------
// XOAUTH2 SASL string
// ---------------------------------------------------------------------------

/// Build the XOAUTH2 SASL initial response string (already base64-encoded).
/// Format: `base64("user=<email>\x01auth=Bearer <token>\x01\x01")`
pub fn build_xoauth2_string(email: &str, access_token: &str) -> String {
    use base64::engine::general_purpose::STANDARD;
    let raw = format!("user={email}\x01auth=Bearer {access_token}\x01\x01");
    STANDARD.encode(raw.as_bytes())
}
