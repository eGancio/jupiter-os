// Eval Fase 0 — misura l'affidabilità del tool calling di un LLM locale
// (llama-server, endpoint OpenAI-compatible) sui tool MCP reali dei Moon.
// NON esegue i tool: valuta solo la PROPOSTA di chiamata (zero side effect).
//
//   node run-eval.mjs [--config restricted-required,restricted-auto,full-auto]
//
// Richiede tools-snapshot.json (da dump-tools.mjs) e llama-server già in
// ascolto (default http://127.0.0.1:8080/v1, override LOCALOPS_EVAL_URL).
import * as fs from "fs";
import * as path from "path";
import { fileURLToPath } from "url";

const here = path.dirname(fileURLToPath(import.meta.url));
const BASE_URL = process.env.LOCALOPS_EVAL_URL || "http://127.0.0.1:8080/v1";
const MODEL = process.env.LOCALOPS_EVAL_MODEL || "qwen3-30b-a3b-instruct";
const snapshot = JSON.parse(fs.readFileSync(path.join(here, "tools-snapshot.json"), "utf8"));
const dataset = JSON.parse(fs.readFileSync(path.join(here, "dataset.json"), "utf8"));

// Stesso system prompt di openai-compat.mjs:39-43 (fedeltà alla produzione).
const SYSTEM_PROMPT = [
  "Sei l'assistente di JupiterOS.",
  "Rispondi nella lingua dell'utente.",
  "Usa gli strumenti quando servono dati; se non li hai, dillo invece di inventare.",
  // Riga extra = simulazione del campo `system` del profilo workflow (Fase 2).
  ...(dataset.systemHint ? [dataset.systemHint] : []),
].join("\n");

const overrides = dataset.toolDescriptionOverrides || {};
const allTools = snapshot.tools.map(({ server, ...t }) => {
  const d = overrides[t.function.name];
  return d ? { ...t, function: { ...t.function, description: d } } : t;
});
const restrictedNames = new Set(dataset.restrictedSurface);
const restrictedTools = allTools.filter((t) => restrictedNames.has(t.function.name));
if (restrictedTools.length !== restrictedNames.size) {
  throw new Error(`superficie ristretta incompleta: trovati ${restrictedTools.map(t => t.function.name).join(", ")}`);
}

const CONFIGS = {
  // Ordine: prima le ristrette (prefisso corto), poi full — --cache-reuse
  // riusa il prefisso di schemi identico tra richieste della stessa config.
  "restricted-required": { tools: restrictedTools, tool_choice: "required", skipNegatives: true },
  "restricted-auto": { tools: restrictedTools, tool_choice: "auto" },
  "full-auto": { tools: allTools, tool_choice: "auto" },
};
const wanted = (process.argv.includes("--config")
  ? process.argv[process.argv.indexOf("--config") + 1].split(",")
  : Object.keys(CONFIGS));

// Validatore minimale (type/required/enum) sugli schemi già "relaxed" del sidecar.
function typeOk(value, type) {
  const types = Array.isArray(type) ? type : [type];
  return types.some((t) => {
    if (t === "null") return value === null;
    if (t === "string") return typeof value === "string";
    if (t === "integer") return Number.isInteger(value) || (typeof value === "string" && /^-?\d+$/.test(value));
    if (t === "number") return typeof value === "number" || (typeof value === "string" && !isNaN(Number(value)));
    if (t === "boolean") return typeof value === "boolean";
    if (t === "array") return Array.isArray(value);
    if (t === "object") return value !== null && typeof value === "object" && !Array.isArray(value);
    return true;
  });
}
function validateArgs(args, schema) {
  if (!schema || typeof schema !== "object") return { ok: true, errors: [] };
  const errors = [];
  for (const req of schema.required || []) {
    if (args[req] === undefined || args[req] === null) errors.push(`manca required "${req}"`);
  }
  const props = schema.properties || {};
  for (const [k, v] of Object.entries(args)) {
    const p = props[k];
    if (!p) { errors.push(`arg sconosciuto "${k}"`); continue; }
    if (v === undefined) continue;
    if (p.type && !typeOk(v, p.type)) errors.push(`"${k}": tipo ${typeof v} non in ${JSON.stringify(p.type)}`);
    if (p.enum && !p.enum.includes(v)) errors.push(`"${k}": valore fuori enum`);
  }
  return { ok: errors.length === 0, errors };
}
function matchExpectedArgs(args, expected) {
  const misses = [];
  for (const [k, cond] of Object.entries(expected || {})) {
    const v = args[k];
    if (cond.re !== undefined) {
      if (typeof v !== "string" || !new RegExp(cond.re, "i").test(v)) misses.push(`${k}!~/${cond.re}/`);
    } else if (cond.eq !== undefined) {
      const norm = (x) => (typeof x === "string" && /^-?\d+$/.test(x) ? Number(x) : x);
      if (norm(v) !== norm(cond.eq)) misses.push(`${k}=${JSON.stringify(v)}≠${JSON.stringify(cond.eq)}`);
    }
  }
  return { ok: misses.length === 0, misses };
}

async function ask(tools, tool_choice, prompt) {
  const t0 = performance.now();
  const res = await fetch(`${BASE_URL}/chat/completions`, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      ...(process.env.LOCALOPS_EVAL_KEY ? { Authorization: `Bearer ${process.env.LOCALOPS_EVAL_KEY}` } : {}),
    },
    body: JSON.stringify({
      model: MODEL,
      messages: [{ role: "system", content: SYSTEM_PROMPT }, { role: "user", content: prompt }],
      tools, tool_choice, stream: false, max_tokens: 512,
      // Selezione tool deterministica: per un agente ops la temperatura da
      // chat (0.7) introduce solo varianza sulla scelta del tool.
      temperature: 0,
    }),
  });
  const wallMs = performance.now() - t0;
  if (!res.ok) throw new Error(`HTTP ${res.status}: ${(await res.text()).slice(0, 300)}`);
  const body = await res.json();
  const msg = body.choices?.[0]?.message || {};
  return { toolCalls: msg.tool_calls || [], content: msg.content || "", usage: body.usage, timings: body.timings, wallMs };
}

const offeredByConfig = {};
const results = [];
for (const cfgName of wanted) {
  const cfg = CONFIGS[cfgName];
  const offered = new Set(cfg.tools.map((t) => t.function.name));
  offeredByConfig[cfgName] = cfg.tools.length;
  console.log(`\n═══ ${cfgName} — ${cfg.tools.length} tool, tool_choice=${cfg.tool_choice} ═══`);
  for (const c of dataset.cases) {
    const isNegative = c.expected.tool === null;
    if (isNegative && cfg.skipNegatives) continue;
    let r;
    try {
      r = await ask(cfg.tools, cfg.tool_choice, c.prompt);
    } catch (e) {
      results.push({ config: cfgName, id: c.id, error: String(e.message || e) });
      console.log(`  ${c.id}  ERRORE: ${e.message}`);
      continue;
    }
    const call = r.toolCalls[0] || null;
    let row = {
      config: cfgName, id: c.id, negative: isNegative,
      called: call?.function?.name ?? null, nCalls: r.toolCalls.length,
      wallMs: Math.round(r.wallMs), promptTokens: r.usage?.prompt_tokens,
      prefillMs: r.timings?.prompt_ms ? Math.round(r.timings.prompt_ms) : undefined,
      genTps: r.timings?.predicted_per_second ? +r.timings.predicted_per_second.toFixed(1) : undefined,
    };
    if (isNegative) {
      row.correct = call === null;
      row.falsePositive = call !== null;
    } else if (!call) {
      row.correct = false; row.noCall = true;
    } else {
      const accepted = new Set([c.expected.tool, ...(c.acceptTools || [])]);
      row.hallucinated = !offered.has(call.function.name);
      row.correct = accepted.has(call.function.name);
      let args = {};
      try { args = JSON.parse(call.function.arguments || "{}"); row.argsParse = true; }
      catch { row.argsParse = false; }
      if (row.argsParse) {
        const schema = cfg.tools.find((t) => t.function.name === call.function.name)?.function.parameters;
        const val = validateArgs(args, schema);
        row.argsValid = val.ok; if (!val.ok) row.argsErrors = val.errors;
        // il match sugli args attesi ha senso solo se ha scelto il tool primario
        if (call.function.name === c.expected.tool) {
          const m = matchExpectedArgs(args, c.expected.args);
          row.argsMatch = m.ok; if (!m.ok) row.argsMisses = m.misses;
        }
        row.args = args;
      } else { row.argsValid = false; }
    }
    results.push(row);
    const flag = row.correct ? "✓" : "✗";
    console.log(`  ${flag} ${c.id}  →  ${row.called ?? "(nessun tool)"}${row.argsValid === false ? "  [ARGS INVALIDI]" : ""}${row.hallucinated ? "  [ALLUCINATO]" : ""}  ${row.wallMs}ms`);
  }
}

// ── Riepilogo per configurazione ──
console.log("\n══════════ RIEPILOGO ══════════");
const summary = {};
for (const cfgName of wanted) {
  const rows = results.filter((r) => r.config === cfgName && !r.error);
  const pos = rows.filter((r) => !r.negative);
  const neg = rows.filter((r) => r.negative);
  const errors = results.filter((r) => r.config === cfgName && r.error).length;
  const pct = (n, d) => (d ? `${((100 * n) / d).toFixed(1)}%` : "n/a");
  const lat = rows.map((r) => r.wallMs).sort((a, b) => a - b);
  const s = {
    tools: offeredByConfig[cfgName],
    casi: rows.length, errori: errors,
    toolCorretto: pct(pos.filter((r) => r.correct).length, pos.length),
    argsValidi: pct(pos.filter((r) => r.argsValid).length, pos.length),
    argsAttesiOk: pct(pos.filter((r) => r.argsMatch).length, pos.filter((r) => r.argsMatch !== undefined).length),
    allucinati: pos.filter((r) => r.hallucinated).length,
    falsiPositivi: neg.length ? pct(neg.filter((r) => r.falsePositive).length, neg.length) : "n/a",
    latMediaMs: Math.round(lat.reduce((a, b) => a + b, 0) / (lat.length || 1)),
    latP95Ms: lat[Math.floor(lat.length * 0.95)] ?? null,
  };
  summary[cfgName] = s;
  console.log(`\n${cfgName}:`, JSON.stringify(s, null, 2));
}
fs.writeFileSync(path.join(here, "results.json"), JSON.stringify({ ranAt: new Date().toISOString(), baseUrl: BASE_URL, summary, results }, null, 2));
console.log(`\nDettaglio → ${path.join(here, "results.json")}`);
