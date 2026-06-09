// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

//! Moon Metis — the JupiterOS knowledge layer.
//!
//! Digests heterogeneous documents (preventivi, norme, bandi, generici) into a
//! **local, queryable, citable** knowledge base. Design principle:
//! **LLM = judgement (little), deterministic Rust = work (a lot)**. The heavy
//! ingest is pure Rust (extract → chunk → embed → index); the LLM lives at the
//! edges (nature/procedure judgement, driven by the agent; cited synthesis).

pub mod config;
pub mod extract;
pub mod chunk;
pub mod procedure;
pub mod llm;
pub mod state;
pub mod ingest;
pub mod server;
