// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

use thiserror::Error;

#[derive(Error, Debug)]
pub enum EuropaError {
    #[error("Telegram error: {0}")]
    Telegram(String),

    #[error("Adapter error: {0}")]
    Adapter(String),

    #[error("Config error: {0}")]
    Config(String),

    #[error("Store error: {0}")]
    Store(#[from] jupiteros_shared::error::SharedError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Not authorized: {0}")]
    NotAuthorized(String),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, EuropaError>;
