#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

/**
 * JupiterOS Agent Sidecar — transport layer.
 *
 * Owns the stdin/stdout JSONL protocol with the Rust/Tauri backend and delegates
 * the actual agent runtime to a pluggable Engine (see engines/engine.mjs). The
 * Engine yields normalized events that ARE the protocol events, so the Rust-side
 * JSONL contract is identical across engines.
 *
 * Commands (stdin):  create_session, send, stop, dispose, set_model,
 *                    set_permission_mode, compact_session
 * Events (stdout):   ready, session_created, text_delta, thinking_*, tool_*,
 *                    result, system_init, done, error, session_compacted
 */

import * as readline from "readline";
import { createEngine, registerEngine } from "./engines/engine.mjs";
import { ClaudeAgentEngine, oneShot } from "./engines/claude.mjs";
import { OllamaEngine } from "./engines/ollama.mjs";
import { GeminiEngine } from "./engines/gemini.mjs";
import { OpenAICompatEngine } from "./engines/openai-compat.mjs";

// Register the built-in engines. Adding a new backend is a single
// registerEngine() line + a new engine file. OpenAI-compatible providers
// (Groq, OpenRouter, …) share one engine, parameterized by baseUrl + key env.
registerEngine("claude", (cfg) => new ClaudeAgentEngine(cfg));
// ollama: maxToolTokens 2000 (non 4000) — modello locale su CPU: la prompt eval
// costa secondi per migliaio di token, e num_ctx è 8192; metà del vecchio budget
// se ne andava in schemi prima ancora di domanda e risultati.
registerEngine("ollama", (cfg) => new OllamaEngine({ ...cfg, maxToolTokens: 2000 }));
registerEngine("gemini", (cfg) => new GeminiEngine({ ...cfg, maxToolTokens: 20000 }));
// groq: maxToolTokens 3000 (non 6000) — con più Moon attivi (io+gtd = 24 tool ≈ 6k
// token di schemi) il menu intero saturava da solo metà del TPM free (12k/min) a
// OGNI iterazione → 429 dopo i tool. Sotto budget il router per keyword restringe
// ai tool del Moon pertinente.
registerEngine("groq", (cfg) => new OpenAICompatEngine(cfg, { name: "groq", baseUrl: "https://api.groq.com/openai/v1", keyEnvs: ["GROQ_API_KEY"], maxToolTokens: 3000 }));
registerEngine("openrouter", (cfg) => new OpenAICompatEngine(cfg, { name: "openrouter", baseUrl: "https://openrouter.ai/api/v1", keyEnvs: ["OPENROUTER_API_KEY"], maxToolTokens: 20000 }));
// dwarfstar: server locale OpenAI-compatible di antirez/ds4 (DeepSeek V4, offline).
// baseUrl include /v1 perché l'engine fa append di /chat/completions; nessuna auth
// (defaultKey è un placeholder che ds4 ignora). Override host/porta via env
// DWARFSTAR_BASE_URL. budget tool largo: modello locale ad alto contesto.
registerEngine("dwarfstar", (cfg) => new OpenAICompatEngine(cfg, { name: "dwarfstar", baseUrl: process.env.DWARFSTAR_BASE_URL || "http://127.0.0.1:8000/v1", keyEnvs: ["DWARFSTAR_API_KEY"], defaultKey: "dsv4-local", maxToolTokens: 20000 }));
// localops: llama-server locale (llama.cpp, CPU) con Qwen3-30B-A3B per le
// operazioni. Il server è lanciato come daemon dal .mcp.json (porta 8080).
// budget tool stretto come ollama: su CPU ogni token di schema è prefill;
// l'eval Fase 0 (scripts/localops-eval) mostra che la superficie ristretta è
// ciò che rende il modello affidabile, non un limite da allargare.
registerEngine("localops", (cfg) => new OpenAICompatEngine(cfg, { name: "localops", baseUrl: process.env.LOCALOPS_BASE_URL || "http://127.0.0.1:8080/v1", keyEnvs: ["LOCALOPS_API_KEY"], defaultKey: "localops-local", maxToolTokens: 2500 }));

// ── Per-session state: { engine, options } ───────────────────
// options = neutral per-session config the transport passes to engine.run.
const sessions = new Map();

// ── Emit JSON events to stdout ───────────────────────────────

function emit(obj) {
  // A failed stdout write must NEVER crash the whole sidecar (and with it every
  // session). Without this guard a single EPIPE bubbled up to uncaughtException
  // and killed the process — turning every later message into "broken pipe".
  try {
    process.stdout.write(JSON.stringify(obj) + "\n");
  } catch (err) {
    process.stderr.write(`[SIDECAR] emit failed: ${err && err.message}\n`);
  }
}

function emitError(sessionId, error) {
  emit({ event: "error", session_id: sessionId || "", error: String(error) });
}

// ── Command handlers ─────────────────────────────────────────

function handleCreateSession(cmd) {
  const cfg = {
    model: cmd.model || "opus",
    cwd: cmd.cwd || process.cwd(),
    mcpConfigPath: cmd.mcp_config || "",
  };
  const engine = createEngine(cmd.engine || "claude", cfg);

  // Restored from disk, or null for new
  if (cmd.sdk_session_id) {
    engine.setResumeToken(cmd.sdk_session_id);
  }

  sessions.set(cmd.id, {
    engine,
    options: {
      model: cfg.model,
      cwd: cfg.cwd,
      mcp_config: cmd.mcp_config || "",
      // Permission mode (Claude only; other engines ignore it). Default "auto".
      permission_mode: cmd.permission_mode || "auto",
    },
  });
  emit({ event: "session_created", session_id: cmd.id });
}

async function handleSend(cmd) {
  const entry = sessions.get(cmd.session_id);
  if (!entry) {
    emitError(cmd.session_id, "Session not found");
    return;
  }

  const options = {
    session_id: cmd.session_id,
    model: entry.options.model,
    cwd: entry.options.cwd,
    mcpConfigPath: entry.options.mcp_config,
    permissionMode: entry.options.permission_mode,
    images: cmd.images,
    // Scudo PII: {placeholder → valore vero}, solo per ripristinare gli
    // argomenti dei tool che girano in locale. Mai inviato al cloud.
    piiMapping: cmd.pii_mapping,
  };

  try {
    for await (const ev of entry.engine.run(cmd.message, options)) {
      emit(ev);
    }
    emit({ event: "done", session_id: cmd.session_id });
  } catch (err) {
    if (err.name !== "AbortError") {
      process.stderr.write(`[SIDECAR] Error: ${err.message}\n`);
      emitError(cmd.session_id, err.message);
    }
  }
}

function handleStop(cmd) {
  const entry = sessions.get(cmd.session_id);
  if (entry) {
    entry.engine.interrupt();
    emit({ event: "done", session_id: cmd.session_id });
  }
}

function handleDispose(cmd) {
  const entry = sessions.get(cmd.session_id);
  if (entry) {
    entry.engine.close();
    sessions.delete(cmd.session_id);
  }
}

function handleCompactSession(cmd) {
  const entry = sessions.get(cmd.session_id);
  if (entry) {
    // Reset context while keeping the same UI session (no new chat in sidebar)
    entry.engine.compact();
    process.stderr.write(`[SIDECAR] compact session=${cmd.session_id} — context reset\n`);
    emit({ event: "session_compacted", session_id: cmd.session_id });
  }
}

function handleSetModel(cmd) {
  // Update model for all future runs in a session (passed per-turn to engine.run)
  const entry = sessions.get(cmd.session_id);
  if (entry) {
    entry.options.model = cmd.model;
  }
}

// ── Background auto-titler (cheap, Haiku) ────────────────────
// The Rust backend fires `classify` when a chat goes idle; we run a tool-less
// one-shot and emit the raw JSON back for Rust to parse and persist.
const CLASSIFY_SYSTEM = `Dai un titolo a una conversazione di chat.

Rispondi ESCLUSIVAMENTE con un oggetto JSON, senza testo attorno, in questa forma:
{"title": string}

"title": un titolo conciso della chat, 3-5 parole, in italiano, senza virgolette.`;

async function handleClassify(cmd) {
  try {
    const result = await oneShot(cmd.prompt, cmd.model || "haiku", CLASSIFY_SYSTEM);
    emit({ event: "classify_result", session_id: cmd.session_id, result });
  } catch (err) {
    process.stderr.write(`[SIDECAR] classify error: ${err && err.message}\n`);
    // Emit an empty result so Rust stamps classified_at and doesn't retry forever.
    emit({ event: "classify_result", session_id: cmd.session_id, result: "" });
  }
}

function handleSetPermissionMode(cmd) {
  // Update permission mode for all future runs in a session. Read per-turn by
  // the Claude engine; other engines ignore it.
  const entry = sessions.get(cmd.session_id);
  if (entry) {
    entry.options.permission_mode = cmd.permission_mode || "auto";
  }
}

// ── Main stdin reader loop ───────────────────────────────────

const rl = readline.createInterface({ input: process.stdin, terminal: false });

rl.on("line", async (line) => {
  const trimmed = line.trim();
  if (!trimmed) return;

  let cmd;
  try {
    cmd = JSON.parse(trimmed);
  } catch {
    emitError("", `Invalid JSON: ${trimmed.slice(0, 100)}`);
    return;
  }

  switch (cmd.cmd) {
    case "create_session":
      handleCreateSession(cmd);
      break;
    case "send":
      // Run in background so we can process stop commands concurrently
      handleSend(cmd).catch((err) => emitError(cmd.session_id, err.message));
      break;
    case "classify":
      // Background classification: never blocks the send/stop loop.
      handleClassify(cmd).catch((err) =>
        process.stderr.write(`[SIDECAR] classify dispatch error: ${err && err.message}\n`),
      );
      break;
    case "stop":
      handleStop(cmd);
      break;
    case "dispose":
      handleDispose(cmd);
      break;
    case "set_model":
      handleSetModel(cmd);
      break;
    case "set_permission_mode":
      handleSetPermissionMode(cmd);
      break;
    case "compact_session":
      handleCompactSession(cmd);
      break;
    default:
      emitError("", `Unknown command: ${cmd.cmd}`);
  }
});

rl.on("close", () => {
  process.stderr.write("[SIDECAR] stdin closed (readline close event) — exiting\n");
  process.exit(0);
});

process.on("exit", (code) => {
  process.stderr.write(`[SIDECAR] process.exit called with code ${code}\n`);
});

// A broken stdout pipe (the Rust backend stopped reading, or is replacing us)
// is delivered asynchronously as a stream 'error' event. Handle it here so it
// never reaches uncaughtException: EPIPE means the parent's read end is gone —
// exit CLEANLY so the backend can respawn a fresh sidecar instead of seeing a
// crash. With this + the emit() guard, one bad write no longer kills the chat.
process.stdout.on("error", (err) => {
  if (err && (err.code === "EPIPE" || err.code === "ERR_STREAM_DESTROYED")) {
    process.stderr.write("[SIDECAR] stdout pipe closed — exiting cleanly\n");
    process.exit(0);
  }
  process.stderr.write(`[SIDECAR] stdout error: ${err && err.message}\n`);
});

process.on("uncaughtException", (err) => {
  // Don't die noisily on a pipe error — let the clean-exit / respawn path win.
  if (err && (err.code === "EPIPE" || err.code === "ERR_STREAM_DESTROYED")) {
    process.stderr.write("[SIDECAR] uncaughtException (pipe) — exiting cleanly\n");
    process.exit(0);
  }
  process.stderr.write(`[SIDECAR] uncaughtException: ${err.stack}\n`);
  process.exit(1);
});

process.on("unhandledRejection", (reason) => {
  process.stderr.write(`[SIDECAR] unhandledRejection: ${reason}\n`);
});

// Handle process signals gracefully
process.on("SIGTERM", () => {
  for (const [, entry] of sessions) {
    entry.engine.close();
  }
  process.exit(0);
});

process.on("SIGINT", () => {
  for (const [, entry] of sessions) {
    entry.engine.close();
  }
  process.exit(0);
});

// Signal ready
emit({ event: "ready" });
