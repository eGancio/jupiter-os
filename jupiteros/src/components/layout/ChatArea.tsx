// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

import { useEffect, useRef, useState, useCallback } from "react";
import { ChatMessage } from "../chat/ChatMessage";
import { InputBar } from "../chat/InputBar";
import { ChatTabs } from "../chat/ChatTabs";
import { getMoonLabel } from "../../lib/moonInfo";
import { COMMANDS } from "../../lib/commands";
import { getClaudeMdStatus, setChatPermissionMode, type ChatPermissionMode } from "../../lib/tauri";
import { useT } from "../../i18n";
import type { ServiceInfo } from "../../types";
import { MAX_TABS, type useChat } from "../../hooks/useChat";

const ZOOM_MIN = 0.7;
const ZOOM_MAX = 1.5;
const ZOOM_STEP = 0.1;

/** Map MCP tool prefix to Moon name */
function toolToMoon(toolName: string): string | null {
  const parts = toolName.split("__");
  if (parts.length >= 2) return parts[1]; // mcp__memory__list_emails → memory
  return null;
}

/** Friendly display for tool name */
function toolDisplayName(toolName: string): string {
  const parts = toolName.split("__");
  return parts.length >= 3 ? parts[parts.length - 1] : toolName;
}

interface Props {
  chat: ReturnType<typeof useChat>;
  services?: ServiceInfo[];
  onPreviewChart?: (filePath: string) => void;
  onOpenEngineSettings?: () => void;
}

export function ChatArea({ chat, services, onPreviewChart, onOpenEngineSettings }: Props) {
  const { messages, streaming, sendMessage, stopStreaming, newSession, activeTool, activeThinking, getUsage, addSystemMessage, clearMessages, lastInputTokens, changeModel, activeFile } = chat;
  const { t } = useT();
  const scrollRef = useRef<HTMLDivElement>(null);
  // True while the user is "stuck" at the bottom. Set to false as soon as they
  // scroll up, so streaming doesn't yank the viewport down while they read.
  const pinnedToBottom = useRef(true);
  const [zoom, setZoom] = useState(1);
  const [permissionMode] = useState<ChatPermissionMode>("auto");
  const [claudeMdExists, setClaudeMdExists] = useState(false);
  const [claudeMdPath, setClaudeMdPath] = useState("");

  // Engine badge: derived from the active tab's session (single source of truth)
  const activeEngine =
    chat.sessions.find((s) => s.id === chat.sessionId)?.engine ?? "claude";

  // Check CLAUDE.md status on mount
  useEffect(() => {
    getClaudeMdStatus()
      .then((status) => {
        setClaudeMdExists(status.exists);
        setClaudeMdPath(status.path);
      })
      .catch(() => { /* ignore */ });
  }, []);

  // Per-tab scroll: when switching tab, land pinned to the bottom of the
  // target conversation (reading position is not preserved across tabs).
  useEffect(() => {
    pinnedToBottom.current = true;
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [chat.sessionId]);

  // Push the permission mode to the backend whenever it changes (Claude reads
  // it per-turn; other engines ignore it). Keeps the SDK mode in sync with the
  // toggle without threading it through every sendMessage call.
  useEffect(() => {
    setChatPermissionMode(permissionMode).catch(() => { /* ignore */ });
  }, [permissionMode]);

  // Auto-scroll to bottom on new messages or active tool change — but ONLY when
  // the user is already pinned to the bottom. If they scrolled up to read, we
  // leave the viewport alone so reading and streaming stay independent.
  useEffect(() => {
    if (scrollRef.current && pinnedToBottom.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [messages, activeTool, activeThinking]);

  // Track whether the user is at (or very near) the bottom. A small threshold
  // tolerates sub-pixel rounding and keeps "stick to bottom" feeling natural.
  const handleScroll = () => {
    const el = scrollRef.current;
    if (!el) return;
    const distanceFromBottom = el.scrollHeight - el.scrollTop - el.clientHeight;
    pinnedToBottom.current = distanceFromBottom < 40;
  };

  // Keyboard zoom: Ctrl+Plus / Ctrl+Minus / Ctrl+0
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      // Escape to stop streaming
      if (e.key === "Escape" && streaming) {
        e.preventDefault();
        stopStreaming();
        return;
      }
      if (e.ctrlKey || e.metaKey) {
        if (e.key === "=" || e.key === "+") {
          e.preventDefault();
          setZoom((z) => Math.min(+(z + ZOOM_STEP).toFixed(1), ZOOM_MAX));
        } else if (e.key === "-") {
          e.preventDefault();
          setZoom((z) => Math.max(+(z - ZOOM_STEP).toFixed(1), ZOOM_MIN));
        } else if (e.key === "0") {
          e.preventDefault();
          setZoom(1);
        }
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [streaming, stopStreaming]);

  // Ctrl + mouse wheel zoom — chat only. Never zooms the whole app.
  useEffect(() => {
    const handler = (e: WheelEvent) => {
      if (!e.ctrlKey && !e.metaKey) return;
      // Block WebKit's native whole-app zoom on Ctrl+wheel
      e.preventDefault();
      // Only adjust zoom when the wheel is over the chat messages area
      const area = scrollRef.current;
      if (!area || !(e.target instanceof Node) || !area.contains(e.target)) return;
      if (e.deltaY < 0) {
        setZoom((z) => Math.min(+(z + ZOOM_STEP).toFixed(1), ZOOM_MAX));
      } else if (e.deltaY > 0) {
        setZoom((z) => Math.max(+(z - ZOOM_STEP).toFixed(1), ZOOM_MIN));
      }
    };
    window.addEventListener("wheel", handler, { passive: false });
    return () => window.removeEventListener("wheel", handler);
  }, []);

  const zoomIn = useCallback(() => setZoom((z) => Math.min(+(z + ZOOM_STEP).toFixed(1), ZOOM_MAX)), []);
  const zoomOut = useCallback(() => setZoom((z) => Math.max(+(z - ZOOM_STEP).toFixed(1), ZOOM_MIN)), []);
  const zoomReset = useCallback(() => setZoom(1), []);

  // Slash command handler
  const handleCommand = useCallback(
    (name: string, args: string) => {
      switch (name) {
        case "/clear":
        case "/new":
          newSession();
          break;

        case "/stop":
          stopStreaming();
          break;

        case "/model": {
          let m = args.trim().toLowerCase();
          // scorciatoie per fable-5 (l'SDK vuole l'id pieno)
          if (m === "fable" || m === "fable-5") m = "claude-fable-5";
          if (!m || !["haiku", "sonnet", "opus", "claude-fable-5"].includes(m)) {
            addSystemMessage(t("cmd.model.usage"));
          } else {
            changeModel(m).then(() => {
              addSystemMessage(t("cmd.model.changed", { model: m }));
            }).catch((e: unknown) => {
              addSystemMessage(t("cmd.model.error", { error: String(e) }));
            });
          }
          break;
        }

        case "/cost": {
          const usage = getUsage();
          const lines = [
            t("cmd.cost.title"),
            "",
            `| ${t("cmd.cost.metric")} | ${t("cmd.cost.value")} |`,
            `|--------|-------|`,
            `| ${t("cmd.cost.inputTokens")} | ${usage.inputTokens.toLocaleString()} |`,
            `| ${t("cmd.cost.outputTokens")} | ${usage.outputTokens.toLocaleString()} |`,
            `| ${t("cmd.cost.totalCost")} | $${usage.totalCostUsd.toFixed(4)} |`,
          ];
          addSystemMessage(lines.join("\n"));
          break;
        }

        case "/help": {
          const lines = [
            t("cmd.help.title"),
            "",
            ...COMMANDS.map((c) => `- \`${c.name}\` — ${t(c.descKey)}${c.usage ? ` (${c.usage})` : ""}`),
          ];
          addSystemMessage(lines.join("\n"));
          break;
        }

        case "/mcp": {
          if (!services || services.length === 0) {
            addSystemMessage(t("cmd.mcp.none"));
          } else {
            const lines = [
              t("cmd.mcp.title"),
              "",
              `| ${t("cmd.mcp.colMoon")} | ${t("cmd.mcp.colStatus")} | ${t("cmd.mcp.colType")} |`,
              `|------|--------|------|`,
              ...services.map((s) =>
                `| ${s.name} | ${s.running ? t("cmd.mcp.active") : t("cmd.mcp.stopped")} | ${s.kind === "McpServer" ? "MCP" : "Daemon"} |`
              ),
            ];
            addSystemMessage(lines.join("\n"));
          }
          break;
        }

        case "/compact":
          // Compact: reset SDK context window but stay in the same chat session.
          // Gathers a summary of recent messages and sends it as context to the fresh session.
          (async () => {
            const recentMsgs = chat.messages.slice(-10);
            const summaryParts = recentMsgs
              .filter((m) => m.role !== "system")
              .map((m) => `${m.role}: ${m.content.slice(0, 300)}`)
              .join("\n");

            await chat.compactSession();
            addSystemMessage(t("cmd.compact.done"));

            if (summaryParts) {
              sendMessage(
                `[Context from previous conversation — do NOT repeat this, just acknowledge briefly and continue]\n\n${summaryParts}\n\nContinue the conversation naturally.`
              );
            }
          })();
          break;

        case "/plan": {
          // Plan mode needs the approval-popup UI (not built yet). For now the
          // chat runs in a single real "Auto mode" (does everything).
          addSystemMessage(t("cmd.plan.unavailable"));
          break;
        }

        default:
          addSystemMessage(t("cmd.unknown", { name }));
      }
    },
    [newSession, stopStreaming, getUsage, addSystemMessage, clearMessages, services, messages, sendMessage, permissionMode, changeModel, t]
  );

  // Plan mode is now the real SDK permission mode (read-only planning enforced
  // by the engine), so no prompt-prepend hack and no auto-reset is needed.
  const handleSend = useCallback(
    (text: string, images?: any[]) => {
      // Sending your own message always re-pins to the bottom.
      pinnedToBottom.current = true;
      sendMessage(text, images);
    },
    [sendMessage]
  );

  // Single mode for now ("Auto mode" = do everything). No cycling until the
  // plan / interactive modes get their approval-popup UI.

  // Show waiting indicator if streaming but no assistant message yet
  const showWaiting =
    streaming &&
    (messages.length === 0 || messages[messages.length - 1].role === "user");

  // Show activity bar when streaming with a tool active (assistant msg already exists)
  const showActivity = streaming && !showWaiting && activeTool !== null;

  return (
    <div className="flex-1 flex flex-col min-w-0 bg-jupiter-bg">
      <div className="flex-1 min-h-0 p-6 flex flex-col overflow-hidden">
        {/* Unified iridescent gradient frame */}
        <div className="unified-gradient-container flex-1 min-h-0 flex flex-col">
          <div className="bg-jupiter-elevated rounded-[23px] flex-1 flex flex-col overflow-hidden">
      {/* Tab bar */}
      <div className="h-11 border-b border-jupiter-orange/25 flex items-center px-2 gap-1.5 bg-jupiter-surface/50 flex-shrink-0">
        <ChatTabs
          tabs={chat.tabs}
          activeId={chat.sessionId}
          sessions={chat.sessions}
          tabStatus={chat.tabStatus}
          onSelect={chat.switchSession}
          onClose={chat.closeTab}
          onNew={newSession}
          maxTabs={MAX_TABS}
        />

        {/* CLAUDE.md indicator */}
        {claudeMdExists && (
          <div
            className="px-2 py-0.5 text-[10px] rounded bg-jupiter-green/15 text-jupiter-green border border-jupiter-green/30 flex items-center gap-1"
            title={claudeMdPath}
          >
            <span className="w-1.5 h-1.5 rounded-full bg-jupiter-green" />
            CLAUDE.md
          </div>
        )}

        {/* Engine & model settings (engine of the ACTIVE tab's session) */}
        <button
          onClick={onOpenEngineSettings}
          className="ml-auto px-2 py-0.5 text-[10px] rounded bg-jupiter-blue/10 text-jupiter-blue border border-jupiter-blue/25 hover:bg-jupiter-blue/20 flex items-center gap-1 transition-colors font-mono"
          title="Engine & model"
        >
          <span className="text-[11px]">&#9881;</span>
          {activeEngine} · {chat.model}
        </button>

        {/* Zoom controls */}
        <div className="flex items-center gap-0.5">
          <button
            onClick={zoomOut}
            className="w-6 h-6 flex items-center justify-center text-[13px] text-jupiter-dim hover:text-white hover:bg-jupiter-elevated rounded transition-colors"
            title={t("chat.zoomOut")}
          >
            −
          </button>
          <button
            onClick={zoomReset}
            className="px-1.5 h-6 flex items-center justify-center text-[10px] text-jupiter-dim hover:text-white hover:bg-jupiter-elevated rounded transition-colors font-mono"
            title={t("chat.zoomReset")}
          >
            {Math.round(zoom * 100)}%
          </button>
          <button
            onClick={zoomIn}
            className="w-6 h-6 flex items-center justify-center text-[13px] text-jupiter-dim hover:text-white hover:bg-jupiter-elevated rounded transition-colors"
            title={t("chat.zoomIn")}
          >
            +
          </button>
        </div>
      </div>

      {/* Chat messages area */}
      <div
        ref={scrollRef}
        onScroll={handleScroll}
        className="flex-1 overflow-y-auto px-4 py-3 space-y-4 min-h-0"
        style={{ fontSize: `${14 * zoom}px` }}
      >
        {messages.length === 0 && !streaming ? (
          <EmptyState />
        ) : (
          <>
            {messages.map((m) => (
              <ChatMessage key={m.id} message={m} onPreviewChart={onPreviewChart} />
            ))}
            {showWaiting && <WaitingIndicator activeTool={activeTool} activeThinking={activeThinking} />}
            {showActivity && <ActivityBar activeTool={activeTool} />}
          </>
        )}
      </div>

      {/* Input bar */}
      <InputBar
        onSend={handleSend}
        onStop={stopStreaming}
        onCommand={handleCommand}
        disabled={streaming}
        streaming={streaming}
        zoom={zoom}
        permissionMode={permissionMode}
        showPermissionToggle={activeEngine === "claude"}
        activeTool={activeTool}
        contextPercent={lastInputTokens > 0 ? Math.min(100, Math.round(lastInputTokens / 2000)) : 0}
        activeFile={activeFile}
      />
          </div>
        </div>
      </div>
    </div>
  );
}

function WaitingIndicator({ activeTool, activeThinking }: { activeTool: string | null; activeThinking: boolean }) {
  const { t } = useT();
  const moonCode = activeTool ? toolToMoon(activeTool) : null;
  const moon = moonCode ? getMoonLabel(moonCode, t) : null;
  const toolName = activeTool ? toolDisplayName(activeTool) : null;

  // Thinking → violet dots (same language as ThinkingBlock); tool/waiting → orange.
  const dotCls = activeThinking ? "bg-jupiter-violet" : "bg-jupiter-orange";
  return (
    <div className="flex gap-3 justify-start">
      <div className="assistant-orb flex-shrink-0 mt-1" />
      <div className="bg-transparent text-white rounded-lg py-2">
        <div className="flex items-center gap-2.5">
          <div className="flex gap-1">
            <span className={`w-1.5 h-1.5 rounded-full ${dotCls} animate-bounce`} style={{ animationDelay: "0ms" }} />
            <span className={`w-1.5 h-1.5 rounded-full ${dotCls} animate-bounce`} style={{ animationDelay: "150ms" }} />
            <span className={`w-1.5 h-1.5 rounded-full ${dotCls} animate-bounce`} style={{ animationDelay: "300ms" }} />
          </div>
          {activeTool ? (
            <span className="text-[0.85em] text-jupiter-muted">
              {t("chat.using")} <span className="font-mono text-jupiter-orange">{toolName}</span>
              {moon && (
                <> {t("chat.on")} <span className="text-jupiter-primary">{moon}</span></>
              )}
            </span>
          ) : activeThinking ? (
            <span className="text-[0.85em] text-jupiter-violet">{t("chat.thinking")}</span>
          ) : (
            <span className="text-[0.85em] text-jupiter-muted">{t("chat.waiting")}</span>
          )}
        </div>
      </div>
    </div>
  );
}

function ActivityBar({ activeTool }: { activeTool: string | null }) {
  const { t } = useT();
  const moonCode = activeTool ? toolToMoon(activeTool) : null;
  const moon = moonCode ? getMoonLabel(moonCode, t) : null;
  const toolName = activeTool ? toolDisplayName(activeTool) : null;

  return (
    <div className="flex justify-start">
      <div className="bg-jupiter-elevated/30 border border-jupiter-orange/15 rounded-lg px-3 py-1.5 flex items-center gap-2">
        <span className="w-1.5 h-1.5 rounded-full bg-jupiter-amber animate-pulse" />
        <span className="text-[0.8em] text-jupiter-muted">
          {t("chat.running")} <span className="font-mono text-jupiter-amber">{toolName}</span>
          {moon && (
            <> {t("chat.on")} <span className="text-jupiter-blue">{moon}</span></>
          )}
        </span>
      </div>
    </div>
  );
}

function EmptyState() {
  const { t } = useT();
  return (
    <div className="flex-1 flex flex-col items-center justify-center text-jupiter-dim h-full">
      <div className="text-center max-w-md space-y-4">
        <img src="/logo.png" alt="JupiterOS" className="w-16 h-16 mx-auto opacity-80" />
        <div>
          <h2 className="text-lg font-bold text-white tracking-wide font-display">JupiterOS</h2>
          <p className="text-[11px] text-jupiter-blue tracking-widest font-display">jupiteros.ai</p>
        </div>
        <p className="text-sm leading-relaxed text-jupiter-muted">
          {t("chat.empty.body1")}
          <br />
          {t("chat.empty.body2")}
        </p>
      </div>
    </div>
  );
}
