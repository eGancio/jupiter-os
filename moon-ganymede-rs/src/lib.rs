// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

//! Moon Ganymede — the JupiterOS "LLM Wiki": a file-based operational memory.
//!
//! Plain `.md` files on disk with YAML frontmatter. No vectors, no Qdrant.
//! Claude *reads* the wiki with the built-in `Read`/`Grep`/`Glob` tools; it
//! *writes* only through this Moon, so every mutation goes through frontmatter
//! validation, tag canonicalisation, atomic writes and automatic re-indexing.

pub mod config;
pub mod fact;
pub mod frontmatter;
pub mod index;
pub mod server;
pub mod setup;
pub mod taxonomy;
