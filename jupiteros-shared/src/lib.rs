// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

pub mod error;
pub mod store;
pub mod doc_store;
pub mod embeddings;
pub mod file_utils;

pub use error::{SharedError, Result};
pub use store::MessageStore;
pub use doc_store::{DocChunk, DocHit, DocStore, DocSummary};
pub use embeddings::OnnxEmbedding;
