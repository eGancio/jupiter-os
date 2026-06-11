// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

import { useEffect, useState } from "react";
import { useT } from "../../i18n";
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

/** Group consecutive identical, completed tool calls — never merge a pending
 * one, nor a call with nested subagent activity (its row must stay single). */
function groupCalls(toolCalls: ToolCallInfo[]): ToolGroup[] {
  const groups: ToolGroup[] = [];
  for (const tc of toolCalls) {
    // Subagent tools render nested under their parent, never in the main flow.
    if (tc.parentToolUseId) continue;
    const { server, bareName } = parseToolName(tc.name);
    const st = callStatus(tc);
    const prev = groups[groups.length - 1];
    const canMerge =
      prev &&
      prev.server === server &&
      prev.bareName === bareName &&
      prev.status === st &&
      st !== "pending" &&
      !tc.children?.length &&
      !prev.calls.some((c) => c.children?.length);
    if (canMerge) {
      prev!.calls.push(tc);
    } else {
      groups.push({ server, bareName, calls: [tc], status: st });
    }
  }
  return groups;
}

/** Pull a human-meaningful string field out of a (possibly still-streaming)
 * tool input JSON. Full parse first, then a regex that tolerates a truncated
 * accumulation so labels appear live while the input is still being typed. */
function extractInputField(input: string, keys: string[]): string | null {
  try {
    const obj = JSON.parse(input);
    for (const k of keys) {
      if (typeof obj[k] === "string" && obj[k]) return obj[k];
    }
  } catch {
    for (const k of keys) {
      const m = input.match(new RegExp(`"${k}"\\s*:\\s*"((?:[^"\\\\]|\\\\.)*)`));
      if (m && m[1]) {
        try {
          return JSON.parse(`"${m[1]}"`);
        } catch {
          return m[1];
        }
      }
    }
  }
  return null;
}

/** Friendly one-line label for a tool call: the search query, the site being
 * read (hostname+path for URLs), or the task description for Agent calls. */
function friendlyLabel(tc: ToolCallInfo): string | null {
  const raw = extractInputField(tc.input, ["query", "url", "description", "prompt"]);
  if (!raw) return null;
  let label = raw;
  if (/^https?:\/\//i.test(raw)) {
    try {
      const u = new URL(raw);
      label = u.hostname.replace(/^www\./, "") + (u.pathname !== "/" ? u.pathname : "");
    } catch {
      /* keep raw */
    }
  }
  label = label.replace(/\s+/g, " ").trim();
  return label.length > 60 ? `${label.slice(0, 57)}…` : label;
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
  const { t } = useT();
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
        // A group holding nested subagent activity is never merged → single call.
        const children = g.calls.length === 1 ? g.calls[0].children : undefined;
        const desc = children?.length ? friendlyLabel(g.calls[0]) : null;
        return (
          <li key={idx} className="tool-timeline-group">
            <span className="tool-timeline-item">
              <span className="tool-timeline-dot" data-status={g.status} />
              <span className={`tool-badge ${serverBadgeClass(g.server)}`}>{g.server}</span>
              <span className="tool-name">{g.bareName}</span>
              {desc && <span className="tool-desc">{desc}</span>}
              {count > 1 && <span className="tool-count">×{count}</span>}
              {g.status === "ok" && (
                <span className="tool-ok">
                  <span className="material-symbols-outlined text-[13px]">check</span>
                </span>
              )}
              {g.status === "err" && (
                <span className="tool-err">
                  <span className="material-symbols-outlined text-[13px]">close</span>
                </span>
              )}
              {pendingCall?.startedAt && (
                <PendingElapsed startedAt={pendingCall.startedAt} />
              )}
              {chartPath && onPreviewChart && (
                <button
                  type="button"
                  onClick={() => onPreviewChart(chartPath)}
                  className="tool-chart-btn flex items-center text-jupiter-muted hover:text-jupiter-orange transition-colors"
                  title={t("tool.openChart")}
                >
                  <span className="material-symbols-outlined text-[14px]">monitoring</span>
                </button>
              )}
            </span>
            {children && children.length > 0 && (
              <ul className="tool-timeline-nested">
                {children.map((c, ci) => {
                  const st = callStatus(c);
                  const label = friendlyLabel(c);
                  const { bareName } = parseToolName(c.name);
                  return (
                    <li key={c.id || ci} className="tool-timeline-subitem">
                      <span className="tool-timeline-dot" data-status={st} />
                      <span className="tool-subname">{bareName}</span>
                      {label && <span className="tool-sublabel">{label}</span>}
                    </li>
                  );
                })}
              </ul>
            )}
          </li>
        );
      })}
    </ul>
  );
}
