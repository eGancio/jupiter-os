// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

//! The MCP surface of Moon Ganymede. Reading the wiki is done by Claude with
//! the built-in `Read`/`Grep`/`Glob` tools; this server only exposes the
//! *write/validate/index* operations, so every mutation passes through
//! frontmatter validation, tag canonicalisation, atomic writes and automatic
//! re-indexing.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::Local;
use rmcp::handler::server::tool::{Parameters, ToolRouter};
use rmcp::model::*;
use rmcp::{tool, tool_handler, tool_router, ServerHandler};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::Mutex;
use tracing::info;

use crate::config::Config;
use crate::fact;
use crate::frontmatter::Document;
use crate::index;
use crate::taxonomy::Taxonomy;

// ===========================================================================
// Server struct
// ===========================================================================

/// Moon Ganymede MCP Server — the LLM Wiki write/validate/index surface.
pub struct MoonGanymedeServer {
    pub config: Config,
    /// Serialises commits/reindex across SSE connections (one writer at a time).
    write_lock: Arc<Mutex<()>>,
    tool_router: ToolRouter<Self>,
}

// ===========================================================================
// Parameter structs — doc comments become JSON schema descriptions for the LLM
// ===========================================================================

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ValidateTagsParams {
    /// The proposed file content (frontmatter `---` block + body), OR just the
    /// YAML frontmatter. Tags are normalised and validated; nothing is written.
    content: String,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct FindTargetParams {
    /// An id, a title, or a free name of the entity/affair to locate.
    query: String,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct CommitFactParams {
    /// Destination path, relative to the wiki root (e.g. "progetti/acme.md").
    /// Must end in `.md` and stay inside the wiki.
    path: String,
    /// Full file content: frontmatter `---` block + Markdown body.
    content: String,
    /// If true, values not yet in the taxonomy are added to their axis (after
    /// the user approved them). If false (default), unknown tags are rejected.
    #[serde(default)]
    allow_new_tags: bool,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct AddTagParams {
    /// Axis name (e.g. "societa", "reparto", "referenti").
    asse: String,
    /// Canonical value to add (e.g. "acme-srl").
    valore: String,
    /// Optional synonyms that should map to this canonical value.
    #[serde(default)]
    aliases: Vec<String>,
}

// ===========================================================================
// Tools
// ===========================================================================

#[tool_router]
impl MoonGanymedeServer {
    pub fn new(config: Config) -> Self {
        Self::with_lock(config, Arc::new(Mutex::new(())))
    }

    pub fn with_lock(config: Config, write_lock: Arc<Mutex<()>>) -> Self {
        Self {
            config,
            write_lock,
            tool_router: Self::tool_router(),
        }
    }

    fn load_taxonomy(&self) -> Result<Taxonomy, String> {
        Taxonomy::load(&self.config.taxonomy_path())
            .map_err(|e| format!("Tassonomia non caricabile: {e}"))
    }

    /// Return the controlled vocabulary (axes, allowed values, aliases, fields).
    /// Consult this before proposing tags.
    #[tool(
        name = "wiki_list_taxonomy",
        description = "Restituisce il vocabolario controllato del Wiki: assi, valori ammessi, alias e campi. Consultalo PRIMA di proporre tag, per usare valori esistenti."
    )]
    pub async fn wiki_list_taxonomy(&self) -> Result<String, String> {
        let tax = self.load_taxonomy()?;
        serde_json::to_string_pretty(&tax).map_err(|e| format!("Serializzazione: {e}"))
    }

    /// Normalise + validate the tags in a proposed fact (read-only, no write).
    #[tool(
        name = "wiki_validate_tags",
        description = "Normalizza e valida i tag del frontmatter proposto (case-fold + alias). NON scrive. Ritorna i valori canonici e l'elenco dei valori sconosciuti con suggerimenti. Chiamalo prima di mostrare la bozza all'utente."
    )]
    pub async fn wiki_validate_tags(
        &self,
        params: Parameters<ValidateTagsParams>,
    ) -> Result<String, String> {
        let tax = self.load_taxonomy()?;
        let doc = parse_doc_or_frontmatter(&params.0.content)?;
        let report = tax.validate(&doc);
        Ok(json!({
            "ok": report.is_ok(),
            "canonical": report.canonical,
            "unknown": report.unknown,
        })
        .to_string())
    }

    /// Locate an existing fact-file for an entity/affair, by id/title/name.
    #[tool(
        name = "wiki_find_target",
        description = "Cerca un file-entità esistente nel Wiki (per id, titolo o nome). Ritorna il path da AGGIORNARE se trovato, altrimenti 'new' per creare un nuovo file. Usalo per evitare duplicati."
    )]
    pub async fn wiki_find_target(
        &self,
        params: Parameters<FindTargetParams>,
    ) -> Result<String, String> {
        let facts = fact::scan_wiki(&self.config.wiki_dir);
        match fact::find_target(&facts, &params.0.query) {
            Some((s, how, score)) => Ok(json!({
                "found": true,
                "path": s.rel_path,
                "id": s.id,
                "titolo": s.titolo,
                "match": how,
                "score": score,
            })
            .to_string()),
            None => {
                let candidates: Vec<_> = facts
                    .iter()
                    .take(10)
                    .map(|s| json!({"path": s.rel_path, "id": s.id, "titolo": s.titolo}))
                    .collect();
                Ok(json!({
                    "found": false,
                    "suggestion": "new",
                    "slug": fact::slugify(&params.0.query),
                    "candidates": candidates,
                })
                .to_string())
            }
        }
    }

    /// Validate, write atomically (with backup) and re-index a fact-file.
    /// This is the ONLY supported write path into the wiki.
    #[tool(
        name = "wiki_commit_fact",
        description = "Scrive un fatto nel Wiki: valida i tag, fa backup, scrittura atomica e rigenera gli indici. UNICO modo di scrivere nel Wiki (non usare Write diretto). Con tag sconosciuti fallisce, salvo allow_new_tags=true (solo dopo conferma utente)."
    )]
    pub async fn wiki_commit_fact(
        &self,
        params: Parameters<CommitFactParams>,
    ) -> Result<String, String> {
        let p = params.0;
        let _guard = self.write_lock.lock().await;

        // 1. Parse + resolve destination (must stay inside the wiki).
        let mut doc = parse_doc_or_frontmatter(&p.content)?;
        let abs_path = resolve_wiki_path(&self.config.wiki_dir, &p.path)?;

        // 2. Ensure a stable id (default from filename stem).
        if doc.get_str("id").unwrap_or_default().is_empty() {
            if let Some(stem) = abs_path.file_stem().and_then(|s| s.to_str()) {
                doc.set_str("id", stem);
            }
        }

        // 3. Validate; optionally absorb new values into the taxonomy.
        let mut tax = self.load_taxonomy()?;
        let report = tax.validate(&doc);
        if !report.is_ok() {
            if p.allow_new_tags {
                for u in &report.unknown {
                    tax.add_value(&u.asse, &u.valore, &[])
                        .map_err(|e| format!("Aggiunta valore '{}' a '{}': {e}", u.valore, u.asse))?;
                }
                tax.save(&self.config.taxonomy_path())
                    .map_err(|e| format!("Salvataggio tassonomia: {e}"))?;
            } else {
                return Ok(json!({
                    "ok": false,
                    "error": "tag_sconosciuti",
                    "message": "Alcuni valori non sono nel vocabolario. Conferma con l'utente e ripeti con allow_new_tags=true, oppure correggi i tag con i suggerimenti.",
                    "unknown": report.unknown,
                })
                .to_string());
            }
        }

        // 4. Canonicalise tags onto the document.
        tax.canonicalize(&mut doc);

        // 5. Serialise.
        let final_content = doc
            .to_string()
            .map_err(|e| format!("Serializzazione documento: {e}"))?;

        // 6. Backup existing file, then atomic write.
        let existed = abs_path.exists();
        if existed {
            backup_file(&self.config.history_dir(), &self.config.wiki_dir, &abs_path)?;
        }
        atomic_write(&abs_path, &final_content)?;
        info!(
            "Committed fact to {:?} ({})",
            abs_path,
            if existed { "updated" } else { "created" }
        );

        // 7. Reindex.
        let reindexed = index::reindex(&self.config.wiki_dir, &tax)
            .map_err(|e| format!("Reindex: {e}"))?;

        let rel = abs_path
            .strip_prefix(&self.config.wiki_dir)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| p.path.clone());

        Ok(json!({
            "ok": true,
            "path": rel,
            "action": if existed { "updated" } else { "created" },
            "canonical": tax.validate(&doc).canonical,
            "reindexed": reindexed,
        })
        .to_string())
    }

    /// Add a new value (+ optional aliases) to a taxonomy axis. Use only after
    /// the user has approved introducing the new tag.
    #[tool(
        name = "wiki_add_tag",
        description = "Aggiunge un valore (con eventuali alias) a un asse della tassonomia. Usare SOLO dopo conferma esplicita dell'utente di voler introdurre un nuovo tag."
    )]
    pub async fn wiki_add_tag(&self, params: Parameters<AddTagParams>) -> Result<String, String> {
        let p = params.0;
        let _guard = self.write_lock.lock().await;
        let mut tax = self.load_taxonomy()?;
        let canon = tax
            .add_value(&p.asse, &p.valore, &p.aliases)
            .map_err(|e| format!("{e}"))?;
        tax.save(&self.config.taxonomy_path())
            .map_err(|e| format!("Salvataggio tassonomia: {e}"))?;
        Ok(json!({"ok": true, "asse": p.asse, "valore": canon, "aliases": p.aliases}).to_string())
    }

    /// Rebuild all view-indices from scratch.
    #[tool(
        name = "wiki_reindex",
        description = "Rigenera da zero tutti gli indici-vista (_index/by-*.md). Utile dopo modifiche manuali ai file .md."
    )]
    pub async fn wiki_reindex(&self) -> Result<String, String> {
        let _guard = self.write_lock.lock().await;
        let tax = self.load_taxonomy()?;
        let written = index::reindex(&self.config.wiki_dir, &tax)
            .map_err(|e| format!("Reindex: {e}"))?;
        Ok(json!({"ok": true, "reindexed": written}).to_string())
    }
}

// ===========================================================================
// ServerHandler implementation — MCP protocol
// ===========================================================================

#[tool_handler]
impl ServerHandler for MoonGanymedeServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::default(),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation {
                name: "moon-ganymede".to_string(),
                version: "0.1.0".to_string(),
            },
            instructions: Some(
                "Moon Ganymede — LLM Wiki (memoria operativa). I file .md si LEGGONO con i \
                 tool built-in (Read/Grep/Glob); si SCRIVONO solo con wiki_commit_fact. \
                 Prima di scrivere: consulta wiki_list_taxonomy, individua il file con \
                 wiki_find_target, valida con wiki_validate_tags, mostra la bozza e attendi \
                 conferma."
                    .into(),
            ),
        }
    }
}

// ===========================================================================
// Helpers
// ===========================================================================

/// Parse full `.md` content; if it has no frontmatter delimiters, treat the
/// whole string as a YAML frontmatter block.
fn parse_doc_or_frontmatter(content: &str) -> Result<Document, String> {
    let trimmed = content.trim_start_matches('\u{feff}');
    let candidate = if trimmed.trim_start().starts_with("---") {
        content.to_string()
    } else {
        format!("---\n{}\n---\n", content.trim())
    };
    Document::parse(&candidate).map_err(|e| format!("Frontmatter non valido: {e}"))
}

/// Resolve a wiki-relative path safely: must be `.md`, inside the wiki, and not
/// a reserved file (`_*`, `_index/`, `.history/`).
fn resolve_wiki_path(wiki_dir: &Path, rel: &str) -> Result<PathBuf, String> {
    let rel = rel.trim().replace('\\', "/");
    if rel.is_empty() {
        return Err("Path vuoto.".into());
    }
    if !rel.ends_with(".md") {
        return Err("Il path deve terminare con .md".into());
    }
    let relp = Path::new(&rel);
    if relp.is_absolute() {
        return Err("Usa un path relativo alla radice del Wiki.".into());
    }
    for comp in relp.components() {
        use std::path::Component;
        match comp {
            Component::ParentDir => return Err("'..' non consentito nel path.".into()),
            Component::Normal(s) => {
                let s = s.to_string_lossy();
                if s.starts_with('_') || s == ".history" {
                    return Err(format!(
                        "'{s}' è riservato (indici/tassonomia/template/backup): non scrivibile."
                    ));
                }
            }
            _ => {}
        }
    }
    Ok(wiki_dir.join(relp))
}

/// Copy an existing file into `.history/` before overwriting it.
fn backup_file(history_dir: &Path, wiki_dir: &Path, file: &Path) -> Result<(), String> {
    std::fs::create_dir_all(history_dir).map_err(|e| format!("Backup dir: {e}"))?;
    let rel = file
        .strip_prefix(wiki_dir)
        .unwrap_or(file)
        .to_string_lossy()
        .replace(['/', '\\'], "__");
    let stamp = Local::now().format("%Y%m%d-%H%M%S");
    let backup = history_dir.join(format!("{rel}.{stamp}.bak"));
    std::fs::copy(file, &backup).map_err(|e| format!("Backup: {e}"))?;
    Ok(())
}

/// Write atomically: temp file in the same dir, then rename over the target.
fn atomic_write(path: &Path, content: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("Crea cartella: {e}"))?;
    }
    let tmp = path.with_extension("md.tmp");
    std::fs::write(&tmp, content).map_err(|e| format!("Scrittura temp: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("Rename atomico: {e}"))?;
    Ok(())
}
