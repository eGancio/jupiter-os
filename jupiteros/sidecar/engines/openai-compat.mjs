// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

/**
 * OpenAICompatEngine — one engine for any OpenAI-compatible Chat Completions API:
 * Groq, OpenRouter, Together, a local vLLM, etc. Registered once per provider in
 * agent.mjs with a different baseUrl + key env (so "groq" and "openrouter" share
 * this code).
 *
 *   Chat + tool-calling over the Moon MCP tools, using the OpenAI function format
 *   (tools:[{type:"function",function:{...}}], response message.tool_calls with
 *   function.arguments as a JSON STRING; results fed back as role:"tool" with
 *   tool_call_id). Non-streaming for reliability — these providers are fast.
 *
 * Implements the Engine contract (see engine.mjs). Safety: the same tool DENYLIST
 * as the other engines (isUnsafeTool) hides destructive tools; minimal prompt.
 *
 * NOTE: cloud — data leaves the machine. Free tiers are rate-capped (and may be
 * used to improve the provider's models). Not for sensitive client data.
 *
 * TODO(refactor): the MCP-connect + denylist block is now duplicated across the
 * ollama/gemini/openai-compat engines — extract a shared helper into engine.mjs.
 */

import { loadMcpServers, relevantToolNames } from "./engine.mjs";
import { isUnsafeTool } from "./ollama.mjs";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { SSEClientTransport } from "@modelcontextprotocol/sdk/client/sse.js";
import { StreamableHTTPClientTransport } from "@modelcontextprotocol/sdk/client/streamableHttp.js";

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

function flattenToolResult(res) {
  const content = res?.content;
  if (Array.isArray(content)) {
    const text = content.filter((c) => c && c.type === "text").map((c) => c.text).join("");
    return text || JSON.stringify(content);
  }
  if (typeof content === "string") return content;
  return JSON.stringify(res ?? "");
}

export class OpenAICompatEngine {
  constructor(cfg = {}, provider = {}) {
    this.messages = [];
    this.controller = null;
    this.aborted = false;
    this.name = provider.name || "openai";
    this.baseUrl = (provider.baseUrl || cfg.baseUrl || "").replace(/\/$/, "");
    const envs = provider.keyEnvs || ["OPENAI_API_KEY"];
    this.apiKey = envs.map((e) => process.env[e]).find(Boolean) || cfg.apiKey || "";
    this.keyHint = envs[0];
    this.mcp = null;
    this.toolSeq = 0;
    this.systemSeeded = false;
  }

  async *run(prompt, options) {
    const { session_id, model, mcpConfigPath } = options;
    if (!this.apiKey) {
      throw new Error(`${this.keyHint} mancante — aggiungila a credentials.env (accanto al binario) e riavvia l'app.`);
    }

    const mcp = await this.#ensureMcp(mcpConfigPath);
    if (!this.systemSeeded) {
      this.messages.unshift({ role: "system", content: SYSTEM_PROMPT });
      this.systemSeeded = true;
    }
    this.messages.push({ role: "user", content: String(prompt ?? "") });
    this.aborted = false;

    // Route: expose only the tools of the Moon(s) this message is about, so we
    // don't blow the provider's token-per-minute limit (Groq free ≈ 12k TPM) or
    // trigger random tool calls. null → no relevant Moon → chat with no tools.
    const allowed = relevantToolNames(prompt, mcp.index);
    const selected = allowed ? mcp.tools.filter((t) => allowed.has(t.function.name)) : [];
    const tools = selected.length ? selected : undefined;
    process.stderr.write(`[SIDECAR] ${this.name}: ${selected.length}/${mcp.tools.length} tool esposti per questo messaggio\n`);
    const usage = { input_tokens: 0, output_tokens: 0 };
    let iters = 0;

    while (true) {
      iters++;
      this.controller = new AbortController();

      let response;
      try {
        response = await fetch(`${this.baseUrl}/chat/completions`, {
          method: "POST",
          headers: { "Content-Type": "application/json", Authorization: `Bearer ${this.apiKey}` },
          body: JSON.stringify({ model, messages: this.messages, ...(tools ? { tools, tool_choice: "auto" } : {}) }),
          signal: this.controller.signal,
        });
      } catch (e) {
        this.controller = null;
        if (this.aborted || e?.name === "AbortError") throw abortError();
        throw new Error(`${this.name} non raggiungibile — controlla la connessione. [${e?.message || e}]`);
      }

      if (!response.ok) {
        let detail = "";
        try { detail = (await response.text()).trim(); } catch { /* ignore */ }
        this.controller = null;
        if (response.status === 401 || response.status === 403) {
          throw new Error(`${this.name}: chiave non valida o non autorizzata (${response.status}).${detail ? ` [${detail.slice(0, 200)}]` : ""}`);
        }
        if (response.status === 404) {
          throw new Error(`${this.name}: modello "${model}" non trovato (404).${detail ? ` [${detail.slice(0, 200)}]` : ""}`);
        }
        if (response.status === 429) {
          throw new Error(`${this.name}: limite di frequenza raggiunto (429). Riprova tra un po'.${detail ? ` [${detail.slice(0, 200)}]` : ""}`);
        }
        throw new Error(`${this.name} ha risposto ${response.status} ${response.statusText}${detail ? `: ${detail.slice(0, 200)}` : ""}`);
      }

      let j;
      try { j = await response.json(); } catch (e) {
        this.controller = null;
        if (this.aborted || e?.name === "AbortError") throw abortError();
        throw e;
      }
      this.controller = null;

      const u = j.usage || {};
      usage.input_tokens += u.prompt_tokens || 0;
      usage.output_tokens += u.completion_tokens || 0;

      const msg = j.choices?.[0]?.message || {};
      const toolCalls = Array.isArray(msg.tool_calls) ? msg.tool_calls : [];

      if (msg.content) yield { event: "text_delta", session_id, text: msg.content };
      if (this.aborted) throw abortError();

      if (!toolCalls.length) {
        this.messages.push({ role: "assistant", content: msg.content || "" });
        break;
      }

      // Keep the assistant turn (with its tool_calls) for context, then run them.
      this.messages.push({ role: "assistant", content: msg.content || "", tool_calls: toolCalls });
      for (const tc of toolCalls) {
        if (this.aborted) throw abortError();
        const name = tc.function?.name || "";
        let args = {};
        try { args = JSON.parse(tc.function?.arguments || "{}"); } catch { /* keep {} */ }
        // Models sometimes emit arguments as "null"/array/scalar; MCP tools want
        // an object → coerce, else the server rejects with "expected record".
        if (!args || typeof args !== "object" || Array.isArray(args)) args = {};
        const tool_id = tc.id || `o${++this.toolSeq}_${name || "tool"}`;
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
        this.messages.push({ role: "tool", tool_call_id: tc.id, content: resultText });
      }

      if (iters >= MAX_TOOL_ITERS) {
        const m = "\n\n[Limite di passaggi tool raggiunto: mi fermo.]";
        yield { event: "text_delta", session_id, text: m };
        this.messages.push({ role: "assistant", content: m });
        break;
      }
    }

    yield {
      event: "result", session_id, subtype: "success", total_cost_usd: 0,
      usage: { input_tokens: usage.input_tokens, output_tokens: usage.output_tokens, cache_read_tokens: 0 },
      num_turns: iters, duration_ms: 0,
    };
  }

  async #ensureMcp(mcpConfigPath) {
    if (!this.mcp) {
      let servers = {};
      if (mcpConfigPath) { try { servers = loadMcpServers(mcpConfigPath); } catch { servers = {}; } }
      this.mcp = { tools: [], index: new Map(), clients: [], excluded: 0, servers, connected: new Set(), warned: new Set() };
    }
    const m = this.mcp;
    for (const [serverName, def] of Object.entries(m.servers || {})) {
      if (m.connected.has(serverName)) continue;
      if (!def.url || (def.type !== "sse" && def.type !== "http")) {
        if (!m.warned.has(serverName)) { m.warned.add(serverName); process.stderr.write(`[SIDECAR] ${this.name} MCP: salto ${serverName} (transport ${def.type})\n`); }
        continue;
      }
      let client = null;
      let transport = null;
      try {
        transport = def.type === "http"
          ? new StreamableHTTPClientTransport(new URL(def.url))
          : new SSEClientTransport(new URL(def.url));
        client = new Client({ name: `jupiteros-${this.name}`, version: "0.1.0" }, { capabilities: {} });
        await client.connect(transport);
        const listed = await client.listTools();
        m.clients.push(client);
        m.connected.add(serverName);
        for (const t of (listed?.tools || [])) {
          if (isUnsafeTool(t.name)) { m.excluded++; continue; }
          if (m.index.has(t.name)) continue;
          m.index.set(t.name, { client, realName: t.name, server: serverName });
          m.tools.push({ type: "function", function: { name: t.name, description: t.description || "", parameters: t.inputSchema || { type: "object", properties: {} } } });
        }
        process.stderr.write(`[SIDECAR] ${this.name} MCP: ${serverName} connesso (${(listed?.tools || []).length} tool)\n`);
      } catch (e) {
        try { await client?.close?.(); } catch { /* ignore */ }
        try { await transport?.close?.(); } catch { /* ignore */ }
        if (!m.warned.has(serverName)) { m.warned.add(serverName); process.stderr.write(`[SIDECAR] ${this.name} MCP: ${serverName} non raggiungibile — riprovo dopo (${e?.message || e})\n`); }
      }
    }
    return m;
  }

  interrupt() {
    this.aborted = true;
    if (this.controller) { try { this.controller.abort(); } catch { /* ignore */ } this.controller = null; }
  }

  close() {
    this.interrupt();
    if (this.mcp) { for (const c of this.mcp.clients) { try { c.close(); } catch { /* ignore */ } } this.mcp = null; }
  }

  getResumeToken() { return null; }
  setResumeToken(_token) { /* no-op */ }
  compact() { this.messages = []; this.systemSeeded = false; }
}
