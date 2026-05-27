pub mod telegram;

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use jupiteros_shared::store::{IndexResult, MessageStore};
use serde::{Deserialize, Serialize};

use crate::error::Result;

/// Contact information returned by list_contacts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contact {
    pub name: String,
    pub id: i64,
    pub contact_type: ContactType,
    pub username: Option<String>,
}

/// Type of messaging entity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ContactType {
    User,
    Group,
    Channel,
}

impl std::fmt::Display for ContactType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContactType::User => write!(f, "user"),
            ContactType::Group => write!(f, "group"),
            ContactType::Channel => write!(f, "channel"),
        }
    }
}

/// A resolved messaging entity (chat, user, channel).
#[derive(Debug, Clone)]
pub struct Entity {
    pub id: i64,
    pub name: String,
    pub entity_type: ContactType,
}

/// Abstraction over messaging platforms (Telegram, WhatsApp, Discord).
///
/// Each adapter implements this trait. Moon Europa's server calls the trait
/// methods, unaware of the underlying platform.
#[async_trait]
pub trait MessagingAdapter: Send + Sync {
    /// Channel name: "telegram", "whatsapp", etc.
    fn channel_name(&self) -> &str;

    /// Initialize connection to the messaging service.
    async fn connect(&mut self) -> Result<()>;

    /// Clean up connection.
    async fn disconnect(&mut self) -> Result<()>;

    /// List available chats/contacts.
    async fn list_contacts(&self) -> Result<Vec<Contact>>;

    /// Send a message (text + optional file attachments).
    async fn send_message(
        &self,
        to: &str,
        message: &str,
        attachments: &[PathBuf],
    ) -> Result<String>;

    /// Send a message and also index it in the vector store.
    async fn send_message_and_index(
        &self,
        to: &str,
        message: &str,
        attachments: &[PathBuf],
        store: &MessageStore,
    ) -> Result<String>;

    /// Download media/files from a chat.
    async fn save_media(
        &self,
        chat: &str,
        message_id: Option<i64>,
        last_n: usize,
        save_dir: &Path,
    ) -> Result<Vec<PathBuf>>;

    /// Resolve configured entities (folders, chat IDs).
    async fn resolve_entities(&self) -> Result<Vec<Entity>>;

    /// Bulk-index chat history into the vector store.
    async fn bulk_index(
        &self,
        store: &MessageStore,
        entity: &Entity,
    ) -> Result<IndexResult>;

    /// Start real-time listener (blocks forever).
    /// Indexes new/edited messages as they arrive.
    async fn start_listener(
        &self,
        store: &MessageStore,
        entities: &[Entity],
    ) -> Result<()>;
}
