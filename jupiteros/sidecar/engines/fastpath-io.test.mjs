// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli
//
// Unit test del matcher fast-path Moon Io.
//   node --test engines/fastpath-io.test.mjs
//
// Regola d'oro: i NEGATIVI sono i test che contano — un falso positivo
// (match deterministico sbagliato) è peggio di un LLM lento.

import { test } from "node:test";
import assert from "node:assert/strict";
import { matchIoFastPath, normalizePrompt, probeConfirmsContact } from "./fastpath-io.mjs";

function expectMatch(prompt, tool, args, probe = false) {
  const r = matchIoFastPath(prompt);
  assert.ok(r, `"${prompt}" doveva matchare ${tool}, è andato al modello`);
  assert.equal(r.tool, tool, `"${prompt}" → ${r.tool}, atteso ${tool}`);
  if (args) assert.deepEqual(r.args, args, `"${prompt}" args errati`);
  assert.equal(!!r.probe, probe, `"${prompt}" probe=${!!r.probe}, atteso ${probe}`);
}

function expectNull(prompt) {
  const r = matchIoFastPath(prompt);
  assert.equal(r, null, `"${prompt}" doveva andare al modello, ha matchato ${r?.tool} ${JSON.stringify(r?.args)}`);
}

test("normalizePrompt: accenti, spazi, punteggiatura finale", () => {
  assert.equal(normalizePrompt("  più   recenti?! "), "piu recenti");
  assert.equal(normalizePrompt("Leggi le EMAIL."), "Leggi le EMAIL");
});

// ── read_recent_emails ──
test("leggi: ultime N email", () => {
  expectMatch("Leggimi le ultime 3 email", "read_recent_emails", { limit: 3 });
  expectMatch("leggi il contenuto delle ultime cinque mail", "read_recent_emails", { limit: 5 });
  expectMatch("leggi le ultime 8 email", "read_recent_emails", { limit: 5 }); // clamp a 5
  expectMatch("leggi le email", "read_recent_emails", { limit: 3 }); // default
  expectMatch("leggimi le ultime tre e-mail piu recenti", "read_recent_emails", { limit: 3 });
  expectMatch("Leggi le ultime email di Marco", "read_recent_emails", { limit: 3, contact: "Marco" });
});

test("leggi: forma singolare", () => {
  expectMatch("leggi l'ultima email", "read_recent_emails", { limit: 1 });
  expectMatch("leggimi la mail di Marco", "read_recent_emails", { limit: 1, contact: "Marco" });
  expectMatch("leggi l'email di Sara", "read_recent_emails", { limit: 1, contact: "Sara" });
});

// ── Sonda: minuscoli plausibili → tool in silenzio, decide il runtime ──
test("contatti minuscoli → modalita sonda", () => {
  expectMatch("leggi la mail di assoholding", "read_recent_emails", { limit: 1, contact: "assoholding" }, true);
  expectMatch("email di assoholding", "search_by_contact", { contact: "assoholding", limit: 20 }, true);
  expectMatch("email di lavoro", "search_by_contact", { contact: "lavoro", limit: 20 }, true); // 0 risultati a runtime → modello
  expectMatch("dammi le email da mario rossi", "search_by_contact", { contact: "mario rossi", limit: 20 }, true);
});

// ── list_emails ──
test("elenca: ultime email", () => {
  expectMatch("Elenca le ultime email", "list_emails", { limit: 10 });
  expectMatch("ultime 20 mail", "list_emails", { limit: 20 });
  expectMatch("mostrami le mie email recenti", "list_emails", { limit: 10 });
  expectMatch("le ultime email", "list_emails", { limit: 10 });
  expectMatch("visualizza le ultime quindici email", "list_emails", { limit: 15 });
});

// ── search_emails (argomento) ──
test("cerca: per argomento", () => {
  expectMatch("cerca email su fatture", "search_emails", { query: "fatture", limit: 10 });
  expectMatch("trovami le mail che parlano di preventivo Rossi", "search_emails", { query: "preventivo Rossi", limit: 10 });
  expectMatch("cerca le email in merito a wallbox", "search_emails", { query: "wallbox", limit: 10 });
});

// ── search_by_contact ──
test("cerca: per contatto", () => {
  expectMatch("email di Mario Rossi", "search_by_contact", { contact: "Mario Rossi", limit: 20 });
  expectMatch("cerca le email da mario@rossi.it", "search_by_contact", { contact: "mario@rossi.it", limit: 20 });
  expectMatch("dammi le ultime 5 email di Bianchi", "search_by_contact", { contact: "Bianchi", limit: 5 });
});

// ── get_stats ──
test("statistiche", () => {
  expectMatch("Quante email ho?", "get_stats", {});
  expectMatch("QUANTE MAIL CI SONO", "get_stats", {});
  expectMatch("statistiche della posta", "get_stats", {});
});

// ── NEGATIVI: mutazione (G1) ──
test("mai su intenti di scrittura/azione", () => {
  expectNull("rispondi all'email di Mario");
  expectNull("inoltra l'ultima mail a Luca");
  expectNull("leggi le ultime email e rispondi a quella urgente");
  expectNull("invia una email a Sara");
  expectNull("scrivi una mail al commercialista");
  expectNull("elimina le email di spam");
});

// ── NEGATIVI: anafora (G2) ──
test("mai su riferimenti alla conversazione", () => {
  expectNull("e quelle di ieri?");
  expectNull("mostrami anche le altre");
  expectNull("leggi le ultime 3 email di quello di prima");
  expectNull("ancora");
});

// ── NEGATIVI: vincoli temporali / filtri (G3) ──
test("mai su filtri che gli args non esprimono", () => {
  expectNull("cerca le email di ieri");
  expectNull("mostrami le email non lette");
  expectNull("quante email ho ricevuto da Mario ieri?");
  expectNull("le email della settimana scorsa");
  expectNull("email con allegati");
});

// ── Conferma sonda: il contatto deve stare in From:/To:, non nell'oggetto ──
test("probeConfirmsContact", () => {
  const hit = "Found 1 emails involving 'assoholding':\nFrom: Assoholding <mail@assoholding.it>\nTo: e.mancinelli@emotion-team.com\nSubject: CRS-FATCA";
  const miss = "Found 1 emails involving 'lavoro':\nFrom: SOLAX POWER <r.p@solaxpower.it>\nTo: e.mancinelli@emotion-team.com\nSubject: visibilità per il tuo lavoro";
  assert.equal(probeConfirmsContact(hit, "assoholding"), true);
  assert.equal(probeConfirmsContact(miss, "lavoro"), false); // "lavoro" solo nell'oggetto
  assert.equal(probeConfirmsContact("", "x"), false);
});

// ── NEGATIVI: code di/da che nemmeno la sonda accetta ──
test("di/da non sondabile → modello", () => {
  expectNull("email di chi mi ha contattato per i preventivi"); // >2 token
  expectNull("leggi le email di q");                            // token troppo corto
});

// ── NEGATIVI: negazione, composte, altri canali ──
test("negazioni, richieste composte, altri canali", () => {
  expectNull("non mostrarmi le email");
  expectNull("ultimi messaggi telegram");
  expectNull("leggi le email? e i messaggi slack?"); // due "?"
});

// ── NEGATIVI: fuori dominio ──
test("chat normale al modello", () => {
  expectNull("ciao");
  expectNull("che ore sono");
  expectNull("quanto costa una wallbox?");
  expectNull("dammi una mano col preventivo");
});
