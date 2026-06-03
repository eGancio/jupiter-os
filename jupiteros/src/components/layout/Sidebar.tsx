// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

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
}: Props) {
  const { enabled: autostart, toggle: toggleAutostart } = useAutostart();
  const { t, lang, setLang } = useT();

  const handleStartAll = async () => {
    await startAll();
    onRefresh();
  };
  const handleStopAll = async () => {
    await stopAll();
    onRefresh();
  };

  return (
    <aside className="w-[220px] min-w-[220px] border-r border-jupiter-orange/25 flex flex-col bg-jupiter-surface select-none">
      <Logo />

      <hr className="border-jupiter-orange/25" />

      {/* Chat sessions */}
      <div className="py-2">
        <ChatList
          sessions={chatSessions}
          activeId={activeSessionId}
          onSelect={onSelectSession}
          onNew={onNewSession}
          onDelete={onDeleteSession}
          onRename={onRenameSession}
        />
      </div>

      <hr className="border-jupiter-orange/25" />

      {/* Moons + Daemons */}
      <div className="flex-1 overflow-y-auto py-2">
        <MoonList
          services={services}
          selected={selectedMoon}
          onSelect={onSelectMoon}
          onInfoClick={onMoonInfo}
          onAddMoon={onAddMoon}
          onRemoveMoon={onRemoveMoon}
        />
      </div>

      <hr className="border-jupiter-orange/25" />

      {/* Bottom controls */}
      <div className="p-3 space-y-2">
        <div className="flex gap-1.5">
          <button
            onClick={handleStartAll}
            className="flex-1 px-2 py-1 text-[11px] rounded bg-jupiter-orange/15 text-jupiter-orange border border-jupiter-orange/30 hover:bg-jupiter-orange/25 hover:text-jupiter-orange-light transition-colors"
          >
            {t("sidebar.startAll")}
          </button>
          <button
            onClick={handleStopAll}
            className="flex-1 px-2 py-1 text-[11px] rounded bg-jupiter-orange/15 text-jupiter-orange border border-jupiter-orange/30 hover:bg-jupiter-orange/25 hover:text-jupiter-orange-light transition-colors"
          >
            {t("sidebar.stopAll")}
          </button>
        </div>
        <label className="flex items-center gap-2 text-[11px] text-jupiter-dim cursor-pointer">
          <input
            type="checkbox"
            checked={autostart}
            onChange={toggleAutostart}
            className="rounded border-jupiter-orange/25 bg-jupiter-elevated accent-jupiter-blue"
          />
          {t("sidebar.startWithSystem")}
        </label>

        {/* Language selector */}
        <div className="flex items-center gap-1.5 pt-0.5">
          <span className="text-[11px] text-jupiter-dim flex-1">{t("sidebar.language")}</span>
          <div className="flex gap-1 bg-jupiter-elevated rounded p-0.5">
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
    </aside>
  );
}
