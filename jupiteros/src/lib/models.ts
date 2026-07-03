// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

// Sorgente unica dei modelli selezionabili per engine e delle etichette mostrate
// nella UI. Usato sia dal dropdown in chat (ModelDropdown) sia dal modale
// EngineSettings, così l'elenco non è duplicato.

/** Modelli selezionabili per ogni engine (alias Claude o id grezzo). */
export const ENGINE_MODELS: Record<string, string[]> = {
  claude: ["sonnet", "haiku", "opus", "claude-fable-5"],
  // qwen3 30B-A3B instruct (MoE, 3B attivi): il miglior locale su CPU.
  ollama: ["qwen3:30b-a3b-instruct-2507-q4_K_M", "qwen2.5:7b", "qwen2.5:3b"],
  gemini: ["gemini-2.5-flash", "gemini-flash-latest", "gemini-2.5-pro"],
  groq: ["llama-3.3-70b-versatile", "openai/gpt-oss-120b", "llama-3.1-8b-instant"],
  // dwarfstar (antirez/ds4): server locale OpenAI-compatible per DeepSeek V4.
  // Entrambi gli id sono alias del GGUF caricato sul server.
  dwarfstar: ["deepseek-v4-flash", "deepseek-v4-pro"],
  // localops: llama-server (llama.cpp) su CPU per le operazioni; l'id è
  // l'--alias del GGUF caricato dal daemon localops-llm (.mcp.json).
  // Qwen3.6-35B-A3B: 100% sull'eval tool-calling (tools/localops-eval),
  // contro l'88.5% del 30B-A3B-2507. Thinking disabilitato da template.
  localops: ["qwen3.6-35b-a3b"],
  // openrouter: API multi-modello (chiave OPENROUTER_API_KEY in
  // credentials.env). V4 Flash: 96.2% tool-corretto sulla superficie piena
  // (59 tool) nell'eval localops-eval — regge i Moon senza filtri.
  openrouter: ["deepseek/deepseek-v4-flash", "deepseek/deepseek-v4-pro"],
};

/** Famiglie Claude → nome leggibile. */
const CLAUDE_FAMILY_LABEL: Record<string, string> = {
  opus: "Opus",
  sonnet: "Sonnet",
  haiku: "Haiku",
  fable: "Fable",
};

// Etichette di ripiego per gli alias Claude quando NON conosciamo ancora l'id
// risolto dall'SDK. Volutamente SENZA numero di versione: mostrare un numero
// hardcoded rischia di diventare stale (è il bug che vogliamo evitare). Il
// numero appare solo quando è quello reale risolto a runtime (system_init).
export const CLAUDE_ALIAS_FALLBACK: Record<string, string> = {
  opus: "Opus",
  sonnet: "Sonnet",
  haiku: "Haiku",
  "claude-fable-5": "Fable 5",
};

/**
 * Converte un id modello Claude risolto in etichetta leggibile con versione.
 *   claude-opus-4-8   → "Opus 4.8"
 *   claude-sonnet-4-6 → "Sonnet 4.6"
 *   claude-haiku-4-5  → "Haiku 4.5"
 *   claude-fable-5    → "Fable 5"
 * Ritorna null se non riconosciuto (così il chiamante può fare fallback).
 */
export function parseClaudeModelId(fullId: string): string | null {
  if (!fullId) return null;
  // claude-fable-5 (e simili: famiglia + un solo numero)
  let m = fullId.match(/^claude-([a-z]+)-(\d+)$/);
  if (m) {
    const fam = CLAUDE_FAMILY_LABEL[m[1]];
    return fam ? `${fam} ${m[2]}` : null;
  }
  // claude-opus-4-8, claude-sonnet-4-6, … (famiglia + major-minor, eventuale data)
  m = fullId.match(/^claude-([a-z]+)-(\d+)-(\d+)/);
  if (m) {
    const fam = CLAUDE_FAMILY_LABEL[m[1]];
    return fam ? `${fam} ${m[2]}.${m[3]}` : null;
  }
  return null;
}

/** Famiglia Claude di un alias/id ("opus" da "opus" o da "claude-opus-4-8"). */
function claudeFamilyOf(idOrAlias: string): string | null {
  if (idOrAlias === "claude-fable-5" || idOrAlias === "fable") return "fable";
  const m = idOrAlias.match(/^(?:claude-)?([a-z]+)/);
  const fam = m?.[1];
  return fam && CLAUDE_FAMILY_LABEL[fam] ? fam : null;
}

/**
 * Etichetta da mostrare per (engine, alias) nella UI.
 * - Claude: se `resolvedFullId` è noto E appartiene alla stessa famiglia
 *   dell'alias, mostra la versione reale ("Opus 4.8"); altrimenti il fallback
 *   senza numero ("Opus"). Per fable, fallback "Fable 5".
 * - Altri engine: l'id grezzo così com'è.
 */
export function modelDisplayLabel(
  engine: string,
  alias: string,
  resolvedFullId?: string | null,
): string {
  if (engine !== "claude") return alias;

  if (resolvedFullId) {
    const aliasFam = claudeFamilyOf(alias);
    const resolvedFam = claudeFamilyOf(resolvedFullId);
    if (aliasFam && aliasFam === resolvedFam) {
      const parsed = parseClaudeModelId(resolvedFullId);
      if (parsed) return parsed;
    }
  }

  if (CLAUDE_ALIAS_FALLBACK[alias]) return CLAUDE_ALIAS_FALLBACK[alias];
  return parseClaudeModelId(alias) ?? alias;
}
