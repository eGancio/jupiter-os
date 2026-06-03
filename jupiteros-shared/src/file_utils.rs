// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

use std::io::Read;
use std::path::Path;

use calamine::Reader as XlsxReader;
use tracing::warn;

use crate::error::{Result, SharedError};

/// Extract text content from a file.
///
/// Supports: PDF, DOCX, XLSX, CSV, TXT, MD, JSON, XML, HTML, YAML, LOG, INI.
/// Replaces Python's pdfplumber + python-docx + openpyxl.
pub fn extract_text(path: &Path, max_chars: usize) -> Result<String> {
    let max_chars = max_chars.clamp(500, 25_000);

    if !path.exists() || !path.is_file() {
        return Err(SharedError::FileParsing(format!(
            "File not found: {}",
            path.display()
        )));
    }

    let file_size = std::fs::metadata(path)
        .map(|m| m.len())
        .unwrap_or(0);
    let file_name = path.file_name().unwrap_or_default().to_string_lossy();
    let ext = path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();

    let text = match ext.as_str() {
        "pdf" => extract_pdf(path, max_chars)?,
        "docx" => extract_docx(path)?,
        "xlsx" | "xls" => extract_xlsx(path)?,
        "zip" => extract_zip(path, max_chars)?,
        "csv" | "txt" | "md" | "json" | "xml" | "html" | "yaml" | "yml" | "log" | "ini"
        | "cfg" | "conf" => extract_plaintext(path)?,
        _ => {
            // Try as plaintext
            extract_plaintext(path).map_err(|_| {
                SharedError::FileParsing(format!(
                    "Unsupported format: .{ext}. Supported: pdf, docx, xlsx, zip, csv, txt, md, json, xml, html, yaml"
                ))
            })?
        }
    };

    // Build header
    let header = format!("File: {} ({} bytes, .{})\n{}\n", file_name, file_size, ext, "=".repeat(50));
    let available = max_chars.saturating_sub(header.len());

    if text.len() <= available {
        Ok(format!("{header}{text}"))
    } else {
        let truncated: String = text.chars().take(available.saturating_sub(50)).collect();
        Ok(format!(
            "{header}{truncated}\n\n[...truncated, total file size: {} bytes]",
            file_size
        ))
    }
}

/// Extract text from PDF using pdf-extract (handles compressed streams, font encoding, CMAP).
fn extract_pdf(path: &Path, _max_chars: usize) -> Result<String> {
    let text = pdf_extract::extract_text(path)
        .map_err(|e| SharedError::FileParsing(format!("PDF extraction failed: {e}")))?;

    if text.trim().is_empty() {
        return Err(SharedError::FileParsing(
            "PDF has no extractable text (may be scanned/image-based)".into(),
        ));
    }

    Ok(text)
}

/// Extract text from DOCX (ZIP with XML inside).
fn extract_docx(path: &Path) -> Result<String> {
    let file = std::fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| SharedError::FileParsing(format!("Invalid DOCX: {e}")))?;

    let mut doc_xml = String::new();
    {
        let mut entry = archive
            .by_name("word/document.xml")
            .map_err(|e| SharedError::FileParsing(format!("No document.xml in DOCX: {e}")))?;
        entry.read_to_string(&mut doc_xml)?;
    }

    // Parse XML and extract text from <w:t> tags
    let mut text = String::new();
    let mut reader = quick_xml::Reader::from_str(&doc_xml);
    let mut in_text = false;
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(quick_xml::events::Event::Start(ref e)) | Ok(quick_xml::events::Event::Empty(ref e)) => {
                let local = e.local_name();
                if local.as_ref() == b"t" {
                    in_text = true;
                } else if local.as_ref() == b"p" && !text.is_empty() {
                    // New paragraph
                    text.push('\n');
                }
            }
            Ok(quick_xml::events::Event::Text(ref e)) if in_text => {
                if let Ok(t) = e.unescape() {
                    text.push_str(&t);
                }
            }
            Ok(quick_xml::events::Event::End(ref e)) => {
                if e.local_name().as_ref() == b"t" {
                    in_text = false;
                }
            }
            Ok(quick_xml::events::Event::Eof) => break,
            Err(e) => {
                warn!("DOCX XML parse error: {e}");
                break;
            }
            _ => {}
        }
        buf.clear();
    }

    Ok(text)
}

/// Extract text from XLSX using calamine.
fn extract_xlsx(path: &Path) -> Result<String> {
    let mut workbook: calamine::Xlsx<std::io::BufReader<std::fs::File>> =
        calamine::open_workbook(path)
            .map_err(|e| SharedError::FileParsing(format!("Invalid XLSX: {e}")))?;

    let mut text = String::new();
    let sheet_names: Vec<String> = workbook.sheet_names().to_vec();

    for name in sheet_names {
        if let Ok(range) = workbook.worksheet_range(&name) {
            text.push_str(&format!("--- Sheet: {} ---\n", name));
            for row in range.rows() {
                let cells: Vec<String> = row
                    .iter()
                    .map(|cell: &calamine::Data| match cell {
                        calamine::Data::Empty => String::new(),
                        calamine::Data::String(s) => s.clone(),
                        calamine::Data::Float(f) => f.to_string(),
                        calamine::Data::Int(i) => i.to_string(),
                        calamine::Data::Bool(b) => b.to_string(),
                        calamine::Data::DateTime(dt) => format!("{dt}"),
                        _ => String::new(),
                    })
                    .collect();
                text.push_str(&cells.join("\t"));
                text.push('\n');
            }
        }
    }

    Ok(text)
}

/// Read plaintext file (UTF-8 with fallback).
fn extract_plaintext(path: &Path) -> Result<String> {
    let data = std::fs::read(path)?;
    Ok(String::from_utf8_lossy(&data).to_string())
}

/// Extract content from a ZIP archive: lists all files and inlines text from
/// supported entries (txt/md/json/xml/html/csv/log/pdf/docx/xlsx). Other
/// entries are listed by name + size.
fn extract_zip(path: &Path, max_chars: usize) -> Result<String> {
    let file = std::fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| SharedError::FileParsing(format!("Invalid ZIP: {e}")))?;

    let mut out = String::new();
    out.push_str(&format!("ZIP contains {} entries:\n", archive.len()));

    // Pass 1: index summary
    let mut entries: Vec<(String, u64, bool)> = Vec::with_capacity(archive.len());
    for i in 0..archive.len() {
        let entry = archive
            .by_index(i)
            .map_err(|e| SharedError::FileParsing(format!("ZIP entry {i}: {e}")))?;
        let name = entry.name().to_string();
        let size = entry.size();
        let is_dir = entry.is_dir();
        entries.push((name, size, is_dir));
    }
    for (name, size, is_dir) in &entries {
        if *is_dir {
            out.push_str(&format!("  [DIR ] {}\n", name));
        } else {
            out.push_str(&format!("  [FILE] {} ({} bytes)\n", name, size));
        }
    }
    out.push_str(&"=".repeat(50));
    out.push('\n');

    // Pass 2: extract text from supported entries
    let temp_dir = std::env::temp_dir().join(format!("jupiteros-zip-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&temp_dir);

    for i in 0..archive.len() {
        if out.len() >= max_chars {
            out.push_str("\n[...output truncated]\n");
            break;
        }

        let mut entry = match archive.by_index(i) {
            Ok(e) => e,
            Err(_) => continue,
        };
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_string();
        let inner_ext = Path::new(&name)
            .extension()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase();

        let supported = matches!(
            inner_ext.as_str(),
            "txt" | "md" | "json" | "xml" | "html" | "yaml" | "yml" | "log" | "ini"
            | "cfg" | "conf" | "csv" | "pdf" | "docx" | "xlsx" | "xls"
        );
        if !supported {
            continue;
        }

        // Read entry into a temp file (calamine/pdf-extract need a real path)
        let temp_path = temp_dir.join(format!("entry-{}.{}", i, inner_ext));
        let mut buf = Vec::new();
        if entry.read_to_end(&mut buf).is_err() {
            continue;
        }
        if std::fs::write(&temp_path, &buf).is_err() {
            continue;
        }

        out.push_str(&format!("\n--- {} ---\n", name));
        match inner_ext.as_str() {
            "pdf" => match extract_pdf(&temp_path, max_chars) {
                Ok(t) => out.push_str(&t),
                Err(e) => out.push_str(&format!("[PDF extraction failed: {e}]")),
            },
            "docx" => match extract_docx(&temp_path) {
                Ok(t) => out.push_str(&t),
                Err(e) => out.push_str(&format!("[DOCX extraction failed: {e}]")),
            },
            "xlsx" | "xls" => match extract_xlsx(&temp_path) {
                Ok(t) => out.push_str(&t),
                Err(e) => out.push_str(&format!("[XLSX extraction failed: {e}]")),
            },
            _ => {
                out.push_str(&String::from_utf8_lossy(&buf));
            }
        }
        let _ = std::fs::remove_file(&temp_path);
        out.push('\n');
    }

    let _ = std::fs::remove_dir_all(&temp_dir);
    Ok(out)
}
