// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

//! Scudo PII: anonimizzazione locale via backend rizzo-pii (NER 0.3B + regex
//! checksum, http://127.0.0.1:5005). Jupiter lo gestisce come Qdrant: adopt se
//! già vivo, altrimenti spawn del binario. Il dizionario {placeholder→valore}
//! resta nella sessione, su disco locale — mai trasmesso.

use std::collections::BTreeMap;
use std::net::{TcpStream, ToSocketAddrs};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use crate::config;

const PII_ADDR: &str = "127.0.0.1:5005";

fn child_slot() -> &'static Mutex<Option<Child>> {
    static CHILD: OnceLock<Mutex<Option<Child>>> = OnceLock::new();
    CHILD.get_or_init(|| Mutex::new(None))
}

fn tcp_up(addr: &str) -> bool {
    addr.to_socket_addrs()
        .ok()
        .and_then(|mut a| a.next())
        .map(|sa| TcpStream::connect_timeout(&sa, Duration::from_secs(2)).is_ok())
        .unwrap_or(false)
}

/// Path del backend: entry `rizzo-pii` nei `daemons` del .mcp.json, altrimenti
/// il path dell'installazione utente standard.
fn backend_command() -> (String, Vec<String>, BTreeMap<String, String>) {
    if let Ok(raw) = std::fs::read_to_string(config::mcp_json_path()) {
        if let Ok(cfg) = serde_json::from_str::<config::McpConfig>(&raw) {
            if let Some(def) = cfg.daemons.get("rizzo-pii") {
                return (def.command.clone(), def.args.clone(), def.env.clone());
            }
        }
    }
    let home = std::env::var("HOME").unwrap_or_default();
    (
        format!("{home}/.local/opt/rizzo-pii/usr/lib/Rizzo PII/backend/pii-backend/pii-backend"),
        Vec::new(),
        BTreeMap::new(),
    )
}

/// Avvia il backend PII se non già vivo. Non blocca: la readiness la aspetta
/// `wait_ready()` alla prima send con scudo attivo.
pub fn ensure_running() {
    if tcp_up(PII_ADDR) {
        return;
    }
    {
        let slot = child_slot().lock().unwrap();
        if slot.is_some() {
            return; // spawn già in corso (modello in caricamento)
        }
    }
    let (cmd, args, mut env) = backend_command();
    if !std::path::Path::new(&cmd).exists() {
        return; // nessun backend installato: l'errore chiaro esce alla send
    }
    env.entry("PII_HOST".into()).or_insert_with(|| "127.0.0.1".into());
    env.entry("PII_PORT".into()).or_insert_with(|| "5005".into());
    match Command::new(&cmd)
        .args(&args)
        .envs(&env)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => {
            *child_slot().lock().unwrap() = Some(child);
        }
        Err(_) => {}
    }
}

/// Attende che il backend risponda (il primo avvio carica il modello, ~10s).
pub fn wait_ready(timeout_secs: u64) -> bool {
    for _ in 0..timeout_secs {
        if tcp_up(PII_ADDR) {
            return true;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    false
}

pub fn shutdown() {
    if let Some(mut child) = child_slot().lock().unwrap().take() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// POST /analyze → (testo anonimizzato, mapping placeholder→valore).
pub async fn analyze(text: &str) -> Result<(String, BTreeMap<String, String>), String> {
    ensure_running();
    if !tcp_up(PII_ADDR) && !wait_ready(60) {
        return Err(
            "Scudo PII: il motore di anonimizzazione non risponde (backend rizzo-pii non \
             installato o non avviabile). Il messaggio NON è stato inviato."
                .to_string(),
        );
    }
    let resp = reqwest::Client::new()
        .post(format!("http://{PII_ADDR}/analyze"))
        .json(&serde_json::json!({ "text": text }))
        .timeout(Duration::from_secs(120))
        .send()
        .await
        .map_err(|e| format!("Scudo PII: errore di rete verso il motore locale: {e}"))?;
    let v: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Scudo PII: risposta non valida dal motore: {e}"))?;
    let anon = v
        .get("anonymized_text")
        .and_then(|t| t.as_str())
        .ok_or("Scudo PII: risposta senza testo anonimizzato")?
        .to_string();
    let mut map = BTreeMap::new();
    if let Some(m) = v.get("mapping").and_then(|m| m.as_object()) {
        for (k, val) in m {
            if let Some(s) = val.as_str() {
                map.insert(k.clone(), s.to_string());
            }
        }
    }
    Ok((anon, map))
}

/// Spezza "[FULLNAME_3]" in ("FULLNAME", 3).
fn split_placeholder(ph: &str) -> Option<(&str, u32)> {
    let inner = ph.strip_prefix('[')?.strip_suffix(']')?;
    let us = inner.rfind('_')?;
    let idx: u32 = inner[us + 1..].parse().ok()?;
    Some((&inner[..us], idx))
}

/// Il backend è stateless: ogni /analyze riparte da [TAG_1], che collide coi
/// messaggi precedenti della stessa chat. Rimappa i placeholder nuovi sul
/// dizionario di sessione: stesso valore → stesso placeholder di prima; valore
/// nuovo → primo indice libero per quel TAG. Ritorna (testo riscritto, voci da
/// aggiungere al dizionario di sessione).
pub fn renumber(
    anon_text: &str,
    new_map: &BTreeMap<String, String>,
    session_map: &BTreeMap<String, String>,
) -> (String, BTreeMap<String, String>) {
    // valore → placeholder già assegnato in sessione
    let by_value: BTreeMap<&str, &str> = session_map
        .iter()
        .map(|(ph, val)| (val.as_str(), ph.as_str()))
        .collect();
    // TAG → indice massimo già usato in sessione
    let mut max_idx: BTreeMap<String, u32> = BTreeMap::new();
    for ph in session_map.keys() {
        if let Some((tag, idx)) = split_placeholder(ph) {
            let e = max_idx.entry(tag.to_string()).or_insert(0);
            *e = (*e).max(idx);
        }
    }

    let mut text = anon_text.to_string();
    let mut additions = BTreeMap::new();
    for (ph, value) in new_map {
        let target = if let Some(existing) = by_value.get(value.as_str()) {
            existing.to_string()
        } else if session_map.contains_key(ph) && session_map.get(ph) != Some(value) {
            // placeholder già preso da un ALTRO valore → nuovo indice
            let Some((tag, _)) = split_placeholder(ph) else { continue };
            let next = max_idx.get(tag).copied().unwrap_or(0) + 1;
            max_idx.insert(tag.to_string(), next);
            format!("[{tag}_{next}]")
        } else {
            ph.clone()
        };
        if target != *ph {
            text = text.replace(ph.as_str(), &target);
        }
        additions.insert(target, value.clone());
    }
    (text, additions)
}

/// Rimette i valori veri al posto dei placeholder.
pub fn restore(text: &str, map: &BTreeMap<String, String>) -> String {
    let mut out = text.to_string();
    for (ph, value) in map {
        out = out.replace(ph.as_str(), value);
    }
    out
}

/// Rewriter per lo streaming: i placeholder possono arrivare spezzati su più
/// delta ("[CF" + "_1] ..."). Trattiene la coda dal primo '[' che può ancora
/// essere l'inizio di un placeholder, con un cap per non stallare su '[' veri.
pub struct StreamRestorer {
    buf: String,
}

const HOLDBACK_CAP: usize = 48;

/// true se `s` (che inizia con '[') può ancora diventare `[A-Z_]+_[0-9]+]`.
fn could_be_placeholder_prefix(s: &str) -> bool {
    if s.len() > HOLDBACK_CAP {
        return false;
    }
    for (i, c) in s.char_indices() {
        match (i, c) {
            (0, '[') => {}
            (0, _) => return false,
            (_, 'A'..='Z') | (_, '_') | (_, '0'..='9') => {}
            _ => return false,
        }
    }
    true
}

impl StreamRestorer {
    pub fn new() -> Self {
        Self { buf: String::new() }
    }

    /// Processa un delta: ritorna il testo emettibile (già ripristinato),
    /// trattenendo l'eventuale coda che può essere un placeholder a metà.
    pub fn push(&mut self, delta: &str, map: &BTreeMap<String, String>) -> String {
        self.buf.push_str(delta);
        let restored = restore(&self.buf, map);
        // Trova l'ultimo '[' la cui coda è ancora un prefisso plausibile.
        let hold_from = restored
            .rfind('[')
            .filter(|&i| could_be_placeholder_prefix(&restored[i..]))
            .unwrap_or(restored.len());
        let out = restored[..hold_from].to_string();
        self.buf = restored[hold_from..].to_string();
        out
    }

    /// Fine risposta: svuota tutto (ripristinando eventuali placeholder completi).
    pub fn flush(&mut self, map: &BTreeMap<String, String>) -> String {
        let out = restore(&self.buf, map);
        self.buf.clear();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn renumber_reuses_placeholder_for_same_value() {
        let session = map(&[("[FULLNAME_1]", "Mario Rossi")]);
        let new = map(&[("[FULLNAME_1]", "Mario Rossi")]);
        let (text, add) = renumber("Ciao [FULLNAME_1]", &new, &session);
        assert_eq!(text, "Ciao [FULLNAME_1]");
        assert_eq!(add.get("[FULLNAME_1]").unwrap(), "Mario Rossi");
    }

    #[test]
    fn renumber_bumps_index_on_collision() {
        let session = map(&[("[FULLNAME_1]", "Mario Rossi")]);
        let new = map(&[("[FULLNAME_1]", "Giulia Bianchi")]);
        let (text, add) = renumber("Scrivi a [FULLNAME_1]", &new, &session);
        assert_eq!(text, "Scrivi a [FULLNAME_2]");
        assert_eq!(add.get("[FULLNAME_2]").unwrap(), "Giulia Bianchi");
    }

    #[test]
    fn renumber_same_value_new_tag_reuses_old_placeholder() {
        // Il valore era già noto come [FULLNAME_1]: il nuovo [FULLNAME_1]
        // dell'analisi stateless viene ricondotto a quello.
        let session = map(&[("[FULLNAME_1]", "Mario Rossi"), ("[IBAN_1]", "IT60X...")]);
        let new = map(&[("[FULLNAME_1]", "Mario Rossi"), ("[IBAN_1]", "IT99Y...")]);
        let (text, add) = renumber("[FULLNAME_1] paga su [IBAN_1]", &new, &session);
        assert_eq!(text, "[FULLNAME_1] paga su [IBAN_2]");
        assert_eq!(add.len(), 2);
        assert_eq!(add.get("[IBAN_2]").unwrap(), "IT99Y...");
    }

    #[test]
    fn stream_restorer_handles_split_placeholder() {
        let m = map(&[("[CF_1]", "RSSMRA80A01H501U")]);
        let mut r = StreamRestorer::new();
        let mut out = String::new();
        out += &r.push("Il CF è [C", &m);
        out += &r.push("F_", &m);
        out += &r.push("1] come detto", &m);
        out += &r.flush(&m);
        assert_eq!(out, "Il CF è RSSMRA80A01H501U come detto");
    }

    #[test]
    fn stream_restorer_passes_literal_brackets() {
        let m = map(&[("[CF_1]", "X")]);
        let mut r = StreamRestorer::new();
        let mut out = String::new();
        out += &r.push("array[i] = 3; nota [vedi sotto]", &m);
        out += &r.flush(&m);
        assert_eq!(out, "array[i] = 3; nota [vedi sotto]");
    }

    #[test]
    fn stream_restorer_flush_restores_pending_complete_placeholder() {
        let m = map(&[("[IBAN_1]", "IT60X")]);
        let mut r = StreamRestorer::new();
        let a = r.push("Bonifico su [IBAN_1", &m);
        assert_eq!(a, "Bonifico su ");
        let b = r.flush(&m);
        // "]" mai arrivato: il flush ripristina quello che può
        assert_eq!(b, "[IBAN_1");
        let mut r2 = StreamRestorer::new();
        let c = r2.push("Su [IBAN_1]", &m);
        let d = r2.flush(&m);
        assert_eq!(format!("{c}{d}"), "Su IT60X");
    }

    #[test]
    fn split_placeholder_parses() {
        assert_eq!(split_placeholder("[FULLNAME_12]"), Some(("FULLNAME", 12)));
        assert_eq!(split_placeholder("[niente]"), None);
    }
}
