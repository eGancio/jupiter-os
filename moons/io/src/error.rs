// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

use thiserror::Error;

#[derive(Error, Debug)]
pub enum IoError {
    #[error("IMAP error: {0}")]
    Imap(String),

    #[error("SMTP error: {0}")]
    Smtp(String),

    #[error("CalDAV error: {0}")]
    CalDav(String),

    #[error("Email parse error: {0}")]
    Parse(String),

    #[error("Signature error: {0}")]
    Signature(String),

    #[error("Config error: {0}")]
    Config(String),

    #[error("Store error: {0}")]
    Store(#[from] jupiteros_shared::error::SharedError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, IoError>;
