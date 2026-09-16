// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

//! Thin client for the Tavily API (https://docs.tavily.com).
//!
//! Two endpoints are used:
//! - `POST /search`  → discovery: clean results (title, url, content, score).
//! - `POST /extract` → fetch a page's full clean text (the VERIFY step).
//!
//! Mirrors the reqwest + retry/backoff pattern of `moons/io/src/gmail_api.rs`:
//! a shared `Client` with a timeout, and up to 3 attempts on 429/503.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tracing::warn;

const SEARCH_URL: &str = "https://api.tavily.com/search";
const EXTRACT_URL: &str = "https://api.tavily.com/extract";
const MAX_ATTEMPTS: usize = 3;

/// A single web result from `/search`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub url: String,
    /// Clean, snippet-length content extracted by Tavily (not raw HTML).
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub score: f64,
}

#[derive(Debug, Deserialize)]
struct SearchResponse {
    #[serde(default)]
    results: Vec<SearchHit>,
}

#[derive(Debug, Deserialize)]
struct ExtractItem {
    #[serde(default)]
    url: String,
    #[serde(default)]
    raw_content: String,
}

#[derive(Debug, Deserialize)]
struct ExtractResponse {
    #[serde(default)]
    results: Vec<ExtractItem>,
}

/// Tavily HTTP client. Cheap to clone (wraps an `Arc` inside `reqwest::Client`).
#[derive(Clone)]
pub struct TavilyClient {
    http: reqwest::Client,
    api_key: String,
}

impl TavilyClient {
    pub fn new(api_key: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            http,
            api_key: api_key.into(),
        }
    }

    /// Web search. `depth` is "basic" (fast) or "advanced" (deeper crawl).
    pub async fn search(
        &self,
        query: &str,
        max_results: usize,
        depth: &str,
    ) -> Result<Vec<SearchHit>, String> {
        let body = serde_json::json!({
            "query": query,
            "search_depth": depth,
            "max_results": max_results,
            "include_answer": false,
            "include_raw_content": false,
        });
        let resp: SearchResponse = self.post_json(SEARCH_URL, &body).await?;
        Ok(resp.results)
    }

    /// Fetch a single URL's clean full text.
    pub async fn extract(&self, url: &str) -> Result<(String, String), String> {
        let body = serde_json::json!({
            "urls": [url],
            "extract_depth": "basic",
        });
        let resp: ExtractResponse = self.post_json(EXTRACT_URL, &body).await?;
        match resp.results.into_iter().next() {
            Some(item) => Ok((item.url, item.raw_content)),
            None => Err(format!("Tavily non ha estratto contenuto da {url}")),
        }
    }

    /// POST JSON with Bearer auth, parse the typed response, retry on 429/503.
    async fn post_json<T: for<'de> Deserialize<'de>>(
        &self,
        endpoint: &str,
        body: &serde_json::Value,
    ) -> Result<T, String> {
        let mut last_err = String::new();
        for attempt in 0..MAX_ATTEMPTS {
            let resp = self
                .http
                .post(endpoint)
                .bearer_auth(&self.api_key)
                .json(body)
                .send()
                .await;

            let resp = match resp {
                Ok(r) => r,
                Err(e) => {
                    last_err = format!("Richiesta a Tavily fallita: {e}");
                    break; // network/transport error — don't hammer
                }
            };

            let status = resp.status();
            if status.is_success() {
                return resp
                    .json::<T>()
                    .await
                    .map_err(|e| format!("Risposta Tavily non valida: {e}"));
            }

            // Retry only on rate-limit / transient unavailability.
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS
                || status == reqwest::StatusCode::SERVICE_UNAVAILABLE
            {
                let detail = resp.text().await.unwrap_or_default();
                last_err = format!("Tavily {status}: {detail}");
                let backoff_ms = 500 * (attempt as u64 + 1) * 3; // 500ms, 1.5s, 4.5s
                warn!("Tavily {status}, retry tra {backoff_ms}ms (tentativo {})", attempt + 1);
                tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
                continue;
            }

            // Other errors (401 bad key, 400 bad request, …): fail fast.
            let detail = resp.text().await.unwrap_or_default();
            return Err(format!("Tavily {status}: {detail}"));
        }
        Err(last_err)
    }
}
