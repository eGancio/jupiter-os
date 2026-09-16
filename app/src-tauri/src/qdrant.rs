// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

//! Host-owned Qdrant supervisor.
//!
//! JupiterOS — non i Moon — è il proprietario dell'istanza Qdrant condivisa:
//! la avvia nel `setup()` dell'app, PRIMA che qualunque Moon parta, così ogni
//! Moon trova la porta già occupata e si limita a connettersi. L'`ensure_qdrant`
//! dentro Io/Europa resta solo come fallback per uso standalone/dev.
//!
//! Questo elimina la race d'avvio in cui il primo Moon a partire montava la
//! PROPRIA cartella storage (sparizione "fantasma" dei dati indicizzati dagli
//! altri giorni). Lo storage è derivato dalla def del server `io` nel
//! `.mcp.json` (env `QDRANT_STORAGE_DIR`, fallback `<cwd io>/data/qdrant`).

use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use crate::config::{mcp_json_path, McpConfig, ServerDef};

static CHILD: OnceLock<Mutex<Option<Child>>> = OnceLock::new();

fn child_slot() -> &'static Mutex<Option<Child>> {
    CHILD.get_or_init(|| Mutex::new(None))
}

/// Avvia (se serve) il Qdrant condiviso. Non blocca il setup dell'app:
/// lo spawn è immediato, l'attesa di readiness gira in un thread.
/// Best-effort: se manca la def o il binario (prima installazione), non fa
/// nulla e lascia il bootstrap al fallback dei Moon.
pub fn ensure_running() {
    let Some(def) = io_server_def() else {
        eprintln!("[qdrant] nessuna def 'io'/'europa' in .mcp.json: skip (bootstrap ai Moon)");
        return;
    };
    let Some(cwd) = def.cwd.as_ref().map(PathBuf::from).filter(|p| p.is_dir()) else {
        eprintln!("[qdrant] cwd mancante o inesistente nella def: skip");
        return;
    };

    let grpc_addr = def
        .env
        .get("QDRANT_URL")
        .map(|u| {
            u.trim_start_matches("http://")
                .trim_start_matches("https://")
                .trim_end_matches('/')
                .to_string()
        })
        .unwrap_or_else(|| "127.0.0.1:6334".to_string());

    if tcp_up(&grpc_addr) {
        println!("[qdrant] già attivo su {grpc_addr}: adottato, nessuno spawn");
        return;
    }

    let bin = cwd
        .join("data")
        .join("qdrant")
        .join(if cfg!(windows) { "qdrant.exe" } else { "qdrant" });
    if !bin.exists() {
        eprintln!(
            "[qdrant] binario assente ({}): prima esecuzione, lo scaricherà Moon Io",
            bin.display()
        );
        return;
    }

    let storage_root = def
        .env
        .get("QDRANT_STORAGE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| cwd.join("data").join("qdrant"));
    let storage = storage_root.join("storage");
    let snapshots = storage_root.join("snapshots");
    let _ = std::fs::create_dir_all(&storage);
    let _ = std::fs::create_dir_all(&snapshots);

    match Command::new(&bin)
        .env("QDRANT__STORAGE__STORAGE_PATH", &storage)
        .env("QDRANT__STORAGE__SNAPSHOTS_PATH", &snapshots)
        .env("QDRANT__TELEMETRY_DISABLED", "true")
        // cwd del Moon Io: così `./static` (web UI su :6333/dashboard) viene servita
        .current_dir(&cwd)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => {
            println!(
                "[qdrant] avviato (pid {}) — storage {}",
                child.id(),
                storage.display()
            );
            if let Ok(mut slot) = child_slot().lock() {
                *slot = Some(child);
            }
            std::thread::spawn(move || {
                for i in 0..30 {
                    if tcp_up(&grpc_addr) {
                        println!("[qdrant] pronto ({}s)", i + 1);
                        return;
                    }
                    std::thread::sleep(Duration::from_secs(1));
                }
                eprintln!("[qdrant] non pronto dopo 30s");
            });
        }
        Err(e) => eprintln!("[qdrant] spawn fallito: {e}"),
    }
}

/// Ferma il Qdrant di proprietà dell'host (se lo abbiamo avviato noi).
/// Un'istanza preesistente "adottata" non viene toccata.
pub fn shutdown() {
    if let Some(mut child) = child_slot().lock().ok().and_then(|mut g| g.take()) {
        let _ = child.kill();
        let _ = child.wait();
        println!("[qdrant] fermato");
    }
}

fn io_server_def() -> Option<ServerDef> {
    let cfg: McpConfig =
        serde_json::from_str(&std::fs::read_to_string(mcp_json_path()).ok()?).ok()?;
    cfg.servers
        .get("io")
        .or_else(|| cfg.servers.get("europa"))
        .cloned()
}

fn tcp_up(addr: &str) -> bool {
    use std::net::ToSocketAddrs;
    addr.to_socket_addrs()
        .ok()
        .and_then(|mut it| it.next())
        .map(|sa| TcpStream::connect_timeout(&sa, Duration::from_secs(2)).is_ok())
        .unwrap_or(false)
}
