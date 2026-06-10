// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

/**
 * Fast-path deterministico per Moon Io — SOLO modelli locali (OllamaEngine).
 *
 * I modelli locali piccoli falliscono la SCELTA del tool ("leggi le ultime
 * email" → domande inutili o index_emails), non l'esecuzione. Questo matcher
 * intercetta le richieste email frequenti PRIMA del modello e le mappa
 * direttamente su (tool, args): risposta in ~2s, sempre giusta. Tutto ciò che
 * non matcha con confidenza ALTA cade al modello (return null).
 *
 * Filosofia anti-falso-positivo (un match deterministico sbagliato è peggio di
 * un LLM lento):
 *   • gate negativi PRIMA dei pattern (mutazione, anafora, filtri temporali,
 *     negazioni, altri canali) — un hit qualsiasi → null;
 *   • pattern ancorati ^…$ sull'intera stringa normalizzata: una clausola in
 *     più qualsiasi rompe l'ancora → null;
 *   • su "di/da X" MAI indovinare contatto-vs-argomento: o X valida come
 *     contatto (maiuscola/indirizzo email) o null.
 * Solo tool di LETTURA: il danno massimo di un match sbagliato è un elenco
 * sbagliato, mai un'azione.
 */

// ── Normalizzazione ──────────────────────────────────────────
// CASE-PRESERVING: l'euristica contatti distingue "email di Marco" (contatto)
// da "email di lavoro" (argomento → al modello) tramite la maiuscola.
export function normalizePrompt(s) {
  return String(s ?? "")
    .normalize("NFD").replace(/[̀-ͯ]/g, "") // via gli accenti (più → piu)
    .replace(/\s+/g, " ")
    .trim()
    .replace(/[?!.\s]+$/, "");
}

const NUM_WORDS = {
  un: 1, una: 1, due: 2, tre: 3, quattro: 4, cinque: 5,
  sei: 6, sette: 7, otto: 8, nove: 9, dieci: 10, quindici: 15, venti: 20,
};
const NUMW = "(\\d{1,3}|un|una|due|tre|quattro|cinque|sei|sette|otto|nove|dieci|quindici|venti)";
const EMAILN = "(?:e-?mail|mail)";

function parseNum(s, fallback) {
  if (!s) return fallback;
  const t = s.toLowerCase();
  if (/^\d+$/.test(t)) return parseInt(t, 10);
  return NUM_WORDS[t] ?? fallback;
}

const clamp = (n, lo, hi) => Math.min(Math.max(n, lo), hi);

// ── Gate negativi (un hit → al modello) ──────────────────────
const GATES = [
  // G1 mutazione: qualsiasi intento di scrittura/azione — MAI fast-path
  // (volutamente largo: blocca anche participi innocui tipo "email inviate da
  // X"; falso negativo accettabile, ci pensa il modello + denylist).
  /\b(rispond|invia|inoltr|manda|scriv|componi|bozz|cancell|elimin|archivia|segna|sposta|firma|allega)/i,
  // G2 anafora / riferimenti alla conversazione: serve la history, solo modello
  /^(e|ed|anche|pure|poi|invece|ok|ora|adesso|allora)\b/i,
  /\b(quell[aeio]|quest[aeio]|stess[aeio]|altr[aeio]|precedent\w*|di nuovo|come prima|ancora)\b/i,
  // G3 vincoli temporali/filtri che gli args dei tool non esprimono
  /\b(ieri|oggi|domani|stamattina|stasera|settimana|mese|anno|gennaio|febbraio|marzo|aprile|maggio|giugno|luglio|agosto|settembre|ottobre|novembre|dicembre|prima di|dopo|non lett\w*|da legger\w*|important\w*|urgent\w*|allegat\w*|calendario|evento|appuntament\w*)\b/i,
  // G4 negazioni e richieste composte
  /\bnon\b/i,
  /\b(e poi|e dopo|e anche)\b/i,
  // G5 altri canali → Moon Europa, non Io
  /\b(telegram|slack|teams|whatsapp|messagg\w*)\b/i,
];

// ── Validatore contatto per "di/da X" ────────────────────────
// X è un contatto sse: indirizzo email, oppure 1-2 token che iniziano per
// MAIUSCOLA (sulla stringa case-preserved). "email di lavoro" → null (mai
// indovinare tra search_by_contact e search_emails).
const CAP_BLACKLIST = new Set(["Lavoro", "Spam", "Posta", "Banca", "Email", "Mail", "Casella", "Inbox", "Archivio"]);

function asContact(raw) {
  const x = String(raw || "").trim();
  if (!x) return null;
  if (/^[\w.+-]+@[\w-]+\.[\w.-]{2,}$/.test(x)) return x;
  const toks = x.split(/\s+/);
  if (toks.length > 2) return null;
  if (!toks.every((t) => /^\p{Lu}[\p{L}'’.-]*$/u.test(t))) return null;
  if (toks.length === 1 && CAP_BLACKLIST.has(toks[0])) return null;
  return x;
}

// Contatto "sonda" per i minuscoli ("email di assoholding"): non possiamo
// distinguere a priori un mittente da un argomento ("di lavoro"), quindi NON
// indoviniamo — l'engine chiama il tool in silenzio e decide dai DATI: risultati
// → era un contatto, mostra; zero risultati → fallthrough al modello.
function asProbeContact(raw) {
  const x = String(raw || "").trim();
  if (!x || x.length > 40) return null;
  const toks = x.split(/\s+/);
  if (toks.length > 2) return null;
  if (!toks.every((t) => /^[\p{L}\p{N}'’.@_-]{2,}$/u.test(t))) return null;
  return x;
}

// Risposta "vuota" dei tool Io (read_recent_emails / search_by_contact):
// usata dall'engine per decidere il fallthrough della sonda.
export function isEmptyIoResult(text) {
  return /^\s*Nessuna email trovata/i.test(String(text || ""));
}

// I tool Io filtrano per contatto anche sull'OGGETTO ("per il tuo lavoro"
// matcha contact="lavoro"), quindi risultati ≠ contatto vero. La sonda è
// confermata solo se il contatto compare in una riga From:/To: dei risultati.
export function probeConfirmsContact(text, contact) {
  const c = String(contact || "").toLowerCase();
  if (!c) return false;
  return String(text || "")
    .split("\n")
    .some((l) => /^\s*(from|to):/i.test(l) && l.toLowerCase().includes(c));
}

// Esito del match di/da: contatto sicuro → diretto; plausibile → sonda; no → null.
function contactOrProbe(tail, mk) {
  const sure = asContact(tail);
  if (sure) return mk(sure, false);
  const probe = asProbeContact(tail);
  if (probe) return mk(probe, true);
  return null;
}

// ── Pattern positivi (in ordine di precedenza) ───────────────

// P1 — get_stats
const P1A = new RegExp(`^quant[ae] ${EMAILN} (ho|ci sono)( in (casella|inbox|archivio))?$`, "i");
const P1B = new RegExp(`^statistiche (della |sulla |delle )?(posta|casella|${EMAILN})$`, "i");

// P2 — read_recent_emails: "leggi(mi)/apri(mi) [il contenuto delle] ultime [N] email [di/da X]"
const P2 = new RegExp(
  `^(?:leggi(?:mi)?|apri(?:mi)?)\\s+(?:il contenuto (?:delle|di)\\s+)?(?:le\\s+)?(?:mie\\s+)?(?:ultime?\\s+)?(?:${NUMW}\\s+)?${EMAILN}(?:\\s+(?:ricevute|arrivate|recenti|piu recenti))?(?:\\s+(?:di|da)\\s+(.+))?$`,
  "i",
);
// P2S — forma singolare: "leggi l'ultima email", "leggi la mail di Marco"
const P2S = new RegExp(
  `^(?:leggi(?:mi)?|apri(?:mi)?)\\s+(?:l['’]\\s*ultima\\s+|l['’]\\s*|la\\s+)${EMAILN}(?:\\s+(?:di|da)\\s+(.+))?$`,
  "i",
);

// P3 — search_emails: marcatori d'argomento espliciti (su/riguardo/…) → sempre topic
const P3 = new RegExp(
  `^(?:cerca(?:mi)?|trova(?:mi)?|mostra(?:mi)?|dammi|elenca(?:mi)?)\\s+(?:tra le\\s+|nelle\\s+|le\\s+)?${EMAILN}\\s+(?:su|sul|sulla|sullo|sugli|sui|riguardo(?: a)?|riguardanti|che parlano di|a proposito di|in merito a|sul tema)\\s+(.{2,60})$`,
  "i",
);

// P4 — search_by_contact: "cerca/mostra/… le [ultime N] email di/da PERSONA" + forma nuda "email di PERSONA"
const P4A = new RegExp(
  `^(?:cerca(?:mi)?|trova(?:mi)?|mostra(?:mi)?|dammi|elenca(?:mi)?|fammi vedere|lista)\\s+(?:le\\s+)?(?:ultime\\s+)?(?:${NUMW}\\s+)?${EMAILN}\\s+(?:ricevute da|arrivate da|di|da|del|della)\\s+(.+)$`,
  "i",
);
const P4B = new RegExp(`^(?:le\\s+)?${EMAILN}\\s+(?:di|da)\\s+(.+)$`, "i");

// P5 — list_emails: "elenca/mostra/… le [mie] ultime [N] email" + forma nuda "ultime [N] email"
const P5A = new RegExp(
  `^(?:elenca(?:mi)?|mostra(?:mi)?|lista|dammi|fammi vedere|visualizza)\\s+(?:le\\s+)?(?:mie\\s+)?(?:ultime\\s+)?(?:${NUMW}\\s+)?${EMAILN}(?:\\s+(?:ricevute|recenti|in arrivo|in casella|nella casella))?$`,
  "i",
);
const P5B = new RegExp(`^(?:le\\s+)?ultime\\s+(?:${NUMW}\\s+)?${EMAILN}$`, "i");

// ── Matcher ──────────────────────────────────────────────────
/** @returns {{tool: string, args: object, header: string} | null} */
export function matchIoFastPath(prompt) {
  const original = String(prompt ?? "");
  // G4: più di un "?" = richiesta composta (contato sull'originale, la
  // normalizzazione toglie solo quelli in coda).
  if ((original.match(/\?/g) || []).length > 1) return null;

  const norm = normalizePrompt(original);
  if (!norm || norm.length > 80) return null; // G0
  for (const g of GATES) {
    if (g.test(norm)) return null;
  }

  let m;

  if (P1A.test(norm) || P1B.test(norm)) {
    return { tool: "get_stats", args: {}, header: "Statistiche della casella:" };
  }

  if ((m = norm.match(P2))) {
    const limit = clamp(parseNum(m[1], 3), 1, 5);
    const tail = m[2];
    if (tail !== undefined) {
      // di/da presente: contatto sicuro → diretto; plausibile → sonda; no → modello
      return contactOrProbe(tail, (contact, probe) => ({
        tool: "read_recent_emails",
        args: { limit, contact },
        header: `Contenuto delle ultime ${limit} email di ${contact}:`,
        probe,
      }));
    }
    return { tool: "read_recent_emails", args: { limit }, header: `Contenuto delle ultime ${limit} email:` };
  }

  if ((m = norm.match(P2S))) {
    const tail = m[1];
    if (tail !== undefined) {
      return contactOrProbe(tail, (contact, probe) => ({
        tool: "read_recent_emails",
        args: { limit: 1, contact },
        header: `L'ultima email di ${contact}:`,
        probe,
      }));
    }
    return { tool: "read_recent_emails", args: { limit: 1 }, header: "L'ultima email:" };
  }

  if ((m = norm.match(P3))) {
    const query = m[1].trim();
    return { tool: "search_emails", args: { query, limit: 10 }, header: `Email trovate su "${query}":` };
  }

  if ((m = norm.match(P4A))) {
    const limit = clamp(parseNum(m[1], 20), 1, 50);
    return contactOrProbe(m[2], (contact, probe) => ({
      tool: "search_by_contact",
      args: { contact, limit },
      header: `Email di ${contact}:`,
      probe,
    }));
  }
  if ((m = norm.match(P4B))) {
    return contactOrProbe(m[1], (contact, probe) => ({
      tool: "search_by_contact",
      args: { contact, limit: 20 },
      header: `Email di ${contact}:`,
      probe,
    }));
  }

  if ((m = norm.match(P5A)) || (m = norm.match(P5B))) {
    const limit = clamp(parseNum(m[1], 10), 1, 30); // cap 30: budget contesto locale
    return { tool: "list_emails", args: { limit }, header: `Le ultime ${limit} email:` };
  }

  return null;
}
