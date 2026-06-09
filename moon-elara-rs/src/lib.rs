// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

//! Moon Elara — external high-signal sources for market/idea-validation research.
//!
//! Two tools, only *defensible* sources (no closed-platform scraping):
//! - `news_search`   → GDELT DOC 2.0 (free, no key): real-time news.
//! - `reddit_search` → Reddit official API (OAuth app-only): community signal.
//!
//! Live querying only. The deep-research skill distils the output into a citable
//! insight — the raw firehose must NEVER be ingested into Metis.

pub mod config;
pub mod gdelt;
pub mod reddit;
pub mod server;
