// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

//! The controlled vocabulary that keeps tags coherent over time.
//!
//! `_taxonomy.yaml` declares **axes** (each with allowed `valori` and an
//! `alias` map) and **campi** (frontmatter fields, each optionally bound to an
//! axis). Every value written through the Moon is normalised (case-fold +
//! alias) and validated against its axis. Unknown values are *rejected* with
//! fuzzy suggestions; new values enter the vocabulary only via an explicit
//! `wiki_add_tag` call after the user approves — never silently.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::frontmatter::Document;

#[derive(Debug, Error)]
pub enum TaxonomyError {
    #[error("taxonomy file not found at {0}")]
    NotFound(String),
    #[error("cannot read taxonomy: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid taxonomy YAML: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("unknown axis '{0}'")]
    UnknownAxis(String),
}

/// One classification axis: its canonical values plus alias → canonical map.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Axis {
    #[serde(default)]
    pub valori: Vec<String>,
    #[serde(default)]
    pub alias: BTreeMap<String, String>,
}

/// How a frontmatter field is validated.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldSpec {
    /// "controlled" (bound to an axis), "free", "text" or "date".
    #[serde(default = "default_field_tipo")]
    pub tipo: String,
    /// Axis name when `tipo == "controlled"`.
    #[serde(default)]
    pub asse: Option<String>,
    /// Whether the field holds a list of values.
    #[serde(default)]
    pub lista: bool,
}

fn default_field_tipo() -> String {
    "free".to_string()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Taxonomy {
    #[serde(default)]
    pub assi: BTreeMap<String, Axis>,
    #[serde(default)]
    pub campi: BTreeMap<String, FieldSpec>,
}

/// A field value that did not resolve to a known axis value.
#[derive(Debug, Clone, Serialize)]
pub struct UnknownValue {
    pub campo: String,
    pub asse: String,
    pub valore: String,
    pub suggerimenti: Vec<String>,
}

/// Outcome of validating one document's frontmatter against the taxonomy.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ValidationReport {
    /// field -> canonical value(s) (only for controlled fields that resolved).
    pub canonical: BTreeMap<String, Vec<String>>,
    pub unknown: Vec<UnknownValue>,
}

impl ValidationReport {
    pub fn is_ok(&self) -> bool {
        self.unknown.is_empty()
    }
}

impl Taxonomy {
    pub fn load(path: &Path) -> Result<Taxonomy, TaxonomyError> {
        if !path.exists() {
            return Err(TaxonomyError::NotFound(path.display().to_string()));
        }
        let raw = std::fs::read_to_string(path)?;
        let tax: Taxonomy = serde_yaml::from_str(&raw)?;
        Ok(tax)
    }

    pub fn save(&self, path: &Path) -> Result<(), TaxonomyError> {
        let yaml = serde_yaml::to_string(self)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, yaml)?;
        Ok(())
    }

    /// Resolve a raw value on an axis to its canonical form.
    /// Order: exact (case-insensitive) value match → alias (case-insensitive).
    pub fn normalize(&self, axis: &str, value: &str) -> Option<String> {
        let ax = self.assi.get(axis)?;
        let v = value.trim();
        let v_lower = v.to_lowercase();
        if let Some(canon) = ax.valori.iter().find(|c| c.to_lowercase() == v_lower) {
            return Some(canon.clone());
        }
        for (alias, canon) in &ax.alias {
            if alias.to_lowercase() == v_lower {
                return Some(canon.clone());
            }
        }
        None
    }

    /// Top fuzzy suggestions for an unknown value on an axis.
    pub fn suggest(&self, axis: &str, value: &str, limit: usize) -> Vec<String> {
        let Some(ax) = self.assi.get(axis) else {
            return Vec::new();
        };
        let v_lower = value.trim().to_lowercase();
        let mut scored: Vec<(f64, &String)> = ax
            .valori
            .iter()
            .map(|c| (strsim::jaro_winkler(&v_lower, &c.to_lowercase()), c))
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scored
            .into_iter()
            .filter(|(score, _)| *score >= 0.6)
            .take(limit)
            .map(|(_, c)| c.clone())
            .collect()
    }

    /// Validate a document's frontmatter; report canonical forms and unknowns.
    pub fn validate(&self, doc: &Document) -> ValidationReport {
        let mut report = ValidationReport::default();
        for (field, spec) in &self.campi {
            if spec.tipo != "controlled" {
                continue;
            }
            let Some(axis) = spec.asse.as_deref() else {
                continue;
            };
            let raw_values = if spec.lista {
                doc.get_list(field)
            } else {
                doc.get_str(field).into_iter().collect()
            };
            let mut canon_values = Vec::new();
            for raw in raw_values {
                if raw.is_empty() {
                    continue;
                }
                match self.normalize(axis, &raw) {
                    Some(canon) => canon_values.push(canon),
                    None => report.unknown.push(UnknownValue {
                        campo: field.clone(),
                        asse: axis.to_string(),
                        valore: raw.clone(),
                        suggerimenti: self.suggest(axis, &raw, 3),
                    }),
                }
            }
            if !canon_values.is_empty() {
                report.canonical.insert(field.clone(), canon_values);
            }
        }
        report
    }

    /// Apply the canonical forms from a validation report back onto a document,
    /// so the file stored on disk always uses canonical tags.
    pub fn canonicalize(&self, doc: &mut Document) {
        let report = self.validate(doc);
        for (field, values) in report.canonical {
            let spec = match self.campi.get(&field) {
                Some(s) => s,
                None => continue,
            };
            if spec.lista {
                doc.set_list(&field, &values);
            } else if let Some(first) = values.first() {
                doc.set_str(&field, first);
            }
        }
    }

    /// Add a value (and optional aliases) to an axis. Idempotent on the value.
    pub fn add_value(
        &mut self,
        axis: &str,
        value: &str,
        aliases: &[String],
    ) -> Result<String, TaxonomyError> {
        let ax = self
            .assi
            .get_mut(axis)
            .ok_or_else(|| TaxonomyError::UnknownAxis(axis.to_string()))?;
        let value = value.trim().to_string();
        if !ax.valori.iter().any(|v| v.eq_ignore_ascii_case(&value)) {
            ax.valori.push(value.clone());
            ax.valori.sort();
        }
        for a in aliases {
            let a = a.trim().to_string();
            if !a.is_empty() {
                ax.alias.insert(a, value.clone());
            }
        }
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Taxonomy {
        let yaml = r#"
assi:
  reparto:
    valori: [amministrazione, commerciale, tecnico]
    alias:
      admin: amministrazione
      Amministrazione: amministrazione
      sales: commerciale
  stato:
    valori: [in-corso, bloccato, chiuso]
campi:
  reparto: { tipo: controlled, asse: reparto }
  stato: { tipo: controlled, asse: stato }
  referenti: { tipo: controlled, asse: referenti, lista: true }
"#;
        serde_yaml::from_str(yaml).unwrap()
    }

    #[test]
    fn normalize_alias_and_case() {
        let t = sample();
        assert_eq!(t.normalize("reparto", "ADMIN").as_deref(), Some("amministrazione"));
        assert_eq!(t.normalize("reparto", "Amministrazione").as_deref(), Some("amministrazione"));
        assert_eq!(t.normalize("reparto", "Commerciale").as_deref(), Some("commerciale"));
        assert_eq!(t.normalize("reparto", "marketing"), None);
    }

    #[test]
    fn validate_reports_unknown_with_suggestions() {
        let t = sample();
        let doc = Document::parse("---\nreparto: aministrazione\nstato: in-corso\n---\nx").unwrap();
        let r = t.validate(&doc);
        assert!(!r.is_ok());
        assert_eq!(r.unknown.len(), 1);
        assert_eq!(r.unknown[0].campo, "reparto");
        assert!(r.unknown[0].suggerimenti.contains(&"amministrazione".to_string()));
        assert_eq!(r.canonical.get("stato").unwrap(), &vec!["in-corso".to_string()]);
    }

    #[test]
    fn add_value_grows_axis() {
        let mut t = sample();
        t.add_value("reparto", "legale", &["legal".to_string()]).unwrap();
        assert_eq!(t.normalize("reparto", "legale").as_deref(), Some("legale"));
        assert_eq!(t.normalize("reparto", "legal").as_deref(), Some("legale"));
    }
}
