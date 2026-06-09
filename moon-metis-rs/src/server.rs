// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

//! The MCP surface of Moon Metis — the knowledge layer.
//!
//! Tools are deterministic Rust "muscle": preview / ingest / search over the
//! local document knowledge base. The "judgement" (deciding a document's nature
//! and synthesising a cited answer) is the AGENT's job — Metis only returns the
//! cited evidence. Hard rule for the agent: **cite or abstain** (every claim
//! must carry source + page/section, or say it cannot be found).

use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;

use rmcp::handler::server::tool::{Parameters, ToolRouter};
use rmcp::model::*;
use rmcp::{tool, tool_handler, tool_router, ServerHandler};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::Mutex;

use jupiteros_shared::DocStore;

use crate::config::Config;
use crate::extract;
use crate::ingest;
use crate::llm::LlmClient;
use crate::procedure;
use crate::state::IngestState;

// ===========================================================================
// Parameter structs — doc comments become JSON schema descriptions for the LLM
// ===========================================================================

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct PreviewParams {
    /// Percorso del file da ispezionare (PDF, DOCX, XLSX, TXT, MD, …).
    file_path: String,
    /// Quante pagine iniziali restituire per il giudizio sulla natura (default 3).
    #[serde(default)]
    pages: Option<u32>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct IngestParams {
    /// Percorso del file da indicizzare.
    file_path: String,
    /// Natura del documento: generico|norma|bando|preventivo. Se assente, "generico".
    /// Scegli la natura DOPO aver letto il preview (è il tuo giudizio).
    #[serde(default)]
    nature: Option<String>,
    /// Reindicizza anche se il file è invariato (default false).
    #[serde(default)]
    force: bool,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct IngestFolderParams {
    /// Percorso della cartella da indicizzare.
    dir: String,
    /// Scendi nelle sottocartelle (default true).
    #[serde(default = "default_true")]
    recursive: bool,
    /// Natura da applicare a TUTTI i file della cartella (se omogenea). Vuoto = generico.
    #[serde(default)]
    nature: Option<String>,
    /// Reindicizza anche i file invariati (default false).
    #[serde(default)]
    force: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct SearchParams {
    /// La domanda / query in linguaggio naturale.
    query: String,
    /// Filtra per natura (generico|norma|bando|preventivo). Vuoto = tutte.
    #[serde(default)]
    nature: Option<String>,
    /// Limita a un documento specifico (doc_id). Vuoto = tutti.
    #[serde(default)]
    doc_id: Option<String>,
    /// Numero massimo di passaggi da restituire (default 8).
    #[serde(default)]
    limit: Option<u32>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct DocIdParams {
    /// Identificativo del documento (doc_id) restituito da ingest/list/search.
    doc_id: String,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ChunkIdParams {
    /// Identificativo del chunk (chunk_id) restituito dalla ricerca.
    chunk_id: String,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ClassifyParams {
    /// Percorso del file di cui indovinare la natura tramite l'LLM locale.
    file_path: String,
}

// ===========================================================================
// Server
// ===========================================================================

pub struct MoonMetisServer {
    config: Config,
    store: Arc<DocStore>,
    state: Arc<Mutex<IngestState>>,
    llm: LlmClient,
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl MoonMetisServer {
    pub fn new(config: Config, store: Arc<DocStore>, state: Arc<Mutex<IngestState>>) -> Self {
        let llm = LlmClient::new(&config.llm_base_url, &config.llm_model, &config.llm_api_key);
        Self {
            config,
            store,
            state,
            llm,
            tool_router: Self::tool_router(),
        }
    }

    fn state_path(&self) -> PathBuf {
        self.config.state_path()
    }

    /// Preview: estrai le prime pagine così l'AGENTE può decidere la natura.
    #[tool(
        name = "metis_preview",
        description = "Estrae le prime pagine di un documento (testo page-aware) per farti DECIDERE la natura (generico|norma|bando|preventivo) prima di indicizzarlo. Segnala se il PDF è scansionato (immagine, non ancora indicizzabile)."
    )]
    pub async fn metis_preview(
        &self,
        params: Parameters<PreviewParams>,
    ) -> Result<String, String> {
        let p = params.0;
        let path = PathBuf::from(&p.file_path);
        let n_pages = p.pages.unwrap_or(3).max(1) as usize;

        let extraction = extract::extract(&path).map_err(|e| e.to_string())?;
        let preview: String = extraction
            .pages
            .iter()
            .take(n_pages)
            .map(|pg| {
                if pg.number > 0 {
                    format!("--- pagina {} ---\n{}", pg.number, pg.text)
                } else {
                    pg.text.clone()
                }
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        let preview: String = preview.chars().take(6000).collect();

        Ok(json!({
            "format": extraction.format,
            "is_scanned": extraction.is_scanned,
            "page_count": extraction.page_count(),
            "total_chars": extraction.total_chars(),
            "known_natures": procedure::KNOWN_NATURES,
            "preview": preview,
        })
        .to_string())
    }

    /// Ingest deterministico di un singolo file.
    #[tool(
        name = "metis_ingest",
        description = "Indicizza UN file nella knowledge base: estrazione page-aware → chunking strutturale (per natura) → embedding → Qdrant. Passa 'nature' dopo aver letto il preview. Incrementale: salta i file invariati (usa force=true per riforzare)."
    )]
    pub async fn metis_ingest(&self, params: Parameters<IngestParams>) -> Result<String, String> {
        let p = params.0;
        let path = PathBuf::from(&p.file_path);
        let mut state = self.state.lock().await;
        let result =
            ingest::ingest_file(&self.store, &mut state, &path, p.nature.as_deref(), p.force).await;
        state
            .save(&self.state_path())
            .map_err(|e| format!("Salvataggio stato: {e}"))?;
        serde_json::to_string(&result).map_err(|e| e.to_string())
    }

    /// Ingest di un'intera cartella (scope-level, resumabile).
    #[tool(
        name = "metis_ingest_folder",
        description = "Indicizza tutti i file supportati di una cartella (pdf/docx/xlsx/txt/md/csv/html…). Resumabile: i file invariati vengono saltati. Usa 'nature' solo se la cartella è omogenea, altrimenti lascia che ogni file vada come 'generico' e ricategorizza i singoli con metis_ingest."
    )]
    pub async fn metis_ingest_folder(
        &self,
        params: Parameters<IngestFolderParams>,
    ) -> Result<String, String> {
        let p = params.0;
        let dir = PathBuf::from(&p.dir);
        let mut state = self.state.lock().await;
        let results = ingest::ingest_folder(
            &self.store,
            &mut state,
            &self.state_path(),
            &dir,
            p.recursive,
            p.nature.as_deref(),
            p.force,
        )
        .await;

        let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for r in &results {
            *counts.entry(r.status.clone()).or_insert(0) += 1;
        }
        Ok(json!({
            "total_files": results.len(),
            "counts": counts,
            "results": results,
        })
        .to_string())
    }

    /// Ricerca CITATA nella knowledge base.
    #[tool(
        name = "metis_search",
        description = "Ricerca semantica+lessicale (ibrida) nella knowledge base. Restituisce passaggi con CITAZIONE (titolo, pagina, sezione). REGOLA: cita o astieniti — usa SOLO questi passaggi per rispondere, cita fonte+pagina; se non trovi, dillo."
    )]
    pub async fn metis_search(&self, params: Parameters<SearchParams>) -> Result<String, String> {
        let p = params.0;
        let limit = p.limit.unwrap_or(8).clamp(1, 50) as usize;
        let hits = self
            .store
            .search(&p.query, p.nature.as_deref(), p.doc_id.as_deref(), limit)
            .await
            .map_err(|e| format!("Ricerca: {e}"))?;

        let enriched: Vec<serde_json::Value> = hits
            .iter()
            .map(|h| {
                let citation = format_citation(&h.title, h.page, &h.section_path);
                json!({
                    "citation": citation,
                    "title": h.title,
                    "source_path": h.source_path,
                    "page": h.page,
                    "section": h.section_path,
                    "nature": h.nature,
                    "score": h.score,
                    "doc_id": h.doc_id,
                    "chunk_id": h.chunk_id,
                    "text": h.text,
                })
            })
            .collect();

        Ok(json!({
            "query": p.query,
            "hits": enriched,
            "note": "Rispondi citando fonte+pagina/sezione. Se i passaggi non bastano, astieniti.",
        })
        .to_string())
    }

    /// Elenco dei documenti indicizzati.
    #[tool(
        name = "metis_list_documents",
        description = "Elenca i documenti presenti nella knowledge base (titolo, natura, n. chunk, pagine, doc_id)."
    )]
    pub async fn metis_list_documents(&self) -> Result<String, String> {
        let docs = self
            .store
            .list_documents()
            .await
            .map_err(|e| format!("Elenco: {e}"))?;
        serde_json::to_string(&docs).map_err(|e| e.to_string())
    }

    /// Dettagli e indice dei chunk di un documento.
    #[tool(
        name = "metis_doc_info",
        description = "Restituisce i chunk di un documento (ordinati), con pagina e sezione: utile per ispezionare la struttura o recuperare il testo esatto da citare."
    )]
    pub async fn metis_doc_info(&self, params: Parameters<DocIdParams>) -> Result<String, String> {
        let p = params.0;
        let chunks = self
            .store
            .doc_chunks(&p.doc_id)
            .await
            .map_err(|e| format!("doc_info: {e}"))?;
        if chunks.is_empty() {
            return Err(format!("Nessun documento con doc_id {}", p.doc_id));
        }
        let first = &chunks[0];
        Ok(json!({
            "doc_id": first.doc_id,
            "title": first.title,
            "source_path": first.source_path,
            "nature": first.nature,
            "chunk_count": chunks.len(),
            "chunks": chunks.iter().map(|c| json!({
                "chunk_id": c.chunk_id,
                "chunk_index": c.chunk_index,
                "page": c.page,
                "section": c.section_path,
                "preview": c.text.chars().take(160).collect::<String>(),
            })).collect::<Vec<_>>(),
        })
        .to_string())
    }

    /// Recupera il testo integrale di un chunk (per citazione precisa).
    #[tool(
        name = "metis_get_chunk",
        description = "Restituisce il testo integrale di un singolo chunk (con citazione) dato il suo chunk_id."
    )]
    pub async fn metis_get_chunk(
        &self,
        params: Parameters<ChunkIdParams>,
    ) -> Result<String, String> {
        let p = params.0;
        let hit = self
            .store
            .get_chunk(&p.chunk_id)
            .await
            .map_err(|e| format!("get_chunk: {e}"))?
            .ok_or_else(|| format!("Nessun chunk con id {}", p.chunk_id))?;
        Ok(json!({
            "citation": format_citation(&hit.title, hit.page, &hit.section_path),
            "doc_id": hit.doc_id,
            "title": hit.title,
            "source_path": hit.source_path,
            "page": hit.page,
            "section": hit.section_path,
            "text": hit.text,
        })
        .to_string())
    }

    /// Classifica la natura via LLM locale (opzionale, batch/headless).
    #[tool(
        name = "metis_classify_nature",
        description = "Indovina la natura di un documento (generico|norma|bando|preventivo) usando l'LLM configurato in Jupiter (METIS_LLM_*). Utile in batch quando l'agente non legge il preview a mano. Se l'LLM non è configurato, restituisce un errore: in tal caso usa metis_preview e decidi tu."
    )]
    pub async fn metis_classify_nature(
        &self,
        params: Parameters<ClassifyParams>,
    ) -> Result<String, String> {
        if !self.config.llm_enabled() {
            return Err("LLM non configurato (METIS_LLM_BASE_URL). Usa metis_preview e decidi la natura.".into());
        }
        let p = params.0;
        let path = PathBuf::from(&p.file_path);
        let extraction = extract::extract(&path).map_err(|e| e.to_string())?;
        let preview: String = extraction
            .pages
            .iter()
            .take(3)
            .map(|pg| pg.text.clone())
            .collect::<Vec<_>>()
            .join("\n\n");
        let nature = self
            .llm
            .classify_nature(&preview)
            .await
            .map_err(|e| format!("Classificazione LLM: {e}"))?;
        Ok(json!({ "file_path": p.file_path, "nature": nature }).to_string())
    }
}

/// Build a compact human citation: "Titolo, p.3 — Art. 12".
fn format_citation(title: &str, page: u32, section: &str) -> String {
    let mut s = title.to_string();
    if page > 0 {
        s.push_str(&format!(", p.{page}"));
    }
    if !section.trim().is_empty() {
        s.push_str(&format!(" — {}", section.trim()));
    }
    s
}

#[tool_handler]
impl ServerHandler for MoonMetisServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::default(),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation {
                name: "moon-metis".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
            instructions: Some(
                "Moon Metis — knowledge layer locale e CITABILE. Flusso: metis_preview \
                 (decidi la natura) → metis_ingest/metis_ingest_folder → metis_search. \
                 REGOLA FERREA: cita o astieniti — rispondi solo dai passaggi restituiti, \
                 citando fonte + pagina/sezione; se non bastano, dillo."
                    .into(),
            ),
        }
    }
}
