// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

import { useState, useEffect, useCallback, useRef } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  createChatSession,
  sendChatMessage,
  stopChat as stopChatCmd,
  listChatSessions,
  deleteChatSession as deleteChatSessionCmd,
  renameChatSession as renameChatSessionCmd,
  compactChatSession as compactChatSessionCmd,
  classifyChatSession,
  getChatModel,
  setChatSessionModel,
  flushSessionMessages,
  getChatEngine,
} from "../lib/tauri";
import { ENGINE_MODELS } from "../lib/models";
import type {
  ChatMessage,
  ChatSessionInfo,
  ChatToast,
  PastedImage,
  TabStatus,
  ToolCallInfo,
  TextDeltaEvent,
  ThinkingDeltaEvent,
  ToolStartEvent,
  ToolInputDeltaEvent,
  ToolResultEvent,
  ChatDoneEvent,
  ChatErrorEvent,
  UsageEvent,
} from "../types";

let msgCounter = 0;
function nextMsgId() {
  return `msg-${Date.now()}-${++msgCounter}`;
}

/** Max concurrent open tabs — tetto alto anti-overload (ogni tab tiene viva
 *  una sessione sidecar, quindi non illimitato). */
export const MAX_TABS = 16;
const TABS_STORAGE_KEY = "jupiteros.chatTabs.v1";

// ── Sequential parts accumulation ───────────────────────────────
// These keep the legacy fields (thinking/content/toolCalls) in sync — needed
// for persistence — while also appending to `parts` so the UI can render
// thinking/text/tool blocks in true chronological order. The `tool` part holds
// the SAME ToolCallInfo reference stored in toolCalls[], so result updates
// propagate to the rendered part automatically.
function appendThinkingPart(msg: ChatMessage, text: string) {
  msg.thinking = (msg.thinking || "") + text;
  const parts = (msg.parts ||= []);
  const tail = parts[parts.length - 1];
  if (tail && tail.kind === "thinking") tail.text += text;
  else parts.push({ kind: "thinking", text });
}
function appendTextPart(msg: ChatMessage, text: string) {
  msg.content += text;
  const parts = (msg.parts ||= []);
  const tail = parts[parts.length - 1];
  if (tail && tail.kind === "text") tail.text += text;
  else parts.push({ kind: "text", text });
}
function pushToolPart(msg: ChatMessage, tc: ToolCallInfo) {
  msg.toolCalls.push(tc);
  (msg.parts ||= []).push({ kind: "tool", tool: tc });
}

// Rebuild a ChatMessage from a persisted backend row, restoring the
// parent/children nesting of subagent tool calls. Old sessions on disk have
// no id/parent_tool_id fields and reload flat, exactly as before.
function backendMsgToChatMessage(m: any): ChatMessage {
  // Messages flushed mid-stream (sidecar crashed) have streaming=true and
  // pending tool calls (result null). Mark those tools as interrupted so the
  // timeline doesn't show a forever-spinner.
  const interrupted = m.streaming === true;
  const toolCalls: ToolCallInfo[] = (m.tool_calls || []).map((tc: any) => ({
    name: tc.name,
    id: tc.id || tc.name, // old sessions lack ids — fallback to name
    input: tc.input || "",
    result:
      interrupted && (tc.result === null || tc.result === undefined)
        ? "[ERROR] Sessione interrotta"
        : tc.result,
    parentToolUseId: tc.parent_tool_id || undefined,
  }));
  const byId = new Map(toolCalls.filter((tc) => tc.id).map((tc) => [tc.id!, tc]));
  for (const tc of toolCalls) {
    if (!tc.parentToolUseId) continue;
    const parent = byId.get(tc.parentToolUseId);
    if (parent && parent !== tc) (parent.children ||= []).push(tc);
    else tc.parentToolUseId = undefined; // orphan → render flat
  }
  return {
    id: m.id || nextMsgId(),
    role: m.role as "user" | "assistant" | "system",
    content: m.content,
    thinking: m.thinking || undefined,
    toolCalls,
    timestamp: (m.timestamp || 0) * 1000,
    // streaming flag intentionally omitted — treat as completed.
  };
}

/** Volatile per-session run state. Lives in the ref (no re-render per delta);
 * it is the source of truth — the React singletons below only mirror the
 * ACTIVE session's runtime. */
interface SessionRuntime {
  streaming: boolean;
  error: string | null;
  activeTool: string | null;
  activeThinking: boolean;
  activeFile: string | null;
  lastInputTokens: number;
  pendingQueue: Array<{ text: string; images?: PastedImage[] }>;
}

function newRuntime(): SessionRuntime {
  return {
    streaming: false,
    error: null,
    activeTool: null,
    activeThinking: false,
    activeFile: null,
    lastInputTokens: 0,
    pendingQueue: [],
  };
}

/** Messages + metadata stored per session */
interface SessionData {
  messages: ChatMessage[];
  usage: { inputTokens: number; outputTokens: number; totalCostUsd: number };
  runtime: SessionRuntime;
}

export function useChat() {
  // Session management
  const [sessions, setSessions] = useState<ChatSessionInfo[]>([]);
  const [activeSessionId, setActiveSessionId] = useState<string | null>(null);

  // Per-session data stored in a ref (not state, to avoid re-renders on every delta)
  const sessionDataRef = useRef<Map<string, SessionData>>(new Map());

  // Active session's messages (synced from sessionDataRef)
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [streaming, setStreaming] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [activeTool, setActiveTool] = useState<string | null>(null);
  const [activeThinking, setActiveThinking] = useState(false);
  const [model, setModel] = useState("opus");
  // Full model id resolved by the SDK for the ACTIVE session (e.g.
  // "claude-opus-4-8") — lets the UI show the real version, not just the alias.
  const [resolvedModel, setResolvedModel] = useState<string | null>(null);
  const [lastInputTokens, setLastInputTokens] = useState(0);
  const [activeFile, setActiveFile] = useState<string | null>(null);

  // ── Chat workspace: open tabs + per-tab status + toasts ──────
  const [openTabs, setOpenTabs] = useState<string[]>([]);
  const [tabStatus, setTabStatus] = useState<Record<string, TabStatus>>({});
  const [toasts, setToasts] = useState<ChatToast[]>([]);

  const activeRef = useRef<string | null>(null);
  const openTabsRef = useRef<string[]>([]);
  // sid → full model id resolved by the SDK (from the system_init event).
  const resolvedModelRef = useRef<Map<string, string>>(new Map());
  const sessionsRef = useRef<ChatSessionInfo[]>([]);
  const toastCounter = useRef(0);
  const toastTimers = useRef<Map<number, ReturnType<typeof setTimeout>>>(new Map());

  useEffect(() => {
    sessionsRef.current = sessions;
  }, [sessions]);

  /** setOpenTabs keeping the ref mirror in sync (updater must be pure). */
  function updateTabs(updater: (prev: string[]) => string[]) {
    setOpenTabs((prev) => {
      const next = updater(prev);
      openTabsRef.current = next;
      return next;
    });
  }

  /** Patch a tab's volatile status; no-op (no re-render) when nothing changes. */
  function patchTab(sid: string, patch: Partial<TabStatus>) {
    setTabStatus((prev) => {
      const cur = prev[sid] ?? { streaming: false, unread: false, error: false };
      const next = { ...cur, ...patch };
      if (
        next.streaming === cur.streaming &&
        next.unread === cur.unread &&
        next.error === cur.error
      ) {
        return prev;
      }
      return { ...prev, [sid]: next };
    });
  }

  const dismissToast = useCallback((id: number) => {
    const t = toastTimers.current.get(id);
    if (t) {
      clearTimeout(t);
      toastTimers.current.delete(id);
    }
    setToasts((prev) => prev.filter((x) => x.id !== id));
  }, []);

  /** Toast for background-session done/error (or kind "info" for tab-cap feedback).
   * Stack capped at 3 (oldest dropped), auto-dismiss after 6s. */
  function pushToast(sessionId: string, kind: ChatToast["kind"]) {
    const id = ++toastCounter.current;
    const title = sessionsRef.current.find((s) => s.id === sessionId)?.title || "";
    setToasts((prev) => {
      const next = [...prev, { id, sessionId, kind, title }];
      while (next.length > 3) {
        const drop = next.shift()!;
        const t = toastTimers.current.get(drop.id);
        if (t) {
          clearTimeout(t);
          toastTimers.current.delete(drop.id);
        }
      }
      return next;
    });
    toastTimers.current.set(
      id,
      setTimeout(() => dismissToast(id), 6000)
    );
  }

  useEffect(() => {
    const timers = toastTimers.current;
    return () => {
      timers.forEach((t) => clearTimeout(t));
      timers.clear();
    };
  }, []);

  // Persist open tabs across restarts (reconciled against real sessions on boot).
  useEffect(() => {
    if (openTabs.length === 0) return;
    try {
      localStorage.setItem(
        TABS_STORAGE_KEY,
        JSON.stringify({ open: openTabs, active: activeSessionId })
      );
    } catch {
      // storage full/unavailable — tabs just won't survive restart
    }
  }, [openTabs, activeSessionId]);

  // Debounced eager-flush of per-session message snapshot to disk.
  // Survives sidecar crashes mid-stream (worst case: ~500ms of deltas lost).
  const flushTimerRef = useRef<Map<string, ReturnType<typeof setTimeout>>>(new Map());

  function scheduleFlush(sid: string) {
    const existing = flushTimerRef.current.get(sid);
    if (existing) clearTimeout(existing);
    const id = setTimeout(() => {
      flushTimerRef.current.delete(sid);
      const data = sessionDataRef.current.get(sid);
      if (data) {
        flushSessionMessages(sid, data.messages).catch(() => {
          // Best-effort; the backend's "done" handler will save on completion anyway.
        });
      }
    }, 500);
    flushTimerRef.current.set(sid, id);
  }

  function cancelFlush(sid: string) {
    const t = flushTimerRef.current.get(sid);
    if (t) {
      clearTimeout(t);
      flushTimerRef.current.delete(sid);
    }
  }

  // Helper: get or create session data
  function getSessionData(sid: string): SessionData {
    let data = sessionDataRef.current.get(sid);
    if (!data) {
      data = {
        messages: [],
        usage: { inputTokens: 0, outputTokens: 0, totalCostUsd: 0 },
        runtime: newRuntime(),
      };
      sessionDataRef.current.set(sid, data);
    }
    return data;
  }

  // Helper: update messages state if it's the active session
  function syncMessages(sid: string) {
    if (sid === activeRef.current) {
      const data = sessionDataRef.current.get(sid);
      if (data) {
        setMessages([...data.messages]);
      }
    }
  }

  /** Mirror a session's runtime into the active-session React state.
   * Call ONLY for the active session (typically right after activating it). */
  function syncRuntime(sid: string) {
    const rt = getSessionData(sid).runtime;
    setStreaming(rt.streaming);
    setError(rt.error);
    setActiveTool(rt.activeTool);
    setActiveThinking(rt.activeThinking);
    setActiveFile(rt.activeFile);
    setLastInputTokens(rt.lastInputTokens);
  }

  // Load existing sessions on mount, or create a new one if none exist.
  // Open tabs are restored from localStorage, reconciled against real sessions.
  useEffect(() => {
    listChatSessions().then(async (list) => {
      sessionsRef.current = list;
      if (list.length > 0) {
        setSessions(list);

        // Backlog sweep: auto-title chats created before this feature (or whose
        // earlier attempt failed). Rust guards skip anything not worth
        // (re)titling; stagger to avoid spawning many Haiku one-shots at once.
        // `chats-reclassified` refreshes as they land.
        void (async () => {
          const pending = list.filter((s) => s.message_count >= 2);
          for (const s of pending) {
            classifyChatSession(s.id).catch(() => {});
            await new Promise((r) => setTimeout(r, 400));
          }
        })();

        let open: string[] = [];
        let active: string | null = null;
        try {
          const saved = JSON.parse(localStorage.getItem(TABS_STORAGE_KEY) || "null");
          if (saved && Array.isArray(saved.open)) {
            const ids = new Set(list.map((s) => s.id));
            open = saved.open.filter((id: unknown): id is string =>
              typeof id === "string" && ids.has(id)
            ).slice(0, MAX_TABS);
            active =
              typeof saved.active === "string" && open.includes(saved.active)
                ? saved.active
                : open[open.length - 1] ?? null;
          }
        } catch {
          // corrupt storage — fall through to default
        }
        if (open.length === 0 || !active) {
          const last = list[list.length - 1];
          open = [last.id];
          active = last.id;
        }

        updateTabs(() => open);
        setActiveSessionId(active);
        activeRef.current = active;
        getSessionData(active);
        const info = list.find((s) => s.id === active);
        if (info?.model) setModel(info.model);
        else getChatModel().then(setModel).catch(() => {});

        // Load messages for the active session (other tabs lazy-load on switch)
        try {
          const { getChatMessages } = await import("../lib/tauri");
          const msgs = await getChatMessages(active);
          if (msgs.length > 0) {
            const data = getSessionData(active);
            data.messages = msgs.map(backendMsgToChatMessage);
            syncMessages(active);
          }
        } catch {
          // ignore — messages will load when user sends a message
        }
      } else {
        // No saved sessions — create a fresh one
        const id = await createChatSession();
        setActiveSessionId(id);
        activeRef.current = id;
        getSessionData(id);
        updateTabs(() => [id]);
        getChatModel().then(setModel).catch(() => {});
        refreshSessionList();
      }
    });
  }, []);

  // Refresh session list from backend
  const refreshSessionList = useCallback(async () => {
    try {
      const list = await listChatSessions();
      sessionsRef.current = list;
      setSessions(list);
    } catch {
      // ignore
    }
  }, []);

  // ── Core send (no queue check) ──────────────────────────
  // Takes the session id explicitly: the done-handler drains queued messages
  // of BACKGROUND sessions too, so it can't rely on activeRef.
  const doSend = useCallback(
    async (sid: string, text: string, images?: PastedImage[]) => {
      if (!sid) return;
      const data = getSessionData(sid);
      const rt = data.runtime;
      rt.error = null;
      rt.streaming = true;
      rt.activeTool = null;
      patchTab(sid, { streaming: true });
      if (sid === activeRef.current) {
        setError(null);
        setStreaming(true);
        setActiveTool(null);
      }

      // Add user message
      data.messages.push({
        id: nextMsgId(),
        role: "user",
        content: text,
        images: images?.map((img) => ({ dataUrl: img.dataUrl, name: img.name })),
        toolCalls: [],
        timestamp: Date.now(),
      });
      syncMessages(sid);

      // Extract base64
      const imageDataList = images?.map((img) => {
        const idx = img.dataUrl.indexOf(",");
        return idx >= 0 ? img.dataUrl.slice(idx + 1) : img.dataUrl;
      });

      try {
        await sendChatMessage(sid, text, imageDataList);
      } catch (e) {
        const errMsg = String(e);
        rt.streaming = false;
        rt.error = errMsg;
        patchTab(sid, { streaming: false, error: true });
        if (sid === activeRef.current) {
          setStreaming(false);
          setError(errMsg);
        } else {
          patchTab(sid, { unread: true });
          pushToast(sid, "error");
        }
        // Show the error as an assistant message so it's visible in the chat
        data.messages.push({
          id: nextMsgId(),
          role: "assistant",
          content: `Error: ${errMsg}`,
          toolCalls: [],
          timestamp: Date.now(),
          streaming: false,
        });
        syncMessages(sid);
      }
    },
    []
  );

  // Listen to Tauri events
  useEffect(() => {
    const unlisteners: Promise<() => void>[] = [];

    // Thinking delta
    unlisteners.push(
      listen<ThinkingDeltaEvent>("claude-thinking-delta", (event) => {
        const sid = event.payload.session_id;
        if (!sid) return;
        const data = getSessionData(sid);
        const rt = data.runtime;

        rt.activeThinking = true;
        if (!rt.streaming) {
          rt.streaming = true;
          patchTab(sid, { streaming: true });
        }
        if (sid === activeRef.current) setActiveThinking(true);

        const msgs = data.messages;
        const last = msgs[msgs.length - 1];
        if (last && last.role === "assistant" && last.streaming) {
          appendThinkingPart(last, event.payload.thinking);
        } else {
          msgs.push({
            id: nextMsgId(),
            role: "assistant",
            content: "",
            thinking: event.payload.thinking,
            toolCalls: [],
            parts: [{ kind: "thinking", text: event.payload.thinking }],
            timestamp: Date.now(),
            streaming: true,
          });
        }
        syncMessages(sid);
        scheduleFlush(sid);
      })
    );

    // Text delta
    unlisteners.push(
      listen<TextDeltaEvent>("claude-text-delta", (event) => {
        const sid = event.payload.session_id;
        if (!sid) return;
        const data = getSessionData(sid);
        const rt = data.runtime;

        rt.activeTool = null;
        rt.activeThinking = false;
        rt.activeFile = null;
        if (!rt.streaming) {
          rt.streaming = true;
          patchTab(sid, { streaming: true });
        }
        if (sid === activeRef.current) {
          setActiveTool(null);
          setActiveThinking(false);
          setActiveFile(null);
        }

        const msgs = data.messages;
        const last = msgs[msgs.length - 1];
        if (last && last.role === "assistant" && last.streaming) {
          appendTextPart(last, event.payload.text);
        } else {
          msgs.push({
            id: nextMsgId(),
            role: "assistant",
            content: event.payload.text,
            toolCalls: [],
            parts: [{ kind: "text", text: event.payload.text }],
            timestamp: Date.now(),
            streaming: true,
          });
        }
        syncMessages(sid);
        scheduleFlush(sid);
      })
    );

    // Tool start
    unlisteners.push(
      listen<ToolStartEvent>("claude-tool-start", (event) => {
        const sid = event.payload.session_id;
        if (!sid) return;
        const data = getSessionData(sid);

        data.runtime.activeTool = event.payload.tool_name;
        if (sid === activeRef.current) setActiveTool(event.payload.tool_name);

        const msgs = data.messages;
        const last = msgs[msgs.length - 1];
        // Dedupe: engines may re-announce the same tool_use (streaming delta +
        // complete message). A second entry would never get a result and show
        // up as a phantom ✗ next to the real ✓.
        if (
          event.payload.tool_id &&
          last &&
          last.role === "assistant" &&
          last.toolCalls.some((tc) => tc.id === event.payload.tool_id)
        ) {
          return;
        }
        const tc: ToolCallInfo = {
          name: event.payload.tool_name,
          id: event.payload.tool_id,
          input: "",
          result: undefined,
          startedAt: Date.now(),
        };
        if (last && last.role === "assistant") {
          // Subagent tool: nest under its parent Agent call. The child lives
          // in toolCalls (flat — persistence, [Stopped]/result matching) AND
          // as a ref in parent.children (nested rendering). No part is pushed
          // so it never shows in the main flow. Parent not found (e.g. resume
          // mid-run) → degrade to a flat top-level tool.
          const parentId = event.payload.parent_tool_id || "";
          const parent = parentId
            ? last.toolCalls.find((p) => p.id === parentId)
            : undefined;
          if (parent) {
            tc.parentToolUseId = parentId;
            last.toolCalls.push(tc);
            (parent.children ||= []).push(tc);
          } else {
            pushToolPart(last, tc);
          }
        } else {
          msgs.push({
            id: nextMsgId(),
            role: "assistant",
            content: "",
            toolCalls: [tc],
            parts: [{ kind: "tool", tool: tc }],
            timestamp: Date.now(),
            streaming: true,
          });
        }
        syncMessages(sid);
        scheduleFlush(sid);
      })
    );

    // Tool input delta
    unlisteners.push(
      listen<ToolInputDeltaEvent>("claude-tool-input-delta", (event) => {
        const sid = event.payload.session_id;
        if (!sid) return;
        const data = getSessionData(sid);

        const msgs = data.messages;
        const last = msgs[msgs.length - 1];
        if (last && last.role === "assistant" && last.toolCalls.length > 0) {
          // Match by id when available (parallel subagents interleave deltas);
          // fallback to the last tool for engines without ids on deltas.
          const tid = event.payload.tool_id;
          const lastTc =
            (tid ? last.toolCalls.find((tc) => tc.id === tid) : undefined) ||
            last.toolCalls[last.toolCalls.length - 1];
          lastTc.input += event.payload.partial_json;

          // Extract file path from accumulated tool input (runtime always,
          // React state only for the active session)
          try {
            const fileMatch = lastTc.input.match(/"(?:file_path|path|notebook_path)"\s*:\s*"([^"]+)"/);
            if (fileMatch) {
              const fullPath = fileMatch[1];
              const fileName = fullPath.split(/[/\\]/).pop() || fullPath;
              data.runtime.activeFile = fileName;
              if (sid === activeRef.current) setActiveFile(fileName);
            }
          } catch { /* ignore partial JSON parse errors */ }

          syncMessages(sid);
          scheduleFlush(sid);
        }
      })
    );

    // Tool result
    unlisteners.push(
      listen<ToolResultEvent>("claude-tool-result", (event) => {
        const sid = event.payload.session_id;
        if (!sid) return;
        const data = getSessionData(sid);

        data.runtime.activeTool = null;
        data.runtime.activeFile = null;
        if (sid === activeRef.current) {
          setActiveTool(null);
          setActiveFile(null);
        }

        const msgs = data.messages;
        for (let i = msgs.length - 1; i >= 0; i--) {
          if (msgs[i].role === "assistant" && msgs[i].toolCalls.length > 0) {
            const tcs = msgs[i].toolCalls;
            const idx = tcs.findIndex((tc) => tc.id === event.payload.tool_id);
            if (idx !== -1) {
              tcs[idx].result = event.payload.result;
              break;
            }
            const noResultIdx = tcs.findIndex((tc) => tc.result === undefined);
            if (noResultIdx !== -1) {
              tcs[noResultIdx].result = event.payload.result;
              break;
            }
          }
        }
        syncMessages(sid);
        scheduleFlush(sid);
      })
    );

    // Done
    unlisteners.push(
      listen<ChatDoneEvent>("claude-done", (event) => {
        const sid = event.payload.session_id;
        if (!sid) return;
        // Backend persists the finalized message on done; drop any pending flush
        // to avoid racing with backend's own save_sessions_to_disk.
        cancelFlush(sid);
        const data = getSessionData(sid);
        const rt = data.runtime;

        const msgs = data.messages;
        const last = msgs[msgs.length - 1];
        if (last && last.role === "assistant" && last.streaming) {
          last.streaming = false;
          // Tools that never got a tool_result (SDK ended turn without one)
          // would otherwise pulse "pending" forever. Mark them as failed so
          // the timeline reflects the truth.
          for (const tc of last.toolCalls) {
            if (tc.result === undefined || tc.result === null) {
              tc.result = "[ERROR] Nessuna risposta dal tool";
            }
          }
        }

        rt.activeTool = null;
        rt.activeThinking = false;
        rt.activeFile = null;

        // Drain this session's queue (works for background tabs too), or
        // finish the run.
        const next = rt.pendingQueue.shift();
        if (next) {
          // Small delay to let UI update
          setTimeout(() => doSend(sid, next.text, next.images), 100);
        } else {
          rt.streaming = false;
          patchTab(sid, { streaming: false });
        }

        if (sid === activeRef.current) {
          setActiveTool(null);
          setActiveThinking(false);
          setActiveFile(null);
          syncMessages(sid);
          if (!next) setStreaming(false);
        } else if (!next) {
          // Background run finished: badge + clickable toast
          patchTab(sid, { unread: true });
          pushToast(sid, "done");
        }

        // Refresh session list (title may have updated)
        refreshSessionList();

        // Chat is now idle: fire a background Haiku auto-title. Rust guards
        // make this a no-op unless the chat is worth titling, so it's safe to
        // call on every turn. The `chats-reclassified` event refreshes the
        // sidebar once the title comes back.
        if (!next) {
          classifyChatSession(sid).catch(() => {});
        }
      })
    );

    // Auto-title came back → refresh the sidebar.
    unlisteners.push(
      listen<{ session_id: string }>("chats-reclassified", () => {
        refreshSessionList();
      })
    );

    // Model resolved by the SDK (real version, e.g. "claude-opus-4-8")
    unlisteners.push(
      listen<{ session_id: string; model: string }>(
        "claude-model-resolved",
        (event) => {
          const sid = event.payload.session_id;
          const full = event.payload.model;
          if (!sid || !full) return;
          resolvedModelRef.current.set(sid, full);
          if (sid === activeRef.current) setResolvedModel(full);
        },
      ),
    );

    // Usage
    unlisteners.push(
      listen<UsageEvent>("claude-usage", (event) => {
        const sid = event.payload.session_id;
        if (!sid) return;
        const data = getSessionData(sid);

        // Track context window % — only from the final result event (total_cost_usd > 0)
        // to avoid fluctuations from intermediate usage_update events during agentic loops
        if (event.payload.input_tokens > 0 && event.payload.total_cost_usd > 0) {
          data.runtime.lastInputTokens = event.payload.input_tokens;
          if (sid === activeRef.current) {
            setLastInputTokens(event.payload.input_tokens);
          }
        }

        // Accumulate cost only from final result events (total_cost_usd > 0)
        if (event.payload.total_cost_usd > 0) {
          data.usage.inputTokens += event.payload.input_tokens;
          data.usage.outputTokens += event.payload.output_tokens;
          data.usage.totalCostUsd += event.payload.total_cost_usd;
        }
      })
    );

    // Error
    unlisteners.push(
      listen<ChatErrorEvent>("claude-error", (event) => {
        const sid = event.payload.session_id;
        if (!sid) return;
        cancelFlush(sid);
        const data = getSessionData(sid);
        const rt = data.runtime;

        const msgs = data.messages;
        const last = msgs[msgs.length - 1];
        if (last && last.role === "assistant" && last.streaming) {
          last.streaming = false;
          last.content = last.content || `Error: ${event.payload.error}`;
          for (const tc of last.toolCalls) {
            if (tc.result === undefined || tc.result === null) {
              tc.result = "[ERROR] Sessione interrotta";
            }
          }
        } else {
          msgs.push({
            id: nextMsgId(),
            role: "assistant",
            content: `Error: ${event.payload.error}`,
            toolCalls: [],
            timestamp: Date.now(),
            streaming: false,
          });
        }

        // Clear THIS session's queue — messages would be sent into a broken state
        rt.pendingQueue = [];
        rt.streaming = false;
        rt.error = event.payload.error;
        rt.activeTool = null;
        rt.activeThinking = false;
        rt.activeFile = null;
        patchTab(sid, { streaming: false, error: true });

        if (sid === activeRef.current) {
          setStreaming(false);
          setActiveTool(null);
          setActiveThinking(false);
          setActiveFile(null);
          setError(event.payload.error);
          syncMessages(sid);
        } else {
          patchTab(sid, { unread: true });
          pushToast(sid, "error");
        }
      })
    );

    return () => {
      unlisteners.forEach((p) => p.then((fn) => fn()));
    };
  }, [refreshSessionList, doSend]);

  // ── Actions ────────────────────────────────────────────────

  const sendMessage = useCallback(
    async (text: string, images?: PastedImage[]) => {
      const sid = activeRef.current;
      if (!sid) return;
      const data = getSessionData(sid);
      if (data.runtime.streaming) {
        // Queue the message — it will be sent when streaming ends
        data.messages.push({
          id: nextMsgId(),
          role: "user",
          content: text,
          images: images?.map((img) => ({ dataUrl: img.dataUrl, name: img.name })),
          toolCalls: [],
          timestamp: Date.now(),
        });
        syncMessages(sid);
        data.runtime.pendingQueue.push({ text, images });
        return;
      }
      await doSend(sid, text, images);
    },
    [doSend]
  );

  const stopStreaming = useCallback(async () => {
    const sid = activeRef.current;
    if (!sid) return;

    // Mark any running tools as stopped so they don't show as "running" forever
    const data = sessionDataRef.current.get(sid);
    if (data) {
      for (const msg of data.messages) {
        if (msg.streaming) msg.streaming = false;
        for (const tc of msg.toolCalls) {
          if (tc.result === undefined) {
            tc.result = "[Stopped]";
          }
        }
      }
      data.runtime.streaming = false;
      data.runtime.activeTool = null;
      data.runtime.activeThinking = false;
      data.runtime.activeFile = null;
      data.runtime.pendingQueue = [];
      syncMessages(sid);
    }
    patchTab(sid, { streaming: false });

    try {
      await stopChatCmd(sid);
    } catch {
      // ignore
    }
    setStreaming(false);
    setActiveTool(null);
    setActiveFile(null);
    setActiveThinking(false);
  }, []);

  const switchSession = useCallback(
    async (targetId: string) => {
      if (targetId === activeRef.current) return;
      setActiveSessionId(targetId);
      activeRef.current = targetId;
      const data = getSessionData(targetId);

      // If no messages in memory, try loading from backend (persisted)
      if (data.messages.length === 0) {
        try {
          const { getChatMessages } = await import("../lib/tauri");
          const msgs = await getChatMessages(targetId);
          if (msgs.length > 0) {
            data.messages = msgs.map(backendMsgToChatMessage);
          }
        } catch {
          // ignore
        }
      }

      setMessages([...data.messages]);
      // Restore the target's real run state (fixes the old "switching loses
      // the streaming indicator" behavior) and clear its attention flags.
      syncRuntime(targetId);
      patchTab(targetId, { unread: false, error: false });
      const info = sessionsRef.current.find((s) => s.id === targetId);
      if (info?.model) setModel(info.model);
      setResolvedModel(resolvedModelRef.current.get(targetId) ?? null);
    },
    []
  );

  /** Create a new session in a new tab. Returns the id, or null if refused
   * (tab cap reached — feedback shown via toast). */
  const newSession = useCallback(async (): Promise<string | null> => {
    if (openTabsRef.current.length >= MAX_TABS) {
      pushToast("", "info");
      return null;
    }
    const id = await createChatSession();
    // Coerenza engine/modello: le nuove sessioni ereditano l'ultimo modello
    // scelto (default globale nel backend), che può appartenere a un ALTRO
    // engine (es. engine tornato a ollama con modello di localops rimasto
    // come default). Se il modello non è nella lista dell'engine attivo,
    // riparti dal default di quell'engine.
    try {
      const eng = await getChatEngine();
      const list = ENGINE_MODELS[eng];
      if (list?.length && !list.includes(model)) {
        await setChatSessionModel(id, list[0]);
        setModel(list[0]);
      }
    } catch { /* engine sconosciuto: lascia il modello com'è */ }
    setActiveSessionId(id);
    activeRef.current = id;
    getSessionData(id);
    updateTabs((prev) => (prev.includes(id) ? prev : [...prev, id]));
    setMessages([]);
    setStreaming(false);
    setError(null);
    setActiveTool(null);
    setActiveFile(null);
    setActiveThinking(false);
    setLastInputTokens(0);
    await refreshSessionList();
    return id;
  }, [refreshSessionList, model]);

  /** Open a session as a tab (or just activate it if already open). */
  const openTab = useCallback(
    async (sid: string) => {
      // Guard against stale toast clicks on deleted sessions
      if (!sessionsRef.current.some((s) => s.id === sid)) return;
      if (openTabsRef.current.includes(sid)) {
        await switchSession(sid);
        return;
      }
      if (openTabsRef.current.length >= MAX_TABS) {
        pushToast("", "info");
        return;
      }
      updateTabs((prev) => (prev.includes(sid) ? prev : [...prev, sid]));
      await switchSession(sid);
    },
    [switchSession]
  );

  /** Close a tab. The session is NOT deleted; a streaming session keeps
   * running headless (its done/error toast can reopen it). */
  const closeTab = useCallback(
    async (sid: string) => {
      const tabs = openTabsRef.current;
      const idx = tabs.indexOf(sid);
      if (idx === -1) return;
      const next = tabs.filter((id) => id !== sid);
      updateTabs(() => next);

      if (sid !== activeRef.current) return;

      // Right neighbor first, else left
      const neighbor = next[Math.min(idx, next.length - 1)];
      if (neighbor) {
        await switchSession(neighbor);
        return;
      }
      // Last tab closed: open the most recent other session, else create new
      const candidate = [...sessionsRef.current].reverse().find((s) => s.id !== sid);
      if (candidate) {
        updateTabs(() => [candidate.id]);
        await switchSession(candidate.id);
      } else {
        await newSession();
      }
    },
    [switchSession, newSession]
  );

  /** Compact: reset SDK context window but keep the same UI session. */
  const compactSession = useCallback(async () => {
    const sid = activeRef.current;
    if (!sid) return;
    await compactChatSessionCmd(sid);
    getSessionData(sid).runtime.lastInputTokens = 0;
    setLastInputTokens(0);
  }, []);

  const deleteSession = useCallback(
    async (targetId: string) => {
      cancelFlush(targetId);
      try {
        await deleteChatSessionCmd(targetId);
      } catch {
        // ignore
      }
      sessionDataRef.current.delete(targetId);
      updateTabs((prev) => prev.filter((id) => id !== targetId));
      setTabStatus((prev) => {
        if (!(targetId in prev)) return prev;
        const next = { ...prev };
        delete next[targetId];
        return next;
      });
      setToasts((prev) => prev.filter((t) => t.sessionId !== targetId));

      // If deleting active session, switch to another open tab, then any
      // remaining session, then create new
      if (targetId === activeRef.current) {
        const remainingTab = openTabsRef.current.find((id) => id !== targetId);
        const remaining = sessions.filter((s) => s.id !== targetId);
        if (remainingTab) {
          await switchSession(remainingTab);
        } else if (remaining.length > 0) {
          const last = remaining[remaining.length - 1];
          updateTabs((prev) => (prev.includes(last.id) ? prev : [...prev, last.id]));
          await switchSession(last.id);
        } else {
          await newSession();
        }
      }
      await refreshSessionList();
    },
    [sessions, switchSession, newSession, refreshSessionList]
  );

  const renameSession = useCallback(
    async (targetId: string, title: string) => {
      try {
        await renameChatSessionCmd(targetId, title);
        await refreshSessionList();
      } catch {
        // ignore
      }
    },
    [refreshSessionList]
  );

  /** Get usage for active session (for /cost command) */
  const getUsage = useCallback(() => {
    if (!activeRef.current) return { inputTokens: 0, outputTokens: 0, totalCostUsd: 0 };
    return getSessionData(activeRef.current).usage;
  }, []);

  /** Change the ACTIVE session's model (per-tab). Other tabs keep theirs;
   * new sessions inherit the last choice (backend updates the default). */
  const changeModel = useCallback(async (newModel: string) => {
    const sid = activeRef.current;
    if (!sid) return;
    try {
      await setChatSessionModel(sid, newModel);
      setModel(newModel);
      // The resolved version is now stale (different family/version); clear it
      // so the UI falls back to the alias label until the next system_init.
      resolvedModelRef.current.delete(sid);
      setResolvedModel(null);
      await refreshSessionList();
    } catch {
      // ignore
    }
  }, [refreshSessionList]);

  /** Clear frontend messages for the active session (backend keeps full context) */
  const clearMessages = useCallback(() => {
    if (!activeRef.current) return;
    const sid = activeRef.current;
    const data = getSessionData(sid);
    data.messages = [];
    syncMessages(sid);
  }, []);

  /** Inject a local system message into the active session */
  const addSystemMessage = useCallback((content: string) => {
    if (!activeRef.current) return;
    const sid = activeRef.current;
    const data = getSessionData(sid);
    data.messages.push({
      id: nextMsgId(),
      role: "system",
      content,
      toolCalls: [],
      timestamp: Date.now(),
    });
    syncMessages(sid);
  }, []);

  return {
    // Active session state
    messages,
    streaming,
    sessionId: activeSessionId,
    error,
    activeTool,
    activeThinking,
    model,
    resolvedModel,
    lastInputTokens,
    activeFile,

    // Session management
    sessions,
    switchSession,
    deleteSession,
    renameSession,
    newSession,
    compactSession,

    // Chat workspace (tabs + toasts)
    tabs: openTabs,
    tabStatus,
    openTab,
    closeTab,
    toasts,
    dismissToast,

    // Actions
    sendMessage,
    stopStreaming,
    getUsage,
    addSystemMessage,
    clearMessages,
    changeModel,
  };
}
