import type { ServiceInfo } from "../../types";
import { ServiceDetail } from "../services/ServiceDetail";
import { useT } from "../../i18n";

interface Props {
  service: ServiceInfo | null;
  onRefresh: () => void;
  onClose: () => void;
}

export function ServicePanel({ service, onRefresh, onClose }: Props) {
  const { t } = useT();
  if (!service) return null;

  return (
    <div className="w-[320px] min-w-[280px] border-l border-jupiter-orange/25 bg-jupiter-surface flex flex-col min-h-0">
      {/* Panel header */}
      <div className="h-9 border-b border-jupiter-orange/25 flex items-center justify-between px-3 bg-jupiter-surface/50 flex-shrink-0">
        <span className="text-[11px] font-semibold text-white uppercase tracking-wider">
          {t("service.detail")}
        </span>
        <button
          onClick={onClose}
          className="text-jupiter-dim hover:text-white text-sm transition-colors"
        >
          &#x2715;
        </button>
      </div>
      <ServiceDetail service={service} onRefresh={onRefresh} />
    </div>
  );
}
