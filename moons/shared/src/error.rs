// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

use thiserror::Error;

#[derive(Error, Debug)]
pub enum SharedError {
    #[error("Qdrant error: {0}")]
    Qdrant(String),

    #[error("Embedding error: {0}")]
    Embedding(String),

    #[error("File parsing error: {0}")]
    FileParsing(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, SharedError>;
