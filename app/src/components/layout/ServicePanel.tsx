// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

import type { ServiceInfo } from "../../types";
import { ServiceDetail } from "../services/ServiceDetail";
import { getMoonIcon, getMoonColor } from "../../lib/moonInfo";
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
      {/* Panel header — same language as the suggestions panel */}
      <div className="h-9 border-b border-jupiter-orange/25 flex items-center justify-between px-3 bg-jupiter-surface/50 flex-shrink-0">
        <span className="flex items-center gap-1.5 text-[11px] font-semibold text-white uppercase tracking-wider">
          <span
            className="material-symbols-outlined text-[15px]"
            style={{ color: getMoonColor(service.name) }}
          >
            {getMoonIcon(service.name)}
          </span>
          {t("service.detail")}
        </span>
        <button
          onClick={onClose}
          title={t("common.cancel")}
          className="flex items-center text-jupiter-dim hover:text-white transition-colors"
        >
          <span className="material-symbols-outlined text-[18px]">close</span>
        </button>
      </div>
      <ServiceDetail service={service} onRefresh={onRefresh} />
    </div>
  );
}
