// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use jupiteros_shared::store::{EmptyDatePoint, MessageData, MessageStore};
use rmcp::handler::server::tool::{Parameters, ToolRouter};
use rmcp::model::*;
use rmcp::{tool, tool_handler, tool_router, ServerHandler};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Mutex;
use tracing::{info, warn};

use crate::blacklist::Blacklist;
use crate::calendar_client::{CalendarClient, CreateEventParams, EventResponse};
use crate::config::Config;
use crate::email_client::SendEmailParams as ClientSendParams;
use crate::email_indexer::EmailIndexer;
use crate::mail_backend::{build_backend, MailBackend};
use crate::oauth::AccessTokenCache;
use crate::signature;

// ===========================================================================
// Background task tracking
// ===========================================================================

#[derive(Clone, Serialize)]
pub struct IndexTaskStatus {
    pub running: bool,
    pub phase: String,        // "reset", "indexing", "idle"
    pub folder: String,
    pub indexed: usize,
    pub skipped: usize,
    pub errors: usize,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub message: String,
}

impl Default for IndexTaskStatus {
    fn default() -> Self {
        Self {
            running: false,
            phase: "idle".into(),
            folder: String::new(),
            indexed: 0,
            skipped: 0,
            errors: 0,
            started_at: None,
            finished_at: None,
            message: "Nessuna operazione in corso.".into(),
        }
    }
}

// ===========================================================================
// Server struct
// ===========================================================================

/// Moon Io MCP Server.
///
/// Exposes 21 tools for email operations (IMAP/SMTP/CalDAV).
pub struct MoonIoServer {
    pub config: Config,
    pub store: Arc<MessageStore>,
    pub index_lock: Arc<Mutex<()>>,
    pub task_status: Arc<Mutex<IndexTaskStatus>>,
    pub oauth_cache: AccessTokenCache,
    tool_router: ToolRouter<Self>,
}

// ===========================================================================
// Parameter structs — doc comments become JSON schema descriptions for the LLM
// ===========================================================================

#[derive(Serialize, Deserialize, JsonSchema)]
struct ListEmailsParams {
    /// Max results (1-200, default 20)
    limit: Option<u32>,
    /// Filter by contact name or email
    contact: Option<String>,
    /// Pagination offset (default 0)
    offset: Option<u32>,
    /// Account name to filter by (e.g. "primary", "gmail"). Omit for all accounts.
    account: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
struct ReadRecentEmailsParams {
    /// Max emails to read (1-5, default 3)
    limit: Option<u32>,
    /// Filter by contact name or email
    contact: Option<String>,
    /// Account name to filter by (e.g. "primary", "gmail"). Omit for all accounts.
    account: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
struct ReadEmailParams {
    /// UID of the email to read
    email_uid: u32,
    /// IMAP folder (default: "INBOX")
    folder: Option<String>,
    /// Max characters to return (500-25000, default 25000)
    max_chars: Option<u32>,
    /// Account name (e.g. "primary", "gmail"). Defaults to the primary account.
    account: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
struct SearchEmailsParams {
    /// Search query (semantic search)
    query: String,
    /// Max results (1-30, default 10)
    limit: Option<u32>,
    /// Account name to filter by (e.g. "primary", "gmail"). Omit to search across all accounts.
    account: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
struct SearchByContactParams {
    /// Contact name or email address
    contact: String,
    /// Max results (1-50, default 20)
    limit: Option<u32>,
    /// Account name to filter by (e.g. "primary", "gmail"). Omit to search across all accounts.
    account: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
struct SendEmailToolParams {
    /// Recipient email address(es), comma-separated
    to: String,
    /// Email subject
    subject: String,
    /// Email body (plain text, NO firma — la firma viene aggiunta automaticamente)
    body: String,
    /// CC addresses, comma-separated
    cc: Option<String>,
    /// Display name for From header
    from_name: Option<String>,
    /// Comma-separated file paths for attachments
    attachments: Option<String>,
    /// Signature name to use (default: "default")
    signature_name: Option<String>,
    /// Account to send from (e.g. "primary", "gmail"). Defaults to the primary account.
    account: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
struct ReplyEmailParams {
    /// UID of the email to reply to
    email_uid: u32,
    /// Reply body (plain text, NO firma)
    body: String,
    /// CC addresses, comma-separated
    cc: Option<String>,
    /// IMAP folder (default: "INBOX")
    folder: Option<String>,
    /// Comma-separated file paths for attachments
    attachments: Option<String>,
    /// Signature name (default: "default")
    signature_name: Option<String>,
    /// Account where the original email lives (e.g. "primary", "gmail"). Defaults to the primary account.
    account: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
struct GetAttachmentsParams {
    /// UID of the email
    email_uid: u32,
    /// IMAP folder (default: "INBOX")
    folder: Option<String>,
    /// Account (e.g. "primary", "gmail"). Defaults to the primary account.
    account: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
struct SaveAttachmentParams {
    /// UID of the email
    email_uid: u32,
    /// Index of the attachment (0-based)
    attachment_index: u32,
    /// IMAP folder (default: "INBOX")
    folder: Option<String>,
    /// Directory to save the file (default: ~/Downloads)
    save_dir: Option<String>,
    /// Account (e.g. "primary", "gmail"). Defaults to the primary account.
    account: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
struct DeleteEmailParams {
    /// UID of the email to delete
    email_uid: u32,
    /// IMAP folder where the email lives (default: "INBOX")
    folder: Option<String>,
    /// Account (e.g. "primary", "gmail"). If omitted, inferred from the index.
    account: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
struct BlacklistAddParams {
    /// Sender to add: either "addr@host" or "Name <addr@host>". Substring matches too: "@badprovider.com".
    sender: String,
    /// Account (e.g. "primary", "gmail"). Defaults to the primary account.
    account: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
struct BlacklistRemoveParams {
    /// Sender to remove (same format used when adding).
    sender: String,
    /// Account (e.g. "primary", "gmail"). Defaults to the primary account.
    account: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
struct BlacklistListParams {
    /// Account (e.g. "primary", "gmail"). Omit to list all accounts.
    account: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
struct DeleteEmailsBulkParams {
    /// Filter by sender (substring match on FROM header, e.g. "news@booking.com" or "@spammers.com").
    sender: Option<String>,
    /// Filter by subject (substring match).
    subject_contains: Option<String>,
    /// Filter by date: YYYY-MM-DD. Matches emails strictly before this date.
    older_than: Option<String>,
    /// IMAP folder to search (default: "INBOX").
    folder: Option<String>,
    /// Account name (e.g. "primary", "gmail"). Defaults to the primary account.
    account: Option<String>,
    /// Max emails to delete in one run (safety limit, default 1000).
    limit: Option<u32>,
    /// If true (DEFAULT), only count matches and show preview — does NOT delete.
    /// Set to false explicitly after user confirmation.
    dry_run: Option<bool>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
struct BlacklistSweepParams {
    /// Account name (e.g. "primary", "gmail"). Defaults to the primary account.
    account: Option<String>,
    /// IMAP folder to sweep (default: "INBOX").
    folder: Option<String>,
    /// If true (DEFAULT), only count matches per blacklisted sender — does NOT delete.
    /// Set to false explicitly after user confirmation.
    dry_run: Option<bool>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
struct CheckBouncedParams {
    /// IMAP folder to check (default: "INBOX")
    folder: Option<String>,
    /// Max emails to check (1-50, default 20)
    limit: Option<u32>,
    /// Account (e.g. "primary", "gmail"). Defaults to the primary account.
    account: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
struct IndexEmailsParams {
    /// IMAP folder to index (default: "INBOX")
    folder: Option<String>,
    /// Limit how many to index (0 = all new)
    limit: Option<u32>,
    /// Force full re-index (re-processes all emails, updates existing records)
    full_reindex: Option<bool>,
    /// Account to index (e.g. "primary", "gmail"). Defaults to all configured accounts.
    account: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
struct RepairEmptyDatesParams {
    /// Account to repair (e.g. "aruba", "gmail"). REQUIRED so we know which
    /// backend to query for the IMAP INTERNALDATE / Gmail internalDate.
    account: String,
    /// If true, only count matching points and return a preview — does NOT
    /// modify Qdrant. Default false (actually repair).
    dry_run: Option<bool>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
struct IndexMessageParams {
    /// Unique message ID
    message_id: String,
    /// Email body text
    text: String,
    /// Sender email
    sender: Option<String>,
    /// Recipient email
    recipient: Option<String>,
    /// Email subject
    subject: Option<String>,
    /// Date (ISO format)
    date: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
struct ListCalendarEventsParams {
    /// Start date (YYYY-MM-DD, default: today)
    start_date: Option<String>,
    /// End date (YYYY-MM-DD, default: start + 30 days)
    end_date: Option<String>,
    /// Max events (1-100, default 20)
    limit: Option<u32>,
    /// Account whose calendar to query (default: primary).
    account: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
struct CreateCalendarEventParams {
    /// Event title/summary
    summary: String,
    /// Start date/time (ISO 8601 or YYYY-MM-DD)
    start: String,
    /// End date/time (ISO 8601 or YYYY-MM-DD)
    end: String,
    /// Event description
    description: Option<String>,
    /// Event location
    location: Option<String>,
    /// Attendees email addresses, comma-separated
    attendees: Option<String>,
    /// Account whose calendar to use (default: primary).
    account: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
struct RespondCalendarEventParams {
    /// UID of the calendar event
    event_uid: String,
    /// Optional comment with your response
    comment: Option<String>,
    /// Account whose calendar to use (default: primary).
    account: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
struct DeleteCalendarEventParams {
    /// UID of the calendar event to delete
    event_uid: String,
    /// Account whose calendar to use (default: primary).
    account: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
struct ExtractFileTextParams {
    /// Path to the file
    file_path: String,
    /// Max characters to extract (default 15000)
    max_chars: Option<u32>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
struct SetSignatureParams {
    /// Your display name
    name: String,
    /// Your email address
    email: String,
    /// Your job title/role
    role: Option<String>,
    /// Company name
    company: Option<String>,
    /// Phone number
    phone: Option<String>,
    /// URL to profile photo
    photo_url: Option<String>,
    /// Accent color (hex, e.g. "#2b5797")
    color: Option<String>,
    /// Signature style: "modern", "classic", "minimal"
    style: Option<String>,
    /// Name to save the signature as (default: "default")
    signature_name: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
struct GetSignatureParams {
    /// Name of the signature (default: "default")
    signature_name: Option<String>,
}

// ===========================================================================
// Tool implementations
// ===========================================================================

#[tool_router]
impl MoonIoServer {
    pub fn new(
        config: Config,
        store: Arc<MessageStore>,
        task_status: Arc<Mutex<IndexTaskStatus>>,
        index_lock: Arc<Mutex<()>>,
    ) -> Self {
        Self {
            config,
            store,
            index_lock,
            task_status,
            oauth_cache: AccessTokenCache::new(),
            tool_router: Self::tool_router(),
        }
    }

    /// Resolve an account by name (or default if None), and return an EmailClient.
    fn email_client(&self, account: Option<&str>) -> Result<Box<dyn MailBackend>, String> {
        let acc = self
            .config
            .resolve_account(account)
            .ok_or_else(|| match account {
                Some(n) => format!("Account '{n}' non configurato."),
                None => "Nessun account email configurato.".to_string(),
            })?;
        Ok(build_backend(
            acc.clone(),
            self.config.signature_dir.clone(),
            self.oauth_cache.clone(),
        ))
    }

    /// All configured account names (e.g. ["primary", "gmail"]).
    fn account_names(&self) -> Vec<String> {
        self.config.accounts.iter().map(|a| a.name.clone()).collect()
    }

    /// Resolve an email UID to (account, stored MessageData) — multi-account safe.
    ///
    /// - If `account_param` is given: looks up only in that account. Returns
    ///   `(account, None)` if the UID isn't in the index (caller can still try IMAP).
    /// - If `account_param` is `None`: searches across all accounts.
    ///     - 0 matches → `(default_account, None)`.
    ///     - 1 match → `(matched_account, Some(data))`.
    ///     - >1 matches → returns an error (caller must specify `account`).
    async fn resolve_uid(
        &self,
        uid: u32,
        account_param: Option<&str>,
    ) -> Result<(String, Option<jupiteros_shared::store::MessageData>), String> {
        let account_filter = account_param.filter(|s| !s.is_empty());
        if let Some(acc) = account_filter {
            // Validate the account name first
            let canonical = self
                .config
                .account(acc)
                .map(|a| a.name.clone())
                .ok_or_else(|| format!("Account '{acc}' non configurato."))?;
            let stored = self
                .store
                .get_by_imap_uid_in_account(uid, &canonical)
                .await
                .map_err(|e| format!("Store lookup: {e}"))?;
            return Ok((canonical, stored));
        }

        // No account specified: search across all
        let matches = self
            .store
            .find_all_by_imap_uid(uid)
            .await
            .map_err(|e| format!("Store lookup: {e}"))?;
        match matches.len() {
            0 => {
                let default = self
                    .config
                    .default_account()
                    .map(|a| a.name.clone())
                    .ok_or_else(|| "Nessun account configurato.".to_string())?;
                Ok((default, None))
            }
            1 => {
                let m = matches.into_iter().next().unwrap();
                let acc = if m.account.is_empty() {
                    self.config
                        .default_account()
                        .map(|a| a.name.clone())
                        .unwrap_or_default()
                } else {
                    m.account.clone()
                };
                Ok((acc, Some(m)))
            }
            _ => {
                let accs: Vec<String> = matches.iter().map(|m| m.account.clone()).collect();
                Err(format!(
                    "UID {uid} ambiguo: presente in più account ({}). Specifica `account` esplicitamente.",
                    accs.join(", ")
                ))
            }
        }
    }

    fn calendar_client(&self, account: Option<&str>) -> Option<CalendarClient> {
        let acc = self.config.resolve_account(account)?;
        if acc.has_caldav() {
            Some(CalendarClient::new(
                &acc.caldav_url,
                &acc.caldav_username,
                &acc.caldav_password,
            ))
        } else {
            None
        }
    }

    // ===================================================================
    // EMAIL READING
    // ===================================================================

    /// List email headers (date, sender, subject, preview). Fast, metadata only.
    #[tool(
        name = "list_emails",
        description = "List email headers (date, sender, subject, preview). Fast, metadata only. Max 200. Use for overview."
    )]
    async fn list_emails(
        &self,
        params: Parameters<ListEmailsParams>,
    ) -> Result<String, String> {
        let p = params.0;
        let limit = p.limit.unwrap_or(20).clamp(1, 200) as usize;
        let offset = p.offset.unwrap_or(0) as usize;
        let contact_ref = p.contact.as_deref().filter(|s| !s.is_empty());
        let account_filter = p.account.as_deref().filter(|s| !s.is_empty());

        // Account filter is applied server-side on Qdrant, so we can use
        // `limit`/`offset` as-is — pagination works correctly.
        let headers = self
            .store
            .list_messages(limit, contact_ref, offset, account_filter)
            .await
            .map_err(|e| format!("List emails: {e}"))?;

        if headers.is_empty() {
            return Ok("Nessuna email trovata.".to_string());
        }

        let total_label = format!(
            "Showing {}-{} emails",
            offset + 1,
            offset + headers.len()
        );
        let mut lines = vec![
            total_label,
            format!(
                "{:<7} | {:<18} | {:<40} | {:<40} | {}",
                "UID", "Data", "Da", "A", "Oggetto"
            ),
            "-".repeat(155),
        ];

        let show_account = self.config.accounts.len() > 1;
        for h in &headers {
            let date = format_date_short(&h.date);
            let sender: String = h.sender.chars().take(40).collect();
            let recipient: String = h.recipient.chars().take(40).collect();
            let subject: String = h.subject.chars().take(60).collect();
            let uid_str = h.imap_uid.map(|u| u.to_string()).unwrap_or_else(|| "-".to_string());
            let folder_tag = if h.folder.is_empty() || h.folder == "INBOX" {
                String::new()
            } else if h.folder.contains("Sent") || h.folder.contains("sent") {
                " [S]".to_string()
            } else {
                format!(" [{}]", h.folder.chars().take(10).collect::<String>())
            };
            let account_tag = if show_account && !h.account.is_empty() {
                format!(" [{}]", h.account)
            } else {
                String::new()
            };
            lines.push(format!(
                "{:<7} | {:<18} | {:<40} | {:<40} | {}{}{}",
                uid_str, date, sender, recipient, subject, folder_tag, account_tag
            ));
        }

        Ok(lines.join("\n"))
    }

    /// Read recent emails with full body text. Max 5.
    #[tool(
        name = "read_recent_emails",
        description = "Read recent emails with full body text. Max 5. Use ONLY when user asks to READ content."
    )]
    async fn read_recent_emails(
        &self,
        params: Parameters<ReadRecentEmailsParams>,
    ) -> Result<String, String> {
        let p = params.0;
        let limit = p.limit.unwrap_or(3).clamp(1, 5) as usize;
        let contact_ref = p.contact.as_deref().filter(|s| !s.is_empty());
        let account_filter = p.account.as_deref().filter(|s| !s.is_empty());

        let budget_per_email = ((25000 - limit * 150) / limit).max(500);

        let fetch_limit = if account_filter.is_some() { limit * 4 } else { limit };
        let mut emails = self
            .store
            .read_messages_full(fetch_limit, contact_ref, budget_per_email)
            .await
            .map_err(|e| format!("Read emails: {e}"))?;

        if let Some(acc) = account_filter {
            emails.retain(|e| e.account.eq_ignore_ascii_case(acc));
            emails.truncate(limit);
        }

        if emails.is_empty() {
            return Ok("Nessuna email trovata.".to_string());
        }

        let mut output = format!("Recent emails ({} results):\n", emails.len());

        for (i, email) in emails.iter().enumerate() {
            let date = format_date_short(&email.date);
            let mut body = email.text.clone();
            if body.len() >= budget_per_email {
                body.push_str("\n[...troncato...]");
            }

            let uid_str = email.imap_uid.map(|u| format!(" [UID: {}]", u)).unwrap_or_default();
            let folder_str = if !email.folder.is_empty() {
                format!("\nFolder: {}", email.folder)
            } else {
                String::new()
            };
            output.push_str(&format!(
                "\n{}\nEmail {} - {}{}\nFrom: {}\nTo: {}\nSubject: {}{}\n{}\n{}\n",
                "=".repeat(60),
                i + 1,
                date,
                uid_str,
                email.sender,
                email.recipient,
                email.subject,
                folder_str,
                "-".repeat(40),
                body
            ));
        }

        Ok(output)
    }

    /// Read a single email by its IMAP UID with full body text.
    #[tool(
        name = "read_email",
        description = "Read a single email by IMAP UID with full body text. Use when you need to read one specific email."
    )]
    async fn read_email(
        &self,
        params: Parameters<ReadEmailParams>,
    ) -> Result<String, String> {
        let p = params.0;
        let max_chars = p.max_chars.unwrap_or(25000).clamp(500, 25000) as usize;

        // First try Qdrant (fast, no IMAP connection needed)
        let (resolved_account, stored) = self.resolve_uid(p.email_uid, p.account.as_deref()).await?;
        if let Some(email) = stored {
            let date = format_date_short(&email.date);
            let effective_folder = if !email.folder.is_empty() {
                &email.folder
            } else {
                p.folder.as_deref().unwrap_or("INBOX")
            };
            let mut body: String = email.text.chars().take(max_chars).collect();
            if email.text.len() > max_chars {
                body.push_str("\n[...troncato...]");
            }
            let account_tag = if email.account.is_empty() {
                String::new()
            } else {
                format!(" [{}]", email.account)
            };

            return Ok(format!(
                "Email [UID: {}]{}\nFrom: {}\nTo: {}\nSubject: {}\nDate: {}\nFolder: {}\n{}\n{}",
                p.email_uid,
                account_tag,
                email.sender,
                email.recipient,
                email.subject,
                date,
                effective_folder,
                "-".repeat(60),
                body,
            ));
        }

        // Fallback: fetch from IMAP directly
        let folder = p.folder.as_deref().unwrap_or("INBOX");
        let client = self.email_client(Some(&resolved_account))?;
        match client.fetch_emails_by_uids(folder, &[p.email_uid]).await {
            Ok(emails) => {
                let email_data = emails.into_iter().next()
                    .ok_or_else(|| format!("Email UID {} non trovata in {}", p.email_uid, folder))?;

                let mut body: String = email_data.body.chars().take(max_chars).collect();
                if email_data.body.len() > max_chars {
                    body.push_str("\n[...troncato...]");
                }
                let to_str = email_data.to.join(", ");
                Ok(format!(
                    "Email [UID: {}]\nFrom: {}\nTo: {}\nSubject: {}\nDate: {}\nFolder: {}\n{}\n{}",
                    p.email_uid,
                    email_data.from,
                    to_str,
                    email_data.subject,
                    email_data.date,
                    folder,
                    "-".repeat(60),
                    body,
                ))
            }
            Err(e) => Err(format!("Errore lettura email UID {}: {e}", p.email_uid)),
        }
    }

    // ===================================================================
    // EMAIL SEARCH
    // ===================================================================

    /// Semantic search in indexed emails.
    #[tool(
        name = "search_emails",
        description = "Search emails by content (semantic + keyword). Returns results with similarity scores."
    )]
    async fn search_emails(
        &self,
        params: Parameters<SearchEmailsParams>,
    ) -> Result<String, String> {
        let p = params.0;
        let limit = p.limit.unwrap_or(10).clamp(1, 30) as usize;
        let account_filter = p.account.as_deref().filter(|s| !s.is_empty());

        // Account filter is applied server-side on Qdrant.
        let results = self
            .store
            .search(&p.query, None, limit, account_filter)
            .await
            .map_err(|e| format!("Search: {e}"))?;

        if results.is_empty() {
            return Ok(format!("Nessun risultato per '{}'.", p.query));
        }

        let mut output = format!(
            "Found {} results for: \"{}\"\n\n",
            results.len(),
            p.query
        );
        for (i, r) in results.iter().enumerate() {
            let sender = r.metadata.get("sender").cloned().unwrap_or_default();
            let recipient = r.metadata.get("recipient").cloned().unwrap_or_default();
            let subject = r.metadata.get("subject").cloned().unwrap_or_default();
            let date = r.metadata.get("date").cloned().unwrap_or_default();
            let uid_str = r.imap_uid.map(|u| format!(" [UID: {}]", u)).unwrap_or_default();
            let folder_str = if !r.folder.is_empty() {
                format!(" [{}]", r.folder)
            } else {
                String::new()
            };
            let preview: String = r.text.chars().take(800).collect();
            let similarity = if r.score > 0.0 {
                format!(" (similarity: {:.2})", r.score)
            } else {
                String::new()
            };
            output.push_str(&format!(
                "--- Result {}{}{}{} ---\nFrom: {}\nTo: {}\nSubject: {}\nDate: {}\nText: {}\n\n",
                i + 1,
                similarity,
                uid_str,
                folder_str,
                sender,
                recipient,
                subject,
                format_date_short(&date),
                preview
            ));
        }
        Ok(output)
    }

    /// Find all emails with a specific contact.
    #[tool(
        name = "search_by_contact",
        description = "Find all emails with a specific person (sender or recipient)."
    )]
    async fn search_by_contact(
        &self,
        params: Parameters<SearchByContactParams>,
    ) -> Result<String, String> {
        let p = params.0;
        let limit = p.limit.unwrap_or(20).clamp(1, 50) as usize;
        let account_filter = p.account.as_deref().filter(|s| !s.is_empty());

        // Account filter is applied server-side on Qdrant.
        let results = self
            .store
            .search_by_contact(&p.contact, limit, account_filter)
            .await
            .map_err(|e| format!("Search by contact: {e}"))?;

        if results.is_empty() {
            return Ok(format!("Nessuna email trovata con '{}'.", p.contact));
        }

        let mut output = format!(
            "Found {} emails involving '{}':\n\n",
            results.len(),
            p.contact
        );
        for (i, r) in results.iter().enumerate() {
            let sender = r.metadata.get("sender").cloned().unwrap_or_default();
            let recipient = r.metadata.get("recipient").cloned().unwrap_or_default();
            let subject = r.metadata.get("subject").cloned().unwrap_or_default();
            let date = r.metadata.get("date").cloned().unwrap_or_default();
            let uid_str = r.imap_uid.map(|u| format!(" [UID: {}]", u)).unwrap_or_default();
            let folder_str = if !r.folder.is_empty() {
                format!(" [{}]", r.folder)
            } else {
                String::new()
            };
            let preview: String = r.text.chars().take(500).collect();
            output.push_str(&format!(
                "--- {}. [EMAIL]{}{} ---\nFrom: {}\nTo: {}\nSubject: {}\nDate: {}\nText: {}\n\n",
                i + 1,
                uid_str,
                folder_str,
                sender,
                recipient,
                subject,
                format_date_short(&date),
                preview
            ));
        }
        Ok(output)
    }

    // ===================================================================
    // EMAIL SEND / REPLY
    // ===================================================================

    /// Send an email. IMPORTANT: Always show draft and get user confirmation!
    #[tool(
        name = "send_email",
        description = "Send an email. IMPORTANT: Always show draft and get user confirmation before sending! Signature is auto-appended."
    )]
    async fn send_email(
        &self,
        params: Parameters<SendEmailToolParams>,
    ) -> Result<String, String> {
        let p = params.0;
        let account_name = p.account.clone();

        let to_list: Vec<String> = p
            .to
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        let cc_list: Vec<String> = p
            .cc
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        let attach_paths: Vec<PathBuf> = p
            .attachments
            .unwrap_or_default()
            .split(',')
            .map(|s| PathBuf::from(s.trim()))
            .filter(|p| !p.as_os_str().is_empty() && p.exists())
            .collect();

        let client_params = ClientSendParams {
            to: to_list,
            subject: p.subject,
            body: p.body,
            cc: cc_list,
            from_name: p.from_name.or_else(|| Some(" ".to_string())),
            attachments: attach_paths,
            signature_name: p.signature_name.unwrap_or_else(|| "default".to_string()),
            in_reply_to: None,
            references: None,
        };

        let client = self.email_client(account_name.as_deref())?;
        match client.send_email(client_params).await {
            Ok(result) => Ok(serde_json::json!({
                "success": result.success,
                "to": result.to,
                "subject": result.subject,
                "saved_to_sent": result.saved_to_sent,
            })
            .to_string()),
            Err(e) => Err(format!("Errore invio: {e}")),
        }
    }

    /// Reply to an email in the same thread.
    #[tool(
        name = "reply_email",
        description = "Reply to an email in the same thread. Signature is auto-appended. Always show draft first!"
    )]
    async fn reply_email(
        &self,
        params: Parameters<ReplyEmailParams>,
    ) -> Result<String, String> {
        let p = params.0;

        let cc_list: Vec<String> = p
            .cc
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        let attach_paths: Vec<PathBuf> = p
            .attachments
            .unwrap_or_default()
            .split(',')
            .map(|s| PathBuf::from(s.trim()))
            .filter(|pa| !pa.as_os_str().is_empty() && pa.exists())
            .collect();

        let folder = p.folder.as_deref().unwrap_or("INBOX");
        let sig_name = p.signature_name.as_deref().unwrap_or("default");

        // Multi-account safe resolution
        let (resolved_account, _stored) =
            self.resolve_uid(p.email_uid, p.account.as_deref()).await?;
        let client = self.email_client(Some(&resolved_account))?;
        match client
            .reply_email(p.email_uid, folder, &p.body, cc_list, sig_name, attach_paths)
            .await
        {
            Ok(result) => Ok(serde_json::json!({
                "success": result.success,
                "to": result.to,
                "subject": result.subject,
                "saved_to_sent": result.saved_to_sent,
            })
            .to_string()),
            Err(e) => Err(format!("Errore reply: {e}")),
        }
    }

    // ===================================================================
    // ATTACHMENTS
    // ===================================================================

    /// Get attachment info for an email.
    #[tool(
        name = "get_email_attachments",
        description = "List attachments of an email (filename, size, content type)."
    )]
    async fn get_email_attachments(
        &self,
        params: Parameters<GetAttachmentsParams>,
    ) -> Result<String, String> {
        let p = params.0;
        let folder = p.folder.as_deref().unwrap_or("INBOX");

        let (resolved_account, _) = self.resolve_uid(p.email_uid, p.account.as_deref()).await?;
        let client = self.email_client(Some(&resolved_account))?;

        match client.get_attachments(p.email_uid, folder).await {
            Ok(attachments) => {
                if attachments.is_empty() {
                    return Ok("Nessun allegato.".to_string());
                }

                let list: Vec<Value> = attachments
                    .iter()
                    .map(|a| {
                        serde_json::json!({
                            "index": a.index,
                            "filename": a.filename,
                            "content_type": a.content_type,
                            "size": a.size,
                        })
                    })
                    .collect();

                Ok(serde_json::to_string_pretty(&list).unwrap_or_default())
            }
            Err(e) => Err(format!("Errore: {e}")),
        }
    }

    /// Save an attachment to disk.
    #[tool(
        name = "save_attachment",
        description = "Download and save an email attachment to disk."
    )]
    async fn save_attachment(
        &self,
        params: Parameters<SaveAttachmentParams>,
    ) -> Result<String, String> {
        let p = params.0;
        let folder = p.folder.as_deref().unwrap_or("INBOX");
        let save_dir = p
            .save_dir
            .map(PathBuf::from)
            .unwrap_or_else(|| dirs::download_dir().unwrap_or_else(|| PathBuf::from(".")));

        if !save_dir.exists() {
            std::fs::create_dir_all(&save_dir)
                .map_err(|e| format!("Errore creazione directory: {e}"))?;
        }

        let (resolved_account, _) = self.resolve_uid(p.email_uid, p.account.as_deref()).await?;
        let client = self.email_client(Some(&resolved_account))?;
        let data = client
            .download_attachment(p.email_uid, p.attachment_index as usize, folder)
            .await
            .map_err(|e| format!("Errore download: {e}"))?;

        // Avoid overwriting
        let mut dest = save_dir.join(&data.filename);
        let mut counter = 1;
        while dest.exists() {
            let stem = PathBuf::from(&data.filename);
            let name = stem.file_stem().unwrap_or_default().to_string_lossy();
            let ext = stem
                .extension()
                .map(|e| format!(".{}", e.to_string_lossy()))
                .unwrap_or_default();
            dest = save_dir.join(format!("{name}({counter}){ext}"));
            counter += 1;
        }

        std::fs::write(&dest, &data.data)
            .map_err(|e| format!("Errore salvataggio: {e}"))?;

        Ok(serde_json::json!({
            "saved": true,
            "path": dest.display().to_string(),
            "filename": data.filename,
            "size": data.data.len(),
        })
        .to_string())
    }

    /// Check for bounced/undeliverable emails.
    #[tool(
        name = "check_bounced_emails",
        description = "Check for bounced/undeliverable emails in a folder."
    )]
    async fn check_bounced_emails(
        &self,
        params: Parameters<CheckBouncedParams>,
    ) -> Result<String, String> {
        let p = params.0;
        let folder = p.folder.as_deref().unwrap_or("INBOX");
        let limit = p.limit.unwrap_or(20).clamp(1, 50) as usize;

        let client = self.email_client(p.account.as_deref())?;
        let bounces = client
            .check_bounced(folder, limit)
            .await
            .map_err(|e| format!("Errore: {e}"))?;

        if bounces.is_empty() {
            return Ok("Nessuna email rimbalzata trovata.".to_string());
        }

        let list: Vec<Value> = bounces
            .iter()
            .map(|b| {
                serde_json::json!({
                    "message_id": b.message_id,
                    "failed_email": b.failed_email,
                    "reason": b.reason,
                    "date": b.date,
                })
            })
            .collect();

        Ok(serde_json::to_string_pretty(&list).unwrap_or_default())
    }

    // ===================================================================
    // DELETE / BLACKLIST
    // ===================================================================

    /// Delete an email by moving it to the Trash folder.
    #[tool(
        name = "delete_email",
        description = "Move an email to the Trash folder (recoverable). IMPORTANT: Confirm with the user before invoking — this is a destructive action visible to the user's mailbox."
    )]
    async fn delete_email(
        &self,
        params: Parameters<DeleteEmailParams>,
    ) -> Result<String, String> {
        let p = params.0;
        let folder = p.folder.as_deref().unwrap_or("INBOX").to_string();

        // Multi-account safe resolution (errors out if UID is ambiguous)
        let (resolved_account, stored) =
            self.resolve_uid(p.email_uid, p.account.as_deref()).await?;
        let effective_folder = stored
            .as_ref()
            .map(|e| {
                if e.folder.is_empty() {
                    folder.clone()
                } else {
                    e.folder.clone()
                }
            })
            .unwrap_or(folder);

        let client = self.email_client(Some(&resolved_account))?;
        let account_label = client.account_name().to_string();

        let trash = client
            .move_to_trash(&effective_folder, p.email_uid)
            .await
            .map_err(|e| format!("Errore spostamento in Trash: {e}"))?;

        // Best-effort: remove from Qdrant (account-scoped so we don't delete the
        // wrong row if another account happens to have the same UID).
        let removed = self
            .store
            .delete_by_imap_uid_in_account(p.email_uid, &resolved_account)
            .await
            .unwrap_or(false);

        Ok(serde_json::json!({
            "success": true,
            "uid": p.email_uid,
            "account": account_label,
            "from_folder": effective_folder,
            "to_folder": trash,
            "removed_from_index": removed,
        })
        .to_string())
    }

    /// Add a sender to the per-account blacklist.
    #[tool(
        name = "blacklist_add",
        description = "Add a sender to the per-account blacklist. Future emails from this sender will be auto-moved to Trash and not indexed. Accepts \"addr@host\", \"Name <addr@host>\", or substrings like \"@badprovider.com\"."
    )]
    async fn blacklist_add(
        &self,
        params: Parameters<BlacklistAddParams>,
    ) -> Result<String, String> {
        let p = params.0;
        let account = self
            .config
            .resolve_account(p.account.as_deref())
            .ok_or_else(|| "Account non configurato.".to_string())?;
        let account_name = account.name.clone();

        let mut bl = Blacklist::load(&self.config.data_dir, &account_name);
        let added = bl.add(&p.sender);
        if added {
            bl.save(&self.config.data_dir, &account_name)
                .map_err(|e| format!("Errore salvataggio blacklist: {e}"))?;
        }

        Ok(serde_json::json!({
            "added": added,
            "sender": p.sender,
            "account": account_name,
            "total_entries": bl.senders.len(),
        })
        .to_string())
    }

    /// Remove a sender from the per-account blacklist.
    #[tool(
        name = "blacklist_remove",
        description = "Remove a sender from the per-account blacklist."
    )]
    async fn blacklist_remove(
        &self,
        params: Parameters<BlacklistRemoveParams>,
    ) -> Result<String, String> {
        let p = params.0;
        let account = self
            .config
            .resolve_account(p.account.as_deref())
            .ok_or_else(|| "Account non configurato.".to_string())?;
        let account_name = account.name.clone();

        let mut bl = Blacklist::load(&self.config.data_dir, &account_name);
        let removed = bl.remove(&p.sender);
        if removed {
            bl.save(&self.config.data_dir, &account_name)
                .map_err(|e| format!("Errore salvataggio blacklist: {e}"))?;
        }

        Ok(serde_json::json!({
            "removed": removed,
            "sender": p.sender,
            "account": account_name,
            "total_entries": bl.senders.len(),
        })
        .to_string())
    }

    /// List blacklist entries (for one account or all).
    #[tool(
        name = "blacklist_list",
        description = "List all blacklisted senders, optionally filtered by account."
    )]
    async fn blacklist_list(
        &self,
        params: Parameters<BlacklistListParams>,
    ) -> Result<String, String> {
        let p = params.0;

        let accounts: Vec<&crate::config::AccountConfig> = match p.account.as_deref() {
            Some(name) if !name.is_empty() => {
                let a = self
                    .config
                    .account(name)
                    .ok_or_else(|| format!("Account '{name}' non configurato."))?;
                vec![a]
            }
            _ => self.config.accounts.iter().collect(),
        };

        let mut output = serde_json::Map::new();
        for a in &accounts {
            let bl = Blacklist::load(&self.config.data_dir, &a.name);
            output.insert(a.name.clone(), serde_json::json!(bl.senders));
        }

        Ok(serde_json::to_string_pretty(&Value::Object(output)).unwrap_or_default())
    }

    /// Bulk-delete emails matching filters. ALWAYS run with dry_run=true first.
    #[tool(
        name = "delete_emails_bulk",
        description = "Bulk-delete emails matching filters (sender, subject, older_than). DEFAULT dry_run=true (preview only). IMPORTANT: After preview, get EXPLICIT user confirmation BEFORE running with dry_run=false. Destructive: moves matched emails to Trash."
    )]
    async fn delete_emails_bulk(
        &self,
        params: Parameters<DeleteEmailsBulkParams>,
    ) -> Result<String, String> {
        let p = params.0;
        let folder = p.folder.unwrap_or_else(|| "INBOX".into());
        let limit = p.limit.unwrap_or(1000).clamp(1, 10_000) as usize;
        let dry_run = p.dry_run.unwrap_or(true);

        // At least one filter required to prevent accidental "delete everything"
        let has_filter = p.sender.as_deref().map(|s| !s.is_empty()).unwrap_or(false)
            || p.subject_contains.as_deref().map(|s| !s.is_empty()).unwrap_or(false)
            || p.older_than.as_deref().map(|s| !s.is_empty()).unwrap_or(false);
        if !has_filter {
            return Err("Specifica almeno un filtro: sender, subject_contains, o older_than.".into());
        }

        let client = self.email_client(p.account.as_deref())?;
        let account_label = client.account_name().to_string();

        let uids = client
            .search_uids_filtered(
                &folder,
                p.sender.as_deref().filter(|s| !s.is_empty()),
                p.subject_contains.as_deref().filter(|s| !s.is_empty()),
                p.older_than.as_deref().filter(|s| !s.is_empty()),
            )
            .await
            .map_err(|e| format!("Errore SEARCH: {e}"))?;

        let total_matched = uids.len();
        let truncated = total_matched > limit;
        let to_process: Vec<u32> = uids.into_iter().take(limit).collect();

        // Build a small preview using the Qdrant index when possible.
        // Account is fixed (we already built the IMAP client for it), so look up
        // scoped to that account to avoid cross-account UID collisions.
        let mut preview: Vec<Value> = Vec::new();
        for uid in to_process.iter().take(5) {
            if let Ok(Some(e)) = self
                .store
                .get_by_imap_uid_in_account(*uid, &account_label)
                .await
            {
                preview.push(serde_json::json!({
                    "uid": uid,
                    "from": e.sender,
                    "subject": e.subject,
                    "date": e.date,
                }));
            } else {
                preview.push(serde_json::json!({ "uid": uid }));
            }
        }

        if dry_run {
            return Ok(serde_json::json!({
                "dry_run": true,
                "account": account_label,
                "folder": folder,
                "total_matched": total_matched,
                "would_delete": to_process.len(),
                "truncated_by_limit": truncated,
                "limit": limit,
                "preview_first_5": preview,
                "next_step": "Mostra questi numeri all'utente e CHIEDI conferma esplicita. Poi richiama con dry_run=false."
            }).to_string());
        }

        // Real deletion
        let (moved, trash) = client
            .bulk_move_to_trash(&folder, &to_process)
            .await
            .map_err(|e| format!("Errore bulk move: {e}"))?;

        // Best-effort: remove from Qdrant scoped to this account.
        let mut removed_from_index = 0usize;
        for uid in &to_process {
            if self
                .store
                .delete_by_imap_uid_in_account(*uid, &account_label)
                .await
                .unwrap_or(false)
            {
                removed_from_index += 1;
            }
        }

        Ok(serde_json::json!({
            "dry_run": false,
            "account": account_label,
            "from_folder": folder,
            "to_folder": trash,
            "moved": moved,
            "total_matched": total_matched,
            "truncated_by_limit": truncated,
            "removed_from_index": removed_from_index,
        }).to_string())
    }

    /// Apply the blacklist retroactively to a folder.
    #[tool(
        name = "blacklist_sweep",
        description = "Find all emails in the folder from senders already in the blacklist, and move them to Trash. DEFAULT dry_run=true (preview only). IMPORTANT: After preview, get EXPLICIT user confirmation BEFORE running with dry_run=false."
    )]
    async fn blacklist_sweep(
        &self,
        params: Parameters<BlacklistSweepParams>,
    ) -> Result<String, String> {
        let p = params.0;
        let folder = p.folder.unwrap_or_else(|| "INBOX".into());
        let dry_run = p.dry_run.unwrap_or(true);

        let account = self
            .config
            .resolve_account(p.account.as_deref())
            .ok_or_else(|| "Account non configurato.".to_string())?;
        let account_name = account.name.clone();

        let bl = Blacklist::load(&self.config.data_dir, &account_name);
        if bl.senders.is_empty() {
            return Ok(serde_json::json!({
                "account": account_name,
                "folder": folder,
                "message": "Blacklist vuota. Aggiungi mittenti con blacklist_add prima.",
            }).to_string());
        }

        let client = self.email_client(Some(&account_name))?;

        // For each blacklisted sender, run IMAP SEARCH FROM. Accumulate UIDs.
        let mut per_sender: Vec<(String, usize)> = Vec::new();
        let mut all_uids: std::collections::BTreeSet<u32> = std::collections::BTreeSet::new();

        for sender in &bl.senders {
            match client
                .search_uids_filtered(&folder, Some(sender), None, None)
                .await
            {
                Ok(uids) => {
                    per_sender.push((sender.clone(), uids.len()));
                    for u in uids {
                        all_uids.insert(u);
                    }
                }
                Err(e) => {
                    warn!("blacklist_sweep: SEARCH FROM {sender} failed: {e}");
                    per_sender.push((sender.clone(), 0));
                }
            }
        }

        let total = all_uids.len();
        let breakdown: Vec<Value> = per_sender
            .iter()
            .filter(|(_, n)| *n > 0)
            .map(|(s, n)| serde_json::json!({ "sender": s, "matches": n }))
            .collect();

        if dry_run {
            return Ok(serde_json::json!({
                "dry_run": true,
                "account": account_name,
                "folder": folder,
                "blacklist_entries": bl.senders.len(),
                "total_to_trash": total,
                "by_sender": breakdown,
                "next_step": "Mostra il totale all'utente, CHIEDI conferma esplicita, poi richiama con dry_run=false."
            }).to_string());
        }

        if total == 0 {
            return Ok(serde_json::json!({
                "dry_run": false,
                "account": account_name,
                "folder": folder,
                "moved": 0,
                "message": "Nessuna email da mittenti blacklistati in questa folder."
            }).to_string());
        }

        let uids: Vec<u32> = all_uids.into_iter().collect();
        let (moved, trash) = client
            .bulk_move_to_trash(&folder, &uids)
            .await
            .map_err(|e| format!("Errore bulk move: {e}"))?;

        let mut removed_from_index = 0usize;
        for uid in &uids {
            if self
                .store
                .delete_by_imap_uid_in_account(*uid, &account_name)
                .await
                .unwrap_or(false)
            {
                removed_from_index += 1;
            }
        }

        Ok(serde_json::json!({
            "dry_run": false,
            "account": account_name,
            "from_folder": folder,
            "to_folder": trash,
            "moved": moved,
            "by_sender": breakdown,
            "removed_from_index": removed_from_index,
        }).to_string())
    }

    // ===================================================================
    // INDEXING
    // ===================================================================

    /// Reset the email index and re-index all folders in background.
    #[tool(
        name = "reset_index",
        description = "Delete and recreate the email search index, then re-index all folders in background. Returns immediately. Use index_status to monitor progress."
    )]
    async fn reset_index(&self) -> Result<String, String> {
        // Check if already running
        {
            let status = self.task_status.lock().await;
            if status.running {
                return Err(format!(
                    "Operazione già in corso: {} ({})",
                    status.phase, status.folder
                ));
            }
        }

        let store = self.store.clone();
        let config = self.config.clone();
        let index_lock = self.index_lock.clone();
        let task_status = self.task_status.clone();
        let oauth_cache = self.oauth_cache.clone();

        tokio::spawn(async move {
            let _guard = index_lock.lock().await;

            // Phase: reset
            {
                let mut s = task_status.lock().await;
                s.running = true;
                s.phase = "reset".into();
                s.folder = String::new();
                s.indexed = 0;
                s.skipped = 0;
                s.errors = 0;
                s.started_at = Some(Utc::now().to_rfc3339());
                s.finished_at = None;
                s.message = "Cancellazione collection Qdrant...".into();
            }

            info!("reset_index: deleting collection");
            if let Err(e) = store.reset_collection().await {
                warn!("reset_index: collection reset failed: {e}");
                let mut s = task_status.lock().await;
                s.running = false;
                s.phase = "error".into();
                s.message = format!("Errore reset: {e}");
                s.finished_at = Some(Utc::now().to_rfc3339());
                return;
            }

            // Delete indexer state files (one per account)
            for account in &config.accounts {
                let state_file = if account.name.is_empty() || account.name == "primary" {
                    "indexer_state.json".to_string()
                } else {
                    format!("indexer_state_{}.json", account.name)
                };
                let state_path = std::path::Path::new(&config.data_dir).join(&state_file);
                if state_path.exists() {
                    let _ = std::fs::remove_file(&state_path);
                }
            }
            info!("reset_index: collection recreated, indexer state cleared for all accounts");

            let skip = ["trash", "cestino", "junk", "spam", "drafts", "bozze"];
            let mut total_indexed = 0usize;
            let mut total_skipped = 0usize;
            let mut total_errors = 0usize;
            let mut folder_count = 0usize;

            // Iterate every configured account
            for account in &config.accounts {
                info!("reset_index: [{}] starting", account.name);
                let acc_client = build_backend(
                    account.clone(),
                    config.signature_dir.clone(),
                    oauth_cache.clone(),
                );
                let folders = match acc_client.list_folders().await {
                    Ok(f) => f,
                    Err(e) => {
                        warn!("reset_index: [{}] list folders failed: {e}", account.name);
                        total_errors += 1;
                        continue;
                    }
                };

                let folders_to_index: Vec<String> = folders
                    .into_iter()
                    .filter(|f| {
                        let fl = f.to_lowercase();
                        !skip.iter().any(|s| fl.contains(s))
                    })
                    .collect();
                folder_count += folders_to_index.len();

                for folder in &folders_to_index {
                    {
                        let mut s = task_status.lock().await;
                        s.phase = "indexing".into();
                        s.folder = format!("{}/{}", account.name, folder);
                        s.message = format!("Indicizzazione [{}] {}...", account.name, folder);
                    }

                    let idx_client = build_backend(
                        account.clone(),
                        config.signature_dir.clone(),
                        oauth_cache.clone(),
                    );
                    let indexer = EmailIndexer::new(store.clone(), idx_client, &config.data_dir);

                    match indexer.index_folder(folder, 0, true, false).await {
                        Ok(result) => {
                            total_indexed += result.indexed;
                            total_skipped += result.skipped;
                            total_errors += result.errors;
                            info!(
                                "reset_index: [{}] {folder} done — indexed={}, skipped={}, errors={}",
                                account.name, result.indexed, result.skipped, result.errors
                            );
                            let mut s = task_status.lock().await;
                            s.indexed = total_indexed;
                            s.skipped = total_skipped;
                            s.errors = total_errors;
                        }
                        Err(e) => {
                            warn!("reset_index: [{}] error indexing {folder}: {e}", account.name);
                            total_errors += 1;
                            let mut s = task_status.lock().await;
                            s.errors = total_errors;
                        }
                    }
                }
            }

            {
                let mut s = task_status.lock().await;
                s.running = false;
                s.phase = "completed".into();
                s.folder = String::new();
                s.finished_at = Some(Utc::now().to_rfc3339());
                s.message = format!(
                    "Re-indicizzazione completata: {} indicizzate, {} skipped, {} errori su {} folder ({} account).",
                    total_indexed, total_skipped, total_errors, folder_count, config.accounts.len()
                );
            }
            info!("reset_index: all done — indexed={total_indexed}, skipped={total_skipped}, errors={total_errors}");
        });

        Ok("Reset e re-indicizzazione avviati in background. Usa index_status per monitorare il progresso.".to_string())
    }

    /// Repair Qdrant points whose `date` payload is empty by re-querying
    /// IMAP INTERNALDATE / Gmail internalDate. Does NOT re-download bodies
    /// nor recompute embeddings — only patches the date payload.
    #[tool(
        name = "repair_empty_dates",
        description = "Fix Qdrant points whose 'date' payload is empty (broken indexing pre-fix) by re-querying IMAP INTERNALDATE / Gmail internalDate. Lightweight: no body download, no re-embedding. Returns {scanned, repaired, skipped_no_uid, skipped_no_date, errors}."
    )]
    async fn repair_empty_dates(
        &self,
        params: Parameters<RepairEmptyDatesParams>,
    ) -> Result<String, String> {
        let p = params.0;
        let dry_run = p.dry_run.unwrap_or(false);
        let account_name = p.account.trim().to_string();
        if account_name.is_empty() {
            return Err("Parametro 'account' obbligatorio (es. \"aruba\", \"gmail\").".into());
        }

        let account = self
            .config
            .account(&account_name)
            .cloned()
            .ok_or_else(|| format!("Account '{account_name}' non configurato."))?;

        // 1. Scroll Qdrant for points with date = "".
        let points: Vec<EmptyDatePoint> = self
            .store
            .scroll_empty_dates(Some(&account_name))
            .await
            .map_err(|e| format!("Scroll empty dates: {e}"))?;

        let scanned = points.len();
        info!(
            "repair_empty_dates [{}]: scanned {} points with empty date (dry_run={})",
            account_name, scanned, dry_run
        );

        // Group by folder. Skip points without imap_uid (can't query server).
        let mut by_folder: std::collections::HashMap<String, Vec<(String, u32)>> =
            std::collections::HashMap::new();
        let mut skipped_no_uid = 0usize;
        for pt in points {
            let Some(uid) = pt.imap_uid else {
                skipped_no_uid += 1;
                continue;
            };
            if pt.folder.is_empty() {
                skipped_no_uid += 1;
                continue;
            }
            by_folder
                .entry(pt.folder.clone())
                .or_default()
                .push((pt.point_id, uid));
        }

        if dry_run {
            let preview: Vec<Value> = by_folder
                .iter()
                .map(|(folder, items)| serde_json::json!({
                    "folder": folder,
                    "count": items.len(),
                }))
                .collect();
            return Ok(serde_json::json!({
                "account": account_name,
                "dry_run": true,
                "scanned": scanned,
                "would_repair": scanned - skipped_no_uid,
                "skipped_no_uid": skipped_no_uid,
                "by_folder": preview,
            })
            .to_string());
        }

        // 2. For each folder, query backend for INTERNALDATE per UID.
        let backend = build_backend(
            account.clone(),
            self.config.signature_dir.clone(),
            self.oauth_cache.clone(),
        );

        let mut repaired = 0usize;
        let mut skipped_no_date = 0usize;
        let mut errors = 0usize;

        for (folder, items) in &by_folder {
            // Chunk UIDs to keep IMAP commands bounded.
            for chunk in items.chunks(500) {
                let uids: Vec<u32> = chunk.iter().map(|(_, u)| *u).collect();
                let dates = match backend.fetch_internal_dates(folder, &uids).await {
                    Ok(m) => m,
                    Err(e) => {
                        warn!(
                            "repair_empty_dates [{}] {folder}: fetch_internal_dates failed: {e}",
                            account_name
                        );
                        errors += chunk.len();
                        continue;
                    }
                };

                let mut batch: Vec<(String, String)> = Vec::with_capacity(chunk.len());
                for (point_id, uid) in chunk {
                    match dates.get(uid) {
                        Some(d) => batch.push((point_id.clone(), d.clone())),
                        None => skipped_no_date += 1,
                    }
                }

                if batch.is_empty() {
                    continue;
                }

                match self.store.set_date_payload_batch(&batch).await {
                    Ok(n) => {
                        repaired += n;
                        info!(
                            "repair_empty_dates [{}] {folder}: patched {n}/{} (running total {repaired})",
                            account_name,
                            batch.len()
                        );
                    }
                    Err(e) => {
                        warn!(
                            "repair_empty_dates [{}] {folder}: set_date_payload_batch failed: {e}",
                            account_name
                        );
                        errors += batch.len();
                    }
                }
            }
        }

        Ok(serde_json::json!({
            "account": account_name,
            "dry_run": false,
            "scanned": scanned,
            "repaired": repaired,
            "skipped_no_uid": skipped_no_uid,
            "skipped_no_date": skipped_no_date,
            "errors": errors,
        })
        .to_string())
    }

    /// Manually trigger email indexing (background).
    #[tool(
        name = "index_emails",
        description = "Trigger email indexing for a folder in background. Returns immediately. Use index_status to monitor progress."
    )]
    async fn index_emails(
        &self,
        params: Parameters<IndexEmailsParams>,
    ) -> Result<String, String> {
        let p = params.0;
        let folder = p.folder.unwrap_or_else(|| "INBOX".into());
        let limit = p.limit.unwrap_or(0) as usize;
        let full_reindex = p.full_reindex.unwrap_or(false);

        // Resolve target accounts: explicit name → just that one; otherwise all.
        let target_accounts: Vec<crate::config::AccountConfig> = match p.account.as_deref() {
            Some(name) if !name.is_empty() => {
                let acc = self
                    .config
                    .account(name)
                    .ok_or_else(|| format!("Account '{name}' non configurato."))?;
                vec![acc.clone()]
            }
            _ => self.config.accounts.clone(),
        };

        if target_accounts.is_empty() {
            return Err("Nessun account email configurato.".into());
        }

        {
            let status = self.task_status.lock().await;
            if status.running {
                return Err(format!(
                    "Operazione già in corso: {} ({})",
                    status.phase, status.folder
                ));
            }
        }

        let store = self.store.clone();
        let config = self.config.clone();
        let index_lock = self.index_lock.clone();
        let task_status = self.task_status.clone();
        let oauth_cache = self.oauth_cache.clone();
        let folder_for_msg = folder.clone();
        let accounts_for_msg: Vec<String> =
            target_accounts.iter().map(|a| a.name.clone()).collect();

        tokio::spawn(async move {
            let _guard = index_lock.lock().await;

            {
                let mut s = task_status.lock().await;
                s.running = true;
                s.phase = "indexing".into();
                s.folder = folder.clone();
                s.indexed = 0;
                s.skipped = 0;
                s.errors = 0;
                s.started_at = Some(Utc::now().to_rfc3339());
                s.finished_at = None;
                s.message = format!("Indicizzazione {}...", folder);
            }

            let mut total_indexed = 0usize;
            let mut total_skipped = 0usize;
            let mut total_errors = 0usize;

            for account in &target_accounts {
                let client = build_backend(
                    account.clone(),
                    config.signature_dir.clone(),
                    oauth_cache.clone(),
                );
                let indexer = EmailIndexer::new(store.clone(), client, &config.data_dir);

                {
                    let mut s = task_status.lock().await;
                    s.folder = format!("{}/{}", account.name, folder);
                    s.message = format!("[{}] Indicizzazione {}...", account.name, folder);
                }

                match indexer.index_folder(&folder, limit, full_reindex, false).await {
                    Ok(result) => {
                        info!(
                            "index_emails: [{}] {folder} done — indexed={}, skipped={}, errors={}",
                            account.name, result.indexed, result.skipped, result.errors
                        );
                        total_indexed += result.indexed;
                        total_skipped += result.skipped;
                        total_errors += result.errors;
                    }
                    Err(e) => {
                        warn!("index_emails: [{}] error on {folder}: {e}", account.name);
                        total_errors += 1;
                    }
                }
            }

            let mut s = task_status.lock().await;
            s.running = false;
            s.phase = "completed".into();
            s.indexed = total_indexed;
            s.skipped = total_skipped;
            s.errors = total_errors;
            s.finished_at = Some(Utc::now().to_rfc3339());
            s.message = format!(
                "{}: {} indicizzate, {} skipped, {} errori.",
                folder, total_indexed, total_skipped, total_errors
            );
        });

        Ok(format!(
            "Indicizzazione di '{}' avviata in background su {} account ({}). Usa index_status per monitorare.",
            folder_for_msg,
            accounts_for_msg.len(),
            accounts_for_msg.join(", ")
        ))
    }

    /// Check indexing task status.
    #[tool(
        name = "index_status",
        description = "Check the status of a running or last completed indexing operation."
    )]
    async fn index_status(&self) -> Result<String, String> {
        let s = self.task_status.lock().await;
        Ok(serde_json::to_string_pretty(&*s).unwrap_or_else(|_| "Errore lettura stato".into()))
    }

    /// Manually index a single message.
    #[tool(
        name = "index_message",
        description = "Manually add a single email to the search database."
    )]
    async fn index_message(
        &self,
        params: Parameters<IndexMessageParams>,
    ) -> Result<String, String> {
        let p = params.0;

        let msg = MessageData {
            id: p.message_id.clone(),
            text: p.text,
            sender: p.sender.unwrap_or_default(),
            recipient: p.recipient.unwrap_or_default(),
            subject: p.subject.unwrap_or_default(),
            date: p.date.unwrap_or_else(|| Utc::now().to_rfc3339()),
            channel: "email".to_string(),
            folder: String::new(),
            imap_uid: None,
            account: String::new(),
        };

        let is_new = self
            .store
            .index_message(msg)
            .await
            .map_err(|e| format!("Index: {e}"))?;

        let status = if is_new { "Indexed" } else { "Already exists" };
        Ok(format!("{}: {}", status, p.message_id))
    }

    /// List configured email accounts.
    #[tool(
        name = "list_accounts",
        description = "List all configured email accounts (names, addresses, IMAP hosts). Use to discover which accounts are available."
    )]
    async fn list_accounts(&self) -> Result<String, String> {
        if self.config.accounts.is_empty() {
            return Ok("Nessun account configurato.".into());
        }
        let list: Vec<Value> = self
            .config
            .accounts
            .iter()
            .enumerate()
            .map(|(i, a)| {
                serde_json::json!({
                    "name": a.name,
                    "email": a.imap_username,
                    "imap_host": a.imap_host,
                    "smtp_host": a.smtp_host,
                    "is_default": i == 0,
                })
            })
            .collect();
        Ok(serde_json::to_string_pretty(&list).unwrap_or_default())
    }

    /// Get email database statistics.
    #[tool(
        name = "get_stats",
        description = "Get email database statistics: total count, count per channel."
    )]
    async fn get_stats(&self) -> Result<String, String> {
        let stats = self
            .store
            .get_stats()
            .await
            .map_err(|e| format!("Stats: {e}"))?;

        let mut output = String::from("Email database stats:\n");
        for (channel, count) in &stats.by_channel {
            output.push_str(&format!("  {}: {}\n", channel, count));
        }
        output.push_str(&format!("  Total: {}\n", stats.total));

        Ok(output)
    }

    // ===================================================================
    // CALENDAR
    // ===================================================================

    /// List calendar events.
    #[tool(
        name = "list_calendar_events",
        description = "List upcoming calendar events from CalDAV. Supports date range filtering."
    )]
    async fn list_calendar_events(
        &self,
        params: Parameters<ListCalendarEventsParams>,
    ) -> Result<String, String> {
        let p = params.0;
        let cal = self
            .calendar_client(p.account.as_deref())
            .ok_or_else(|| "CalDAV non configurato per questo account.".to_string())?;

        let start = p.start_date.as_deref().filter(|s| !s.is_empty());
        let end = p.end_date.as_deref().filter(|s| !s.is_empty());
        let limit = p.limit.unwrap_or(20).clamp(1, 100) as usize;

        let events = cal
            .list_events(start, end, limit)
            .await
            .map_err(|e| format!("Errore calendario: {e}"))?;

        if events.is_empty() {
            return Ok("Nessun evento trovato.".to_string());
        }

        let list: Vec<Value> = events
            .iter()
            .map(|e| {
                serde_json::json!({
                    "uid": e.uid,
                    "summary": e.summary,
                    "start": e.start,
                    "end": e.end,
                    "description": e.description,
                    "location": e.location,
                    "status": e.status,
                    "organizer": e.organizer,
                    "attendees": e.attendees,
                })
            })
            .collect();

        Ok(serde_json::to_string_pretty(&list).unwrap_or_default())
    }

    /// Create a new calendar event.
    #[tool(
        name = "create_calendar_event",
        description = "Create a new calendar event on CalDAV. Returns event UID."
    )]
    async fn create_calendar_event(
        &self,
        params: Parameters<CreateCalendarEventParams>,
    ) -> Result<String, String> {
        let p = params.0;
        let cal = self
            .calendar_client(p.account.as_deref())
            .ok_or_else(|| "CalDAV non configurato per questo account.".to_string())?;

        let attendee_list: Vec<String> = p
            .attendees
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        let event_params = CreateEventParams {
            summary: p.summary.clone(),
            start: p.start,
            end: p.end,
            description: p.description,
            location: p.location,
            attendees: attendee_list,
        };

        let uid = cal
            .create_event(event_params)
            .await
            .map_err(|e| format!("Errore creazione evento: {e}"))?;

        Ok(serde_json::json!({
            "success": true,
            "uid": uid,
            "summary": p.summary,
        })
        .to_string())
    }

    /// Accept a calendar event invitation.
    #[tool(
        name = "accept_calendar_event",
        description = "Accept a calendar event invitation (RSVP = ACCEPTED)."
    )]
    async fn accept_calendar_event(
        &self,
        params: Parameters<RespondCalendarEventParams>,
    ) -> Result<String, String> {
        self.respond_calendar_event_inner(&params.0.event_uid, EventResponse::Accepted, params.0.comment.as_deref())
            .await
    }

    /// Decline a calendar event invitation.
    #[tool(
        name = "decline_calendar_event",
        description = "Decline a calendar event invitation (RSVP = DECLINED)."
    )]
    async fn decline_calendar_event(
        &self,
        params: Parameters<RespondCalendarEventParams>,
    ) -> Result<String, String> {
        self.respond_calendar_event_inner(&params.0.event_uid, EventResponse::Declined, params.0.comment.as_deref())
            .await
    }

    /// Tentatively accept a calendar event.
    #[tool(
        name = "tentative_calendar_event",
        description = "Tentatively accept a calendar event (RSVP = TENTATIVE)."
    )]
    async fn tentative_calendar_event(
        &self,
        params: Parameters<RespondCalendarEventParams>,
    ) -> Result<String, String> {
        self.respond_calendar_event_inner(&params.0.event_uid, EventResponse::Tentative, params.0.comment.as_deref())
            .await
    }

    /// Delete a calendar event.
    #[tool(
        name = "delete_calendar_event",
        description = "Delete a calendar event."
    )]
    async fn delete_calendar_event(
        &self,
        params: Parameters<DeleteCalendarEventParams>,
    ) -> Result<String, String> {
        let p = params.0;
        let cal = self
            .calendar_client(p.account.as_deref())
            .ok_or_else(|| "CalDAV non configurato per questo account.".to_string())?;

        cal.delete_event(&p.event_uid)
            .await
            .map_err(|e| format!("Errore: {e}"))?;

        Ok(serde_json::json!({
            "success": true,
            "uid": p.event_uid,
        })
        .to_string())
    }

    // ===================================================================
    // FILE EXTRACTION
    // ===================================================================

    /// Extract text from a file (PDF, DOCX, XLSX, etc.).
    #[tool(
        name = "extract_file_text",
        description = "Extract text content from PDF, DOCX, XLSX, CSV, TXT, and other files."
    )]
    async fn extract_file_text(
        &self,
        params: Parameters<ExtractFileTextParams>,
    ) -> Result<String, String> {
        let p = params.0;
        let path = PathBuf::from(&p.file_path);
        let max = p.max_chars.unwrap_or(15000) as usize;

        jupiteros_shared::file_utils::extract_text(&path, max)
            .map_err(|e| format!("Errore estrazione: {e}"))
    }

    // ===================================================================
    // SIGNATURES
    // ===================================================================

    /// Create and save an email signature.
    #[tool(
        name = "set_email_signature",
        description = "Create and save a professional HTML email signature."
    )]
    async fn set_email_signature(
        &self,
        params: Parameters<SetSignatureParams>,
    ) -> Result<String, String> {
        let p = params.0;
        let sig_name = p.signature_name.as_deref().unwrap_or("default");

        let content = signature::create_default_signature(
            &p.name,
            &p.email,
            p.phone.as_deref(),
            p.company.as_deref(),
            p.role.as_deref(),
            p.photo_url.as_deref(),
            p.color.as_deref(),
            p.style.as_deref(),
        );

        signature::save_signature(&self.config.signature_dir, sig_name, &content)
            .map_err(|e| format!("Errore: {e}"))?;

        Ok(serde_json::json!({
            "success": true,
            "name": sig_name,
            "preview": content.chars().take(200).collect::<String>(),
        })
        .to_string())
    }

    /// Get a saved email signature.
    #[tool(
        name = "get_email_signature",
        description = "Get a saved email signature by name."
    )]
    async fn get_email_signature(
        &self,
        params: Parameters<GetSignatureParams>,
    ) -> Result<String, String> {
        let p = params.0;
        let name = p.signature_name.as_deref().unwrap_or("default");

        match signature::get_signature(&self.config.signature_dir, name) {
            Some(content) => Ok(serde_json::json!({
                "name": name,
                "content": content,
            })
            .to_string()),
            None => Ok(format!("Firma '{}' non trovata.", name)),
        }
    }

    /// List all saved email signatures.
    #[tool(
        name = "list_email_signatures",
        description = "List all saved email signature names."
    )]
    async fn list_email_signatures(&self) -> Result<String, String> {
        let names = signature::list_signatures(&self.config.signature_dir);
        Ok(serde_json::json!({
            "signatures": names,
            "count": names.len(),
        })
        .to_string())
    }
}

// ===========================================================================
// Helper method (not a tool — used internally by accept/decline/tentative)
// ===========================================================================

impl MoonIoServer {
    async fn respond_calendar_event_inner(
        &self,
        event_uid: &str,
        response: EventResponse,
        comment: Option<&str>,
    ) -> Result<String, String> {
        let cal = self
            .calendar_client(None)
            .ok_or_else(|| "CalDAV non configurato.".to_string())?;

        cal.respond_to_event(event_uid, response, comment)
            .await
            .map_err(|e| format!("Errore: {e}"))?;

        Ok(serde_json::json!({
            "success": true,
            "uid": event_uid,
            "response": response.as_partstat(),
        })
        .to_string())
    }
}

// ===========================================================================
// ServerHandler implementation — MCP protocol
// ===========================================================================

#[tool_handler]
impl ServerHandler for MoonIoServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::default(),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation {
                name: "moon-io".to_string(),
                version: "0.1.0".to_string(),
            },
            instructions: Some(
                "Moon Io - Email MCP server. \
                 Handles email: read, search, send, reply, attachments, \
                 calendar (CalDAV), signatures. \
                 Signature is auto-appended to sent emails."
                    .into(),
            ),
        }
    }
}

// ===========================================================================
// Utilities
// ===========================================================================

/// Format a date string to a short display format.
fn format_date_short(date_str: &str) -> String {
    if let Ok(dt) = DateTime::parse_from_rfc3339(date_str) {
        dt.format("%d/%m/%Y %H:%M").to_string()
    } else {
        date_str.chars().take(16).collect()
    }
}
