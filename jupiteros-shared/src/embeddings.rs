// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use ort::session::Session;
use ort::value::Tensor;
use tracing::{debug, info};

use crate::error::{Result, SharedError};

// XLM-RoBERTa special tokens — excluded from sparse lexical weights.
const XLMR_CLS_ID: u32 = 0; // <s>
const XLMR_PAD_ID: u32 = 1; // <pad>
const XLMR_SEP_ID: u32 = 2; // </s>
const XLMR_UNK_ID: u32 = 3; // <unk>
const XLMR_MASK_ID: u32 = 250001; // <mask>

const BGE_M3_DENSE_DIM: usize = 1024;

/// Maximum tokens per input sequence. BGE-M3 supports up to 8192, but the
/// attention matrices grow quadratically with sequence length AND linearly
/// with batch size. Capping at 512 keeps per-batch RAM usage bounded while
/// covering ~95% of real-world emails without losing meaningful content.
const MAX_SEQ_LEN: usize = 512;

/// Maximum number of sequences per ONNX forward pass. Larger batches are
/// chunked internally. With 24 transformer layers and 512 tokens, 16
/// sequences fit comfortably in a few GB of working memory.
const ONNX_BATCH_SIZE: usize = 16;

/// Sparse lexical embedding produced by BGE-M3: a map from vocabulary token id
/// to its learned weight (only non-zero entries kept).
#[derive(Debug, Clone, Default)]
pub struct SparseEmbedding {
    pub indices: Vec<u32>,
    pub values: Vec<f32>,
}

/// Hybrid output: dense semantic vector + sparse lexical weights.
#[derive(Debug, Clone)]
pub struct HybridEmbedding {
    pub dense: Vec<f32>,
    pub sparse: SparseEmbedding,
}

/// ONNX-based embedding model — currently configured for BGE-M3 (INT8 quantized).
///
/// Produces both dense (1024-dim, CLS-pooled, L2-normalized) and sparse
/// (token-weighted lexical) representations in a single forward pass.
///
/// Inputs to the ONNX session: `input_ids`, `attention_mask` (no token_type_ids).
/// Outputs (by position):
///   - outputs[0] = dense last_hidden_state [batch, seq_len, hidden_size]
///   - outputs[1] = sparse token weights [batch, seq_len, 1] (already ReLU'd)
///   - outputs[2] = ColBERT embeddings (not consumed yet)
pub struct OnnxEmbedding {
    session: Mutex<Session>,
    tokenizer: tokenizers::Tokenizer,
    dense_size: usize,
}

impl OnnxEmbedding {
    /// Load ONNX model + tokenizer from a directory.
    ///
    /// Expects `model.onnx` and `tokenizer.json` inside `model_dir`.
    pub fn load(model_dir: &Path) -> Result<Self> {
        let model_path = model_dir.join("model.onnx");
        let tokenizer_path = model_dir.join("tokenizer.json");

        if !model_path.exists() {
            return Err(SharedError::Embedding(format!(
                "Model not found: {}",
                model_path.display()
            )));
        }
        if !tokenizer_path.exists() {
            return Err(SharedError::Embedding(format!(
                "Tokenizer not found: {}",
                tokenizer_path.display()
            )));
        }

        info!("Loading ONNX model from {}", model_path.display());
        let session = Session::builder()
            .map_err(|e| SharedError::Embedding(format!("Session builder: {e}")))?
            .commit_from_file(&model_path)
            .map_err(|e| SharedError::Embedding(format!("Load model: {e}")))?;

        let mut tokenizer = tokenizers::Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| SharedError::Embedding(format!("Load tokenizer: {e}")))?;

        // Cap tokenization at MAX_SEQ_LEN so pathological inputs (huge base64
        // blobs, inlined images, minified HTML) can't blow up BPE encoding.
        // Without this the tokenizer chews the full ~30k-char body before the
        // model ever sees the 512-token cap, which can wedge a whole batch.
        tokenizer
            .with_truncation(Some(tokenizers::TruncationParams {
                max_length: MAX_SEQ_LEN,
                ..Default::default()
            }))
            .map_err(|e| SharedError::Embedding(format!("Set tokenizer truncation: {e}")))?;

        info!("BGE-M3 embedding model loaded ({} dense dims, sparse enabled)", BGE_M3_DENSE_DIM);

        Ok(Self {
            session: Mutex::new(session),
            tokenizer,
            dense_size: BGE_M3_DENSE_DIM,
        })
    }

    /// Embed a single text into a dense vector (for backward compatibility).
    pub fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let results = self.embed_batch(&[text])?;
        results
            .into_iter()
            .next()
            .ok_or_else(|| SharedError::Embedding("embed_batch returned empty results".into()))
    }

    /// Embed multiple texts into dense vectors (more efficient for indexing).
    pub fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        let hybrids = self.embed_batch_hybrid(texts)?;
        Ok(hybrids.into_iter().map(|h| h.dense).collect())
    }

    /// Embed a single text into a hybrid (dense + sparse) representation.
    pub fn embed_hybrid(&self, text: &str) -> Result<HybridEmbedding> {
        let mut results = self.embed_batch_hybrid(&[text])?;
        results
            .pop()
            .ok_or_else(|| SharedError::Embedding("embed_batch_hybrid returned empty results".into()))
    }

    /// Embed multiple texts into hybrid (dense + sparse) representations,
    /// chunked internally to avoid ONNX Runtime memory blow-ups (BGE-M3 has
    /// 24 layers — attention matrices grow as `batch * seq * seq * 4`).
    pub fn embed_batch_hybrid(&self, texts: &[&str]) -> Result<Vec<HybridEmbedding>> {
        if texts.is_empty() {
            return Ok(vec![]);
        }
        let mut out = Vec::with_capacity(texts.len());
        for chunk in texts.chunks(ONNX_BATCH_SIZE) {
            let mut part = self.embed_batch_hybrid_inner(chunk)?;
            out.append(&mut part);
        }
        Ok(out)
    }

    /// Single forward pass over a chunk of at most `ONNX_BATCH_SIZE` texts.
    fn embed_batch_hybrid_inner(&self, texts: &[&str]) -> Result<Vec<HybridEmbedding>> {
        if texts.is_empty() {
            return Ok(vec![]);
        }

        // Tokenize all texts.
        let encodings = self
            .tokenizer
            .encode_batch(texts.to_vec(), true)
            .map_err(|e| SharedError::Embedding(format!("Tokenize: {e}")))?;

        let batch_size = encodings.len();
        let max_len = encodings
            .iter()
            .map(|e| e.get_ids().len())
            .max()
            .unwrap_or(0)
            .min(MAX_SEQ_LEN);

        // Build padded input arrays. BGE-M3 (XLM-RoBERTa) uses only input_ids + attention_mask.
        let mut input_ids_data = vec![XLMR_PAD_ID as i64; batch_size * max_len];
        let mut attention_mask_data = vec![0i64; batch_size * max_len];

        for (i, encoding) in encodings.iter().enumerate() {
            let ids = encoding.get_ids();
            let mask = encoding.get_attention_mask();
            let len = ids.len().min(max_len);

            for j in 0..len {
                input_ids_data[i * max_len + j] = ids[j] as i64;
                attention_mask_data[i * max_len + j] = mask[j] as i64;
            }
        }

        let shape = [batch_size, max_len];
        let input_ids_tensor = Tensor::from_array((shape, input_ids_data))
            .map_err(|e| SharedError::Embedding(format!("Build input_ids tensor: {e}")))?;
        let attention_mask_tensor = Tensor::from_array((shape, attention_mask_data))
            .map_err(|e| SharedError::Embedding(format!("Build attention_mask tensor: {e}")))?;

        // Run ONNX inference.
        let mut session = self
            .session
            .lock()
            .map_err(|e| SharedError::Embedding(format!("Lock session: {e}")))?;
        let outputs = session
            .run(ort::inputs! {
                "input_ids" => input_ids_tensor,
                "attention_mask" => attention_mask_tensor,
            })
            .map_err(|e| SharedError::Embedding(format!("Inference: {e}")))?;

        // --------- Output 0: dense (last_hidden_state) → CLS pooling ----------
        let (dense_shape, dense_data) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| SharedError::Embedding(format!("Extract dense tensor: {e}")))?;

        let dense_dims: Vec<usize> = dense_shape.iter().map(|&d| d as usize).collect();
        let dense_seq_len = if dense_dims.len() >= 2 { dense_dims[1] } else { max_len };
        let dense_hidden = if dense_dims.len() >= 3 { dense_dims[2] } else { self.dense_size };

        // --------- Output 1: sparse token weights -----------------------------
        // Expected shape: [batch, seq_len, 1] (per-token learned weight, already ReLU'd).
        // If the export is [batch, seq_len], we handle that too.
        let (sparse_shape, sparse_data) = outputs[1]
            .try_extract_tensor::<f32>()
            .map_err(|e| SharedError::Embedding(format!("Extract sparse tensor: {e}")))?;

        let sparse_dims: Vec<usize> = sparse_shape.iter().map(|&d| d as usize).collect();
        let sparse_seq_len = if sparse_dims.len() >= 2 { sparse_dims[1] } else { max_len };
        let sparse_last = if sparse_dims.len() >= 3 { sparse_dims[2] } else { 1 };

        let mut results = Vec::with_capacity(batch_size);

        for i in 0..batch_size {
            // ----- Dense: CLS token pooling (first token) + L2 normalize -----
            let mut dense = vec![0f32; self.dense_size];
            let cls_offset = i * dense_seq_len * dense_hidden;
            for k in 0..dense_hidden.min(self.dense_size) {
                let idx = cls_offset + k;
                if idx < dense_data.len() {
                    dense[k] = dense_data[idx];
                }
            }
            let norm: f32 = dense.iter().map(|v| v * v).sum::<f32>().sqrt();
            if norm > 0.0 {
                for v in dense.iter_mut() {
                    *v /= norm;
                }
            }

            // ----- Sparse: aggregate per-token weights into {token_id → max_weight} -----
            let ids = encodings[i].get_ids();
            let mask = encodings[i].get_attention_mask();
            let mut sparse_map: HashMap<u32, f32> = HashMap::new();

            for j in 0..sparse_seq_len {
                // Skip padding positions.
                if mask.get(j).copied().unwrap_or(0) == 0 {
                    continue;
                }
                let tok_id = ids.get(j).copied().unwrap_or(XLMR_PAD_ID);
                if is_special_token(tok_id) {
                    continue;
                }
                let w_idx = i * sparse_seq_len * sparse_last + j * sparse_last;
                if w_idx >= sparse_data.len() {
                    continue;
                }
                let w = sparse_data[w_idx];
                if w <= 0.0 {
                    continue;
                }
                sparse_map
                    .entry(tok_id)
                    .and_modify(|prev| {
                        if w > *prev {
                            *prev = w;
                        }
                    })
                    .or_insert(w);
            }

            let mut sparse_pairs: Vec<(u32, f32)> = sparse_map.into_iter().collect();
            sparse_pairs.sort_by_key(|&(k, _)| k);
            let (indices, values): (Vec<u32>, Vec<f32>) = sparse_pairs.into_iter().unzip();

            results.push(HybridEmbedding {
                dense,
                sparse: SparseEmbedding { indices, values },
            });
        }

        debug!("Embedded {} texts (hybrid dense+sparse)", batch_size);
        Ok(results)
    }

    /// Dense vector dimensionality (1024 for BGE-M3).
    pub fn vector_size(&self) -> usize {
        self.dense_size
    }

    /// Alias for `vector_size()`.
    pub fn dimension(&self) -> usize {
        self.dense_size
    }
}

#[inline]
fn is_special_token(id: u32) -> bool {
    id == XLMR_CLS_ID
        || id == XLMR_PAD_ID
        || id == XLMR_SEP_ID
        || id == XLMR_UNK_ID
        || id == XLMR_MASK_ID
}

// OnnxEmbedding is Send+Sync because Session and Tokenizer are thread-safe.
unsafe impl Send for OnnxEmbedding {}
unsafe impl Sync for OnnxEmbedding {}
