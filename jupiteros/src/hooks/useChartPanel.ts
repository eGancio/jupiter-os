import { useState, useEffect, useCallback } from "react";
import { listen } from "@tauri-apps/api/event";
import { readChartFile } from "../lib/tauri";
import type { ToolResultEvent, ChartEntry } from "../types";

/** Extract an HTML file path from a tool result string */
function extractHtmlPath(result: string): string | null {
  // Windows absolute path
  const winMatch = result.match(/([A-Za-z]:\\[^\s"'`\n]+\.html)/);
  if (winMatch) return winMatch[1];
  // Unix absolute path
  const unixMatch = result.match(/(\/[^\s"'`\n]+\.html)/);
  if (unixMatch) return unixMatch[1];
  return null;
}

let chartCounter = 0;

export function useChartPanel() {
  const [charts, setCharts] = useState<ChartEntry[]>([]);
  const [activeChartId, setActiveChartId] = useState<string | null>(null);
  const [panelOpen, setPanelOpen] = useState(false);

  useEffect(() => {
    // Listen to ALL tool results — check if result contains an amalthea HTML path
    const unlisten = listen<ToolResultEvent>("claude-tool-result", (event) => {
      const { result, session_id } = event.payload;

      const filePath = extractHtmlPath(result);
      if (!filePath) return;

      // Only auto-open for amalthea output files
      if (!filePath.toLowerCase().includes("amalthea")) return;

      // Read HTML from disk via Rust, then create blob URL
      readChartFile(filePath)
        .then((html) => {
          const blob = new Blob([html], { type: "text/html" });
          const blobUrl = URL.createObjectURL(blob);
          const id = `chart-${Date.now()}-${++chartCounter}`;
          const title = filePath.split(/[/\\]/).pop() || "chart.html";

          const entry: ChartEntry = {
            id,
            filePath,
            blobUrl,
            title,
            timestamp: Date.now(),
            sessionId: session_id,
          };

          setCharts((prev) => [...prev, entry]);
          setActiveChartId(id);
          setPanelOpen(true);
        })
        .catch(() => {
          // File read failed — ignore silently
        });
    });

    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  const selectChart = useCallback((id: string) => {
    setActiveChartId(id);
    setPanelOpen(true);
  }, []);

  const closePanel = useCallback(() => {
    setPanelOpen(false);
  }, []);

  const removeChart = useCallback(
    (id: string) => {
      setCharts((prev) => {
        const entry = prev.find((c) => c.id === id);
        if (entry) URL.revokeObjectURL(entry.blobUrl);
        const filtered = prev.filter((c) => c.id !== id);
        if (id === activeChartId) {
          const last = filtered[filtered.length - 1];
          setActiveChartId(last?.id ?? null);
          if (!last) setPanelOpen(false);
        }
        return filtered;
      });
    },
    [activeChartId]
  );

  /** Open panel for a chart matching a file path (used by ToolCallCard preview button) */
  const openChartByPath = useCallback(
    (filePath: string) => {
      const existing = charts.find((c) => c.filePath === filePath);
      if (existing) {
        setActiveChartId(existing.id);
        setPanelOpen(true);
      } else {
        // Chart not in list yet — read and add it
        readChartFile(filePath)
          .then((html) => {
            const blob = new Blob([html], { type: "text/html" });
            const blobUrl = URL.createObjectURL(blob);
            const id = `chart-${Date.now()}-${++chartCounter}`;
            const title = filePath.split(/[/\\]/).pop() || "chart.html";

            const entry: ChartEntry = {
              id,
              filePath,
              blobUrl,
              title,
              timestamp: Date.now(),
              sessionId: "",
            };

            setCharts((prev) => [...prev, entry]);
            setActiveChartId(id);
            setPanelOpen(true);
          })
          .catch(() => {});
      }
    },
    [charts]
  );

  const activeChart = charts.find((c) => c.id === activeChartId) ?? null;

  return {
    charts,
    activeChart,
    activeChartId,
    panelOpen,
    selectChart,
    closePanel,
    removeChart,
    openChartByPath,
  };
}
