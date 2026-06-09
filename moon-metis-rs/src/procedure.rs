// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

//! Built-in ingest **procedures**, one per document *nature*.
//!
//! A procedure is the "define once per type, execute on every file of that type"
//! recipe: how to split the document structurally and how big the chunks are.
//! The LLM/agent picks the nature; the deterministic Rust pipeline applies the
//! procedure. New custom procedures (persisted) are a later stage.

use regex::Regex;

/// The recipe applied deterministically to every file of a given nature.
#[derive(Clone)]
pub struct Procedure {
    pub nature: String,
    pub description: String,
    /// Regex marking the START of a structural section. Its match text becomes
    /// the chunk's `section_path` (e.g. "Art. 12"). `None` = flat windowing.
    pub section_regex: Option<Regex>,
    /// Approximate chunk size in characters (~1500 chars ≈ 400–500 tokens,
    /// comfortably under the BGE-M3 512-token cap).
    pub target_chars: usize,
    /// Overlap in characters between consecutive chunks.
    pub overlap_chars: usize,
}

/// Natures with a dedicated built-in procedure.
pub const KNOWN_NATURES: &[&str] = &["generico", "norma", "bando", "preventivo"];

/// Resolve a procedure by nature (falls back to "generico").
pub fn get(nature: &str) -> Procedure {
    match nature.trim().to_lowercase().as_str() {
        "norma" | "normativa" | "legge" | "decreto" => norma(),
        "bando" | "avviso" | "gara" => bando(),
        "preventivo" | "offerta" | "fattura" => preventivo(),
        _ => generico(),
    }
}

fn generico() -> Procedure {
    Procedure {
        nature: "generico".into(),
        description: "Documento generico: finestratura a paragrafi con overlap.".into(),
        section_regex: None,
        target_chars: 1500,
        overlap_chars: 200,
    }
}

fn norma() -> Procedure {
    // Articoli / Titoli / Capi a inizio riga.
    let re = Regex::new(
        r"(?im)^[ \t]*(art(?:icolo)?\.?\s*\d+[a-z\-]*|titolo\s+[ivxlcdm]+|capo\s+[ivxlcdm]+)\b",
    )
    .expect("regex norma valida");
    Procedure {
        nature: "norma".into(),
        description: "Normativa: split per Articolo/Titolo/Capo, section_path citabile.".into(),
        section_regex: Some(re),
        target_chars: 1500,
        overlap_chars: 150,
    }
}

fn bando() -> Procedure {
    // Articoli oppure sezioni numerate "N. Titolo".
    let re = Regex::new(
        r"(?im)^[ \t]*(art(?:icolo)?\.?\s*\d+[a-z\-]*|\d{1,2}\.\s+\S.{2,70})\s*$",
    )
    .expect("regex bando valida");
    Procedure {
        nature: "bando".into(),
        description: "Bando/avviso: split per articoli e sezioni numerate.".into(),
        section_regex: Some(re),
        target_chars: 1500,
        overlap_chars: 200,
    }
}

fn preventivo() -> Procedure {
    Procedure {
        nature: "preventivo".into(),
        description: "Preventivo/offerta: documento breve, finestratura fitta.".into(),
        section_regex: None,
        target_chars: 1200,
        overlap_chars: 100,
    }
}
