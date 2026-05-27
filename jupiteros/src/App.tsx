import { useState } from "react";
import { Sidebar } from "./components/layout/Sidebar";
import { ChatArea } from "./components/layout/ChatArea";
import { ServicePanel } from "./components/layout/ServicePanel";
import { ChartPanel } from "./components/layout/ChartPanel";
import { useServices } from "./hooks/useServices";
import { useChat } from "./hooks/useChat";
import { useChartPanel } from "./hooks/useChartPanel";
import { MoonInfoModal } from "./components/services/MoonInfoModal";

function App() {
  const [selectedMoon, setSelectedMoon] = useState<string | null>(null);
  const [infoMoonName, setInfoMoonName] = useState<string | null>(null);
  const { services, refresh } = useServices();
  const chat = useChat();
  const chartPanel = useChartPanel();

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
    </div>
  );
}

export default App;
