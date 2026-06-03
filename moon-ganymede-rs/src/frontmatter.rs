// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

//! Parsing and serialisation of Markdown files with a YAML frontmatter block.
//!
//! The frontmatter is intentionally kept as an *ordered, untyped* mapping
//! (`serde_yaml::Mapping`) instead of a fixed struct: the taxonomy — and hence
//! the set of fields — is user-customisable, so we must round-trip arbitrary
//! keys without losing them. Typed access to well-known fields (id, stato, …)
//! is provided through helper methods.

use serde_yaml::{Mapping, Value};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum FrontmatterError {
    #[error("missing frontmatter: file must start with a '---' YAML block")]
    Missing,
    #[error("unterminated frontmatter: no closing '---' found")]
    Unterminated,
    #[error("frontmatter is not a YAML mapping (key: value pairs)")]
    NotAMapping,
    #[error("invalid YAML in frontmatter: {0}")]
    Yaml(#[from] serde_yaml::Error),
}

/// A parsed `.md` document: ordered frontmatter mapping + raw body.
#[derive(Debug, Clone)]
pub struct Document {
    pub frontmatter: Mapping,
    pub body: String,
}

impl Document {
    /// Parse a full `.md` file into frontmatter + body.
    pub fn parse(content: &str) -> Result<Document, FrontmatterError> {
        let (fm_str, body) = split(content)?;
        let value: Value = serde_yaml::from_str(fm_str)?;
        let frontmatter = match value {
            Value::Mapping(m) => m,
            // An empty frontmatter (`---\n---`) parses to Null; treat as empty map.
            Value::Null => Mapping::new(),
            _ => return Err(FrontmatterError::NotAMapping),
        };
        Ok(Document {
            frontmatter,
            body: body.to_string(),
        })
    }

    /// Serialise back to a `.md` file with a `---` frontmatter block.
    pub fn to_string(&self) -> Result<String, FrontmatterError> {
        let yaml = serde_yaml::to_string(&Value::Mapping(self.frontmatter.clone()))?;
        // serde_yaml already ends with a newline; ensure the body is separated.
        let body = self.body.trim_start_matches('\n');
        Ok(format!("---\n{yaml}---\n{body}"))
    }

    /// Read a well-known scalar field as a trimmed string.
    pub fn get_str(&self, key: &str) -> Option<String> {
        match self.frontmatter.get(Value::from(key))? {
            Value::String(s) => Some(s.trim().to_string()),
            Value::Number(n) => Some(n.to_string()),
            Value::Bool(b) => Some(b.to_string()),
            _ => None,
        }
    }

    /// Read a field as a list of strings, accepting both a scalar and a sequence.
    pub fn get_list(&self, key: &str) -> Vec<String> {
        match self.frontmatter.get(Value::from(key)) {
            Some(Value::Sequence(seq)) => seq
                .iter()
                .filter_map(|v| match v {
                    Value::String(s) => Some(s.trim().to_string()),
                    Value::Number(n) => Some(n.to_string()),
                    _ => None,
                })
                .filter(|s| !s.is_empty())
                .collect(),
            Some(Value::String(s)) if !s.trim().is_empty() => vec![s.trim().to_string()],
            _ => Vec::new(),
        }
    }

    pub fn set_str(&mut self, key: &str, value: &str) {
        self.frontmatter
            .insert(Value::from(key), Value::from(value));
    }

    pub fn set_list(&mut self, key: &str, values: &[String]) {
        let seq = values.iter().map(|s| Value::from(s.as_str())).collect();
        self.frontmatter
            .insert(Value::from(key), Value::Sequence(seq));
    }

    /// All frontmatter keys, in document order.
    pub fn keys(&self) -> Vec<String> {
        self.frontmatter
            .keys()
            .filter_map(|k| k.as_str().map(|s| s.to_string()))
            .collect()
    }
}

/// Split a `.md` file into (frontmatter_yaml, body). The file must start with
/// `---` on its own line and have a closing `---` line.
pub fn split(content: &str) -> Result<(&str, &str), FrontmatterError> {
    let trimmed = content.trim_start_matches('\u{feff}'); // strip BOM if present
    let rest = trimmed
        .strip_prefix("---\n")
        .or_else(|| trimmed.strip_prefix("---\r\n"))
        .ok_or(FrontmatterError::Missing)?;

    // Find the closing delimiter line.
    let mut offset = 0usize;
    for line in rest.split_inclusive('\n') {
        let bare = line.trim_end_matches(['\n', '\r']);
        if bare == "---" {
            let fm = &rest[..offset];
            let body = &rest[offset + line.len()..];
            return Ok((fm, body));
        }
        offset += line.len();
    }
    Err(FrontmatterError::Unterminated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_roundtrip() {
        let src = "---\nid: acme\nstato: in-corso\ntags:\n- fornitori\n- contratto\n---\n## Sintesi\nhello\n";
        let doc = Document::parse(src).unwrap();
        assert_eq!(doc.get_str("id").as_deref(), Some("acme"));
        assert_eq!(doc.get_str("stato").as_deref(), Some("in-corso"));
        assert_eq!(doc.get_list("tags"), vec!["fornitori", "contratto"]);
        assert!(doc.body.contains("## Sintesi"));
        // Re-serialising must keep the frontmatter parseable.
        let out = doc.to_string().unwrap();
        let doc2 = Document::parse(&out).unwrap();
        assert_eq!(doc2.get_str("id").as_deref(), Some("acme"));
    }

    #[test]
    fn missing_frontmatter() {
        assert!(matches!(
            Document::parse("no frontmatter here"),
            Err(FrontmatterError::Missing)
        ));
    }

    #[test]
    fn unterminated_frontmatter() {
        assert!(matches!(
            Document::parse("---\nid: acme\n"),
            Err(FrontmatterError::Unterminated)
        ));
    }

    #[test]
    fn scalar_as_list() {
        let doc = Document::parse("---\nreferenti: mario-rossi\n---\nbody").unwrap();
        assert_eq!(doc.get_list("referenti"), vec!["mario-rossi"]);
    }
}
