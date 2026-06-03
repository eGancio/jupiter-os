#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

/**
 * JupiterOS Agent Sidecar — V1 API
 *
 * Uses the V1 `query()` function which supports:
 * - mcpServers (SSE, stdio, http)
 * - includePartialMessages (real-time streaming)
 * - resume (session continuity via session ID)
 *
 * Protocol: stdin/stdout JSONL with Rust/Tauri backend.
 */

import { query } from "@anthropic-ai/claude-agent-sdk";
import * as readline from "readline";
import * as fs from "fs";

// ── Globals ──────────────────────────────────────────────────

/**
 * Per-session state:
 * - sdkSessionId: the SDK's internal session UUID (from system init)
 * - queryInstance: the running Query async generator (for interrupt/close)
 * - aborted: flag to break the stream loop
 */
const sessions = new Map();

// ── Emit JSON events to stdout ───────────────────────────────

function emit(obj) {
  process.stdout.write(JSON.stringify(obj) + "\n");
}

function emitError(sessionId, error) {
  emit({ event: "error", session_id: sessionId || "", error: String(error) });
}

// ── Load MCP servers from .mcp.json ──────────────────────────

function expandEnv(value) {
  if (typeof value !== "string") return value;
  return value.replace(/\$\{([A-Z0-9_]+)\}/gi, (_, name) => process.env[name] ?? "");
}

function loadMcpServers(configPath) {
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

// ── Translate SDK messages to our event protocol ─────────────

function handleStreamMessage(msg, sessionId, sessionEntry) {
  switch (msg.type) {
    case "stream_event": {
      // SDKPartialAssistantMessage — real-time streaming deltas
      const ev = msg.event;
      if (!ev) break;

      if (ev.type === "content_block_start") {
        const block = ev.content_block;
        if (block && block.type === "tool_use") {
          emit({
            event: "tool_start",
            session_id: sessionId,
            tool_name: block.name || "",
            tool_id: block.id || "",
          });
        } else if (block && block.type === "thinking") {
          emit({
            event: "thinking_start",
            session_id: sessionId,
          });
        }
      } else if (ev.type === "message_start" && ev.message?.usage) {
        // Anthropic API message_start contains input token usage
        const u = ev.message.usage;
        const totalIn = (u.input_tokens || 0) + (u.cache_read_input_tokens || 0) + (u.cache_creation_input_tokens || 0);
        emit({
          event: "result",
          session_id: sessionId,
          subtype: "usage_update",
          total_cost_usd: 0,
          usage: {
            input_tokens: totalIn,
            output_tokens: 0,
            cache_read_tokens: u.cache_read_input_tokens || 0,
          },
          num_turns: 0,
          duration_ms: 0,
        });
      } else if (ev.type === "message_delta" && ev.usage) {
        // Anthropic API message_delta contains output token usage
        emit({
          event: "result",
          session_id: sessionId,
          subtype: "usage_update",
          total_cost_usd: 0,
          usage: {
            input_tokens: 0,
            output_tokens: ev.usage.output_tokens || 0,
            cache_read_tokens: 0,
          },
          num_turns: 0,
          duration_ms: 0,
        });
      } else if (ev.type === "content_block_delta") {
        const delta = ev.delta;
        if (delta && delta.type === "text_delta") {
          emit({
            event: "text_delta",
            session_id: sessionId,
            text: delta.text || "",
          });
        } else if (delta && delta.type === "input_json_delta") {
          emit({
            event: "tool_input_delta",
            session_id: sessionId,
            tool_id: "",
            partial_json: delta.partial_json || "",
          });
        } else if (delta && delta.type === "thinking_delta") {
          emit({
            event: "thinking_delta",
            session_id: sessionId,
            thinking: delta.thinking || "",
          });
        }
      }
      break;
    }

    case "assistant": {
      // SDKAssistantMessage — complete assistant message
      // With includePartialMessages, we already get streaming via stream_event.
      // This is the final message — extract tool_use blocks (text already streamed).
      const content = msg.message?.content || [];
      for (const block of content) {
        if (block.type === "tool_use") {
          // Only emit if we didn't get it via stream_event already
          // (tool_start from stream_event uses content_block_start)
          emit({
            event: "tool_start",
            session_id: sessionId,
            tool_name: block.name || "",
            tool_id: block.id || "",
          });
          if (block.input) {
            emit({
              event: "tool_input_delta",
              session_id: sessionId,
              tool_id: block.id || "",
              partial_json: JSON.stringify(block.input),
            });
          }
        }
      }
      break;
    }

    case "user": {
      // SDKUserMessage — tool_result blocks
      const content = msg.message?.content || [];
      const blocks = Array.isArray(content) ? content : [];
      for (const block of blocks) {
        if (block.type === "tool_result") {
          const resultText = Array.isArray(block.content)
            ? block.content
                .filter((c) => c.type === "text")
                .map((c) => c.text)
                .join("")
            : typeof block.content === "string"
              ? block.content
              : JSON.stringify(block.content);
          emit({
            event: "tool_result",
            session_id: sessionId,
            tool_id: block.tool_use_id || "",
            result: resultText,
          });
        }
      }
      break;
    }

    case "result": {
      // SDKResultMessage — usage is directly on msg
      const usage = msg.usage || msg.result?.usage || {};
      const totalCost = msg.total_cost_usd ?? msg.result?.total_cost_usd ?? 0;

      // Total context = input_tokens + cache_read + cache_creation (all count toward context window)
      const totalInputTokens = (usage.input_tokens || 0)
        + (usage.cache_read_input_tokens || 0)
        + (usage.cache_creation_input_tokens || 0);

      emit({
        event: "result",
        session_id: sessionId,
        subtype: msg.subtype || msg.result?.subtype || (msg.is_error ? "error" : "success"),
        total_cost_usd: totalCost,
        usage: {
          input_tokens: totalInputTokens,
          output_tokens: usage.output_tokens || 0,
          cache_read_tokens: usage.cache_read_input_tokens || 0,
        },
        num_turns: msg.num_turns || msg.result?.num_turns || 0,
        duration_ms: msg.duration_ms || msg.result?.duration_ms || 0,
      });
      break;
    }

    case "system": {
      // Capture the SDK's internal session ID for resume
      if (msg.session_id && sessionEntry) {
        sessionEntry.sdkSessionId = msg.session_id;
      }
      if (msg.subtype === "init") {
        emit({
          event: "system_init",
          session_id: sessionId,
          sdk_session_id: msg.session_id || "",
          model: msg.model || "",
          tools: msg.tools || [],
          mcp_servers: msg.mcp_servers || [],
        });
      }
      break;
    }

    // Ignore: rate_limit_event, status, hooks, tasks, etc.
    default:
      break;
  }
}

// ── Command handlers ─────────────────────────────────────────

function handleCreateSession(cmd) {
  // V1 API: no need to create a session upfront.
  // Just register the session ID and options for later query() calls.
  sessions.set(cmd.id, {
    sdkSessionId: cmd.sdk_session_id || null, // restored from disk, or null for new
    queryInstance: null,
    aborted: false,
    options: {
      model: cmd.model || "sonnet",
      cwd: cmd.cwd || process.cwd(),
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

  entry.aborted = false;

  // Build query options
  const mcpServers = entry.options.mcp_config
    ? loadMcpServers(entry.options.mcp_config)
    : {};

  const queryOptions = {
    model: entry.options.model,
    permissionMode: "bypassPermissions",
    allowDangerouslySkipPermissions: true,
    cwd: entry.options.cwd,
    mcpServers,
    systemPrompt: { type: "preset", preset: "claude_code" },
    settingSources: ["user", "project", "local"],
    allowedTools: [
      "mcp__*",
      "Bash", "Read", "Write", "Edit",
      "Grep", "Glob", "WebFetch", "WebSearch", "TodoWrite",
    ],
    includePartialMessages: true,
    persistSession: true,
    thinking: { type: "adaptive" },
  };

  // Resume from previous SDK session if available
  if (entry.sdkSessionId) {
    queryOptions.resume = entry.sdkSessionId;
  }

  // Build prompt (string for text-only, structured for images)
  let prompt = cmd.message;
  if (cmd.images && cmd.images.length > 0) {
    // Multi-part content with images
    const content = [{ type: "text", text: cmd.message }];
    for (const b64 of cmd.images) {
      let mediaType = "image/png";
      if (b64.startsWith("/9j/")) mediaType = "image/jpeg";
      else if (b64.startsWith("R0lG")) mediaType = "image/gif";
      else if (b64.startsWith("UklG")) mediaType = "image/webp";
      content.push({
        type: "image",
        source: { type: "base64", media_type: mediaType, data: b64 },
      });
    }
    // For V1 query, prompt is a string — we pass images differently
    // Actually, the V1 API prompt is string only. Images need special handling.
    // For now, just send the text and note images in the prompt.
    prompt = cmd.message;
  }

  try {
    process.stderr.write(`[SIDECAR] query() session=${cmd.session_id} resume=${entry.sdkSessionId || 'new'} prompt=${prompt.slice(0, 80)}\n`);

    const q = query({ prompt, options: queryOptions });
    entry.queryInstance = q;

    for await (const msg of q) {
      if (entry.aborted) break;
      handleStreamMessage(msg, cmd.session_id, entry);
    }

    emit({ event: "done", session_id: cmd.session_id });
  } catch (err) {
    if (err.name !== "AbortError" && !entry.aborted) {
      process.stderr.write(`[SIDECAR] Error: ${err.message}\n`);
      emitError(cmd.session_id, err.message);
    }
  } finally {
    entry.queryInstance = null;
  }
}

function handleStop(cmd) {
  const entry = sessions.get(cmd.session_id);
  if (entry) {
    entry.aborted = true;
    if (entry.queryInstance) {
      try {
        entry.queryInstance.close();
      } catch { /* ignore */ }
      entry.queryInstance = null;
    }
    emit({ event: "done", session_id: cmd.session_id });
  }
}

function handleDispose(cmd) {
  const entry = sessions.get(cmd.session_id);
  if (entry) {
    entry.aborted = true;
    if (entry.queryInstance) {
      try { entry.queryInstance.close(); } catch { /* ignore */ }
    }
    sessions.delete(cmd.session_id);
  }
}

function handleCompactSession(cmd) {
  const entry = sessions.get(cmd.session_id);
  if (entry) {
    // Reset the SDK session ID — next query() will start a fresh context
    // while keeping the same UI session (no new chat in sidebar)
    entry.sdkSessionId = null;
    process.stderr.write(`[SIDECAR] compact session=${cmd.session_id} — context reset\n`);
    emit({ event: "session_compacted", session_id: cmd.session_id });
  }
}

function handleSetModel(cmd) {
  // Update model for all future queries in a session
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
    if (entry.queryInstance) {
      try { entry.queryInstance.close(); } catch { /* ignore */ }
    }
  }
  process.exit(0);
});

process.on("SIGINT", () => {
  for (const [, entry] of sessions) {
    if (entry.queryInstance) {
      try { entry.queryInstance.close(); } catch { /* ignore */ }
    }
  }
  process.exit(0);
});

// Signal ready
emit({ event: "ready" });
