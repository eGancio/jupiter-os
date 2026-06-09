// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

//! Document-oriented vector store for Moon Metis (the knowledge layer).
//!
//! Mirrors [`crate::store::MessageStore`] (same hybrid dense+sparse schema and
//! Reciprocal Rank Fusion search) but carries a **document-shaped, citation-rich
//! payload** instead of the email-shaped one: every chunk knows its source file,
//! page and structural section so answers can be **cited** (file / page / Art.).
//!
//! It deliberately reuses the same [`OnnxEmbedding`] and the same Qdrant instance
//! as the other Moons — only the collection name and payload differ.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use qdrant_client::qdrant::{
    point_id::PointIdOptions, vector::Vector as VectorEnum, vectors::VectorsOptions,
    vectors_config::Config as VectorsConfigEnum, Condition, CreateCollectionBuilder,
    CreateFieldIndexCollectionBuilder, DeletePointsBuilder, DenseVector, Distance, FieldType,
    Filter, GetPointsBuilder, NamedVectors, PointId, PointStruct, ScrollPointsBuilder,
    SearchPointsBuilder, SparseIndices, SparseVector, SparseVectorConfig, SparseVectorParams,
    UpsertPointsBuilder, Value as QdrantValue, Vector, VectorParams, VectorParamsMap, Vectors,
    VectorsConfig,
};
use qdrant_client::Qdrant;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::embeddings::{HybridEmbedding, OnnxEmbedding};
use crate::error::{Result, SharedError};
use crate::store::IndexResult;

const DENSE_VEC_NAME: &str = "dense";
const SPARSE_VEC_NAME: &str = "sparse";

/// Constant `k` for Reciprocal Rank Fusion (Cormack et al. 2009).
const RRF_K: f32 = 60.0;

/// Hard cap on the stored chunk text (chunks are already small, this is a
/// belt-and-braces guard against pathological inputs).
const MAX_TEXT_LEN: usize = 20_000;

/// One indexed chunk of a document, ready to embed + upsert.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocChunk {
    /// Stable per-document id (derived from the source path by the ingester).
    pub doc_id: String,
    /// Absolute or display path of the source file.
    pub source_path: String,
    /// Human-friendly document title (file name, or first heading).
    pub title: String,
    /// Document nature: "generico" | "norma" | "bando" | "preventivo" | ...
    pub nature: String,
    /// 1-based page number the chunk came from (0 if the format has no pages).
    pub page: u32,
    /// Structural path within the document, e.g. "Art. 12 c.3" or a heading.
    pub section_path: String,
    /// 0-based chunk index within the document (stable ordering).
    pub chunk_index: u32,
    /// The chunk text.
    pub text: String,
    /// Content hash of the WHOLE source file (for incremental skip).
    pub content_hash: String,
}

/// A search hit carrying everything needed to render a citation.
#[derive(Debug, Clone, Serialize)]
pub struct DocHit {
    pub chunk_id: String,
    pub doc_id: String,
    pub source_path: String,
    pub title: String,
    pub nature: String,
    pub page: u32,
    pub section_path: String,
    pub chunk_index: u32,
    pub text: String,
    pub score: f32,
}

/// Document-level summary (one row per ingested file).
#[derive(Debug, Clone, Serialize)]
pub struct DocSummary {
    pub doc_id: String,
    pub source_path: String,
    pub title: String,
    pub nature: String,
    pub chunks: usize,
    pub pages: u32,
    pub content_hash: String,
}

/// Qdrant-backed document store (knowledge base).
pub struct DocStore {
    client: Qdrant,
    embedding: Arc<OnnxEmbedding>,
    collection_name: String,
    namespace: Option<String>,
}

impl DocStore {
    /// Connect to Qdrant and ensure the collection exists with the hybrid schema.
    /// `collection_name` is typically "metis_docs".
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

    async fn ensure_collection(&self) -> Result<()> {
        let exists = self
            .client
            .collection_exists(&self.collection_name)
            .await
            .map_err(|e| SharedError::Qdrant(format!("Check collection: {e}")))?;

        if !exists {
            info!(
                "Creating collection '{}' with hybrid schema (dense {} dim + sparse)",
                self.collection_name,
                self.embedding.vector_size()
            );

            let mut dense_map = HashMap::new();
            dense_map.insert(
                DENSE_VEC_NAME.to_string(),
                VectorParams {
                    size: self.embedding.vector_size() as u64,
                    distance: Distance::Cosine as i32,
                    ..Default::default()
                },
            );

            let mut sparse_map = HashMap::new();
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

        // Keyword/integer payload indexes for fast filtering. Idempotent.
        for (field, ftype) in [
            ("nature", FieldType::Keyword),
            ("doc_id", FieldType::Keyword),
            ("source_path", FieldType::Keyword),
            ("page", FieldType::Integer),
        ] {
            if let Err(e) = self
                .client
                .create_field_index(CreateFieldIndexCollectionBuilder::new(
                    &self.collection_name,
                    field,
                    ftype,
                ))
                .await
            {
                debug!("Index '{field}' (may already exist): {e}");
            }
        }

        Ok(())
    }

    /// Deterministic point id for a chunk (UUID v5 of "doc_id:chunk_index").
    fn chunk_point_id(doc_id: &str, chunk_index: u32) -> String {
        let key = format!("{doc_id}:{chunk_index}");
        Uuid::new_v5(&Uuid::NAMESPACE_OID, key.as_bytes()).to_string()
    }

    fn build_hybrid_vectors(hybrid: &HybridEmbedding) -> Vectors {
        let mut map = HashMap::new();
        map.insert(
            DENSE_VEC_NAME.to_string(),
            Vector {
                vector: Some(VectorEnum::Dense(DenseVector {
                    data: hybrid.dense.clone(),
                })),
                ..Default::default()
            },
        );
        map.insert(
            SPARSE_VEC_NAME.to_string(),
            Vector {
                vector: Some(VectorEnum::Sparse(SparseVector {
                    values: hybrid.sparse.values.clone(),
                    indices: hybrid.sparse.indices.clone(),
                })),
                ..Default::default()
            },
        );
        Vectors {
            vectors_options: Some(VectorsOptions::Vectors(NamedVectors { vectors: map })),
        }
    }

    fn build_payload(chunk: &DocChunk, namespace: &Option<String>) -> HashMap<String, QdrantValue> {
        let mut payload: HashMap<String, QdrantValue> = HashMap::new();
        payload.insert("doc_id".into(), chunk.doc_id.clone().into());
        payload.insert("source_path".into(), chunk.source_path.clone().into());
        payload.insert("title".into(), chunk.title.clone().into());
        payload.insert("nature".into(), chunk.nature.clone().into());
        payload.insert("page".into(), QdrantValue::from(chunk.page as i64));
        payload.insert("section_path".into(), chunk.section_path.clone().into());
        payload.insert("chunk_index".into(), QdrantValue::from(chunk.chunk_index as i64));
        payload.insert(
            "text".into(),
            chunk.text.chars().take(MAX_TEXT_LEN).collect::<String>().into(),
        );
        payload.insert("content_hash".into(), chunk.content_hash.clone().into());
        payload.insert("indexed_at".into(), Utc::now().to_rfc3339().into());
        if let Some(ns) = namespace {
            payload.insert("_namespace".into(), ns.clone().into());
        }
        payload
    }

    fn payload_str(payload: &HashMap<String, QdrantValue>, key: &str) -> String {
        payload
            .get(key)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_default()
    }

    fn payload_u32(payload: &HashMap<String, QdrantValue>, key: &str) -> u32 {
        payload
            .get(key)
            .and_then(|v| v.as_integer())
            .map(|i| i as u32)
            .unwrap_or(0)
    }

    fn point_id_str(pt_id: &Option<PointId>) -> String {
        pt_id
            .as_ref()
            .and_then(|p| p.point_id_options.as_ref())
            .map(|o| match o {
                PointIdOptions::Uuid(s) => s.clone(),
                PointIdOptions::Num(n) => n.to_string(),
            })
            .unwrap_or_default()
    }

    fn namespace_conditions(&self) -> Vec<Condition> {
        let mut c = Vec::new();
        if let Some(ns) = &self.namespace {
            c.push(Condition::matches("_namespace", ns.clone()));
        }
        c
    }

    // -----------------------------------------------------------------------
    // Indexing
    // -----------------------------------------------------------------------

    /// Index a batch of chunks. With `upsert == true` existing points are
    /// overwritten; with `false` already-present chunk ids are skipped.
    pub async fn index_chunks(&self, chunks: Vec<DocChunk>, upsert: bool) -> Result<IndexResult> {
        let mut result = IndexResult::default();
        if chunks.is_empty() {
            return Ok(result);
        }

        // Embed all chunk texts in one batched, blocking pass.
        let texts_owned: Vec<String> = chunks.iter().map(|c| c.text.clone()).collect();
        let embedding = self.embedding.clone();
        let hybrids = tokio::task::spawn_blocking(move || {
            let refs: Vec<&str> = texts_owned.iter().map(|s| s.as_str()).collect();
            embedding.embed_batch_hybrid(&refs)
        })
        .await
        .map_err(|e| SharedError::Embedding(format!("Spawn blocking: {e}")))?
        .map_err(|e| SharedError::Embedding(format!("Batch embed: {e}")))?;

        let points: Vec<PointStruct> = chunks
            .iter()
            .zip(hybrids.iter())
            .map(|(chunk, hybrid)| {
                let pid = Self::chunk_point_id(&chunk.doc_id, chunk.chunk_index);
                PointStruct {
                    id: Some(pid.as_str().into()),
                    vectors: Some(Self::build_hybrid_vectors(hybrid)),
                    payload: Self::build_payload(chunk, &self.namespace),
                }
            })
            .collect();

        let _ = upsert; // Qdrant upsert is idempotent; flag kept for API parity.
        for batch in points.chunks(200) {
            self.client
                .upsert_points(UpsertPointsBuilder::new(&self.collection_name, batch.to_vec()))
                .await
                .map_err(|e| SharedError::Qdrant(format!("Batch upsert: {e}")))?;
            result.indexed += batch.len();
        }

        debug!("DocStore indexed {} chunks", result.indexed);
        Ok(result)
    }

    /// Delete all chunks belonging to a document (used before re-ingesting a
    /// changed file). Returns the number of points removed.
    pub async fn delete_document(&self, doc_id: &str) -> Result<usize> {
        let mut conditions = self.namespace_conditions();
        conditions.push(Condition::matches("doc_id", doc_id.to_string()));

        let mut ids: Vec<PointId> = Vec::new();
        let mut offset: Option<PointId> = None;
        loop {
            let mut scroll = ScrollPointsBuilder::new(&self.collection_name)
                .limit(500u32)
                .with_payload(false)
                .with_vectors(false)
                .filter(Filter::must(conditions.clone()));
            if let Some(off) = offset {
                scroll = scroll.offset(off);
            }
            let resp = self
                .client
                .scroll(scroll)
                .await
                .map_err(|e| SharedError::Qdrant(format!("Scroll for delete: {e}")))?;
            for pt in &resp.result {
                if let Some(id) = pt.id.clone() {
                    ids.push(id);
                }
            }
            offset = resp.next_page_offset;
            if offset.is_none() {
                break;
            }
        }

        if ids.is_empty() {
            return Ok(0);
        }
        let n = ids.len();
        for batch in ids.chunks(200) {
            self.client
                .delete_points(
                    DeletePointsBuilder::new(&self.collection_name).points(batch.to_vec()),
                )
                .await
                .map_err(|e| SharedError::Qdrant(format!("Delete: {e}")))?;
        }
        Ok(n)
    }

    // -----------------------------------------------------------------------
    // Search
    // -----------------------------------------------------------------------

    /// Hybrid (dense + sparse, RRF-fused) search. Optional exact filters on
    /// `nature` and `doc_id`. Returns citation-ready hits.
    pub async fn search(
        &self,
        query: &str,
        nature: Option<&str>,
        doc_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<DocHit>> {
        let query_hybrid = self
            .embedding
            .embed_hybrid(query)
            .map_err(|e| SharedError::Embedding(format!("Embed query: {e}")))?;

        let mut conditions = self.namespace_conditions();
        if let Some(n) = nature.filter(|s| !s.is_empty()) {
            conditions.push(Condition::matches("nature", n.to_string()));
        }
        if let Some(d) = doc_id.filter(|s| !s.is_empty()) {
            conditions.push(Condition::matches("doc_id", d.to_string()));
        }
        let filter = if conditions.is_empty() {
            None
        } else {
            Some(Filter::must(conditions))
        };

        let candidate_limit = ((limit * 5).max(30).min(200)) as u64;

        // Dense retrieval.
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

        // Sparse retrieval (skipped if the query has no sparse terms).
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
                    warn!("Sparse search failed (dense-only fallback): {e}");
                    vec![]
                }
            }
        };

        // Reciprocal Rank Fusion.
        let mut rrf: HashMap<String, (f32, qdrant_client::qdrant::ScoredPoint)> = HashMap::new();
        let merge = |pool: Vec<qdrant_client::qdrant::ScoredPoint>,
                     rrf: &mut HashMap<String, (f32, qdrant_client::qdrant::ScoredPoint)>| {
            for (rank, pt) in pool.into_iter().enumerate() {
                let id = Self::point_id_str(&pt.id);
                if id.is_empty() {
                    continue;
                }
                let contribution = 1.0 / (RRF_K + (rank as f32) + 1.0);
                rrf.entry(id)
                    .and_modify(|(s, _)| *s += contribution)
                    .or_insert((contribution, pt));
            }
        };
        merge(dense_results, &mut rrf);
        merge(sparse_results, &mut rrf);

        let mut fused: Vec<(f32, qdrant_client::qdrant::ScoredPoint)> = rrf.into_values().collect();
        fused.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        let hits = fused
            .into_iter()
            .take(limit)
            .map(|(score, pt)| {
                let chunk_id = Self::point_id_str(&pt.id);
                let p = pt.payload;
                DocHit {
                    chunk_id,
                    doc_id: Self::payload_str(&p, "doc_id"),
                    source_path: Self::payload_str(&p, "source_path"),
                    title: Self::payload_str(&p, "title"),
                    nature: Self::payload_str(&p, "nature"),
                    page: Self::payload_u32(&p, "page"),
                    section_path: Self::payload_str(&p, "section_path"),
                    chunk_index: Self::payload_u32(&p, "chunk_index"),
                    text: Self::payload_str(&p, "text"),
                    score,
                }
            })
            .collect();

        Ok(hits)
    }

    // -----------------------------------------------------------------------
    // Inventory
    // -----------------------------------------------------------------------

    /// Scroll every chunk (namespace-scoped) and apply `f` to each `(payload)`.
    async fn scroll_all<F: FnMut(&HashMap<String, QdrantValue>, &str)>(
        &self,
        extra: Vec<Condition>,
        mut f: F,
    ) -> Result<()> {
        let mut conditions = self.namespace_conditions();
        conditions.extend(extra);

        let mut offset: Option<PointId> = None;
        loop {
            let mut scroll = ScrollPointsBuilder::new(&self.collection_name)
                .limit(500u32)
                .with_payload(true)
                .with_vectors(false);
            if !conditions.is_empty() {
                scroll = scroll.filter(Filter::must(conditions.clone()));
            }
            if let Some(off) = offset {
                scroll = scroll.offset(off);
            }
            let resp = self
                .client
                .scroll(scroll)
                .await
                .map_err(|e| SharedError::Qdrant(format!("Scroll: {e}")))?;
            for pt in &resp.result {
                let id = Self::point_id_str(&pt.id);
                f(&pt.payload, &id);
            }
            offset = resp.next_page_offset;
            if offset.is_none() {
                break;
            }
        }
        Ok(())
    }

    /// One row per ingested document, sorted by title.
    pub async fn list_documents(&self) -> Result<Vec<DocSummary>> {
        let mut docs: HashMap<String, DocSummary> = HashMap::new();
        self.scroll_all(vec![], |p, _id| {
            let doc_id = Self::payload_str(p, "doc_id");
            if doc_id.is_empty() {
                return;
            }
            let page = Self::payload_u32(p, "page");
            let entry = docs.entry(doc_id.clone()).or_insert_with(|| DocSummary {
                doc_id: doc_id.clone(),
                source_path: Self::payload_str(p, "source_path"),
                title: Self::payload_str(p, "title"),
                nature: Self::payload_str(p, "nature"),
                chunks: 0,
                pages: 0,
                content_hash: Self::payload_str(p, "content_hash"),
            });
            entry.chunks += 1;
            entry.pages = entry.pages.max(page);
        })
        .await?;

        let mut out: Vec<DocSummary> = docs.into_values().collect();
        out.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
        Ok(out)
    }

    /// Map of doc_id -> content_hash, for incremental skip decisions.
    pub async fn document_hashes(&self) -> Result<HashMap<String, String>> {
        let mut map = HashMap::new();
        self.scroll_all(vec![], |p, _id| {
            let doc_id = Self::payload_str(p, "doc_id");
            if !doc_id.is_empty() {
                map.entry(doc_id).or_insert_with(|| Self::payload_str(p, "content_hash"));
            }
        })
        .await?;
        Ok(map)
    }

    /// All chunks of a document, ordered by chunk_index (for inspection).
    pub async fn doc_chunks(&self, doc_id: &str) -> Result<Vec<DocHit>> {
        let mut hits: Vec<DocHit> = Vec::new();
        self.scroll_all(
            vec![Condition::matches("doc_id", doc_id.to_string())],
            |p, id| {
                hits.push(DocHit {
                    chunk_id: id.to_string(),
                    doc_id: Self::payload_str(p, "doc_id"),
                    source_path: Self::payload_str(p, "source_path"),
                    title: Self::payload_str(p, "title"),
                    nature: Self::payload_str(p, "nature"),
                    page: Self::payload_u32(p, "page"),
                    section_path: Self::payload_str(p, "section_path"),
                    chunk_index: Self::payload_u32(p, "chunk_index"),
                    text: Self::payload_str(p, "text"),
                    score: 0.0,
                });
            },
        )
        .await?;
        hits.sort_by_key(|h| h.chunk_index);
        Ok(hits)
    }

    /// Fetch a single chunk by its point id.
    pub async fn get_chunk(&self, chunk_id: &str) -> Result<Option<DocHit>> {
        let pid: PointId = chunk_id.into();
        let resp = self
            .client
            .get_points(
                GetPointsBuilder::new(&self.collection_name, vec![pid])
                    .with_payload(true)
                    .with_vectors(false),
            )
            .await
            .map_err(|e| SharedError::Qdrant(format!("Get chunk: {e}")))?;

        Ok(resp.result.into_iter().next().map(|pt| {
            let p = pt.payload;
            DocHit {
                chunk_id: chunk_id.to_string(),
                doc_id: Self::payload_str(&p, "doc_id"),
                source_path: Self::payload_str(&p, "source_path"),
                title: Self::payload_str(&p, "title"),
                nature: Self::payload_str(&p, "nature"),
                page: Self::payload_u32(&p, "page"),
                section_path: Self::payload_str(&p, "section_path"),
                chunk_index: Self::payload_u32(&p, "chunk_index"),
                text: Self::payload_str(&p, "text"),
                score: 0.0,
            }
        }))
    }
}
