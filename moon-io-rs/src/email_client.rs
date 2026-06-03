// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

use std::path::PathBuf;

use base64::Engine as _;
use chrono::{DateTime, FixedOffset};
use futures::StreamExt;
use mail_parser::MimeHeaders;
use regex::Regex;
use tokio_util::compat::TokioAsyncReadCompatExt;
use tracing::{debug, info, warn};

use crate::config::{AccountConfig, AuthMode};
use crate::error::{IoError, Result};
use crate::oauth::{self, AccessTokenCache};
use crate::signature;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct EmailData {
    pub message_id: String,
    pub from: String,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub subject: String,
    pub date: String,
    pub body: String,
    pub in_reply_to: Option<String>,
    pub references: Option<String>,
    pub attachments: Vec<AttachmentInfo>,
    pub uid: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct AttachmentInfo {
    pub index: usize,
    pub filename: String,
    pub content_type: String,
    pub size: usize,
}

#[derive(Debug, Clone)]
pub struct AttachmentData {
    pub filename: String,
    pub content_type: String,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct SendEmailParams {
    pub to: Vec<String>,
    pub subject: String,
    pub body: String,
    pub cc: Vec<String>,
    pub from_name: Option<String>,
    pub attachments: Vec<PathBuf>,
    pub signature_name: String,
    pub in_reply_to: Option<String>,
    pub references: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SendResult {
    pub success: bool,
    pub to: Vec<String>,
    pub subject: String,
    pub saved_to_sent: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct BounceInfo {
    pub message_id: String,
    pub failed_email: String,
    pub reason: String,
    pub date: String,
}

// Type alias for the IMAP session with TLS over compat layer
type ImapSession =
    async_imap::Session<async_native_tls::TlsStream<tokio_util::compat::Compat<tokio::net::TcpStream>>>;

// ---------------------------------------------------------------------------
// EmailClient
// ---------------------------------------------------------------------------

pub struct EmailClient {
    account: AccountConfig,
    signature_dir: PathBuf,
    /// Shared in-process OAuth access-token cache. New `EmailClient`s built
    /// from the same daemon share this so refresh requests aren't duplicated.
    oauth_cache: AccessTokenCache,
}

impl EmailClient {
    pub fn new(account: AccountConfig, signature_dir: PathBuf) -> Self {
        Self {
            account,
            signature_dir,
            oauth_cache: AccessTokenCache::new(),
        }
    }

    /// Like `new` but shares an existing access-token cache (preferred when
    /// many clients are built per indexing batch — avoids refresh hammering).
    pub fn new_with_cache(
        account: AccountConfig,
        signature_dir: PathBuf,
        oauth_cache: AccessTokenCache,
    ) -> Self {
        Self {
            account,
            signature_dir,
            oauth_cache,
        }
    }

    /// Get a valid OAuth access token for this account (refreshes if needed).
    async fn oauth_access_token(&self) -> Result<String> {
        oauth::get_access_token(&self.account.name, &self.oauth_cache)
            .await
            .map_err(|e| IoError::Imap(format!("OAuth access token: {e}")))
    }

    /// Account name (e.g. "primary", "gmail").
    pub fn account_name(&self) -> &str {
        &self.account.name
    }

    /// Configured "Sent" folder for saving copies after send.
    pub fn sent_folder(&self) -> &str {
        &self.account.sent_folder
    }

    // -----------------------------------------------------------------------
    // IMAP connection helpers
    // -----------------------------------------------------------------------

    async fn connect_imap(&self) -> Result<ImapSession> {
        let name = &self.account.name;
        let host = &self.account.imap_host;
        let port = self.account.imap_port;

        debug!("[{name}] connect_imap: TCP connect {host}:{port}");
        let tcp_fut = tokio::net::TcpStream::connect(format!("{host}:{port}"));
        let tcp = tokio::time::timeout(std::time::Duration::from_secs(20), tcp_fut)
            .await
            .map_err(|_| IoError::Imap(format!("TCP connect timeout to {host}:{port}")))?
            .map_err(|e| IoError::Imap(format!("TCP connect: {e}")))?;

        let tcp_compat = tcp.compat();
        debug!("[{name}] connect_imap: TLS handshake");
        let tls = async_native_tls::TlsConnector::new();
        let tls_fut = tls.connect(host.as_str(), tcp_compat);
        let tls_stream = tokio::time::timeout(std::time::Duration::from_secs(20), tls_fut)
            .await
            .map_err(|_| IoError::Imap(format!("TLS timeout to {host}")))?
            .map_err(|e| IoError::Imap(format!("TLS: {e}")))?;

        debug!("[{name}] connect_imap: IMAP client created, starting auth");
        let client = async_imap::Client::new(tls_stream);

        let session = match self.account.auth_mode {
            AuthMode::Password => {
                debug!("[{name}] connect_imap: LOGIN (password)");
                let login_fut = client
                    .login(&self.account.imap_username, &self.account.imap_password);
                tokio::time::timeout(std::time::Duration::from_secs(30), login_fut)
                    .await
                    .map_err(|_| IoError::Imap(format!("LOGIN timeout for {name}")))?
                    .map_err(|e| IoError::Imap(format!("Login: {}", e.0)))?
            }
            AuthMode::OAuth => {
                info!("[{name}] OAuth: refreshing access token");
                let access_token = self.oauth_access_token().await?;
                info!(
                    "[{name}] OAuth: token OK ({}c), sending AUTHENTICATE XOAUTH2",
                    access_token.chars().count()
                );
                let auth = XOauth2 {
                    user: self.account.imap_username.clone(),
                    access_token,
                    first_call: true,
                };
                let auth_fut = client.authenticate("XOAUTH2", auth);
                let session = tokio::time::timeout(
                    std::time::Duration::from_secs(30),
                    auth_fut,
                )
                .await
                .map_err(|_| IoError::Imap(format!("XOAUTH2 timeout for {name}")))?
                .map_err(|e| IoError::Imap(format!("XOAUTH2: {}", e.0)))?;
                info!("[{name}] OAuth: XOAUTH2 authenticated");
                session
            }
        };

        Ok(session)
    }

    /// Collect a stream of Fetch results into a Vec.
    async fn collect_fetch(
        &self,
        mut stream: impl futures::Stream<
                Item = std::result::Result<async_imap::types::Fetch, async_imap::error::Error>,
            > + Unpin,
    ) -> Result<Vec<async_imap::types::Fetch>> {
        let mut results = Vec::new();
        while let Some(item) = stream.next().await {
            let fetch = item.map_err(|e| IoError::Imap(format!("Fetch: {e}")))?;
            results.push(fetch);
        }
        Ok(results)
    }

    // -----------------------------------------------------------------------
    // Folder discovery
    // -----------------------------------------------------------------------

    /// List all IMAP folders on the server.
    pub async fn list_folders(&self) -> Result<Vec<String>> {
        let mut session = self.connect_imap().await?;

        let folders = {
            let folders_stream = session
                .list(None, Some("*"))
                .await
                .map_err(|e| IoError::Imap(format!("LIST: {e}")))?;

            use futures::StreamExt;
            let mut result = Vec::new();
            futures::pin_mut!(folders_stream);
            while let Some(item) = folders_stream.next().await {
                let name = item
                    .map_err(|e| IoError::Imap(format!("LIST item: {e}")))?;
                result.push(name.name().to_string());
            }
            result
        };

        session.logout().await.ok();
        Ok(folders)
    }

    // -----------------------------------------------------------------------
    // Read operations
    // -----------------------------------------------------------------------

    /// List emails (headers only) from a folder.
    pub async fn list_emails(
        &self,
        folder: &str,
        sender_filter: Option<&str>,
        limit: usize,
    ) -> Result<Vec<EmailData>> {
        let mut session = self.connect_imap().await?;
        session
            .select(folder)
            .await
            .map_err(|e| IoError::Imap(format!("SELECT {folder}: {e}")))?;

        let search_cmd = match sender_filter {
            Some(sender) => format!("FROM \"{}\"", sender.replace('"', "")),
            None => "ALL".to_string(),
        };

        let uids = session
            .uid_search(&search_cmd)
            .await
            .map_err(|e| IoError::Imap(format!("SEARCH: {e}")))?;

        let mut uid_list: Vec<u32> = uids.into_iter().collect();
        uid_list.sort();
        uid_list.reverse();

        let fetch_count = uid_list.len().min(limit);
        if fetch_count == 0 {
            session.logout().await.ok();
            return Ok(vec![]);
        }

        let uid_set: String = uid_list[..fetch_count]
            .iter()
            .map(|u| u.to_string())
            .collect::<Vec<_>>()
            .join(",");

        let messages_stream = session
            .uid_fetch(&uid_set, "(UID BODY.PEEK[HEADER] BODY.PEEK[TEXT] INTERNALDATE)")
            .await
            .map_err(|e| IoError::Imap(format!("FETCH: {e}")))?;

        let messages_vec = self.collect_fetch(messages_stream).await?;

        let mut results = Vec::new();
        for msg in &messages_vec {
            if let Some(header) = msg.header() {
                if let Some(parsed) = mail_parser::MessageParser::default().parse(header) {
                    let email = self.parse_mail_header(&parsed, msg.uid, msg.internal_date());
                    results.push(email);
                }
            }
        }

        session.logout().await.ok();
        Ok(results)
    }

    /// Read a single email in full (body + attachments info).
    pub async fn read_email(&self, email_uid: u32, folder: &str) -> Result<EmailData> {
        let mut session = self.connect_imap().await?;
        session
            .select(folder)
            .await
            .map_err(|e| IoError::Imap(format!("SELECT {folder}: {e}")))?;

        let uid_str = email_uid.to_string();
        let messages_stream = session
            .uid_fetch(&uid_str, "(BODY.PEEK[] INTERNALDATE)")
            .await
            .map_err(|e| IoError::Imap(format!("FETCH: {e}")))?;

        let messages_vec = self.collect_fetch(messages_stream).await?;

        let msg = messages_vec
            .first()
            .ok_or_else(|| IoError::NotFound(format!("Email UID {email_uid} not found")))?;

        let internal_date = msg.internal_date();
        let body = msg.body().unwrap_or_default();
        let parsed = mail_parser::MessageParser::default()
            .parse(body)
            .ok_or_else(|| IoError::Parse("Failed to parse email".into()))?;

        let email = self.parse_full_email(&parsed, Some(email_uid), internal_date);

        session.logout().await.ok();
        Ok(email)
    }

    /// Get attachment info for an email.
    pub async fn get_attachments(
        &self,
        email_uid: u32,
        folder: &str,
    ) -> Result<Vec<AttachmentInfo>> {
        let mut session = self.connect_imap().await?;
        session
            .select(folder)
            .await
            .map_err(|e| IoError::Imap(format!("SELECT {folder}: {e}")))?;

        let uid_str = email_uid.to_string();
        let messages_stream = session
            .uid_fetch(&uid_str, "BODY.PEEK[]")
            .await
            .map_err(|e| IoError::Imap(format!("FETCH: {e}")))?;

        let messages_vec = self.collect_fetch(messages_stream).await?;

        let msg = messages_vec
            .first()
            .ok_or_else(|| IoError::NotFound(format!("Email UID {email_uid} not found")))?;

        let body = msg.body().unwrap_or_default();
        let parsed = mail_parser::MessageParser::default()
            .parse(body)
            .ok_or_else(|| IoError::Parse("Failed to parse email".into()))?;

        let attachments = self.extract_attachments_info(&parsed);

        session.logout().await.ok();
        Ok(attachments)
    }

    /// Download a specific attachment by index.
    pub async fn download_attachment(
        &self,
        email_uid: u32,
        attachment_index: usize,
        folder: &str,
    ) -> Result<AttachmentData> {
        let mut session = self.connect_imap().await?;
        session
            .select(folder)
            .await
            .map_err(|e| IoError::Imap(format!("SELECT {folder}: {e}")))?;

        let uid_str = email_uid.to_string();
        let messages_stream = session
            .uid_fetch(&uid_str, "BODY.PEEK[]")
            .await
            .map_err(|e| IoError::Imap(format!("FETCH: {e}")))?;

        let messages_vec = self.collect_fetch(messages_stream).await?;

        let msg = messages_vec
            .first()
            .ok_or_else(|| IoError::NotFound(format!("Email UID {email_uid} not found")))?;

        let raw = msg.body().unwrap_or_default();
        let parsed = mail_parser::MessageParser::default()
            .parse(raw)
            .ok_or_else(|| IoError::Parse("Failed to parse email".into()))?;

        let attachment_indices = &parsed.attachments;

        if attachment_index >= attachment_indices.len() {
            session.logout().await.ok();
            return Err(IoError::NotFound(format!(
                "Attachment index {attachment_index} not found (total: {})",
                attachment_indices.len()
            )));
        }

        let part_idx = attachment_indices[attachment_index];
        let part = &parsed.parts[part_idx];

        let filename = part
            .attachment_name()
            .unwrap_or("attachment")
            .to_string();
        let content_type = part
            .content_type()
            .map(|ct| {
                let main = ct.c_type.as_ref();
                let sub = ct.c_subtype.as_deref().unwrap_or("octet-stream");
                format!("{main}/{sub}")
            })
            .unwrap_or_else(|| "application/octet-stream".into());
        let data = part.contents().to_vec();

        session.logout().await.ok();
        Ok(AttachmentData {
            filename,
            content_type,
            data,
        })
    }

    /// Search emails via IMAP SEARCH.
    pub async fn search_emails(
        &self,
        query: &str,
        folder: &str,
        limit: usize,
    ) -> Result<Vec<EmailData>> {
        let mut session = self.connect_imap().await?;
        session
            .select(folder)
            .await
            .map_err(|e| IoError::Imap(format!("SELECT {folder}: {e}")))?;

        let safe_query = query.replace('"', "");
        let search_cmd = format!(
            "OR SUBJECT \"{}\" BODY \"{}\"",
            safe_query, safe_query
        );

        let uids = session
            .uid_search(&search_cmd)
            .await
            .map_err(|e| IoError::Imap(format!("SEARCH: {e}")))?;

        let mut uid_list: Vec<u32> = uids.into_iter().collect();
        uid_list.sort();
        uid_list.reverse();

        let fetch_count = uid_list.len().min(limit);
        if fetch_count == 0 {
            session.logout().await.ok();
            return Ok(vec![]);
        }

        let uid_set: String = uid_list[..fetch_count]
            .iter()
            .map(|u| u.to_string())
            .collect::<Vec<_>>()
            .join(",");

        let messages_stream = session
            .uid_fetch(&uid_set, "(UID BODY.PEEK[HEADER] INTERNALDATE)")
            .await
            .map_err(|e| IoError::Imap(format!("FETCH: {e}")))?;

        let messages_vec = self.collect_fetch(messages_stream).await?;

        let mut results = Vec::new();
        for msg in &messages_vec {
            if let Some(header) = msg.header() {
                if let Some(parsed) = mail_parser::MessageParser::default().parse(header) {
                    results.push(self.parse_mail_header(&parsed, msg.uid, msg.internal_date()));
                }
            }
        }

        session.logout().await.ok();
        Ok(results)
    }

    // -----------------------------------------------------------------------
    // Send operations
    // -----------------------------------------------------------------------

    /// Send an email via SMTP.
    pub async fn send_email(&self, params: SendEmailParams) -> Result<SendResult> {
        use lettre::message::header::ContentType;
        use lettre::message::{Attachment, MultiPart, SinglePart};
        use lettre::transport::smtp::authentication::Credentials;
        use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

        // Load signature. Per-account first: a signature stored under the
        // account's name wins over the generic `signature_name`. An empty
        // entry under the account name is an explicit opt-out (no signature
        // at all), NOT a fallback trigger.
        let sig_content =
            match signature::get_signature(&self.signature_dir, &self.account.name) {
                Some(s) if s.trim().is_empty() => None,
                Some(s) => Some(s),
                None => signature::get_signature(&self.signature_dir, &params.signature_name),
            };
        let has_html_sig = sig_content
            .as_ref()
            .map(|s| s.trim_start().starts_with('<'))
            .unwrap_or(false);

        // Build body with signature
        let (plain_body, html_body) = if let Some(ref sig) = sig_content {
            if has_html_sig {
                let plain = format!(
                    "{}\n\n-- \n(firma HTML non visualizzabile in testo)",
                    params.body
                );
                let html = format!(
                    "<div style=\"font-family: Roboto, Arial, sans-serif; font-size: 12pt; line-height: 1.5;\">{}</div><br>{sig}",
                    params.body.replace('\n', "<br>")
                );
                (plain, Some(html))
            } else {
                let plain = format!("{}\n\n-- \n{sig}", params.body);
                (plain, None)
            }
        } else {
            (params.body.clone(), None)
        };

        // From address
        let from_name = params.from_name.unwrap_or_else(|| " ".to_string());
        let from_addr = format!("{} <{}>", from_name, self.account.imap_username);

        // Build the message
        let mut builder = Message::builder()
            .from(
                from_addr
                    .parse()
                    .map_err(|e| IoError::Smtp(format!("From: {e}")))?,
            )
            .subject(&params.subject);

        for to in &params.to {
            builder = builder.to(to.parse().map_err(|e| IoError::Smtp(format!("To: {e}")))?);
        }
        for cc in &params.cc {
            builder = builder.cc(cc.parse().map_err(|e| IoError::Smtp(format!("Cc: {e}")))?);
        }

        if let Some(ref reply_to) = params.in_reply_to {
            builder = builder.in_reply_to(reply_to.to_string());
        }
        if let Some(ref refs) = params.references {
            builder = builder.references(refs.to_string());
        }

        // Build MIME body
        let text_part = SinglePart::builder()
            .content_type(ContentType::TEXT_PLAIN)
            .body(plain_body.clone());

        let body_multipart = if let Some(ref html) = html_body {
            let html_part = SinglePart::builder()
                .content_type(ContentType::TEXT_HTML)
                .body(html.clone());
            MultiPart::alternative()
                .singlepart(text_part)
                .singlepart(html_part)
        } else {
            MultiPart::alternative().singlepart(text_part)
        };

        let final_multipart = if params.attachments.is_empty() {
            body_multipart
        } else {
            let mut mixed = MultiPart::mixed().multipart(body_multipart);
            for attach_path in &params.attachments {
                if !attach_path.exists() {
                    warn!("Attachment not found: {}", attach_path.display());
                    continue;
                }
                let filename = attach_path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                let data = tokio::fs::read(attach_path).await.map_err(IoError::Io)?;
                let content_type = ContentType::parse("application/octet-stream")
                    .unwrap_or(ContentType::TEXT_PLAIN);
                let attachment = Attachment::new(filename).body(data, content_type);
                mixed = mixed.singlepart(attachment);
            }
            mixed
        };

        let email_msg = builder
            .multipart(final_multipart)
            .map_err(|e| IoError::Smtp(format!("Build message: {e}")))?;

        let raw_message = email_msg.formatted();

        // Send via SMTP — Password (LOGIN/PLAIN) or OAuth (XOAUTH2)
        use lettre::transport::smtp::authentication::Mechanism;

        let (smtp_secret, smtp_mechanism) = match self.account.auth_mode {
            AuthMode::Password => (
                self.account.imap_password.clone(),
                vec![Mechanism::Plain, Mechanism::Login],
            ),
            AuthMode::OAuth => {
                let token = self.oauth_access_token().await?;
                (token, vec![Mechanism::Xoauth2])
            }
        };

        let creds = Credentials::new(self.account.imap_username.clone(), smtp_secret);

        let mailer = AsyncSmtpTransport::<Tokio1Executor>::relay(&self.account.smtp_host)
            .map_err(|e| IoError::Smtp(format!("Relay: {e}")))?
            .port(self.account.smtp_port)
            .credentials(creds)
            .authentication(smtp_mechanism)
            .build();

        mailer
            .send(email_msg)
            .await
            .map_err(|e| IoError::Smtp(format!("Send: {e}")))?;

        info!(
            "Email sent to {:?}, subject: {}",
            params.to, params.subject
        );

        let saved_to_sent = self
            .save_to_sent(&raw_message)
            .await
            .unwrap_or_else(|e| {
                warn!("Failed to save to Sent: {e}");
                false
            });

        Ok(SendResult {
            success: true,
            to: params.to,
            subject: params.subject,
            saved_to_sent,
            error: None,
        })
    }

    /// Reply to an email (preserves threading headers).
    pub async fn reply_email(
        &self,
        email_uid: u32,
        folder: &str,
        body: &str,
        cc: Vec<String>,
        signature_name: &str,
        attachments: Vec<PathBuf>,
    ) -> Result<SendResult> {
        let original = self.read_email(email_uid, folder).await?;

        let references = {
            let mut refs = original.references.unwrap_or_default();
            if !original.message_id.is_empty() {
                if !refs.is_empty() {
                    refs.push(' ');
                }
                refs.push_str(&original.message_id);
            }
            refs
        };

        let re_regex = Regex::new(r"(?i)^re\s*:\s*").unwrap();
        let subject = if re_regex.is_match(&original.subject) {
            original.subject.clone()
        } else {
            format!("RE: {}", original.subject)
        };

        let reply_to = extract_email_address(&original.from);

        let params = SendEmailParams {
            to: vec![reply_to],
            subject,
            body: body.to_string(),
            cc,
            from_name: Some(" ".to_string()),
            attachments,
            signature_name: signature_name.to_string(),
            in_reply_to: Some(original.message_id.clone()),
            references: Some(references),
        };

        self.send_email(params).await
    }

    // -----------------------------------------------------------------------
    // Bounce detection
    // -----------------------------------------------------------------------

    pub async fn check_bounced(
        &self,
        folder: &str,
        limit: usize,
    ) -> Result<Vec<BounceInfo>> {
        let bounce_patterns = [
            "Mail Delivery Failed",
            "Delivery Status Notification",
            "Undelivered Mail Returned to Sender",
            "Mail delivery failed",
            "Returned mail",
            "Failure Notice",
            "MAILER-DAEMON",
        ];

        let mut session = self.connect_imap().await?;
        session
            .select(folder)
            .await
            .map_err(|e| IoError::Imap(format!("SELECT {folder}: {e}")))?;

        let mut all_uids: Vec<u32> = Vec::new();
        for pattern in &bounce_patterns {
            let safe = pattern.replace('"', "");
            let cmd = format!("OR SUBJECT \"{}\" FROM \"{}\"", safe, safe);
            if let Ok(uids) = session.uid_search(&cmd).await {
                for uid in uids {
                    if !all_uids.contains(&uid) {
                        all_uids.push(uid);
                    }
                }
            }
        }

        all_uids.sort();
        all_uids.reverse();
        let fetch_count = all_uids.len().min(limit);

        if fetch_count == 0 {
            session.logout().await.ok();
            return Ok(vec![]);
        }

        let uid_set: String = all_uids[..fetch_count]
            .iter()
            .map(|u| u.to_string())
            .collect::<Vec<_>>()
            .join(",");

        let messages_stream = session
            .uid_fetch(&uid_set, "(BODY.PEEK[] INTERNALDATE)")
            .await
            .map_err(|e| IoError::Imap(format!("FETCH: {e}")))?;

        let messages_vec = self.collect_fetch(messages_stream).await?;

        let email_regex = Regex::new(r"[\w\.\-]+@[\w\.\-]+\.\w+").unwrap();
        let mut bounces = Vec::new();
        let mut seen_ids = std::collections::HashSet::new();

        for msg in &messages_vec {
            let internal_date = msg.internal_date();
            let raw = msg.body().unwrap_or_default();
            let parsed = match mail_parser::MessageParser::default().parse(raw) {
                Some(p) => p,
                None => continue,
            };

            let message_id = parsed.message_id().unwrap_or("").to_string();
            if !seen_ids.insert(message_id.clone()) {
                continue;
            }

            let body_text = parsed.body_text(0).unwrap_or_default().to_string();

            if let Some(failed_email) = email_regex.find(&body_text) {
                let failed = failed_email.as_str().to_string();
                if failed.to_lowercase() == self.account.imap_username.to_lowercase() {
                    continue;
                }

                let reason = classify_bounce_reason(&body_text);
                let date = parsed
                    .date()
                    .map(|d| d.to_rfc3339())
                    .or_else(|| internal_date.map(|d| d.to_rfc3339()))
                    .unwrap_or_default();

                bounces.push(BounceInfo {
                    message_id,
                    failed_email: failed,
                    reason,
                    date,
                });
            }
        }

        session.logout().await.ok();
        Ok(bounces)
    }

    // -----------------------------------------------------------------------
    // IMAP IDLE (for daemon)
    // -----------------------------------------------------------------------

    /// Watch a folder using IMAP IDLE. Runs one IDLE cycle.
    /// The daemon loop handles reconnection.
    pub async fn idle_watch<F, Fut>(
        &self,
        folder: &str,
        idle_timeout_secs: u64,
        on_new_mail: F,
    ) -> Result<()>
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = ()>,
    {
        let mut session = self.connect_imap().await?;
        session
            .select(folder)
            .await
            .map_err(|e| IoError::Imap(format!("SELECT {folder}: {e}")))?;

        info!("Starting IDLE on {folder}");

        // idle() consumes the session
        let mut idle = session.idle();

        idle.init()
            .await
            .map_err(|e| IoError::Imap(format!("IDLE init: {e}")))?;

        let (idle_wait, _stop) =
            idle.wait_with_timeout(std::time::Duration::from_secs(idle_timeout_secs));

        let response = idle_wait
            .await
            .map_err(|e| IoError::Imap(format!("IDLE wait: {e}")))?;

        // Recover the session and cleanly end IDLE
        let mut session = idle
            .done()
            .await
            .map_err(|e| IoError::Imap(format!("IDLE done: {e}")))?;

        match response {
            async_imap::extensions::idle::IdleResponse::NewData(_) => {
                debug!("IDLE {folder}: new data received");
                on_new_mail().await;
            }
            async_imap::extensions::idle::IdleResponse::Timeout => {
                debug!("IDLE {folder}: timeout");
            }
            async_imap::extensions::idle::IdleResponse::ManualInterrupt => {
                debug!("IDLE {folder}: interrupted");
            }
        }

        session.logout().await.ok();
        Ok(())
    }

    /// Fetch UIDs greater than a given UID (for incremental indexing).
    ///
    /// On Gmail/Workspace, `UID SEARCH ALL` on the first run can hang for
    /// minutes on huge mailboxes (Gmail's All Mail can have 100k+ messages).
    /// We special-case Gmail and use `SINCE` (last 365 days) for the initial
    /// fetch — that's enough for recent email and the bulk indexer will catch
    /// up over time via incremental fetches.
    pub async fn fetch_uids_since(
        &self,
        folder: &str,
        since_uid: u32,
    ) -> Result<Vec<u32>> {
        let name = &self.account.name;
        info!("[{name}] fetch_uids_since: connecting to IMAP for {folder}");
        let mut session = self.connect_imap().await?;
        info!("[{name}] fetch_uids_since: connected, SELECT {folder}");

        let select_fut = session.select(folder);
        tokio::time::timeout(std::time::Duration::from_secs(30), select_fut)
            .await
            .map_err(|_| IoError::Imap(format!("SELECT timeout on {folder}")))?
            .map_err(|e| IoError::Imap(format!("SELECT {folder}: {e}")))?;
        info!("[{name}] fetch_uids_since: SELECT {folder} OK");

        let is_gmail = self.account.imap_host.contains("gmail.com");
        let search_cmd = if since_uid > 0 {
            format!("UID {}:*", since_uid + 1)
        } else if is_gmail {
            // Initial Gmail bulk: limit to last 365 days to keep SEARCH bounded.
            let since = chrono::Utc::now() - chrono::Duration::days(365);
            format!("SINCE {}", since.format("%d-%b-%Y"))
        } else {
            "ALL".to_string()
        };

        info!("[{name}] UID SEARCH on {folder}: {search_cmd}");

        let search_fut = session.uid_search(&search_cmd);
        let uids = tokio::time::timeout(std::time::Duration::from_secs(120), search_fut)
            .await
            .map_err(|_| IoError::Imap(format!("SEARCH timeout on {folder}")))?
            .map_err(|e| IoError::Imap(format!("SEARCH: {e}")))?;

        let mut uid_list: Vec<u32> = uids.into_iter().filter(|&u| u > since_uid).collect();
        uid_list.sort();

        info!(
            "[{name}] UID SEARCH on {folder} returned {} UIDs",
            uid_list.len()
        );

        session.logout().await.ok();
        Ok(uid_list)
    }

    /// Fetch only `INTERNALDATE` for a list of UIDs. Lightweight (no body,
    /// no headers). Returns a map UID → RFC3339. UIDs not present on the
    /// server are silently omitted.
    pub async fn fetch_internal_dates(
        &self,
        folder: &str,
        uids: &[u32],
    ) -> Result<std::collections::HashMap<u32, String>> {
        if uids.is_empty() {
            return Ok(std::collections::HashMap::new());
        }

        let mut session = self.connect_imap().await?;
        session
            .select(folder)
            .await
            .map_err(|e| IoError::Imap(format!("SELECT {folder}: {e}")))?;

        let uid_set: String = uids
            .iter()
            .map(|u| u.to_string())
            .collect::<Vec<_>>()
            .join(",");

        let messages_stream = session
            .uid_fetch(&uid_set, "(UID INTERNALDATE)")
            .await
            .map_err(|e| IoError::Imap(format!("FETCH INTERNALDATE: {e}")))?;

        let messages_vec = self.collect_fetch(messages_stream).await?;

        let mut out = std::collections::HashMap::with_capacity(messages_vec.len());
        for msg in &messages_vec {
            let (Some(uid), Some(idate)) = (msg.uid, msg.internal_date()) else {
                continue;
            };
            out.insert(uid, idate.to_rfc3339());
        }

        session.logout().await.ok();
        Ok(out)
    }

    /// Fetch full email data for a list of UIDs (for indexing).
    pub async fn fetch_emails_by_uids(
        &self,
        folder: &str,
        uids: &[u32],
    ) -> Result<Vec<EmailData>> {
        if uids.is_empty() {
            return Ok(vec![]);
        }

        let mut session = self.connect_imap().await?;
        session
            .select(folder)
            .await
            .map_err(|e| IoError::Imap(format!("SELECT {folder}: {e}")))?;

        let uid_set: String = uids
            .iter()
            .map(|u| u.to_string())
            .collect::<Vec<_>>()
            .join(",");

        let messages_stream = session
            .uid_fetch(&uid_set, "(UID BODY.PEEK[] INTERNALDATE)")
            .await
            .map_err(|e| IoError::Imap(format!("FETCH: {e}")))?;

        let messages_vec = self.collect_fetch(messages_stream).await?;

        let mut results = Vec::new();
        for msg in &messages_vec {
            let internal_date = msg.internal_date();
            let raw = msg.body().unwrap_or_default();
            if let Some(parsed) = mail_parser::MessageParser::default().parse(raw) {
                let email = self.parse_full_email(&parsed, msg.uid, internal_date);
                results.push(email);
            }
        }

        session.logout().await.ok();
        Ok(results)
    }

    // -----------------------------------------------------------------------
    // Private: save to sent
    // -----------------------------------------------------------------------

    // -----------------------------------------------------------------------
    // Delete / move-to-trash
    // -----------------------------------------------------------------------

    /// Auto-discover the Trash folder name on this account.
    /// Looks for case-insensitive matches: "trash", "cestino", "[Gmail]/Trash".
    /// Returns the first match.
    pub async fn find_trash_folder(&self) -> Result<String> {
        let folders = self.list_folders().await?;
        let candidates = ["[Gmail]/Trash", "Trash", "INBOX.Trash", "Cestino", "INBOX.Cestino"];

        // Exact match first
        for c in &candidates {
            if folders.iter().any(|f| f == c) {
                return Ok(c.to_string());
            }
        }
        // Case-insensitive substring match
        for f in &folders {
            let lower = f.to_lowercase();
            if lower.contains("trash") || lower.contains("cestino") {
                return Ok(f.clone());
            }
        }
        Err(IoError::NotFound(
            "Cartella Trash non trovata sull'account".into(),
        ))
    }

    /// Search UIDs in a folder with IMAP criteria. All filters are AND-combined.
    /// `before`: YYYY-MM-DD, matches emails received strictly before this date.
    pub async fn search_uids_filtered(
        &self,
        folder: &str,
        sender: Option<&str>,
        subject_contains: Option<&str>,
        before: Option<&str>,
    ) -> Result<Vec<u32>> {
        let mut session = self.connect_imap().await?;
        session
            .select(folder)
            .await
            .map_err(|e| IoError::Imap(format!("SELECT {folder}: {e}")))?;

        let mut parts: Vec<String> = Vec::new();
        if let Some(s) = sender {
            let safe = s.replace('"', "");
            parts.push(format!("FROM \"{}\"", safe));
        }
        if let Some(s) = subject_contains {
            let safe = s.replace('"', "");
            parts.push(format!("SUBJECT \"{}\"", safe));
        }
        if let Some(d) = before {
            if let Some(imap_date) = format_imap_date(d) {
                parts.push(format!("BEFORE {}", imap_date));
            }
        }
        let cmd = if parts.is_empty() {
            "ALL".to_string()
        } else {
            parts.join(" ")
        };

        let uids = session
            .uid_search(&cmd)
            .await
            .map_err(|e| IoError::Imap(format!("SEARCH ({cmd}): {e}")))?;
        let mut list: Vec<u32> = uids.into_iter().collect();
        list.sort();
        list.reverse();

        session.logout().await.ok();
        Ok(list)
    }

    /// Move a batch of UIDs from a folder to Trash in a single IMAP session.
    /// Returns (count_moved, trash_folder_name).
    pub async fn bulk_move_to_trash(
        &self,
        source_folder: &str,
        uids: &[u32],
    ) -> Result<(usize, String)> {
        if uids.is_empty() {
            return Ok((0, String::new()));
        }
        let trash = self.find_trash_folder().await?;
        let is_in_trash = source_folder.eq_ignore_ascii_case(&trash);

        let mut session = self.connect_imap().await?;
        session
            .select(source_folder)
            .await
            .map_err(|e| IoError::Imap(format!("SELECT {source_folder}: {e}")))?;

        // IMAP supports comma-separated UID lists, but with thousands of UIDs the
        // command line can get huge. Chunk to keep each command < 16K bytes.
        const CHUNK: usize = 500;
        let mut moved = 0usize;

        for chunk in uids.chunks(CHUNK) {
            let uid_set: String = chunk
                .iter()
                .map(|u| u.to_string())
                .collect::<Vec<_>>()
                .join(",");

            if !is_in_trash {
                session
                    .uid_copy(&uid_set, &trash)
                    .await
                    .map_err(|e| IoError::Imap(format!("UID COPY chunk: {e}")))?;
            }

            {
                let store_stream = session
                    .uid_store(&uid_set, "+FLAGS (\\Deleted)")
                    .await
                    .map_err(|e| IoError::Imap(format!("UID STORE chunk: {e}")))?;
                let _ = self.collect_fetch(store_stream).await;
            }

            moved += chunk.len();
        }

        // Single EXPUNGE at the end (covers all chunks)
        {
            let expunge_stream = session
                .expunge()
                .await
                .map_err(|e| IoError::Imap(format!("EXPUNGE: {e}")))?;
            use futures::StreamExt;
            futures::pin_mut!(expunge_stream);
            while expunge_stream.next().await.is_some() {}
        }

        session.logout().await.ok();
        info!(
            "[{}] bulk_move_to_trash: moved {moved} UIDs from {source_folder} to {trash}",
            self.account.name
        );
        Ok((moved, trash))
    }

    /// Move an email to the Trash folder.
    /// IMAP-universal flow: COPY to Trash + set \Deleted flag + EXPUNGE.
    /// Returns the trash folder name used.
    pub async fn move_to_trash(&self, source_folder: &str, uid: u32) -> Result<String> {
        let trash = self.find_trash_folder().await?;

        // If we're already in Trash, hard-delete (EXPUNGE) instead of looping.
        let is_in_trash = source_folder.eq_ignore_ascii_case(&trash);

        let mut session = self.connect_imap().await?;
        session
            .select(source_folder)
            .await
            .map_err(|e| IoError::Imap(format!("SELECT {source_folder}: {e}")))?;

        let uid_str = uid.to_string();

        // COPY to Trash (skip if we're already there)
        if !is_in_trash {
            session
                .uid_copy(&uid_str, &trash)
                .await
                .map_err(|e| IoError::Imap(format!("UID COPY {uid} → {trash}: {e}")))?;
        }

        // Mark \Deleted (scoped so the fetch stream is dropped before next borrow)
        {
            let store_stream = session
                .uid_store(&uid_str, "+FLAGS (\\Deleted)")
                .await
                .map_err(|e| IoError::Imap(format!("UID STORE: {e}")))?;
            let _ = self.collect_fetch(store_stream).await;
        }

        // EXPUNGE (also scoped — expunge_stream borrows session mutably)
        {
            let expunge_stream = session
                .expunge()
                .await
                .map_err(|e| IoError::Imap(format!("EXPUNGE: {e}")))?;
            use futures::StreamExt;
            futures::pin_mut!(expunge_stream);
            while expunge_stream.next().await.is_some() {}
        }

        session.logout().await.ok();
        info!(
            "[{}] Moved UID {uid} from {source_folder} to {trash}",
            self.account.name
        );
        Ok(trash)
    }

    async fn save_to_sent(&self, raw_message: &[u8]) -> Result<bool> {
        let mut session = self.connect_imap().await?;

        let folder = self.account.sent_folder.as_str();
        let result = session.append(folder, None, None, raw_message).await;

        match result {
            Ok(_) => {
                debug!("Saved copy to {folder}");
                session.logout().await.ok();
                Ok(true)
            }
            Err(e) => {
                warn!("Failed to APPEND to {folder}: {e}");
                session.logout().await.ok();
                Ok(false)
            }
        }
    }

    // -----------------------------------------------------------------------
    // Private: parsing helpers
    // -----------------------------------------------------------------------

    fn parse_mail_header(
        &self,
        parsed: &mail_parser::Message<'_>,
        uid: Option<u32>,
        internal_date: Option<DateTime<FixedOffset>>,
    ) -> EmailData {
        let from = parsed
            .from()
            .and_then(|f| f.first())
            .map(|a| {
                let name = a.name().unwrap_or("");
                let addr = a.address().unwrap_or("");
                if name.is_empty() {
                    addr.to_string()
                } else {
                    format!("{name} <{addr}>")
                }
            })
            .unwrap_or_default();

        let to = parsed
            .to()
            .map(|list| {
                list.iter()
                    .map(|a| a.address().unwrap_or("").to_string())
                    .collect()
            })
            .unwrap_or_default();

        let cc = parsed
            .cc()
            .map(|list| {
                list.iter()
                    .map(|a| a.address().unwrap_or("").to_string())
                    .collect()
            })
            .unwrap_or_default();

        let subject = parsed
            .subject()
            .unwrap_or("(nessun oggetto)")
            .to_string();

        // Header Date often missing/malformed on bulk newsletters and certain
        // mailer-daemon bounces — fall back to IMAP INTERNALDATE (always present).
        let date = parsed
            .date()
            .map(|d| d.to_rfc3339())
            .or_else(|| internal_date.map(|d| d.to_rfc3339()))
            .unwrap_or_default();

        let message_id = parsed.message_id().unwrap_or("").to_string();

        let in_reply_to = parsed.in_reply_to().as_text().map(|s| s.to_string());

        let references = parsed
            .references()
            .as_text_list()
            .map(|list| {
                list.iter()
                    .map(|s| s.as_ref() as &str)
                    .collect::<Vec<&str>>()
                    .join(" ")
            });

        EmailData {
            message_id,
            from,
            to,
            cc,
            subject,
            date,
            body: String::new(),
            in_reply_to,
            references,
            attachments: vec![],
            uid,
        }
    }

    fn parse_full_email(
        &self,
        parsed: &mail_parser::Message<'_>,
        uid: Option<u32>,
        internal_date: Option<DateTime<FixedOffset>>,
    ) -> EmailData {
        let mut email = self.parse_mail_header(parsed, uid, internal_date);

        let body = parsed
            .body_text(0)
            .map(|s| s.to_string())
            .or_else(|| parsed.body_html(0).map(|html| html_to_text(&html)))
            .unwrap_or_default();

        email.body = body.chars().take(5000).collect();
        email.attachments = self.extract_attachments_info(parsed);

        email
    }

    fn extract_attachments_info(
        &self,
        parsed: &mail_parser::Message<'_>,
    ) -> Vec<AttachmentInfo> {
        let mut attachments = Vec::new();

        for (index, &part_idx) in parsed.attachments.iter().enumerate() {
            let part = &parsed.parts[part_idx];

            let filename = part
                .attachment_name()
                .unwrap_or("attachment")
                .to_string();
            let content_type = part
                .content_type()
                .map(|ct| {
                    let main = ct.c_type.as_ref();
                    let sub = ct.c_subtype.as_deref().unwrap_or("octet-stream");
                    format!("{main}/{sub}")
                })
                .unwrap_or_else(|| "application/octet-stream".into());
            let size = part.contents().len();

            attachments.push(AttachmentInfo {
                index,
                filename,
                content_type,
                size,
            });
        }

        attachments
    }
}

// ---------------------------------------------------------------------------
// Utility functions
// ---------------------------------------------------------------------------

fn html_to_text(html: &str) -> String {
    let tag_re = Regex::new(r"<[^>]+>").unwrap();
    let text = tag_re.replace_all(html, " ");
    let text = text.replace("&nbsp;", " ");
    let text = text.replace("&amp;", "&");
    let text = text.replace("&lt;", "<");
    let text = text.replace("&gt;", ">");
    let text = text.replace("&quot;", "\"");
    let ws_re = Regex::new(r"\s+").unwrap();
    ws_re.replace_all(&text, " ").trim().to_string()
}

pub(crate) fn extract_email_address(from: &str) -> String {
    let re = Regex::new(r"<([^>]+)>").unwrap();
    if let Some(cap) = re.captures(from) {
        cap[1].to_string()
    } else {
        from.trim().to_string()
    }
}

/// SASL XOAUTH2 authenticator for IMAP. async-imap will encode the response
/// as base64 itself, so we return raw bytes.
///
/// IMPORTANT: SASL XOAUTH2 is special — on auth failure the server replies
/// with a base64 JSON error as a "challenge", and the client MUST respond
/// with an EMPTY line to abort the exchange. If we keep sending the same
/// auth string, the server hangs forever waiting. We track `first_call` to
/// emit the XOAUTH2 init only on the first process() invocation.
struct XOauth2 {
    user: String,
    access_token: String,
    first_call: bool,
}

impl async_imap::Authenticator for XOauth2 {
    type Response = Vec<u8>;
    fn process(&mut self, challenge: &[u8]) -> Self::Response {
        if self.first_call {
            self.first_call = false;
            format!(
                "user={}\x01auth=Bearer {}\x01\x01",
                self.user, self.access_token
            )
            .into_bytes()
        } else {
            // Second call → server sent an error challenge.
            // Log it for debugging, then return empty to abort.
            if !challenge.is_empty() {
                let decoded = base64::engine::general_purpose::STANDARD
                    .decode(challenge)
                    .ok()
                    .and_then(|b| String::from_utf8(b).ok())
                    .unwrap_or_else(|| String::from_utf8_lossy(challenge).into_owned());
                tracing::warn!("XOAUTH2 server challenge after auth: {decoded}");
            }
            Vec::new()
        }
    }
}

/// Convert "YYYY-MM-DD" to IMAP date format "DD-Mon-YYYY".
/// Returns None on parse failure.
fn format_imap_date(yyyy_mm_dd: &str) -> Option<String> {
    let parsed = chrono::NaiveDate::parse_from_str(yyyy_mm_dd, "%Y-%m-%d").ok()?;
    Some(parsed.format("%d-%b-%Y").to_string())
}

pub(crate) fn classify_bounce_reason(body: &str) -> String {
    let body_lower = body.to_lowercase();
    if body_lower.contains("user unknown") || body_lower.contains("mailbox not found") {
        "Mailbox does not exist".to_string()
    } else if body_lower.contains("quota exceeded") || body_lower.contains("mailbox full") {
        "Mailbox full".to_string()
    } else if body_lower.contains("rejected") || body_lower.contains("spam") {
        "Rejected by server".to_string()
    } else if body_lower.contains("temporary") || body_lower.contains("try again") {
        "Temporary failure".to_string()
    } else {
        "Delivery failed".to_string()
    }
}

// ===========================================================================
// MailBackend trait impl (glue layer over the existing IMAP+SMTP methods)
// ===========================================================================

#[async_trait::async_trait]
impl crate::mail_backend::MailBackend for EmailClient {
    fn account_name(&self) -> &str {
        EmailClient::account_name(self)
    }

    fn sent_folder(&self) -> &str {
        EmailClient::sent_folder(self)
    }

    fn backend_kind(&self) -> &'static str {
        "imap"
    }

    async fn list_folders(&self) -> Result<Vec<String>> {
        EmailClient::list_folders(self).await
    }

    async fn fetch_uids_since(&self, folder: &str, since_uid: u32) -> Result<Vec<u32>> {
        EmailClient::fetch_uids_since(self, folder, since_uid).await
    }

    async fn fetch_emails_by_uids(
        &self,
        folder: &str,
        uids: &[u32],
    ) -> Result<Vec<EmailData>> {
        EmailClient::fetch_emails_by_uids(self, folder, uids).await
    }

    async fn fetch_internal_dates(
        &self,
        folder: &str,
        uids: &[u32],
    ) -> Result<std::collections::HashMap<u32, String>> {
        EmailClient::fetch_internal_dates(self, folder, uids).await
    }

    async fn search_uids_filtered(
        &self,
        folder: &str,
        sender: Option<&str>,
        subject_contains: Option<&str>,
        before: Option<&str>,
    ) -> Result<Vec<u32>> {
        EmailClient::search_uids_filtered(self, folder, sender, subject_contains, before).await
    }

    async fn get_attachments(
        &self,
        email_uid: u32,
        folder: &str,
    ) -> Result<Vec<AttachmentInfo>> {
        EmailClient::get_attachments(self, email_uid, folder).await
    }

    async fn download_attachment(
        &self,
        email_uid: u32,
        attachment_index: usize,
        folder: &str,
    ) -> Result<AttachmentData> {
        EmailClient::download_attachment(self, email_uid, attachment_index, folder).await
    }

    async fn move_to_trash(&self, source_folder: &str, uid: u32) -> Result<String> {
        EmailClient::move_to_trash(self, source_folder, uid).await
    }

    async fn bulk_move_to_trash(
        &self,
        source_folder: &str,
        uids: &[u32],
    ) -> Result<(usize, String)> {
        EmailClient::bulk_move_to_trash(self, source_folder, uids).await
    }

    async fn send_email(&self, params: SendEmailParams) -> Result<SendResult> {
        EmailClient::send_email(self, params).await
    }

    async fn reply_email(
        &self,
        email_uid: u32,
        folder: &str,
        body: &str,
        cc: Vec<String>,
        signature_name: &str,
        attachments: Vec<PathBuf>,
    ) -> Result<SendResult> {
        EmailClient::reply_email(self, email_uid, folder, body, cc, signature_name, attachments)
            .await
    }

    async fn check_bounced(&self, folder: &str, limit: usize) -> Result<Vec<BounceInfo>> {
        EmailClient::check_bounced(self, folder, limit).await
    }

    async fn watch_once(
        &self,
        folder: &str,
        timeout_secs: u64,
        on_new_mail: crate::mail_backend::NewMailCallback,
    ) -> Result<()> {
        // Adapt the Arc'd boxed callback to the generic-closure signature of
        // the existing `idle_watch` implementation.
        EmailClient::idle_watch(self, folder, timeout_secs, move || {
            let cb = on_new_mail.clone();
            async move { cb().await }
        })
        .await
    }
}
