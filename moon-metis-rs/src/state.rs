// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

//! Incremental ingest state — a JSON map of source file -> content hash, so a
//! re-run skips unchanged files (pattern borrowed from moon-io's IndexerState:
//! atomic write via tmp + rename so a crash never corrupts it).

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocState {
    pub doc_id: String,
    pub content_hash: String,
    pub nature: String,
    pub indexed_at: String,
    pub chunks: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IngestState {
    /// Keyed by source file path (as supplied to ingest).
    #[serde(default)]
    pub docs: HashMap<String, DocState>,
}

impl IngestState {
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Atomic save: write to a temp file, then rename over the target.
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let tmp = path.with_extension("json.tmp");
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// True if this path is already indexed with the same content hash.
    pub fn is_unchanged(&self, source_path: &str, content_hash: &str) -> bool {
        self.docs
            .get(source_path)
            .map(|d| d.content_hash == content_hash)
            .unwrap_or(false)
    }

    /// Remove the entry matching this `doc_id`. The map is keyed by source path,
    /// but each `DocState` carries its `doc_id`, so we find the key first.
    /// Returns the removed source path, if any.
    pub fn remove_by_doc_id(&mut self, doc_id: &str) -> Option<String> {
        let key = self
            .docs
            .iter()
            .find(|(_, d)| d.doc_id == doc_id)
            .map(|(k, _)| k.clone());
        if let Some(k) = key {
            self.docs.remove(&k);
            return Some(k);
        }
        None
    }
}
