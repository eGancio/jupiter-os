// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

export function Logo() {
  return (
    <div className="flex items-center justify-center px-4 py-3 gap-2.5">
      <img src="/logo.png" alt="JupiterOS" className="w-7 h-7" />
      <div className="flex flex-col">
        <span className="text-sm font-bold tracking-wide text-jupiter-text leading-tight font-display">JupiterOS</span>
        <span className="text-[9px] text-jupiter-dim tracking-wider leading-tight font-display">jupiteros.ai</span>
      </div>
    </div>
  );
}
