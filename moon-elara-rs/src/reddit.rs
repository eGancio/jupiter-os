// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

//! Client for the official Reddit API (https://www.reddit.com/dev/api).
//!
//! App-only ("userless") OAuth: we exchange client_id + client_secret for a
//! ~1h bearer token (cached in memory, refreshed on demand), then call the
//! authenticated search endpoint. Reddit REQUIRES a unique User-Agent.
//!
//! Mirrors the reqwest + retry/backoff pattern of `moon-io-rs/src/gmail_api.rs`
//! and the cached-token idea of `moon-io-rs/src/oauth.rs`. Rate: ~100 QPM.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tracing::warn;

const TOKEN_URL: &str = "https://www.reddit.com/api/v1/access_token";
const API_BASE: &str = "https://oauth.reddit.com";
const MAX_ATTEMPTS: usize = 3;

/// A single Reddit post (link or self) from a search.
#[derive(Debug, Clone, Serialize)]
pub struct RedditHit {
    pub title: String,
    /// The post's link target (external URL for link posts, thread URL for self).
    pub url: String,
    /// Full URL of the Reddit discussion (use this to cite).
    pub permalink: String,
    pub subreddit: String,
    pub score: i64,
    pub num_comments: i64,
    /// Creation time, Unix epoch seconds.
    pub created_utc: i64,
    /// Self-text excerpt (truncated), empty for link posts.
    pub excerpt: String,
}

// --- Reddit API response shapes (only the fields we use) ---

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    expires_in: u64,
}

#[derive(Deserialize)]
struct Listing {
    data: ListingData,
}
#[derive(Deserialize)]
struct ListingData {
    #[serde(default)]
    children: Vec<Child>,
}
#[derive(Deserialize)]
struct Child {
    #[serde(default)]
    data: ChildData,
}
#[derive(Default, Deserialize)]
struct ChildData {
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    permalink: String,
    #[serde(default)]
    subreddit: String,
    #[serde(default)]
    score: i64,
    #[serde(default)]
    num_comments: i64,
    #[serde(default)]
    created_utc: f64,
    #[serde(default)]
    selftext: String,
}

#[derive(Clone)]
struct CachedToken {
    value: String,
    expires_at: Instant,
}

/// Reddit HTTP client with an in-memory cached app-only token.
#[derive(Clone)]
pub struct RedditClient {
    http: reqwest::Client,
    client_id: String,
    client_secret: String,
    user_agent: String,
    token: Arc<Mutex<Option<CachedToken>>>,
}

impl RedditClient {
    pub fn new(
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
        user_agent: impl Into<String>,
    ) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            http,
            client_id: client_id.into(),
            client_secret: client_secret.into(),
            user_agent: user_agent.into(),
            token: Arc::new(Mutex::new(None)),
        }
    }

    /// Return a valid bearer token, refreshing if absent or near expiry.
    /// `force` skips the cache (used after a 401).
    async fn access_token(&self, force: bool) -> Result<String, String> {
        {
            let guard = self.token.lock().await;
            if !force {
                if let Some(tok) = guard.as_ref() {
                    if tok.expires_at > Instant::now() + Duration::from_secs(60) {
                        return Ok(tok.value.clone());
                    }
                }
            }
        }

        let resp = self
            .http
            .post(TOKEN_URL)
            .basic_auth(&self.client_id, Some(&self.client_secret))
            .header("User-Agent", &self.user_agent)
            .form(&[("grant_type", "client_credentials")])
            .send()
            .await
            .map_err(|e| format!("Richiesta token Reddit fallita: {e}"))?;

        let status = resp.status();
        if !status.is_success() {
            let detail = resp.text().await.unwrap_or_default();
            return Err(format!(
                "Auth Reddit {status}: {} (controlla REDDIT_CLIENT_ID/SECRET)",
                detail.chars().take(160).collect::<String>()
            ));
        }

        let tr: TokenResponse = resp
            .json()
            .await
            .map_err(|e| format!("Token Reddit non valido: {e}"))?;
        let expires_at = Instant::now() + Duration::from_secs(tr.expires_in.max(60));
        let mut guard = self.token.lock().await;
        *guard = Some(CachedToken {
            value: tr.access_token.clone(),
            expires_at,
        });
        Ok(tr.access_token)
    }

    /// Search Reddit. `subreddit` restricts to one sub; `sort` is
    /// relevance|hot|top|new|comments; `time` is hour|day|week|month|year|all.
    pub async fn search(
        &self,
        query: &str,
        subreddit: Option<&str>,
        sort: &str,
        time: &str,
        limit: usize,
    ) -> Result<Vec<RedditHit>, String> {
        let limit_s = limit.clamp(1, 25).to_string();
        let path = match subreddit {
            Some(sr) if !sr.trim().is_empty() => format!("/r/{}/search", sr.trim()),
            _ => "/search".to_string(),
        };
        let url = format!("{API_BASE}{path}");

        let mut params: Vec<(&str, &str)> = vec![
            ("q", query),
            ("limit", &limit_s),
            ("sort", sort),
            ("t", time),
            ("type", "link"),
            ("raw_json", "1"),
        ];
        if subreddit.map(|s| !s.trim().is_empty()).unwrap_or(false) {
            params.push(("restrict_sr", "1"));
        }

        let mut forced = false;
        let mut last_err = String::new();
        for attempt in 0..MAX_ATTEMPTS {
            let token = self.access_token(forced).await?;
            let resp = self
                .http
                .get(&url)
                .bearer_auth(&token)
                .header("User-Agent", &self.user_agent)
                .query(&params)
                .send()
                .await
                .map_err(|e| format!("Richiesta a Reddit fallita: {e}"))?;

            let status = resp.status();
            if status.is_success() {
                let listing: Listing = resp
                    .json()
                    .await
                    .map_err(|e| format!("Risposta Reddit non valida: {e}"))?;
                return Ok(listing
                    .data
                    .children
                    .into_iter()
                    .map(|c| {
                        let d = c.data;
                        RedditHit {
                            title: d.title,
                            url: d.url,
                            permalink: if d.permalink.is_empty() {
                                String::new()
                            } else {
                                format!("https://www.reddit.com{}", d.permalink)
                            },
                            subreddit: d.subreddit,
                            score: d.score,
                            num_comments: d.num_comments,
                            created_utc: d.created_utc as i64,
                            excerpt: d.selftext.chars().take(280).collect(),
                        }
                    })
                    .collect());
            }

            // Token expired/invalid → refresh once and retry.
            if status == reqwest::StatusCode::UNAUTHORIZED && !forced {
                forced = true;
                continue;
            }

            if status == reqwest::StatusCode::TOO_MANY_REQUESTS
                || status == reqwest::StatusCode::SERVICE_UNAVAILABLE
            {
                let detail = resp.text().await.unwrap_or_default();
                last_err = format!("Reddit {status}: {detail}");
                let backoff_ms = 500 * (attempt as u64 + 1) * 3;
                warn!("Reddit {status}, retry tra {backoff_ms}ms (tentativo {})", attempt + 1);
                tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
                continue;
            }

            let detail = resp.text().await.unwrap_or_default();
            return Err(format!("Reddit {status}: {}", detail.chars().take(160).collect::<String>()));
        }
        Err(if last_err.is_empty() {
            "Reddit: troppi tentativi falliti".to_string()
        } else {
            last_err
        })
    }
}
