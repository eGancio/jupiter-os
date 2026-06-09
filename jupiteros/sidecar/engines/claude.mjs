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
import { loadMcpServers } from "./engine.mjs";

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

export class ClaudeAgentEngine {
  constructor(cfg = {}) {
    // Per-session SDK state (formerly the sessions Map entry fields).
    this.sdkSessionId = null;
    this.queryInstance = null;
    this.aborted = false;
  }

  /**
   * Run one turn. Yields normalized events (the exact objects the transport
   * emits). Errors propagate to the transport; the transport decides done/error.
   */
  async *run(prompt, options) {
    const { session_id, model, cwd, mcpConfigPath, permissionMode } = options;

    const mcpServers = mcpConfigPath ? loadMcpServers(mcpConfigPath) : {};

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
    };

    // Resume from previous SDK session if available
    if (this.sdkSessionId) {
      queryOptions.resume = this.sdkSessionId;
    }

    // NOTE: images are accepted by the transport but, as in the V1 sidecar, the
    // prompt is sent text-only. Preserved as-is to keep behavior identical.

    this.aborted = false;
    process.stderr.write(
      `[SIDECAR] query() session=${session_id} resume=${this.sdkSessionId || "new"} prompt=${String(prompt).slice(0, 80)}\n`,
    );

    const q = query({ prompt, options: queryOptions });
    this.queryInstance = q;

    try {
      for await (const msg of q) {
        if (this.aborted) break;
        yield* this.#translate(msg, session_id);
      }
    } finally {
      this.queryInstance = null;
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
    switch (msg.type) {
      case "stream_event": {
        // SDKPartialAssistantMessage — real-time streaming deltas
        const ev = msg.event;
        if (!ev) break;

        if (ev.type === "content_block_start") {
          const block = ev.content_block;
          if (block && block.type === "tool_use") {
            yield {
              event: "tool_start",
              session_id: sessionId,
              tool_name: block.name || "",
              tool_id: block.id || "",
            };
          } else if (block && block.type === "thinking") {
            yield {
              event: "thinking_start",
              session_id: sessionId,
            };
          }
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
            yield {
              event: "text_delta",
              session_id: sessionId,
              text: delta.text || "",
            };
          } else if (delta && delta.type === "input_json_delta") {
            yield {
              event: "tool_input_delta",
              session_id: sessionId,
              tool_id: "",
              partial_json: delta.partial_json || "",
            };
          } else if (delta && delta.type === "thinking_delta") {
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
        const content = msg.message?.content || [];
        for (const block of content) {
          if (block.type === "tool_use") {
            yield {
              event: "tool_start",
              session_id: sessionId,
              tool_name: block.name || "",
              tool_id: block.id || "",
            };
            if (block.input) {
              yield {
                event: "tool_input_delta",
                session_id: sessionId,
                tool_id: block.id || "",
                partial_json: JSON.stringify(block.input),
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
