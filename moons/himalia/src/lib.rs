// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

//! Moon Himalia — web research for LLMs via the Tavily API.
//!
//! Himalia upgrades the deep-research skill's web step: instead of the native
//! `WebSearch`/`WebFetch` (snippets + raw HTML), it returns LLM-clean results
//! and full-text extraction — `web_search` (discovery) and `web_extract` (VERIFY).
//!
//! Quando è configurato un token **openapi.it** (`OPENAPI_TOKEN`), Himalia espone
//! anche i tool **imprese** (bilanci/visure ufficiali di SRL/SpA), così la deep
//! research può incrociare il web con i dati societari depositati. Senza token,
//! quei tool tornano un errore chiaro — esattamente come i tool web senza Tavily.

pub mod config;
pub mod ledger;
pub mod openapi;
pub mod server;
pub mod tavily;
