// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

use std::path::PathBuf;

/// Runtime configuration for Moon Elara.
///
/// Everything is read from environment variables (populated from `.mcp.json` by
/// the JupiterOS host), mirroring the other Moons. GDELT needs no credentials;
/// Reddit needs an app (client id + secret) — when absent, `reddit_search`
/// returns a clear error and the skill falls back to news/web only.
#[derive(Clone, Debug)]
pub struct Config {
    /// Reddit app client id (empty = reddit_search disabled).
    pub reddit_client_id: String,
    /// Reddit app client secret.
    pub reddit_client_secret: String,
    /// User-Agent sent to Reddit/GDELT (Reddit REQUIRES a unique one).
    pub reddit_user_agent: String,
    /// Directory for the rolling log file.
    pub data_dir: PathBuf,
    /// MCP transport: "sse" (default for JupiterOS) or "stdio".
    pub mcp_transport: String,
    pub mcp_host: String,
    pub mcp_port: u16,
}

impl Config {
    pub fn from_env() -> Self {
        let reddit_client_id = std::env::var("REDDIT_CLIENT_ID").unwrap_or_default();
        let reddit_client_secret = std::env::var("REDDIT_CLIENT_SECRET").unwrap_or_default();
        let reddit_user_agent = std::env::var("REDDIT_USER_AGENT")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "jupiteros-elara/0.1".into());
        let data_dir =
            PathBuf::from(std::env::var("DATA_DIR").unwrap_or_else(|_| "../../data".into()));
        let mcp_transport =
            std::env::var("MCP_TRANSPORT").unwrap_or_else(|_| "sse".into());
        let mcp_host = std::env::var("MCP_HOST").unwrap_or_else(|_| "127.0.0.1".into());
        let mcp_port: u16 = std::env::var("MCP_PORT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(8800);

        Self {
            reddit_client_id,
            reddit_client_secret,
            reddit_user_agent,
            data_dir,
            mcp_transport,
            mcp_host,
            mcp_port,
        }
    }

    /// True when both Reddit credentials are configured.
    pub fn has_reddit_creds(&self) -> bool {
        !self.reddit_client_id.trim().is_empty() && !self.reddit_client_secret.trim().is_empty()
    }
}
