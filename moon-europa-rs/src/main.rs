use std::sync::Arc;

use rmcp::ServiceExt;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

use moon_europa_rs::adapters::telegram::TelegramAdapter;
use moon_europa_rs::adapters::MessagingAdapter;
use moon_europa_rs::config::Config;
use moon_europa_rs::daemon::TelegramDaemon;
use moon_europa_rs::server::EuropaServer;
use moon_europa_rs::setup;

use jupiteros_shared::embeddings::OnnxEmbedding;
use jupiteros_shared::store::MessageStore;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load .env file (ignore if missing)
    let _ = dotenvy::dotenv();

    // Logging
    tracing_subscriber::fmt()
        .with_ansi(false)
        .with_target(false)
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,ort=warn,h2=warn,hyper=warn,rustls=warn")),
        )
        .init();

    // Check for --auth flag (interactive Telegram login)
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--auth") {
        let config = Config::from_env().map_err(|e| anyhow::anyhow!(e))?;
        return TelegramAdapter::auth_interactive(&config).await;
    }

    info!("Moon Europa starting...");

    // Load config
    let config = Config::from_env().map_err(|e| anyhow::anyhow!(e))?;
    info!(
        "Transport: {}, Host: {}, Port: {}",
        config.mcp_transport, config.mcp_host, config.mcp_port
    );

    // --- Auto-setup: download ONNX model + start Qdrant if needed ---
    setup::ensure_onnx_model(&config.onnx_model_dir).await?;
    setup::ensure_qdrant(&config.qdrant_url, &config.data_dir).await?;

    // Init embeddings model
    info!(
        "Loading ONNX embedding model from {:?}...",
        config.onnx_model_dir
    );
    let embedder = OnnxEmbedding::load(&config.onnx_model_dir)?;
    info!("Embedding model loaded (dim={})", embedder.dimension());

    // Init Qdrant store
    let store = MessageStore::new(
        &config.qdrant_url,
        Arc::new(embedder),
        "europa_messages",
        None,
    )
    .await?;
    let store = Arc::new(store);
    info!("Qdrant store ready");

    // Init Telegram adapter
    let mut adapter = TelegramAdapter::new(config.clone());

    if config.has_telegram() {
        info!("Connecting to Telegram...");
        adapter.connect().await?;
        info!("Telegram connected");
    } else {
        info!("Telegram not configured (no API credentials), skipping connection");
    }

    let adapter = Arc::new(adapter);

    // Spawn daemon (bulk index + listener) in background
    if config.has_telegram() {
        let daemon = TelegramDaemon::new(
            Arc::clone(&adapter),
            Arc::clone(&store),
            config.clone(),
        );
        let handle = tokio::spawn(async move {
            daemon.run_forever().await;
        });
        info!("Indexer daemon spawned");

        // Monitor daemon task — log if it exits or panics
        tokio::spawn(async move {
            match handle.await {
                Ok(()) => error!("Telegram daemon exited unexpectedly (no error)"),
                Err(e) => error!("Telegram daemon PANICKED: {e}"),
            }
        });
    }

    // Start MCP server
    match config.mcp_transport.as_str() {
        "sse" => {
            info!(
                "Starting SSE transport on {}:{}...",
                config.mcp_host, config.mcp_port
            );
            let addr: std::net::SocketAddr =
                format!("{}:{}", config.mcp_host, config.mcp_port).parse()?;

            let sse_server = rmcp::transport::SseServer::serve(addr).await?;

            // with_service creates a new EuropaServer for each client connection
            let ct = sse_server.with_service(move || {
                EuropaServer::new(
                    Arc::clone(&store),
                    Arc::clone(&adapter),
                    config.clone(),
                )
            });

            info!("Moon Europa MCP server running (SSE)");
            ct.cancelled().await;
        }
        _ => {
            // stdio transport (default)
            info!("Starting stdio transport...");
            let server =
                EuropaServer::new(Arc::clone(&store), Arc::clone(&adapter), config.clone());
            let transport = rmcp::transport::io::stdio();
            let server_handle = server.serve(transport).await?;
            info!("Moon Europa MCP server running (stdio)");
            server_handle.waiting().await?;
        }
    }

    Ok(())
}
