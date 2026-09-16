// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use regex::Regex;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::error::{IoError, Result};

/// Per-account blacklist of sender email addresses.
///
/// Senders are stored normalised (lowercase, address-only). Matching is
/// substring-insensitive: an entry like "spam.example.com" matches any
/// sender whose address contains that string.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Blacklist {
    #[serde(default)]
    pub senders: Vec<String>,
}

impl Blacklist {
    /// Path to the blacklist file for a given account.
    pub fn path_for(data_dir: &Path, account: &str) -> PathBuf {
        let filename = if account.is_empty() || account == "primary" {
            "blacklist.json".to_string()
        } else {
            format!("blacklist_{}.json", account)
        };
        data_dir.join(filename)
    }

    /// Load the blacklist for an account. Missing file → empty list.
    pub fn load(data_dir: &Path, account: &str) -> Self {
        let path = Self::path_for(data_dir, account);
        match std::fs::read_to_string(&path) {
            Ok(content) => match serde_json::from_str::<Self>(&content) {
                Ok(mut bl) => {
                    // Normalize on load (defensive)
                    bl.senders = bl
                        .senders
                        .into_iter()
                        .map(|s| s.trim().to_lowercase())
                        .filter(|s| !s.is_empty())
                        .collect::<HashSet<_>>()
                        .into_iter()
                        .collect();
                    bl.senders.sort();
                    bl
                }
                Err(e) => {
                    warn!("Failed to parse blacklist {path:?}: {e}");
                    Self::default()
                }
            },
            Err(_) => Self::default(),
        }
    }

    /// Atomically save the blacklist for an account.
    pub fn save(&self, data_dir: &Path, account: &str) -> Result<()> {
        let path = Self::path_for(data_dir, account);
        let tmp = path.with_extension("json.tmp");
        let content = serde_json::to_string_pretty(self)
            .map_err(|e| IoError::Other(format!("Serialize blacklist: {e}")))?;
        std::fs::write(&tmp, &content)?;
        std::fs::rename(&tmp, &path)?;
        debug!("Saved blacklist with {} entries to {path:?}", self.senders.len());
        Ok(())
    }

    /// Add a sender. Returns true if added, false if already present.
    pub fn add(&mut self, sender: &str) -> bool {
        let normalized = normalize(sender);
        if normalized.is_empty() {
            return false;
        }
        if self.senders.iter().any(|s| s == &normalized) {
            return false;
        }
        self.senders.push(normalized);
        self.senders.sort();
        true
    }

    /// Remove a sender. Returns true if removed.
    pub fn remove(&mut self, sender: &str) -> bool {
        let normalized = normalize(sender);
        let before = self.senders.len();
        self.senders.retain(|s| s != &normalized);
        before != self.senders.len()
    }

    /// Check if a sender (raw "Name <addr@host>" or just "addr@host") is blacklisted.
    /// Uses substring matching on the normalized address — so a blacklist entry
    /// "spam@" or "@badprovider.com" both work.
    pub fn matches(&self, sender_raw: &str) -> bool {
        let addr = extract_address(sender_raw).to_lowercase();
        if addr.is_empty() {
            return false;
        }
        self.senders.iter().any(|s| {
            addr == *s || addr.contains(s)
        })
    }
}

/// Extract the bare address from a "Name <email@host>" string.
/// Falls back to the trimmed input if no `<...>` is present.
fn extract_address(raw: &str) -> String {
    let re = Regex::new(r"<([^>]+)>").unwrap();
    if let Some(cap) = re.captures(raw) {
        cap[1].trim().to_lowercase()
    } else {
        raw.trim().to_lowercase()
    }
}

/// Normalize a sender entry for storage: lowercase + extract address part if wrapped.
fn normalize(raw: &str) -> String {
    extract_address(raw)
}
