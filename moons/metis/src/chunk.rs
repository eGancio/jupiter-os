// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

//! Structure-aware chunking.
//!
//! Each page is split into structural sections (per the procedure's regex), and
//! every section is windowed into ~`target_chars` chunks with overlap. Section
//! labels (e.g. "Art. 12") are carried across page breaks so a chunk that
//! continues an article on the next page still cites the right section. Page
//! numbers come straight from the page-aware extraction.

use jupiteros_shared::DocChunk;

use crate::extract::Extraction;
use crate::procedure::Procedure;

/// Build the chunk list for a document.
pub fn chunk_document(
    extraction: &Extraction,
    procedure: &Procedure,
    doc_id: &str,
    source_path: &str,
    title: &str,
    content_hash: &str,
) -> Vec<DocChunk> {
    let mut out = Vec::new();
    let mut chunk_index: u32 = 0;
    // The section label carried across page boundaries.
    let mut carried = String::new();

    for page in &extraction.pages {
        if page.text.trim().is_empty() {
            continue;
        }

        let segments: Vec<(String, &str)> = match &procedure.section_regex {
            Some(re) => split_sections(&page.text, re, &mut carried),
            None => vec![(carried.clone(), page.text.as_str())],
        };

        for (label, seg_text) in segments {
            for chunk_text in window(seg_text, procedure.target_chars, procedure.overlap_chars) {
                out.push(DocChunk {
                    doc_id: doc_id.to_string(),
                    source_path: source_path.to_string(),
                    title: title.to_string(),
                    nature: procedure.nature.clone(),
                    page: page.number,
                    section_path: label.clone(),
                    chunk_index,
                    text: chunk_text,
                    content_hash: content_hash.to_string(),
                });
                chunk_index += 1;
            }
        }
    }

    out
}

/// Split a page into `(section_label, text)` segments at the section regex.
/// `carried` holds the label active at the page start and is updated to the
/// last label seen (so it carries onto the next page).
fn split_sections<'a>(
    text: &'a str,
    re: &regex::Regex,
    carried: &mut String,
) -> Vec<(String, &'a str)> {
    let matches: Vec<(usize, String)> = re
        .find_iter(text)
        .map(|m| (m.start(), normalize_label(m.as_str())))
        .collect();

    if matches.is_empty() {
        return vec![(carried.clone(), text)];
    }

    let mut segs: Vec<(String, &str)> = Vec::new();

    // Preamble before the first match keeps the carried label.
    let first_start = matches[0].0;
    if first_start > 0 && !text[..first_start].trim().is_empty() {
        segs.push((carried.clone(), &text[..first_start]));
    }

    for i in 0..matches.len() {
        let start = matches[i].0;
        let end = if i + 1 < matches.len() {
            matches[i + 1].0
        } else {
            text.len()
        };
        let label = matches[i].1.clone();
        segs.push((label.clone(), &text[start..end]));
        *carried = label;
    }

    segs
}

/// Normalise a matched header into a compact, single-line section label.
fn normalize_label(raw: &str) -> String {
    let collapsed = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.chars().take(60).collect()
}

/// Window `text` into ~`target` char chunks with `overlap`, backing off to a
/// whitespace boundary so words aren't cut. UTF-8 safe (operates on chars).
fn window(text: &str, target: usize, overlap: usize) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    if n == 0 {
        return vec![];
    }
    if n <= target {
        let s = text.trim();
        return if s.is_empty() {
            vec![]
        } else {
            vec![s.to_string()]
        };
    }

    let mut out = Vec::new();
    let mut start = 0usize;
    let min_back = (target as f32 * 0.6) as usize;

    while start < n {
        let mut end = (start + target).min(n);
        if end < n {
            // Back off to the last whitespace within [start+min_back, end).
            let lo = start + min_back;
            let mut b = end;
            while b > lo && !chars[b - 1].is_whitespace() {
                b -= 1;
            }
            if b > lo {
                end = b;
            }
        }

        let chunk: String = chars[start..end].iter().collect();
        let trimmed = chunk.trim();
        if !trimmed.is_empty() {
            out.push(trimmed.to_string());
        }

        if end >= n {
            break;
        }
        start = if end > overlap { end - overlap } else { end };
    }

    out
}
