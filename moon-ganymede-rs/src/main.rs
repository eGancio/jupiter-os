// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

use rmcp::ServiceExt;
use tracing::{error, info};
use tracing_subscriber::prelude::*;
use tracing_subscriber::EnvFilter;

use moon_ganymede_rs::config::Config;
use moon_ganymede_rs::server::MoonGanymedeServer;
use moon_ganymede_rs::setup;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load .env if present
    let _ = dotenvy::dotenv();

    let config = Config::from_env();

    // Setup logging: stderr + rolling file (data_dir/moon-ganymede.log)
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,h2=warn,hyper=warn,rustls=warn"));

    std::fs::create_dir_all(&config.data_dir).ok();
    let file_appender =
        tracing_appender::rolling::never(&config.data_dir, "moon-ganymede.log");
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

    info!("Moon Ganymede starting...");
    info!("Wiki dir: {:?}", config.wiki_dir);
    info!(
        "MCP: {}:{} ({})",
        config.mcp_host, config.mcp_port, config.mcp_transport
    );

    // First-run: create the wiki skeleton (taxonomy + template + dirs) if absent.
    if let Err(e) = setup::ensure_wiki_skeleton(&config) {
        error!("Wiki setup failed: {e}");
        return Err(anyhow::anyhow!("Wiki setup failed: {e}"));
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
            // Shared write lock so concurrent SSE connections can't interleave
            // commits/reindex on the same files.
            let write_lock = std::sync::Arc::new(tokio::sync::Mutex::new(()));
            let cfg = config.clone();
            let ct = sse_server.with_service(move || {
                MoonGanymedeServer::with_lock(cfg.clone(), write_lock.clone())
            });

            info!("Moon Ganymede MCP server running (SSE)");
            ct.cancelled().await;
        }
        _ => {
            info!("Starting stdio transport...");
            let server = MoonGanymedeServer::new(config);
            let transport = rmcp::transport::io::stdio();
            let server_handle = server.serve(transport).await?;
            info!("Moon Ganymede MCP server running (stdio)");
            server_handle.waiting().await?;
        }
    }

    info!("Moon Ganymede shutting down");
    Ok(())
}
