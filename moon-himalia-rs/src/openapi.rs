// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

//! Client per le API imprese di **openapi.it** (prodotto "visure camerali").
//!
//! Flusso reale dell'API (asincrono, restituisce DOCUMENTI non JSON di numeri):
//!   1. `POST /{tipo}`            con `{"cf_piva_id": "..."}`  → ottieni un `id`
//!   2. `GET  /{tipo}/{id}`       polling finché lo stato = "Visura evasa"
//!   3. `GET  /{tipo}/{id}/allegati` → zip base64 con PDF + XBRL + verbale
//!
//! Per il bilancio l'XBRL è machine-readable: lo parsiamo in numeri chiave; il PDF
//! lo salviamo su disco e ne restituiamo il path, così la chat può passarlo a Metis
//! (`metis_ingest`) e renderlo una fonte CITABILE. Auth: `Authorization: Bearer`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::Engine;
use serde::Serialize;
use serde_json::Value;
use tracing::{info, warn};

const PROD_HOST: &str = "visurecamerali.openapi.it";
const SANDBOX_HOST: &str = "test.visurecamerali.openapi.it";
const COMPANY_HOST: &str = "company.openapi.com";

/// Stima del costo (€, IVA esclusa) per tipo documento. Conservativa: serve solo
/// al guardrail di spesa, non è la fattura. Il bilancio ottico è ~4,50€.
pub fn costo_stimato_eur(tipo: &str) -> f64 {
    match tipo {
        "bilancio-ottico" => 4.50,
        t if t.starts_with("ordinaria") => 5.50,
        t if t.starts_with("storica") => 7.00,
        _ => 5.00,
    }
}

/// Tipi documento supportati (whitelist → endpoint path 1:1).
pub fn tipo_valido(tipo: &str) -> bool {
    matches!(
        tipo,
        "bilancio-ottico"
            | "ordinaria-societa-capitale"
            | "storica-societa-capitale"
            | "ordinaria-societa-persone"
            | "storica-societa-persone"
            | "ordinaria-impresa-individuale"
            | "storica-impresa-individuale"
    )
}

/// Un file estratto dagli allegati e salvato su disco.
#[derive(Debug, Clone, Serialize)]
pub struct SavedFile {
    pub path: String,
    /// "pdf" | "xbrl" | "other"
    pub kind: String,
}

/// Esito di una richiesta documento andata a buon fine.
#[derive(Debug, Clone, Serialize)]
pub struct DocumentResult {
    pub id: String,
    pub tipo: String,
    pub cf_piva: String,
    pub files: Vec<SavedFile>,
    /// Path del PDF principale (da passare a Metis), se presente.
    pub pdf_path: Option<String>,
    /// Numeri chiave estratti dall'XBRL (vuoto = non estratti, vedi `note`).
    pub bilancio: BTreeMap<String, FattoBilancio>,
    pub note: String,
}

/// Un dato di bilancio estratto dall'XBRL, con l'anno del contesto.
#[derive(Debug, Clone, Serialize)]
pub struct FattoBilancio {
    pub valore: f64,
    pub anno: Option<i32>,
}

#[derive(Clone)]
pub struct OpenApiClient {
    http: reqwest::Client,
    token: String,
    company_token: String,
    sandbox: bool,
    output_dir: PathBuf,
}

impl OpenApiClient {
    pub fn new(
        token: impl Into<String>,
        company_token: impl Into<String>,
        sandbox: bool,
        output_dir: PathBuf,
    ) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            http,
            token: token.into(),
            company_token: company_token.into(),
            sandbox,
            output_dir,
        }
    }

    fn host(&self) -> &'static str {
        if self.sandbox {
            SANDBOX_HOST
        } else {
            PROD_HOST
        }
    }

    // ---- 1. POST: crea la richiesta, ritorna l'id --------------------------

    pub async fn richiedi(
        &self,
        tipo: &str,
        cf_piva: &str,
        anno: Option<i32>,
    ) -> Result<String, String> {
        let url = format!("https://{}/{}", self.host(), tipo);
        let mut body = serde_json::Map::new();
        body.insert("cf_piva_id".into(), Value::String(cf_piva.to_string()));
        if let Some(a) = anno {
            body.insert("anno_chiusura".into(), Value::from(a));
        }
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&self.token)
            .json(&Value::Object(body))
            .send()
            .await
            .map_err(|e| format!("POST {url} fallita: {e}"))?;
        let v = parse_or_err(resp, &url).await?;
        find_id(&v).ok_or_else(|| {
            format!("openapi: nessun id nella risposta a {url}: {}", compact(&v))
        })
    }

    // ---- 2. GET stato ------------------------------------------------------

    pub async fn stato(&self, tipo: &str, id: &str) -> Result<(String, Value), String> {
        let url = format!("https://{}/{}/{}", self.host(), tipo, id);
        let resp = self
            .http
            .get(&url)
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|e| format!("GET {url} fallita: {e}"))?;
        let v = parse_or_err(resp, &url).await?;
        let stato = find_state(&v).unwrap_or_else(|| "sconosciuto".into());
        Ok((stato, v))
    }

    /// Polling con backoff finché lo stato indica "evasa" o scade il timeout.
    pub async fn attendi(
        &self,
        tipo: &str,
        id: &str,
        timeout: Duration,
    ) -> Result<(), String> {
        let start = tokio::time::Instant::now();
        let mut wait = Duration::from_secs(2);
        loop {
            let (stato, _) = self.stato(tipo, id).await?;
            if stato_pronto(&stato) {
                return Ok(());
            }
            if start.elapsed() >= timeout {
                return Err(format!(
                    "Timeout: la richiesta {id} è ancora in stato '{stato}'. \
                     Riprova più tardi con stato_richiesta(id='{id}', tipo='{tipo}')."
                ));
            }
            info!("openapi: richiesta {id} stato '{stato}', attendo {:?}", wait);
            tokio::time::sleep(wait).await;
            wait = (wait * 2).min(Duration::from_secs(10));
        }
    }

    // ---- 3. GET allegati + salvataggio ------------------------------------

    pub async fn scarica_allegati(
        &self,
        tipo: &str,
        id: &str,
        cf_piva: &str,
    ) -> Result<Vec<SavedFile>, String> {
        let url = format!("https://{}/{}/{}/allegati", self.host(), tipo, id);
        let resp = self
            .http
            .get(&url)
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|e| format!("GET {url} fallita: {e}"))?;
        let v = parse_or_err(resp, &url).await?;

        // L'API impacchetta i documenti in base64 (zip o file singoli) dentro il
        // JSON, ma lo schema esatto del campo varia. Strategia robusta: raccogli
        // OGNI stringa base64 plausibile, decodificala e riconoscila dal magic.
        let mut blobs: Vec<Vec<u8>> = Vec::new();
        raccogli_base64(&v, &mut blobs);
        if blobs.is_empty() {
            return Err(format!(
                "openapi: nessun allegato decodificabile in {url}: {}",
                compact(&v)
            ));
        }

        let dir = self.output_dir.join(format!("{cf_piva}_{tipo}_{id}"));
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("Impossibile creare {}: {e}", dir.display()))?;

        let mut saved = Vec::new();
        for (i, blob) in blobs.into_iter().enumerate() {
            saved.extend(salva_blob(&dir, i, &blob));
        }
        if saved.is_empty() {
            return Err("openapi: allegati decodificati ma nessun file salvato".into());
        }
        Ok(saved)
    }

    // ---- dati_impresa: fatturato + bilancio sintetico + soci (/IT-advanced) --

    /// GET company.openapi.com/IT-advanced/{cf_piva} → dato strutturato (fatturato,
    /// totale attivo, patrimonio netto, costo personale, dipendenti, soci, storico).
    /// Ritorna il primo record di `data`. Usa il token company (non quello visure).
    pub async fn dati_impresa(&self, cf_piva: &str) -> Result<Value, String> {
        let url = format!("https://{COMPANY_HOST}/IT-advanced/{}", urlencoding(cf_piva));
        let resp = self
            .http
            .get(&url)
            .bearer_auth(&self.company_token)
            .send()
            .await
            .map_err(|e| format!("GET {url} fallita: {e}"))?;
        let v = parse_or_err(resp, &url).await?;
        v.pointer("/data/0").cloned().ok_or_else(|| {
            format!("Nessun dato per '{cf_piva}' (impresa inesistente o non società di capitali).")
        })
    }

    // ---- cerca_impresa: nome → P.IVA (company.openapi.com, token separato) --

    pub async fn cerca_impresa(
        &self,
        denominazione: &str,
        provincia: Option<&str>,
    ) -> Result<Value, String> {
        let mut url = format!(
            "https://{COMPANY_HOST}/IT-search?denominazione={}",
            urlencoding(denominazione)
        );
        if let Some(p) = provincia {
            url.push_str(&format!("&provincia={}", urlencoding(p)));
        }
        let resp = self
            .http
            .get(&url)
            .bearer_auth(&self.company_token)
            .send()
            .await
            .map_err(|e| format!("GET {url} fallita: {e}"))?;
        parse_or_err(resp, &url).await
    }
}

// ===========================================================================
// Helpers JSON / parsing risposta
// ===========================================================================

async fn parse_or_err(resp: reqwest::Response, url: &str) -> Result<Value, String> {
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("openapi {status} su {url}: {}", trunc(&text, 400)));
    }
    let v: Value = serde_json::from_str(&text)
        .map_err(|e| format!("openapi: risposta non-JSON da {url}: {e} — {}", trunc(&text, 200)))?;
    // openapi.it incapsula in {"success":bool,"message":..,"data":..}
    if v.get("success").and_then(Value::as_bool) == Some(false) {
        let msg = v
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("errore non specificato");
        return Err(format!("openapi success=false su {url}: {msg}"));
    }
    Ok(v)
}

/// Cerca un id di richiesta nelle posizioni note (data.id, id, data.data.id).
fn find_id(v: &Value) -> Option<String> {
    let candidates = [
        v.pointer("/data/id"),
        v.pointer("/id"),
        v.pointer("/data/data/id"),
        v.pointer("/data/request_id"),
        v.pointer("/request_id"),
    ];
    for c in candidates.into_iter().flatten() {
        match c {
            Value::String(s) if !s.is_empty() => return Some(s.clone()),
            Value::Number(n) => return Some(n.to_string()),
            _ => {}
        }
    }
    None
}

/// Estrae il campo stato dalle posizioni note.
fn find_state(v: &Value) -> Option<String> {
    let candidates = [
        v.pointer("/data/stato_richiesta"),
        v.pointer("/data/state"),
        v.pointer("/data/status"),
        v.pointer("/stato_richiesta"),
        v.pointer("/state"),
        v.pointer("/status"),
    ];
    for c in candidates.into_iter().flatten() {
        if let Value::String(s) = c {
            if !s.is_empty() {
                return Some(s.clone());
            }
        }
    }
    None
}

pub fn stato_pronto(stato: &str) -> bool {
    let s = stato.to_ascii_lowercase();
    s.contains("evas") || s == "completed" || s == "done" || s == "success"
}

/// Raccoglie ricorsivamente ogni stringa che si decodifica come base64 "lunga".
fn raccogli_base64(v: &Value, out: &mut Vec<Vec<u8>>) {
    match v {
        Value::String(s) if s.len() > 100 => {
            let cleaned: String = s.chars().filter(|c| !c.is_whitespace()).collect();
            if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&cleaned) {
                if bytes.len() > 64 {
                    out.push(bytes);
                }
            }
        }
        Value::Array(a) => a.iter().for_each(|x| raccogli_base64(x, out)),
        Value::Object(o) => o.values().for_each(|x| raccogli_base64(x, out)),
        _ => {}
    }
}

/// Riconosce un blob dal magic number e lo salva (zip → estrai, pdf/xml → file).
fn salva_blob(dir: &Path, idx: usize, blob: &[u8]) -> Vec<SavedFile> {
    if blob.starts_with(b"PK\x03\x04") {
        return estrai_zip(dir, blob);
    }
    if blob.starts_with(b"%PDF") {
        let p = dir.join(format!("documento_{idx}.pdf"));
        if std::fs::write(&p, blob).is_ok() {
            return vec![SavedFile {
                path: p.to_string_lossy().into_owned(),
                kind: "pdf".into(),
            }];
        }
    }
    if sembra_xml(blob) {
        let p = dir.join(format!("dati_{idx}.xml"));
        if std::fs::write(&p, blob).is_ok() {
            return vec![SavedFile {
                path: p.to_string_lossy().into_owned(),
                kind: "xbrl".into(),
            }];
        }
    }
    Vec::new()
}

fn estrai_zip(dir: &Path, blob: &[u8]) -> Vec<SavedFile> {
    let reader = std::io::Cursor::new(blob);
    let mut zip = match zip::ZipArchive::new(reader) {
        Ok(z) => z,
        Err(e) => {
            warn!("openapi: zip non leggibile: {e}");
            return Vec::new();
        }
    };
    let mut out = Vec::new();
    for i in 0..zip.len() {
        let mut entry = match zip.by_index(i) {
            Ok(e) => e,
            Err(_) => continue,
        };
        if entry.is_dir() {
            continue;
        }
        let name = sanitize(entry.name());
        let mut buf = Vec::new();
        if std::io::copy(&mut entry, &mut buf).is_err() {
            continue;
        }
        let lower = name.to_ascii_lowercase();
        let kind = if lower.ends_with(".pdf") || buf.starts_with(b"%PDF") {
            "pdf"
        } else if lower.ends_with(".xbrl") || lower.ends_with(".xml") || sembra_xml(&buf) {
            "xbrl"
        } else {
            "other"
        };
        let p = dir.join(&name);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        if std::fs::write(&p, &buf).is_ok() {
            out.push(SavedFile {
                path: p.to_string_lossy().into_owned(),
                kind: kind.into(),
            });
        }
    }
    out
}

fn sembra_xml(b: &[u8]) -> bool {
    let head: Vec<u8> = b.iter().copied().take(64).filter(|c| !c.is_ascii_whitespace()).collect();
    head.starts_with(b"<?xml") || head.starts_with(b"<")
}

fn sanitize(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let cleaned: String = base
        .chars()
        .map(|c| if c.is_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '_' })
        .collect();
    if cleaned.is_empty() { "file".into() } else { cleaned }
}

fn urlencoding(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            b' ' => "%20".to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn compact(v: &Value) -> String {
    trunc(&v.to_string(), 300)
}

fn trunc(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_string()
    } else {
        format!("{}…", &s[..n])
    }
}

// ===========================================================================
// Parsing XBRL (tassonomia ITCC) — best effort
// ===========================================================================

/// Pattern (substring del local-name, in minuscolo) → etichetta leggibile.
/// Substring matching = tollerante alle varianti di versione della tassonomia.
const DIZIONARIO: &[(&str, &str)] = &[
    ("ricavivenditeprestazioni", "Ricavi vendite e prestazioni"),
    ("totalevaloreproduzione", "Valore della produzione"),
    ("valoreproduzionetotale", "Valore della produzione"),
    ("totalecostiproduzione", "Costi della produzione"),
    ("differenzavalorecostiproduzione", "Differenza valore-costi (EBIT)"),
    ("utileperditaesercizio", "Utile/perdita d'esercizio"),
    ("risultatoesercizio", "Utile/perdita d'esercizio"),
    ("totaleattivo", "Totale attivo"),
    ("totalepatrimonionetto", "Patrimonio netto"),
    ("totaledebiti", "Totale debiti"),
];

/// Parsa l'XBRL e ritorna i numeri chiave, tenendo per ogni concetto l'anno più recente.
pub fn parse_xbrl(xml: &str) -> BTreeMap<String, FattoBilancio> {
    let mut out: BTreeMap<String, FattoBilancio> = BTreeMap::new();
    let doc = match roxmltree::Document::parse(xml) {
        Ok(d) => d,
        Err(e) => {
            warn!("XBRL non parsabile: {e}");
            return out;
        }
    };

    // 1) contextRef → anno
    let mut ctx_anno: BTreeMap<String, i32> = BTreeMap::new();
    for ctx in doc.descendants().filter(|n| n.tag_name().name() == "context") {
        if let Some(id) = ctx.attribute("id") {
            if let Some(anno) = anno_da_contesto(ctx) {
                ctx_anno.insert(id.to_string(), anno);
            }
        }
    }

    // 2) fatti
    for node in doc.descendants().filter(|n| n.is_element()) {
        let local = node.tag_name().name().to_ascii_lowercase();
        let Some(label) = DIZIONARIO.iter().find(|(p, _)| local.contains(p)).map(|(_, l)| *l)
        else {
            continue;
        };
        let Some(ctx_ref) = node.attribute("contextRef") else {
            continue;
        };
        let Some(valore) = node.text().and_then(parse_num) else {
            continue;
        };
        let anno = ctx_anno.get(ctx_ref).copied();
        match out.get(label) {
            Some(prev) if prev.anno >= anno => {} // tieni il più recente
            _ => {
                out.insert(label.to_string(), FattoBilancio { valore, anno });
            }
        }
    }
    out
}

fn anno_da_contesto(ctx: roxmltree::Node) -> Option<i32> {
    // period → instant | endDate (preferisci endDate/instant = data di chiusura)
    for tag in ["instant", "endDate", "startDate"] {
        if let Some(n) = ctx.descendants().find(|n| n.tag_name().name() == tag) {
            if let Some(txt) = n.text() {
                if let Ok(anno) = txt.trim().get(0..4).unwrap_or("").parse::<i32>() {
                    return Some(anno);
                }
            }
        }
    }
    None
}

fn parse_num(s: &str) -> Option<f64> {
    let t = s.trim().replace([' ', '\u{a0}'], "");
    if t.is_empty() {
        return None;
    }
    // XBRL usa il punto come separatore decimale; rimuovi eventuali separatori migliaia.
    t.parse::<f64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    // XBRL sintetico in stile ITCC: due esercizi (2022, 2023). Verifica che il
    // parser mappi contextRef→anno e tenga il valore dell'anno PIÙ RECENTE.
    const SAMPLE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<xbrli:xbrl xmlns:xbrli="http://www.xbrl.org/2003/instance" xmlns:itcc-ci="http://www.infocamere.it/itcc">
  <xbrli:context id="C2022">
    <xbrli:period><xbrli:instant>2022-12-31</xbrli:instant></xbrli:period>
  </xbrli:context>
  <xbrli:context id="C2023">
    <xbrli:period><xbrli:instant>2023-12-31</xbrli:instant></xbrli:period>
  </xbrli:context>
  <itcc-ci:TotaleAttivo contextRef="C2022">900000</itcc-ci:TotaleAttivo>
  <itcc-ci:TotaleAttivo contextRef="C2023">1200000.50</itcc-ci:TotaleAttivo>
  <itcc-ci:TotaleValoreProduzione contextRef="C2023">2500000</itcc-ci:TotaleValoreProduzione>
  <itcc-ci:UtilePerditaEsercizio contextRef="C2023">150000</itcc-ci:UtilePerditaEsercizio>
</xbrli:xbrl>"#;

    #[test]
    fn xbrl_estrae_anno_piu_recente() {
        let m = parse_xbrl(SAMPLE);
        let attivo = m.get("Totale attivo").expect("totale attivo mancante");
        assert_eq!(attivo.anno, Some(2023));
        assert_eq!(attivo.valore, 1_200_000.50);
        assert_eq!(m.get("Valore della produzione").unwrap().valore, 2_500_000.0);
        assert_eq!(m.get("Utile/perdita d'esercizio").unwrap().anno, Some(2023));
    }

    #[test]
    fn tipo_e_costo_coerenti() {
        assert!(tipo_valido("bilancio-ottico"));
        assert!(!tipo_valido("inesistente"));
        assert_eq!(costo_stimato_eur("bilancio-ottico"), 4.50);
    }

    #[test]
    fn stato_pronto_riconosce_evasa() {
        assert!(stato_pronto("Visura evasa"));
        assert!(!stato_pronto("in lavorazione"));
    }
}
