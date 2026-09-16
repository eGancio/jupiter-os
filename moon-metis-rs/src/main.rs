// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

use std::sync::Arc;

use rmcp::ServiceExt;
use tokio::sync::Mutex;
use tracing::{error, info};
use tracing_subscriber::prelude::*;
use tracing_subscriber::EnvFilter;

use jupiteros_shared::{DocStore, OnnxEmbedding};
use moon_metis_rs::config::Config;
use moon_metis_rs::ingest;
use moon_metis_rs::server::MoonMetisServer;
use moon_metis_rs::state::IngestState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();

    let config = Config::from_env();

    // Logging: stderr + rolling file (data_dir/moon-metis.log).
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,h2=warn,hyper=warn,rustls=warn"));
    std::fs::create_dir_all(&config.data_dir).ok();
    let file_appender = tracing_appender::rolling::never(&config.data_dir, "moon-metis.log");
    let (non_blocking_file, _guard) = tracing_appender::non_blocking(file_appender);
    tracing_subscriber::registry()
        .with(filter)
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_target(false)
                .with_writer(std::io::stderr),
        )
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_target(false)
                .with_writer(non_blocking_file),
        )
        .init();

    info!("Moon Metis starting...");
    info!("Model dir: {:?}", config.embedding_model_dir);
    info!("Qdrant: {} / collection {}", config.qdrant_url, config.collection_name);
    info!("LLM enabled: {} ({})", config.llm_enabled(), config.llm_model);
    info!(
        "MCP: {}:{} ({})",
        config.mcp_host, config.mcp_port, config.mcp_transport
    );

    // The `ort` crate loads the ONNX Runtime dylib dynamically. Io/Europa bundle
    // it under `data/lib/`; since we reuse Io's model dir, the dylib is at
    // `<model_dir>/../lib/`. Point ORT_DYLIB_PATH at the first one that exists.
    if std::env::var("ORT_DYLIB_PATH").is_err() {
        let mut lib_dirs: Vec<std::path::PathBuf> = vec![config.data_dir.join("lib")];
        if let Some(parent) = config.embedding_model_dir.parent() {
            lib_dirs.push(parent.join("lib"));
        }
        for dir in lib_dirs {
            // Platform-aware: .dylib on macOS, fallback .so (layout Linux).
            if let Some(c) = jupiteros_shared::find_ort_dylib(&dir) {
                info!("ORT dylib: {}", c.display());
                std::env::set_var("ORT_DYLIB_PATH", &c);
                break;
            }
        }
    }

    // The BGE-M3 model is shared with Moon Io (which downloads it). Don't
    // re-download here: fail with a clear, actionable message if it's missing.
    if !config.embedding_model_dir.join("model.onnx").exists() {
        error!(
            "Modello embedding non trovato in {}",
            config.embedding_model_dir.display()
        );
        return Err(anyhow::anyhow!(
            "model.onnx non trovato in {}. Avvia prima Moon Io (scarica BGE-M3) \
             oppure imposta EMBEDDING_MODEL_DIR su una cartella con model.onnx + tokenizer.json.",
            config.embedding_model_dir.display()
        ));
    }

    // Local OCR for scanned PDFs (best-effort): download the ~12 MB ocrs models
    // on first run and load the engine. Any failure is logged and OCR stays off
    // — scanned PDFs then fall back to the old "skip" behaviour; the Moon still
    // starts normally.
    if config.ocr_enabled {
        match moon_metis_rs::setup::ensure_ocr_models(&config.ocr_model_dir).await {
            Ok(()) => match moon_metis_rs::ocr::init(
                &config.ocr_model_dir,
                config.ocr_dpi,
                config.ocr_max_pages,
            ) {
                Ok(()) => info!(
                    "OCR: ready (modelli in {}, {} DPI, max {} pagine)",
                    config.ocr_model_dir.display(),
                    config.ocr_dpi,
                    config.ocr_max_pages
                ),
                Err(e) => error!("OCR init fallito, OCR disattivo: {e}"),
            },
            Err(e) => error!("Download modelli OCR fallito, OCR disattivo: {e}"),
        }
    } else {
        info!("OCR disabilitato (METIS_OCR_ENABLED=false)");
    }

    let embedding = Arc::new(OnnxEmbedding::load(&config.embedding_model_dir)?);
    let store = Arc::new(
        DocStore::new(&config.qdrant_url, embedding, &config.collection_name, None).await?,
    );
    let state = Arc::new(Mutex::new(IngestState::load(&config.state_path())));

    // CLI one-shot ingest mode — used by the GUI file-picker so ingestion is
    // deterministic (no LLM/agent): `moon-metis ingest [--nature N] [-r] [--force] <path...>`.
    // Prints a JSON array of per-file results to stdout, then exits.
    let argv: Vec<String> = std::env::args().collect();
    if argv.get(1).map(String::as_str) == Some("ingest") {
        return run_cli_ingest(&config, &store, &state, &argv[2..]).await;
    }

    match config.mcp_transport.as_str() {
        "sse" => {
            info!(
                "Starting SSE transport on {}:{}...",
                config.mcp_host, config.mcp_port
            );
            let addr: std::net::SocketAddr =
                format!("{}:{}", config.mcp_host, config.mcp_port).parse()?;
            let sse_server = rmcp::transport::SseServer::serve(addr).await?;

            let store_c = store.clone();
            let state_c = state.clone();
            let cfg = config.clone();
            let ct = sse_server.with_service(move || {
                MoonMetisServer::new(cfg.clone(), store_c.clone(), state_c.clone())
            });

            info!("Moon Metis MCP server running (SSE)");
            ct.cancelled().await;
        }
        _ => {
            info!("Starting stdio transport...");
            let server = MoonMetisServer::new(config, store, state);
            let transport = rmcp::transport::io::stdio();
            let server_handle = server.serve(transport).await?;
            info!("Moon Metis MCP server running (stdio)");
            server_handle.waiting().await?;
        }
    }

    info!("Moon Metis shutting down");
    Ok(())
}

/// One-shot CLI ingest (file or folder paths). Output: JSON array of results on
/// stdout. Logs go to stderr/file, so stdout stays clean for the caller (GUI).
async fn run_cli_ingest(
    config: &Config,
    store: &Arc<DocStore>,
    state: &Arc<Mutex<IngestState>>,
    args: &[String],
) -> anyhow::Result<()> {
    let mut nature: Option<String> = None;
    let mut recursive = false;
    let mut force = false;
    let mut paths: Vec<String> = Vec::new();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--nature" => {
                i += 1;
                if i < args.len() {
                    nature = Some(args[i].clone());
                }
            }
            "--recursive" | "-r" => recursive = true,
            "--force" => force = true,
            other => paths.push(other.to_string()),
        }
        i += 1;
    }

    let mut st = state.lock().await;
    let mut results: Vec<ingest::FileResult> = Vec::new();
    for p in &paths {
        let path = std::path::Path::new(p);
        if path.is_dir() {
            let mut r = ingest::ingest_folder(
                store,
                &mut st,
                &config.state_path(),
                path,
                recursive,
                nature.as_deref(),
                force,
            )
            .await;
            results.append(&mut r);
        } else {
            let r = ingest::ingest_file(store, &mut st, path, nature.as_deref(), force).await;
            results.push(r);
        }
    }
    if let Err(e) = st.save(&config.state_path()) {
        info!("save state: {e}");
    }

    println!("{}", serde_json::to_string(&results)?);
    Ok(())
}
