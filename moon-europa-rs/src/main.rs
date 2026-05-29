use std::collections::HashMap;
use std::sync::Arc;

use rmcp::ServiceExt;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

use moon_europa_rs::adapters::slack::SlackAdapter;
use moon_europa_rs::adapters::teams::TeamsAdapter;
use moon_europa_rs::adapters::telegram::TelegramAdapter;
use moon_europa_rs::adapters::telegram_bot::TelegramBotAdapter;
use moon_europa_rs::adapters::MessagingAdapter;
use moon_europa_rs::config::Config;
use moon_europa_rs::daemon::ChannelDaemon;
use moon_europa_rs::server::{AdapterMap, EuropaServer};
use moon_europa_rs::setup;

use jupiteros_shared::embeddings::OnnxEmbedding;
use jupiteros_shared::store::MessageStore;

/// Write/remove a `<channel>.connected` marker in the data dir so the GUI can
/// show whether moon-europa actually connected to that channel.
fn set_channel_connected(data_dir: &std::path::Path, channel: &str, ok: bool) {
    let path = data_dir.join(format!("{channel}.connected"));
    if ok {
        let _ = std::fs::write(&path, b"1");
    } else {
        let _ = std::fs::remove_file(&path);
    }
}

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

    // Point ORT to the bundled runtime library if present in data/lib/
    // (mirrors moon-io) and not already overridden by the environment.
    let bundled_ort = config.data_dir.join("lib").join("libonnxruntime.so");
    if bundled_ort.exists() && std::env::var("ORT_DYLIB_PATH").is_err() {
        std::env::set_var("ORT_DYLIB_PATH", &bundled_ort);
        info!("ORT dylib: {:?}", bundled_ort);
    }

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

    // --- Build the adapter registry from the configured channels ---
    let mut adapters: AdapterMap = HashMap::new();

    // Reset all connection markers; each successful connect writes its own.
    // The GUI reads these to show the REAL connection state (not just "creds saved").
    for ch in ["telegram", "slack", "teams"] {
        set_channel_connected(&config.data_dir, ch, false);
    }

    // Telegram: a bot token (if present) serves the channel via the lightweight
    // HTTP Bot adapter; otherwise the user account (MTProto) is used.
    if config.telegram_is_bot() {
        info!("Connecting Telegram (bot mode)...");
        let mut bot = TelegramBotAdapter::new(config.clone());
        match bot.connect().await {
            Ok(()) => {
                adapters.insert("telegram".into(), Arc::new(bot));
                set_channel_connected(&config.data_dir, "telegram", true);
                info!("Telegram bot connected");
            }
            Err(e) => warn!("Telegram bot connect failed: {e}"),
        }
    } else if config.has_telegram() {
        info!("Connecting Telegram (user account)...");
        let mut tg = TelegramAdapter::new(config.clone());
        match tg.connect().await {
            Ok(()) => {
                adapters.insert("telegram".into(), Arc::new(tg));
                set_channel_connected(&config.data_dir, "telegram", true);
                info!("Telegram connected");
            }
            Err(e) => warn!("Telegram connect failed: {e}"),
        }
    } else {
        info!("Telegram not configured, skipping");
    }

    // Slack
    if config.has_slack() {
        info!("Connecting Slack...");
        let mut sl = SlackAdapter::new(config.clone());
        match sl.connect().await {
            Ok(()) => {
                adapters.insert("slack".into(), Arc::new(sl));
                set_channel_connected(&config.data_dir, "slack", true);
                info!("Slack connected");
            }
            Err(e) => warn!("Slack connect failed: {e}"),
        }
    }

    // Microsoft Teams
    if config.has_teams() {
        info!("Connecting Teams...");
        let mut tm = TeamsAdapter::new(config.clone());
        match tm.connect().await {
            Ok(()) => {
                adapters.insert("teams".into(), Arc::new(tm));
                set_channel_connected(&config.data_dir, "teams", true);
                info!("Teams connected");
            }
            Err(e) => warn!("Teams connect failed: {e}"),
        }
    }

    if adapters.is_empty() {
        info!("No messaging channel configured. Configure one from the Europa panel in JupiterOS.");
    }

    // Spawn one indexer daemon per connected channel
    for (name, adapter) in &adapters {
        let daemon = ChannelDaemon::new(
            Arc::clone(adapter),
            Arc::clone(&store),
            config.clone(),
        );
        let channel = name.clone();
        let handle = tokio::spawn(async move {
            daemon.run_forever().await;
        });
        info!("Indexer daemon spawned for '{channel}'");

        // Monitor daemon task — log if it exits or panics
        tokio::spawn(async move {
            match handle.await {
                Ok(()) => error!("Daemon '{channel}' exited unexpectedly (no error)"),
                Err(e) => error!("Daemon '{channel}' PANICKED: {e}"),
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
                    adapters.clone(),
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
                EuropaServer::new(Arc::clone(&store), adapters.clone(), config.clone());
            let transport = rmcp::transport::io::stdio();
            let server_handle = server.serve(transport).await?;
            info!("Moon Europa MCP server running (stdio)");
            server_handle.waiting().await?;
        }
    }

    Ok(())
}
