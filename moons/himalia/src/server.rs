// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

//! The MCP surface of Moon Himalia — web research via Tavily.
//!
//! Two tools, mapped 1:1 onto the deep-research skill:
//! - `web_search`  → Step 2 (fan-out discovery): clean results, not raw HTML.
//! - `web_extract` → Step 3 (VERIFY): a page's full clean text to cross-check.
//!
//! Citing is the model's job (the skill): every web claim carries its URL.

use std::collections::BTreeMap;
use std::future::Future;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use rmcp::handler::server::tool::{Parameters, ToolRouter};
use rmcp::model::*;
use rmcp::{tool, tool_handler, tool_router, ServerHandler};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::config::Config;
use crate::ledger::{self, Ledger};
use crate::openapi::{self, DocumentResult, OpenApiClient, SavedFile};
use crate::tavily::TavilyClient;

// ===========================================================================
// Server struct
// ===========================================================================

/// Moon Himalia MCP Server — web search + extraction for LLM research,
/// più (se configurato OPENAPI_TOKEN) i dati societari ufficiali via openapi.it.
pub struct HimaliaServer {
    config: Config,
    tavily: TavilyClient,
    openapi: OpenApiClient,
    ledger: Mutex<Ledger>,
    tool_router: ToolRouter<Self>,
}

// ===========================================================================
// Parameter structs — doc comments become JSON schema descriptions for the LLM
// ===========================================================================

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct WebSearchParams {
    /// The search query. Be specific; vary phrasing across sub-questions.
    query: String,
    /// Max results to return (default 5, clamped 1..=10).
    #[serde(default)]
    max_results: Option<u32>,
    /// Depth: "basic" (fast, default) or "advanced" (deeper crawl, slower).
    #[serde(default)]
    depth: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct WebExtractParams {
    /// The exact URL of the page to fetch as clean full text (for verifying a
    /// claim against the primary source).
    url: String,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct CercaImpresaParams {
    /// Denominazione / ragione sociale dell'impresa da cercare (es. "EMOTION SRL").
    denominazione: String,
    /// Sigla provincia (es. "MI", "RM") per restringere. Opzionale.
    #[serde(default)]
    provincia: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct BilancioParams {
    /// Codice fiscale o partita IVA dell'impresa (campo `cf_piva_id` di openapi.it).
    cf_piva: String,
    /// Anno di chiusura del bilancio (opzionale; default = ultimo depositato).
    #[serde(default)]
    anno: Option<i32>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct VisuraParams {
    /// Codice fiscale o partita IVA dell'impresa.
    cf_piva: String,
    /// Tipo di visura: "ordinaria-societa-capitale" (default, per SRL/SpA),
    /// "storica-societa-capitale", "ordinaria-societa-persone", ecc.
    #[serde(default)]
    tipo: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct DatiImpresaParams {
    /// Codice fiscale o partita IVA dell'impresa (società di capitali: SRL/SpA).
    cf_piva: String,
}

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct StatoRichiestaParams {
    /// L'id restituito da una richiesta precedente rimasta in lavorazione.
    id: String,
    /// Il tipo documento usato nella richiesta (es. "bilancio-ottico").
    tipo: String,
}

// ===========================================================================
// Tools
// ===========================================================================

#[tool_router]
impl HimaliaServer {
    pub fn new(config: Config) -> Self {
        let tavily = TavilyClient::new(config.tavily_api_key.clone());
        let openapi = OpenApiClient::new(
            config.openapi_token.clone(),
            config.openapi_company_token.clone(),
            config.openapi_sandbox,
            config.imprese_output_dir.clone(),
        );
        let ledger = Mutex::new(Ledger::load(
            config.data_dir.join("imprese_ledger.json"),
            &ledger::current_month(),
        ));
        Self {
            config,
            tavily,
            openapi,
            ledger,
            tool_router: Self::tool_router(),
        }
    }

    /// Guard: a clear, non-panicking error when no key is configured, so the
    /// skill can fall back to native WebSearch/WebFetch.
    fn require_key(&self) -> Result<(), String> {
        if self.config.has_api_key() {
            Ok(())
        } else {
            Err("Tavily non configurato: imposta TAVILY_API_KEY nel .mcp.json del Moon \
                 himalia. Nel frattempo usa WebSearch/WebFetch nativi."
                .to_string())
        }
    }

    /// Guard documenti camerali (bilancio/visura): richiede OPENAPI_TOKEN.
    fn require_openapi(&self) -> Result<(), String> {
        if self.config.has_openapi_key() {
            Ok(())
        } else {
            Err("openapi.it non configurato: imposta OPENAPI_TOKEN nel .mcp.json del Moon \
                 himalia per abilitare bilanci e visure ufficiali di SRL/SpA. Senza token \
                 procedo solo con la ricerca web."
                .to_string())
        }
    }

    /// Guard dati strutturati (dati_impresa/cerca_impresa): richiede il token
    /// company (prodotto company.openapi.com), distinto da quello delle visure.
    fn require_openapi_company(&self) -> Result<(), String> {
        if self.config.has_openapi_company_key() {
            Ok(())
        } else {
            Err("company.openapi.com non configurato: imposta OPENAPI_COMPANY_TOKEN nel \
                 .mcp.json del Moon himalia per abilitare i dati economici (fatturato, soci). \
                 Senza token procedo solo con la ricerca web."
                .to_string())
        }
    }

    /// Cuore del flusso imprese: cache → guardrail spesa → POST → polling →
    /// download → parse XBRL. Non tiene mai il lock del ledger across `await`.
    async fn procura_documento(
        &self,
        tipo: &str,
        cf_piva: &str,
        anno: Option<i32>,
    ) -> Result<DocumentResult, String> {
        if !openapi::tipo_valido(tipo) {
            return Err(format!(
                "Tipo documento non valido: '{tipo}'. Usa es. 'bilancio-ottico' o \
                 'ordinaria-societa-capitale'."
            ));
        }
        let key = Ledger::key(tipo, cf_piva, anno);

        // 1) cache hit → €0, nessuna chiamata di rete
        let cached_dir = { self.ledger.lock().unwrap().cached(&key) };
        if let Some(dir) = cached_dir {
            if Path::new(&dir).is_dir() {
                let files = scan_dir(&dir);
                if !files.is_empty() {
                    return Ok(build_result(
                        tipo,
                        cf_piva,
                        "(cache)".into(),
                        files,
                        "Documento già scaricato in precedenza: riuso dalla cache, nessun \
                         costo e nessuna nuova chiamata."
                            .into(),
                    ));
                }
            }
        }

        // 2) guardrail spesa (in sandbox è gratis)
        let cost = if self.config.openapi_sandbox {
            0.0
        } else {
            openapi::costo_stimato_eur(tipo)
        };
        {
            let l = self.ledger.lock().unwrap();
            if !l.can_spend(cost, self.config.imprese_max_spend_eur) {
                return Err(format!(
                    "Tetto di spesa mensile raggiunto: spesi {:.2}€ su {:.2}€ e questa \
                     chiamata costa ~{:.2}€. Alza IMPRESE_MAX_SPEND_EUR nel .mcp.json o \
                     attendi il mese prossimo. (usa costo_residuo per i dettagli)",
                    l.month_spent(),
                    self.config.imprese_max_spend_eur,
                    cost
                ));
            }
        }

        // 3) rete: POST → polling → download (nessun lock tenuto)
        let id = self.openapi.richiedi(tipo, cf_piva, anno).await?;
        self.openapi
            .attendi(tipo, &id, Duration::from_secs(90))
            .await?;
        let files = self.openapi.scarica_allegati(tipo, &id, cf_piva).await?;

        // 4) registra spesa + memorizza cache
        let dir = files
            .first()
            .map(|f| ledger::dir_of(&f.path))
            .unwrap_or_default();
        {
            let mut l = self.ledger.lock().unwrap();
            if cost > 0.0 {
                l.record(cost);
            }
            if !dir.is_empty() {
                l.store(&key, &dir);
            }
        }

        let nota = if self.config.openapi_sandbox {
            "Modalità sandbox: nessun addebito.".into()
        } else {
            format!("Addebito stimato: ~{cost:.2}€ (IVA escl.).")
        };
        Ok(build_result(tipo, cf_piva, id, files, nota))
    }

    /// Web search returning clean, LLM-ready results (not raw HTML snippets).
    #[tool(
        name = "web_search",
        description = "Ricerca sul web con risultati PULITI per LLM (titolo, url, contenuto sintetico, score) via Tavily. Usala per la fase di scoperta della deep research (fan-out su sotto-domande). Cita SEMPRE l'URL nelle affermazioni. Per verificare un dato sulla fonte primaria usa poi web_extract."
    )]
    pub async fn web_search(
        &self,
        params: Parameters<WebSearchParams>,
    ) -> Result<String, String> {
        self.require_key()?;
        let p = params.0;
        let max_results = p.max_results.unwrap_or(5).clamp(1, 10) as usize;
        let depth = match p.depth.as_deref() {
            Some("advanced") => "advanced",
            _ => "basic",
        };

        let hits = self.tavily.search(&p.query, max_results, depth).await?;

        if hits.is_empty() {
            return Ok(json!({
                "query": p.query,
                "results": [],
                "note": "Nessun risultato. Riprova con altra formulazione o astieniti.",
            })
            .to_string());
        }

        Ok(json!({
            "query": p.query,
            "results": hits,
            "note": "Cita l'URL per ogni affermazione. Per il claim chiave verifica con web_extract sulla fonte più autorevole.",
        })
        .to_string())
    }

    /// Fetch a single URL's full clean text — the VERIFY step.
    #[tool(
        name = "web_extract",
        description = "Estrae il TESTO PULITO integrale di una pagina dato il suo URL (via Tavily). Usala nello step VERIFY della deep research per controllare un'affermazione sulla fonte primaria invece che sullo snippet. Cita l'URL."
    )]
    pub async fn web_extract(
        &self,
        params: Parameters<WebExtractParams>,
    ) -> Result<String, String> {
        self.require_key()?;
        let p = params.0;
        let (url, content) = self.tavily.extract(&p.url).await?;

        if content.trim().is_empty() {
            return Ok(json!({
                "url": url,
                "content": "",
                "note": "Pagina non estraibile (paywall/JS/blocco). Prova un'altra fonte o WebFetch nativo.",
            })
            .to_string());
        }

        Ok(json!({
            "url": url,
            "content": content,
            "note": "Testo della fonte primaria: verifica qui il dato prima di citarlo.",
        })
        .to_string())
    }

    // =======================================================================
    // Tool IMPRESE — dati societari ufficiali via openapi.it (gated da OPENAPI_TOKEN)
    // =======================================================================

    /// Risolve una ragione sociale nella sua P.IVA / codice fiscale.
    #[tool(
        name = "cerca_impresa",
        description = "Trova la partita IVA / codice fiscale di un'impresa italiana data la ragione sociale (es. per poi chiamare bilancio_impresa o visura_impresa). Richiede OPENAPI_COMPANY_TOKEN. Se hai già la P.IVA, salta questo passo."
    )]
    pub async fn cerca_impresa(
        &self,
        params: Parameters<CercaImpresaParams>,
    ) -> Result<String, String> {
        self.require_openapi_company()?;
        let p = params.0;
        let v = self
            .openapi
            .cerca_impresa(&p.denominazione, p.provincia.as_deref())
            .await?;
        Ok(json!({
            "query": p.denominazione,
            "risultati": v,
            "note": "Prendi il cf_piva_id del match giusto e usalo con bilancio_impresa o visura_impresa.",
        })
        .to_string())
    }

    /// Dato economico sintetico a basso costo: fatturato + bilancio chiave + soci.
    #[tool(
        name = "dati_impresa",
        description = "Dato economico SINTETICO e a basso costo (~0,03€) di una società di capitali italiana (SRL/SpA) data la P.IVA/CF: fatturato (turnover), totale attivo, patrimonio netto, costo del personale, n. dipendenti, capitale sociale, assetto proprietario (soci con quote) e storico pluriennale. È lo strumento giusto per SCREENARE molte aziende (scouting/market research) senza pagare il bilancio completo. NB: NON contiene l'utile netto, quindi il margine netto NON è calcolabile da qui (serve il bilancio). Richiede OPENAPI_COMPANY_TOKEN. Disponibile solo per società di capitali (chi deposita il bilancio)."
    )]
    pub async fn dati_impresa(
        &self,
        params: Parameters<DatiImpresaParams>,
    ) -> Result<String, String> {
        self.require_openapi_company()?;
        let cf = params.0.cf_piva.trim().to_string();
        let key = Ledger::key("dati-advanced", &cf, None);

        // cache (JSON salvato): riuso a costo zero entro il mese
        let cached = { self.ledger.lock().unwrap().cached(&key) };
        if let Some(p) = cached {
            if let Ok(s) = std::fs::read_to_string(&p) {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) {
                    return Ok(formatta_dati(&v, "Dato dalla cache: nessun costo."));
                }
            }
        }

        // guardrail spesa
        let cost = if self.config.openapi_sandbox {
            0.0
        } else {
            COSTO_DATI_IMPRESA
        };
        {
            let l = self.ledger.lock().unwrap();
            if !l.can_spend(cost, self.config.imprese_max_spend_eur) {
                return Err(format!(
                    "Tetto di spesa mensile raggiunto: spesi {:.2}€ su {:.2}€. \
                     Alza IMPRESE_MAX_SPEND_EUR o attendi il mese prossimo.",
                    l.month_spent(),
                    self.config.imprese_max_spend_eur
                ));
            }
        }

        let v = self.openapi.dati_impresa(&cf).await?;

        // salva JSON + registra spesa + cache
        std::fs::create_dir_all(&self.config.imprese_output_dir).ok();
        let p = self.config.imprese_output_dir.join(format!("{cf}_advanced.json"));
        std::fs::write(&p, serde_json::to_string_pretty(&v).unwrap_or_default()).ok();
        {
            let mut l = self.ledger.lock().unwrap();
            if cost > 0.0 {
                l.record(cost);
            }
            l.store(&key, &p.to_string_lossy());
        }

        let nota = if self.config.openapi_sandbox {
            "Modalità sandbox: nessun addebito.".to_string()
        } else {
            format!("Addebito stimato: ~{cost:.3}€.")
        };
        Ok(formatta_dati(&v, &nota))
    }

    /// Scarica il bilancio ufficiale depositato e ne estrae i numeri chiave.
    #[tool(
        name = "bilancio_impresa",
        description = "Recupera il BILANCIO ufficiale depositato di una SRL/SpA italiana (documento camerale) data la P.IVA/CF: scarica PDF + XBRL e ne estrae i numeri chiave (valore della produzione, EBIT, utile, totale attivo, patrimonio netto, debiti). Usalo nella deep research per analisi economico-finanziaria, due diligence o scouting M&A su dati REALI e citabili. Restituisce anche pdf_path: passalo a metis_ingest per citare il documento. A pagamento (~4,50€/chiamata, con tetto di spesa e cache)."
    )]
    pub async fn bilancio_impresa(
        &self,
        params: Parameters<BilancioParams>,
    ) -> Result<String, String> {
        self.require_openapi()?;
        let p = params.0;
        let res = self
            .procura_documento("bilancio-ottico", p.cf_piva.trim(), p.anno)
            .await?;
        Ok(serde_json::to_string(&res).unwrap_or_else(|e| format!("{{\"errore\":\"{e}\"}}")))
    }

    /// Scarica una visura camerale (società di capitale per default).
    #[tool(
        name = "visura_impresa",
        description = "Recupera la VISURA camerale ufficiale di un'impresa italiana data la P.IVA/CF (default: ordinaria società di capitale, per SRL/SpA). Dà sede, soci, amministratori, oggetto sociale, stato attività — base per identità societaria e struttura proprietaria in due diligence/scouting. Restituisce pdf_path per metis_ingest. A pagamento, con tetto di spesa e cache."
    )]
    pub async fn visura_impresa(
        &self,
        params: Parameters<VisuraParams>,
    ) -> Result<String, String> {
        self.require_openapi()?;
        let p = params.0;
        let tipo = p
            .tipo
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .unwrap_or("ordinaria-societa-capitale");
        let res = self.procura_documento(tipo, p.cf_piva.trim(), None).await?;
        Ok(serde_json::to_string(&res).unwrap_or_else(|e| format!("{{\"errore\":\"{e}\"}}")))
    }

    /// Ricontrolla una richiesta rimasta in lavorazione e scarica se pronta.
    #[tool(
        name = "stato_richiesta",
        description = "Ricontrolla una richiesta openapi.it rimasta 'in lavorazione' (quando bilancio_impresa/visura_impresa è andato in timeout) e, se è ora evasa, scarica gli allegati. Passa id e tipo della richiesta originale."
    )]
    pub async fn stato_richiesta(
        &self,
        params: Parameters<StatoRichiestaParams>,
    ) -> Result<String, String> {
        self.require_openapi()?;
        let p = params.0;
        let tipo = p.tipo.trim();
        if !openapi::tipo_valido(tipo) {
            return Err(format!("Tipo documento non valido: '{tipo}'."));
        }
        let (stato, _) = self.openapi.stato(tipo, p.id.trim()).await?;
        if !crate::openapi::stato_pronto(&stato) {
            return Ok(json!({
                "id": p.id, "tipo": tipo, "stato": stato,
                "note": "Ancora in lavorazione. Riprova tra poco.",
            })
            .to_string());
        }
        let files = self
            .openapi
            .scarica_allegati(tipo, p.id.trim(), "richiesta")
            .await?;
        let res = build_result(tipo, "richiesta", p.id.clone(), files, "Richiesta evasa e scaricata.".into());
        Ok(serde_json::to_string(&res).unwrap_or_else(|e| format!("{{\"errore\":\"{e}\"}}")))
    }

    /// Mostra la spesa del mese rispetto al tetto configurato.
    #[tool(
        name = "costo_residuo",
        description = "Mostra quanto è stato speso questo mese in chiamate a pagamento openapi.it (bilanci/visure) e quanto resta prima del tetto IMPRESE_MAX_SPEND_EUR. Usalo prima di lanciare molte richieste."
    )]
    pub async fn costo_residuo(&self) -> Result<String, String> {
        self.require_openapi()?;
        let l = self.ledger.lock().unwrap();
        let max = self.config.imprese_max_spend_eur;
        let spent = l.month_spent();
        Ok(json!({
            "mese": l.month(),
            "speso_eur": spent,
            "tetto_eur": max,
            "residuo_eur": (max - spent).max(0.0),
            "sandbox": self.config.openapi_sandbox,
        })
        .to_string())
    }
}

// ===========================================================================
// Helper imprese (funzioni libere → niente vincoli di borrow su &self)
// ===========================================================================

/// Costo stimato di una call /IT-advanced (€). Solo per il guardrail di spesa.
const COSTO_DATI_IMPRESA: f64 = 0.028;

/// Estrae i campi utili dal record /IT-advanced in un JSON pulito e leggibile.
fn formatta_dati(rec: &serde_json::Value, nota: &str) -> String {
    let s = |ptr: &str| rec.pointer(ptr).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let n = |ptr: &str| rec.pointer(ptr).and_then(|v| v.as_f64());

    let fatturato = n("/balanceSheets/last/turnover");
    let costo_personale = n("/balanceSheets/last/totalStaffCost");
    let incidenza = match (costo_personale, fatturato) {
        (Some(c), Some(f)) if f > 0.0 => Some((c / f * 1000.0).round() / 10.0),
        _ => None,
    };

    // storico sintetico (anno → fatturato, dipendenti)
    let storico: Vec<serde_json::Value> = rec
        .pointer("/balanceSheets/all")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .map(|y| {
                    json!({
                        "anno": y.get("year").and_then(|x| x.as_i64()),
                        "fatturato": y.get("turnover").and_then(|x| x.as_f64()),
                        "dipendenti": y.get("employees").and_then(|x| x.as_i64()),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    // soci con quota
    let soci: Vec<serde_json::Value> = rec
        .pointer("/shareHolders")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .map(|s| {
                    let nome = s
                        .get("companyName")
                        .and_then(|x| x.as_str())
                        .filter(|x| !x.is_empty())
                        .map(String::from)
                        .unwrap_or_else(|| {
                            format!(
                                "{} {}",
                                s.get("name").and_then(|x| x.as_str()).unwrap_or(""),
                                s.get("surname").and_then(|x| x.as_str()).unwrap_or("")
                            )
                            .trim()
                            .to_string()
                        });
                    json!({"socio": nome, "quota_pct": s.get("percentShare").and_then(|x| x.as_f64())})
                })
                .collect()
        })
        .unwrap_or_default();

    json!({
        "azienda": {
            "denominazione": s("/companyName"),
            "cf_piva": s("/vatCode"),
            "forma_giuridica": s("/detailedLegalForm/description"),
            "stato": s("/activityStatus"),
            "comune": s("/address/registeredOffice/town"),
            "provincia": s("/address/registeredOffice/province"),
            "ateco": s("/atecoClassification/ateco2022/description"),
        },
        "bilancio_ultimo": {
            "anno": rec.pointer("/balanceSheets/last/year").and_then(|v| v.as_i64()),
            "fatturato": fatturato,
            "totale_attivo": n("/balanceSheets/last/totalAssets"),
            "patrimonio_netto": n("/balanceSheets/last/netWorth"),
            "costo_personale": costo_personale,
            "incidenza_costo_personale_pct": incidenza,
            "dipendenti": rec.pointer("/balanceSheets/last/employees").and_then(|v| v.as_i64()),
            "capitale_sociale": n("/balanceSheets/last/shareCapital"),
        },
        "storico": storico,
        "soci": soci,
        "note": format!(
            "{nota} ATTENZIONE: utile netto e margine netto NON sono in questo dato \
             (serve il bilancio o /IT-full). Per le spese dettagliate serve il bilancio."
        ),
    })
    .to_string()
}

/// Costruisce il risultato: trova il PDF e parsa l'XBRL se presente.
fn build_result(
    tipo: &str,
    cf_piva: &str,
    id: String,
    files: Vec<SavedFile>,
    base_note: String,
) -> DocumentResult {
    let pdf_path = files.iter().find(|f| f.kind == "pdf").map(|f| f.path.clone());
    let mut bilancio = BTreeMap::new();
    if let Some(x) = files.iter().find(|f| f.kind == "xbrl") {
        if let Ok(xml) = std::fs::read_to_string(&x.path) {
            bilancio = openapi::parse_xbrl(&xml);
        }
    }
    let mut note = base_note;
    if bilancio.is_empty() {
        note.push_str(
            " Numeri non estratti automaticamente dall'XBRL: usa il PDF (pdf_path → metis_ingest \
             per renderlo citabile, poi cita con metis_search).",
        );
    } else {
        note.push_str(" pdf_path → metis_ingest per citare il documento nel knowledge layer.");
    }
    DocumentResult {
        id,
        tipo: tipo.to_string(),
        cf_piva: cf_piva.to_string(),
        files,
        pdf_path,
        bilancio,
        note,
    }
}

/// Ricostruisce la lista file da una directory di cache (kind dall'estensione).
fn scan_dir(dir: &str) -> Vec<SavedFile> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for e in entries.flatten() {
        let path = e.path();
        if !path.is_file() {
            continue;
        }
        let lower = path
            .extension()
            .and_then(|x| x.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let kind = match lower.as_str() {
            "pdf" => "pdf",
            "xbrl" | "xml" => "xbrl",
            _ => "other",
        };
        out.push(SavedFile {
            path: path.to_string_lossy().into_owned(),
            kind: kind.to_string(),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn config_senza_chiavi() -> Config {
        Config {
            tavily_api_key: String::new(),
            openapi_token: String::new(),
            openapi_company_token: String::new(),
            openapi_sandbox: false,
            imprese_output_dir: PathBuf::from("/tmp/himalia-test/imprese"),
            imprese_max_spend_eur: 50.0,
            data_dir: PathBuf::from("/tmp/himalia-test"),
            mcp_transport: "stdio".into(),
            mcp_host: "127.0.0.1".into(),
            mcp_port: 8700,
        }
    }

    // Il formatter estrae i campi giusti dal record reale /IT-advanced e calcola
    // l'incidenza del costo del personale; soci e storico vengono normalizzati.
    #[test]
    fn formatta_dati_estrae_fatturato_e_soci() {
        let rec = serde_json::json!({
            "companyName": "OPENAPI S.P.A.",
            "vatCode": "12485671007",
            "detailedLegalForm": {"description": "SOCIETA' PER AZIONI"},
            "activityStatus": "ATTIVA",
            "address": {"registeredOffice": {"town": "ROMA", "province": "RM"}},
            "atecoClassification": {"ateco2022": {"description": "Produzione di software"}},
            "balanceSheets": {
                "last": {"year": 2024, "turnover": 3671995.0, "totalAssets": 1118403.0,
                         "netWorth": 338315.0, "totalStaffCost": 617428.0,
                         "employees": 15, "shareCapital": 50000.0},
                "all": [{"year": 2023, "turnover": 3000000.0, "employees": 14}]
            },
            "shareHolders": [{"companyName": "OPEN HOLDING S.R.L.", "percentShare": 100.0}]
        });
        let out = formatta_dati(&rec, "test");
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["bilancio_ultimo"]["fatturato"], 3671995.0);
        // incidenza costo personale = 617428/3671995 ≈ 16,8%
        assert_eq!(v["bilancio_ultimo"]["incidenza_costo_personale_pct"], 16.8);
        assert_eq!(v["soci"][0]["socio"], "OPEN HOLDING S.R.L.");
        assert_eq!(v["soci"][0]["quota_pct"], 100.0);
        assert_eq!(v["storico"][0]["anno"], 2023);
        // utile/margine netto NON deve essere promesso
        assert!(v["note"].as_str().unwrap().contains("margine netto NON"));
    }

    // Cuore della richiesta utente: senza OPENAPI_TOKEN i tool imprese NON
    // operano — tornano un errore chiaro (nessuna chiamata di rete, nessun costo).
    #[tokio::test]
    async fn imprese_disabilitati_senza_token() {
        let srv = HimaliaServer::new(config_senza_chiavi());
        let err = srv
            .bilancio_impresa(Parameters(BilancioParams {
                cf_piva: "12485671007".into(),
                anno: None,
            }))
            .await
            .expect_err("senza token deve fallire");
        assert!(err.contains("OPENAPI_TOKEN"), "messaggio inatteso: {err}");
    }
}

// ===========================================================================
// ServerHandler implementation — MCP protocol
// ===========================================================================

#[tool_handler]
impl ServerHandler for HimaliaServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::default(),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation {
                name: "moon-himalia".to_string(),
                version: "0.1.0".to_string(),
            },
            instructions: Some(
                "Moon Himalia — ricerca web per LLM (Tavily) + dati societari ufficiali \
                 (openapi.it). WEB: web_search per scoprire fonti (fan-out della deep research), \
                 web_extract per leggere la fonte primaria e verificare un'affermazione; cita \
                 SEMPRE l'URL. IMPRESE: per SCREENARE molte aziende (scouting/market research) \
                 usa dati_impresa (~0,03€: fatturato, attivo, patrimonio netto, costo personale, \
                 dipendenti, soci, storico) — NON dà l'utile/margine netto. Per il dato ufficiale \
                 e citabile usa bilancio_impresa e visura_impresa (documenti camerali di SRL/SpA, \
                 PDF + numeri da XBRL, ~4,50€) solo sulle aziende che approfondisci. cerca_impresa \
                 risolve la ragione sociale in P.IVA; costo_residuo mostra la spesa vs il tetto; \
                 stato_richiesta riprende una richiesta in timeout. Il pdf_path va passato a \
                 metis_ingest per renderlo citabile. dati_impresa/cerca_impresa richiedono \
                 OPENAPI_COMPANY_TOKEN; bilancio/visura richiedono OPENAPI_TOKEN. Senza le \
                 rispettive chiavi i tool tornano un errore chiaro e si prosegue col web."
                    .into(),
            ),
        }
    }
}
