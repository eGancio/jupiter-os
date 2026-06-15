// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

import { useState } from "react";
import { Logo } from "../sidebar/Logo";
import { ChatList } from "../sidebar/ChatList";
import { MoonList } from "../sidebar/MoonList";
import { useAutostart } from "../../hooks/useAutostart";
import { startAll, stopAll } from "../../lib/tauri";
import { useT } from "../../i18n";
import type { ServiceInfo, ChatSessionInfo } from "../../types";

interface Props {
  services: ServiceInfo[];
  selectedMoon: string | null;
  onSelectMoon: (name: string) => void;
  onMoonInfo: (name: string) => void;
  onRefresh: () => void;
  onAddMoon: () => void;
  onRemoveMoon: (name: string) => void;
  chatSessions: ChatSessionInfo[];
  activeSessionId: string | null;
  onSelectSession: (id: string) => void;
  onNewSession: () => void;
  onDeleteSession: (id: string) => void;
  onRenameSession: (id: string, title: string) => void;
  collapsed: boolean;
  onToggleCollapse: () => void;
}

export function Sidebar({
  services,
  selectedMoon,
  onSelectMoon,
  onMoonInfo,
  onRefresh,
  onAddMoon,
  onRemoveMoon,
  chatSessions,
  activeSessionId,
  onSelectSession,
  onNewSession,
  onDeleteSession,
  onRenameSession,
  collapsed,
  onToggleCollapse,
}: Props) {
  const { enabled: autostart, toggle: toggleAutostart } = useAutostart();
  const { t, lang, setLang } = useT();
  const [showSystem, setShowSystem] = useState(false);

  const handleStartAll = async () => {
    await startAll();
    onRefresh();
  };
  const handleStopAll = async () => {
    await stopAll();
    onRefresh();
  };

  return (
    <aside
      className={`${
        collapsed ? "w-[64px]" : "w-[240px]"
      } flex-shrink-0 h-full flex flex-col bg-jupiter-surface border-r border-jupiter-border overflow-x-hidden sidebar-transition select-none`}
    >
      <Logo collapsed={collapsed} onToggleCollapse={onToggleCollapse} />

      {/* New Chat */}
      <div className={`${collapsed ? "px-2" : "px-4"} mb-6`}>
        <button
          onClick={onNewSession}
          title={t("chatlist.new")}
          className="w-full flex items-center justify-center gap-2 bg-jupiter-orange text-white font-bold text-[12px] py-3 rounded-xl hover:opacity-90 transition-all glow-orange active:scale-95 uppercase tracking-wider overflow-hidden"
        >
          <span className="material-symbols-outlined text-[18px] flex-shrink-0 fill">
            add_circle
          </span>
          {!collapsed && <span className="whitespace-nowrap">{t("chatlist.new")}</span>}
        </button>
      </div>

      {/* Nav: Chats + Moons */}
      <nav className="flex-1 overflow-y-auto overflow-x-hidden custom-scrollbar">
        {!collapsed && (
          <div className="mb-6">
            <ChatList
              sessions={chatSessions}
              activeId={activeSessionId}
              onSelect={onSelectSession}
              onNew={onNewSession}
              onDelete={onDeleteSession}
              onRename={onRenameSession}
            />
          </div>
        )}
        <MoonList
          services={services}
          selected={selectedMoon}
          onSelect={onSelectMoon}
          onInfoClick={onMoonInfo}
          onAddMoon={onAddMoon}
          onRemoveMoon={onRemoveMoon}
          collapsed={collapsed}
        />
      </nav>

      {/* Footer */}
      <div className={`${collapsed ? "p-2" : "p-4"} border-t border-jupiter-border relative`}>
        {/* System controls popover. With the sidebar collapsed (64px rail) the
            in-flow anchoring would squeeze it to ~32px and the rail's
            overflow-x-hidden would clip any overflow — so it becomes a fixed
            flyout next to the rail instead. */}
        {showSystem && (
          <div
            className={`${
              collapsed
                ? "fixed left-[72px] bottom-4 w-60"
                : "absolute left-4 right-4 bottom-[calc(100%-0.5rem)] mb-2"
            } rounded-xl bg-jupiter-elevated border border-jupiter-border card-shadow p-3 space-y-3 z-50`}
          >
            <div className="flex gap-1.5">
              <button
                onClick={handleStartAll}
                className="flex-1 flex items-center justify-center gap-1 px-2 py-1.5 text-[11px] rounded-lg bg-jupiter-surface text-jupiter-muted hover:text-white transition-colors"
              >
                <span className="material-symbols-outlined text-[14px] fill text-jupiter-green">
                  play_arrow
                </span>
                {t("sidebar.startAll")}
              </button>
              <button
                onClick={handleStopAll}
                className="flex-1 flex items-center justify-center gap-1 px-2 py-1.5 text-[11px] rounded-lg bg-jupiter-surface text-jupiter-muted hover:text-white transition-colors"
              >
                <span className="material-symbols-outlined text-[14px] fill text-jupiter-red">
                  stop
                </span>
                {t("sidebar.stopAll")}
              </button>
            </div>
            <label className="flex items-center gap-2 text-[11px] text-jupiter-dim cursor-pointer">
              <input
                type="checkbox"
                checked={autostart}
                onChange={toggleAutostart}
                className="rounded border-jupiter-border bg-jupiter-surface accent-jupiter-orange"
              />
              {t("sidebar.startWithSystem")}
            </label>
            <div className="flex items-center gap-1.5">
              <span className="text-[11px] text-jupiter-dim flex-1">{t("sidebar.language")}</span>
              <div className="flex gap-1 bg-jupiter-surface rounded p-0.5">
                {(["it", "en"] as const).map((l) => (
                  <button
                    key={l}
                    onClick={() => setLang(l)}
                    className={`px-2 py-0.5 text-[10px] rounded uppercase font-medium transition-colors ${
                      lang === l
                        ? "bg-jupiter-orange text-white"
                        : "text-jupiter-dim hover:text-white"
                    }`}
                  >
                    {l}
                  </button>
                ))}
              </div>
            </div>
          </div>
        )}

        <div className="space-y-1 mb-3">
          <button
            onClick={() => setShowSystem((v) => !v)}
            title={collapsed ? t("sidebar.settings") : undefined}
            className={`w-full flex items-center ${collapsed ? "justify-center px-0" : "gap-3 px-4"} py-2 rounded-lg transition-colors ${
              showSystem
                ? "bg-jupiter-elevated text-jupiter-primary"
                : "text-jupiter-muted hover:bg-jupiter-elevated"
            }`}
          >
            <span className="material-symbols-outlined text-[20px] flex-shrink-0">settings</span>
            {!collapsed && (
              <span className="whitespace-nowrap text-[13px]">{t("sidebar.settings")}</span>
            )}
          </button>
        </div>
      </div>
    </aside>
  );
}
