// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

use std::path::PathBuf;

/// Runtime configuration for Moon Ganymede.
///
/// Everything is read from environment variables (populated from `.mcp.json`
/// by the JupiterOS host), mirroring the convention used by the other Moons:
/// relative defaults are resolved against the process working directory, which
/// the host sets to the Moon's own folder (e.g. `moons/ganymede`). With that
/// cwd, the `../../wiki` default lands at `<repo>/wiki`, a sibling of the
/// `../../data` directory used by the other Moons.
#[derive(Clone, Debug)]
pub struct Config {
    /// Root directory holding the Wiki `.md` files, taxonomy and indices.
    pub wiki_dir: PathBuf,
    /// Directory for the rolling log file.
    pub data_dir: PathBuf,
    /// MCP transport: "sse" (default for JupiterOS) or "stdio".
    pub mcp_transport: String,
    pub mcp_host: String,
    pub mcp_port: u16,
}

impl Config {
    pub fn from_env() -> Self {
        let wiki_dir = PathBuf::from(
            std::env::var("GANYMEDE_WIKI_DIR").unwrap_or_else(|_| "../../wiki".into()),
        );
        let data_dir =
            PathBuf::from(std::env::var("DATA_DIR").unwrap_or_else(|_| "../../data".into()));
        let mcp_transport =
            std::env::var("MCP_TRANSPORT").unwrap_or_else(|_| "sse".into());
        let mcp_host = std::env::var("MCP_HOST").unwrap_or_else(|_| "127.0.0.1".into());
        let mcp_port: u16 = std::env::var("MCP_PORT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(8400);

        Self {
            wiki_dir,
            data_dir,
            mcp_transport,
            mcp_host,
            mcp_port,
        }
    }

    // --- Well-known paths inside the wiki ---

    pub fn taxonomy_path(&self) -> PathBuf {
        self.wiki_dir.join("_taxonomy.yaml")
    }

    pub fn template_path(&self) -> PathBuf {
        self.wiki_dir.join("_template.md")
    }

    pub fn index_dir(&self) -> PathBuf {
        self.wiki_dir.join("_index")
    }

    pub fn history_dir(&self) -> PathBuf {
        self.wiki_dir.join(".history")
    }
}
