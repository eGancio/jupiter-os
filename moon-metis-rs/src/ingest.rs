// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

//! The deterministic ingest pipeline:
//! `FILE → extract (page-aware) → chunk (per procedure) → embed → upsert`.
//!
//! Incremental: a file whose content hash is unchanged since last run is
//! skipped. Re-ingesting a changed file deletes its old chunks first, so the
//! knowledge base never accumulates stale duplicates.

use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::Serialize;
use sha2::{Digest, Sha256};
use tracing::{info, warn};
use uuid::Uuid;

use jupiteros_shared::DocStore;

use crate::chunk::chunk_document;
use crate::extract;
use crate::procedure;
use crate::state::{DocState, IngestState};

/// Extensions Metis will attempt to ingest from a folder walk.
pub const SUPPORTED_EXTS: &[&str] = &[
    "pdf", "docx", "xlsx", "xls", "txt", "md", "csv", "html", "htm", "json", "xml", "yaml", "yml",
];

/// Per-file ingest outcome (JSON-serializable for tool responses).
#[derive(Debug, Clone, Serialize)]
pub struct FileResult {
    pub source_path: String,
    /// "indexed" | "skipped" | "scanned" | "empty" | "error"
    pub status: String,
    pub doc_id: String,
    pub title: String,
    pub nature: String,
    pub pages: u32,
    pub chunks: usize,
    pub message: String,
}

impl FileResult {
    fn simple(source_path: &str, status: &str, message: &str) -> Self {
        Self {
            source_path: source_path.to_string(),
            status: status.to_string(),
            doc_id: String::new(),
            title: String::new(),
            nature: String::new(),
            pages: 0,
            chunks: 0,
            message: message.to_string(),
        }
    }
}

/// SHA-256 hex of file bytes.
pub fn content_hash(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

/// Stable document id derived from the (canonical) source path.
pub fn doc_id_for(source_path: &str) -> String {
    Uuid::new_v5(&Uuid::NAMESPACE_URL, source_path.as_bytes()).to_string()
}

fn is_supported(path: &Path) -> bool {
    path.extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .map(|e| SUPPORTED_EXTS.contains(&e.as_str()))
        .unwrap_or(false)
}

/// Ingest a single file. `nature` is the agent's judgement (defaults to
/// "generico"); `force` re-indexes even if unchanged.
pub async fn ingest_file(
    store: &DocStore,
    state: &mut IngestState,
    path: &Path,
    nature: Option<&str>,
    force: bool,
) -> FileResult {
    let source_path = canonical_string(path);

    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => return FileResult::simple(&source_path, "error", &format!("Lettura file: {e}")),
    };
    let hash = content_hash(&bytes);

    if !force && state.is_unchanged(&source_path, &hash) {
        return FileResult::simple(&source_path, "skipped", "Invariato dall'ultimo ingest");
    }

    // Extraction is CPU-bound and, for scanned PDFs, may run OCR over many pages
    // (seconds each) — keep it off the async runtime threads.
    let path_buf = path.to_path_buf();
    let extraction = match tokio::task::spawn_blocking(move || extract::extract(&path_buf)).await {
        Ok(Ok(x)) => x,
        Ok(Err(e)) => return FileResult::simple(&source_path, "error", &e.to_string()),
        Err(e) => {
            return FileResult::simple(&source_path, "error", &format!("Task estrazione: {e}"))
        }
    };

    // Scanned PDF that OCR couldn't recover (OCR off, failed, or empty result).
    if extraction.is_scanned && !extraction.ocr_applied {
        return FileResult::simple(
            &source_path,
            "scanned",
            "Documento immagine (PDF scansionato): OCR non disponibile o senza testo, non indicizzato.",
        );
    }

    let nature = nature.filter(|s| !s.is_empty()).unwrap_or("generico");
    let proc = procedure::get(nature);
    let title = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| source_path.clone());
    let doc_id = doc_id_for(&source_path);

    let chunks = chunk_document(&extraction, &proc, &doc_id, &source_path, &title, &hash);
    if chunks.is_empty() {
        return FileResult::simple(&source_path, "empty", "Nessun testo estraibile dopo il chunking");
    }
    let n_chunks = chunks.len();
    let pages = extraction.page_count();

    // Replace any previous version of this document, then index fresh.
    if let Err(e) = store.delete_document(&doc_id).await {
        warn!("delete_document {doc_id}: {e}");
    }
    if let Err(e) = store.index_chunks(chunks, true).await {
        return FileResult::simple(&source_path, "error", &format!("Indicizzazione: {e}"));
    }

    state.docs.insert(
        source_path.clone(),
        DocState {
            doc_id: doc_id.clone(),
            content_hash: hash,
            nature: proc.nature.clone(),
            indexed_at: Utc::now().to_rfc3339(),
            chunks: n_chunks,
        },
    );

    info!("Indicizzato {source_path} ({n_chunks} chunk, {pages} pagine, natura={})", proc.nature);
    FileResult {
        source_path,
        status: "indexed".into(),
        doc_id,
        title,
        nature: proc.nature,
        pages,
        chunks: n_chunks,
        message: String::new(),
    }
}

/// Ingest every supported file in a folder. Saves state after each file so the
/// run is resumable. Returns one `FileResult` per file considered.
pub async fn ingest_folder(
    store: &DocStore,
    state: &mut IngestState,
    state_path: &Path,
    dir: &Path,
    recursive: bool,
    nature: Option<&str>,
    force: bool,
) -> Vec<FileResult> {
    let mut results = Vec::new();
    let files = collect_files(dir, recursive);

    for file in files {
        let r = ingest_file(store, state, &file, nature, force).await;
        // Persist progress after every file (resumability).
        if let Err(e) = state.save(state_path) {
            warn!("Salvataggio stato: {e}");
        }
        results.push(r);
    }

    results
}

/// Collect supported files under `dir` (optionally recursive).
fn collect_files(dir: &Path, recursive: bool) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let entries = match std::fs::read_dir(&d) {
            Ok(e) => e,
            Err(e) => {
                warn!("read_dir {}: {e}", d.display());
                continue;
            }
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                if recursive {
                    stack.push(p);
                }
            } else if is_supported(&p) {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// Canonicalised path string (falls back to the lossy display path).
fn canonical_string(path: &Path) -> String {
    std::fs::canonicalize(path)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| path.to_string_lossy().to_string())
}
