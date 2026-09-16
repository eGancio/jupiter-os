// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

//! Guardrail costi + cache per le chiamate a pagamento di openapi.it.
//!
//! Perché esiste: ogni bilancio costa ~4,50€. Senza freno, un utente curioso che
//! clicca 50 aziende brucia 225€. Due meccanismi, persistiti su disco (JSON):
//!   - **tetto di spesa mensile**: oltre soglia, le chiamate vengono rifiutate;
//!   - **cache documenti**: stesso (tipo, P.IVA, anno) già scaricato = riuso a €0.
//!
//! La cache è anche una leva di margine quando la feature andrà in prodotto.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Ledger {
    /// Mese corrente in formato "YYYY-MM"; cambiando mese la spesa si azzera.
    month: String,
    /// Spesa accumulata nel mese (€, IVA esclusa, stimata).
    spent_eur: f64,
    /// key = "tipo|cf_piva|anno" → directory dei file scaricati.
    cache: BTreeMap<String, String>,
    #[serde(skip)]
    path: PathBuf,
}

impl Ledger {
    pub fn load(path: PathBuf, now_month: &str) -> Self {
        let mut l: Ledger = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        l.path = path;
        l.roll_month(now_month);
        l
    }

    /// Azzera la spesa se è cambiato il mese (la cache resta valida).
    fn roll_month(&mut self, now_month: &str) {
        if self.month != now_month {
            self.month = now_month.to_string();
            self.spent_eur = 0.0;
        }
    }

    pub fn key(tipo: &str, cf_piva: &str, anno: Option<i32>) -> String {
        format!("{tipo}|{cf_piva}|{}", anno.map(|a| a.to_string()).unwrap_or_default())
    }

    pub fn cached(&self, key: &str) -> Option<String> {
        self.cache.get(key).cloned()
    }

    pub fn month_spent(&self) -> f64 {
        self.spent_eur
    }

    pub fn month(&self) -> &str {
        &self.month
    }

    /// True se aggiungere `eur` resta entro il tetto `max`.
    pub fn can_spend(&self, eur: f64, max: f64) -> bool {
        self.spent_eur + eur <= max + f64::EPSILON
    }

    /// Registra una spesa e persiste.
    pub fn record(&mut self, eur: f64) {
        self.spent_eur += eur;
        self.save();
    }

    /// Memorizza un documento in cache e persiste.
    pub fn store(&mut self, key: &str, dir: &str) {
        self.cache.insert(key.to_string(), dir.to_string());
        self.save();
    }

    fn save(&self) {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(&self.path, s);
        }
    }
}

/// Mese corrente "YYYY-MM" dall'orologio locale.
pub fn current_month() -> String {
    chrono::Local::now().format("%Y-%m").to_string()
}

/// Helper: ricava il path della directory di cache da uno SavedFile salvato.
pub fn dir_of(path: &str) -> String {
    Path::new(path)
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}
