// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

import type { ServiceInfo } from "../../types";
import { getMoonInfo, getMoonIcon, getMoonLabel } from "../../lib/moonInfo";
import { useT } from "../../i18n";

interface Props {
  services: ServiceInfo[];
  selected: string | null;
  onSelect: (name: string) => void;
  onInfoClick: (name: string) => void;
  onAddMoon: () => void;
  onRemoveMoon: (name: string) => void;
  collapsed?: boolean;
}

function MoonRow({
  svc,
  selected,
  onSelect,
  onInfoClick,
  onRemoveMoon,
  removable,
  collapsed,
}: {
  svc: ServiceInfo;
  selected: boolean;
  onSelect: (name: string) => void;
  onInfoClick: (name: string) => void;
  onRemoveMoon: (name: string) => void;
  removable: boolean;
  collapsed: boolean;
}) {
  const { t } = useT();

  // Collapsed: an icon-only rail entry with a small status dot. The full label
  // is exposed via the native tooltip so hovering still tells you which Moon.
  if (collapsed) {
    return (
      <button
        onClick={() => onSelect(svc.name)}
        title={getMoonLabel(svc.name, t)}
        className={`group relative w-full flex items-center justify-center py-2 rounded-lg transition-colors ${
          selected
            ? "bg-jupiter-orange/10 text-jupiter-primary"
            : "text-jupiter-muted hover:bg-jupiter-elevated"
        }`}
      >
        <span className="material-symbols-outlined text-[20px]">{getMoonIcon(svc.name)}</span>
        <span
          className={`absolute top-1 right-1 w-1.5 h-1.5 rounded-full ${
            svc.running ? "bg-jupiter-green shadow-[0_0_6px_#3fb950]" : "bg-jupiter-pink"
          }`}
        />
      </button>
    );
  }

  return (
    <button
      onClick={() => onSelect(svc.name)}
      className={`group w-full text-left flex items-center gap-3 px-4 py-2 rounded-lg transition-colors ${
        selected
          ? "bg-jupiter-orange/10 text-jupiter-primary font-bold border-r-2 border-jupiter-orange"
          : "text-jupiter-muted hover:bg-jupiter-elevated"
      }`}
    >
      <span className="material-symbols-outlined text-[20px] flex-shrink-0">
        {getMoonIcon(svc.name)}
      </span>
      <span className="flex-1 truncate whitespace-nowrap text-[13px]">{getMoonLabel(svc.name, t)}</span>
      <span className="ml-auto flex items-center gap-2 flex-shrink-0">
        {removable && (
          <span
            onClick={(e) => {
              e.stopPropagation();
              onRemoveMoon(svc.name);
            }}
            className="opacity-0 group-hover:opacity-70 hover:!opacity-100 text-jupiter-red text-xs cursor-pointer transition-opacity"
            title={t("moonlist.remove")}
          >
            ×
          </span>
        )}
        <span
          onClick={(e) => {
            e.stopPropagation();
            onInfoClick(svc.name);
          }}
          className="material-symbols-outlined text-[16px] opacity-50 hover:opacity-100 cursor-pointer transition-opacity"
          title={t("moonlist.info")}
        >
          info
        </span>
        <span
          className={`w-2 h-2 rounded-full flex-shrink-0 ${
            svc.running ? "bg-jupiter-green shadow-[0_0_8px_#3fb950]" : "bg-jupiter-pink"
          }`}
        />
      </span>
    </button>
  );
}

function SectionHeader({ label, onAdd }: { label: string; onAdd?: () => void }) {
  return (
    <div className="px-4 mb-2 flex justify-between items-center">
      <span className="text-[12px] font-bold text-jupiter-muted opacity-50 uppercase tracking-widest whitespace-nowrap">
        {label}
      </span>
      {onAdd && (
        <span
          onClick={onAdd}
          className="material-symbols-outlined text-[18px] opacity-50 hover:opacity-100 cursor-pointer"
        >
          add
        </span>
      )}
    </div>
  );
}

export function MoonList({
  services,
  selected,
  onSelect,
  onInfoClick,
  onAddMoon,
  onRemoveMoon,
  collapsed = false,
}: Props) {
  const { t } = useT();
  const servers = services.filter((s) => s.kind === "McpServer");
  const daemons = services.filter((s) => s.kind === "Daemon");

  return (
    <>
      <div className="mb-6">
        {!collapsed && <SectionHeader label={t("moonlist.moons")} onAdd={onAddMoon} />}
        <div className={`${collapsed ? "px-1.5" : "px-2"} space-y-1`}>
          {servers.map((svc) => (
            <MoonRow
              key={svc.name}
              svc={svc}
              selected={selected === svc.name}
              onSelect={onSelect}
              onInfoClick={onInfoClick}
              onRemoveMoon={onRemoveMoon}
              removable={getMoonInfo(svc.name) === null}
              collapsed={collapsed}
            />
          ))}
        </div>
      </div>

      {daemons.length > 0 && (
        <div className="mb-6">
          {!collapsed && <SectionHeader label={t("moonlist.daemons")} />}
          <div className={`${collapsed ? "px-1.5" : "px-2"} space-y-1`}>
            {daemons.map((svc) => (
              <MoonRow
                key={svc.name}
                svc={svc}
                selected={selected === svc.name}
                onSelect={onSelect}
                onInfoClick={onInfoClick}
                onRemoveMoon={onRemoveMoon}
                removable={false}
                collapsed={collapsed}
              />
            ))}
          </div>
        </div>
      )}
    </>
  );
}
