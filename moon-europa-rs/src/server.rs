use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;

use chrono::Utc;
use jupiteros_shared::store::{MessageData, MessageStore};
use rmcp::handler::server::tool::{Parameters, ToolRouter};
use rmcp::model::*;
use rmcp::{tool, tool_handler, tool_router, ServerHandler};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::adapters::telegram::TelegramAdapter;
use crate::adapters::MessagingAdapter;
use crate::config::Config;

/// Moon Europa MCP Server.
///
/// Exposes 10 tools for messaging operations (Telegram, future WhatsApp/Discord).
pub struct EuropaServer {
    store: Arc<MessageStore>,
    adapter: Arc<TelegramAdapter>,
    config: Config,
    tool_router: ToolRouter<Self>,
}

// ---- Parameter structs ----

#[derive(Serialize, Deserialize, JsonSchema)]
struct ListMessagesParams {
    /// Channel: telegram (default)
    channel: Option<String>,
    /// Max results (1-200, default 20)
    limit: Option<u32>,
    /// Filter by contact name
    contact: Option<String>,
    /// Pagination offset
    offset: Option<u32>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
struct ReadMessagesParams {
    /// Channel: telegram (default)
    channel: Option<String>,
    /// Max results (1-5, default 5)
    limit: Option<u32>,
    /// Filter by contact name
    contact: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
struct SearchMessagesParams {
    /// Search query
    query: String,
    /// Channel filter (empty = all)
    channel: Option<String>,
    /// Max results (1-50, default 10)
    limit: Option<u32>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
struct SearchByContactParams {
    /// Contact name to search
    contact: String,
    /// Channel filter (empty = all)
    channel: Option<String>,
    /// Max results (1-50, default 20)
    limit: Option<u32>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
struct SendMessageParams {
    /// Channel: telegram (default)
    channel: Option<String>,
    /// Recipient: name, @username, or ID
    to: String,
    /// Message text
    message: Option<String>,
    /// Comma-separated file paths to attach
    attachments: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
struct ListContactsParams {
    /// Channel: telegram (default)
    channel: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
struct SaveMediaParams {
    /// Channel: telegram (default)
    channel: Option<String>,
    /// Chat name, @username, or ID
    chat: String,
    /// Specific message ID (0 = use last_n)
    message_id: Option<i64>,
    /// Download last N messages with media (1-10, default 1)
    last_n: Option<u32>,
    /// Save directory (default: ~/Downloads)
    save_dir: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
struct ExtractFileTextParams {
    /// Path to the file
    file_path: String,
    /// Max characters to extract (500-25000, default 15000)
    max_chars: Option<u32>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
struct IndexMessageParams {
    /// Channel: telegram
    channel: Option<String>,
    /// Unique message ID
    message_id: String,
    /// Message text content
    text: String,
    /// Sender name
    sender: Option<String>,
    /// Recipient/chat name
    recipient: Option<String>,
    /// Date (ISO format)
    date: Option<String>,
}

// ---- Tool implementations ----

#[tool_router]
impl EuropaServer {
    pub fn new(store: Arc<MessageStore>, adapter: Arc<TelegramAdapter>, config: Config) -> Self {
        Self {
            store,
            adapter,
            config,
            tool_router: Self::tool_router(),
        }
    }

    /// List message headers (date, sender, chat, preview). Fast, metadata only.
    #[tool(
        name = "list_messages",
        description = "List message headers (date, sender, chat, preview). Use for overview. Max 200."
    )]
    async fn list_messages(
        &self,
        params: Parameters<ListMessagesParams>,
    ) -> Result<String, String> {
        let p = params.0;
        let limit = p.limit.unwrap_or(20).clamp(1, 200) as usize;
        let offset = p.offset.unwrap_or(0) as usize;
        let contact_ref = p.contact.as_deref().filter(|s| !s.is_empty());

        let headers = self
            .store
            .list_messages(limit, contact_ref, offset, None)
            .await
            .map_err(|e| format!("List messages: {e}"))?;

        if headers.is_empty() {
            return Ok("No messages found.".to_string());
        }

        let mut output = format!(
            "Messages ({} results):\n{}\n",
            headers.len(),
            "-".repeat(110)
        );

        for h in &headers {
            let date_short = &h.date.get(..16).unwrap_or(&h.date);
            let preview_clean: String = h
                .preview
                .chars()
                .filter(|c| !c.is_control())
                .take(80)
                .collect();
            output.push_str(&format!(
                "{} | {} -> {} | {}\n",
                date_short, h.sender, h.recipient, preview_clean
            ));
        }

        Ok(output)
    }

    /// Read full message bodies. Max 5.
    #[tool(
        name = "read_messages",
        description = "Read full message bodies. Max 5 messages. Use only when user asks to READ content."
    )]
    async fn read_messages(
        &self,
        params: Parameters<ReadMessagesParams>,
    ) -> Result<String, String> {
        let p = params.0;
        let limit = p.limit.unwrap_or(5).clamp(1, 5) as usize;
        let contact_ref = p.contact.as_deref().filter(|s| !s.is_empty());

        let headers = self
            .store
            .list_messages(50, contact_ref, 0, None)
            .await
            .map_err(|e| format!("Read messages: {e}"))?;

        let messages: Vec<_> = headers.into_iter().take(limit).collect();

        if messages.is_empty() {
            return Ok("No messages found.".to_string());
        }

        let total_budget = 25_000usize;
        let per_msg_budget = (total_budget - limit * 150) / messages.len().max(1);

        let mut output = format!("Messages ({} results):\n\n", messages.len());

        for (i, m) in messages.iter().enumerate() {
            let text: String = m.preview.chars().take(per_msg_budget).collect();
            output.push_str(&format!(
                "--- Message {} ---\nFrom: {}\nChat: {}\nDate: {}\n\n{}\n\n",
                i + 1,
                m.sender,
                m.recipient,
                m.date,
                text
            ));
        }

        Ok(output)
    }

    /// Semantic search across messages.
    #[tool(
        name = "search_messages",
        description = "Search messages by content (semantic + keyword). Returns results with similarity scores."
    )]
    async fn search_messages(
        &self,
        params: Parameters<SearchMessagesParams>,
    ) -> Result<String, String> {
        let p = params.0;
        let limit = p.limit.unwrap_or(10).clamp(1, 50) as usize;

        let results = self
            .store
            .search(&p.query, None, limit, None)
            .await
            .map_err(|e| format!("Search: {e}"))?;

        if results.is_empty() {
            return Ok(format!("No results for: '{}'", p.query));
        }

        let mut output = format!(
            "Search results for '{}' ({} found):\n\n",
            p.query,
            results.len()
        );

        for (i, r) in results.iter().enumerate() {
            let sender = r.metadata.get("sender").cloned().unwrap_or_default();
            let date = r.metadata.get("date").cloned().unwrap_or_default();
            let preview: String = r.text.chars().take(300).collect();
            output.push_str(&format!(
                "{}. [score: {:.2}] {} | {}\n   {}\n\n",
                i + 1,
                r.score,
                sender,
                &date.get(..16).unwrap_or(&date),
                preview
            ));
        }

        Ok(output)
    }

    /// Find all messages with a specific contact.
    #[tool(
        name = "search_by_contact",
        description = "Find all messages with a specific person (sender or recipient)."
    )]
    async fn search_by_contact(
        &self,
        params: Parameters<SearchByContactParams>,
    ) -> Result<String, String> {
        let p = params.0;
        let limit = p.limit.unwrap_or(20).clamp(1, 50) as usize;

        let results = self
            .store
            .search_by_contact(&p.contact, limit, None)
            .await
            .map_err(|e| format!("Search by contact: {e}"))?;

        if results.is_empty() {
            return Ok(format!("No messages found with: '{}'", p.contact));
        }

        let mut output = format!(
            "Messages with '{}' ({} found):\n{}\n",
            p.contact,
            results.len(),
            "-".repeat(110)
        );

        for r in &results {
            let sender = r.metadata.get("sender").cloned().unwrap_or_default();
            let date = r.metadata.get("date").cloned().unwrap_or_default();
            let preview: String = r
                .text
                .chars()
                .filter(|c| !c.is_control())
                .take(100)
                .collect();
            output.push_str(&format!(
                "{} | {} | {}\n",
                &date.get(..16).unwrap_or(&date),
                sender,
                preview
            ));
        }

        Ok(output)
    }

    /// Send a message on Telegram.
    #[tool(
        name = "send_message",
        description = "Send a message on Telegram. IMPORTANT: Always show draft and get user confirmation before sending!"
    )]
    async fn send_message(
        &self,
        params: Parameters<SendMessageParams>,
    ) -> Result<String, String> {
        let p = params.0;
        let message = p.message.unwrap_or_default();
        let attachment_paths: Vec<PathBuf> = p
            .attachments
            .unwrap_or_default()
            .split(',')
            .map(|s| PathBuf::from(s.trim()))
            .filter(|p| !p.as_os_str().is_empty())
            .collect();

        let result = self
            .adapter
            .send_message_and_index(&p.to, &message, &attachment_paths, &self.store)
            .await
            .map_err(|e| format!("Send: {e}"))?;

        Ok(result)
    }

    /// List available Telegram contacts/chats.
    #[tool(
        name = "list_contacts",
        description = "List available Telegram contacts and chats."
    )]
    async fn list_contacts(
        &self,
        params: Parameters<ListContactsParams>,
    ) -> Result<String, String> {
        let _p = params.0;
        let contacts = self
            .adapter
            .list_contacts()
            .await
            .map_err(|e| format!("List contacts: {e}"))?;

        if contacts.is_empty() {
            return Ok("No contacts found.".to_string());
        }

        let mut output = format!("Contacts ({}):\n{}\n", contacts.len(), "-".repeat(80));

        for c in &contacts {
            let username = c.username.as_deref().unwrap_or("-");
            output.push_str(&format!(
                "{} | ID: {} | Type: {} | @{}\n",
                c.name, c.id, c.contact_type, username
            ));
        }

        Ok(output)
    }

    /// Download media/files from a Telegram chat.
    #[tool(
        name = "save_media",
        description = "Download photos, documents, videos from a Telegram chat."
    )]
    async fn save_media(
        &self,
        params: Parameters<SaveMediaParams>,
    ) -> Result<String, String> {
        let p = params.0;
        let last_n = p.last_n.unwrap_or(1).clamp(1, 10) as usize;
        let save_dir = p
            .save_dir
            .map(PathBuf::from)
            .unwrap_or_else(|| dirs::download_dir().unwrap_or_else(|| PathBuf::from(".")));

        let msg_id = p.message_id.filter(|id| *id > 0);

        let files = self
            .adapter
            .save_media(&p.chat, msg_id, last_n, &save_dir)
            .await
            .map_err(|e| format!("Save media: {e}"))?;

        if files.is_empty() {
            return Ok("No media found to download.".to_string());
        }

        let mut output = format!("Downloaded {} file(s):\n", files.len());
        for f in &files {
            output.push_str(&format!("  - {}\n", f.display()));
        }

        Ok(output)
    }

    /// Extract text from a file.
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
        let max_chars = p.max_chars.unwrap_or(15000) as usize;

        let text = jupiteros_shared::file_utils::extract_text(&path, max_chars)
            .map_err(|e| format!("Extract: {e}"))?;

        Ok(text)
    }

    /// Get message database statistics.
    #[tool(
        name = "get_stats",
        description = "Get message database statistics: total count, count per channel."
    )]
    async fn get_stats(&self) -> Result<String, String> {
        let stats = self
            .store
            .get_stats()
            .await
            .map_err(|e| format!("Stats: {e}"))?;

        let mut output = String::from("Message database stats:\n");
        for (channel, count) in &stats.by_channel {
            output.push_str(&format!("  {}: {}\n", channel, count));
        }
        output.push_str(&format!("  Total: {}\n", stats.total));

        Ok(output)
    }

    /// Manually index a single message.
    #[tool(
        name = "index_message",
        description = "Manually add a message to the search database."
    )]
    async fn index_message(
        &self,
        params: Parameters<IndexMessageParams>,
    ) -> Result<String, String> {
        let p = params.0;
        let channel = p.channel.unwrap_or_else(|| "telegram".into());

        let msg = MessageData {
            id: p.message_id.clone(),
            text: p.text,
            sender: p.sender.unwrap_or_default(),
            recipient: p.recipient.unwrap_or_default(),
            subject: String::new(),
            date: p.date.unwrap_or_else(|| Utc::now().to_rfc3339()),
            channel,
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
}

// Implement ServerHandler for MCP protocol
#[tool_handler]
impl ServerHandler for EuropaServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::default(),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation {
                name: "moon-europa".to_string(),
                version: "0.1.0".to_string(),
            },
            instructions: Some(
                "Moon Europa - Messaging MCP server. \
                 Handles Telegram messages: read, search, send, contacts, media download."
                    .into(),
            ),
        }
    }
}
