use std::sync::Arc;
use std::time::Duration;

use jupiteros_shared::store::MessageStore;
use tracing::{error, info, warn};

use crate::adapters::telegram::IndexerState;
use crate::adapters::MessagingAdapter;
use crate::config::Config;
use crate::error::Result;

/// Background indexer daemon for a single messaging channel.
///
/// Two phases:
/// 1. Bulk indexing - Import all message history (incremental, stateful)
/// 2. Real-time listener - Index new messages as they arrive (blocks forever)
///
/// Auto-restarts on crash after a configurable delay. Works for any adapter
/// implementing [`MessagingAdapter`] (Telegram, Slack, Teams, ...).
pub struct ChannelDaemon {
    adapter: Arc<dyn MessagingAdapter>,
    store: Arc<MessageStore>,
    config: Config,
}

impl ChannelDaemon {
    pub fn new(
        adapter: Arc<dyn MessagingAdapter>,
        store: Arc<MessageStore>,
        config: Config,
    ) -> Self {
        Self {
            adapter,
            store,
            config,
        }
    }

    /// Run the daemon with automatic restart on crash.
    ///
    /// This method never returns under normal operation.
    pub async fn run_forever(&self) {
        loop {
            match self.run_once().await {
                Ok(()) => {
                    // Listener returned cleanly (shouldn't happen)
                    warn!("Daemon returned unexpectedly, restarting...");
                }
                Err(e) => {
                    error!("Daemon crashed: {}", e);
                }
            }

            info!(
                "Restarting daemon in {} seconds...",
                self.config.reconnect_delay_secs
            );
            tokio::time::sleep(Duration::from_secs(self.config.reconnect_delay_secs)).await;
        }
    }

    /// Run one full cycle: bulk index + listener.
    async fn run_once(&self) -> Result<()> {
        // Sanity check: detect Qdrant data loss vs stale indexer state.
        // If Qdrant was restarted with empty storage, the state file still has
        // high-water message IDs and the daemon would think "nothing to index".
        let state_path = self.config.data_dir.join("indexer_state.json");
        let state = IndexerState::load(&state_path);
        let max_claimed_id: i32 = state
            .last_message_id
            .values()
            .copied()
            .max()
            .unwrap_or(0);

        if max_claimed_id > 0 {
            match self.store.get_stats().await {
                Ok(stats) => {
                    let expected_min = (max_claimed_id as usize) / 20;
                    if stats.total < expected_min && expected_min > 50 {
                        warn!(
                            "DATA LOSS DETECTED: indexer state claims msg_id up to {} \
                             but Qdrant only has {} points (expected >= {}). \
                             Resetting indexer state for full re-index.",
                            max_claimed_id, stats.total, expected_min
                        );
                        let empty = IndexerState::default();
                        empty.save(&state_path);
                    } else {
                        info!(
                            "Qdrant sanity check OK: {} points, state max msg_id={}",
                            stats.total, max_claimed_id
                        );
                    }
                }
                Err(e) => {
                    warn!("Could not verify Qdrant data integrity: {e}");
                }
            }
        }

        // Phase 1: Bulk indexing
        info!("Phase 1: Bulk indexing...");
        let entities = self.adapter.resolve_entities().await?;

        for entity in &entities {
            match self.adapter.bulk_index(&self.store, entity).await {
                Ok(result) => {
                    info!(
                        "  '{}': {} indexed, {} skipped",
                        entity.name, result.indexed, result.skipped
                    );
                }
                Err(e) => {
                    warn!("  '{}': bulk index failed: {}", entity.name, e);
                }
            }
            // Anti-flood: wait between entities to avoid Telegram rate limits
            tokio::time::sleep(Duration::from_secs(2)).await;
        }

        // Log stats after bulk
        match self.store.get_stats().await {
            Ok(stats) => info!("DB stats after bulk: {} total messages", stats.total),
            Err(e) => warn!("Failed to get stats: {}", e),
        }

        // Phase 2: Real-time listener (blocks forever)
        info!("Phase 2: Starting real-time listener...");
        self.adapter.start_listener(&self.store, &entities).await?;

        Ok(())
    }
}
