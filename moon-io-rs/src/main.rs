use std::sync::Arc;

use rmcp::ServiceExt;
use tokio::sync::Mutex;
use tracing::{error, info};
use tracing_subscriber::prelude::*;
use tracing_subscriber::EnvFilter;

use moon_io_rs::config::Config;
use moon_io_rs::credentials;
use moon_io_rs::daemon::EmailDaemon;
use moon_io_rs::oauth;
use moon_io_rs::server::{IndexTaskStatus, MoonIoServer};
use moon_io_rs::setup;

use jupiteros_shared::embeddings::OnnxEmbedding;
use jupiteros_shared::store::MessageStore;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load .env if present
    let _ = dotenvy::dotenv();

    // ---- CLI mode: `moon-io credentials <subcommand>` or `moon-io oauth <subcommand>` ----
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(|s| s.as_str()) == Some("credentials") {
        return run_credentials_cli(&args).await.map_err(anyhow::Error::msg);
    }
    if args.get(1).map(|s| s.as_str()) == Some("oauth") {
        return run_oauth_cli(&args).await.map_err(anyhow::Error::msg);
    }

    // Load config early so we can use data_dir for log file
    let config = Config::from_env().map_err(|e| anyhow::anyhow!("Config error: {e}"))?;

    // Setup logging: stderr + file (data_dir/moon-io.log)
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,ort=warn,h2=warn,hyper=warn,rustls=warn"));

    let file_appender =
        tracing_appender::rolling::never(&config.data_dir, "moon-io.log");
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

    info!("Moon Io starting...");
    info!("Accounts configured: {}", config.accounts.len());
    for a in &config.accounts {
        info!(
            "  [{}] IMAP {}:{} ({}), SMTP {}:{}",
            a.name, a.imap_host, a.imap_port, a.imap_username, a.smtp_host, a.smtp_port
        );
    }
    info!(
        "MCP: {}:{} ({})",
        config.mcp_host, config.mcp_port, config.mcp_transport
    );
    info!("Qdrant: {}", config.qdrant_url);

    // --- Auto-setup: download ONNX model + start Qdrant if needed ---
    setup::ensure_onnx_model(&config.onnx_model_dir).await?;
    setup::ensure_qdrant(&config.qdrant_url, &config.data_dir).await?;

    // Load embedding model
    info!(
        "Loading ONNX embedding model from {:?}...",
        config.onnx_model_dir
    );
    let embedding = Arc::new(OnnxEmbedding::load(&config.onnx_model_dir)?);

    // Create message store
    let store = Arc::new(
        MessageStore::new(&config.qdrant_url, embedding, "emails", None).await?,
    );
    info!("Message store ready (collection: emails)");

    // Spawn daemon in background (with panic/exit logging)
    {
        let daemon_config = config.clone();
        let daemon_store = store.clone();
        let handle = tokio::spawn(async move {
            let daemon = EmailDaemon::new(daemon_config, daemon_store);
            daemon.run().await;
        });
        info!("Email daemon spawned");

        // Monitor daemon task — log if it exits or panics
        tokio::spawn(async move {
            match handle.await {
                Ok(()) => error!("Email daemon exited unexpectedly (no error)"),
                Err(e) => error!("Email daemon PANICKED: {e}"),
            }
        });
    }

    // Shared state — survives SSE connection drops
    let shared_task_status = Arc::new(Mutex::new(IndexTaskStatus::default()));
    let shared_index_lock = Arc::new(Mutex::new(()));

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

            // with_service creates a new MoonIoServer for each client connection,
            // but they all share the same task_status + index_lock so background
            // tasks survive connection drops
            let ct = sse_server.with_service(move || {
                MoonIoServer::new(
                    config.clone(),
                    Arc::clone(&store),
                    Arc::clone(&shared_task_status),
                    Arc::clone(&shared_index_lock),
                )
            });

            info!("Moon Io MCP server running (SSE)");
            ct.cancelled().await;
        }
        _ => {
            // stdio transport (default for Claude Code / JupiterOS)
            info!("Starting stdio transport...");
            let server = MoonIoServer::new(config, store, shared_task_status, shared_index_lock);
            let transport = rmcp::transport::io::stdio();
            let server_handle = server.serve(transport).await?;
            info!("Moon Io MCP server running (stdio)");
            server_handle.waiting().await?;
        }
    }

    info!("Moon Io shutting down");
    Ok(())
}

// ===========================================================================
// CLI subcommand: credentials management
// ===========================================================================

async fn run_credentials_cli(args: &[String]) -> Result<(), String> {
    let sub = args.get(2).map(|s| s.as_str()).unwrap_or("help");
    match sub {
        "set" => {
            let account = args
                .get(3)
                .ok_or_else(|| "Usage: moon-io credentials set <account>".to_string())?;
            let password = rpassword::prompt_password(format!(
                "Password for account '{account}' (hidden): "
            ))
            .map_err(|e| format!("Read password: {e}"))?;
            if password.trim().is_empty() {
                return Err("Password vuota, annullato.".into());
            }
            credentials::set_password(account, password.trim())?;
            println!("Salvata password per account '{account}' nel Windows Credential Manager.");
            println!("Ricordati di rimuovere la password dal file .mcp.json se era lì.");
            Ok(())
        }
        "delete" | "remove" | "rm" => {
            let account = args
                .get(3)
                .ok_or_else(|| "Usage: moon-io credentials delete <account>".to_string())?;
            let removed = credentials::delete_password(account)?;
            if removed {
                println!("Rimossa password per account '{account}'.");
            } else {
                println!("Nessuna password trovata per account '{account}'.");
            }
            Ok(())
        }
        "list" | "ls" => {
            // Iterate well-known account names; we don't have a way to list all
            // entries in keyring without a service-wide query, so we check the
            // ones declared via env.
            let known: Vec<String> = std::env::var("ACCOUNTS")
                .unwrap_or_default()
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            let mut to_check: Vec<String> = vec!["primary".into(), "gmail".into()];
            if let Ok(name) = std::env::var("IMAP_ACCOUNT_NAME") {
                if !name.is_empty() && !to_check.contains(&name) {
                    to_check.push(name);
                }
            }
            for n in known {
                if !to_check.contains(&n) {
                    to_check.push(n);
                }
            }
            println!("Account → password in keyring?");
            for name in &to_check {
                let has = credentials::has_password(name);
                println!("  {name:10} {}", if has { "yes" } else { "no" });
            }
            Ok(())
        }
        "info" => {
            let account = args
                .get(3)
                .ok_or_else(|| "Usage: moon-io credentials info <account>".to_string())?;
            match credentials::get_password(account) {
                Some(p) => {
                    println!("Account: {account}");
                    println!("  In keyring:    yes");
                    println!("  Password len:  {}", p.chars().count());
                    println!("  Has whitespace: {}", p.chars().any(|c| c.is_whitespace()));
                    println!("  Starts with:   {}", p.chars().next().unwrap_or(' '));
                    println!("  Ends with:     {}", p.chars().last().unwrap_or(' '));
                    println!("  ASCII only:    {}", p.chars().all(|c| c.is_ascii()));
                }
                None => {
                    println!("Account: {account}");
                    println!("  In keyring:    NO");
                }
            }
            Ok(())
        }
        "test" => {
            // Real IMAP login test using the keyring password.
            // Requires IMAP_HOST/IMAP_USERNAME from env (so run with .mcp.json env).
            let account = args
                .get(3)
                .ok_or_else(|| "Usage: moon-io credentials test <account>".to_string())?;

            let password = credentials::get_password(account)
                .ok_or_else(|| format!("Nessuna password nel keyring per '{account}'."))?;

            // Resolve username + host from env (must be run from .mcp.json context)
            let (username, host, port) = if account == "gmail" {
                (
                    std::env::var("GMAIL_USERNAME").unwrap_or_default(),
                    "imap.gmail.com".to_string(),
                    993u16,
                )
            } else {
                (
                    std::env::var("IMAP_USERNAME").unwrap_or_default(),
                    std::env::var("IMAP_HOST")
                        .unwrap_or_else(|_| "imaps.aruba.it".into()),
                    std::env::var("IMAP_PORT")
                        .ok()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(993u16),
                )
            };

            if username.is_empty() {
                return Err(
                    "Username vuoto. Lancia con le env del .mcp.json oppure setta IMAP_USERNAME / GMAIL_USERNAME.".into(),
                );
            }

            println!("Testing IMAP login...");
            println!("  Account:  {account}");
            println!("  Host:     {host}:{port}");
            println!("  Username: {username}");
            println!("  Password: {} chars from keyring", password.chars().count());

            use tokio_util::compat::TokioAsyncReadCompatExt;
            let tls = async_native_tls::TlsConnector::new();
            let tcp = tokio::net::TcpStream::connect(format!("{host}:{port}"))
                .await
                .map_err(|e| format!("TCP connect: {e}"))?;
            let tls_stream = tls
                .connect(&host, tcp.compat())
                .await
                .map_err(|e| format!("TLS: {e}"))?;
            let client = async_imap::Client::new(tls_stream);
            match client.login(&username, &password).await {
                Ok(mut session) => {
                    println!();
                    println!("  RESULT:   OK ✓ login accepted by server");
                    let _ = session.logout().await;
                    Ok(())
                }
                Err((e, _)) => {
                    println!();
                    println!("  RESULT:   FAIL ✗");
                    println!("  Server says: {e}");
                    Err(format!("Login failed: {e}"))
                }
            }
        }
        _ => {
            println!("Usage: moon-io credentials <subcommand>");
            println!();
            println!("Subcommands:");
            println!("  set <account>     Store a password in Windows Credential Manager");
            println!("  delete <account>  Remove a stored password");
            println!("  list              Show which accounts have a stored password");
            println!("  info <account>    Show metadata about a stored password (length, ASCII, etc.) — never prints it");
            println!("  test <account>    Probe an IMAP login with the stored password");
            println!();
            println!("Account names: 'primary' (or IMAP_ACCOUNT_NAME), 'gmail', or any from ACCOUNTS=...");
            Ok(())
        }
    }
}

// ===========================================================================
// CLI subcommand: OAuth flow
// ===========================================================================

async fn run_oauth_cli(args: &[String]) -> Result<(), String> {
    let sub = args.get(2).map(|s| s.as_str()).unwrap_or("help");
    match sub {
        "connect" => {
            // moon-io oauth connect <account>
            // Reads client_id and client_secret from env vars:
            //   MOONIO_OAUTH_CLIENT_ID
            //   MOONIO_OAUTH_CLIENT_SECRET
            // (env keeps secrets out of shell history and process tables of
            //  other users)
            let account = args
                .get(3)
                .ok_or_else(|| "Usage: moon-io oauth connect <account>".to_string())?;

            let client_id = std::env::var("MOONIO_OAUTH_CLIENT_ID")
                .map_err(|_| "Missing env MOONIO_OAUTH_CLIENT_ID".to_string())?;
            let client_secret = std::env::var("MOONIO_OAUTH_CLIENT_SECRET")
                .map_err(|_| "Missing env MOONIO_OAUTH_CLIENT_SECRET".to_string())?;

            if client_id.trim().is_empty() {
                return Err("MOONIO_OAUTH_CLIENT_ID vuoto".into());
            }
            if client_secret.trim().is_empty() {
                return Err("MOONIO_OAUTH_CLIENT_SECRET vuoto".into());
            }

            println!("Starting OAuth flow for account '{account}'...");
            println!("Apertura del browser per il consenso Google...");

            oauth::authorize_interactive(
                account,
                client_id.trim(),
                client_secret.trim(),
                300, // 5 minutes timeout
            )
            .await?;

            println!("OAuth completato. Token salvato in keyring sotto 'oauth:{account}'.");
            println!("Restart Moon Io per attivare l'account con OAuth.");
            Ok(())
        }
        "disconnect" => {
            let account = args
                .get(3)
                .ok_or_else(|| "Usage: moon-io oauth disconnect <account>".to_string())?;
            let removed = oauth::delete_credentials(account)?;
            if removed {
                println!("Credenziali OAuth rimosse per '{account}'.");
            } else {
                println!("Nessuna credenziale OAuth trovata per '{account}'.");
            }
            Ok(())
        }
        "status" => {
            let account = args
                .get(3)
                .ok_or_else(|| "Usage: moon-io oauth status <account>".to_string())?;
            if oauth::has_credentials(account) {
                println!("Account '{account}': OAuth attivo ✓");
            } else {
                println!("Account '{account}': OAuth non configurato.");
            }
            Ok(())
        }
        _ => {
            println!("Usage: moon-io oauth <subcommand> <account>");
            println!();
            println!("Subcommands:");
            println!("  connect <account>     Run OAuth authorization flow (opens browser)");
            println!("                        Reads MOONIO_OAUTH_CLIENT_ID and");
            println!("                        MOONIO_OAUTH_CLIENT_SECRET from env.");
            println!("  disconnect <account>  Remove stored OAuth credentials");
            println!("  status <account>      Check if OAuth is configured");
            Ok(())
        }
    }
}
