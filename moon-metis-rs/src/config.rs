// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

use std::path::PathBuf;

/// Runtime configuration for Moon Metis.
///
/// Everything is read from environment variables (populated from `.mcp.json`
/// by the JupiterOS host, which sets the process cwd to `moon-metis-rs`).
/// By default Metis **reuses Io's already-downloaded BGE-M3 model** and the
/// shared Qdrant instance, so it does not re-download anything.
#[derive(Clone, Debug)]
pub struct Config {
    /// Directory for the rolling log file and the ingest state file.
    pub data_dir: PathBuf,
    /// Directory containing `model.onnx` + `tokenizer.json` (BGE-M3).
    /// Defaults to Io's model dir so the ~570 MB model is shared, not duplicated.
    pub embedding_model_dir: PathBuf,
    /// Qdrant gRPC URL (shared instance, started by Moon Io).
    pub qdrant_url: String,
    /// Qdrant collection name for the knowledge base.
    pub collection_name: String,

    // --- LLM client (OpenAI-compatible). Inherits Jupiter's configured engine
    //     via env injected at spawn. Empty base_url => LLM features disabled. ---
    pub llm_base_url: String,
    pub llm_model: String,
    pub llm_api_key: String,

    // --- MCP transport ---
    pub mcp_transport: String,
    pub mcp_host: String,
    pub mcp_port: u16,
}

impl Config {
    pub fn from_env() -> Self {
        let data_dir =
            PathBuf::from(std::env::var("DATA_DIR").unwrap_or_else(|_| "data".into()));

        let embedding_model_dir = PathBuf::from(
            std::env::var("EMBEDDING_MODEL_DIR")
                .unwrap_or_else(|_| "../moon-io-rs/data/model".into()),
        );

        let qdrant_url =
            std::env::var("QDRANT_URL").unwrap_or_else(|_| "http://127.0.0.1:6334".into());
        let collection_name =
            std::env::var("METIS_COLLECTION").unwrap_or_else(|_| "metis_docs".into());

        let llm_base_url = std::env::var("METIS_LLM_BASE_URL").unwrap_or_default();
        let llm_model =
            std::env::var("METIS_LLM_MODEL").unwrap_or_else(|_| "qwen2.5:7b".into());
        let llm_api_key = std::env::var("METIS_LLM_API_KEY").unwrap_or_default();

        let mcp_transport = std::env::var("MCP_TRANSPORT").unwrap_or_else(|_| "sse".into());
        let mcp_host = std::env::var("MCP_HOST").unwrap_or_else(|_| "127.0.0.1".into());
        let mcp_port: u16 = std::env::var("MCP_PORT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(8600);

        Self {
            data_dir,
            embedding_model_dir,
            qdrant_url,
            collection_name,
            llm_base_url,
            llm_model,
            llm_api_key,
            mcp_transport,
            mcp_host,
            mcp_port,
        }
    }

    /// Path of the incremental ingest state file.
    pub fn state_path(&self) -> PathBuf {
        self.data_dir.join("metis_ingest_state.json")
    }

    /// Whether the LLM client is configured (base URL present).
    pub fn llm_enabled(&self) -> bool {
        !self.llm_base_url.is_empty()
    }
}
