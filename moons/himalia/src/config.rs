// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

use std::path::PathBuf;

/// Runtime configuration for Moon Himalia.
///
/// Everything is read from environment variables (populated from `.mcp.json`
/// by the JupiterOS host), mirroring the convention used by the other Moons.
/// `DATA_DIR` defaults to `../../data` resolved against the process working
/// directory, which the host sets to the Moon's own folder (`moons/himalia`).
#[derive(Clone, Debug)]
pub struct Config {
    /// Tavily API key. Empty = web tools are disabled and return a clear error
    /// (the deep-research skill then falls back to native WebSearch/WebFetch).
    pub tavily_api_key: String,
    /// openapi.it token (prodotto "visure camerali"/bilancio). Vuoto = i tool
    /// imprese (bilancio/visura) tornano un errore chiaro, identico a come Tavily
    /// disabilita i tool web. La presenza della chiave È l'interruttore.
    pub openapi_token: String,
    /// Token opzionale per `company.openapi.com` (solo `cerca_impresa`: nome→P.IVA).
    pub openapi_company_token: String,
    /// Se true usa l'host sandbox `test.visurecamerali.openapi.it` (nessun addebito).
    pub openapi_sandbox: bool,
    /// Cartella dove salvare PDF/XBRL scaricati (default: `data_dir/imprese`).
    pub imprese_output_dir: PathBuf,
    /// Tetto di spesa mensile in euro per le chiamate a pagamento (guardrail costi).
    pub imprese_max_spend_eur: f64,
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
        let openapi_token = std::env::var("OPENAPI_TOKEN").unwrap_or_default();
        let openapi_company_token =
            std::env::var("OPENAPI_COMPANY_TOKEN").unwrap_or_default();
        let openapi_sandbox = matches!(
            std::env::var("OPENAPI_SANDBOX")
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase()
                .as_str(),
            "1" | "true" | "yes" | "on"
        );
        let imprese_max_spend_eur = std::env::var("IMPRESE_MAX_SPEND_EUR")
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(50.0);
        let data_dir =
            PathBuf::from(std::env::var("DATA_DIR").unwrap_or_else(|_| "../../data".into()));
        let imprese_output_dir = std::env::var("IMPRESE_OUTPUT_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| data_dir.join("imprese"));
        let mcp_transport =
            std::env::var("MCP_TRANSPORT").unwrap_or_else(|_| "sse".into());
        let mcp_host = std::env::var("MCP_HOST").unwrap_or_else(|_| "127.0.0.1".into());
        let mcp_port: u16 = std::env::var("MCP_PORT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(8700);

        Self {
            tavily_api_key,
            openapi_token,
            openapi_company_token,
            openapi_sandbox,
            imprese_output_dir,
            imprese_max_spend_eur,
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

    /// True when an openapi.it token is configured (i tool imprese sono attivi).
    pub fn has_openapi_key(&self) -> bool {
        !self.openapi_token.trim().is_empty()
    }

    /// True when the optional company-search token is configured.
    pub fn has_openapi_company_key(&self) -> bool {
        !self.openapi_company_token.trim().is_empty()
    }
}
