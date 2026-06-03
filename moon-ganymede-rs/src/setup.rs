// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

//! First-run setup: materialise the wiki skeleton (directories, an opinionated
//! default `_taxonomy.yaml`, and the canonical `_template.md`) if they don't
//! exist yet. Both files are plain text the user can edit by hand afterwards;
//! the agent then sticks to whatever the taxonomy declares.

use tracing::info;

use crate::config::Config;
use crate::index;
use crate::taxonomy::Taxonomy;

/// Opinionated default controlled vocabulary. Hand-written (with comments) so
/// the user can read and tweak it. Open axes (società, referenti, tags) start
/// empty and grow via `wiki_add_tag` after explicit confirmation.
pub const DEFAULT_TAXONOMY: &str = r#"# Tassonomia del Wiki (Moon Ganymede)
# Vocabolario controllato dei tag. Modificabile a mano: l'agente vi si attiene.
#
# - assi: ogni asse ha `valori` ammessi e una mappa `alias` (sinonimo -> canonico).
# - campi: i campi del frontmatter; quelli `controlled` sono legati a un asse e
#   vengono validati alla scrittura.
#
# Gli assi "aperti" (societa, referenti, tags) partono vuoti e crescono solo
# tramite wiki_add_tag, dopo conferma dell'utente. Niente valori silenziosi.

assi:
  tipo:
    valori: [progetto, cliente, fornitore, persona, area, altro]
    alias:
      client: cliente
      supplier: fornitore
      project: progetto
  reparto:
    valori: [amministrazione, commerciale, tecnico, legale, marketing, hr]
    alias:
      admin: amministrazione
      sales: commerciale
      it: tecnico
      legal: legale
  stato:
    valori: [in-corso, bloccato, in-attesa, chiuso]
    alias:
      aperto: in-corso
      open: in-corso
      attivo: in-corso
      done: chiuso
      completato: chiuso
      blocked: bloccato
  priorita:
    valori: [alta, media, bassa]
    alias:
      high: alta
      medium: media
      low: bassa
  societa:
    valori: []
    alias: {}
  referenti:
    valori: []
    alias: {}
  tags:
    valori: []
    alias: {}

campi:
  tipo: { tipo: controlled, asse: tipo }
  societa: { tipo: controlled, asse: societa }
  reparto: { tipo: controlled, asse: reparto }
  stato: { tipo: controlled, asse: stato }
  priorita: { tipo: controlled, asse: priorita }
  referenti: { tipo: controlled, asse: referenti, lista: true }
  tags: { tipo: controlled, asse: tags, lista: true }
"#;

/// Canonical fact template. `tipo` decides the top-level folder; all other
/// axes are queried through the generated view-indices.
pub const DEFAULT_TEMPLATE: &str = r#"---
id: ESEMPIO-slug-stabile
tipo: progetto
titolo: "Titolo leggibile"
societa: ""
reparto: ""
referenti: []
stato: in-corso
priorita: media
tags: []
creato: 2026-01-01
aggiornato: 2026-01-01
---

## Sintesi
Una-due righe sullo stato attuale (questa parte si riscrive).

## Stato / Timeline
- 2026-01-01 — primo aggiornamento (append-only, datato)

## Decisioni
- 2026-01-01 — decisione presa — con chi

## Referenti
- Nome Cognome (ruolo, contatto)

## Questioni aperte
- [ ] domanda o azione pendente

## Note
"#;

/// Ensure the wiki directory tree and seed files exist. Idempotent.
pub fn ensure_wiki_skeleton(config: &Config) -> anyhow::Result<()> {
    std::fs::create_dir_all(&config.wiki_dir)?;
    std::fs::create_dir_all(config.index_dir())?;
    std::fs::create_dir_all(config.history_dir())?;

    let tax_path = config.taxonomy_path();
    if !tax_path.exists() {
        std::fs::write(&tax_path, DEFAULT_TAXONOMY)?;
        info!("Wrote default taxonomy at {:?}", tax_path);
    }

    let tpl_path = config.template_path();
    if !tpl_path.exists() {
        std::fs::write(&tpl_path, DEFAULT_TEMPLATE)?;
        info!("Wrote default template at {:?}", tpl_path);
    }

    // Build the initial indices so a fresh wiki already has a browsable README.
    if let Ok(tax) = Taxonomy::load(&tax_path) {
        let _ = index::reindex(&config.wiki_dir, &tax);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_taxonomy_parses() {
        let tax: Taxonomy = serde_yaml::from_str(DEFAULT_TAXONOMY).unwrap();
        assert!(tax.assi.contains_key("reparto"));
        assert_eq!(
            tax.normalize("reparto", "ADMIN").as_deref(),
            Some("amministrazione")
        );
        assert!(tax.campi.get("referenti").unwrap().lista);
    }

    #[test]
    fn default_template_has_valid_frontmatter() {
        use crate::frontmatter::Document;
        let doc = Document::parse(DEFAULT_TEMPLATE).unwrap();
        assert_eq!(doc.get_str("tipo").as_deref(), Some("progetto"));
        assert!(doc.body.contains("## Decisioni"));
    }
}
