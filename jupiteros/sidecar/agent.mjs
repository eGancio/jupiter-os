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
 * Commands (stdin):  create_session, send, stop, dispose, set_model, compact_session
 * Events (stdout):   ready, session_created, text_delta, thinking_*, tool_*,
 *                    result, system_init, done, error, session_compacted
 */

import * as readline from "readline";
import { createEngine, registerEngine } from "./engines/engine.mjs";
import { ClaudeAgentEngine } from "./engines/claude.mjs";

// Register the built-in engines. Adding a new backend (Phase 0.3: generic /
// Ollama) is a single registerEngine() line + a new engine file.
registerEngine("claude", (cfg) => new ClaudeAgentEngine(cfg));

// ── Per-session state: { engine, options } ───────────────────
// options = neutral per-session config the transport passes to engine.run.
const sessions = new Map();

// ── Emit JSON events to stdout ───────────────────────────────

function emit(obj) {
  process.stdout.write(JSON.stringify(obj) + "\n");
}

function emitError(sessionId, error) {
  emit({ event: "error", session_id: sessionId || "", error: String(error) });
}

// ── Command handlers ─────────────────────────────────────────

function handleCreateSession(cmd) {
  const cfg = {
    model: cmd.model || "sonnet",
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
    images: cmd.images,
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
    case "stop":
      handleStop(cmd);
      break;
    case "dispose":
      handleDispose(cmd);
      break;
    case "set_model":
      handleSetModel(cmd);
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

process.on("uncaughtException", (err) => {
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
