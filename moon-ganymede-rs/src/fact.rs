// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

//! The fact-file model: each `.md` file under the wiki is one entity/affair
//! (a project, a company, …) that accumulates dated facts in fixed sections.
//! This module handles stable slugs and scanning the wiki for existing files.

use std::path::{Path, PathBuf};

use crate::frontmatter::Document;

/// Turn arbitrary text into a stable, filesystem-safe slug.
/// `"Progetto Acme S.r.l."` -> `"progetto-acme-s-r-l"`.
pub fn slugify(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut prev_dash = false;
    for ch in input.trim().chars() {
        let c = ch.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() {
            out.push(c);
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// A lightweight summary of one fact-file, built from its frontmatter.
/// Used by indexing and target resolution without re-reading bodies.
#[derive(Debug, Clone)]
pub struct FactSummary {
    /// Absolute path on disk.
    pub path: PathBuf,
    /// Path relative to the wiki root, with forward slashes (for links).
    pub rel_path: String,
    pub id: String,
    pub tipo: String,
    pub titolo: String,
    pub stato: String,
    pub aggiornato: String,
    /// The full parsed frontmatter, for arbitrary-axis indexing.
    pub doc: Document,
}

/// Recursively scan the wiki for fact files, skipping reserved entries
/// (`_index`, `.history`, files whose name starts with `_`).
pub fn scan_wiki(wiki_dir: &Path) -> Vec<FactSummary> {
    let mut out = Vec::new();
    scan_dir(wiki_dir, wiki_dir, &mut out);
    out.sort_by(|a, b| b.aggiornato.cmp(&a.aggiornato));
    out
}

fn scan_dir(root: &Path, dir: &Path, out: &mut Vec<FactSummary>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || name.starts_with('_') {
            continue; // .history, _index, _taxonomy.yaml, _template.md
        }
        if path.is_dir() {
            scan_dir(root, &path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
            if let Some(summary) = summarize(root, &path) {
                out.push(summary);
            }
        }
    }
}

fn summarize(root: &Path, path: &Path) -> Option<FactSummary> {
    let content = std::fs::read_to_string(path).ok()?;
    let doc = Document::parse(&content).ok()?;
    let rel_path = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");
    let id = doc.get_str("id").unwrap_or_else(|| {
        path.file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default()
    });
    Some(FactSummary {
        path: path.to_path_buf(),
        rel_path,
        id,
        tipo: doc.get_str("tipo").unwrap_or_default(),
        titolo: doc.get_str("titolo").unwrap_or_default(),
        stato: doc.get_str("stato").unwrap_or_default(),
        aggiornato: doc.get_str("aggiornato").unwrap_or_default(),
        doc,
    })
}

/// Find an existing fact-file matching `query` (an id, a title or a free name).
/// Returns the best match together with how it matched and a confidence score.
pub fn find_target<'a>(
    summaries: &'a [FactSummary],
    query: &str,
) -> Option<(&'a FactSummary, &'static str, f64)> {
    let q = query.trim();
    let q_slug = slugify(q);
    let q_lower = q.to_lowercase();

    // 1. Exact id match.
    if let Some(s) = summaries.iter().find(|s| s.id == q_slug || s.id == q) {
        return Some((s, "id", 1.0));
    }
    // 2. Exact (case-insensitive) title match.
    if let Some(s) = summaries
        .iter()
        .find(|s| s.titolo.to_lowercase() == q_lower)
    {
        return Some((s, "titolo", 1.0));
    }
    // 3. Fuzzy match against id and title; conservative threshold.
    let mut best: Option<(&FactSummary, f64)> = None;
    for s in summaries {
        let score = strsim::jaro_winkler(&q_slug, &s.id)
            .max(strsim::jaro_winkler(&q_lower, &s.titolo.to_lowercase()));
        if best.map_or(true, |(_, b)| score > b) {
            best = Some((s, score));
        }
    }
    match best {
        Some((s, score)) if score >= 0.88 => Some((s, "fuzzy", score)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_basics() {
        assert_eq!(slugify("Progetto Acme S.r.l."), "progetto-acme-s-r-l");
        assert_eq!(slugify("  Hello   World  "), "hello-world");
        assert_eq!(slugify("già-ok"), "gi-ok");
        assert_eq!(slugify("ACME"), "acme");
    }
}
