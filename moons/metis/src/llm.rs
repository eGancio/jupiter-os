// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

//! Thin OpenAI-compatible chat client.
//!
//! Metis does NOT own model selection: it inherits whatever engine Jupiter is
//! configured with, via env injected at spawn (`METIS_LLM_BASE_URL` /
//! `_MODEL` / `_API_KEY`). The same OpenAI-compatible interface targets a local
//! Ollama (`http://localhost:11434/v1`), Gemini's OpenAI endpoint, OpenRouter,
//! etc. It is used for **high-volume, low-stakes** enrichment (e.g. a nature
//! pre-hint in batch); the high-stakes ingest judgement stays with the agent.

use anyhow::{anyhow, Result};
use serde_json::json;

#[derive(Clone)]
pub struct LlmClient {
    http: reqwest::Client,
    base_url: String,
    model: String,
    api_key: String,
}

impl LlmClient {
    pub fn new(base_url: &str, model: &str, api_key: &str) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            model: model.to_string(),
            api_key: api_key.to_string(),
        }
    }

    /// Single-turn chat completion. Returns the assistant message text.
    pub async fn chat(&self, system: &str, user: &str) -> Result<String> {
        if self.base_url.is_empty() {
            return Err(anyhow!(
                "LLM non configurato (METIS_LLM_BASE_URL vuoto)"
            ));
        }
        let url = format!("{}/chat/completions", self.base_url);
        let body = json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": user},
            ],
            "temperature": 0.0,
            "stream": false,
        });

        let mut req = self.http.post(&url).json(&body);
        if !self.api_key.is_empty() {
            req = req.bearer_auth(&self.api_key);
        }

        let resp = req
            .send()
            .await
            .map_err(|e| anyhow!("LLM request: {e}"))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let txt = resp.text().await.unwrap_or_default();
            return Err(anyhow!("LLM HTTP {status}: {txt}"));
        }

        let v: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| anyhow!("LLM decode: {e}"))?;
        let content = v["choices"][0]["message"]["content"]
            .as_str()
            .ok_or_else(|| anyhow!("LLM: risposta senza content"))?;
        Ok(content.trim().to_string())
    }

    /// Guess a document nature from preview text. Returns one of the known
    /// natures (defaults to "generico" on any uncertainty).
    pub async fn classify_nature(&self, preview: &str) -> Result<String> {
        let system = "Sei un classificatore di documenti. Rispondi con UNA sola parola tra: \
            generico, norma, bando, preventivo. Nessun'altra parola.";
        let user = format!(
            "Classifica la NATURA di questo documento dalle prime pagine:\n\n{}",
            preview.chars().take(4000).collect::<String>()
        );
        let raw = self.chat(system, &user).await?;
        let nature = raw.to_lowercase();
        let picked = crate::procedure::KNOWN_NATURES
            .iter()
            .find(|n| nature.contains(*n))
            .map(|s| s.to_string())
            .unwrap_or_else(|| "generico".to_string());
        Ok(picked)
    }
}
