// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

//! `MailBackend` — abstract trait over IMAP+SMTP and Gmail REST API.
//!
//! Two implementations:
//! - `EmailClient` in `email_client.rs` — IMAP4rev1 + SMTP, used for Aruba and
//!   any account whose host is not `*.gmail.com`.
//! - `GmailApiBackend` in `gmail_api.rs` — Gmail REST API v1, used when host
//!   matches `gmail.com` (avoids the async-imap XOAUTH2 deadlock on Gmail).
//!
//! Both share the OAuth access-token cache from `oauth::AccessTokenCache`.

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;

use crate::config::{AccountConfig, AuthMode};
use crate::email_client::{
    AttachmentData, AttachmentInfo, BounceInfo, EmailClient, EmailData, SendEmailParams, SendResult,
};
use crate::error::Result;
use crate::oauth::AccessTokenCache;

/// Boxed async closure invoked when the watcher detects new mail.
/// Arc'd because the watcher loop may store and re-invoke it.
pub type NewMailCallback =
    Arc<dyn Fn() -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// Abstract mail backend used by daemon, indexer and MCP server.
#[async_trait]
pub trait MailBackend: Send + Sync {
    /// Account name (e.g. "primary", "gmail").
    fn account_name(&self) -> &str;

    /// Configured "Sent" folder (used by SMTP send to APPEND a copy).
    fn sent_folder(&self) -> &str;

    /// Static label of the backend impl ("imap" or "gmail-api"). For logging.
    fn backend_kind(&self) -> &'static str;

    // ----- Folder discovery -----
    async fn list_folders(&self) -> Result<Vec<String>>;

    // ----- Read -----
    async fn fetch_uids_since(&self, folder: &str, since_uid: u32) -> Result<Vec<u32>>;
    async fn fetch_emails_by_uids(
        &self,
        folder: &str,
        uids: &[u32],
    ) -> Result<Vec<EmailData>>;

    /// Fetch only the server-side `INTERNALDATE` (IMAP) or `internalDate`
    /// (Gmail) for the given UIDs. Returns a map UID → RFC3339. Used by the
    /// `repair_empty_dates` MCP tool to fix points whose `date` payload is
    /// empty without re-downloading the body or re-embedding.
    async fn fetch_internal_dates(
        &self,
        folder: &str,
        uids: &[u32],
    ) -> Result<HashMap<u32, String>>;

    async fn search_uids_filtered(
        &self,
        folder: &str,
        sender: Option<&str>,
        subject_contains: Option<&str>,
        before: Option<&str>,
    ) -> Result<Vec<u32>>;

    // ----- Attachments -----
    async fn get_attachments(
        &self,
        email_uid: u32,
        folder: &str,
    ) -> Result<Vec<AttachmentInfo>>;
    async fn download_attachment(
        &self,
        email_uid: u32,
        attachment_index: usize,
        folder: &str,
    ) -> Result<AttachmentData>;

    // ----- Delete -----
    async fn move_to_trash(&self, source_folder: &str, uid: u32) -> Result<String>;
    async fn bulk_move_to_trash(
        &self,
        source_folder: &str,
        uids: &[u32],
    ) -> Result<(usize, String)>;

    // ----- Send (Fase 2 for Gmail API — IMAP backend implements normally) -----
    async fn send_email(&self, params: SendEmailParams) -> Result<SendResult>;
    async fn reply_email(
        &self,
        email_uid: u32,
        folder: &str,
        body: &str,
        cc: Vec<String>,
        signature_name: &str,
        attachments: Vec<PathBuf>,
    ) -> Result<SendResult>;

    // ----- Misc -----
    async fn check_bounced(
        &self,
        folder: &str,
        limit: usize,
    ) -> Result<Vec<BounceInfo>>;

    /// One iteration of the watcher loop (IMAP IDLE cycle or 60s polling).
    /// Daemon calls this in a reconnect-loop. Returns when something new
    /// triggers the callback or the timeout elapses.
    async fn watch_once(
        &self,
        folder: &str,
        timeout_secs: u64,
        on_new_mail: NewMailCallback,
    ) -> Result<()>;
}

/// Build the right backend implementation for an account.
///
/// - Gmail Workspace with OAuth → `GmailApiBackend` (REST via reqwest)
/// - Anything else → `EmailClient` (IMAP+SMTP via async-imap/lettre)
pub fn build_backend(
    account: AccountConfig,
    signature_dir: PathBuf,
    oauth_cache: AccessTokenCache,
) -> Box<dyn MailBackend> {
    if account.imap_host.contains("gmail.com") && account.auth_mode == AuthMode::OAuth {
        Box::new(crate::gmail_api::GmailApiBackend::new(
            account,
            signature_dir,
            oauth_cache,
        ))
    } else {
        Box::new(EmailClient::new_with_cache(
            account,
            signature_dir,
            oauth_cache,
        ))
    }
}
