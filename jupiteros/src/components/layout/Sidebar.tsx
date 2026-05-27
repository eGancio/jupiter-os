import { Logo } from "../sidebar/Logo";
import { ChatList } from "../sidebar/ChatList";
import { MoonList } from "../sidebar/MoonList";
import { useAutostart } from "../../hooks/useAutostart";
import { startAll, stopAll } from "../../lib/tauri";
import type { ServiceInfo, ChatSessionInfo } from "../../types";

interface Props {
  services: ServiceInfo[];
  selectedMoon: string | null;
  onSelectMoon: (name: string) => void;
  onMoonInfo: (name: string) => void;
  onRefresh: () => void;
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
  chatSessions,
  activeSessionId,
  onSelectSession,
  onNewSession,
  onDeleteSession,
  onRenameSession,
}: Props) {
  const { enabled: autostart, toggle: toggleAutostart } = useAutostart();

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
            Start All
          </button>
          <button
            onClick={handleStopAll}
            className="flex-1 px-2 py-1 text-[11px] rounded bg-jupiter-orange/15 text-jupiter-orange border border-jupiter-orange/30 hover:bg-jupiter-orange/25 hover:text-jupiter-orange-light transition-colors"
          >
            Stop All
          </button>
        </div>
        <label className="flex items-center gap-2 text-[11px] text-jupiter-dim cursor-pointer">
          <input
            type="checkbox"
            checked={autostart}
            onChange={toggleAutostart}
            className="rounded border-jupiter-orange/25 bg-jupiter-elevated accent-jupiter-blue"
          />
          Start with Windows
        </label>
      </div>
    </aside>
  );
}
