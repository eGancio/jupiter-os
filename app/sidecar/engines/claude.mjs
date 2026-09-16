// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

/**
 * ClaudeAgentEngine — the Engine implementation backed by the Anthropic Claude
 * Agent SDK (`query()`). This is the extraction of the original agent.mjs logic:
 * the query loop and the SDK-message → normalized-event translation that used to
 * live in `handleStreamMessage` now live here. Behavior is byte-for-byte identical
 * to the pre-seam sidecar for Claude.
 *
 * Claude-specific option assembly (claude_code preset, adaptive thinking,
 * allowedTools, permissionMode, resume gating) lives INSIDE this engine — exactly
 * what a future GenericEngine will not share.
 */

import { query } from "@anthropic-ai/claude-agent-sdk";
import { loadMcpServers, waitForMcpServers } from "./engine.mjs";
import { mkdtempSync, writeFileSync, rmSync } from "fs";
import { tmpdir } from "os";
import { join } from "path";

const ALLOWED_TOOLS = [
  "mcp__*",
  "Bash", "Read", "Write", "Edit",
  "Grep", "Glob", "WebFetch", "WebSearch", "TodoWrite",
];

// Single real mode for now: "Auto mode" = do everything, no prompts
// (SDK 'bypassPermissions'). Plan / interactive modes are intentionally NOT
// exposed yet — they require an approval-popup UI (Phase 2). Mapping every
// value to bypass guarantees the chat never gets stuck waiting on a popup
// that doesn't exist.
function toSdkPermissionMode(_mode) {
  return "bypassPermissions";
}

// Sniff the image MIME type from the first bytes of a base64 string. The GUI
// strips the `data:<mime>;base64,` prefix before sending, so the media_type
// the Anthropic API requires is no longer self-described — recover it from the
// magic bytes. Covers every format the clipboard/file picker produce; defaults
// to PNG (the screenshot format) when unrecognized.
function sniffImageMediaType(base64) {
  if (base64.startsWith("/9j/")) return "image/jpeg";
  if (base64.startsWith("iVBORw0KGgo")) return "image/png";
  if (base64.startsWith("R0lGOD")) return "image/gif";
  if (base64.startsWith("UklGR")) return "image/webp"; // RIFF container
  if (base64.startsWith("Qk")) return "image/bmp";
  return "image/png";
}

// Extension for a sniffed media type, so the temp file is named in a way the
// Read tool recognizes as an image (it renders png/jpg/gif/webp/bmp visually).
function extForMediaType(mt) {
  switch (mt) {
    case "image/jpeg": return "jpg";
    case "image/gif": return "gif";
    case "image/webp": return "webp";
    case "image/bmp": return "bmp";
    default: return "png";
  }
}

// Write pasted images to a fresh temp dir and return { dir, prompt }.
//
// Rationale: inlining base64 forced the SDK onto the streaming-stdin path
// (`--input-format stream-json`), on which the child `claude` CLI intermittently
// died at startup with the opaque "exited with code 1" — a crash the plain
// string-prompt (`--print`) path never showed. Handing the model file PATHS lets
// it open them with the Read tool (native Claude Code image handling), keeps the
// robust string-prompt path, and stops pushing MB of base64 through two stdin
// pipes. The caller rm -rf's `dir` after the turn.
function buildImageFilePrompt(text, images) {
  const dir = mkdtempSync(join(tmpdir(), "jupiter-imgs-"));
  const paths = [];
  images.forEach((data, i) => {
    const p = join(dir, `paste-${i + 1}.${extForMediaType(sniffImageMediaType(data))}`);
    writeFileSync(p, Buffer.from(data, "base64"));
    paths.push(p);
  });
  const list = paths.map((p) => `- ${p}`).join("\n");
  const note =
    `[L'utente ha allegato ${paths.length} immagine/i. ` +
    `Aprile con il tool Read prima di rispondere:]\n${list}`;
  const prompt = text ? `${text}\n\n${note}` : note;
  return { dir, prompt };
}

/**
 * Fire a single, tool-less, non-persisted completion and return the plain text.
 *
 * Used by the background chat classifier: it must be CHEAP and isolated — no
 * MCP Moons, no skills, no project settings (which would load the guardrail
 * CLAUDE.md and every skill), no session resume, one turn only. Reuses the same
 * `claude` CLI auth the chat sessions use, so it needs no extra API key.
 */
export async function oneShot(prompt, model, systemPrompt) {
  const queryOptions = {
    model: model || "haiku",
    permissionMode: "bypassPermissions",
    allowDangerouslySkipPermissions: true,
    mcpServers: {},
    allowedTools: [],
    // Load NOTHING from the project/user config: a bare classifier, not an agent.
    settingSources: [],
    includePartialMessages: false,
    persistSession: false,
    maxTurns: 1,
  };
  if (systemPrompt) queryOptions.systemPrompt = systemPrompt;

  let text = "";
  const q = query({ prompt, options: queryOptions });
  for await (const msg of q) {
    if (msg.type === "assistant") {
      const content = msg.message?.content || [];
      for (const block of content) {
        if (block.type === "text") text += block.text || "";
      }
    }
  }
  return text.trim();
}

export class ClaudeAgentEngine {
  constructor(cfg = {}) {
    // Per-session SDK state (formerly the sessions Map entry fields).
    this.sdkSessionId = null;
    this.queryInstance = null;
    this.aborted = false;
    this.mcpWaitDone = false;
    // tool_use ids already announced via stream_event/content_block_start.
    // The complete "assistant" message repeats the same blocks: without this
    // the UI would get a duplicate tool_start that never receives a result.
    this.seenToolIds = new Set();
    // Open tool_use blocks keyed by `${parent_tool_use_id}:${block index}` →
    // tool_id. input_json_delta events only carry the block index; with
    // parallel subagents "append to the last tool" would hit the wrong tool,
    // so we resolve the real tool_id here.
    this.openToolBlocks = new Map();
  }

  /**
   * Run one turn. Yields normalized events (the exact objects the transport
   * emits). Errors propagate to the transport; the transport decides done/error.
   */
  async *run(prompt, options) {
    const { session_id, model, cwd, mcpConfigPath, permissionMode, images, piiMapping } = options;

    const mcpServers = mcpConfigPath ? loadMcpServers(mcpConfigPath) : {};

    // Primo turno della sessione: aspetta (max 5s) che i Moon finiscano di
    // bootare — l'SDK connette gli MCP all'avvio della query e non riprova nel
    // turno; senza l'attesa il primo messaggio dopo l'avvio resta senza tool.
    if (!this.mcpWaitDone) {
      this.mcpWaitDone = true;
      await waitForMcpServers(mcpServers);
    }

    const queryOptions = {
      model,
      permissionMode: toSdkPermissionMode(permissionMode),
      allowDangerouslySkipPermissions: true,
      cwd,
      mcpServers,
      systemPrompt: { type: "preset", preset: "claude_code" },
      settingSources: ["user", "project", "local"],
      allowedTools: ALLOWED_TOOLS,
      skills: "all",
      includePartialMessages: true,
      persistSession: true,
      thinking: { type: "adaptive" },
      // Capture the child `claude` CLI's stderr. Without this the SDK discards
      // it, so a CLI that dies at startup (e.g. on an image/streaming-input
      // turn) surfaces only the opaque "Claude Code process exited with code 1"
      // with zero cause. Tagging it CLI-STDERR routes it into the same
      // .claude-gui-debug.log the Rust side captures from our stderr, so the
      // real reason is on record the next time it crashes.
      stderr: (line) => {
        process.stderr.write(`[CLI-STDERR ${session_id}] ${line}\n`);
      },
    };

    // Scudo PII: il modello vede placeholder, ma i tool girano IN LOCALE e
    // devono lavorare sui valori veri — ripristinali negli argomenti prima
    // dell'esecuzione. Hook PreToolUse con updatedInput, NON canUseTool:
    // sotto bypassPermissions l'SDK salta canUseTool (verificato), mentre gli
    // hook di lifecycle girano sempre.
    if (piiMapping && Object.keys(piiMapping).length > 0) {
      const restoreDeep = (v) => {
        if (typeof v === "string") {
          let s = v;
          for (const [ph, real] of Object.entries(piiMapping)) s = s.split(ph).join(real);
          return s;
        }
        if (Array.isArray(v)) return v.map(restoreDeep);
        if (v && typeof v === "object")
          return Object.fromEntries(Object.entries(v).map(([k, x]) => [k, restoreDeep(x)]));
        return v;
      };
      const hasPlaceholder = (v) =>
        Object.keys(piiMapping).some((ph) => JSON.stringify(v).includes(ph));
      queryOptions.hooks = {
        PreToolUse: [
          {
            hooks: [
              async (input) => {
                if (!input.tool_input || !hasPlaceholder(input.tool_input)) return {};
                process.stderr.write(`[PII-TOOLS ${session_id}] restore su ${input.tool_name}\n`);
                return {
                  hookSpecificOutput: {
                    hookEventName: "PreToolUse",
                    updatedInput: restoreDeep(input.tool_input),
                  },
                };
              },
            ],
          },
        ],
      };
    }

    // Resume from previous SDK session if available
    if (this.sdkSessionId) {
      queryOptions.resume = this.sdkSessionId;
    }

    // With images, write them to temp files and pass the model their PATHS in a
    // plain string prompt (it opens them with Read). This keeps the robust
    // string-prompt path instead of switching to streaming-stdin, which is where
    // the child CLI intermittently died at startup. Without images the prompt is
    // the bare string, byte-for-byte the prior behavior.
    const hasImages = Array.isArray(images) && images.length > 0;
    let imageTempDir = null;
    let promptInput = prompt;
    if (hasImages) {
      const built = buildImageFilePrompt(prompt, images);
      imageTempDir = built.dir;
      promptInput = built.prompt;
    }

    this.aborted = false;
    process.stderr.write(
      `[SIDECAR] query() session=${session_id} resume=${this.sdkSessionId || "new"} images=${hasImages ? images.length : 0} prompt=${String(prompt).slice(0, 80)}\n`,
    );

    const q = query({ prompt: promptInput, options: queryOptions });
    this.queryInstance = q;

    try {
      for await (const msg of q) {
        if (this.aborted) break;
        yield* this.#translate(msg, session_id);
      }
    } finally {
      this.queryInstance = null;
      // Best-effort cleanup of the pasted-image temp dir. The model has already
      // Read the files by the time the turn ends; a leftover dir is harmless but
      // we avoid unbounded /tmp growth across a long session.
      if (imageTempDir) {
        try { rmSync(imageTempDir, { recursive: true, force: true }); } catch { /* ignore */ }
      }
    }
  }

  interrupt() {
    this.aborted = true;
    if (this.queryInstance) {
      try { this.queryInstance.close(); } catch { /* ignore */ }
      this.queryInstance = null;
    }
  }

  close() {
    this.interrupt();
  }

  getResumeToken() {
    return this.sdkSessionId;
  }

  setResumeToken(token) {
    this.sdkSessionId = token ?? null;
  }

  compact() {
    // Reset the SDK session ID — next run() starts a fresh context while the
    // transport keeps the same UI session.
    this.sdkSessionId = null;
  }

  // ── SDK message → normalized event translation (was handleStreamMessage) ──

  *#translate(msg, sessionId) {
    // Subagent messages (Task/Agent tool) carry the parent tool_use id on the
    // SDK wrapper. Propagated as `parent_tool_id` on tool events so the UI can
    // nest subagent activity under its Agent row.
    const parentId = msg.parent_tool_use_id || "";

    switch (msg.type) {
      case "stream_event": {
        // SDKPartialAssistantMessage — real-time streaming deltas
        const ev = msg.event;
        if (!ev) break;

        if (ev.type === "content_block_start") {
          const block = ev.content_block;
          if (block && block.type === "tool_use") {
            if (block.id) this.seenToolIds.add(block.id);
            this.openToolBlocks.set(`${parentId}:${ev.index}`, block.id || "");
            yield {
              event: "tool_start",
              session_id: sessionId,
              tool_name: block.name || "",
              tool_id: block.id || "",
              parent_tool_id: parentId,
            };
          } else if (block && block.type === "thinking") {
            // Subagent thinking must not toggle the main chat's thinking UI.
            if (parentId) break;
            yield {
              event: "thinking_start",
              session_id: sessionId,
            };
          }
        } else if (ev.type === "content_block_stop") {
          this.openToolBlocks.delete(`${parentId}:${ev.index}`);
        } else if (ev.type === "message_start" && ev.message?.usage) {
          // Anthropic API message_start contains input token usage
          const u = ev.message.usage;
          const totalIn = (u.input_tokens || 0) + (u.cache_read_input_tokens || 0) + (u.cache_creation_input_tokens || 0);
          yield {
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
          };
        } else if (ev.type === "message_delta" && ev.usage) {
          // Anthropic API message_delta contains output token usage
          yield {
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
          };
        } else if (ev.type === "content_block_delta") {
          const delta = ev.delta;
          if (delta && delta.type === "text_delta") {
            // Subagent text must never leak into the main assistant reply.
            if (parentId) break;
            yield {
              event: "text_delta",
              session_id: sessionId,
              text: delta.text || "",
            };
          } else if (delta && delta.type === "input_json_delta") {
            yield {
              event: "tool_input_delta",
              session_id: sessionId,
              tool_id: this.openToolBlocks.get(`${parentId}:${ev.index}`) || "",
              partial_json: delta.partial_json || "",
              parent_tool_id: parentId,
            };
          } else if (delta && delta.type === "thinking_delta") {
            if (parentId) break;
            yield {
              event: "thinking_delta",
              session_id: sessionId,
              thinking: delta.thinking || "",
            };
          }
        }
        break;
      }

      case "assistant": {
        // SDKAssistantMessage — complete assistant message.
        // Text already streamed via stream_event; extract tool_use blocks.
        // Skip blocks already announced by content_block_start (streaming on):
        // re-emitting them creates a phantom duplicate that never resolves.
        const content = msg.message?.content || [];
        for (const block of content) {
          if (block.type === "tool_use") {
            if (block.id && this.seenToolIds.has(block.id)) continue;
            if (block.id) this.seenToolIds.add(block.id);
            yield {
              event: "tool_start",
              session_id: sessionId,
              tool_name: block.name || "",
              tool_id: block.id || "",
              parent_tool_id: parentId,
            };
            if (block.input) {
              yield {
                event: "tool_input_delta",
                session_id: sessionId,
                tool_id: block.id || "",
                partial_json: JSON.stringify(block.input),
                parent_tool_id: parentId,
              };
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
            yield {
              event: "tool_result",
              session_id: sessionId,
              tool_id: block.tool_use_id || "",
              result: resultText,
              parent_tool_id: parentId,
            };
          }
        }
        break;
      }

      case "result": {
        // SDKResultMessage — usage is directly on msg
        const usage = msg.usage || msg.result?.usage || {};
        const totalCost = msg.total_cost_usd ?? msg.result?.total_cost_usd ?? 0;

        // Total context = input_tokens + cache_read + cache_creation
        const totalInputTokens = (usage.input_tokens || 0)
          + (usage.cache_read_input_tokens || 0)
          + (usage.cache_creation_input_tokens || 0);

        yield {
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
        };
        break;
      }

      case "system": {
        // Capture the SDK's internal session ID for resume
        if (msg.session_id) {
          this.sdkSessionId = msg.session_id;
        }
        if (msg.subtype === "init") {
          yield {
            event: "system_init",
            session_id: sessionId,
            sdk_session_id: msg.session_id || "",
            model: msg.model || "",
            tools: msg.tools || [],
            mcp_servers: msg.mcp_servers || [],
          };
        }
        break;
      }

      // Ignore: rate_limit_event, status, hooks, tasks, etc.
      default:
        break;
    }
  }
}
