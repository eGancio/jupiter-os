// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

import { useEffect, useState } from "react";
import type { ServiceInfo } from "../../types";
import { getMoonLabel } from "../../lib/moonInfo";
import { startService, stopService, restartService } from "../../lib/tauri";
import { useLogs } from "../../hooks/useLogs";
import { LogViewer } from "./LogViewer";
import { StatusBadge } from "./StatusBadge";
import { CredentialsPanel } from "./CredentialsPanel";
import { EuropaCredentialsPanel } from "./EuropaCredentialsPanel";
import { EmailsPanel } from "./EmailsPanel";
import { MetisIngestPanel } from "./MetisIngestPanel";
import { BrandKitPanel } from "./BrandKitPanel";
import { useT } from "../../i18n";

interface Props {
  service: ServiceInfo | null;
  onRefresh: () => void;
}

type Tab = "details" | "emails" | "ingest";

export function ServiceDetail({ service, onRefresh }: Props) {
  const { t } = useT();
  const { lines, scrollRef, clearLogs } = useLogs(service?.name ?? null);
  const [tab, setTab] = useState<Tab>("details");

  // Reset to the details tab whenever the selected moon changes.
  useEffect(() => {
    setTab("details");
  }, [service?.name]);

  if (!service) {
    return (
      <div className="flex-1 flex items-center justify-center text-jupiter-dim text-sm">
        {t("service.selectMoon")}
      </div>
    );
  }

  const isIo = service.name === "io";
  const isMetis = service.name === "metis";
  // The optional second tab for moons that have a dedicated panel.
  const secondTab: [Tab, string] | null = isIo
    ? ["emails", "Email"]
    : isMetis
    ? ["ingest", "Ingest"]
    : null;
  const kindLabel = service.kind === "McpServer" ? t("service.mcpServer") : t("service.daemon");

  const handleStart = async () => {
    await startService(service.name);
    onRefresh();
  };
  const handleStop = async () => {
    await stopService(service.name);
    onRefresh();
  };
  const handleRestart = async () => {
    await restartService(service.name);
    onRefresh();
  };

  return (
    <div className="flex flex-col h-full p-3 min-h-0">
      {/* Header */}
      <div className="flex items-center gap-2 mb-2">
        <h2 className="text-sm font-extrabold text-jupiter-text">{getMoonLabel(service.name)}</h2>
        <span className="text-[10px] text-jupiter-dim uppercase tracking-wider px-1.5 py-0.5 rounded-md bg-jupiter-elevated">
          {kindLabel}
        </span>
      </div>

      {/* Command info — plain dim line, no box */}
      <div className="text-[11px] space-y-0.5 mb-3 text-jupiter-dim font-mono">
        <div className="truncate" title={service.command}>
          <span className="text-jupiter-dim/60">cmd</span>{" "}
          <span className="text-jupiter-muted">{service.command.split(/[/\\]/).pop()}</span>
        </div>
        {service.args.length > 0 && (
          <div className="truncate" title={service.args.join(" ")}>
            <span className="text-jupiter-dim/60">args</span>{" "}
            <span className="text-jupiter-muted">{service.args.join(" ")}</span>
          </div>
        )}
      </div>

      {/* Tabs — segmented control (Io → Email, Metis → Ingest) */}
      {secondTab && (
        <div className="flex gap-1 bg-jupiter-elevated rounded-lg p-1 text-[10px] mb-3 flex-shrink-0">
          {([["details", "Service Detail"], secondTab] as [Tab, string][]).map(
            ([id, label]) => (
              <button
                key={id}
                onClick={() => setTab(id)}
                className={`flex-1 py-1.5 rounded-md font-semibold uppercase tracking-wider transition-colors ${
                  tab === id
                    ? "bg-jupiter-orange text-white"
                    : "text-jupiter-dim hover:text-white"
                }`}
              >
                {label}
              </button>
            )
          )}
        </div>
      )}

      {isIo && tab === "emails" ? (
        <EmailsPanel />
      ) : isMetis && tab === "ingest" ? (
        <MetisIngestPanel />
      ) : (
        <>
          {/* Controls — flat, no colored boxes; the status dot carries the color */}
          <div className="flex items-center gap-1.5 mb-3 flex-wrap">
            {service.virtual ? (
              <span className="px-2.5 py-1.5 text-[11px] rounded-lg bg-jupiter-elevated text-jupiter-dim">
                {t("service.stdio")}
              </span>
            ) : !service.running ? (
              <button
                onClick={handleStart}
                className="flex items-center gap-1.5 px-3 py-1.5 text-[11px] font-bold uppercase tracking-wider rounded-lg bg-jupiter-orange text-white hover:opacity-90 glow-orange active:scale-95 transition-all"
              >
                <span className="material-symbols-outlined text-[14px] fill">play_arrow</span>
                {t("service.start")}
              </button>
            ) : (
              <>
                <button
                  onClick={handleStop}
                  className="flex items-center gap-1.5 px-3 py-1.5 text-[11px] rounded-lg bg-jupiter-elevated text-jupiter-muted hover:text-jupiter-red transition-colors"
                >
                  <span className="material-symbols-outlined text-[14px] fill text-jupiter-red">
                    stop
                  </span>
                  {t("service.stop")}
                </button>
                <button
                  onClick={handleRestart}
                  className="flex items-center gap-1.5 px-3 py-1.5 text-[11px] rounded-lg bg-jupiter-elevated text-jupiter-muted hover:text-white transition-colors"
                >
                  <span className="material-symbols-outlined text-[14px]">restart_alt</span>
                  {t("service.restart")}
                </button>
              </>
            )}
            {!service.virtual && <StatusBadge running={service.running} />}
            <div className="flex-1" />
            {!service.virtual && (
              <button
                onClick={clearLogs}
                title={t("service.clear")}
                className="flex items-center px-1.5 py-1.5 rounded-lg text-jupiter-dim hover:text-white hover:bg-jupiter-elevated transition-colors"
              >
                <span className="material-symbols-outlined text-[15px]">delete_sweep</span>
              </button>
            )}
          </div>

          {/* Credentials (only for the email server "io") */}
          <CredentialsPanel serviceName={service.name} />

          {/* Messaging channels setup (only for the messaging server "europa") */}
          <EuropaCredentialsPanel serviceName={service.name} />

          {/* Brand kit setup (only for the report renderer "thebe") */}
          <BrandKitPanel serviceName={service.name} />

          {/* Logs */}
          <div className="flex items-center gap-1.5 text-[10px] text-jupiter-dim mb-1.5 font-bold uppercase tracking-wider">
            <span className="material-symbols-outlined text-[14px]">terminal</span>
            {t("service.logs")}
          </div>
          <LogViewer lines={lines} scrollRef={scrollRef} />
        </>
      )}
    </div>
  );
}
