// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli
//
// End-to-end smoke test of the Metis ingest+search pipeline against the REAL
// shared infra (BGE-M3 model + running Qdrant). Skips gracefully if either is
// unavailable so it doesn't fail on a bare CI box.
//
// Run with:  cargo test -p moon-metis-rs -- --nocapture

use std::path::PathBuf;
use std::sync::Arc;

use jupiteros_shared::{DocStore, OnnxEmbedding};
use moon_metis_rs::ingest;
use moon_metis_rs::state::IngestState;

fn model_dir() -> PathBuf {
    // Tests run with cwd = crate dir (moon-metis-rs); reuse Io's model.
    PathBuf::from("../moon-io-rs/data/model")
}

fn qdrant_url() -> String {
    std::env::var("QDRANT_URL").unwrap_or_else(|_| "http://127.0.0.1:6334".into())
}

async fn qdrant_up() -> bool {
    let addr = qdrant_url()
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .to_string();
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        tokio::net::TcpStream::connect(addr),
    )
    .await
    .map(|r| r.is_ok())
    .unwrap_or(false)
}

#[tokio::test]
async fn ingest_and_search_pdf_with_citations() {
    // --- Prerequisites ---
    if !model_dir().join("model.onnx").exists() {
        eprintln!("SKIP: BGE-M3 model not found at {:?}", model_dir());
        return;
    }
    if !qdrant_up().await {
        eprintln!("SKIP: Qdrant not reachable at {}", qdrant_url());
        return;
    }

    // --- 1. Page-aware extraction of the 2-page fixture ---
    let pdf = PathBuf::from("tests/fixtures/norma_sample.pdf");
    assert!(pdf.exists(), "fixture PDF missing");
    let extraction = moon_metis_rs::extract::extract(&pdf).expect("extract");
    assert_eq!(extraction.format, "pdf");
    assert!(!extraction.is_scanned, "fixture should have a text layer");
    assert!(
        extraction.page_count() >= 2,
        "expected >=2 pages, got {}",
        extraction.page_count()
    );

    // --- 2. Build the store on a throwaway collection ---
    // ort loads the ONNX Runtime dylib dynamically — point it at Io's bundle.
    if std::env::var("ORT_DYLIB_PATH").is_err() {
        let dylib = PathBuf::from("../moon-io-rs/data/lib/libonnxruntime.so");
        if dylib.exists() {
            std::env::set_var("ORT_DYLIB_PATH", &dylib);
        } else {
            eprintln!("SKIP: libonnxruntime.so not found at {:?}", dylib);
            return;
        }
    }
    let embedding = Arc::new(OnnxEmbedding::load(&model_dir()).expect("load model"));
    let store = Arc::new(
        DocStore::new(&qdrant_url(), embedding, "metis_docs_test", None)
            .await
            .expect("docstore"),
    );

    // --- 3. Ingest as a "norma" ---
    let mut state = IngestState::default();
    let res = ingest::ingest_file(&store, &mut state, &pdf, Some("norma"), true).await;
    assert_eq!(res.status, "indexed", "ingest status: {} ({})", res.status, res.message);
    assert!(res.chunks > 0, "no chunks produced");
    assert!(res.pages >= 2, "page count not propagated: {}", res.pages);
    let doc_id = res.doc_id.clone();

    // --- 4. Cited hybrid search ---
    let hits = store
        .search("entro quando vanno presentate le domande di contributo", None, None, 8)
        .await
        .expect("search");
    assert!(!hits.is_empty(), "search returned no hits");

    // Page-aware citation: at least one hit carries a real page number.
    assert!(
        hits.iter().any(|h| h.page >= 1),
        "no hit carried a page number"
    );
    // Structure-aware citation: the norma procedure tagged an Article section.
    assert!(
        hits.iter().any(|h| h.section_path.to_lowercase().contains("art")),
        "no hit carried an 'Art.' section: {:?}",
        hits.iter().map(|h| h.section_path.clone()).collect::<Vec<_>>()
    );

    // --- 5. nature filter + inventory ---
    let filtered = store
        .search("requisiti impresa", Some("norma"), None, 5)
        .await
        .expect("filtered search");
    assert!(filtered.iter().all(|h| h.nature == "norma"));

    let docs = store.list_documents().await.expect("list");
    assert!(docs.iter().any(|d| d.doc_id == doc_id));

    // --- 6. incremental skip ---
    let again = ingest::ingest_file(&store, &mut state, &pdf, Some("norma"), false).await;
    assert_eq!(again.status, "skipped", "unchanged file should be skipped");

    // --- cleanup ---
    let removed = store.delete_document(&doc_id).await.expect("cleanup");
    assert!(removed > 0);

    eprintln!(
        "OK: {} pagine, {} chunk indicizzati; citazioni pagina/sezione verificate.",
        res.pages, res.chunks
    );
}
