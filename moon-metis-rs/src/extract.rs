// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

//! Page-aware text extraction.
//!
//! PDFs are extracted **per page** via `pdf_extract::extract_text_by_pages`, so
//! every chunk can carry its page number for citations. Other formats (docx,
//! xlsx, plain text) are extracted via the shared `file_utils` and treated as a
//! single page-less unit (page = 0).

use std::path::Path;

use anyhow::{anyhow, Result};

/// A single extracted page (1-based `number`; 0 for page-less formats).
#[derive(Debug, Clone)]
pub struct Page {
    pub number: u32,
    pub text: String,
}

/// Result of extracting a document.
#[derive(Debug, Clone)]
pub struct Extraction {
    pub format: String,
    /// True when a PDF yielded no extractable text on any page (likely scanned
    /// images) — the caller should warn and skip rather than index nothing.
    pub is_scanned: bool,
    pub pages: Vec<Page>,
}

impl Extraction {
    pub fn page_count(&self) -> u32 {
        self.pages.iter().map(|p| p.number).max().unwrap_or(0)
    }

    pub fn total_chars(&self) -> usize {
        self.pages.iter().map(|p| p.text.len()).sum()
    }
}

/// Extract a document into page-aware text.
pub fn extract(path: &Path) -> Result<Extraction> {
    if !path.exists() || !path.is_file() {
        return Err(anyhow!("File non trovato: {}", path.display()));
    }
    let ext = path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();

    if ext == "pdf" {
        return extract_pdf(path);
    }

    // Non-PDF: single page-less unit via the shared extractor.
    // NOTE: file_utils clamps at 25k chars — acceptable for the MVP (most
    // bandi/norme are PDF). Larger docx/xlsx will be truncated.
    let raw = jupiteros_shared::file_utils::extract_text(path, 25_000)
        .map_err(|e| anyhow!("Estrazione fallita: {e}"))?;
    let text = strip_file_utils_header(&raw);

    Ok(Extraction {
        format: ext,
        is_scanned: false,
        pages: vec![Page {
            number: 0,
            text,
        }],
    })
}

fn extract_pdf(path: &Path) -> Result<Extraction> {
    let pages_text = pdf_extract::extract_text_by_pages(path)
        .map_err(|e| anyhow!("Estrazione PDF fallita: {e}"))?;

    let pages: Vec<Page> = pages_text
        .into_iter()
        .enumerate()
        .map(|(i, t)| Page {
            number: (i + 1) as u32,
            text: t,
        })
        .collect();

    let is_scanned = pages.iter().all(|p| p.text.trim().is_empty());

    Ok(Extraction {
        format: "pdf".to_string(),
        is_scanned,
        pages,
    })
}

/// `file_utils::extract_text` prepends a two-line header
/// ("File: ...\n===...\n"). Strip it so it never gets embedded.
fn strip_file_utils_header(raw: &str) -> String {
    let mut lines = raw.splitn(3, '\n');
    let first = lines.next().unwrap_or("");
    let second = lines.next().unwrap_or("");
    if first.starts_with("File: ") && second.starts_with("====") {
        lines.next().unwrap_or("").to_string()
    } else {
        raw.to_string()
    }
}
