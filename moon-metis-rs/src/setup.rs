// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

//! First-run setup for the local OCR models.
//!
//! Mirrors Moon Io's `ensure_onnx_model` pattern: check for the model files plus
//! a version marker, and download them (once) if missing. The two `ocrs` models
//! (`text-detection.rten` + `text-recognition.rten`) total ~12 MB, so — unlike
//! Io's 570 MB embedding model — we fetch them fully into memory rather than
//! streaming. Zero external requirements: everything lands on first run.

use std::path::Path;

use anyhow::{bail, Context};
use tracing::info;

/// Base URL for the official `ocrs` model bucket.
const OCRS_MODELS_BASE: &str = "https://ocrs-models.s3-accelerate.amazonaws.com";

/// `ocrs` model file names (also the names we store them under on disk).
pub const DETECTION_MODEL: &str = "text-detection.rten";
pub const RECOGNITION_MODEL: &str = "text-recognition.rten";

/// Marker file written after a successful download. Bump this string to force a
/// re-download (e.g. when migrating to a newer ocrs model revision).
const OCR_MODEL_VERSION_MARKER: &str = "ocrs-v1";

/// Ensure both ocrs models exist in `model_dir` and match the expected version.
/// Downloads them on first run; a no-op once present.
pub async fn ensure_ocr_models(model_dir: &Path) -> anyhow::Result<()> {
    let detection_path = model_dir.join(DETECTION_MODEL);
    let recognition_path = model_dir.join(RECOGNITION_MODEL);
    let marker_path = model_dir.join(".ocr_model_version");

    let marker_matches = std::fs::read_to_string(&marker_path)
        .map(|s| s.trim() == OCR_MODEL_VERSION_MARKER)
        .unwrap_or(false);

    if detection_path.exists() && recognition_path.exists() && marker_matches {
        info!(
            "OCR models present at {} (version {})",
            model_dir.display(),
            OCR_MODEL_VERSION_MARKER
        );
        return Ok(());
    }

    // Stale/partial state: wipe and re-download cleanly.
    if detection_path.exists() || recognition_path.exists() {
        info!("Detected stale/partial OCR models — re-downloading {OCR_MODEL_VERSION_MARKER}");
        let _ = std::fs::remove_file(&detection_path);
        let _ = std::fs::remove_file(&recognition_path);
        let _ = std::fs::remove_file(&marker_path);
    }

    info!("Downloading ocrs models (~12 MB) to {}...", model_dir.display());
    std::fs::create_dir_all(model_dir).context("Create OCR model directory")?;

    let client = http_client()?;
    for name in [DETECTION_MODEL, RECOGNITION_MODEL] {
        let url = format!("{OCRS_MODELS_BASE}/{name}");
        let dest = model_dir.join(name);
        info!("  downloading {name}...");
        download_file(&client, &url, &dest)
            .await
            .with_context(|| format!("Download {name}"))?;
    }

    // Version marker written last — only after both files landed successfully.
    std::fs::write(&marker_path, OCR_MODEL_VERSION_MARKER)
        .context("Write OCR model version marker")?;

    info!("OCR models ready at {}", model_dir.display());
    Ok(())
}

fn http_client() -> anyhow::Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .user_agent("moon-metis/0.1")
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()?)
}

/// Download a (small) file fully into memory, then write it to disk.
async fn download_file(client: &reqwest::Client, url: &str, dest: &Path) -> anyhow::Result<()> {
    let resp = client.get(url).send().await?;
    if !resp.status().is_success() {
        bail!("HTTP {} for {}", resp.status(), url);
    }
    let bytes = resp.bytes().await?;
    std::fs::write(dest, &bytes)?;
    info!("  {} ({:.1} MB)", dest.display(), bytes.len() as f64 / 1_048_576.0);
    Ok(())
}
