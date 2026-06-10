// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

/**
 * JupiterOS Engine contract + registry.
 *
 * An Engine is the pluggable agent runtime behind a chat session. The sidecar
 * transport (agent.mjs) owns the stdin/stdout JSONL protocol and delegates the
 * actual "run a turn" work to an Engine, so that Claude (today) and future
 * backends (OpenAI-compatible / Ollama, Gemini) are interchangeable.
 *
 * THE NORMALIZED EVENT IS THE PROTOCOL EVENT. `run()` yields exactly the objects
 * that the transport passes to `emit()` — there is no separate vocabulary. This
 * keeps the Rust-side JSONL contract byte-for-byte identical across engines.
 *
 * Normalized event variants (field names are frozen — see claude.rs stdout_reader):
 *   { event:"text_delta",       session_id, text }
 *   { event:"thinking_start",   session_id }
 *   { event:"thinking_delta",   session_id, thinking }
 *   { event:"tool_start",       session_id, tool_name, tool_id }
 *   { event:"tool_input_delta", session_id, tool_id, partial_json }
 *   { event:"tool_result",      session_id, tool_id, result }
 *   { event:"result",           session_id, subtype, total_cost_usd,
 *                               usage:{input_tokens,output_tokens,cache_read_tokens},
 *                               num_turns, duration_ms }
 *   { event:"system_init",      session_id, sdk_session_id, model, tools, mcp_servers }
 *
 * Lifecycle events (`ready`, `session_created`, `done`, `error`,
 * `session_compacted`) are emitted by the TRANSPORT, never by the engine. The
 * engine signals turn completion by the async generator finishing, and failure
 * by throwing.
 *
 * @typedef {Object} Engine
 * @property {(prompt: any, options: object) => AsyncGenerator<object>} run
 *   Run one turn. Yields normalized events. `options` carries neutral per-turn
 *   inputs: { session_id, model, cwd, mcpConfigPath, images }.
 * @property {() => void} interrupt  Cooperative interrupt of the in-flight run.
 * @property {() => void} close      Dispose any per-session resources.
 * @property {() => (string|null)} getResumeToken  Engine-opaque resume token.
 * @property {(token: string|null) => void} setResumeToken  Restore/clear it.
 * @property {() => void} compact    Reset context (fresh turn, same UI session).
 */

import * as fs from "fs";

// ── Shared MCP config helpers (engine-agnostic) ──────────────

export function expandEnv(value) {
  if (typeof value !== "string") return value;
  return value.replace(/\$\{([A-Z0-9_]+)\}/gi, (_, name) => process.env[name] ?? "");
}

// ── Per-message tool routing (engine-agnostic) ───────────────
// Local/free models choke when handed ALL Moon tools on every call: the schemas
// alone cost thousands of tokens (Groq free is ~12k tokens/MINUTE), and with a
// big tool menu the model calls things at random (e.g. a calendar lookup for
// "ciao"). So expose only the tools of the Moon(s) the message is actually about.
// Returns a Set of allowed tool names, or null (no relevant Moon → send no tools
// and let the model just chat). Claude doesn't use this — it has its own
// deferred-tool/ToolSearch mechanism via the SDK.

const MOON_KEYWORDS = {
  io: ["email", "mail", "posta", "inbox", "mittente", "allegat", "calendar", "evento", "appuntament", "firma", "casella"],
  europa: ["messaggi", "messaggio", "telegram", "slack", "teams", "contatt", "chat di"],
  gtd: ["task", "gtd", "progett", "obiettiv", "milestone", "attività", "attivita", "todo", "da fare", "cosa devo", "retrospettiv", "scadenz"],
  metis: ["document", "norma", "bando", "preventiv", "fascicol", "knowledge", "cerca nei", "nei miei file", "pdf"],
  amalthea: ["grafic", "chart", "diagramm", "istogramm", "torta", "barre", "plot", "visualizz", "mappa mentale", "flowchart"],
  callisto: ["video", "trascriv", "trascrizion", "youtube", "sottotitol"],
  himalia: ["cerca sul web", "sul web", "online", "internet", "in rete", "trova fonti", "fonti su", "naviga", "apri la pagina", "leggi la pagina", "sito web", "tavily"],
  elara: ["notizie", "news", "trend", "sentiment", "reddit", "community", "valida", "validare", "idea di business", "ricerca di mercato", "si lamentano", "gdelt"],
};

export function relevantToolNames(message, index) {
  const m = String(message || "").toLowerCase();
  const moons = new Set();
  for (const [moon, kws] of Object.entries(MOON_KEYWORDS)) {
    if (kws.some((k) => m.includes(k))) moons.add(moon);
  }
  if (moons.size === 0) return null; // nothing relevant → no tools, just chat
  const names = new Set();
  for (const [name, info] of index) {
    if (moons.has(info.server)) names.add(name);
  }
  return names.size ? names : null;
}

// Rough token estimate for one tool schema (no tokenizer in-repo): ~4 chars/token.
function estToolTokens(tool) {
  return Math.ceil(JSON.stringify(tool).length / 4);
}

// Budget-aware tool reducer (engine-agnostic). Replaces the old HARD GATE (no
// keyword → zero tools → the model can't see the Moons) with a fail-OPEN,
// token-budgeted selector:
//   • If ALL tools fit the budget → expose them all (like Claude). No crutch.
//   • Else → narrow to the Moon(s) the message is about; if NOTHING matches, keep
//     all and let the cap trim it (fail open, never zero).
//   • ALWAYS cap the result to maxToolTokens so Groq free (~12k tokens/MINUTE)
//     never 429s on tool schemas alone.
// getName(toolObj)=>string adapts the tool shape: openai/ollama → t.function.name,
// gemini → d.name. Returns an array (empty only when no Moon is connected yet).
export function selectToolsForMessage(message, allTools, index, opts = {}) {
  const getName = opts.getName || ((t) => t?.function?.name);
  const budget = Number.isFinite(opts.maxToolTokens) ? opts.maxToolTokens : 6000;
  if (!allTools.length) return allTools;

  const total = allTools.reduce((n, t) => n + estToolTokens(t), 0);
  if (total <= budget) return allTools; // everything fits → expose all

  const allowed = relevantToolNames(message, index);
  const narrowed = allowed ? allTools.filter((t) => allowed.has(getName(t))) : [];
  const candidate = narrowed.length ? narrowed : allTools; // no match → fail open

  const out = [];
  let used = 0;
  for (const t of candidate) {
    const est = estToolTokens(t);
    if (out.length && used + est > budget) break; // always keep ≥1 tool
    used += est;
    out.push(t);
  }
  return out;
}

export function loadMcpServers(configPath) {
  try {
    const raw = fs.readFileSync(configPath, "utf-8");
    const config = JSON.parse(raw);
    const servers = config.mcpServers || {};
    const result = {};
    for (const [name, def] of Object.entries(servers)) {
      if (def.type === "sse" && def.url) {
        result[name] = { type: "sse", url: def.url };
      } else if (def.type === "stdio" && def.command) {
        const env = {};
        for (const [k, v] of Object.entries(def.env || {})) {
          env[k] = expandEnv(v);
        }
        result[name] = {
          type: "stdio",
          command: expandEnv(def.command),
          args: (def.args || []).map(expandEnv),
          env,
        };
      } else if (def.type === "http" && def.url) {
        result[name] = { type: "http", url: def.url };
      }
    }
    return result;
  } catch {
    return {};
  }
}

// ── Tool-arg sanitation (engine-agnostic) ───────────────────
// Open models (esp. Llama on Groq) produce sloppy-but-common tool args: integers
// as STRINGS ("limit":"5"), and null for optional params ("account":null). Groq
// validates the call server-side and 400s before we even see it; the Rust Moons'
// serde also rejects the string. So we (a) relax the schema sent to strict
// validators so they stop rejecting, and (b) coerce/strip the args back to a
// clean object before invoking the tool.

// Relax a JSON-schema so a STRICT validator (Groq) won't 400 on those outputs:
// every leaf `type` also accepts "null" (optional params), and numeric types
// also accept "string". Returns a NEW schema (original untouched).
export function relaxSchemaForValidation(schema) {
  if (!schema || typeof schema !== "object") return schema;
  const out = Array.isArray(schema) ? [] : {};
  for (const [k, v] of Object.entries(schema)) {
    if (k === "type" && typeof v === "string") {
      const t = new Set([v, "null"]);
      if (v === "integer" || v === "number") t.add("string");
      out[k] = [...t];
    } else if (v && typeof v === "object") {
      out[k] = relaxSchemaForValidation(v);
    } else {
      out[k] = v;
    }
  }
  return out;
}

// Sanitize a model's tool-call arguments before the MCP call:
//   • drop null values (an omitted optional → serde None; null can break it);
//   • coerce numeric strings → numbers for params typed integer/number;
//   • SAFETY: never let a re-index tool run a FULL rebuild (~19k emails) — force
//     full_reindex=false. reset_index stays fully denylisted elsewhere.
export function sanitizeToolArgs(name, args, paramSchema) {
  if (!args || typeof args !== "object" || Array.isArray(args)) return {};
  const props = (paramSchema && paramSchema.properties) || {};
  const out = {};
  for (const [k, v] of Object.entries(args)) {
    if (v === null) continue; // optional param → omit rather than pass null
    const t = props[k] && props[k].type;
    const wantsNum = t === "integer" || t === "number"
      || (Array.isArray(t) && (t.includes("integer") || t.includes("number")));
    out[k] = (wantsNum && typeof v === "string" && v.trim() !== "" && Number.isFinite(Number(v)))
      ? Number(v)
      : v;
  }
  if ((name === "index_emails" || name === "index_message") && "full_reindex" in out) {
    out.full_reindex = false;
  }
  return out;
}

// ── Moon boot-race guard (engine-agnostic) ───────────────────
// I Moon ripartono INSIEME all'app, quindi il primo messaggio di una sessione
// spesso arriva mentre stanno ancora bootando. Il Claude SDK connette gli MCP
// all'avvio della query e NON riprova dentro il turno: il modello vede "server
// in connessione", resta senza tool e improvvisa (Bash/curl). Attesa una-tantum,
// limitata: probe in parallelo degli URL sse/http finché tutti su o deadline.
// I Moon volutamente spenti costano al massimo `timeoutMs` UNA volta a sessione.
export async function waitForMcpServers(servers, { timeoutMs = 5000, intervalMs = 400 } = {}) {
  const urls = Object.values(servers || {})
    .filter((d) => (d.type === "sse" || d.type === "http") && d.url)
    .map((d) => d.url);
  if (!urls.length) return;
  const deadline = Date.now() + timeoutMs;
  const pending = new Set(urls);
  while (pending.size) {
    await Promise.all([...pending].map(async (u) => {
      const ctrl = new AbortController();
      const t = setTimeout(() => ctrl.abort(), 1500);
      try {
        // GET (non HEAD): l'endpoint SSE risponde con gli header e inizia lo
        // stream — basta vederli, poi si chiude subito il body.
        const res = await fetch(u, { signal: ctrl.signal, headers: { accept: "text/event-stream" } });
        try { res.body?.cancel?.(); } catch { /* ignore */ }
        if (res.ok || res.status === 405) pending.delete(u);
      } catch { /* ancora giù */ } finally { clearTimeout(t); }
    }));
    if (!pending.size || Date.now() >= deadline) break;
    await new Promise((r) => setTimeout(r, intervalMs));
  }
  if (pending.size) {
    process.stderr.write(`[SIDECAR] MCP wait: ${pending.size} server ancora giù dopo ${timeoutMs}ms — proseguo senza\n`);
  }
}

// ── Context budget helpers (engine-agnostic) ─────────────────
// Free/local backends have hard context ceilings (Groq free ≈ 12k TPM, Ollama
// num_ctx). Tool results and history must be capped BEFORE re-entering the
// prompt, or the provider 429s (Groq) / silently truncates from the TOP and the
// model loses system prompt + tool schemas (Ollama → stalls and garbage).

export function truncateForContext(text, maxChars) {
  if (typeof text !== "string" || text.length <= maxChars) return text;
  return text.slice(0, maxChars)
    + `\n\n[output troncato a ${maxChars} caratteri per i limiti del modello — chiedi un sottoinsieme più piccolo (es. meno email, solo gli ID) per il resto]`;
}

export function estMessagesTokens(messages) {
  return Math.ceil(JSON.stringify(messages).length / 4); // ~4 chars/token, come estToolTokens
}

// Tieni la history sotto budget scartando i turni più vecchi (system prompt in
// [0] e ultimo messaggio sempre preservati). Se il messaggio rimosso era un
// assistant con tool_calls, via anche i suoi role:"tool" orfani, o il provider
// rifiuta la conversazione.
export function trimHistoryInPlace(messages, budgetTokens) {
  while (messages.length > 2 && estMessagesTokens(messages) > budgetTokens) {
    messages.splice(1, 1);
    while (messages.length > 2 && messages[1]?.role === "tool") messages.splice(1, 1);
  }
}

// ── Engine registry / factory ────────────────────────────────

/** @type {Map<string, (cfg: object) => Engine>} */
const ENGINES = new Map();

/**
 * Register an engine factory under a name. Called once at startup by agent.mjs.
 * @param {string} name
 * @param {(cfg: object) => Engine} factory
 */
export function registerEngine(name, factory) {
  ENGINES.set(name, factory);
}

/**
 * Create an engine instance. Unknown/missing name falls back to "claude" so
 * old persisted sessions and the default path keep working unchanged.
 * @param {string} name
 * @param {object} cfg  Per-session config: { model, cwd, mcpConfigPath }.
 * @returns {Engine}
 */
export function createEngine(name, cfg) {
  const factory = ENGINES.get(name) ?? ENGINES.get("claude");
  if (!factory) {
    throw new Error(`No engine registered for "${name}" and no "claude" fallback`);
  }
  return factory(cfg);
}
