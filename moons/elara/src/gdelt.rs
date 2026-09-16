// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

//! Client for the GDELT DOC 2.0 API (https://api.gdeltproject.org/api/v2/doc/doc).
//!
//! Free, no API key — real-time global news. We request `mode=ArtList&format=json`
//! sorted by date. GDELT can answer with HTML or an empty body on a malformed
//! query, so parsing is tolerant: a non-JSON body becomes a clean error, never a
//! panic.

use std::time::Duration;

use serde::{Deserialize, Serialize};

const DOC_URL: &str = "https://api.gdeltproject.org/api/v2/doc/doc";
/// GDELT throttles to ~1 request / 5s and answers 429 with a plain-text notice.
const MAX_ATTEMPTS: usize = 3;
const RATELIMIT_WAIT: Duration = Duration::from_millis(5200);

/// A single news article from GDELT.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewsHit {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub domain: String,
    /// GDELT timestamp, e.g. "20260609T101500Z".
    #[serde(default)]
    pub seendate: String,
    #[serde(default)]
    pub language: String,
}

#[derive(Debug, Deserialize)]
struct DocResponse {
    #[serde(default)]
    articles: Vec<NewsHit>,
}

/// GDELT HTTP client. Cheap to clone.
#[derive(Clone)]
pub struct GdeltClient {
    http: reqwest::Client,
    user_agent: String,
}

impl GdeltClient {
    pub fn new(user_agent: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            http,
            user_agent: user_agent.into(),
        }
    }

    /// News search. `timespan` is a GDELT window like "24h", "7d", "1m", "3m".
    pub async fn search(
        &self,
        query: &str,
        timespan: &str,
        max_results: usize,
    ) -> Result<Vec<NewsHit>, String> {
        let maxrecords = max_results.clamp(1, 250).to_string();

        let mut last_err = String::new();
        for attempt in 0..MAX_ATTEMPTS {
            let resp = self
                .http
                .get(DOC_URL)
                .header("User-Agent", &self.user_agent)
                .query(&[
                    ("query", query),
                    ("mode", "ArtList"),
                    ("format", "json"),
                    ("sort", "DateDesc"),
                    ("maxrecords", &maxrecords),
                    ("timespan", timespan),
                ])
                .send()
                .await
                .map_err(|e| format!("Richiesta a GDELT fallita: {e}"))?;

            let status = resp.status();
            let body = resp
                .text()
                .await
                .map_err(|e| format!("Lettura risposta GDELT: {e}"))?;
            let trimmed = body.trim();

            // GDELT throttles to 1 req / 5s, answering 429 (or a plain-text notice).
            let rate_limited = status == reqwest::StatusCode::TOO_MANY_REQUESTS
                || trimmed.starts_with("Please limit requests");
            if rate_limited {
                last_err = format!("GDELT rate-limit: {}", trimmed.chars().take(160).collect::<String>());
                if attempt + 1 < MAX_ATTEMPTS {
                    tokio::time::sleep(RATELIMIT_WAIT).await;
                    continue;
                }
                return Err(last_err);
            }

            if !status.is_success() {
                return Err(format!("GDELT {status}: {}", trimmed.chars().take(200).collect::<String>()));
            }

            // Empty body = no matches.
            if trimmed.is_empty() {
                return Ok(Vec::new());
            }

            // Tolerant parse: a non-JSON body (HTML error page) → clean error.
            return match serde_json::from_str::<DocResponse>(trimmed) {
                Ok(doc) => Ok(doc.articles),
                Err(_) => Err(format!(
                    "GDELT ha risposto in formato non-JSON (query troppo ampia/malformata?): {}",
                    trimmed.chars().take(160).collect::<String>()
                )),
            };
        }
        Err(last_err)
    }
}
