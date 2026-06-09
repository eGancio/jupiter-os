// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

/**
 * OllamaEngine — a local, offline Engine backed by an Ollama model
 * (e.g. qwen2.5:7b) running on the user's machine.
 *
 *   PHASE A — plain chat streaming from Ollama /api/chat.
 *   PHASE B — agentic tool-calling over the Moon MCP servers + guardrails.
 *
 * It implements the same Engine contract as ClaudeAgentEngine (see engine.mjs):
 * run() is an async generator yielding the frozen normalized events, and the
 * transport (agent.mjs) turns the generator finishing into a `done` event;
 * an in-flight turn is cancelled by throwing AbortError (as the Claude engine
 * does), so the transport does not emit a duplicate `done`.
 *
 * SAFETY — the Claude guardrails (CLAUDE.md REGOLE) and per-Moon skills load
 * ONLY under the Claude Agent SDK, so they DO NOT apply here. Two mitigations:
 *   1. Prompt — CLAUDE.md is read at runtime and injected as the system prompt
 *      (DRY: one source of truth, works on any engine).
 *   2. Tooling — a DENY-LIST removes state-mutating / outward-acting tools
 *      (send/reply/delete/write/...), so a weak 7B physically cannot send an
 *      email or mutate state even if it ignores the prompt. Relax as trust grows.
 *
 * Ollama is stateless per request; agent.mjs keeps ONE engine instance per chat
 * session, so the running conversation lives in-process on `this.messages`.
 * compact() clears it; cross-restart persistence is a deliberate no-op for v1.
 */

import * as fs from "fs";
import * as path from "path";
import { fileURLToPath } from "url";
import { loadMcpServers, relevantToolNames } from "./engine.mjs";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { SSEClientTransport } from "@modelcontextprotocol/sdk/client/sse.js";
import { StreamableHTTPClientTransport } from "@modelcontextprotocol/sdk/client/streamableHttp.js";

const DEFAULT_BASE_URL = "http://127.0.0.1:11434";
const MAX_TOOL_ITERS = 8;

// Substrings (lowercased) that mark a tool as state-mutating / outward-acting.
// Calibrated against the real Moon tools: Io's send_email, reply_email,
// delete_email*, *_calendar_event, set_email_signature, save_attachment,
// blacklist_add/remove/sweep, reset_index, repair_*, index_message/emails;
// Metis ingest; Ganymede *_commit_*. Read tools (list_/get_/read_/search_/
// index_status/extract_*) are NOT matched. Defense-in-depth with the prompt.
const UNSAFE_TOOL_PATTERNS = [
  "send", "reply", "forward",
  "delete", "remove", "create", "update", "write", "upload", "move",
  "reset", "repair", "ingest", "commit",
  "accept", "decline", "tentative",
  "set_", "save_",
  // GTD (and similar) state mutations
  "edit", "close", "transition", "shift", "log_effort",
  "blacklist_add", "blacklist_remove", "blacklist_sweep",
  "index_message", "index_emails",
];

/** True if a tool name looks state-mutating and must be hidden from Ollama. */
export function isUnsafeTool(name) {
  const n = String(name || "").toLowerCase();
  return UNSAFE_TOOL_PATTERNS.some((p) => n.includes(p));
}

// Minimal by design: a small local model has limited context, so a heavy rules
// prompt cripples it (it starts describing tool calls instead of making them).
// Safety lives in the tool DENYLIST (physical, zero context cost), not here.
const SYSTEM_PROMPT = [
  "Sei l'assistente di JupiterOS.",
  "Rispondi nella lingua dell'utente.",
  "Usa gli strumenti quando servono dati; se non li hai, dillo invece di inventare.",
].join("\n");

/** Locate CLAUDE.md: try cwd, then walk up from this engine file. */
function findClaudeMd(cwd) {
  const candidates = [];
  if (cwd) candidates.push(cwd);
  let dir;
  try { dir = path.dirname(fileURLToPath(import.meta.url)); } catch { dir = null; }
  for (let i = 0; dir && i < 8; i++) {
    candidates.push(dir);
    const parent = path.dirname(dir);
    if (parent === dir) break;
    dir = parent;
  }
  const seen = new Set();
  for (const d of candidates) {
    if (!d || seen.has(d)) continue;
    seen.add(d);
    const p = path.join(d, "CLAUDE.md");
    try { if (fs.existsSync(p)) return fs.readFileSync(p, "utf-8"); } catch { /* ignore */ }
  }
  return "";
}

/** Flatten an MCP CallTool result to text (mirrors the Claude engine). */
function flattenToolResult(res) {
  const content = res?.content;
  if (Array.isArray(content)) {
    const text = content.filter((c) => c && c.type === "text").map((c) => c.text).join("");
    return text || JSON.stringify(content);
  }
  if (typeof content === "string") return content;
  return JSON.stringify(res ?? "");
}

function abortError() {
  const e = new Error("Interrotto");
  e.name = "AbortError";
  return e;
}

export class OllamaEngine {
  constructor(cfg = {}) {
    this.messages = [];
    this.controller = null;
    this.aborted = false;
    this.baseUrl = process.env.OLLAMA_BASE_URL || cfg.baseUrl || DEFAULT_BASE_URL;
    // MCP cache: { clients[], tools[] (ollama fmt), index: Map<name,{client,realName,server}>,
    //             excluded, servers, connected:Set, warned:Set }
    this.mcp = null;
    this.toolSeq = 0;
    this.systemSeeded = false;
  }

  async *run(prompt, options) {
    const { session_id, model, cwd, mcpConfigPath } = options;

    const mcp = await this.#ensureMcp(mcpConfigPath);

    // Static minimal system prompt, seeded once. Tools that appear later (a Moon
    // finishing boot) reach the model via the `tools` param of #chatOnce, not the
    // prompt, so there's nothing to refresh here.
    if (!this.systemSeeded) {
      this.messages.push({ role: "system", content: this.#buildSystemPrompt(cwd) });
      this.systemSeeded = true;
    }
    this.messages.push({ role: "user", content: String(prompt ?? "") });

    this.aborted = false;
    // Route: only the relevant Moon's tools for this message (token economy +
    // fewer random tool calls). null → no relevant Moon → plain chat.
    const allowed = relevantToolNames(prompt, mcp.index);
    const tools = allowed ? mcp.tools.filter((t) => allowed.has(t.function.name)) : [];
    // qwen2.5 (and most local models) only emit tool_calls reliably with
    // stream:false. So: real token-streaming for pure chat, single-shot
    // (non-streamed) calls when tools are in play. The final answer of a tool
    // turn therefore arrives at once rather than token-by-token — acceptable
    // for a first cut; revisit if Ollama fixes streamed tool_calls.
    const useStream = tools.length === 0;
    const usage = { input_tokens: 0, output_tokens: 0, duration_ms: 0 };
    let iters = 0;

    while (true) {
      iters++;
      const turn = yield* this.#chatOnce(session_id, model, tools, useStream);
      if (turn.usage) {
        usage.input_tokens += turn.usage.input_tokens;
        usage.output_tokens += turn.usage.output_tokens;
        usage.duration_ms += turn.usage.duration_ms;
      }

      // Keep history aligned with what the user saw (partial text included),
      // then abort cleanly so the transport does not emit a duplicate `done`.
      if (this.aborted) {
        if (turn.text) this.messages.push({ role: "assistant", content: turn.text });
        throw abortError();
      }

      if (!turn.toolCalls.length) {
        if (turn.text) this.messages.push({ role: "assistant", content: turn.text });
        break;
      }

      // Assistant turn that requested tools (kept for context), then run them.
      this.messages.push({ role: "assistant", content: turn.text || "", tool_calls: turn.toolCalls });

      for (const tc of turn.toolCalls) {
        if (this.aborted) throw abortError();
        const name = tc?.function?.name || "";
        let args = tc?.function?.arguments;
        if (typeof args === "string") { try { args = JSON.parse(args); } catch { /* keep */ } }
        if (args == null || typeof args !== "object") args = {};

        const tool_id = `t${++this.toolSeq}_${name || "tool"}`;
        yield { event: "tool_start", session_id, tool_name: name || "(sconosciuto)", tool_id };
        yield { event: "tool_input_delta", session_id, tool_id, partial_json: JSON.stringify(args) };

        let resultText;
        const entry = this.mcp.index.get(name);
        if (!entry) {
          resultText = `[errore] tool non disponibile: "${name}". Usa solo i tool elencati.`;
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
        this.messages.push({ role: "tool", content: resultText });
      }

      if (this.aborted) throw abortError();
      if (iters >= MAX_TOOL_ITERS) {
        const msg = "\n\n[Limite di passaggi tool raggiunto: mi fermo. Riformula se serve.]";
        yield { event: "text_delta", session_id, text: msg };
        this.messages.push({ role: "assistant", content: msg });
        break;
      }
    }

    // One usage result for the whole turn (feeds the token HUD).
    yield {
      event: "result", session_id, subtype: "success", total_cost_usd: 0,
      usage: { input_tokens: usage.input_tokens, output_tokens: usage.output_tokens, cache_read_tokens: 0 },
      num_turns: iters, duration_ms: usage.duration_ms,
    };
  }

  // One model call. `useStream`=true → NDJSON token streaming (pure chat);
  // false → a single non-streamed JSON (needed for reliable tool_calls).
  // Returns {text, toolCalls, usage}.
  async *#chatOnce(session_id, model, tools, useStream) {
    this.controller = new AbortController();
    const body = { model, messages: this.messages, stream: useStream };
    if (tools && tools.length) body.tools = tools;

    let response;
    try {
      response = await fetch(`${this.baseUrl}/api/chat`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body),
        signal: this.controller.signal,
      });
    } catch (e) {
      this.controller = null;
      if (this.aborted || e?.name === "AbortError") return { text: "", toolCalls: [] };
      throw new Error(
        `Ollama non raggiungibile su ${this.baseUrl} — avvia il servizio (\`ollama serve\`) e verifica il modello. [${e?.message || e}]`,
      );
    }

    if (!response.ok) {
      let detail = "";
      try { detail = (await response.text()).trim(); } catch { /* ignore */ }
      this.controller = null;
      if (response.status === 404) {
        throw new Error(`Modello "${model}" non trovato — esegui: \`ollama pull ${model}\`.${detail ? ` [${detail}]` : ""}`);
      }
      throw new Error(`Ollama ha risposto ${response.status} ${response.statusText}${detail ? `: ${detail}` : ""}`);
    }

    // ── Non-streamed: one JSON object (reliable tool_calls on local models) ──
    if (!useStream) {
      let j;
      try {
        j = await response.json();
      } catch (e) {
        this.controller = null;
        if (this.aborted || e?.name === "AbortError") return { text: "", toolCalls: [] };
        throw e;
      }
      this.controller = null;
      if (j?.error) throw new Error(`Ollama: ${j.error}`);
      const text = j?.message?.content || "";
      if (text && !this.aborted) yield { event: "text_delta", session_id, text };
      const toolCalls = Array.isArray(j?.message?.tool_calls) ? j.message.tool_calls : [];
      const usage = {
        input_tokens: j?.prompt_eval_count || 0,
        output_tokens: j?.eval_count || 0,
        duration_ms: Math.round((j?.total_duration || 0) / 1e6),
      };
      return { text, toolCalls, usage };
    }

    // ── Streamed: NDJSON, progressive text_delta ──
    let text = "";
    const toolCalls = [];
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
          if (!line) continue;
          let obj;
          try { obj = JSON.parse(line); } catch { continue; }
          if (obj.error) throw new Error(`Ollama: ${obj.error}`);

          const piece = obj.message?.content;
          if (piece) { text += piece; yield { event: "text_delta", session_id, text: piece }; }

          const tcs = obj.message?.tool_calls;
          if (Array.isArray(tcs) && tcs.length) toolCalls.push(...tcs);

          if (obj.done) {
            usage = {
              input_tokens: obj.prompt_eval_count || 0,
              output_tokens: obj.eval_count || 0,
              duration_ms: Math.round((obj.total_duration || 0) / 1e6),
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
    return { text, toolCalls, usage };
  }

  // Connect to the Moon MCP servers, retrying any not-yet-connected server on
  // EACH turn (so tools appear once a Moon finishes starting up — Moons restart
  // with the app, so the first message often races their boot). Already-connected
  // servers are never re-dialed; the connection set is cached on the instance.
  async #ensureMcp(mcpConfigPath) {
    if (!this.mcp) {
      let servers = {};
      if (mcpConfigPath) {
        try { servers = loadMcpServers(mcpConfigPath); } catch { servers = {}; }
      }
      this.mcp = { clients: [], tools: [], index: new Map(), excluded: 0, servers, connected: new Set(), warned: new Set() };
    }
    const m = this.mcp;
    const servers = m.servers || {};
    const connected = m.connected || (m.connected = new Set());
    const warned = m.warned || (m.warned = new Set());
    let changed = false;

    for (const [serverName, def] of Object.entries(servers)) {
      if (connected.has(serverName)) continue; // already connected this session
      if (!def.url || (def.type !== "sse" && def.type !== "http")) {
        if (!warned.has(serverName)) {
          warned.add(serverName);
          process.stderr.write(`[SIDECAR] ollama MCP: salto ${serverName} (transport ${def.type} non supportato)\n`);
        }
        continue;
      }
      let client = null;
      let transport = null;
      try {
        transport = def.type === "http"
          ? new StreamableHTTPClientTransport(new URL(def.url))
          : new SSEClientTransport(new URL(def.url));
        client = new Client({ name: "jupiteros-ollama", version: "0.1.0" }, { capabilities: {} });
        await client.connect(transport);
        const listed = await client.listTools();
        const serverTools = (listed && listed.tools) || [];
        m.clients.push(client);
        connected.add(serverName);
        changed = true;
        for (const t of serverTools) {
          if (isUnsafeTool(t.name)) { m.excluded++; continue; }
          if (m.index.has(t.name)) continue; // first server wins on a name clash
          m.index.set(t.name, { client, realName: t.name, server: serverName });
          m.tools.push({
            type: "function",
            function: {
              name: t.name,
              description: t.description || "",
              parameters: t.inputSchema || { type: "object", properties: {} },
            },
          });
        }
        process.stderr.write(`[SIDECAR] ollama MCP: ${serverName} connesso (${serverTools.length} tool)\n`);
      } catch (e) {
        // Close the half-open transport so a down moon doesn't leave an
        // EventSource retrying forever. Keep it OUT of `connected` so the next
        // turn retries it (handles Moons that are still booting). Log once.
        try { await client?.close?.(); } catch { /* ignore */ }
        try { await transport?.close?.(); } catch { /* ignore */ }
        if (!warned.has(serverName)) {
          warned.add(serverName);
          process.stderr.write(`[SIDECAR] ollama MCP: ${serverName} non raggiungibile — riprovo al prossimo turno (${e?.message || e})\n`);
        }
      }
    }
    if (changed) {
      process.stderr.write(`[SIDECAR] ollama MCP: ${m.tools.length} tool esposti, ${m.excluded} esclusi per sicurezza\n`);
    }
    return m;
  }

  #buildSystemPrompt(cwd) {
    // The full CLAUDE.md (~1900 tokens) is faithful but saturates a small model's
    // context, so it is strictly opt-in; the default stays minimal.
    if (process.env.JUPITER_OLLAMA_FULL_GUARDRAILS) {
      const guard = findClaudeMd(cwd);
      if (guard) return `${SYSTEM_PROMPT}\n\n--- REGOLE INVARIANTI (CLAUDE.md) ---\n${guard}`;
    }
    return SYSTEM_PROMPT;
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
  setResumeToken(_token) { /* no-op: in-process history only (see class docstring) */ }

  compact() {
    this.messages = [];
    this.systemSeeded = false;
  }
}
