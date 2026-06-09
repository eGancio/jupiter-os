// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

import { useEffect, useRef, useState, useCallback } from "react";
import { ChatMessage } from "../chat/ChatMessage";
import { InputBar } from "../chat/InputBar";
import { COMMANDS } from "../../lib/commands";
import { getClaudeMdStatus, getChatEngine } from "../../lib/tauri";
import { useT } from "../../i18n";
import type { ServiceInfo } from "../../types";
import type { useChat } from "../../hooks/useChat";

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
  const [permissionMode, setPermissionMode] = useState<"auto" | "ask" | "plan">("auto");
  const [claudeMdExists, setClaudeMdExists] = useState(false);
  const [claudeMdPath, setClaudeMdPath] = useState("");
  const [engine, setEngine] = useState("claude");

  // Check CLAUDE.md status on mount
  useEffect(() => {
    getClaudeMdStatus()
      .then((status) => {
        setClaudeMdExists(status.exists);
        setClaudeMdPath(status.path);
      })
      .catch(() => { /* ignore */ });
  }, []);

  // Active engine for the tab-bar badge (model comes from chat.model, reactive).
  // Re-read on session change too: switching engine starts a NEW session, so the
  // badge must refresh (otherwise it stays stuck on the mount-time value).
  useEffect(() => {
    getChatEngine().then(setEngine).catch(() => { /* default claude */ });
  }, [chat.sessionId]);

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
          const m = args.trim().toLowerCase();
          if (!m || !["haiku", "sonnet", "opus"].includes(m)) {
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
          if (permissionMode === "plan") {
            setPermissionMode("auto");
            addSystemMessage(t("cmd.plan.off"));
          } else {
            setPermissionMode("plan");
            addSystemMessage(t("cmd.plan.on"));
          }
          break;
        }

        default:
          addSystemMessage(t("cmd.unknown", { name }));
      }
    },
    [newSession, stopStreaming, getUsage, addSystemMessage, clearMessages, services, messages, sendMessage, permissionMode, changeModel, t]
  );

  // Wrap sendMessage to prepend plan instruction when in plan mode
  const handleSend = useCallback(
    (text: string, images?: any[]) => {
      // Sending your own message always re-pins to the bottom.
      pinnedToBottom.current = true;
      if (permissionMode === "plan") {
        sendMessage("Plan before implementing, show the plan and wait for approval.\n\n" + text, images);
        setPermissionMode("auto");
      } else {
        sendMessage(text, images);
      }
    },
    [permissionMode, sendMessage]
  );

  // Cycle permission mode (silent — no chat message)
  const cyclePermissionMode = useCallback(() => {
    setPermissionMode((prev) =>
      prev === "auto" ? "ask" : prev === "ask" ? "plan" : "auto"
    );
  }, []);

  // Show waiting indicator if streaming but no assistant message yet
  const showWaiting =
    streaming &&
    (messages.length === 0 || messages[messages.length - 1].role === "user");

  // Show activity bar when streaming with a tool active (assistant msg already exists)
  const showActivity = streaming && !showWaiting && activeTool !== null;

  return (
    <div className="flex-1 flex flex-col min-w-0 bg-jupiter-bg">
      {/* Tab bar */}
      <div className="h-9 border-b border-jupiter-orange/25 flex items-center px-2 gap-1 bg-jupiter-surface/50 flex-shrink-0">
        <div className="px-3 py-1 text-[11px] rounded bg-jupiter-elevated text-white flex items-center gap-1.5">
          <span className={`w-1.5 h-1.5 rounded-full ${streaming ? "bg-jupiter-green animate-pulse" : "bg-jupiter-dim"}`} />
          {t("chat.tab")}
        </div>
        <button
          onClick={newSession}
          className="px-2 py-1 text-[11px] text-jupiter-dim hover:text-jupiter-blue hover:bg-jupiter-elevated rounded transition-colors"
          title={t("chat.newChat")}
        >
          +
        </button>

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

        {/* Engine & model settings */}
        <button
          onClick={onOpenEngineSettings}
          className="ml-auto px-2 py-0.5 text-[10px] rounded bg-jupiter-blue/10 text-jupiter-blue border border-jupiter-blue/25 hover:bg-jupiter-blue/20 flex items-center gap-1 transition-colors font-mono"
          title="Engine & model"
        >
          <span className="text-[11px]">&#9881;</span>
          {engine} · {chat.model}
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
        onPermissionModeChange={cyclePermissionMode}
        activeTool={activeTool}
        contextPercent={lastInputTokens > 0 ? Math.min(100, Math.round(lastInputTokens / 2000)) : 0}
        activeFile={activeFile}
      />
    </div>
  );
}

function WaitingIndicator({ activeTool, activeThinking }: { activeTool: string | null; activeThinking: boolean }) {
  const { t } = useT();
  const moon = activeTool ? toolToMoon(activeTool) : null;
  const toolName = activeTool ? toolDisplayName(activeTool) : null;

  return (
    <div className="flex justify-start">
      <div className="bg-transparent text-white rounded-lg px-4 py-2.5">
        <div className="flex items-center gap-2.5">
          <div className="flex gap-1">
            <span className={`w-1.5 h-1.5 rounded-full ${activeThinking ? "bg-jupiter-blue" : "bg-jupiter-blue"} animate-bounce`} style={{ animationDelay: "0ms" }} />
            <span className={`w-1.5 h-1.5 rounded-full ${activeThinking ? "bg-jupiter-blue" : "bg-jupiter-blue"} animate-bounce`} style={{ animationDelay: "150ms" }} />
            <span className={`w-1.5 h-1.5 rounded-full ${activeThinking ? "bg-jupiter-blue" : "bg-jupiter-blue"} animate-bounce`} style={{ animationDelay: "300ms" }} />
          </div>
          {activeTool ? (
            <span className="text-[0.85em] text-jupiter-muted">
              {t("chat.using")} <span className="font-mono text-jupiter-amber">{toolName}</span>
              {moon && (
                <> {t("chat.on")} <span className="text-jupiter-blue">{moon}</span></>
              )}
            </span>
          ) : activeThinking ? (
            <span className="text-[0.85em] text-jupiter-blue">{t("chat.thinking")}</span>
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
  const moon = activeTool ? toolToMoon(activeTool) : null;
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
