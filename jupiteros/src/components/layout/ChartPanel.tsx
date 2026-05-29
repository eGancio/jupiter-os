import type { ChartEntry } from "../../types";
import { useT } from "../../i18n";

interface Props {
  charts: ChartEntry[];
  activeChart: ChartEntry | null;
  activeChartId: string | null;
  onSelectChart: (id: string) => void;
  onRemoveChart: (id: string) => void;
  onClose: () => void;
}

export function ChartPanel({
  charts,
  activeChart,
  activeChartId,
  onSelectChart,
  onRemoveChart,
  onClose,
}: Props) {
  const { t } = useT();
  if (!activeChart) return null;

  return (
    <div className="w-[550px] min-w-[400px] border-l border-jupiter-orange/25 bg-jupiter-surface flex flex-col min-h-0">
      {/* Header */}
      <div className="h-9 border-b border-jupiter-orange/25 flex items-center justify-between px-3 bg-jupiter-surface/50 flex-shrink-0">
        <div className="flex items-center gap-2">
          <span className="text-[11px] font-semibold text-white uppercase tracking-wider">
            {t("chart.preview")}
          </span>
          <span className="text-[10px] text-jupiter-dim">
            {t("chart.count", { count: charts.length })}
          </span>
        </div>
        <button
          onClick={onClose}
          className="text-jupiter-dim hover:text-white text-sm transition-colors"
        >
          &#x2715;
        </button>
      </div>

      {/* Tabs (if multiple charts) */}
      {charts.length > 1 && (
        <div className="border-b border-jupiter-orange/25 flex items-center gap-1 px-2 py-1 overflow-x-auto flex-shrink-0">
          {charts.map((chart) => (
            <div
              key={chart.id}
              className={`flex items-center gap-1 px-2 py-0.5 text-[10px] rounded cursor-pointer transition-colors group whitespace-nowrap ${
                chart.id === activeChartId
                  ? "bg-jupiter-elevated text-white"
                  : "text-jupiter-dim hover:text-jupiter-muted hover:bg-jupiter-elevated/50"
              }`}
              onClick={() => onSelectChart(chart.id)}
            >
              <span className="truncate max-w-[120px]">{chart.title}</span>
              <button
                onClick={(e) => {
                  e.stopPropagation();
                  onRemoveChart(chart.id);
                }}
                className="text-jupiter-dim hover:text-jupiter-red text-[9px] opacity-0 group-hover:opacity-100 transition-opacity ml-1"
              >
                &#x2715;
              </button>
            </div>
          ))}
        </div>
      )}

      {/* Chart iframe */}
      <div className="flex-1 min-h-0">
        <iframe
          key={activeChart.id}
          src={activeChart.blobUrl}
          className="w-full h-full border-0"
          sandbox="allow-scripts allow-same-origin"
          title={activeChart.title}
        />
      </div>

      {/* Footer */}
      <div className="h-6 border-t border-jupiter-orange/25 flex items-center px-3 bg-jupiter-surface/50 flex-shrink-0">
        <span className="text-[9px] text-jupiter-dim truncate font-mono">
          {activeChart.filePath}
        </span>
      </div>
    </div>
  );
}
