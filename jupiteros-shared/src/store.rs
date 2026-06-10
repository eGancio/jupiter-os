// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use qdrant_client::qdrant::{
    point_id::PointIdOptions,
    points_selector::PointsSelectorOneOf,
    vector::Vector as VectorEnum,
    vectors::VectorsOptions,
    vectors_config::Config as VectorsConfigEnum,
    Condition, CreateCollectionBuilder, CreateFieldIndexCollectionBuilder, DeletePointsBuilder,
    DenseVector, Distance, FieldType, Filter, GetPointsBuilder, NamedVectors, OrderBy, PointId,
    PointStruct, PointsIdsList, ScrollPointsBuilder, SearchPointsBuilder,
    SetPayloadPointsBuilder, SparseIndices, SparseVector, SparseVectorConfig, SparseVectorParams,
    UpsertPointsBuilder, Value as QdrantValue, Vector, VectorParams, VectorParamsMap, Vectors,
    VectorsConfig,
};
use qdrant_client::Qdrant;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::embeddings::{HybridEmbedding, OnnxEmbedding};
use crate::error::{Result, SharedError};

/// Named vector keys used in the Qdrant collection schema.
const DENSE_VEC_NAME: &str = "dense";
const SPARSE_VEC_NAME: &str = "sparse";

/// Constant `k` for Reciprocal Rank Fusion. 60 is the value used in the
/// original RRF paper (Cormack et al. 2009) and is widely adopted.
const RRF_K: f32 = 60.0;

const MAX_TEXT_LEN: usize = 30_000;

/// Build a Qdrant `Condition` that filters on the `account` payload field.
/// Returns `None` if the input is empty/None — caller should treat that as
/// "no account filter". Lowercases the value because account names are stored
/// lowercase by the indexer and `Condition::matches` is case-sensitive.
fn account_condition(account: Option<&str>) -> Option<Condition> {
    account
        .filter(|s| !s.is_empty())
        .map(|s| Condition::matches("account", s.to_lowercase()))
}

/// A message to be indexed in the vector store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageData {
    pub id: String,
    pub text: String,
    pub sender: String,
    pub recipient: String,
    pub subject: String,
    pub date: String,
    pub channel: String,
    /// IMAP folder (e.g. "INBOX", "INBOX.Sent"). Empty for non-email messages.
    #[serde(default)]
    pub folder: String,
    /// IMAP UID — only set for emails, needed for attachments/reply.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imap_uid: Option<u32>,
    /// Account name (e.g. "primary", "gmail"). Empty for non-email channels.
    #[serde(default)]
    pub account: String,
}

/// A header-only view of a message (for list operations).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageHeader {
    pub id: String,
    pub sender: String,
    pub recipient: String,
    pub subject: String,
    pub date: String,
    pub channel: String,
    pub preview: String,
    /// IMAP folder (e.g. "INBOX", "INBOX.Sent").
    pub folder: String,
    /// IMAP UID — only set for emails, needed for attachments/reply.
    pub imap_uid: Option<u32>,
    /// Account name (e.g. "primary", "gmail").
    #[serde(default)]
    pub account: String,
}

/// A search result with similarity score.
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub id: String,
    pub text: String,
    pub metadata: HashMap<String, String>,
    pub score: f32,
    /// IMAP folder (e.g. "INBOX", "INBOX.Sent").
    pub folder: String,
    /// IMAP UID — only set for emails.
    pub imap_uid: Option<u32>,
    /// Account name (e.g. "primary", "gmail").
    pub account: String,
}

/// Index operation result.
#[derive(Debug, Default)]
pub struct IndexResult {
    pub indexed: usize,
    pub skipped: usize,
    pub errors: usize,
}

/// Store statistics.
#[derive(Debug)]
pub struct StoreStats {
    pub total: usize,
    pub by_channel: HashMap<String, usize>,
}

/// A point in Qdrant whose `date` payload is empty (broken/missing).
/// Returned by `MessageStore::scroll_empty_dates` so callers can repair the
/// payload in-place using the IMAP `INTERNALDATE` / Gmail `internalDate`.
#[derive(Debug, Clone)]
pub struct EmptyDatePoint {
    /// Qdrant point ID (UUID v5).
    pub point_id: String,
    /// `account` payload field (e.g. "aruba", "gmail"). May be empty for
    /// legacy pre-multi-account points.
    pub account: String,
    /// `folder` payload field (e.g. "INBOX", "INBOX.Sent").
    pub folder: String,
    /// IMAP UID — required to fetch INTERNALDATE from the server.
    pub imap_uid: Option<u32>,
}

/// Qdrant-backed vector store for messages/emails.
///
/// Replaces ChromaDB from the Python implementation.
/// Uses Qdrant with builder-pattern API (qdrant-client v1.17).
pub struct MessageStore {
    client: Qdrant,
    embedding: Arc<OnnxEmbedding>,
    collection_name: String,
    namespace: Option<String>, // For multi-tenant isolation
}

impl MessageStore {
    /// Create a new store backed by Qdrant.
    ///
    /// `collection_name` is typically "emails" or "messages".
    /// `namespace` isolates data per user in multi-tenant mode.
    pub async fn new(
        qdrant_url: &str,
        embedding: Arc<OnnxEmbedding>,
        collection_name: &str,
        namespace: Option<String>,
    ) -> Result<Self> {
        let client = Qdrant::from_url(qdrant_url)
            .build()
            .map_err(|e| SharedError::Qdrant(format!("Connect: {e}")))?;

        let store = Self {
            client,
            embedding,
            collection_name: collection_name.to_string(),
            namespace,
        };

        store.ensure_collection().await?;
        Ok(store)
    }

    /// Ensure the collection exists with the hybrid schema (named vectors
    /// `dense` + `sparse`). If a collection with an incompatible legacy schema
    /// exists (e.g. single un-named vector from the MiniLM era), it is dropped
    /// and recreated — the daemon will re-index from scratch.
    async fn ensure_collection(&self) -> Result<()> {
        let exists = self
            .client
            .collection_exists(&self.collection_name)
            .await
            .map_err(|e| SharedError::Qdrant(format!("Check collection: {e}")))?;

        if exists && !self.has_hybrid_schema().await? {
            warn!(
                "Collection '{}' uses a legacy schema — dropping and recreating for hybrid (dense+sparse) search",
                self.collection_name
            );
            let _ = self
                .client
                .delete_collection(&self.collection_name)
                .await;
        }

        let exists = self
            .client
            .collection_exists(&self.collection_name)
            .await
            .map_err(|e| SharedError::Qdrant(format!("Check collection after migration: {e}")))?;

        if !exists {
            info!(
                "Creating collection '{}' with hybrid schema (dense {} dim + sparse)",
                self.collection_name,
                self.embedding.vector_size()
            );

            let mut dense_map = std::collections::HashMap::new();
            dense_map.insert(
                DENSE_VEC_NAME.to_string(),
                VectorParams {
                    size: self.embedding.vector_size() as u64,
                    distance: Distance::Cosine as i32,
                    ..Default::default()
                },
            );

            let mut sparse_map = std::collections::HashMap::new();
            sparse_map.insert(SPARSE_VEC_NAME.to_string(), SparseVectorParams::default());

            self.client
                .create_collection(
                    CreateCollectionBuilder::new(&self.collection_name)
                        .vectors_config(VectorsConfig {
                            config: Some(VectorsConfigEnum::ParamsMap(VectorParamsMap {
                                map: dense_map,
                            })),
                        })
                        .sparse_vectors_config(SparseVectorConfig { map: sparse_map }),
                )
                .await
                .map_err(|e| SharedError::Qdrant(format!("Create collection: {e}")))?;
        }

        // Ensure payload index on "date" field (Datetime) for efficient order_by.
        // Idempotent: Qdrant ignores if the index already exists.
        if let Err(e) = self
            .client
            .create_field_index(
                CreateFieldIndexCollectionBuilder::new(
                    &self.collection_name,
                    "date",
                    FieldType::Datetime,
                ),
            )
            .await
        {
            debug!("Date index creation (may already exist): {e}");
        }

        // Index "account" (Keyword) for fast filtering in multi-account setups.
        if let Err(e) = self
            .client
            .create_field_index(
                CreateFieldIndexCollectionBuilder::new(
                    &self.collection_name,
                    "account",
                    FieldType::Keyword,
                ),
            )
            .await
        {
            debug!("Account index creation (may already exist): {e}");
        }

        // Index "imap_uid" (Integer) for fast UID lookups (delete/reply paths).
        if let Err(e) = self
            .client
            .create_field_index(
                CreateFieldIndexCollectionBuilder::new(
                    &self.collection_name,
                    "imap_uid",
                    FieldType::Integer,
                ),
            )
            .await
        {
            debug!("imap_uid index creation (may already exist): {e}");
        }

        Ok(())
    }

    /// Check whether the existing collection already has the hybrid schema
    /// (named vector `dense` + sparse vector `sparse`). Returns true if so,
    /// false if it's a legacy single-vector collection that must be dropped.
    async fn has_hybrid_schema(&self) -> Result<bool> {
        let info = self
            .client
            .collection_info(&self.collection_name)
            .await
            .map_err(|e| SharedError::Qdrant(format!("Collection info: {e}")))?;

        let Some(result) = info.result else { return Ok(false) };
        let Some(config) = result.config else { return Ok(false) };
        let Some(params) = config.params else { return Ok(false) };

        let has_named_dense = params
            .vectors_config
            .as_ref()
            .and_then(|vc| vc.config.as_ref())
            .map(|cfg| match cfg {
                VectorsConfigEnum::ParamsMap(m) => m.map.contains_key(DENSE_VEC_NAME),
                _ => false,
            })
            .unwrap_or(false);

        let has_sparse = params
            .sparse_vectors_config
            .as_ref()
            .map(|sc| sc.map.contains_key(SPARSE_VEC_NAME))
            .unwrap_or(false);

        Ok(has_named_dense && has_sparse)
    }

    /// Build a Qdrant `Vectors` payload combining a dense vector and a sparse
    /// vector under the named-vectors schema (qdrant-client v1.17 API).
    fn build_hybrid_vectors(hybrid: &HybridEmbedding) -> Vectors {
        let mut map = std::collections::HashMap::new();

        let dense_vec = Vector {
            vector: Some(VectorEnum::Dense(DenseVector {
                data: hybrid.dense.clone(),
            })),
            ..Default::default()
        };
        map.insert(DENSE_VEC_NAME.to_string(), dense_vec);

        let sparse_vec = Vector {
            vector: Some(VectorEnum::Sparse(SparseVector {
                values: hybrid.sparse.values.clone(),
                indices: hybrid.sparse.indices.clone(),
            })),
            ..Default::default()
        };
        map.insert(SPARSE_VEC_NAME.to_string(), sparse_vec);

        Vectors {
            vectors_options: Some(VectorsOptions::Vectors(NamedVectors { vectors: map })),
        }
    }

    /// Delete and recreate the collection (full reset).
    pub async fn reset_collection(&self) -> Result<()> {
        info!("Deleting collection '{}'", self.collection_name);
        let _ = self
            .client
            .delete_collection(&self.collection_name)
            .await;
        self.ensure_collection().await?;
        info!("Collection '{}' recreated", self.collection_name);
        Ok(())
    }

    /// Scroll all points whose `date` payload is the empty string.
    ///
    /// Server-side filter only narrows to namespace + account: the `date`
    /// field is indexed as `Datetime`, so `Condition::matches("date", "")`
    /// never returns hits (an empty string is not a valid datetime). We scan
    /// the account-scoped slice and inspect the raw payload string here.
    pub async fn scroll_empty_dates(&self, account: Option<&str>) -> Result<Vec<EmptyDatePoint>> {
        let mut conditions: Vec<Condition> = Vec::new();
        if let Some(ns) = &self.namespace {
            conditions.push(Condition::matches("_namespace", ns.clone()));
        }
        if let Some(c) = account_condition(account) {
            conditions.push(c);
        }

        let mut out = Vec::new();
        let mut next_offset: Option<PointId> = None;
        loop {
            let mut scroll = ScrollPointsBuilder::new(&self.collection_name)
                .limit(500u32)
                .with_payload(true)
                .with_vectors(false);
            if !conditions.is_empty() {
                scroll = scroll.filter(Filter::must(conditions.clone()));
            }
            if let Some(off) = next_offset {
                scroll = scroll.offset(off);
            }

            let response = self
                .client
                .scroll(scroll)
                .await
                .map_err(|e| SharedError::Qdrant(format!("Scroll empty dates: {e}")))?;

            for pt in &response.result {
                let date = Self::payload_str(&pt.payload, "date");
                if !date.is_empty() {
                    continue;
                }
                let Some(id_options) = pt.id.as_ref().and_then(|i| i.point_id_options.as_ref())
                else {
                    continue;
                };
                let point_id = match id_options {
                    PointIdOptions::Uuid(s) => s.clone(),
                    PointIdOptions::Num(n) => n.to_string(),
                };
                out.push(EmptyDatePoint {
                    point_id,
                    account: Self::payload_str(&pt.payload, "account"),
                    folder: Self::payload_str(&pt.payload, "folder"),
                    imap_uid: Self::payload_imap_uid(&pt.payload),
                });
            }

            next_offset = response.next_page_offset;
            if next_offset.is_none() {
                break;
            }
        }

        Ok(out)
    }

    /// Set the `date` payload field on a batch of points without re-embedding.
    ///
    /// Each item is `(point_id, date_rfc3339)`. One Qdrant RPC per point —
    /// caller is expected to cap concurrency upstream. Returns the number of
    /// successful updates (errors are logged and skipped).
    pub async fn set_date_payload_batch(
        &self,
        items: &[(String, String)],
    ) -> Result<usize> {
        let mut ok = 0usize;
        for (point_id, date) in items {
            let mut payload: HashMap<String, QdrantValue> = HashMap::new();
            payload.insert("date".into(), date.clone().into());

            let selector = PointsSelectorOneOf::Points(PointsIdsList {
                ids: vec![PointId::from(point_id.as_str())],
            });

            let req = SetPayloadPointsBuilder::new(&self.collection_name, payload)
                .points_selector(selector)
                .wait(false);

            match self.client.set_payload(req).await {
                Ok(_) => ok += 1,
                Err(e) => warn!("set_payload {point_id}: {e}"),
            }
        }
        Ok(ok)
    }

    /// Build a point ID from a message ID string (deterministic UUID v5).
    fn point_id(message_id: &str) -> String {
        let uuid = Uuid::new_v5(&Uuid::NAMESPACE_OID, message_id.as_bytes());
        uuid.to_string()
    }

    /// Build namespace filter for multi-tenant.
    fn namespace_filter(&self) -> Option<Filter> {
        self.namespace.as_ref().map(|ns| {
            Filter::must([Condition::matches("_namespace", ns.clone())])
        })
    }

    /// Build payload from message data.
    fn build_payload(msg: &MessageData, namespace: &Option<String>) -> HashMap<String, QdrantValue> {
        let mut payload: HashMap<String, QdrantValue> = HashMap::new();
        payload.insert("message_id".into(), msg.id.clone().into());
        payload.insert("text".into(), msg.text.chars().take(MAX_TEXT_LEN).collect::<String>().into());
        payload.insert("sender".into(), msg.sender.clone().into());
        payload.insert("recipient".into(), msg.recipient.clone().into());
        payload.insert("subject".into(), msg.subject.clone().into());
        payload.insert("date".into(), msg.date.clone().into());
        payload.insert("channel".into(), msg.channel.clone().into());
        payload.insert("indexed_at".into(), Utc::now().to_rfc3339().into());
        if !msg.folder.is_empty() {
            payload.insert("folder".into(), msg.folder.clone().into());
        }
        if !msg.account.is_empty() {
            payload.insert("account".into(), msg.account.clone().into());
        }
        if let Some(uid) = msg.imap_uid {
            payload.insert("imap_uid".into(), QdrantValue::from(uid as i64));
        }
        if let Some(ns) = namespace {
            payload.insert("_namespace".into(), ns.clone().into());
        }
        payload
    }

    /// Extract string from Qdrant payload value.
    fn payload_str(payload: &HashMap<String, QdrantValue>, key: &str) -> String {
        payload
            .get(key)
            .and_then(|v| v.as_str())
            .cloned()
            .unwrap_or_default()
    }

    /// Extract IMAP UID from payload (stored as integer).
    fn payload_imap_uid(payload: &HashMap<String, QdrantValue>) -> Option<u32> {
        payload
            .get("imap_uid")
            .and_then(|v| v.as_integer())
            .map(|i| i as u32)
    }

    // -----------------------------------------------------------------------
    // Indexing
    // -----------------------------------------------------------------------

    /// Build a hybrid PointStruct (dense + sparse named vectors) from a message
    /// and its hybrid embedding.
    fn build_hybrid_point(
        msg: &MessageData,
        hybrid: &HybridEmbedding,
        namespace: &Option<String>,
    ) -> PointStruct {
        let point_id = Self::point_id(&msg.id);
        let payload = Self::build_payload(msg, namespace);
        PointStruct {
            id: Some(point_id.as_str().into()),
            vectors: Some(Self::build_hybrid_vectors(hybrid)),
            payload,
        }
    }

    /// Index a single message. Returns true if newly indexed, false if duplicate.
    pub async fn index_message(&self, msg: MessageData) -> Result<bool> {
        let point_id = Self::point_id(&msg.id);

        // Check if already exists
        let pid: PointId = point_id.as_str().into();
        let existing = self
            .client
            .get_points(
                GetPointsBuilder::new(&self.collection_name, vec![pid])
                    .with_payload(false)
                    .with_vectors(false),
            )
            .await
            .map_err(|e| SharedError::Qdrant(format!("Get point: {e}")))?;

        if !existing.result.is_empty() {
            return Ok(false);
        }

        let truncated_text: String = msg.text.chars().take(MAX_TEXT_LEN).collect();
        let hybrid = self
            .embedding
            .embed_hybrid(&truncated_text)
            .map_err(|e| SharedError::Embedding(format!("Embed: {e}")))?;

        let point = Self::build_hybrid_point(&msg, &hybrid, &self.namespace);

        self.client
            .upsert_points(UpsertPointsBuilder::new(&self.collection_name, vec![point]))
            .await
            .map_err(|e| SharedError::Qdrant(format!("Upsert: {e}")))?;

        Ok(true)
    }

    /// Upsert a message (update if exists, insert if not).
    pub async fn upsert_message(&self, msg: MessageData) -> Result<()> {
        let truncated_text: String = msg.text.chars().take(MAX_TEXT_LEN).collect();
        let hybrid = self
            .embedding
            .embed_hybrid(&truncated_text)
            .map_err(|e| SharedError::Embedding(format!("Embed: {e}")))?;

        let point = Self::build_hybrid_point(&msg, &hybrid, &self.namespace);

        self.client
            .upsert_points(UpsertPointsBuilder::new(&self.collection_name, vec![point]))
            .await
            .map_err(|e| SharedError::Qdrant(format!("Upsert: {e}")))?;

        Ok(())
    }

    /// Batch index messages. Returns counts of indexed/skipped.
    pub async fn index_batch(&self, messages: Vec<MessageData>, upsert: bool) -> Result<IndexResult> {
        let mut result = IndexResult::default();
        if messages.is_empty() {
            return Ok(result);
        }

        // Deduplicate within batch
        let mut seen = std::collections::HashSet::new();
        let mut unique_msgs = Vec::new();
        for msg in messages {
            if seen.insert(msg.id.clone()) {
                unique_msgs.push(msg);
            } else {
                result.skipped += 1;
            }
        }

        if !upsert {
            // Filter out already-indexed IDs by checking in chunks of 100
            let mut existing_ids = std::collections::HashSet::new();
            for chunk in unique_msgs.chunks(100) {
                let pids: Vec<PointId> = chunk
                    .iter()
                    .map(|m| PointId::from(Self::point_id(&m.id)))
                    .collect();

                let existing = self
                    .client
                    .get_points(
                        GetPointsBuilder::new(&self.collection_name, pids)
                            .with_payload(false)
                            .with_vectors(false),
                    )
                    .await
                    .map_err(|e| SharedError::Qdrant(format!("Get points: {e}")))?;

                for pt in existing.result {
                    if let Some(id) = pt.id {
                        if let Some(qdrant_client::qdrant::point_id::PointIdOptions::Uuid(uuid_str)) = id.point_id_options {
                            existing_ids.insert(uuid_str);
                        }
                    }
                }
            }

            let before = unique_msgs.len();
            unique_msgs.retain(|m| !existing_ids.contains(&Self::point_id(&m.id)));
            result.skipped += before - unique_msgs.len();
        }

        if unique_msgs.is_empty() {
            return Ok(result);
        }

        // Generate hybrid embeddings (dense + sparse) in batch on a blocking
        // thread so the tokio runtime stays responsive.
        let texts_owned: Vec<String> = unique_msgs.iter().map(|m| m.text.clone()).collect();
        let embedding = self.embedding.clone();
        let hybrids = tokio::task::spawn_blocking(move || {
            let refs: Vec<&str> = texts_owned.iter().map(|s| s.as_str()).collect();
            embedding.embed_batch_hybrid(&refs)
        })
        .await
        .map_err(|e| SharedError::Embedding(format!("Spawn blocking: {e}")))?
        .map_err(|e| SharedError::Embedding(format!("Batch embed: {e}")))?;

        // Build points with both dense + sparse named vectors.
        let points: Vec<PointStruct> = unique_msgs
            .iter()
            .zip(hybrids.iter())
            .map(|(msg, hybrid)| Self::build_hybrid_point(msg, hybrid, &self.namespace))
            .collect();

        // Upsert in chunks of 200
        for chunk in points.chunks(200) {
            self.client
                .upsert_points(
                    UpsertPointsBuilder::new(&self.collection_name, chunk.to_vec()),
                )
                .await
                .map_err(|e| SharedError::Qdrant(format!("Batch upsert: {e}")))?;
            result.indexed += chunk.len();
        }

        debug!(
            "Batch indexed: {} new, {} skipped",
            result.indexed, result.skipped
        );
        Ok(result)
    }

    // -----------------------------------------------------------------------
    // Search
    // -----------------------------------------------------------------------

    /// Hybrid search: dense semantic + sparse lexical, fused with Reciprocal
    /// Rank Fusion. Falls back gracefully to dense-only if the sparse vector
    /// is empty (e.g. very short queries with only special tokens).
    pub async fn search(
        &self,
        query: &str,
        contact: Option<&str>,
        limit: usize,
        account: Option<&str>,
    ) -> Result<Vec<SearchResult>> {
        // Generate both dense and sparse query embeddings in a single forward pass.
        let query_hybrid = self
            .embedding
            .embed_hybrid(query)
            .map_err(|e| SharedError::Embedding(format!("Embed query: {e}")))?;

        let mut filter_conditions: Vec<Condition> = Vec::new();

        if let Some(ns) = &self.namespace {
            filter_conditions.push(Condition::matches("_namespace", ns.clone()));
        }
        if let Some(c) = account_condition(account) {
            filter_conditions.push(c);
        }
        if let Some(contact) = contact {
            let contact_lower = contact.to_lowercase();
            let contact_filter = Filter::should([
                Condition::matches("sender", contact_lower.clone()),
                Condition::matches("recipient", contact_lower),
            ]);
            filter_conditions.push(contact_filter.into());
        }

        let filter = if filter_conditions.is_empty() {
            None
        } else {
            Some(Filter::must(filter_conditions))
        };

        // Pull a larger candidate pool from each retriever so RRF has room to
        // shuffle the results meaningfully.
        let candidate_limit = ((limit * 5).max(30).min(200)) as u64;

        // ----- Dense retrieval (semantic similarity) -----
        let dense_results = {
            let mut builder = SearchPointsBuilder::new(
                &self.collection_name,
                query_hybrid.dense.clone(),
                candidate_limit,
            )
            .with_payload(true)
            .vector_name(DENSE_VEC_NAME);
            if let Some(f) = filter.clone() {
                builder = builder.filter(f);
            }
            self.client
                .search_points(builder)
                .await
                .map_err(|e| SharedError::Qdrant(format!("Dense search: {e}")))?
                .result
        };

        // ----- Sparse retrieval (lexical / keyword) -----
        // Skip if the sparse vector is empty (rare but possible for very short
        // queries made of only stopwords/special tokens).
        let sparse_results = if query_hybrid.sparse.indices.is_empty() {
            vec![]
        } else {
            let mut builder = SearchPointsBuilder::new(
                &self.collection_name,
                query_hybrid.sparse.values.clone(),
                candidate_limit,
            )
            .with_payload(true)
            .vector_name(SPARSE_VEC_NAME)
            .sparse_indices(SparseIndices {
                data: query_hybrid.sparse.indices.clone(),
            });
            if let Some(f) = filter {
                builder = builder.filter(f);
            }
            match self.client.search_points(builder).await {
                Ok(r) => r.result,
                Err(e) => {
                    // Don't fail the whole search if the sparse vector is
                    // missing or the index has not yet been populated.
                    warn!("Sparse search failed (falling back to dense-only): {e}");
                    vec![]
                }
            }
        };

        // ----- Reciprocal Rank Fusion -----
        // score_rrf(d) = sum_over_rankings(1 / (k + rank(d)))
        // Lower rank index = better. RRF_K = 60.
        let mut rrf: HashMap<String, (f32, qdrant_client::qdrant::ScoredPoint)> = HashMap::new();

        let merge = |pool: Vec<qdrant_client::qdrant::ScoredPoint>,
                     rrf: &mut HashMap<String, (f32, qdrant_client::qdrant::ScoredPoint)>| {
            for (rank, pt) in pool.into_iter().enumerate() {
                let Some(id) = pt.id.as_ref().and_then(|p| match &p.point_id_options {
                    Some(PointIdOptions::Uuid(s)) => Some(s.clone()),
                    Some(PointIdOptions::Num(n)) => Some(n.to_string()),
                    None => None,
                }) else {
                    continue;
                };
                let contribution = 1.0 / (RRF_K + (rank as f32) + 1.0);
                rrf.entry(id)
                    .and_modify(|(score, _)| *score += contribution)
                    .or_insert((contribution, pt));
            }
        };

        merge(dense_results, &mut rrf);
        merge(sparse_results, &mut rrf);

        let mut fused: Vec<(f32, qdrant_client::qdrant::ScoredPoint)> =
            rrf.into_values().collect();
        fused.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        let mut search_results: Vec<SearchResult> = fused
            .into_iter()
            .take(limit)
            .map(|(score, pt)| {
                let payload = pt.payload;
                let imap_uid = Self::payload_imap_uid(&payload);
                let folder = Self::payload_str(&payload, "folder");
                let account = Self::payload_str(&payload, "account");
                SearchResult {
                    id: Self::payload_str(&payload, "message_id"),
                    text: Self::payload_str(&payload, "text"),
                    metadata: payload
                        .iter()
                        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.clone())))
                        .collect(),
                    score,
                    folder,
                    imap_uid,
                    account,
                }
            })
            .collect();

        // Belt-and-braces post-filter by contact (Qdrant filter is approximate
        // when the field stores full names with diacritics or punctuation).
        if let Some(contact) = contact {
            let contact_lower = contact.to_lowercase();
            search_results.retain(|r| {
                let sender = r.metadata.get("sender").map(|s| s.to_lowercase()).unwrap_or_default();
                let recipient = r.metadata.get("recipient").map(|s| s.to_lowercase()).unwrap_or_default();
                sender.contains(&contact_lower) || recipient.contains(&contact_lower)
            });
        }

        Ok(search_results)
    }

    /// Find all messages with a specific contact.
    ///
    /// Scans ALL points in the collection (client-side substring match on
    /// sender/recipient), then sorts by date descending and truncates.
    /// This ensures no emails are missed regardless of Qdrant internal ordering.
    pub async fn search_by_contact(
        &self,
        contact: &str,
        limit: usize,
        account: Option<&str>,
    ) -> Result<Vec<SearchResult>> {
        let mut all_results = Vec::new();
        let mut offset = None;
        let contact_lower = contact.to_lowercase();

        // Build the scroll filter once (same for every page).
        let mut scroll_conditions: Vec<Condition> = Vec::new();
        if let Some(ns) = &self.namespace {
            scroll_conditions.push(Condition::matches("_namespace", ns.clone()));
        }
        if let Some(c) = account_condition(account) {
            scroll_conditions.push(c);
        }

        loop {
            let mut scroll = ScrollPointsBuilder::new(&self.collection_name)
                .limit(500u32)
                .with_payload(true);

            if !scroll_conditions.is_empty() {
                scroll = scroll.filter(Filter::must(scroll_conditions.clone()));
            }

            if let Some(off) = offset {
                scroll = scroll.offset(off);
            }

            let response = self
                .client
                .scroll(scroll)
                .await
                .map_err(|e| SharedError::Qdrant(format!("Scroll: {e}")))?;

            let page_empty = response.result.is_empty();

            for pt in &response.result {
                let payload = &pt.payload;
                let sender = Self::payload_str(payload, "sender").to_lowercase();
                let recipient = Self::payload_str(payload, "recipient").to_lowercase();
                let subject = Self::payload_str(payload, "subject").to_lowercase();

                if sender.contains(&contact_lower)
                    || recipient.contains(&contact_lower)
                    || subject.contains(&contact_lower)
                {
                    all_results.push(SearchResult {
                        id: Self::payload_str(payload, "message_id"),
                        text: Self::payload_str(payload, "text"),
                        metadata: payload
                            .iter()
                            .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.clone())))
                            .collect(),
                        score: 0.0,
                        folder: Self::payload_str(payload, "folder"),
                        imap_uid: Self::payload_imap_uid(payload),
                        account: Self::payload_str(payload, "account"),
                    });
                }
            }

            offset = response.next_page_offset;
            // Always scan ALL points — do NOT break early on result count.
            // With 18K+ emails in batches of 500, this is ~37 round-trips
            // but ensures no emails are missed.
            if offset.is_none() || page_empty {
                break;
            }
        }

        // Sort by date descending
        all_results.sort_by(|a, b| {
            let date_a = a.metadata.get("date").cloned().unwrap_or_default();
            let date_b = b.metadata.get("date").cloned().unwrap_or_default();
            date_b.cmp(&date_a)
        });

        all_results.truncate(limit);
        Ok(all_results)
    }

    // -----------------------------------------------------------------------
    // List
    // -----------------------------------------------------------------------

    /// List message headers (metadata only, no embeddings).
    ///
    /// Without contact filter: uses Qdrant `order_by` on the `date` payload
    /// index (Datetime) for efficient sorted retrieval.
    ///
    /// With contact filter: scrolls ALL points (client-side substring match
    /// on sender, recipient, and subject), sorts by date descending, then
    /// applies offset + limit. This is slower but ensures no matches are missed.
    pub async fn list_messages(
        &self,
        limit: usize,
        contact: Option<&str>,
        offset: usize,
        account: Option<&str>,
    ) -> Result<Vec<MessageHeader>> {
        let contact_lower = contact.map(|c| c.to_lowercase());
        let need_total = offset + limit;

        if contact_lower.is_some() {
            // --- Contact filter path: scan ALL, filter client-side, sort manually ---
            return self.list_messages_by_contact(
                contact_lower.as_deref().unwrap(),
                limit,
                offset,
                account,
            ).await;
        }

        // --- No contact filter: fast ordered fetch ---
        let order = OrderBy {
            key: "date".into(),
            direction: Some(1), // Direction::Desc
            start_from: None,
        };

        let mut scroll = ScrollPointsBuilder::new(&self.collection_name)
            .limit((need_total as u32).max(1))
            .with_payload(true)
            .order_by(order);

        let mut conditions: Vec<Condition> = Vec::new();
        if let Some(ns) = &self.namespace {
            conditions.push(Condition::matches("_namespace", ns.clone()));
        }
        if let Some(c) = account_condition(account) {
            conditions.push(c);
        }
        if !conditions.is_empty() {
            scroll = scroll.filter(Filter::must(conditions));
        }

        let response = self
            .client
            .scroll(scroll)
            .await
            .map_err(|e| SharedError::Qdrant(format!("Scroll: {e}")))?;

        let all_headers: Vec<MessageHeader> = response
            .result
            .iter()
            .map(|pt| {
                let payload = &pt.payload;
                let text = Self::payload_str(payload, "text");
                let preview: String = text.chars().take(200).collect();
                MessageHeader {
                    id: Self::payload_str(payload, "message_id"),
                    sender: Self::payload_str(payload, "sender"),
                    recipient: Self::payload_str(payload, "recipient"),
                    subject: Self::payload_str(payload, "subject"),
                    date: Self::payload_str(payload, "date"),
                    channel: Self::payload_str(payload, "channel"),
                    preview,
                    folder: Self::payload_str(payload, "folder"),
                    imap_uid: Self::payload_imap_uid(payload),
                    account: Self::payload_str(payload, "account"),
                }
            })
            .collect();

        let start = offset.min(all_headers.len());
        let end = (start + limit).min(all_headers.len());
        Ok(all_headers[start..end].to_vec())
    }

    /// List messages filtered by contact — scans ALL points with client-side
    /// substring matching on sender, recipient, and subject fields.
    async fn list_messages_by_contact(
        &self,
        contact: &str,
        limit: usize,
        offset: usize,
        account: Option<&str>,
    ) -> Result<Vec<MessageHeader>> {
        let contact_lower = contact.to_lowercase();
        let mut all_headers = Vec::new();
        let mut scroll_offset: Option<PointId> = None;

        // Build the scroll filter once (same for every page).
        let mut scroll_conditions: Vec<Condition> = Vec::new();
        if let Some(ns) = &self.namespace {
            scroll_conditions.push(Condition::matches("_namespace", ns.clone()));
        }
        if let Some(c) = account_condition(account) {
            scroll_conditions.push(c);
        }

        loop {
            let mut scroll = ScrollPointsBuilder::new(&self.collection_name)
                .limit(500u32)
                .with_payload(true);

            if !scroll_conditions.is_empty() {
                scroll = scroll.filter(Filter::must(scroll_conditions.clone()));
            }

            if let Some(off) = scroll_offset {
                scroll = scroll.offset(off);
            }

            let response = self
                .client
                .scroll(scroll)
                .await
                .map_err(|e| SharedError::Qdrant(format!("Scroll: {e}")))?;

            let page_empty = response.result.is_empty();

            for pt in &response.result {
                let payload = &pt.payload;
                let sender = Self::payload_str(payload, "sender");
                let recipient = Self::payload_str(payload, "recipient");
                let subject = Self::payload_str(payload, "subject");

                let s_lower = sender.to_lowercase();
                let r_lower = recipient.to_lowercase();
                let subj_lower = subject.to_lowercase();

                if !s_lower.contains(&contact_lower)
                    && !r_lower.contains(&contact_lower)
                    && !subj_lower.contains(&contact_lower)
                {
                    continue;
                }

                let text = Self::payload_str(payload, "text");
                let preview: String = text.chars().take(200).collect();

                all_headers.push(MessageHeader {
                    id: Self::payload_str(payload, "message_id"),
                    sender,
                    recipient,
                    subject,
                    date: Self::payload_str(payload, "date"),
                    channel: Self::payload_str(payload, "channel"),
                    preview,
                    folder: Self::payload_str(payload, "folder"),
                    imap_uid: Self::payload_imap_uid(payload),
                    account: Self::payload_str(payload, "account"),
                });
            }

            scroll_offset = response.next_page_offset;
            if scroll_offset.is_none() || page_empty {
                break;
            }
        }

        // Sort by date descending (RFC 3339 strings sort lexicographically)
        all_headers.sort_by(|a, b| b.date.cmp(&a.date));

        let start = offset.min(all_headers.len());
        let end = (start + limit).min(all_headers.len());
        Ok(all_headers[start..end].to_vec())
    }

    // -----------------------------------------------------------------------
    // Read full text
    // -----------------------------------------------------------------------

    /// Read messages with full body text (up to `max_chars` per message).
    ///
    /// Similar to `list_messages` but returns `MessageData` with the full
    /// text field instead of a truncated preview. Used by `read_recent_emails`.
    ///
    /// Without contact filter: uses Qdrant `order_by` on the `date` payload
    /// index (Datetime), like `list_messages` — the old single-page scroll
    /// returned points in ID order, so "recent" was the newest among an
    /// ARBITRARY sample (emails from years ago showed up as "le ultime").
    ///
    /// With contact filter: scrolls ALL points (client-side substring match),
    /// sorts by date descending, truncates.
    pub async fn read_messages_full(
        &self,
        limit: usize,
        contact: Option<&str>,
        max_chars: usize,
        account: Option<&str>,
    ) -> Result<Vec<MessageData>> {
        let contact_lower = contact.map(|c| c.to_lowercase());

        // Build the shared filter conditions (namespace + account) once.
        let mut base_conditions: Vec<Condition> = Vec::new();
        if let Some(ns) = &self.namespace {
            base_conditions.push(Condition::matches("_namespace", ns.clone()));
        }
        if let Some(c) = account_condition(account) {
            base_conditions.push(c);
        }

        if contact_lower.is_none() {
            // --- No contact filter: fast ordered fetch ---
            let order = OrderBy {
                key: "date".into(),
                direction: Some(1), // Direction::Desc
                start_from: None,
            };
            let mut scroll = ScrollPointsBuilder::new(&self.collection_name)
                .limit((limit as u32).max(1))
                .with_payload(true)
                .order_by(order);
            if !base_conditions.is_empty() {
                scroll = scroll.filter(Filter::must(base_conditions));
            }
            let response = self
                .client
                .scroll(scroll)
                .await
                .map_err(|e| SharedError::Qdrant(format!("Scroll: {e}")))?;
            return Ok(response
                .result
                .iter()
                .map(|pt| {
                    let mut m = Self::point_to_message(pt);
                    m.text = m.text.chars().take(max_chars).collect();
                    m
                })
                .collect());
        }

        // --- Contact filter path: scan ALL, filter client-side, sort manually ---
        let mut results: Vec<MessageData> = Vec::new();
        let mut scroll_offset: Option<PointId> = None;

        loop {
            let mut scroll = ScrollPointsBuilder::new(&self.collection_name)
                .limit(500)
                .with_payload(true);

            if !base_conditions.is_empty() {
                scroll = scroll.filter(Filter::must(base_conditions.clone()));
            }

            if let Some(off) = scroll_offset {
                scroll = scroll.offset(off);
            }

            let response = self
                .client
                .scroll(scroll)
                .await
                .map_err(|e| SharedError::Qdrant(format!("Scroll: {e}")))?;

            let page_empty = response.result.is_empty();

            for pt in &response.result {
                let payload = &pt.payload;
                let sender = Self::payload_str(payload, "sender");
                let recipient = Self::payload_str(payload, "recipient");

                // Contact filter (client-side substring match)
                if let Some(ref cl) = contact_lower {
                    let s_lower = sender.to_lowercase();
                    let r_lower = recipient.to_lowercase();
                    let subj_lower = Self::payload_str(payload, "subject").to_lowercase();
                    if !s_lower.contains(cl) && !r_lower.contains(cl) && !subj_lower.contains(cl) {
                        continue;
                    }
                }

                let full_text = Self::payload_str(payload, "text");
                let text: String = full_text.chars().take(max_chars).collect();

                results.push(MessageData {
                    id: Self::payload_str(payload, "message_id"),
                    text,
                    sender,
                    recipient,
                    subject: Self::payload_str(payload, "subject"),
                    date: Self::payload_str(payload, "date"),
                    channel: Self::payload_str(payload, "channel"),
                    folder: Self::payload_str(payload, "folder"),
                    imap_uid: Self::payload_imap_uid(payload),
                    account: Self::payload_str(payload, "account"),
                });
            }

            scroll_offset = response.next_page_offset;
            if scroll_offset.is_none() || page_empty {
                break;
            }
        }

        // Sort by date descending
        results.sort_by(|a, b| b.date.cmp(&a.date));
        results.truncate(limit);
        Ok(results)
    }

    /// Convert a Qdrant point to MessageData.
    fn point_to_message(pt: &qdrant_client::qdrant::RetrievedPoint) -> MessageData {
        let payload = &pt.payload;
        MessageData {
            id: Self::payload_str(payload, "message_id"),
            text: Self::payload_str(payload, "text"),
            sender: Self::payload_str(payload, "sender"),
            recipient: Self::payload_str(payload, "recipient"),
            subject: Self::payload_str(payload, "subject"),
            date: Self::payload_str(payload, "date"),
            channel: Self::payload_str(payload, "channel"),
            folder: Self::payload_str(payload, "folder"),
            imap_uid: Self::payload_imap_uid(payload),
            account: Self::payload_str(payload, "account"),
        }
    }

    /// Get a single email by its IMAP UID. Returns full message data.
    ///
    /// WARNING: in multi-account setups two different accounts can have the
    /// SAME UID for different emails (each IMAP server numbers from 1).
    /// This method returns a non-deterministic single match — prefer
    /// `get_by_imap_uid_in_account` or `find_all_by_imap_uid`.
    pub async fn get_by_imap_uid(&self, uid: u32) -> Result<Option<MessageData>> {
        let matches = self.find_all_by_imap_uid(uid).await?;
        Ok(matches.into_iter().next())
    }

    /// Get an email by IMAP UID restricted to a specific account.
    /// Correct disambiguation for multi-account stores.
    pub async fn get_by_imap_uid_in_account(
        &self,
        uid: u32,
        account: &str,
    ) -> Result<Option<MessageData>> {
        let mut conditions = vec![
            Condition::matches("imap_uid", uid as i64),
            Condition::matches("account", account.to_string()),
        ];
        if let Some(ns) = &self.namespace {
            conditions.push(Condition::matches("_namespace", ns.clone()));
        }

        let scroll = ScrollPointsBuilder::new(&self.collection_name)
            .limit(1u32)
            .with_payload(true)
            .filter(Filter::must(conditions));

        let response = self
            .client
            .scroll(scroll)
            .await
            .map_err(|e| SharedError::Qdrant(format!("Scroll: {e}")))?;

        Ok(response.result.first().map(Self::point_to_message))
    }

    /// Find ALL emails with a given IMAP UID across accounts.
    /// Used to disambiguate when the caller didn't specify an account.
    /// Returns up to 16 matches (more than the realistic number of accounts).
    pub async fn find_all_by_imap_uid(&self, uid: u32) -> Result<Vec<MessageData>> {
        let mut conditions = vec![Condition::matches("imap_uid", uid as i64)];
        if let Some(ns) = &self.namespace {
            conditions.push(Condition::matches("_namespace", ns.clone()));
        }

        let scroll = ScrollPointsBuilder::new(&self.collection_name)
            .limit(16u32)
            .with_payload(true)
            .filter(Filter::must(conditions));

        let response = self
            .client
            .scroll(scroll)
            .await
            .map_err(|e| SharedError::Qdrant(format!("Scroll: {e}")))?;

        Ok(response.result.iter().map(Self::point_to_message).collect())
    }

    // -----------------------------------------------------------------------
    // Delete
    // -----------------------------------------------------------------------

    /// Delete a message by IMAP UID. Returns true if a point was deleted.
    /// In multi-account stores, prefer `delete_by_imap_uid_in_account` to
    /// avoid deleting from the wrong account.
    pub async fn delete_by_imap_uid(&self, uid: u32) -> Result<bool> {
        let mut conditions = vec![Condition::matches("imap_uid", uid as i64)];
        if let Some(ns) = &self.namespace {
            conditions.push(Condition::matches("_namespace", ns.clone()));
        }
        self.delete_by_filter(Filter::must(conditions)).await
    }

    /// Delete a message by IMAP UID restricted to a specific account.
    /// Correct for multi-account stores.
    pub async fn delete_by_imap_uid_in_account(
        &self,
        uid: u32,
        account: &str,
    ) -> Result<bool> {
        let mut conditions = vec![
            Condition::matches("imap_uid", uid as i64),
            Condition::matches("account", account.to_string()),
        ];
        if let Some(ns) = &self.namespace {
            conditions.push(Condition::matches("_namespace", ns.clone()));
        }
        self.delete_by_filter(Filter::must(conditions)).await
    }

    async fn delete_by_filter(&self, filter: Filter) -> Result<bool> {
        let scroll = ScrollPointsBuilder::new(&self.collection_name)
            .limit(1u32)
            .with_payload(false)
            .filter(filter);

        let response = self
            .client
            .scroll(scroll)
            .await
            .map_err(|e| SharedError::Qdrant(format!("Scroll: {e}")))?;

        let pid = match response.result.first().and_then(|p| p.id.clone()) {
            Some(id) => id,
            None => return Ok(false),
        };

        self.client
            .delete_points(
                DeletePointsBuilder::new(&self.collection_name).points(vec![pid]),
            )
            .await
            .map_err(|e| SharedError::Qdrant(format!("Delete: {e}")))?;

        Ok(true)
    }

    // -----------------------------------------------------------------------
    // Stats
    // -----------------------------------------------------------------------

    /// Get collection statistics.
    pub async fn get_stats(&self) -> Result<StoreStats> {
        let info = self
            .client
            .collection_info(&self.collection_name)
            .await
            .map_err(|e| SharedError::Qdrant(format!("Collection info: {e}")))?;

        let total = info.result
            .map(|r| r.points_count.unwrap_or(0) as usize)
            .unwrap_or(0);

        Ok(StoreStats {
            total,
            by_channel: HashMap::from([("total".to_string(), total)]),
        })
    }
}
