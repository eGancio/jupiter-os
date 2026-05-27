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
  getChatModel,
  setChatModel as setChatModelCmd,
  flushSessionMessages,
} from "../lib/tauri";
import type {
  ChatMessage,
  ChatSessionInfo,
  PastedImage,
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

/** Messages + metadata stored per session */
interface SessionData {
  messages: ChatMessage[];
  usage: { inputTokens: number; outputTokens: number; totalCostUsd: number };
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
  const [model, setModel] = useState("sonnet");
  const [lastInputTokens, setLastInputTokens] = useState(0);
  const [activeFile, setActiveFile] = useState<string | null>(null);

  const activeRef = useRef<string | null>(null);
  const pendingQueue = useRef<Array<{ text: string; images?: PastedImage[] }>>([]);

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
      data = { messages: [], usage: { inputTokens: 0, outputTokens: 0, totalCostUsd: 0 } };
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

  // Load model from backend on mount
  useEffect(() => {
    getChatModel().then(setModel).catch(() => {});
  }, []);

  // Load existing sessions on mount, or create a new one if none exist
  useEffect(() => {
    listChatSessions().then(async (list) => {
      if (list.length > 0) {
        // Use the most recent (last) saved session
        setSessions(list);
        const lastSession = list[list.length - 1];
        setActiveSessionId(lastSession.id);
        activeRef.current = lastSession.id;
        getSessionData(lastSession.id);
        // Load messages for the active session
        try {
          const { getChatMessages } = await import("../lib/tauri");
          const msgs = await getChatMessages(lastSession.id);
          if (msgs.length > 0) {
            const data = getSessionData(lastSession.id);
            data.messages = msgs.map((m: any) => {
              // Messages flushed mid-stream (sidecar crashed) have streaming=true
              // and pending tool calls (result null). Mark those tools as
              // interrupted so the timeline doesn't show a forever-spinner.
              const interrupted = m.streaming === true;
              return {
                id: m.id || nextMsgId(),
                role: m.role as "user" | "assistant" | "system",
                content: m.content,
                thinking: m.thinking || undefined,
                toolCalls: (m.tool_calls || []).map((tc: any) => ({
                  name: tc.name,
                  id: tc.name, // use name as fallback id
                  input: tc.input || "",
                  result:
                    interrupted && (tc.result === null || tc.result === undefined)
                      ? "[ERROR] Sessione interrotta"
                      : tc.result,
                })),
                timestamp: (m.timestamp || 0) * 1000,
                // streaming flag intentionally omitted — treat as completed.
              };
            });
            syncMessages(lastSession.id);
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
        refreshSessionList();
      }
    });
  }, []);

  // Refresh session list from backend
  const refreshSessionList = useCallback(async () => {
    try {
      const list = await listChatSessions();
      setSessions(list);
    } catch {
      // ignore
    }
  }, []);

  // ── Core send (no queue check) ──────────────────────────
  const doSend = useCallback(
    async (text: string, images?: PastedImage[]) => {
      if (!activeRef.current) return;
      const sid = activeRef.current;
      setError(null);
      setStreaming(true);
      setActiveTool(null);

      // Add user message
      const data = getSessionData(sid);
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
        setStreaming(false);
        setError(String(e));
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

        if (sid === activeRef.current) setActiveThinking(true);

        const msgs = data.messages;
        const last = msgs[msgs.length - 1];
        if (last && last.role === "assistant" && last.streaming) {
          last.thinking = (last.thinking || "") + event.payload.thinking;
        } else {
          msgs.push({
            id: nextMsgId(),
            role: "assistant",
            content: "",
            thinking: event.payload.thinking,
            toolCalls: [],
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

        if (sid === activeRef.current) {
          setActiveTool(null);
          setActiveThinking(false);
          setActiveFile(null);
        }

        const msgs = data.messages;
        const last = msgs[msgs.length - 1];
        if (last && last.role === "assistant" && last.streaming) {
          last.content += event.payload.text;
        } else {
          msgs.push({
            id: nextMsgId(),
            role: "assistant",
            content: event.payload.text,
            toolCalls: [],
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

        if (sid === activeRef.current) setActiveTool(event.payload.tool_name);

        const msgs = data.messages;
        const last = msgs[msgs.length - 1];
        const tc: ToolCallInfo = {
          name: event.payload.tool_name,
          id: event.payload.tool_id,
          input: "",
          result: undefined,
          startedAt: Date.now(),
        };
        if (last && last.role === "assistant") {
          last.toolCalls.push(tc);
        } else {
          msgs.push({
            id: nextMsgId(),
            role: "assistant",
            content: "",
            toolCalls: [tc],
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
          const lastTc = last.toolCalls[last.toolCalls.length - 1];
          lastTc.input += event.payload.partial_json;

          // Extract file path from accumulated tool input
          if (sid === activeRef.current) {
            try {
              const fileMatch = lastTc.input.match(/"(?:file_path|path|notebook_path)"\s*:\s*"([^"]+)"/);
              if (fileMatch) {
                const fullPath = fileMatch[1];
                const fileName = fullPath.split(/[/\\]/).pop() || fullPath;
                setActiveFile(fileName);
              }
            } catch { /* ignore partial JSON parse errors */ }
          }

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

        const msgs = data.messages;
        const last = msgs[msgs.length - 1];
        if (last && last.role === "assistant" && last.streaming) {
          last.streaming = false;
          // Tools that never got a tool_result (SDK ended turn without one)
          // would otherwise pulse "pending" forever until the 2-min watchdog
          // fires. Mark them as failed so the timeline reflects the truth.
          for (const tc of last.toolCalls) {
            if (tc.result === undefined || tc.result === null) {
              tc.result = "[ERROR] Nessuna risposta dal tool";
            }
          }
        }

        if (sid === activeRef.current) {
          setActiveTool(null);
          setActiveThinking(false);
          setActiveFile(null);
          syncMessages(sid);

          // Send next queued message, or stop streaming
          const next = pendingQueue.current.shift();
          if (next) {
            // Small delay to let UI update
            setTimeout(() => doSend(next.text, next.images), 100);
          } else {
            setStreaming(false);
          }
        }

        // Refresh session list (title may have updated)
        refreshSessionList();
      })
    );

    // Usage
    unlisteners.push(
      listen<UsageEvent>("claude-usage", (event) => {
        const sid = event.payload.session_id;
        if (!sid) return;
        const data = getSessionData(sid);

        // Track context window % — only from the final result event (total_cost_usd > 0)
        // to avoid fluctuations from intermediate usage_update events during agentic loops
        if (sid === activeRef.current && event.payload.input_tokens > 0 && event.payload.total_cost_usd > 0) {
          setLastInputTokens(event.payload.input_tokens);
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

        if (sid === activeRef.current) {
          // Clear queued messages — they'd be sent into a broken state
          pendingQueue.current = [];
          setStreaming(false);
          setActiveTool(null);
          setActiveThinking(false);
          setActiveFile(null);
          setError(event.payload.error);
          syncMessages(sid);
        }
      })
    );

    return () => {
      unlisteners.forEach((p) => p.then((fn) => fn()));
    };
  }, [refreshSessionList, doSend]);

  // ── Tool timeout watchdog ──────────────────────────────────
  // Every 5s, check if any running tool has exceeded the timeout.
  // If so, mark it as timed out and auto-trigger stop.
  const TOOL_TIMEOUT_MS = 120_000; // 2 minutes
  const streamingRef = useRef(false);
  streamingRef.current = streaming;

  useEffect(() => {
    const interval = setInterval(() => {
      if (!streamingRef.current || !activeRef.current) return;
      const data = sessionDataRef.current.get(activeRef.current);
      if (!data) return;

      const now = Date.now();
      let timedOut = false;

      for (const msg of data.messages) {
        for (const tc of msg.toolCalls) {
          if (tc.result === undefined && tc.startedAt && now - tc.startedAt > TOOL_TIMEOUT_MS) {
            tc.result = "[TIMEOUT] Tool did not respond within 2 minutes. The MCP server may be unresponsive.";
            timedOut = true;
          }
        }
      }

      if (timedOut) {
        // Auto-stop the hanging session
        if (activeRef.current) {
          stopChatCmd(activeRef.current).catch(() => {});
        }
        setStreaming(false);
        setActiveTool(null);
        setActiveFile(null);
        setActiveThinking(false);
        setError("Tool call timed out — MCP server may be unresponsive");
        syncMessages(activeRef.current!);
      }
    }, 5000);

    return () => clearInterval(interval);
  }, []);

  // ── Actions ────────────────────────────────────────────────

  const sendMessage = useCallback(
    async (text: string, images?: PastedImage[]) => {
      if (!activeRef.current) return;
      if (streaming) {
        // Queue the message — it will be sent when streaming ends
        const data = getSessionData(activeRef.current);
        data.messages.push({
          id: nextMsgId(),
          role: "user",
          content: text,
          images: images?.map((img) => ({ dataUrl: img.dataUrl, name: img.name })),
          toolCalls: [],
          timestamp: Date.now(),
        });
        syncMessages(activeRef.current);
        pendingQueue.current.push({ text, images });
        return;
      }
      await doSend(text, images);
    },
    [streaming, doSend]
  );

  const stopStreaming = useCallback(async () => {
    if (!activeRef.current) return;

    // Mark any running tools as stopped so they don't show as "running" forever
    const data = sessionDataRef.current.get(activeRef.current);
    if (data) {
      for (const msg of data.messages) {
        if (msg.streaming) msg.streaming = false;
        for (const tc of msg.toolCalls) {
          if (tc.result === undefined) {
            tc.result = "[Stopped]";
          }
        }
      }
      syncMessages(activeRef.current);
    }

    try {
      await stopChatCmd(activeRef.current);
    } catch {
      // ignore
    }
    setStreaming(false);
    setActiveTool(null);
    setActiveFile(null);
    setActiveThinking(false);
  }, []);

  const newSession = useCallback(async () => {
    const id = await createChatSession();
    setActiveSessionId(id);
    activeRef.current = id;
    getSessionData(id);
    setMessages([]);
    setStreaming(false);
    setError(null);
    setActiveTool(null);
    setActiveFile(null);
    setLastInputTokens(0);
    await refreshSessionList();
  }, [refreshSessionList]);

  /** Compact: reset SDK context window but keep the same UI session. */
  const compactSession = useCallback(async () => {
    const sid = activeRef.current;
    if (!sid) return;
    await compactChatSessionCmd(sid);
    setLastInputTokens(0);
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
            data.messages = msgs.map((m: any) => {
              const interrupted = m.streaming === true;
              return {
                id: m.id || nextMsgId(),
                role: m.role as "user" | "assistant" | "system",
                content: m.content,
                thinking: m.thinking || undefined,
                toolCalls: (m.tool_calls || []).map((tc: any) => ({
                  name: tc.name,
                  id: tc.name,
                  input: tc.input || "",
                  result:
                    interrupted && (tc.result === null || tc.result === undefined)
                      ? "[ERROR] Sessione interrotta"
                      : tc.result,
                })),
                timestamp: (m.timestamp || 0) * 1000,
              };
            });
          }
        } catch {
          // ignore
        }
      }

      setMessages([...data.messages]);
      setStreaming(false);
      setError(null);
      setActiveTool(null);
    },
    []
  );

  const deleteSession = useCallback(
    async (targetId: string) => {
      cancelFlush(targetId);
      try {
        await deleteChatSessionCmd(targetId);
      } catch {
        // ignore
      }
      sessionDataRef.current.delete(targetId);

      // If deleting active session, switch to another or create new
      if (targetId === activeRef.current) {
        const remaining = sessions.filter((s) => s.id !== targetId);
        if (remaining.length > 0) {
          switchSession(remaining[0].id);
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

  /** Change the active model */
  const changeModel = useCallback(async (newModel: string) => {
    try {
      await setChatModelCmd(newModel);
      setModel(newModel);
    } catch {
      // ignore
    }
  }, []);

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
    lastInputTokens,
    activeFile,

    // Session management
    sessions,
    switchSession,
    deleteSession,
    renameSession,
    newSession,
    compactSession,

    // Actions
    sendMessage,
    stopStreaming,
    getUsage,
    addSystemMessage,
    clearMessages,
    changeModel,
  };
}
