// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

use std::path::PathBuf;

/// Runtime configuration for Moon Himalia.
///
/// Everything is read from environment variables (populated from `.mcp.json`
/// by the JupiterOS host), mirroring the convention used by the other Moons.
/// `DATA_DIR` defaults to `../data` resolved against the process working
/// directory, which the host sets to the Moon's own folder (`moon-himalia-rs`).
#[derive(Clone, Debug)]
pub struct Config {
    /// Tavily API key. Empty = web tools are disabled and return a clear error
    /// (the deep-research skill then falls back to native WebSearch/WebFetch).
    pub tavily_api_key: String,
    /// Directory for the rolling log file.
    pub data_dir: PathBuf,
    /// MCP transport: "sse" (default for JupiterOS) or "stdio".
    pub mcp_transport: String,
    pub mcp_host: String,
    pub mcp_port: u16,
}

impl Config {
    pub fn from_env() -> Self {
        let tavily_api_key = std::env::var("TAVILY_API_KEY").unwrap_or_default();
        let data_dir =
            PathBuf::from(std::env::var("DATA_DIR").unwrap_or_else(|_| "../data".into()));
        let mcp_transport =
            std::env::var("MCP_TRANSPORT").unwrap_or_else(|_| "sse".into());
        let mcp_host = std::env::var("MCP_HOST").unwrap_or_else(|_| "127.0.0.1".into());
        let mcp_port: u16 = std::env::var("MCP_PORT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(8700);

        Self {
            tavily_api_key,
            data_dir,
            mcp_transport,
            mcp_host,
            mcp_port,
        }
    }

    /// True when a non-empty Tavily key is configured.
    pub fn has_api_key(&self) -> bool {
        !self.tavily_api_key.trim().is_empty()
    }
}
