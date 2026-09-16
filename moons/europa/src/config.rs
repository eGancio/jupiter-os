// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

use std::path::Path;
use std::path::PathBuf;

/// True only if the Baileys `creds.json` is a USABLE companion session, not just
/// present on disk. A SIGKILL mid-write (or an aborted pairing) can leave a 0-byte
/// or half-written `creds.json`; that file still "exists" but linking with it
/// always fails, so treating it as "configured" makes WhatsApp retry a dead
/// session forever. We require: file present, non-empty, valid JSON, and a linked
/// identity (`me.id` set). NOTE: do NOT gate on `creds.registered` — Baileys only
/// sets that for the pairing-CODE flow, never for QR linking, so a perfectly good
/// QR companion session has `registered: false` forever.
pub fn whatsapp_session_valid(session_dir: &Path) -> bool {
    let creds = session_dir.join("creds.json");
    match std::fs::read_to_string(&creds) {
        Ok(s) if !s.trim().is_empty() => serde_json::from_str::<serde_json::Value>(&s)
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

/// Moon Europa configuration, loaded from environment variables.
#[derive(Debug, Clone)]
pub struct Config {
    // Telegram credentials (user account, MTProto via grammers)
    pub telegram_api_id: i32,
    pub telegram_api_hash: String,
    pub telegram_folder: Option<String>,
    pub telegram_chats: Vec<String>,
    /// Telegram Bot API token (from @BotFather). If set, the `telegram` channel
    /// is served by the lightweight HTTP Bot adapter instead of the user account.
    pub telegram_bot_token: Option<String>,

    // Slack (Web API, bot token "xoxb-...")
    pub slack_bot_token: Option<String>,

    // Microsoft Teams (Graph API via OAuth2)
    pub teams_client_id: Option<String>,
    pub teams_tenant_id: Option<String>,
    /// OAuth2 refresh token for Teams (normally injected from the keyring by the GUI).
    pub teams_refresh_token: Option<String>,

    // WhatsApp (personal account via the Baileys Node helper)
    /// True when the WhatsApp channel should be started (env WHATSAPP_ENABLED or
    /// an existing Baileys session on disk).
    pub whatsapp_enabled: bool,
    /// Baileys multi-file auth state dir (`<DATA_DIR>/whatsapp`).
    pub whatsapp_session_path: PathBuf,
    /// Absolute path to the helper script (`whatsapp-helper/helper.mjs`).
    pub whatsapp_helper_path: PathBuf,
    /// Node binary to run the helper with (defaults to "node" on PATH).
    pub whatsapp_node: String,

    // Storage
    pub data_dir: PathBuf,
    pub session_path: PathBuf,
    pub onnx_model_dir: PathBuf,

    // Server
    pub mcp_host: String,
    pub mcp_port: u16,
    pub mcp_transport: String,

    // Qdrant
    pub qdrant_url: String,

    // Indexer
    pub reconnect_delay_secs: u64,
    pub batch_size: usize,
}

impl Config {
    /// Load configuration from environment variables.
    pub fn from_env() -> Result<Self, String> {
        let data_dir = PathBuf::from(
            std::env::var("DATA_DIR")
                .or_else(|_| std::env::var("CHROMA_PERSIST_DIR")) // legacy compat
                .unwrap_or_else(|_| "./data".into()),
        );

        let session_path = data_dir.join("telegram");

        let telegram_chats_str =
            std::env::var("TELEGRAM_CHATS").unwrap_or_default();
        let telegram_chats: Vec<String> = telegram_chats_str
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        let telegram_folder = std::env::var("TELEGRAM_FOLDER").ok().filter(|s| !s.is_empty());

        let api_id_str = std::env::var("TELEGRAM_API_ID").unwrap_or_default();
        let telegram_api_id: i32 = api_id_str.parse().unwrap_or(0);

        let onnx_model_dir = data_dir.join("model");

        let opt_env = |key: &str| std::env::var(key).ok().filter(|s| !s.is_empty());

        // WhatsApp: enabled via env flag or an already-paired session on disk.
        let whatsapp_session_path = data_dir.join("whatsapp");
        let whatsapp_enabled = std::env::var("WHATSAPP_ENABLED")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false)
            || whatsapp_session_valid(&whatsapp_session_path);
        // Helper path: explicit env, else the repo layout (moons/europa/whatsapp-helper)
        // sits next to the data dir's parent.
        let whatsapp_helper_path = opt_env("WHATSAPP_HELPER")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                data_dir
                    .parent()
                    .unwrap_or(&data_dir)
                    .join("whatsapp-helper")
                    .join("helper.mjs")
            });
        let whatsapp_node = std::env::var("WHATSAPP_NODE").unwrap_or_else(|_| "node".into());

        Ok(Self {
            telegram_api_id,
            telegram_api_hash: std::env::var("TELEGRAM_API_HASH").unwrap_or_default(),
            telegram_folder,
            telegram_chats,
            telegram_bot_token: opt_env("TELEGRAM_BOT_TOKEN"),
            slack_bot_token: opt_env("SLACK_BOT_TOKEN"),
            teams_client_id: opt_env("TEAMS_CLIENT_ID"),
            teams_tenant_id: opt_env("TEAMS_TENANT_ID"),
            teams_refresh_token: opt_env("TEAMS_REFRESH_TOKEN"),
            whatsapp_enabled,
            whatsapp_session_path,
            whatsapp_helper_path,
            whatsapp_node,
            data_dir: data_dir.clone(),
            session_path,
            onnx_model_dir,
            mcp_host: std::env::var("MCP_HOST").unwrap_or_else(|_| "127.0.0.1".into()),
            mcp_port: std::env::var("MCP_PORT")
                .unwrap_or_else(|_| "8200".into())
                .parse()
                .unwrap_or(8200),
            mcp_transport: std::env::var("MCP_TRANSPORT").unwrap_or_else(|_| "stdio".into()),
            qdrant_url: std::env::var("QDRANT_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:6334".into()),
            reconnect_delay_secs: 30,
            batch_size: 200,
        })
    }

    /// Check if the Telegram user account is configured (has API credentials).
    pub fn has_telegram(&self) -> bool {
        self.telegram_api_id != 0 && !self.telegram_api_hash.is_empty()
    }

    /// Check if a Telegram bot token is configured.
    pub fn has_telegram_bot(&self) -> bool {
        self.telegram_bot_token.as_deref().map(|t| !t.is_empty()).unwrap_or(false)
    }

    /// True when the `telegram` channel should be served by the Bot adapter
    /// (a bot token takes precedence over the user account).
    pub fn telegram_is_bot(&self) -> bool {
        self.has_telegram_bot()
    }

    /// Check if Slack is configured (bot token present).
    pub fn has_slack(&self) -> bool {
        self.slack_bot_token.as_deref().map(|t| !t.is_empty()).unwrap_or(false)
    }

    /// Check if Teams is configured (OAuth refresh token present).
    pub fn has_teams(&self) -> bool {
        self.teams_refresh_token.as_deref().map(|t| !t.is_empty()).unwrap_or(false)
            && self.teams_client_id.is_some()
    }

    /// Check if WhatsApp should be started (flag set or session present).
    pub fn has_whatsapp(&self) -> bool {
        self.whatsapp_enabled
    }
}
