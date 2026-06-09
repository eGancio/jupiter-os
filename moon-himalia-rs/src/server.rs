// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

//! The MCP surface of Moon Himalia — web research via Tavily.
//!
//! Two tools, mapped 1:1 onto the deep-research skill:
//! - `web_search`  → Step 2 (fan-out discovery): clean results, not raw HTML.
//! - `web_extract` → Step 3 (VERIFY): a page's full clean text to cross-check.
//!
//! Citing is the model's job (the skill): every web claim carries its URL.

use std::future::Future;

use rmcp::handler::server::tool::{Parameters, ToolRouter};
use rmcp::model::*;
use rmcp::{tool, tool_handler, tool_router, ServerHandler};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::config::Config;
use crate::tavily::TavilyClient;

// ===========================================================================
// Server struct
// ===========================================================================

/// Moon Himalia MCP Server — web search + extraction for LLM research.
pub struct HimaliaServer {
    config: Config,
    tavily: TavilyClient,
    tool_router: ToolRouter<Self>,
}

// ===========================================================================
// Parameter structs — doc comments become JSON schema descriptions for the LLM
// ===========================================================================

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct WebSearchParams {
    /// The search query. Be specific; vary phrasing across sub-questions.
    query: String,
    /// Max results to return (default 5, clamped 1..=10).
    #[serde(default)]
    max_results: Option<u32>,
    /// Depth: "basic" (fast, default) or "advanced" (deeper crawl, slower).
    #[serde(default)]
    depth: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct WebExtractParams {
    /// The exact URL of the page to fetch as clean full text (for verifying a
    /// claim against the primary source).
    url: String,
}

// ===========================================================================
// Tools
// ===========================================================================

#[tool_router]
impl HimaliaServer {
    pub fn new(config: Config) -> Self {
        let tavily = TavilyClient::new(config.tavily_api_key.clone());
        Self {
            config,
            tavily,
            tool_router: Self::tool_router(),
        }
    }

    /// Guard: a clear, non-panicking error when no key is configured, so the
    /// skill can fall back to native WebSearch/WebFetch.
    fn require_key(&self) -> Result<(), String> {
        if self.config.has_api_key() {
            Ok(())
        } else {
            Err("Tavily non configurato: imposta TAVILY_API_KEY nel .mcp.json del Moon \
                 himalia. Nel frattempo usa WebSearch/WebFetch nativi."
                .to_string())
        }
    }

    /// Web search returning clean, LLM-ready results (not raw HTML snippets).
    #[tool(
        name = "web_search",
        description = "Ricerca sul web con risultati PULITI per LLM (titolo, url, contenuto sintetico, score) via Tavily. Usala per la fase di scoperta della deep research (fan-out su sotto-domande). Cita SEMPRE l'URL nelle affermazioni. Per verificare un dato sulla fonte primaria usa poi web_extract."
    )]
    pub async fn web_search(
        &self,
        params: Parameters<WebSearchParams>,
    ) -> Result<String, String> {
        self.require_key()?;
        let p = params.0;
        let max_results = p.max_results.unwrap_or(5).clamp(1, 10) as usize;
        let depth = match p.depth.as_deref() {
            Some("advanced") => "advanced",
            _ => "basic",
        };

        let hits = self.tavily.search(&p.query, max_results, depth).await?;

        if hits.is_empty() {
            return Ok(json!({
                "query": p.query,
                "results": [],
                "note": "Nessun risultato. Riprova con altra formulazione o astieniti.",
            })
            .to_string());
        }

        Ok(json!({
            "query": p.query,
            "results": hits,
            "note": "Cita l'URL per ogni affermazione. Per il claim chiave verifica con web_extract sulla fonte più autorevole.",
        })
        .to_string())
    }

    /// Fetch a single URL's full clean text — the VERIFY step.
    #[tool(
        name = "web_extract",
        description = "Estrae il TESTO PULITO integrale di una pagina dato il suo URL (via Tavily). Usala nello step VERIFY della deep research per controllare un'affermazione sulla fonte primaria invece che sullo snippet. Cita l'URL."
    )]
    pub async fn web_extract(
        &self,
        params: Parameters<WebExtractParams>,
    ) -> Result<String, String> {
        self.require_key()?;
        let p = params.0;
        let (url, content) = self.tavily.extract(&p.url).await?;

        if content.trim().is_empty() {
            return Ok(json!({
                "url": url,
                "content": "",
                "note": "Pagina non estraibile (paywall/JS/blocco). Prova un'altra fonte o WebFetch nativo.",
            })
            .to_string());
        }

        Ok(json!({
            "url": url,
            "content": content,
            "note": "Testo della fonte primaria: verifica qui il dato prima di citarlo.",
        })
        .to_string())
    }
}

// ===========================================================================
// ServerHandler implementation — MCP protocol
// ===========================================================================

#[tool_handler]
impl ServerHandler for HimaliaServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::default(),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation {
                name: "moon-himalia".to_string(),
                version: "0.1.0".to_string(),
            },
            instructions: Some(
                "Moon Himalia — ricerca web per LLM (Tavily). web_search per scoprire fonti \
                 (fan-out della deep research); web_extract per leggere la fonte primaria in \
                 testo pulito e verificare un'affermazione. Cita SEMPRE l'URL. Se non è \
                 configurata la API key, i tool tornano un errore e si usa WebSearch/WebFetch \
                 nativi."
                    .into(),
            ),
        }
    }
}
