// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

import type { RefObject } from "react";
import { useT } from "../../i18n";

interface Props {
  lines: string[];
  scrollRef: RefObject<HTMLDivElement | null>;
}

export function LogViewer({ lines, scrollRef }: Props) {
  const { t } = useT();
  return (
    <div
      ref={scrollRef}
      className="flex-1 overflow-y-auto bg-jupiter-bg rounded border border-jupiter-orange/25 p-2 font-mono text-[11px] leading-5 min-h-0"
    >
      {lines.length === 0 ? (
        <p className="text-jupiter-dim italic">{t("log.empty")}</p>
      ) : (
        lines.map((line, i) => (
          <div key={i} className="whitespace-pre-wrap break-all text-white">
            {line}
          </div>
        ))
      )}
    </div>
  );
}
