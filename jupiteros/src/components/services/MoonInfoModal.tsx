// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

import { useEffect, useState } from "react";
import { getMoonInfo, getMoonColor, getMoonIcon } from "../../lib/moonInfo";
import { useT } from "../../i18n";

interface Props {
  moonName: string;
  running: boolean;
  onClose: () => void;
}

const SIZES = [
  { w: 400, h: 420 },
  { w: 520, h: 560 },
  { w: 700, h: 700 },
];

export function MoonInfoModal({ moonName, running, onClose }: Props) {
  const { t } = useT();
  const info = getMoonInfo(moonName);
  const [sizeIdx, setSizeIdx] = useState(1);
  const size = SIZES[sizeIdx];

  useEffect(() => {
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", handleKey);
    return () => window.removeEventListener("keydown", handleKey);
  }, [onClose]);

  return (
    <div
      className="fixed inset-0 z-50 bg-black/70 flex items-center justify-center"
      onClick={onClose}
    >
      <div
        className="bg-jupiter-bg border border-jupiter-border rounded-xl overflow-hidden flex flex-col card-shadow"
        style={{ width: size.w, height: size.h }}
        onClick={(e) => e.stopPropagation()}
      >
        {/* Header */}
        <div className="px-4 py-3 border-b border-jupiter-border flex items-center justify-between flex-shrink-0">
          <div>
            <div className="flex items-center gap-2">
              <span
                className="material-symbols-outlined text-[18px]"
                style={{ color: getMoonColor(moonName) }}
              >
                {getMoonIcon(moonName)}
              </span>
              <h2 className="text-sm font-bold text-jupiter-text font-display">
                {info?.displayName ?? moonName}
              </h2>
              <span
                className={`w-2 h-2 rounded-full flex-shrink-0 ${
                  running ? "bg-jupiter-green" : "bg-jupiter-red"
                }`}
              />
              <span className="text-[10px] text-jupiter-dim">
                {running ? t("status.running") : t("status.stopped")}
              </span>
            </div>
            {info && (
              <p className="text-[11px] text-jupiter-dim mt-0.5">
                {t(info.categoryKey)}
              </p>
            )}
          </div>
          <div className="flex items-center gap-1 ml-4">
            <button
              onClick={() => setSizeIdx((i) => Math.max(0, i - 1))}
              disabled={sizeIdx === 0}
              className="w-6 h-6 flex items-center justify-center rounded-lg text-jupiter-orange hover:bg-jupiter-orange/15 disabled:opacity-30 disabled:cursor-default transition-colors text-sm font-bold"
            >
              &#x2212;
            </button>
            <button
              onClick={() => setSizeIdx((i) => Math.min(SIZES.length - 1, i + 1))}
              disabled={sizeIdx === SIZES.length - 1}
              className="w-6 h-6 flex items-center justify-center rounded-lg text-jupiter-orange hover:bg-jupiter-orange/15 disabled:opacity-30 disabled:cursor-default transition-colors text-sm font-bold"
            >
              +
            </button>
            <button
              onClick={onClose}
              className="w-6 h-6 flex items-center justify-center rounded-lg text-jupiter-dim hover:text-white hover:bg-jupiter-elevated transition-colors ml-1"
            >
              <span className="material-symbols-outlined text-[16px]">close</span>
            </button>
          </div>
        </div>

        {/* Body */}
        <div className="px-4 py-3 overflow-y-auto flex-1 min-h-0">
          {info ? (
            <>
              <p className="text-[12px] text-jupiter-muted leading-relaxed mb-3">
                {t(info.descKey)}
              </p>

              <div className="flex items-center gap-2 text-[11px] text-jupiter-dim mb-4">
                <span>
                  {t("moonInfo.port")}:{" "}
                  <span className="text-white font-medium">{info.port}</span>
                </span>
                <span className="text-jupiter-dim/30">·</span>
                <span className="text-white">{info.location}</span>
              </div>

              <div className="border-t border-jupiter-border pt-3">
                <h3 className="text-[10px] font-semibold uppercase tracking-wider text-jupiter-dim mb-2">
                  {t("moonInfo.tools", { count: info.tools.length })}
                </h3>
                <div className="space-y-1 pr-1">
                  {info.tools.map((tool) => (
                    <div
                      key={tool.name}
                      className="flex items-baseline gap-2 py-1 px-2 rounded-lg bg-jupiter-elevated/60"
                    >
                      <code className="text-[11px] font-mono whitespace-nowrap flex-shrink-0" style={{ color: getMoonColor(moonName) + "cc" }}>
                        {tool.name}
                      </code>
                      <span className="text-[10px] text-jupiter-muted leading-tight">
                        {t(tool.descKey)}
                      </span>
                    </div>
                  ))}
                </div>
              </div>
            </>
          ) : (
            <p className="text-[12px] text-jupiter-dim">
              {t("moonInfo.none")}
            </p>
          )}
        </div>
      </div>
    </div>
  );
}
