// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

/**
 * GeminiEngine — Google Gemini (free tier via AI Studio API key) as an Engine.
 *
 *   PHASE A — chat streaming via the REST API (no SDK dependency, like Ollama).
 *   PHASE B — Gemini function-calling over the Moon MCP tools. Gemini surfaces
 *             functionCall parts in the SSE stream and takes the result back as a
 *             content with role "function" (both verified against the live API).
 *
 * Implements the Engine contract (see engine.mjs). History is kept in-process as
 * Gemini `contents` ([{role:"user"|"model"|"function", parts:[...]}]). Safety:
 * the same tool DENYLIST as the Ollama engine (isUnsafeTool) hides destructive
 * tools; system prompt stays minimal.
 *
 * Key: GEMINI_API_KEY (or GOOGLE_API_KEY) from credentials.env. NOTE: Gemini is
 * CLOUD — data leaves the machine, and the free tier may be used by Google to
 * improve its models. Not for sensitive client data.
 */

import { loadMcpServers, relevantToolNames } from "./engine.mjs";
import { isUnsafeTool } from "./ollama.mjs";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { SSEClientTransport } from "@modelcontextprotocol/sdk/client/sse.js";
import { StreamableHTTPClientTransport } from "@modelcontextprotocol/sdk/client/streamableHttp.js";

const API_BASE = "https://generativelanguage.googleapis.com/v1beta";
const MAX_TOOL_ITERS = 8;

const SYSTEM_PROMPT = [
  "Sei l'assistente di JupiterOS.",
  "Rispondi nella lingua dell'utente.",
  "Usa gli strumenti quando servono dati; se non li hai, dillo invece di inventare.",
].join("\n");

function abortError() {
  const e = new Error("Interrotto");
  e.name = "AbortError";
  return e;
}

/** Flatten an MCP CallTool result to text (mirrors the Claude/Ollama engines). */
function flattenToolResult(res) {
  const content = res?.content;
  if (Array.isArray(content)) {
    const text = content.filter((c) => c && c.type === "text").map((c) => c.text).join("");
    return text || JSON.stringify(content);
  }
  if (typeof content === "string") return content;
  return JSON.stringify(res ?? "");
}

/**
 * Reduce a JSON Schema (from MCP inputSchema) to the OpenAPI subset Gemini's
 * functionDeclarations accept. Gemini 400s on unknown keys ($schema,
 * additionalProperties, $ref, anyOf, default, ...), so keep only the safe set.
 */
function sanitizeSchema(s) {
  if (!s || typeof s !== "object") return { type: "string" };
  const out = {};
  if (s.type) out.type = Array.isArray(s.type) ? s.type[0] : s.type;
  if (typeof s.description === "string") out.description = s.description;
  if (Array.isArray(s.enum)) out.enum = s.enum;
  if (s.items) out.items = sanitizeSchema(s.items);
  if (s.properties && typeof s.properties === "object") {
    out.properties = {};
    for (const [k, v] of Object.entries(s.properties)) out.properties[k] = sanitizeSchema(v);
  }
  if (Array.isArray(s.required) && s.required.length) out.required = s.required;
  if (!out.type) out.type = out.properties ? "object" : "string";
  if (out.type === "object" && !out.properties) out.properties = {};
  return out;
}

export class GeminiEngine {
  constructor(cfg = {}) {
    /** Gemini history: [{ role: "user"|"model"|"function", parts: [...] }]. */
    this.contents = [];
    this.controller = null;
    this.aborted = false;
    this.apiKey = process.env.GEMINI_API_KEY || process.env.GOOGLE_API_KEY || cfg.apiKey || "";
    // MCP cache: { decls[], index: Map<name,{client,realName}>, clients[], excluded, servers, connected, warned }
    this.mcp = null;
    this.toolSeq = 0;
  }

  async *run(prompt, options) {
    const { session_id, mcpConfigPath } = options;
    // Guard against a non-Gemini model bleeding in from a cross-engine model
    // switch (it would 404). Every Gemini/Gemma model id starts with "gem".
    const model = /^gem/i.test(options.model || "") ? options.model : "gemini-2.5-flash";
    if (!/^gem/i.test(options.model || "")) {
      process.stderr.write(`[SIDECAR] gemini: modello "${options.model}" non valido per Gemini → uso ${model}\n`);
    }
    if (!this.apiKey) {
      throw new Error("GEMINI_API_KEY mancante — aggiungila a credentials.env (accanto al binario) e riavvia l'app.");
    }

    const mcp = await this.#ensureMcp(mcpConfigPath);
    this.contents.push({ role: "user", parts: [{ text: String(prompt ?? "") }] });
    this.aborted = false;

    // Route: only the relevant Moon's tools for this message (token economy +
    // fewer random tool calls). null → no relevant Moon → chat with no tools.
    const allowed = relevantToolNames(prompt, mcp.index);
    const decls = allowed ? mcp.decls.filter((d) => allowed.has(d.name)) : [];
    const tools = decls.length ? [{ functionDeclarations: decls }] : undefined;
    process.stderr.write(`[SIDECAR] gemini: ${decls.length}/${mcp.decls.length} tool esposti per questo messaggio\n`);
    const usage = { input_tokens: 0, output_tokens: 0 };
    let iters = 0;

    while (true) {
      iters++;
      const turn = yield* this.#streamOnce(session_id, model, tools);
      if (turn.usage) { usage.input_tokens += turn.usage.input_tokens; usage.output_tokens += turn.usage.output_tokens; }

      if (this.aborted) {
        if (turn.text) this.contents.push({ role: "model", parts: [{ text: turn.text }] });
        throw abortError();
      }

      if (!turn.functionCalls.length) {
        if (turn.text) this.contents.push({ role: "model", parts: [{ text: turn.text }] });
        break;
      }

      // Record the model turn (text + the function calls it requested).
      const modelParts = [];
      if (turn.text) modelParts.push({ text: turn.text });
      for (const fc of turn.functionCalls) modelParts.push({ functionCall: fc });
      this.contents.push({ role: "model", parts: modelParts });

      // Execute each call sequentially, collect functionResponse parts.
      const responseParts = [];
      for (const fc of turn.functionCalls) {
        if (this.aborted) break;
        const name = fc.name || "";
        const args = (fc.args && typeof fc.args === "object") ? fc.args : {};
        const tool_id = `g${++this.toolSeq}_${name || "tool"}`;
        yield { event: "tool_start", session_id, tool_name: name || "(sconosciuto)", tool_id };
        yield { event: "tool_input_delta", session_id, tool_id, partial_json: JSON.stringify(args) };

        let resultText;
        const entry = this.mcp.index.get(name);
        if (!entry) {
          resultText = `[errore] tool non disponibile: "${name}".`;
        } else {
          try {
            const res = await entry.client.callTool({ name: entry.realName, arguments: args });
            resultText = flattenToolResult(res);
            if (res?.isError) resultText = `[errore tool] ${resultText}`;
          } catch (e) {
            resultText = `[errore tool "${name}"] ${e?.message || e}`;
          }
        }
        yield { event: "tool_result", session_id, tool_id, result: resultText };
        responseParts.push({ functionResponse: { name, response: { result: resultText } } });
      }
      this.contents.push({ role: "function", parts: responseParts });

      if (this.aborted) throw abortError();
      if (iters >= MAX_TOOL_ITERS) {
        const msg = "\n\n[Limite di passaggi tool raggiunto: mi fermo.]";
        yield { event: "text_delta", session_id, text: msg };
        this.contents.push({ role: "model", parts: [{ text: msg }] });
        break;
      }
    }

    yield {
      event: "result", session_id, subtype: "success", total_cost_usd: 0,
      usage: { input_tokens: usage.input_tokens, output_tokens: usage.output_tokens, cache_read_tokens: 0 },
      num_turns: iters, duration_ms: 0,
    };
  }

  // One streamed model call. Yields text_delta; returns {text, functionCalls, usage}.
  async *#streamOnce(session_id, model, tools) {
    this.controller = new AbortController();
    const url = `${API_BASE}/models/${encodeURIComponent(model)}:streamGenerateContent?alt=sse&key=${this.apiKey}`;
    const body = { contents: this.contents, systemInstruction: { parts: [{ text: SYSTEM_PROMPT }] } };
    if (tools) body.tools = tools;

    process.stderr.write(`[SIDECAR] gemini streamGenerateContent session=${session_id} model=${model} tools=${tools ? tools[0].functionDeclarations.length : 0}\n`);

    let response;
    try {
      response = await fetch(url, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body),
        signal: this.controller.signal,
      });
    } catch (e) {
      this.controller = null;
      if (this.aborted || e?.name === "AbortError") return { text: "", functionCalls: [] };
      throw new Error(`Gemini non raggiungibile — controlla la connessione. [${e?.message || e}]`);
    }

    if (!response.ok) {
      let detail = "";
      try { detail = (await response.text()).trim(); } catch { /* ignore */ }
      this.controller = null;
      if (response.status === 400 || response.status === 403) {
        throw new Error(`Gemini ha rifiutato la richiesta (${response.status}) — chiave non valida, modello non abilitato, o schema tool non valido.${detail ? ` [${detail.slice(0, 300)}]` : ""}`);
      }
      if (response.status === 404) {
        throw new Error(`Modello "${model}" non trovato su Gemini.${detail ? ` [${detail.slice(0, 200)}]` : ""}`);
      }
      if (response.status === 429) {
        throw new Error(`Gemini: limite del free tier raggiunto (429). Riprova tra un po'.${detail ? ` [${detail.slice(0, 200)}]` : ""}`);
      }
      throw new Error(`Gemini ha risposto ${response.status} ${response.statusText}${detail ? `: ${detail.slice(0, 300)}` : ""}`);
    }

    let text = "";
    const functionCalls = [];
    let usage = null;
    let buffer = "";
    const decoder = new TextDecoder();
    const reader = response.body.getReader();
    try {
      while (true) {
        if (this.aborted) break;
        const { done, value } = await reader.read();
        if (done) break;
        buffer += decoder.decode(value, { stream: true });
        let nl;
        while ((nl = buffer.indexOf("\n")) >= 0) {
          const line = buffer.slice(0, nl).trim();
          buffer = buffer.slice(nl + 1);
          if (!line || !line.startsWith("data:")) continue;
          const payload = line.slice(5).trim();
          if (!payload || payload === "[DONE]") continue;
          let obj;
          try { obj = JSON.parse(payload); } catch { continue; }
          if (obj.error) throw new Error(`Gemini: ${obj.error.message || JSON.stringify(obj.error)}`);

          for (const p of (obj.candidates?.[0]?.content?.parts || [])) {
            if (typeof p.text === "string" && p.text) {
              text += p.text;
              yield { event: "text_delta", session_id, text: p.text };
            }
            if (p.functionCall) functionCalls.push(p.functionCall);
          }
          if (obj.usageMetadata) {
            usage = {
              input_tokens: obj.usageMetadata.promptTokenCount || 0,
              output_tokens: obj.usageMetadata.candidatesTokenCount || 0,
            };
          }
        }
      }
    } catch (e) {
      if (!(this.aborted || e?.name === "AbortError")) throw e;
    } finally {
      try { reader.releaseLock(); } catch { /* ignore */ }
      this.controller = null;
    }
    return { text, functionCalls, usage };
  }

  // Connect to the Moon MCP servers, retrying unconnected ones each turn.
  async #ensureMcp(mcpConfigPath) {
    if (!this.mcp) {
      let servers = {};
      if (mcpConfigPath) {
        try { servers = loadMcpServers(mcpConfigPath); } catch { servers = {}; }
      }
      this.mcp = { decls: [], index: new Map(), clients: [], excluded: 0, servers, connected: new Set(), warned: new Set() };
    }
    const m = this.mcp;
    for (const [serverName, def] of Object.entries(m.servers || {})) {
      if (m.connected.has(serverName)) continue;
      if (!def.url || (def.type !== "sse" && def.type !== "http")) {
        if (!m.warned.has(serverName)) { m.warned.add(serverName); process.stderr.write(`[SIDECAR] gemini MCP: salto ${serverName} (transport ${def.type})\n`); }
        continue;
      }
      let client = null;
      let transport = null;
      try {
        transport = def.type === "http"
          ? new StreamableHTTPClientTransport(new URL(def.url))
          : new SSEClientTransport(new URL(def.url));
        client = new Client({ name: "jupiteros-gemini", version: "0.1.0" }, { capabilities: {} });
        await client.connect(transport);
        const listed = await client.listTools();
        m.clients.push(client);
        m.connected.add(serverName);
        for (const t of (listed?.tools || [])) {
          if (isUnsafeTool(t.name)) { m.excluded++; continue; }
          if (m.index.has(t.name)) continue;
          m.index.set(t.name, { client, realName: t.name, server: serverName });
          m.decls.push({ name: t.name, description: t.description || "", parameters: sanitizeSchema(t.inputSchema) });
        }
        process.stderr.write(`[SIDECAR] gemini MCP: ${serverName} connesso (${(listed?.tools || []).length} tool)\n`);
      } catch (e) {
        try { await client?.close?.(); } catch { /* ignore */ }
        try { await transport?.close?.(); } catch { /* ignore */ }
        if (!m.warned.has(serverName)) { m.warned.add(serverName); process.stderr.write(`[SIDECAR] gemini MCP: ${serverName} non raggiungibile — riprovo dopo (${e?.message || e})\n`); }
      }
    }
    return m;
  }

  interrupt() {
    this.aborted = true;
    if (this.controller) {
      try { this.controller.abort(); } catch { /* ignore */ }
      this.controller = null;
    }
  }

  close() {
    this.interrupt();
    if (this.mcp) {
      for (const c of this.mcp.clients) { try { c.close(); } catch { /* ignore */ } }
      this.mcp = null;
    }
  }

  getResumeToken() { return null; }
  setResumeToken(_token) { /* no-op: in-process history only */ }
  compact() { this.contents = []; }
}
