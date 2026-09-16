// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::Utc;
use regex::Regex;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use jupiteros_shared::store::{IndexResult, MessageData, MessageStore};

use crate::blacklist::Blacklist;
use crate::error::{IoError, Result};
use crate::mail_backend::MailBackend;

// Smaller batches commit to Qdrant sooner (progress is persisted every
// BATCH_SIZE emails) and bound the blast radius of any single slow email
// during a full re-index. 500 was large enough that one pathological message
// could stall an entire batch with nothing committed.
const BATCH_SIZE: usize = 64;
const ATTACH_MAX_CHARS: usize = 2000;
// The embedding model (BGE-M3) only consumes ~512 tokens via CLS pooling,
// i.e. roughly 2-2.5k characters, so anything beyond a few thousand chars is
// discarded anyway. Keeping this tight prevents the tokenizer from chewing
// huge base64/HTML bodies, which was wedging the indexer.
const MAX_TEXT_LEN: usize = 4_000;

/// Find the largest byte index <= `i` that sits on a UTF-8 char boundary.
/// Prevents panics when slicing strings with multi-byte characters (e.g. \u{a0}).
fn floor_char_boundary(s: &str, i: usize) -> usize {
    if i >= s.len() {
        s.len()
    } else {
        let mut idx = i;
        while idx > 0 && !s.is_char_boundary(idx) {
            idx -= 1;
        }
        idx
    }
}

// ---------------------------------------------------------------------------
// Indexer state persistence
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IndexerState {
    #[serde(default)]
    pub folders: HashMap<String, FolderState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FolderState {
    pub last_uid: u32,
    pub last_indexed_at: String,
}

impl IndexerState {
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let tmp_path = path.with_extension("json.tmp");
        let content = serde_json::to_string_pretty(self)
            .map_err(|e| IoError::Other(format!("Serialize state: {e}")))?;
        std::fs::write(&tmp_path, &content)?;
        std::fs::rename(&tmp_path, path)?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Signature stripping regex patterns
// ---------------------------------------------------------------------------

// Regex builders (constructed on demand, no lazy_static needed)
fn sig_delimiter() -> Regex { Regex::new(r"(?m)^-- ?\r?$").unwrap() }
fn sig_p_iva() -> Regex { Regex::new(r"(?im)^.*P\.?\s*IVA\s*[:\s]?\d").unwrap() }
fn sig_cod_fisc() -> Regex { Regex::new(r"(?im)^.*Cod(?:ice)?\.?\s*Fisc").unwrap() }
fn sig_tel() -> Regex { Regex::new(r"(?im)^.*(?:Tel|Phone|Mob|Fax)\s*[.:+]").unwrap() }
fn sig_sede() -> Regex { Regex::new(r"(?im)^.*Sede\s+(?:legale|operativa)\s*:").unwrap() }
fn sig_web() -> Regex { Regex::new(r"(?im)^.*(?:web|sito\s*web|website)\s*:\s*(?:https?://|www\.)").unwrap() }
fn fwd_header() -> Regex { Regex::new(r"(?im)^-{2,}\s*(?:Messaggio (?:inoltrato|originale)|Forwarded message|Original Message)\s*-{2,}").unwrap() }
fn fwd_inline() -> Regex { Regex::new(r"(?im)^(?:Da|From|Inviato|Sent|Date|Data|A|To|Oggetto|Subject)\s*:").unwrap() }
fn aruba_fwd_da() -> Regex { Regex::new(r#"(?im)^Da\s+"[^"]+"\s+\S+@\S+|^Da\s+\S+@\S+"#).unwrap() }
fn aruba_fwd_oggetto() -> Regex { Regex::new(r"(?im)^Oggetto\s+.+").unwrap() }
fn aruba_inline() -> Regex { Regex::new(r"(?im)^(?:Da|A|Cc|Data|Oggetto)(?:\s|$)").unwrap() }

// ---------------------------------------------------------------------------
// EmailIndexer
// ---------------------------------------------------------------------------

pub struct EmailIndexer {
    store: Arc<MessageStore>,
    client: Box<dyn MailBackend>,
    account_name: String,
    state_path: PathBuf,
    data_dir: PathBuf,
}

impl EmailIndexer {
    pub fn new(store: Arc<MessageStore>, client: Box<dyn MailBackend>, data_dir: &Path) -> Self {
        let account_name = client.account_name().to_string();
        let state_file = if account_name.is_empty() || account_name == "primary" {
            "indexer_state.json".to_string()
        } else {
            format!("indexer_state_{}.json", account_name)
        };
        Self {
            store,
            client,
            state_path: data_dir.join(state_file),
            data_dir: data_dir.to_path_buf(),
            account_name,
        }
    }

    pub fn account_name(&self) -> &str {
        &self.account_name
    }

    /// Index emails from an IMAP folder.
    ///
    /// - `limit`: 0 = all (incremental from last UID), >0 = fetch last N
    /// - `full_reindex`: ignore last UID state
    /// - `extract_attachments`: also extract text from attachments
    pub async fn index_folder(
        &self,
        folder: &str,
        limit: usize,
        full_reindex: bool,
        extract_attachments: bool,
    ) -> Result<IndexResult> {
        let mut state = IndexerState::load(&self.state_path);
        let last_uid = if full_reindex {
            0
        } else {
            state
                .folders
                .get(folder)
                .map(|s| s.last_uid)
                .unwrap_or(0)
        };

        info!(
            "Indexing {folder}: last_uid={last_uid}, limit={limit}, full_reindex={full_reindex}"
        );

        // Get UIDs to process
        let all_uids = if limit > 0 && last_uid == 0 {
            // Fetch latest N
            let mut uids = self.client.fetch_uids_since(folder, 0).await?;
            uids.reverse();
            uids.truncate(limit);
            uids.reverse();
            uids
        } else {
            self.client.fetch_uids_since(folder, last_uid).await?
        };

        if all_uids.is_empty() {
            info!("No new emails in {folder}");
            return Ok(IndexResult::default());
        }

        info!("Found {} new UIDs in {folder}", all_uids.len());

        // Load blacklist for this account (best-effort: errors → empty list).
        let blacklist = Blacklist::load(&self.data_dir, &self.account_name);

        let mut total_result = IndexResult::default();
        let mut max_uid = last_uid;
        let mut trashed = 0usize;

        // Process in batches
        for chunk in all_uids.chunks(BATCH_SIZE) {
            let emails = self.client.fetch_emails_by_uids(folder, chunk).await?;

            let mut messages = Vec::new();
            for email in &emails {
                let uid = email.uid.unwrap_or(0);
                if uid > max_uid {
                    max_uid = uid;
                }

                // Blacklist check: skip indexing AND auto-move to Trash.
                if !blacklist.senders.is_empty() && blacklist.matches(&email.from) {
                    if uid > 0 {
                        match self.client.move_to_trash(folder, uid).await {
                            Ok(_) => {
                                trashed += 1;
                                info!(
                                    "[{}] Blacklist: moved UID {uid} from {folder} to Trash (sender: {})",
                                    self.account_name, email.from
                                );
                            }
                            Err(e) => warn!(
                                "[{}] Blacklist: failed to trash UID {uid}: {e}",
                                self.account_name
                            ),
                        }
                    }
                    continue;
                }

                // Build message ID
                let message_id = if email.message_id.is_empty() {
                    format!("email-{folder}-{uid}")
                } else {
                    email.message_id.clone()
                };

                // Prepare embedding text
                let mut embedding_text =
                    prepare_embedding_text(&email.subject, &email.body);

                // Optionally extract attachment text
                if extract_attachments && !email.attachments.is_empty() {
                    // Note: attachment text extraction would require downloading
                    // each attachment. For now, we include attachment filenames.
                    let attach_names: Vec<String> = email
                        .attachments
                        .iter()
                        .map(|a| a.filename.clone())
                        .collect();
                    if !attach_names.is_empty() {
                        embedding_text.push_str("\n[Allegati: ");
                        embedding_text.push_str(&attach_names.join(", "));
                        embedding_text.push(']');
                    }
                }

                // Truncate
                let text: String = embedding_text.chars().take(MAX_TEXT_LEN).collect();

                // Store ALL recipients (TO + CC) so contact search
                // finds emails where the contact is CC'd or 2nd+ TO.
                let all_recipients: Vec<&str> = email
                    .to
                    .iter()
                    .chain(email.cc.iter())
                    .map(|s| s.as_str())
                    .collect();
                let recipient = all_recipients.join(", ");

                // Namespace the message ID by account so the same Message-ID
                // from two different accounts doesn't collide in Qdrant.
                let scoped_id = if self.account_name.is_empty() || self.account_name == "primary" {
                    message_id
                } else {
                    format!("{}::{}", self.account_name, message_id)
                };

                messages.push(MessageData {
                    id: scoped_id,
                    text,
                    sender: email.from.clone(),
                    recipient,
                    subject: email.subject.clone(),
                    date: email.date.clone(),
                    channel: "email".to_string(),
                    folder: folder.to_string(),
                    imap_uid: if uid > 0 { Some(uid) } else { None },
                    account: self.account_name.clone(),
                });
            }

            let upsert = full_reindex;
            let batch_num = total_result.indexed + total_result.skipped + total_result.errors;
            info!(
                "{folder}: batch {}/{} — processing {} emails...",
                batch_num / BATCH_SIZE + 1,
                (all_uids.len() + BATCH_SIZE - 1) / BATCH_SIZE,
                messages.len()
            );
            match self.store.index_batch(messages, upsert).await {
                Ok(batch_result) => {
                    total_result.indexed += batch_result.indexed;
                    total_result.skipped += batch_result.skipped;
                    total_result.errors += batch_result.errors;
                    info!(
                        "{folder}: batch done — total indexed={}, skipped={}, errors={}",
                        total_result.indexed, total_result.skipped, total_result.errors
                    );
                }
                Err(e) => {
                    warn!("Batch index error in {folder}: {e}");
                    total_result.errors += chunk.len();
                }
            }

            // Save state after each batch (resumable) — ONLY for the unbounded
            // incremental/full pass (limit == 0). A bounded "recent N" pass
            // (limit > 0, e.g. the Phase-2 attachment extraction of the latest
            // 50 emails) indexes the HIGHEST UIDs without touching everything
            // below them, so persisting max_uid there would wrongly advance the
            // incremental cursor to the top of the mailbox and permanently
            // strand every older, not-yet-indexed email. The watermark means
            // "everything up to here is indexed", which is only true for the
            // contiguous limit==0 pass — so only that pass may own it.
            if limit == 0 {
                state.folders.insert(
                    folder.to_string(),
                    FolderState {
                        last_uid: max_uid,
                        last_indexed_at: Utc::now().to_rfc3339(),
                    },
                );
                if let Err(e) = state.save(&self.state_path) {
                    warn!("Failed to save indexer state: {e}");
                }
            }
        }

        info!(
            "{folder}: indexed={}, skipped={}, errors={}, trashed_by_blacklist={}",
            total_result.indexed, total_result.skipped, total_result.errors, trashed
        );

        Ok(total_result)
    }
}

// ---------------------------------------------------------------------------
// Text preparation for embedding
// ---------------------------------------------------------------------------

/// Prepare email text for embedding: strip signature, extract forward, boost subject.
pub fn prepare_embedding_text(subject: &str, body: &str) -> String {
    let stripped = strip_signature(body);
    let (wrapper, forwarded) = extract_forward_content(&stripped);

    // Decide what to use as embedding text
    let main_text = if !forwarded.is_empty() {
        // If the wrapper is just a signature or very short, discard it
        let wrapper_clean = strip_signature(wrapper);
        if wrapper_clean.len() < 50 || has_signature_patterns(wrapper_clean) {
            forwarded.to_string()
        } else {
            format!("{wrapper_clean}\n\n{forwarded}")
        }
    } else {
        stripped.to_string()
    };

    // Boost subject by placing it at top and bottom
    if subject.is_empty() || subject == "(nessun oggetto)" {
        main_text
    } else {
        format!("{subject}\n\n{main_text}\n\n{subject}")
    }
}

/// Strip email signature from text.
fn strip_signature(text: &str) -> &str {
    // 1. RFC 3676 delimiter: "-- "
    if let Some(m) = sig_delimiter().find(text) {
        let head = &text[..m.start()];
        // Guardrail: un delimiter in testa non deve azzerare il corpo.
        if !head.trim().is_empty() {
            return head;
        }
    }

    // 2. Heuristic: scan last 40% of lines for signature patterns.
    //    Serve un minimo di righe: certi mailer emettono il text/plain come
    //    UNA riga unica da migliaia di caratteri, e "l'ultimo 40%" sarebbe
    //    l'intero corpo — un "Web: www…" nella firma CITATA nel thread lo
    //    cancellava per intero (l'indice conservava solo l'oggetto).
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() < 4 {
        return text;
    }

    // Mai partire dalla riga 0: il corpo non può essere tutta firma.
    let start_line = ((lines.len() as f64 * 0.6) as usize).max(1);
    let patterns = [
        sig_p_iva(),
        sig_cod_fisc(),
        sig_tel(),
        sig_sede(),
        sig_web(),
    ];

    for (i, line) in lines.iter().enumerate().skip(start_line) {
        for pat in &patterns {
            if pat.is_match(line) {
                // Found a signature pattern — return text up to this line
                let byte_offset: usize = lines[..i].iter().map(|l| l.len() + 1).sum();
                let head = &text[..floor_char_boundary(text, byte_offset)];
                // Guardrail: se il taglio svuota il testo, meglio la firma
                // nell'indice che nessun contenuto.
                if head.trim().is_empty() {
                    return text;
                }
                return head;
            }
        }
    }

    text
}

/// Check if text contains signature-like patterns.
fn has_signature_patterns(text: &str) -> bool {
    let patterns = [
        sig_p_iva(),
        sig_cod_fisc(),
        sig_tel(),
        sig_sede(),
        sig_web(),
    ];
    patterns.iter().any(|p| p.is_match(text))
}

/// Extract forward content from email body.
/// Returns (wrapper_text, forwarded_body).
fn extract_forward_content(text: &str) -> (&str, &str) {
    // Pattern 1: explicit separator (Gmail/Outlook style)
    if let Some(m) = fwd_header().find(text) {
        let wrapper = &text[..m.start()];
        let rest = &text[m.end()..];
        // Skip inline headers after separator
        let body_start = skip_inline_headers(rest);
        return (wrapper, &rest[body_start..]);
    }

    // Pattern 2: Aruba webmail — detect "Da" + "Oggetto" within first 10 lines
    let lines: Vec<&str> = text.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        if aruba_fwd_da().is_match(line) {
            // Check if "Oggetto" appears within next 10 lines
            let check_end = (i + 10).min(lines.len());
            for j in i..check_end {
                if aruba_fwd_oggetto().is_match(lines[j]) {
                    // Found Aruba-style forward
                    let wrapper_end: usize = lines[..i].iter().map(|l| l.len() + 1).sum();
                    let body_line = j + 1;
                    if body_line < lines.len() {
                        let body_start: usize =
                            lines[..body_line].iter().map(|l| l.len() + 1).sum();
                        return (
                            &text[..floor_char_boundary(text, wrapper_end)],
                            &text[floor_char_boundary(text, body_start)..],
                        );
                    }
                }
            }
        }
    }

    (text, "")
}

/// Skip inline forward headers (Da:, A:, Oggetto:, Data:, etc.)
/// Returns byte offset where the actual body starts.
fn skip_inline_headers(text: &str) -> usize {
    let inline_re = aruba_inline();
    let mut offset = 0;
    let mut found_headers = false;

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if found_headers {
                offset += line.len() + 1;
                break;
            }
            offset += line.len() + 1;
            continue;
        }

        if inline_re.is_match(trimmed) || fwd_inline().is_match(trimmed) {
            found_headers = true;
            offset += line.len() + 1;
        } else if found_headers {
            // First non-header line after headers = body start
            break;
        } else {
            break;
        }
    }

    offset.min(text.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_signature_single_line_not_wiped() {
        // Certi mailer emettono il corpo come UNA riga unica: l'euristica
        // firma non deve azzerarlo anche se nel thread citato compare
        // "Web: www…" (caso reale: fornitore evtaurus, UID 20034/20066).
        let text = "Dear all, follow-up on the order. Quoted signature: Web: www.emotion-team.com and more text after";
        assert_eq!(strip_signature(text), text);
    }

    #[test]
    fn test_strip_signature_few_lines_not_wiped() {
        let text = "Riga uno del corpo
Tel: +39 000 — dentro una citazione
Riga tre";
        assert_eq!(strip_signature(text), text);
    }

    #[test]
    fn test_strip_signature_rfc3676() {
        let text = "Hello world\n\nSome content\n-- \nEdoardo Mancinelli\nCEO";
        let stripped = strip_signature(text);
        assert_eq!(stripped, "Hello world\n\nSome content\n");
    }

    #[test]
    fn test_strip_signature_piva() {
        let mut text = String::new();
        for i in 0..20 {
            text.push_str(&format!("Line {i} of the email body\n"));
        }
        text.push_str("P.IVA 12345678901\nVia Roma 1\n");
        let stripped = strip_signature(&text);
        assert!(!stripped.contains("P.IVA"));
    }

    #[test]
    fn test_extract_forward_gmail() {
        let text = "FYI\n\n---------- Messaggio inoltrato ----------\nDa: Mario <mario@test.com>\nA: Luigi\nOggetto: Test\n\nOriginal content here";
        let (wrapper, forwarded) = extract_forward_content(text);
        assert!(wrapper.contains("FYI"));
        assert!(forwarded.contains("Original content"));
    }

    #[test]
    fn test_prepare_embedding_text_subject_boost() {
        let result = prepare_embedding_text("Important Topic", "Body text here");
        assert!(result.starts_with("Important Topic"));
        assert!(result.ends_with("Important Topic"));
    }
}
