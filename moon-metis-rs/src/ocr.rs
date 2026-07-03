// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

//! Local OCR for scanned/image PDFs.
//!
//! Pipeline: `PDF → pdftoppm (rasterize per page) → ocrs (detect + recognize)`.
//!
//! - **Engine**: `ocrs`, which runs on the pure-Rust `rten` runtime — independent
//!   of the `ort`/ONNX stack used for embeddings, CPU-only, no system deps.
//! - **Rasterization**: poppler's `pdftoppm`, already required by the
//!   `pdftotext` fallback in [`crate::extract`].
//!
//! The `OcrEngine` is loaded once and reused (it is not `Sync`, so it lives
//! behind a `Mutex`; ingest is sequential, so there is no contention). Call
//! [`init`] at startup, then [`ocr_pdf`] per file.

use std::path::Path;
use std::sync::{Mutex, OnceLock};

use anyhow::{anyhow, Context, Result};
use ocrs::{ImageSource, OcrEngine, OcrEngineParams};
use rten::Model;
use tracing::warn;

use crate::setup::{DETECTION_MODEL, RECOGNITION_MODEL};

/// The loaded engine plus its rasterization settings. Held once and reused.
struct Ocr {
    engine: Mutex<OcrEngine>,
    dpi: u32,
    max_pages: usize,
}

static ENGINE: OnceLock<Ocr> = OnceLock::new();

/// Build the OCR engine from the two `.rten` models in `model_dir` and store it
/// (with its rasterization settings) for reuse. Idempotent: a second call is a
/// no-op. Returns an error if the models are missing or fail to load.
pub fn init(model_dir: &Path, dpi: u32, max_pages: usize) -> Result<()> {
    if ENGINE.get().is_some() {
        return Ok(());
    }

    let detection_path = model_dir.join(DETECTION_MODEL);
    let recognition_path = model_dir.join(RECOGNITION_MODEL);

    let detection_model = Model::load_file(&detection_path)
        .map_err(|e| anyhow!("Caricamento {}: {e}", detection_path.display()))?;
    let recognition_model = Model::load_file(&recognition_path)
        .map_err(|e| anyhow!("Caricamento {}: {e}", recognition_path.display()))?;

    let engine = OcrEngine::new(OcrEngineParams {
        detection_model: Some(detection_model),
        recognition_model: Some(recognition_model),
        ..Default::default()
    })
    .map_err(|e| anyhow!("Inizializzazione OcrEngine: {e}"))?;

    // If another thread won the race, just drop ours — both are equivalent.
    let _ = ENGINE.set(Ocr {
        engine: Mutex::new(engine),
        dpi,
        max_pages,
    });
    Ok(())
}

/// Whether the OCR engine is loaded and ready.
pub fn is_ready() -> bool {
    ENGINE.get().is_some()
}

/// OCR a (scanned) PDF into one text string per page, using the DPI / page-cap
/// configured at [`init`].
///
/// Rasterizes up to `max_pages` pages via `pdftoppm`, then runs ocrs on each page
/// image. If the PDF has more pages than the cap the extra pages are skipped with
/// a `warn!` (no silent truncation).
pub fn ocr_pdf(path: &Path) -> Result<Vec<String>> {
    let ocr = ENGINE
        .get()
        .ok_or_else(|| anyhow!("OCR non inizializzato"))?;
    let dpi = ocr.dpi;
    let max_pages = ocr.max_pages;

    let total_pages = pdf_page_count(path);
    if let Some(total) = total_pages {
        if total > max_pages {
            warn!(
                "OCR {}: {total} pagine ma cap a {max_pages} (METIS_OCR_MAX_PAGES) — \
                 le restanti {} non saranno indicizzate",
                path.display(),
                total - max_pages
            );
        }
    }

    let tmp = tempfile::tempdir().context("Creazione cartella temporanea OCR")?;
    let prefix = tmp.path().join("page");

    // pdftoppm -png -r <dpi> -l <last> <pdf> <prefix>  ->  <prefix>-N.png
    let mut cmd = std::process::Command::new("pdftoppm");
    cmd.arg("-png").arg("-r").arg(dpi.to_string());
    cmd.arg("-l").arg(max_pages.to_string());
    cmd.arg(path).arg(&prefix);
    let out = cmd
        .output()
        .map_err(|e| anyhow!("pdftoppm non eseguibile (poppler installato?): {e}"))?;
    if !out.status.success() {
        return Err(anyhow!(
            "pdftoppm fallito: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }

    // Collect the produced PNGs and order them by their numeric page suffix
    // (pdftoppm zero-pads to the page-count width, so lexical sort is unsafe).
    let mut pages: Vec<(u32, std::path::PathBuf)> = std::fs::read_dir(tmp.path())
        .context("Lettura cartella temporanea OCR")?
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            let n = page_number_from_filename(&p)?;
            Some((n, p))
        })
        .collect();
    pages.sort_by_key(|(n, _)| *n);

    if pages.is_empty() {
        return Err(anyhow!("pdftoppm non ha prodotto immagini"));
    }

    let engine = ocr
        .engine
        .lock()
        .map_err(|_| anyhow!("Mutex OCR avvelenato"))?;

    let mut texts = Vec::with_capacity(pages.len());
    for (n, img_path) in pages {
        match ocr_image(&engine, &img_path) {
            Ok(text) => texts.push(text),
            Err(e) => {
                // One bad page shouldn't sink the whole document.
                warn!("OCR pagina {n} di {}: {e}", path.display());
                texts.push(String::new());
            }
        }
    }

    Ok(texts)
}

/// Run ocrs on a single image file, returning its recognized text.
fn ocr_image(engine: &OcrEngine, img_path: &Path) -> Result<String> {
    let img = image::open(img_path)
        .with_context(|| format!("Apertura immagine {}", img_path.display()))?
        .into_rgb8();
    let (w, h) = img.dimensions();
    let source = ImageSource::from_bytes(img.as_raw(), (w, h))
        .map_err(|e| anyhow!("ImageSource: {e}"))?;
    let input = engine
        .prepare_input(source)
        .map_err(|e| anyhow!("prepare_input: {e}"))?;
    engine.get_text(&input).map_err(|e| anyhow!("get_text: {e}"))
}

/// Best-effort page count via poppler's `pdfinfo`. `None` if unavailable.
fn pdf_page_count(path: &Path) -> Option<usize> {
    let out = std::process::Command::new("pdfinfo").arg(path).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines()
        .find_map(|l| l.strip_prefix("Pages:"))
        .and_then(|v| v.trim().parse::<usize>().ok())
}

/// Extract the page number from a `pdftoppm` output filename like `page-12.png`.
fn page_number_from_filename(p: &Path) -> Option<u32> {
    if p.extension().and_then(|e| e.to_str()) != Some("png") {
        return None;
    }
    let stem = p.file_stem()?.to_str()?;
    let digits = stem.rsplit('-').next()?;
    digits.parse::<u32>().ok()
}
