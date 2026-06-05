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
