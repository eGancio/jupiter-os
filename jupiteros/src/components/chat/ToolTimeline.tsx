import { useEffect, useState } from "react";
import type { ToolCallInfo } from "../../types";

interface Props {
  toolCalls: ToolCallInfo[];
  onPreviewChart?: (filePath: string) => void;
}

type Status = "pending" | "ok" | "err";

interface ToolGroup {
  server: string;       // "io", "europa", "amalthea", "dashops", "clickup", "local", …
  bareName: string;     // "list_emails" (stripped of mcp__server__ prefix)
  calls: ToolCallInfo[];
  status: Status;
}

function parseToolName(name: string): { server: string; bareName: string } {
  if (name.startsWith("mcp__")) {
    const parts = name.split("__");
    return {
      server: parts[1] || "local",
      bareName: parts.slice(2).join("__") || name,
    };
  }
  return { server: "local", bareName: name };
}

function callStatus(tc: ToolCallInfo): Status {
  if (tc.result === undefined || tc.result === null) return "pending";
  const r = tc.result;
  if (r.startsWith("[ERROR]") || r.startsWith("[TIMEOUT]") || r === "[Stopped]") return "err";
  return "ok";
}

/** Group consecutive identical, completed tool calls — never merge a pending one. */
function groupCalls(toolCalls: ToolCallInfo[]): ToolGroup[] {
  const groups: ToolGroup[] = [];
  for (const tc of toolCalls) {
    const { server, bareName } = parseToolName(tc.name);
    const st = callStatus(tc);
    const prev = groups[groups.length - 1];
    const canMerge =
      prev &&
      prev.server === server &&
      prev.bareName === bareName &&
      prev.status === st &&
      st !== "pending";
    if (canMerge) {
      prev!.calls.push(tc);
    } else {
      groups.push({ server, bareName, calls: [tc], status: st });
    }
  }
  return groups;
}

/** Pick a tailwind text colour for the server badge. */
function serverBadgeClass(server: string): string {
  switch (server) {
    case "io":
      return "bg-cyan-500/15 text-cyan-300";
    case "europa":
      return "bg-violet-500/15 text-violet-300";
    case "amalthea":
      return "bg-amber-500/15 text-amber-300";
    case "dashops":
      return "bg-rose-500/15 text-rose-300";
    case "clickup":
      return "bg-fuchsia-500/15 text-fuchsia-300";
    case "jupiter-blog":
      return "bg-sky-500/15 text-sky-300";
    default:
      return "bg-jupiter-elevated text-jupiter-muted";
  }
}

/** Extract an HTML chart file path from a tool result. */
function extractHtmlPath(result: string | undefined | null): string | null {
  if (!result) return null;
  const win = result.match(/([A-Za-z]:\\[^\s"'`\n]+\.html)/);
  if (win) return win[1];
  const unix = result.match(/(\/[^\s"'`\n]+\.html)/);
  if (unix) return unix[1];
  return null;
}

/** Format elapsed milliseconds for a still-running tool call. */
function formatElapsed(ms: number): string {
  if (ms < 1000) return "";
  const secs = Math.floor(ms / 1000);
  if (secs < 60) return `${secs}s`;
  const mins = Math.floor(secs / 60);
  const rem = secs % 60;
  return rem > 0 ? `${mins}m ${rem}s` : `${mins}m`;
}

function PendingElapsed({ startedAt }: { startedAt: number }) {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(id);
  }, []);
  const ms = now - startedAt;
  const s = formatElapsed(ms);
  if (!s) return null;
  const warn = ms > 60_000;
  return (
    <span className={`tool-elapsed ${warn ? "tool-elapsed-warn" : ""}`}>{s}</span>
  );
}

export function ToolTimeline({ toolCalls, onPreviewChart }: Props) {
  if (toolCalls.length === 0) return null;
  const groups = groupCalls(toolCalls);

  return (
    <ul className="tool-timeline">
      {groups.map((g, idx) => {
        const count = g.calls.length;
        // Show chart icon when the most recent completed call yielded one.
        const chartPath = (() => {
          for (let i = g.calls.length - 1; i >= 0; i--) {
            const p = extractHtmlPath(g.calls[i].result);
            if (p && p.toLowerCase().includes("amalthea")) return p;
          }
          return null;
        })();
        const pendingCall = g.status === "pending" ? g.calls[g.calls.length - 1] : null;
        return (
          <li key={idx} className="tool-timeline-item">
            <span className="tool-timeline-dot" data-status={g.status} />
            <span className={`tool-badge ${serverBadgeClass(g.server)}`}>{g.server}</span>
            <span className="tool-name">{g.bareName}</span>
            {count > 1 && <span className="tool-count">×{count}</span>}
            {g.status === "ok" && <span className="tool-ok">✓</span>}
            {g.status === "err" && <span className="tool-err">✗</span>}
            {g.status === "pending" && <span className="tool-pending">…</span>}
            {pendingCall?.startedAt && (
              <PendingElapsed startedAt={pendingCall.startedAt} />
            )}
            {chartPath && onPreviewChart && (
              <button
                type="button"
                onClick={() => onPreviewChart(chartPath)}
                className="tool-chart-btn"
                title="Apri chart"
              >
                📊
              </button>
            )}
          </li>
        );
      })}
    </ul>
  );
}
