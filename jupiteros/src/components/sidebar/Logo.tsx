// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

interface Props {
  collapsed: boolean;
  onToggleCollapse: () => void;
}

export function Logo({ collapsed, onToggleCollapse }: Props) {
  if (collapsed) {
    return (
      <div className="px-2 py-5 flex items-center justify-center">
        <button
          onClick={onToggleCollapse}
          className="p-1 hover:bg-jupiter-elevated rounded-lg text-jupiter-muted transition-colors"
          title="Expand sidebar"
        >
          <span className="material-symbols-outlined text-[20px]">chevron_right</span>
        </button>
      </div>
    );
  }
  return (
    <div className="px-4 py-5 flex items-center justify-between gap-3">
      <div className="flex items-center gap-3 overflow-hidden">
        <img
          src="/logo.png"
          alt="JupiterOS"
          className="w-8 h-8 object-contain flex-shrink-0 scale-[1.3]"
        />
        <span className="text-[20px] font-extrabold whitespace-nowrap bg-gradient-to-r from-jupiter-orange via-jupiter-pink to-jupiter-violet bg-clip-text text-transparent">
          JupiterOS
        </span>
      </div>
      <button
        onClick={onToggleCollapse}
        className="p-1 hover:bg-jupiter-elevated rounded-lg text-jupiter-muted transition-colors flex-shrink-0"
        title="Collapse sidebar"
      >
        <span className="material-symbols-outlined text-[20px]">
          {collapsed ? "chevron_right" : "chevron_left"}
        </span>
      </button>
    </div>
  );
}
