// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

import { useEffect, useState } from "react";
import type { ServiceInfo } from "../../types";
import { startService, stopService, restartService } from "../../lib/tauri";
import { useLogs } from "../../hooks/useLogs";
import { LogViewer } from "./LogViewer";
import { StatusBadge } from "./StatusBadge";
import { CredentialsPanel } from "./CredentialsPanel";
import { EuropaCredentialsPanel } from "./EuropaCredentialsPanel";
import { EmailsPanel } from "./EmailsPanel";
import { MetisIngestPanel } from "./MetisIngestPanel";
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
        <h2 className="text-sm font-bold text-white">{service.name}</h2>
        <span className="text-[10px] text-jupiter-dim">({kindLabel})</span>
      </div>

      {/* Command info */}
      <div className="text-[11px] space-y-0.5 mb-2 text-jupiter-dim">
        <div>
          <span className="text-jupiter-dim">cmd:</span>{" "}
          <code className="text-white">{service.command.split(/[/\\]/).pop()}</code>
        </div>
        {service.args.length > 0 && (
          <div>
            <span className="text-jupiter-dim">args:</span>{" "}
            <code className="text-white">{service.args.join(" ")}</code>
          </div>
        )}
      </div>

      <hr className="border-jupiter-orange/25 mb-2" />

      {/* Tabs — moons with a dedicated panel (Io → Email, Metis → Ingest) */}
      {secondTab && (
        <div className="grid grid-cols-2 gap-1 text-[10px] mb-2 flex-shrink-0">
          {([["details", "Service Detail"], secondTab] as [Tab, string][]).map(
            ([id, label]) => (
              <button
                key={id}
                onClick={() => setTab(id)}
                className={`px-2 py-1 rounded border transition-colors ${
                  tab === id
                    ? "bg-jupiter-orange/20 text-jupiter-orange border-jupiter-orange/40"
                    : "text-jupiter-dim border-jupiter-orange/15 hover:border-jupiter-orange/30"
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
          {/* Controls */}
          <div className="flex items-center gap-1.5 mb-2 flex-wrap">
            {service.virtual ? (
              <span className="px-2.5 py-1 text-[11px] rounded bg-jupiter-elevated text-jupiter-dim border border-jupiter-orange/25">
                {t("service.stdio")}
              </span>
            ) : !service.running ? (
              <button
                onClick={handleStart}
                className="px-2.5 py-1 text-[11px] rounded bg-jupiter-green/15 text-jupiter-green border border-jupiter-green/30 hover:bg-jupiter-green/25 transition-colors"
              >
                {t("service.start")}
              </button>
            ) : (
              <>
                <button
                  onClick={handleStop}
                  className="px-2.5 py-1 text-[11px] rounded bg-jupiter-red/15 text-jupiter-red border border-jupiter-red/30 hover:bg-jupiter-red/25 transition-colors"
                >
                  {t("service.stop")}
                </button>
                <button
                  onClick={handleRestart}
                  className="px-2.5 py-1 text-[11px] rounded bg-jupiter-amber/15 text-jupiter-amber border border-jupiter-amber/30 hover:bg-jupiter-amber/25 transition-colors"
                >
                  {t("service.restart")}
                </button>
              </>
            )}
            {!service.virtual && <StatusBadge running={service.running} />}
            <div className="flex-1" />
            {!service.virtual && (
              <button
                onClick={clearLogs}
                className="px-2 py-1 text-[10px] rounded text-jupiter-dim hover:text-white hover:bg-jupiter-elevated transition-colors"
              >
                {t("service.clear")}
              </button>
            )}
          </div>

          {/* Credentials (only for the email server "io") */}
          <CredentialsPanel serviceName={service.name} />

          {/* Messaging channels setup (only for the messaging server "europa") */}
          <EuropaCredentialsPanel serviceName={service.name} />

          {/* Logs */}
          <div className="text-[10px] text-jupiter-dim mb-1 font-semibold uppercase tracking-wider">
            {t("service.logs")}
          </div>
          <LogViewer lines={lines} scrollRef={scrollRef} />
        </>
      )}
    </div>
  );
}
