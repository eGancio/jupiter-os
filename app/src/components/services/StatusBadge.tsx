// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

import { useT } from "../../i18n";

export function StatusBadge({ running }: { running: boolean }) {
  const { t } = useT();
  // Minimal: just a glowing dot + label, no box.
  return (
    <span
      className={`inline-flex items-center gap-1.5 text-[10px] font-bold uppercase tracking-wide ${
        running ? "text-jupiter-green" : "text-jupiter-red"
      }`}
    >
      <span
        className={`w-1.5 h-1.5 rounded-full ${
          running ? "bg-jupiter-green shadow-[0_0_6px_#3fb950]" : "bg-jupiter-red"
        }`}
      />
      {running ? t("status.running") : t("status.stopped")}
    </span>
  );
}
