// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

export interface ServiceInfo {
  name: string;
  kind: "McpServer" | "Daemon";
  running: boolean;
  command: string;
  args: string[];
  cwd: string | null;
  /** True for stdio MCP servers spawned on-demand by the chat sidecar (no GUI start/stop) */
  virtual?: boolean;
}

export interface LogLines {
  lines: string[];
}

export interface AddLocalMoonParams {
  name: string;
  command: string;
  args: string[];
  cwd: string | null;
  envVars: string[];
  port: number;
}

export interface LogLineEvent {
  service: string;
  line: string;
}

export interface StatusChangedEvent {
  name: string;
  running: boolean;
}

// ── Image paste ──────────────────────────────────────────────

export interface PastedImage {
  id: string;
  dataUrl: string; // data:image/png;base64,...
  name: string;
}

// ── Chat types ────────────────────────────────────────────────

export interface ToolCallInfo {
  name: string;
  id?: string;
  input: string;
  result?: string;
  /** Timestamp (ms) when this tool call started */
  startedAt?: number;
}

/** A single ordered chunk of an assistant turn, in arrival order.
 * Used to render thinking/text/tool blocks chronologically (interleaved)
 * instead of bucketed (all thinking → all tools → all text). */
export type MessagePart =
  | { kind: "thinking"; text: string }
  | { kind: "text"; text: string }
  | { kind: "tool"; tool: ToolCallInfo };

export interface ChatMessage {
  id: string;
  role: "user" | "assistant" | "system";
  content: string;
  thinking?: string;
  images?: { dataUrl: string; name: string }[];
  toolCalls: ToolCallInfo[];
  /** Ordered blocks for sequential rendering. Absent on legacy/reloaded
   * messages, which fall back to bucketed rendering. */
  parts?: MessagePart[];
  timestamp: number;
  streaming?: boolean;
}

export interface ChatSessionInfo {
  id: string;
  title: string;
  message_count: number;
}

// Backend ChatMsg (from Rust)
export interface ChatMsgBackend {
  id: string;
  role: string;
  content: string;
  thinking?: string;
  tool_calls: { name: string; input: string; result: string | null }[];
  timestamp: number;
  /** True when this row was persisted mid-stream (sidecar crashed). On load,
   * any pending tool calls inside it are normalized to an error result. */
  streaming?: boolean;
}

// ── Tauri event payloads ──────────────────────────────────────

export interface ThinkingDeltaEvent {
  session_id: string;
  thinking: string;
}

export interface TextDeltaEvent {
  session_id: string;
  text: string;
}

export interface ToolStartEvent {
  session_id: string;
  tool_name: string;
  tool_id: string;
}

export interface ToolInputDeltaEvent {
  session_id: string;
  tool_id: string;
  partial_json: string;
}

export interface ToolResultEvent {
  session_id: string;
  tool_id: string;
  result: string;
}

export interface ChatDoneEvent {
  session_id: string;
}

export interface ChatErrorEvent {
  session_id: string;
  error: string;
}

export interface UsageEvent {
  session_id: string;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  total_cost_usd: number;
}

// ── Chart Preview types ──────────────────────────────────────

export interface ChartEntry {
  id: string;
  filePath: string;
  blobUrl: string;
  title: string;
  timestamp: number;
  sessionId: string;
}
