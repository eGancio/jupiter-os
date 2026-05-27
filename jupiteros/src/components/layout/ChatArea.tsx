import { useEffect, useRef, useState, useCallback } from "react";
import { ChatMessage } from "../chat/ChatMessage";
import { InputBar } from "../chat/InputBar";
import { COMMANDS } from "../../lib/commands";
import { getClaudeMdStatus } from "../../lib/tauri";
import type { useChat } from "../../hooks/useChat";
import type { ServiceInfo } from "../../types";

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
}

export function ChatArea({ chat, services, onPreviewChart }: Props) {
  const { messages, streaming, sendMessage, stopStreaming, newSession, activeTool, activeThinking, getUsage, addSystemMessage, clearMessages, lastInputTokens, changeModel, activeFile } = chat;
  const scrollRef = useRef<HTMLDivElement>(null);
  const [zoom, setZoom] = useState(1);
  const [permissionMode, setPermissionMode] = useState<"auto" | "ask" | "plan">("auto");
  const [claudeMdExists, setClaudeMdExists] = useState(false);
  const [claudeMdPath, setClaudeMdPath] = useState("");

  // Check CLAUDE.md status on mount
  useEffect(() => {
    getClaudeMdStatus()
      .then((status) => {
        setClaudeMdExists(status.exists);
        setClaudeMdPath(status.path);
      })
      .catch(() => { /* ignore */ });
  }, []);

  // Auto-scroll to bottom on new messages or active tool change
  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [messages, activeTool, activeThinking]);

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
            addSystemMessage("Usage: `/model haiku|sonnet|opus`");
          } else {
            changeModel(m).then(() => {
              addSystemMessage(`Model changed to **${m}**. Takes effect from the next message.`);
            }).catch((e: unknown) => {
              addSystemMessage(`Model change error: ${e}`);
            });
          }
          break;
        }

        case "/cost": {
          const usage = getUsage();
          const lines = [
            "**Current session costs**",
            "",
            `| Metric | Value |`,
            `|--------|-------|`,
            `| Input tokens | ${usage.inputTokens.toLocaleString()} |`,
            `| Output tokens | ${usage.outputTokens.toLocaleString()} |`,
            `| Total cost | $${usage.totalCostUsd.toFixed(4)} |`,
          ];
          addSystemMessage(lines.join("\n"));
          break;
        }

        case "/help": {
          const lines = [
            "**Available commands**",
            "",
            ...COMMANDS.map((c) => `- \`${c.name}\` — ${c.description}${c.usage ? ` (${c.usage})` : ""}`),
          ];
          addSystemMessage(lines.join("\n"));
          break;
        }

        case "/mcp": {
          if (!services || services.length === 0) {
            addSystemMessage("No MCP servers configured.");
          } else {
            const lines = [
              "**Moons Status (MCP Servers)**",
              "",
              `| Moon | Status | Type |`,
              `|------|--------|------|`,
              ...services.map((s) =>
                `| ${s.name} | ${s.running ? "Active" : "Stopped"} | ${s.kind === "McpServer" ? "MCP" : "Daemon"} |`
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
            addSystemMessage("Context compacted — context window reset.");

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
            addSystemMessage("**Plan mode deactivated.** Switched to Edit automatically.");
          } else {
            setPermissionMode("plan");
            addSystemMessage("**Plan mode activated.** The next message will include the plan instruction.");
          }
          break;
        }

        default:
          addSystemMessage(`Unknown command: \`${name}\``);
      }
    },
    [newSession, stopStreaming, getUsage, addSystemMessage, clearMessages, services, messages, sendMessage, permissionMode, changeModel]
  );

  // Wrap sendMessage to prepend plan instruction when in plan mode
  const handleSend = useCallback(
    (text: string, images?: any[]) => {
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
          Chat
        </div>
        <button
          onClick={newSession}
          className="px-2 py-1 text-[11px] text-jupiter-dim hover:text-jupiter-blue hover:bg-jupiter-elevated rounded transition-colors"
          title="New chat"
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

        {/* Zoom controls */}
        <div className="ml-auto flex items-center gap-0.5">
          <button
            onClick={zoomOut}
            className="w-6 h-6 flex items-center justify-center text-[13px] text-jupiter-dim hover:text-white hover:bg-jupiter-elevated rounded transition-colors"
            title="Zoom out (Ctrl+-)"
          >
            −
          </button>
          <button
            onClick={zoomReset}
            className="px-1.5 h-6 flex items-center justify-center text-[10px] text-jupiter-dim hover:text-white hover:bg-jupiter-elevated rounded transition-colors font-mono"
            title="Reset zoom (Ctrl+0)"
          >
            {Math.round(zoom * 100)}%
          </button>
          <button
            onClick={zoomIn}
            className="w-6 h-6 flex items-center justify-center text-[13px] text-jupiter-dim hover:text-white hover:bg-jupiter-elevated rounded transition-colors"
            title="Zoom in (Ctrl++)"
          >
            +
          </button>
        </div>
      </div>

      {/* Chat messages area */}
      <div
        ref={scrollRef}
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
              Using <span className="font-mono text-jupiter-amber">{toolName}</span>
              {moon && (
                <> on <span className="text-jupiter-blue">{moon}</span></>
              )}
            </span>
          ) : activeThinking ? (
            <span className="text-[0.85em] text-jupiter-blue">Thinking...</span>
          ) : (
            <span className="text-[0.85em] text-jupiter-muted">Waiting...</span>
          )}
        </div>
      </div>
    </div>
  );
}

function ActivityBar({ activeTool }: { activeTool: string | null }) {
  const moon = activeTool ? toolToMoon(activeTool) : null;
  const toolName = activeTool ? toolDisplayName(activeTool) : null;

  return (
    <div className="flex justify-start">
      <div className="bg-jupiter-elevated/30 border border-jupiter-orange/15 rounded-lg px-3 py-1.5 flex items-center gap-2">
        <span className="w-1.5 h-1.5 rounded-full bg-jupiter-amber animate-pulse" />
        <span className="text-[0.8em] text-jupiter-muted">
          Running <span className="font-mono text-jupiter-amber">{toolName}</span>
          {moon && (
            <> on <span className="text-jupiter-blue">{moon}</span></>
          )}
        </span>
      </div>
    </div>
  );
}

function EmptyState() {
  return (
    <div className="flex-1 flex flex-col items-center justify-center text-jupiter-dim h-full">
      <div className="text-center max-w-md space-y-4">
        <img src="/logo.png" alt="JupiterOS" className="w-16 h-16 mx-auto opacity-80" />
        <div>
          <h2 className="text-lg font-bold text-white tracking-wide font-display">JupiterOS</h2>
          <p className="text-[11px] text-jupiter-blue tracking-widest font-display">jupiteros.ai</p>
        </div>
        <p className="text-sm leading-relaxed text-jupiter-muted">
          Type a message to start a conversation with Claude.
          <br />
          MCP servers (Moons) are active in the sidebar.
        </p>
      </div>
    </div>
  );
}
