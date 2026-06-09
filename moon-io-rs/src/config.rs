// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

use std::path::PathBuf;

use crate::credentials;
use crate::oauth;

/// Per-account auth mode.
#[derive(Debug, Clone, PartialEq)]
pub enum AuthMode {
    /// Plain LOGIN with a password (App Password or normal IMAP password).
    Password,
    /// OAuth 2.0 with XOAUTH2 — used for Gmail Workspace where App Passwords
    /// are disabled by admin policy.
    OAuth,
}

/// Per-account email configuration (IMAP + SMTP + CalDAV).
#[derive(Debug, Clone)]
pub struct AccountConfig {
    /// Short identifier used in MCP tool params (e.g. "primary", "gmail", "aruba").
    pub name: String,

    /// Authentication mode (Password vs OAuth XOAUTH2).
    pub auth_mode: AuthMode,

    // IMAP
    pub imap_host: String,
    pub imap_port: u16,
    pub imap_username: String,
    /// Password (used when `auth_mode == Password`). Empty for OAuth accounts —
    /// the access token is obtained at runtime from the refresh_token stored
    /// in the keyring under `oauth:{name}`.
    pub imap_password: String,

    // SMTP
    pub smtp_host: String,
    pub smtp_port: u16,

    // CalDAV (optional)
    pub caldav_url: String,
    pub caldav_username: String,
    pub caldav_password: String,

    // IMAP folder layout. Gmail uses "[Gmail]/Sent Mail", Aruba "INBOX.Sent".
    pub folders: Vec<String>,
    pub sent_folder: String,
}

impl AccountConfig {
    pub fn has_caldav(&self) -> bool {
        !self.caldav_url.is_empty()
    }
}

/// Global Moon Io configuration.
#[derive(Debug, Clone)]
pub struct Config {
    /// All configured email accounts. First entry is the default.
    pub accounts: Vec<AccountConfig>,

    // Storage
    pub data_dir: PathBuf,
    pub onnx_model_dir: PathBuf,

    // Server
    pub mcp_host: String,
    pub mcp_port: u16,
    pub mcp_transport: String,

    // Qdrant
    pub qdrant_url: String,

    // Indexer
    pub index_interval_secs: u64,

    // Signature
    pub signature_dir: PathBuf,
}

impl Config {
    /// Load configuration from environment variables.
    ///
    /// Account discovery:
    /// - Primary account from `IMAP_USERNAME`/`IMAP_PASSWORD` (+ optional
    ///   `IMAP_HOST`, `IMAP_PORT`, `SMTP_HOST`, `SMTP_PORT`). Named "primary"
    ///   unless `IMAP_ACCOUNT_NAME` is set.
    /// - Gmail account from `GMAIL_USERNAME`/`GMAIL_PASSWORD` (App Password).
    /// - Generic accounts via `ACCOUNTS=name1,name2,...` then per-account env:
    ///   `{NAME}_IMAP_USERNAME`, `{NAME}_IMAP_PASSWORD`, etc.
    pub fn from_env() -> Result<Self, String> {
        let data_dir = PathBuf::from(
            std::env::var("CHROMA_PERSIST_DIR")
                .or_else(|_| std::env::var("DATA_DIR"))
                .unwrap_or_else(|_| "../data".into()),
        );

        let mut accounts: Vec<AccountConfig> = Vec::new();

        // --- Primary account (legacy IMAP_* env) ---
        let primary_user = std::env::var("IMAP_USERNAME").unwrap_or_default();
        let primary_name_env = std::env::var("IMAP_ACCOUNT_NAME").unwrap_or_default();
        let primary_lookup_name = if primary_name_env.is_empty() {
            "primary".to_string()
        } else {
            primary_name_env.clone()
        };
        // Password resolution: env first (back-compat), then OS keyring.
        let primary_pass = match std::env::var("IMAP_PASSWORD") {
            Ok(p) if !p.is_empty() => p,
            _ => credentials::get_password(&primary_lookup_name).unwrap_or_default(),
        };
        if !primary_user.is_empty() && !primary_pass.is_empty() {
            let imap_host =
                std::env::var("IMAP_HOST").unwrap_or_else(|_| "imaps.aruba.it".into());
            let imap_port: u16 = std::env::var("IMAP_PORT")
                .unwrap_or_else(|_| "993".into())
                .parse()
                .unwrap_or(993);
            let smtp_host = std::env::var("SMTP_HOST").unwrap_or_else(|_| {
                imap_host.replace("imaps.", "smtps.").replace("imap.", "smtp.")
            });
            let smtp_port: u16 = std::env::var("SMTP_PORT")
                .unwrap_or_else(|_| "465".into())
                .parse()
                .unwrap_or(465);

            let folders_str = std::env::var("IMAP_FOLDERS")
                .unwrap_or_else(|_| "INBOX,INBOX.Sent".into());
            let folders: Vec<String> = folders_str
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();

            let sent_folder = std::env::var("IMAP_SENT_FOLDER")
                .unwrap_or_else(|_| "INBOX.Sent".into());

            accounts.push(AccountConfig {
                name: primary_lookup_name.clone(),
                auth_mode: AuthMode::Password,
                imap_host,
                imap_port,
                imap_username: primary_user.clone(),
                imap_password: primary_pass.clone(),
                smtp_host,
                smtp_port,
                caldav_url: std::env::var("CALDAV_URL").unwrap_or_default(),
                caldav_username: std::env::var("CALDAV_USERNAME")
                    .unwrap_or_else(|_| primary_user.clone()),
                caldav_password: std::env::var("CALDAV_PASSWORD")
                    .unwrap_or_else(|_| primary_pass.clone()),
                folders,
                sent_folder,
            });
        }

        // --- Gmail account ---
        // Prefer OAuth: if oauth credentials are stored under "oauth:gmail",
        // treat the account as OAuth-based. The username comes from env
        // (GMAIL_USERNAME) — non-sensitive, stays in .mcp.json.
        let gmail_user = std::env::var("GMAIL_USERNAME").unwrap_or_default();
        if !gmail_user.is_empty() && oauth::has_credentials("gmail") {
            accounts.push(AccountConfig {
                name: "gmail".into(),
                auth_mode: AuthMode::OAuth,
                imap_host: std::env::var("GMAIL_IMAP_HOST")
                    .unwrap_or_else(|_| "imap.gmail.com".into()),
                imap_port: 993,
                imap_username: gmail_user.clone(),
                imap_password: String::new(), // unused for OAuth
                smtp_host: std::env::var("GMAIL_SMTP_HOST")
                    .unwrap_or_else(|_| "smtp.gmail.com".into()),
                smtp_port: 465,
                caldav_url: String::new(),
                caldav_username: gmail_user.clone(),
                caldav_password: String::new(),
                folders: vec!["INBOX".into(), "[Gmail]/Sent Mail".into()],
                sent_folder: "[Gmail]/Sent Mail".into(),
            });
        } else {
            // Fallback: password (App Password) — for users who can still use it
            let gmail_pass = match std::env::var("GMAIL_PASSWORD") {
                Ok(p) if !p.is_empty() => p,
                _ => credentials::get_password("gmail").unwrap_or_default(),
            };
            if !gmail_user.is_empty() && !gmail_pass.is_empty() {
                accounts.push(AccountConfig {
                    name: "gmail".into(),
                    auth_mode: AuthMode::Password,
                    imap_host: std::env::var("GMAIL_IMAP_HOST")
                        .unwrap_or_else(|_| "imap.gmail.com".into()),
                    imap_port: 993,
                    imap_username: gmail_user.clone(),
                    imap_password: gmail_pass.clone(),
                    smtp_host: std::env::var("GMAIL_SMTP_HOST")
                        .unwrap_or_else(|_| "smtp.gmail.com".into()),
                    smtp_port: 465,
                    caldav_url: String::new(),
                    caldav_username: gmail_user.clone(),
                    caldav_password: gmail_pass.clone(),
                    folders: vec!["INBOX".into(), "[Gmail]/Sent Mail".into()],
                    sent_folder: "[Gmail]/Sent Mail".into(),
                });
            }
        }

        // --- Generic accounts: ACCOUNTS=foo,bar with FOO_IMAP_USERNAME etc. ---
        if let Ok(list) = std::env::var("ACCOUNTS") {
            for name in list.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
                if accounts.iter().any(|a| a.name == name) {
                    continue;
                }
                let prefix = name
                    .to_uppercase()
                    .chars()
                    .map(|c| if c.is_alphanumeric() { c } else { '_' })
                    .collect::<String>();
                let user = std::env::var(format!("{prefix}_IMAP_USERNAME")).unwrap_or_default();
                let pass = match std::env::var(format!("{prefix}_IMAP_PASSWORD")) {
                    Ok(p) if !p.is_empty() => p,
                    _ => credentials::get_password(name).unwrap_or_default(),
                };
                if user.is_empty() || pass.is_empty() {
                    continue;
                }
                let imap_host = std::env::var(format!("{prefix}_IMAP_HOST"))
                    .unwrap_or_else(|_| "imap.gmail.com".into());
                let imap_port: u16 = std::env::var(format!("{prefix}_IMAP_PORT"))
                    .unwrap_or_else(|_| "993".into())
                    .parse()
                    .unwrap_or(993);
                let smtp_host = std::env::var(format!("{prefix}_SMTP_HOST"))
                    .unwrap_or_else(|_| {
                        imap_host.replace("imap.", "smtp.").replace("imaps.", "smtps.")
                    });
                let smtp_port: u16 = std::env::var(format!("{prefix}_SMTP_PORT"))
                    .unwrap_or_else(|_| "465".into())
                    .parse()
                    .unwrap_or(465);
                // Same default as the primary account: "INBOX,INBOX.Sent". This
                // also matters because the daemon only auto-discovers ALL folders
                // when folders == the default pair; a bare "INBOX" was treated as
                // an explicit user choice and skipped Sent (sent mail unindexed).
                let folders_str = std::env::var(format!("{prefix}_IMAP_FOLDERS"))
                    .unwrap_or_else(|_| "INBOX,INBOX.Sent".into());
                let folders: Vec<String> = folders_str
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                let sent_folder = std::env::var(format!("{prefix}_SENT_FOLDER"))
                    .unwrap_or_else(|_| "INBOX.Sent".into());

                accounts.push(AccountConfig {
                    name: name.to_string(),
                    auth_mode: AuthMode::Password,
                    imap_host,
                    imap_port,
                    imap_username: user.clone(),
                    imap_password: pass.clone(),
                    smtp_host,
                    smtp_port,
                    caldav_url: std::env::var(format!("{prefix}_CALDAV_URL")).unwrap_or_default(),
                    caldav_username: std::env::var(format!("{prefix}_CALDAV_USERNAME"))
                        .unwrap_or_else(|_| user.clone()),
                    caldav_password: std::env::var(format!("{prefix}_CALDAV_PASSWORD"))
                        .unwrap_or_else(|_| pass.clone()),
                    folders,
                    sent_folder,
                });
            }
        }

        let onnx_model_dir = data_dir.join("model");
        let signature_dir = data_dir.join("signatures");

        Ok(Self {
            accounts,
            data_dir: data_dir.clone(),
            onnx_model_dir,
            mcp_host: std::env::var("MCP_HOST").unwrap_or_else(|_| "127.0.0.1".into()),
            mcp_port: std::env::var("MCP_PORT")
                .unwrap_or_else(|_| "8100".into())
                .parse()
                .unwrap_or(8100),
            mcp_transport: std::env::var("MCP_TRANSPORT").unwrap_or_else(|_| "stdio".into()),
            qdrant_url: std::env::var("QDRANT_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:6334".into()),
            index_interval_secs: std::env::var("INDEX_INTERVAL_SECONDS")
                .unwrap_or_else(|_| "300".into())
                .parse()
                .unwrap_or(300),
            signature_dir,
        })
    }

    /// At least one account is configured.
    pub fn has_email(&self) -> bool {
        !self.accounts.is_empty()
    }

    /// Default (primary) account is the first entry.
    pub fn default_account(&self) -> Option<&AccountConfig> {
        self.accounts.first()
    }

    /// Lookup an account by name (case-insensitive).
    pub fn account(&self, name: &str) -> Option<&AccountConfig> {
        self.accounts
            .iter()
            .find(|a| a.name.eq_ignore_ascii_case(name))
    }

    /// Lookup an account or fall back to default. Returns name + ref.
    pub fn resolve_account(&self, name: Option<&str>) -> Option<&AccountConfig> {
        match name {
            Some(n) if !n.is_empty() => self.account(n),
            _ => self.default_account(),
        }
    }
}
