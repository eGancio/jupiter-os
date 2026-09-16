// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

//! The MCP surface of Moon Elara — external high-signal sources.
//!
//! Two tools for the market/idea-validation lens of the deep-research skill:
//! - `news_search`   → GDELT (free): real-time news, what's moving.
//! - `reddit_search` → Reddit (OAuth): community pain/sentiment.
//!
//! Live querying only. Cite the URL. The skill distils into an insight — the raw
//! firehose must NEVER be ingested into Metis.

use std::future::Future;

use rmcp::handler::server::tool::{Parameters, ToolRouter};
use rmcp::model::*;
use rmcp::{tool, tool_handler, tool_router, ServerHandler};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::config::Config;
use crate::gdelt::GdeltClient;
use crate::reddit::RedditClient;

// ===========================================================================
// Server struct
// ===========================================================================

/// Moon Elara MCP Server — news (GDELT) + community (Reddit).
pub struct ElaraServer {
    config: Config,
    gdelt: GdeltClient,
    reddit: RedditClient,
    tool_router: ToolRouter<Self>,
}

// ===========================================================================
// Parameter structs — doc comments become JSON schema descriptions for the LLM
// ===========================================================================

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct NewsSearchParams {
    /// The news query (keywords or a phrase).
    query: String,
    /// Time window: "24h", "7d", "1m" (default), "3m".
    #[serde(default)]
    timespan: Option<String>,
    /// Max articles (default 10, clamped 1..=50).
    #[serde(default)]
    max_results: Option<u32>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct RedditSearchParams {
    /// The search query.
    query: String,
    /// Restrict to a single subreddit (without the "r/"), optional.
    #[serde(default)]
    subreddit: Option<String>,
    /// Sort: "relevance" (default), "hot", "top", "new", "comments".
    #[serde(default)]
    sort: Option<String>,
    /// Time window: "hour", "day", "week", "month", "year" (default), "all".
    #[serde(default)]
    time: Option<String>,
    /// Max posts (default 10, clamped 1..=25).
    #[serde(default)]
    max_results: Option<u32>,
}

// ===========================================================================
// Tools
// ===========================================================================

#[tool_router]
impl ElaraServer {
    pub fn new(config: Config) -> Self {
        let gdelt = GdeltClient::new(config.reddit_user_agent.clone());
        let reddit = RedditClient::new(
            config.reddit_client_id.clone(),
            config.reddit_client_secret.clone(),
            config.reddit_user_agent.clone(),
        );
        Self {
            config,
            gdelt,
            reddit,
            tool_router: Self::tool_router(),
        }
    }

    /// Real-time news search via GDELT (no API key).
    #[tool(
        name = "news_search",
        description = "Cerca NOTIZIE in tempo reale via GDELT (gratis, senza key). Usala per 'cosa si muove ora' su un mercato/tema (validazione idee, timing, mosse competitor). Ritorna titolo+url+dominio+data. Cita SEMPRE l'URL. NON versare il grezzo in Metis: distilla l'insight."
    )]
    pub async fn news_search(
        &self,
        params: Parameters<NewsSearchParams>,
    ) -> Result<String, String> {
        let p = params.0;
        let timespan = p.timespan.as_deref().unwrap_or("1m");
        let max_results = p.max_results.unwrap_or(10).clamp(1, 50) as usize;

        let hits = self.gdelt.search(&p.query, timespan, max_results).await?;
        Ok(json!({
            "query": p.query,
            "timespan": timespan,
            "results": hits,
            "note": "Cita l'URL. Distilla l'insight: NON ingerire articoli grezzi in Metis.",
        })
        .to_string())
    }

    /// Community signal via Reddit search (OAuth app-only).
    #[tool(
        name = "reddit_search",
        description = "Cerca su REDDIT (API ufficiale) il segnale di community: dolore espresso, lamentele su soluzioni esistenti, sentiment. Chiave per la validazione idee. Ritorna titolo, url, permalink (da citare), subreddit, score, n. commenti, estratto. Cita l'URL/permalink. NON versare il grezzo in Metis: distilla. Salva insight aggregati/anonimi (GDPR), non post personali."
    )]
    pub async fn reddit_search(
        &self,
        params: Parameters<RedditSearchParams>,
    ) -> Result<String, String> {
        if !self.config.has_reddit_creds() {
            return Err("Reddit non configurato: imposta REDDIT_CLIENT_ID e \
                        REDDIT_CLIENT_SECRET nel .mcp.json del Moon elara (app 'script' su \
                        reddit.com/prefs/apps). Nel frattempo usa news_search/web."
                .to_string());
        }
        let p = params.0;
        let sort = p.sort.as_deref().unwrap_or("relevance");
        let time = p.time.as_deref().unwrap_or("year");
        let max_results = p.max_results.unwrap_or(10).clamp(1, 25) as usize;

        let hits = self
            .reddit
            .search(&p.query, p.subreddit.as_deref(), sort, time, max_results)
            .await?;
        Ok(json!({
            "query": p.query,
            "subreddit": p.subreddit,
            "results": hits,
            "note": "Cita il permalink. Distilla in insight aggregati/anonimi (GDPR): NON salvare post personali grezzi in Metis.",
        })
        .to_string())
    }
}

// ===========================================================================
// ServerHandler implementation — MCP protocol
// ===========================================================================

#[tool_handler]
impl ServerHandler for ElaraServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::default(),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation {
                name: "moon-elara".to_string(),
                version: "0.1.0".to_string(),
            },
            instructions: Some(
                "Moon Elara — segnale esterno per market/validazione idee. news_search \
                 (GDELT, gratis) per le notizie in tempo reale; reddit_search (OAuth) per \
                 dolore/sentiment di community. Cita SEMPRE l'URL/permalink. Interroga LIVE \
                 e DISTILLA: non ingerire mai il grezzo in Metis; salva solo insight \
                 aggregati/anonimi. Senza credenziali Reddit, reddit_search torna errore e \
                 si usano news/web."
                    .into(),
            ),
        }
    }
}
