// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

//! Local-OCR smoke test. `#[ignore]` by default: it downloads the ocrs models
//! (~12 MB, needs network) and needs a scanned (image-only) PDF fixture passed
//! via the `OCR_TEST_PDF` env var, plus poppler's `pdftoppm` on PATH.
//!
//! Run manually:
//!   OCR_TEST_PDF=/tmp/scanned_test.pdf cargo test -p moon-metis-rs --test ocr_smoke -- --ignored --nocapture

use std::path::PathBuf;

use moon_metis_rs::{extract, ocr, setup};

#[tokio::test]
#[ignore]
async fn ocr_extracts_text_from_scanned_pdf() {
    let pdf = std::env::var("OCR_TEST_PDF").expect("set OCR_TEST_PDF to a scanned PDF path");
    let pdf = PathBuf::from(pdf);

    let model_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data/ocr-model");
    setup::ensure_ocr_models(&model_dir)
        .await
        .expect("download ocrs models");
    ocr::init(&model_dir, 300, 100).expect("init OCR engine");
    assert!(ocr::is_ready());

    // Low-level: OCR straight to per-page text.
    let pages = ocr::ocr_pdf(&pdf).expect("ocr_pdf");
    let joined = pages.join("\n").to_uppercase();
    println!("--- OCR output ---\n{joined}\n------------------");
    assert!(joined.contains("FATTURA"), "atteso 'FATTURA' nel testo OCR");
    assert!(joined.contains("12345"), "atteso '12345' nel testo OCR");

    // End-to-end through extract(): scanned PDF should now carry OCR text.
    let ex = extract::extract(&pdf).expect("extract");
    assert!(ex.is_scanned, "il PDF immagine deve restare is_scanned=true");
    assert!(ex.ocr_applied, "ocr_applied deve essere true dopo l'OCR");
    assert!(ex.total_chars() > 0, "le pagine devono avere testo dopo l'OCR");
}
