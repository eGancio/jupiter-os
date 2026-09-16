// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { ENGINE_MODELS, modelDisplayLabel } from "../../lib/models";

interface Props {
  /** Active engine ("claude", "ollama", …). */
  engine: string;
  /** Active model alias/id of the current session. */
  model: string;
  /** Full model id resolved by the SDK for the active session, if known. */
  resolvedModel?: string | null;
  /** Switch the active session's model. */
  onChange: (model: string) => void;
  /** Open the full engine settings modal (engine binds at session creation). */
  onOpenEngineSettings: () => void;
  /** Menu opening direction. "up" when placed at the bottom (input bar). */
  direction?: "up" | "down";
}

// Dropdown to pick the model for the active session, showing the real version
// number for Claude (e.g. "Opus 4.8"). The trigger matches the input-bar status
// style ("Auto mode"). The menu is rendered in a portal with fixed positioning
// so it escapes the input box's `overflow-hidden` (which would clip it). No
// native <select>: it renders white-on-white under WebKitGTK.
export function ModelDropdown({
  engine,
  model,
  resolvedModel,
  onChange,
  onOpenEngineSettings,
  direction = "down",
}: Props) {
  const [open, setOpen] = useState(false);
  const [pos, setPos] = useState<{ left: number; top?: number; bottom?: number }>({ left: 0 });
  const btnRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);

  // Position the portal menu relative to the trigger (fixed coords).
  useLayoutEffect(() => {
    if (!open || !btnRef.current) return;
    const r = btnRef.current.getBoundingClientRect();
    if (direction === "up") {
      setPos({ left: r.left, bottom: window.innerHeight - r.top + 4 });
    } else {
      setPos({ left: r.left, top: r.bottom + 4 });
    }
  }, [open, direction]);

  // Close on outside click or Escape (account for the portaled menu).
  useEffect(() => {
    if (!open) return;
    const onDoc = (e: MouseEvent) => {
      const t = e.target as Node;
      if (btnRef.current?.contains(t) || menuRef.current?.contains(t)) return;
      setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", onDoc);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDoc);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const models = ENGINE_MODELS[engine] ?? [model];
  const activeLabel = modelDisplayLabel(engine, model, resolvedModel);

  return (
    <>
      <button
        ref={btnRef}
        onClick={() => setOpen((v) => !v)}
        className="flex items-center gap-1.5 whitespace-nowrap text-white/60 hover:text-white transition-colors"
        title="Engine & model"
      >
        <span className="text-[11px]">&#9881;</span>
        <span>{engine} &middot; {activeLabel}</span>
        <span className="text-[9px] opacity-70">{direction === "up" ? "▴" : "▾"}</span>
      </button>

      {open &&
        createPortal(
          <div
            ref={menuRef}
            style={{ position: "fixed", left: pos.left, top: pos.top, bottom: pos.bottom }}
            className="z-[100] min-w-[180px] rounded-md border border-jupiter-orange/20 bg-jupiter-surface shadow-lg py-1"
          >
            {models.map((m) => {
              const sel = m === model;
              // For the active model show the resolved version; for the others
              // only the alias label is known (no resolved id available yet).
              const label = modelDisplayLabel(engine, m, sel ? resolvedModel : null);
              return (
                <button
                  key={m}
                  onClick={() => {
                    if (m !== model) onChange(m);
                    setOpen(false);
                  }}
                  className={`w-full text-left px-3 py-1.5 text-[12px] flex items-center gap-2 transition-colors ${
                    sel
                      ? "text-jupiter-orange bg-jupiter-orange/10"
                      : "text-jupiter-muted hover:bg-jupiter-elevated hover:text-white"
                  }`}
                >
                  <span className="w-3 text-center">{sel ? "✓" : ""}</span>
                  {label}
                </button>
              );
            })}

            <div className="border-t border-jupiter-orange/10 mt-1 pt-1">
              <button
                onClick={() => {
                  setOpen(false);
                  onOpenEngineSettings();
                }}
                className="w-full text-left px-3 py-1.5 text-[12px] text-jupiter-dim hover:bg-jupiter-elevated hover:text-white transition-colors flex items-center gap-2"
              >
                <span className="text-[11px]">&#9881;</span>
                Engine settings&hellip;
              </button>
            </div>
          </div>,
          document.body,
        )}
    </>
  );
}
