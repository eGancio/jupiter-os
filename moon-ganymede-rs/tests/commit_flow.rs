// SPDX-License-Identifier: AGPL-3.0-or-later
// End-to-end exercise of the wiki write path on a temporary directory.

use std::path::PathBuf;

use moon_ganymede_rs::config::Config;
use moon_ganymede_rs::server::MoonGanymedeServer;
use moon_ganymede_rs::setup;
use rmcp::handler::server::tool::Parameters;
use serde_json::Value;

fn temp_config() -> Config {
    // Unique temp dir without relying on Date/rand (use process id + a counter).
    let base = std::env::temp_dir().join(format!(
        "ganymede-test-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    ));
    Config {
        wiki_dir: base.join("wiki"),
        data_dir: base.join("data"),
        mcp_transport: "stdio".into(),
        mcp_host: "127.0.0.1".into(),
        mcp_port: 8400,
    }
}

static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn read(path: PathBuf) -> String {
    std::fs::read_to_string(path).unwrap()
}

#[tokio::test]
async fn full_commit_and_reindex_flow() {
    let config = temp_config();
    setup::ensure_wiki_skeleton(&config).unwrap();
    let wiki = config.wiki_dir.clone();
    let server = MoonGanymedeServer::new(config);

    // Skeleton present.
    assert!(wiki.join("_taxonomy.yaml").exists());
    assert!(wiki.join("_template.md").exists());
    assert!(wiki.join("_index/README.md").exists());

    // 1. Commit a fact with a known reparto but a new società -> rejected first.
    let content = "---\n\
        id: acme\n\
        tipo: progetto\n\
        titolo: \"Progetto Acme\"\n\
        societa: acme-srl\n\
        reparto: Amministrazione\n\
        stato: in-corso\n\
        priorita: alta\n\
        tags: [contratto]\n\
        creato: 2026-05-29\n\
        aggiornato: 2026-05-29\n\
        ---\n\
        ## Sintesi\nNuovo contratto in lavorazione.\n";

    let rejected = server
        .wiki_commit_fact(Parameters(serde_json::from_value(serde_json::json!({
            "path": "progetti/acme.md",
            "content": content,
            "allow_new_tags": false,
        })).unwrap()))
        .await
        .unwrap();
    let v: Value = serde_json::from_str(&rejected).unwrap();
    assert_eq!(v["ok"], false, "new tags must be rejected by default: {rejected}");
    assert!(v["unknown"].as_array().unwrap().iter().any(|u| u["valore"] == "acme-srl"));

    // 2. Retry with allow_new_tags -> committed, società added to taxonomy.
    let ok = server
        .wiki_commit_fact(Parameters(serde_json::from_value(serde_json::json!({
            "path": "progetti/acme.md",
            "content": content,
            "allow_new_tags": true,
        })).unwrap()))
        .await
        .unwrap();
    let v: Value = serde_json::from_str(&ok).unwrap();
    assert_eq!(v["ok"], true, "commit should succeed: {ok}");
    assert_eq!(v["action"], "created");

    // File written, reparto canonicalised to lowercase.
    let saved = read(wiki.join("progetti/acme.md"));
    assert!(saved.contains("reparto: amministrazione"), "got:\n{saved}");
    assert!(saved.contains("## Sintesi"));

    // Index reflects the fact under società and reparto.
    let by_societa = read(wiki.join("_index/by-societa.md"));
    assert!(by_societa.contains("acme-srl"));
    assert!(by_societa.contains("../progetti/acme.md"));
    let by_reparto = read(wiki.join("_index/by-reparto.md"));
    assert!(by_reparto.contains("## amministrazione"));

    // 3. find_target locates the existing file.
    let found = server
        .wiki_find_target(Parameters(serde_json::from_value(serde_json::json!({
            "query": "Progetto Acme",
        })).unwrap()))
        .await
        .unwrap();
    let v: Value = serde_json::from_str(&found).unwrap();
    assert_eq!(v["found"], true, "{found}");
    assert_eq!(v["path"], "progetti/acme.md");

    // 4. validate_tags normalises an alias without writing.
    let validated = server
        .wiki_validate_tags(Parameters(serde_json::from_value(serde_json::json!({
            "content": "reparto: admin\nstato: open\n",
        })).unwrap()))
        .await
        .unwrap();
    let v: Value = serde_json::from_str(&validated).unwrap();
    assert_eq!(v["ok"], true, "{validated}");
    assert_eq!(v["canonical"]["reparto"][0], "amministrazione");
    assert_eq!(v["canonical"]["stato"][0], "in-corso");

    // 5. Reserved path is rejected.
    let reserved = server
        .wiki_commit_fact(Parameters(serde_json::from_value(serde_json::json!({
            "path": "_index/hack.md",
            "content": content,
            "allow_new_tags": true,
        })).unwrap()))
        .await;
    assert!(reserved.is_err(), "writing into _index/ must be refused");

    // 6. Second commit updates and backs up the previous version.
    let _ = server
        .wiki_commit_fact(Parameters(serde_json::from_value(serde_json::json!({
            "path": "progetti/acme.md",
            "content": content.replace("in lavorazione", "firmato"),
            "allow_new_tags": true,
        })).unwrap()))
        .await
        .unwrap();
    let history: Vec<_> = std::fs::read_dir(wiki.join(".history"))
        .unwrap()
        .flatten()
        .collect();
    assert!(!history.is_empty(), "a backup must exist after an update");

    // Cleanup.
    let _ = std::fs::remove_dir_all(wiki.parent().unwrap());
}
