// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

import type { ChatSessionInfo, TabStatus } from "../../types";
import { useT } from "../../i18n";

interface Props {
  tabs: string[];
  activeId: string | null;
  sessions: ChatSessionInfo[];
  tabStatus: Record<string, TabStatus>;
  onSelect: (id: string) => void;
  onClose: (id: string) => void;
  onNew: () => void;
  maxTabs: number;
}

/** Per-tab status dot: streaming (green pulse) > error (red) > unread (amber) > idle. */
function statusDotClass(st: TabStatus | undefined, isActive: boolean): string {
  if (st?.streaming) return "bg-jupiter-green animate-pulse";
  if (st?.error) return "bg-red-500";
  if (st?.unread && !isActive) return "bg-amber-400";
  return "bg-jupiter-dim";
}

/** Browser-style tab strip for concurrent chats. */
export function ChatTabs({ tabs, activeId, sessions, tabStatus, onSelect, onClose, onNew, maxTabs }: Props) {
  const { t } = useT();
  const byId = new Map(sessions.map((s) => [s.id, s]));
  const atCap = tabs.length >= maxTabs;

  return (
    <div className="flex items-center gap-1 flex-1 min-w-0 overflow-x-auto scrollbar-none">
      {tabs.map((id) => {
        const isActive = id === activeId;
        const info = byId.get(id);
        const title = info?.title?.trim() || t("chatlist.untitled");
        return (
          <div
            key={id}
            role="tab"
            aria-selected={isActive}
            title={title}
            onClick={() => !isActive && onSelect(id)}
            onAuxClick={(e) => {
              // Middle-click closes, like a browser
              if (e.button === 1) {
                e.preventDefault();
                onClose(id);
              }
            }}
            className={`group px-3 py-1.5 text-[12.5px] rounded-md border flex items-center gap-2 cursor-pointer select-none flex-shrink min-w-0 max-w-[200px] transition-colors ${
              isActive
                ? "bg-jupiter-elevated text-white border-jupiter-orange/40"
                : "bg-jupiter-elevated/30 text-white/65 border-transparent hover:text-white hover:bg-jupiter-elevated/60"
            }`}
          >
            <span
              className={`w-2 h-2 rounded-full flex-shrink-0 ${statusDotClass(tabStatus[id], isActive)}`}
            />
            <span className="truncate font-medium">{title}</span>
            <button
              onClick={(e) => {
                e.stopPropagation();
                onClose(id);
              }}
              title={t("tabs.close")}
              className={`flex-shrink-0 rounded px-1 text-[14px] leading-none text-jupiter-dim hover:text-white ${
                isActive ? "" : "opacity-0 group-hover:opacity-100"
              }`}
            >
              ×
            </button>
          </div>
        );
      })}
      <button
        onClick={onNew}
        title={atCap ? t("tabs.limit", { max: String(maxTabs) }) : t("tabs.new")}
        className={`px-2.5 py-1 text-[15px] leading-none rounded-md flex-shrink-0 ${
          atCap
            ? "text-jupiter-dim/40 cursor-default"
            : "text-jupiter-dim hover:text-white hover:bg-jupiter-elevated/50"
        }`}
      >
        +
      </button>
    </div>
  );
}
