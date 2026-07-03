// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

//! Deterministic quality gate for the compile path.
//!
//! Compiling a document into a structured wiki is expensive (LLM reasoning) and,
//! worse, *amplifies* whatever it's fed: garbage compiled becomes structured
//! garbage that pollutes the graph and search. So before any compile we run
//! cheap, LLM-free signals to decide whether a document is worth it.
//!
//! This is the muscle behind `metis_assess`, and the precondition of
//! `metis_compile` (Fase 2).

use serde::Serialize;

use crate::extract::Extraction;

/// Below this many extractable characters there isn't enough to compile.
const MIN_CHARS: usize = 200;
/// Fraction of non-whitespace characters that must be alphanumeric. Lower means
/// likely binary garbage or dirty extraction noise (symbol soup).
const MIN_ALPHA_RATIO: f64 = 0.55;
/// `file_utils::extract_text` clamps non-PDF input at 25k chars (see extract.rs).
/// At-or-above this for a non-PDF means the compile would only see the start.
const NONPDF_TRUNCATION_CAP: usize = 25_000;

#[derive(Debug, Clone, Serialize)]
pub struct Signals {
    pub format: String,
    pub is_scanned: bool,
    /// True when the text was produced by OCR (scanned PDF). May contain errors.
    pub ocr_applied: bool,
    pub total_chars: usize,
    pub page_count: u32,
    /// Fraction of non-whitespace chars that are alphanumeric (0.0–1.0).
    pub alpha_ratio: f64,
    /// True when a non-PDF likely hit the 25k extraction cap.
    pub possibly_truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Assessment {
    pub compilable: bool,
    pub reason: String,
    pub signals: Signals,
}

/// Fraction of non-whitespace chars that are alphanumeric. Flags binary garbage
/// or extraction noise without needing an LLM.
fn alpha_ratio(text: &str) -> f64 {
    let mut non_ws = 0usize;
    let mut alnum = 0usize;
    for c in text.chars() {
        if c.is_whitespace() {
            continue;
        }
        non_ws += 1;
        if c.is_alphanumeric() {
            alnum += 1;
        }
    }
    if non_ws == 0 {
        return 0.0;
    }
    alnum as f64 / non_ws as f64
}

/// Assess an already-extracted document. Pure function over the extraction —
/// no I/O, no LLM — so it's trivially testable and fast.
pub fn assess(extraction: &Extraction) -> Assessment {
    let total_chars = extraction.total_chars();
    let full_text = extraction
        .pages
        .iter()
        .map(|p| p.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let ratio = alpha_ratio(&full_text);
    let possibly_truncated = extraction.format != "pdf" && total_chars >= NONPDF_TRUNCATION_CAP;

    let signals = Signals {
        format: extraction.format.clone(),
        is_scanned: extraction.is_scanned,
        ocr_applied: extraction.ocr_applied,
        total_chars,
        page_count: extraction.page_count(),
        alpha_ratio: ratio,
        possibly_truncated,
    };

    // A scanned PDF that OCR could not recover is unusable. A scanned PDF that
    // *was* OCR'd falls through to the normal thresholds below (with a caveat
    // appended to the reason, since OCR text can contain recognition errors).
    let ocr_note = if extraction.ocr_applied {
        " (testo da OCR: può contenere errori di riconoscimento)"
    } else {
        ""
    };

    let (compilable, reason) = if extraction.is_scanned && !extraction.ocr_applied {
        (
            false,
            "PDF scansionato (immagine): nessun testo estraibile, OCR non disponibile o senza esito."
                .to_string(),
        )
    } else if total_chars < MIN_CHARS {
        (
            false,
            format!(
                "Troppo corto ({total_chars} caratteri < {MIN_CHARS}): non c'è abbastanza \
                 contenuto da compilare in modo utile."
            ),
        )
    } else if ratio < MIN_ALPHA_RATIO {
        (
            false,
            format!(
                "Testo poco alfabetico (ratio {ratio:.2} < {MIN_ALPHA_RATIO}): probabile \
                 estrazione sporca o file binario/garbage."
            ),
        )
    } else if possibly_truncated {
        (
            true,
            format!(
                "Compilabile, ma ATTENZIONE: file non-PDF troncato a 25k caratteri — il compile \
                 vedrà solo l'inizio del documento.{ocr_note}"
            ),
        )
    } else {
        (true, format!("Compilabile.{ocr_note}"))
    };

    Assessment {
        compilable,
        reason,
        signals,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::{Extraction, Page};

    fn ext(format: &str, is_scanned: bool, text: &str) -> Extraction {
        Extraction {
            format: format.to_string(),
            is_scanned,
            ocr_applied: false,
            pages: vec![Page {
                number: 1,
                text: text.to_string(),
            }],
        }
    }

    #[test]
    fn rejects_scanned() {
        let a = assess(&ext("pdf", true, ""));
        assert!(!a.compilable);
    }

    #[test]
    fn rejects_too_short() {
        let a = assess(&ext("pdf", false, "ciao"));
        assert!(!a.compilable);
    }

    #[test]
    fn rejects_symbol_soup() {
        let soup = "§#@%^&*<>{}[]|\\/~`±€¶•".repeat(40);
        let a = assess(&ext("pdf", false, &soup));
        assert!(!a.compilable, "alpha_ratio {}", a.signals.alpha_ratio);
    }

    #[test]
    fn accepts_real_prose() {
        let prose = "Articolo 5. Le colonnine di ricarica trasmettono i dati di vendita \
                     all'Agenzia delle Entrate secondo il tracciato CRE. "
            .repeat(10);
        let a = assess(&ext("pdf", false, &prose));
        assert!(a.compilable, "{}", a.reason);
    }
}
