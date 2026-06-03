// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

//! Auto-setup: ONNX model download + Qdrant download & auto-start.
//!
//! Called once at startup to ensure all dependencies are ready
//! before the MCP server begins accepting connections.
//! Zero external requirements — everything is downloaded on first run.

use std::path::Path;

use anyhow::{bail, Context};
use futures_util::StreamExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tracing::info;

// ---------------------------------------------------------------------------
// HuggingFace model download — BGE-M3 INT8 quantized
// ---------------------------------------------------------------------------

const HF_REPO: &str = "gpahal/bge-m3-onnx-int8";
const HF_BASE: &str = "https://huggingface.co";

/// Marker file written after a successful download. Bumping this string forces
/// a re-download of the model (useful when migrating between model versions).
const MODEL_VERSION_MARKER: &str = "bge-m3-int8-v1";

/// Ensure `model.onnx` and `tokenizer.json` exist in `model_dir` and correspond
/// to the current expected model version. If a previous version is found
/// (e.g. the old MiniLM), it is wiped and re-downloaded.
pub async fn ensure_onnx_model(model_dir: &Path) -> anyhow::Result<()> {
    let model_path = model_dir.join("model.onnx");
    let tokenizer_path = model_dir.join("tokenizer.json");
    let marker_path = model_dir.join(".model_version");

    let marker_matches = std::fs::read_to_string(&marker_path)
        .map(|s| s.trim() == MODEL_VERSION_MARKER)
        .unwrap_or(false);

    if model_path.exists() && tokenizer_path.exists() && marker_matches {
        info!(
            "ONNX model present at {} (version {})",
            model_dir.display(),
            MODEL_VERSION_MARKER
        );
        return Ok(());
    }

    // If the model files exist but version marker is missing/stale, wipe them
    // so we re-download the new model cleanly.
    if model_path.exists() || tokenizer_path.exists() {
        info!(
            "Detected stale embedding model — migrating to {}",
            MODEL_VERSION_MARKER
        );
        let _ = std::fs::remove_file(&model_path);
        let _ = std::fs::remove_file(&tokenizer_path);
        let _ = std::fs::remove_file(&marker_path);
    }

    info!("Downloading BGE-M3 INT8 ONNX model from HuggingFace...");
    std::fs::create_dir_all(model_dir).context("Create model directory")?;

    let client = http_client()?;

    // Small file: tokenizer (17 MB but quick).
    let tokenizer_url = format!("{}/{}/resolve/main/tokenizer.json", HF_BASE, HF_REPO);
    info!("Downloading tokenizer.json (~17 MB)...");
    download_streaming(&client, &tokenizer_url, &tokenizer_path)
        .await
        .context("Download tokenizer.json")?;

    // Large file: quantized ONNX model (570 MB).
    let model_url = format!("{}/{}/resolve/main/model_quantized.onnx", HF_BASE, HF_REPO);
    info!("Downloading model_quantized.onnx (~570 MB, this may take several minutes)...");
    download_streaming(&client, &model_url, &model_path)
        .await
        .context("Download model_quantized.onnx")?;

    // Write version marker last — only after both files landed successfully.
    std::fs::write(&marker_path, MODEL_VERSION_MARKER)
        .context("Write model version marker")?;

    info!("BGE-M3 INT8 ready at {}", model_dir.display());
    Ok(())
}

// ---------------------------------------------------------------------------
// Qdrant — download binary + auto-start
// ---------------------------------------------------------------------------

const QDRANT_VERSION: &str = "1.17.0";

/// Platform-specific archive name on GitHub releases.
fn qdrant_archive_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "qdrant-x86_64-pc-windows-msvc.zip"
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        "qdrant-x86_64-unknown-linux-musl.tar.gz"
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "qdrant-aarch64-apple-darwin.tar.gz"
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        "qdrant-x86_64-apple-darwin.tar.gz"
    } else {
        "unsupported"
    }
}

/// Binary name inside the archive.
fn qdrant_binary_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "qdrant.exe"
    } else {
        "qdrant"
    }
}

/// Ensure Qdrant is running. Downloads the binary on first run,
/// then starts it as a child process.
pub async fn ensure_qdrant(qdrant_url: &str, data_dir: &Path) -> anyhow::Result<()> {
    // Already running? Nothing to do.
    if is_reachable(qdrant_url).await {
        info!("Qdrant already running at {}", qdrant_url);
        return Ok(());
    }

    info!("Qdrant not reachable at {} — setting up...", qdrant_url);

    let qdrant_dir = data_dir.join("qdrant");
    let qdrant_bin = qdrant_dir.join(qdrant_binary_name());
    let qdrant_storage = qdrant_dir.join("storage");
    let qdrant_snapshots = qdrant_dir.join("snapshots");

    std::fs::create_dir_all(&qdrant_storage).context("Create Qdrant storage dir")?;
    std::fs::create_dir_all(&qdrant_snapshots).context("Create Qdrant snapshots dir")?;

    // Download binary if missing
    if !qdrant_bin.exists() {
        download_qdrant(&qdrant_dir).await?;
    }

    // Start Qdrant as a background process
    info!("Starting Qdrant from {}...", qdrant_bin.display());
    start_qdrant(&qdrant_bin, &qdrant_storage, &qdrant_snapshots)?;

    // Wait for it to be ready
    wait_for_qdrant(qdrant_url).await?;

    Ok(())
}

/// Download and extract the Qdrant binary from GitHub releases.
async fn download_qdrant(qdrant_dir: &Path) -> anyhow::Result<()> {
    let archive = qdrant_archive_name();
    if archive == "unsupported" {
        bail!("Qdrant auto-download not supported on this platform");
    }

    let version =
        std::env::var("QDRANT_VERSION").unwrap_or_else(|_| QDRANT_VERSION.to_string());
    let url = format!(
        "https://github.com/qdrant/qdrant/releases/download/v{}/{}",
        version, archive
    );

    std::fs::create_dir_all(qdrant_dir).context("Create Qdrant directory")?;

    let archive_path = qdrant_dir.join(archive);
    let client = http_client()?;

    info!("Downloading Qdrant v{} ({})...", version, archive);
    download_streaming(&client, &url, &archive_path)
        .await
        .context("Download Qdrant archive")?;

    // Extract
    info!("Extracting {}...", archive);
    if archive.ends_with(".zip") {
        extract_zip(&archive_path, qdrant_dir)?;
    } else {
        extract_tar_gz(&archive_path, qdrant_dir)?;
    }

    // Clean up archive
    let _ = std::fs::remove_file(&archive_path);

    // Make binary executable (Unix only)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let bin = qdrant_dir.join(qdrant_binary_name());
        let mut perms = std::fs::metadata(&bin)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&bin, perms)?;
    }

    let bin_path = qdrant_dir.join(qdrant_binary_name());
    if !bin_path.exists() {
        bail!(
            "Extraction completed but {} not found in {}",
            qdrant_binary_name(),
            qdrant_dir.display()
        );
    }

    info!("Qdrant binary ready at {}", bin_path.display());
    Ok(())
}

/// Extract a .zip archive (Windows).
fn extract_zip(archive: &Path, dest: &Path) -> anyhow::Result<()> {
    let file = std::fs::File::open(archive)?;
    let mut zip = zip::ZipArchive::new(file)?;

    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        let name = entry.name().to_string();

        if entry.is_dir() {
            continue;
        }

        // Flatten: extract just the filename
        let file_name = Path::new(&name)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or(name.clone());

        let out_path = dest.join(&file_name);
        let mut out_file = std::fs::File::create(&out_path)?;
        std::io::copy(&mut entry, &mut out_file)?;
        info!("  Extracted: {}", out_path.display());
    }

    Ok(())
}

/// Extract a .tar.gz archive (Linux/macOS).
fn extract_tar_gz(archive: &Path, dest: &Path) -> anyhow::Result<()> {
    let file = std::fs::File::open(archive)?;
    let gz = flate2::read::GzDecoder::new(file);
    let mut tar = tar::Archive::new(gz);

    for entry in tar.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.to_path_buf();

        if entry.header().entry_type().is_dir() {
            continue;
        }

        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        if file_name.is_empty() {
            continue;
        }

        let out_path = dest.join(&file_name);
        let mut out_file = std::fs::File::create(&out_path)?;
        std::io::copy(&mut entry, &mut out_file)?;
        info!("  Extracted: {}", out_path.display());
    }

    Ok(())
}

/// Start Qdrant as a detached background process.
fn start_qdrant(
    binary: &Path,
    storage_dir: &Path,
    snapshots_dir: &Path,
) -> anyhow::Result<()> {
    std::process::Command::new(binary)
        .env("QDRANT__STORAGE__STORAGE_PATH", storage_dir)
        .env("QDRANT__STORAGE__SNAPSHOTS_PATH", snapshots_dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("Failed to start Qdrant process")?;

    info!("Qdrant process spawned");
    Ok(())
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Build a reusable HTTP client.
fn http_client() -> anyhow::Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .user_agent("moon-io/0.1")
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()?)
}

/// Download a small file fully into memory, then write to disk.
async fn download_small(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
) -> anyhow::Result<()> {
    let resp = client.get(url).send().await?;
    if !resp.status().is_success() {
        bail!("HTTP {} for {}", resp.status(), url);
    }
    let bytes = resp.bytes().await?;
    std::fs::write(dest, &bytes)?;
    info!("  {} ({} bytes)", dest.display(), bytes.len());
    Ok(())
}

/// Download a large file with streaming (low RAM usage).
async fn download_streaming(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
) -> anyhow::Result<()> {
    let resp = client.get(url).send().await?;
    if !resp.status().is_success() {
        bail!("HTTP {} for {}", resp.status(), url);
    }

    let total = resp.content_length().unwrap_or(0);
    let mut file = tokio::fs::File::create(dest).await?;
    let mut stream = resp.bytes_stream();
    let mut downloaded: u64 = 0;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk).await?;
        downloaded += chunk.len() as u64;

        if total > 0 && downloaded % (5 * 1024 * 1024) < chunk.len() as u64 {
            let pct = (downloaded * 100) / total;
            info!(
                "  {:.1} / {:.1} MB ({}%)",
                downloaded as f64 / 1_048_576.0,
                total as f64 / 1_048_576.0,
                pct
            );
        }
    }

    file.flush().await?;
    info!(
        "  {} ({:.1} MB)",
        dest.display(),
        downloaded as f64 / 1_048_576.0
    );
    Ok(())
}

/// TCP probe to check if Qdrant's gRPC port is accepting connections.
async fn is_reachable(qdrant_url: &str) -> bool {
    let addr = qdrant_url
        .trim_start_matches("http://")
        .trim_start_matches("https://");

    match tokio::time::timeout(
        std::time::Duration::from_secs(2),
        TcpStream::connect(addr),
    )
    .await
    {
        Ok(Ok(_)) => true,
        _ => false,
    }
}

/// Poll until Qdrant is reachable (max 30 s).
async fn wait_for_qdrant(qdrant_url: &str) -> anyhow::Result<()> {
    info!("Waiting for Qdrant...");
    for i in 0..30 {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        if is_reachable(qdrant_url).await {
            info!("Qdrant ready ({}s)", i + 1);
            return Ok(());
        }
    }
    bail!("Qdrant did not start within 30 seconds")
}
