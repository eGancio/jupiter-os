// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

//! Moon Himalia — web research for LLMs via the Tavily API.
//!
//! Himalia upgrades the deep-research skill's web step: instead of the native
//! `WebSearch`/`WebFetch` (snippets + raw HTML), it returns LLM-clean results
//! and full-text extraction. Two tools only — `web_search` (discovery) and
//! `web_extract` (fetch a page's clean text for the VERIFY step).

pub mod config;
pub mod server;
pub mod tavily;
