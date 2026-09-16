// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

//! View-index generation. The same fact files are projected along every
//! controlled axis (`_index/by-<field>.md`) so the wiki can be browsed by
//! reparto, società, stato, … without duplicating any file. Indices are
//! generated artefacts: they carry a "do not edit" header and are rebuilt on
//! every commit (and on demand via `wiki_reindex`). Links use the relative
//! path to the source file.

use std::collections::BTreeMap;
use std::path::Path;

use crate::fact::{self, FactSummary};
use crate::taxonomy::Taxonomy;

const GENERATED_HEADER: &str =
    "> ⚙️ File generato automaticamente da Moon Ganymede. NON modificare a mano: \
     verrà sovrascritto al prossimo `wiki_commit_fact` / `wiki_reindex`.\n";

/// Rebuild all view indices under `<wiki>/_index/`. Returns the relative paths
/// of the files written.
pub fn reindex(wiki_dir: &Path, taxonomy: &Taxonomy) -> std::io::Result<Vec<String>> {
    let index_dir = wiki_dir.join("_index");
    std::fs::create_dir_all(&index_dir)?;

    let facts = fact::scan_wiki(wiki_dir);
    let mut written = Vec::new();

    // One index per controlled field, in deterministic (sorted) order.
    for (field, spec) in &taxonomy.campi {
        if spec.tipo != "controlled" {
            continue;
        }
        let content = render_field_index(field, &facts, spec.lista);
        let fname = format!("by-{}.md", field);
        std::fs::write(index_dir.join(&fname), content)?;
        written.push(format!("_index/{fname}"));
    }

    // Overview README.
    let readme = render_readme(taxonomy, &facts);
    std::fs::write(index_dir.join("README.md"), readme)?;
    written.push("_index/README.md".to_string());

    Ok(written)
}

fn render_field_index(field: &str, facts: &[FactSummary], is_list: bool) -> String {
    // value -> facts that carry it
    let mut groups: BTreeMap<String, Vec<&FactSummary>> = BTreeMap::new();
    let mut missing: Vec<&FactSummary> = Vec::new();

    for f in facts {
        let values = if is_list {
            f.doc.get_list(field)
        } else {
            f.doc.get_str(field).into_iter().collect()
        };
        if values.is_empty() {
            missing.push(f);
        } else {
            for v in values {
                groups.entry(v).or_default().push(f);
            }
        }
    }

    let mut out = String::new();
    out.push_str(&format!("# Indice per `{field}`\n\n"));
    out.push_str(GENERATED_HEADER);
    out.push('\n');

    if groups.is_empty() && missing.is_empty() {
        out.push_str("_Nessun fatto presente._\n");
        return out;
    }

    for (value, items) in &groups {
        out.push_str(&format!("## {value} ({})\n\n", items.len()));
        for f in items {
            out.push_str(&fact_line(f));
        }
        out.push('\n');
    }

    if !missing.is_empty() {
        out.push_str(&format!("## (senza `{field}`) ({})\n\n", missing.len()));
        for f in &missing {
            out.push_str(&fact_line(f));
        }
        out.push('\n');
    }

    out
}

fn fact_line(f: &FactSummary) -> String {
    let label = if f.titolo.is_empty() {
        f.id.clone()
    } else {
        f.titolo.clone()
    };
    let mut meta = Vec::new();
    if !f.stato.is_empty() {
        meta.push(f.stato.clone());
    }
    if !f.aggiornato.is_empty() {
        meta.push(format!("agg. {}", f.aggiornato));
    }
    let suffix = if meta.is_empty() {
        String::new()
    } else {
        format!(" — {}", meta.join(", "))
    };
    // Links are relative to `_index/`, hence the leading `../`.
    format!("- [{label}](../{}){suffix}\n", f.rel_path)
}

fn render_readme(taxonomy: &Taxonomy, facts: &[FactSummary]) -> String {
    let mut out = String::new();
    out.push_str("# Wiki — Panoramica\n\n");
    out.push_str(GENERATED_HEADER);
    out.push('\n');
    out.push_str(&format!("**Fatti totali:** {}\n\n", facts.len()));

    out.push_str("## Indici-vista disponibili\n\n");
    let mut fields: Vec<&String> = taxonomy
        .campi
        .iter()
        .filter(|(_, s)| s.tipo == "controlled")
        .map(|(f, _)| f)
        .collect();
    fields.sort();
    for f in &fields {
        out.push_str(&format!("- [per {f}](by-{f}.md)\n"));
    }
    out.push('\n');

    out.push_str("## Aggiornati di recente\n\n");
    for f in facts.iter().take(15) {
        out.push_str(&fact_line(f));
    }
    out.push('\n');

    out
}
