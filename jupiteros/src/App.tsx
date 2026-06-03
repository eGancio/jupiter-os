// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

import { useState } from "react";
import { Sidebar } from "./components/layout/Sidebar";
import { ChatArea } from "./components/layout/ChatArea";
import { ServicePanel } from "./components/layout/ServicePanel";
import { ChartPanel } from "./components/layout/ChartPanel";
import { useServices } from "./hooks/useServices";
import { useChat } from "./hooks/useChat";
import { useChartPanel } from "./hooks/useChartPanel";
import { MoonInfoModal } from "./components/services/MoonInfoModal";
import { AddMoonModal } from "./components/services/AddMoonModal";
import { removeMoon } from "./lib/tauri";
import { useT } from "./i18n";

function App() {
  const { t } = useT();
  const [selectedMoon, setSelectedMoon] = useState<string | null>(null);
  const [infoMoonName, setInfoMoonName] = useState<string | null>(null);
  const [showAddMoon, setShowAddMoon] = useState(false);
  const { services, refresh } = useServices();
  const chat = useChat();
  const chartPanel = useChartPanel();

  // Suggest the next free port after the highest known Moon port
  const suggestedPort = (() => {
    const ports = services
      .map((s) => {
        const match = s.command.match(/:(\d{4,5})/);
        return match ? parseInt(match[1]) : 0;
      })
      .filter((p) => p >= 8100);
    return ports.length > 0 ? Math.max(...ports) + 100 : 8400;
  })();

  const handleRemoveMoon = async (name: string) => {
    if (!confirm(t("app.removeMoonConfirm", { name }))) return;
    await removeMoon(name);
    refresh();
    if (selectedMoon === name) setSelectedMoon(null);
  };

  const selectedService = selectedMoon
    ? services.find((s) => s.name === selectedMoon) ?? null
    : null;

  // ChartPanel has priority over ServicePanel
  const showChartPanel = chartPanel.panelOpen && chartPanel.activeChart !== null;
  const showServicePanel = !showChartPanel && selectedService !== null;

  return (
    <div className="flex h-screen bg-jupiter-bg text-jupiter-text overflow-hidden">
      <Sidebar
        services={services}
        selectedMoon={selectedMoon}
        onSelectMoon={setSelectedMoon}
        onMoonInfo={setInfoMoonName}
        onRefresh={refresh}
        onAddMoon={() => setShowAddMoon(true)}
        onRemoveMoon={handleRemoveMoon}
        chatSessions={chat.sessions}
        activeSessionId={chat.sessionId}
        onSelectSession={chat.switchSession}
        onNewSession={chat.newSession}
        onDeleteSession={chat.deleteSession}
        onRenameSession={chat.renameSession}
      />
      <ChatArea chat={chat} services={services} onPreviewChart={chartPanel.openChartByPath} />
      {showChartPanel && (
        <ChartPanel
          charts={chartPanel.charts}
          activeChart={chartPanel.activeChart}
          activeChartId={chartPanel.activeChartId}
          onSelectChart={chartPanel.selectChart}
          onRemoveChart={chartPanel.removeChart}
          onClose={chartPanel.closePanel}
        />
      )}
      {showServicePanel && (
        <ServicePanel
          service={selectedService}
          onRefresh={refresh}
          onClose={() => setSelectedMoon(null)}
        />
      )}
      {infoMoonName && (
        <MoonInfoModal
          moonName={infoMoonName}
          running={services.find((s) => s.name === infoMoonName)?.running ?? false}
          onClose={() => setInfoMoonName(null)}
        />
      )}
      {showAddMoon && (
        <AddMoonModal
          suggestedPort={suggestedPort}
          onClose={() => setShowAddMoon(false)}
          onAdded={refresh}
        />
      )}
    </div>
  );
}

export default App;
