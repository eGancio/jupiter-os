// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

import { useEffect, useState } from "react";
import type { useChat } from "../../hooks/useChat";
import { getChatEngine } from "../../lib/tauri";

interface Props {
  chat: ReturnType<typeof useChat>;
  onClose: () => void;
}

interface EngineDef {
  id: string;
  name: string;
  models: string[];
  available: boolean;
  note?: string;
}

// Phase 0.2: only Claude is wired. Ollama / Gemini are shown disabled so the
// structure is visible — Phase 0.3/0.5 flip `available` and add a backend list.
const ENGINES: EngineDef[] = [
  { id: "claude", name: "Claude (Anthropic)", models: ["haiku", "sonnet", "opus"], available: true },
  { id: "ollama", name: "Ollama (local)", models: [], available: false, note: "coming soon" },
  { id: "gemini", name: "Gemini (Google)", models: [], available: false, note: "coming soon" },
];

export function EngineSettings({ chat, onClose }: Props) {
  const [engine, setEngine] = useState("claude");
  const model = chat.model;

  // Read the active engine from the backend (single source of truth).
  useEffect(() => {
    getChatEngine().then(setEngine).catch(() => { /* default claude */ });
  }, []);

  // Escape closes the modal (same as the other modals).
  useEffect(() => {
    const handleKey = (e: KeyboardEvent) => { if (e.key === "Escape") onClose(); };
    window.addEventListener("keydown", handleKey);
    return () => window.removeEventListener("keydown", handleKey);
  }, [onClose]);

  const active = ENGINES.find((e) => e.id === engine) ?? ENGINES[0];

  return (
    <div
      className="fixed inset-0 z-50 bg-black/70 flex items-center justify-center"
      onClick={onClose}
    >
      <div
        className="bg-jupiter-bg border border-jupiter-orange/20 rounded-lg overflow-hidden flex flex-col shadow-2xl"
        style={{ width: 460 }}
        onClick={(e) => e.stopPropagation()}
      >
        {/* Header */}
        <div className="px-4 py-3 border-b border-jupiter-orange/15 flex items-center justify-between flex-shrink-0">
          <div className="flex items-center gap-2">
            <h2 className="text-sm font-bold text-jupiter-text font-display">Engine &amp; Model</h2>
            <span className="px-2 py-0.5 text-[10px] rounded bg-jupiter-blue/15 text-jupiter-blue border border-jupiter-blue/30 font-mono">
              {engine} · {model}
            </span>
          </div>
          <button
            onClick={onClose}
            className="w-6 h-6 flex items-center justify-center rounded text-jupiter-dim hover:text-jupiter-muted hover:bg-jupiter-orange/10 transition-colors text-sm"
          >
            &#x2715;
          </button>
        </div>

        {/* Body */}
        <div className="px-4 py-4 space-y-4">
          {/* Engine selector */}
          <div>
            <h3 className="text-[10px] font-semibold uppercase tracking-wider text-jupiter-dim mb-2">
              Engine
            </h3>
            <div className="space-y-1.5">
              {ENGINES.map((e) => {
                const selected = e.id === engine;
                return (
                  <button
                    key={e.id}
                    disabled={!e.available}
                    onClick={() => e.available && setEngine(e.id)}
                    className={`w-full text-left px-3 py-2 rounded border text-[12px] flex items-center justify-between transition-colors ${
                      selected
                        ? "border-jupiter-orange/50 bg-jupiter-orange/10 text-white"
                        : "border-jupiter-orange/15 bg-jupiter-surface/40 text-jupiter-muted hover:bg-jupiter-elevated"
                    } ${!e.available ? "opacity-40 cursor-default hover:bg-jupiter-surface/40" : ""}`}
                  >
                    <span className="flex items-center gap-2">
                      <span className={`w-2 h-2 rounded-full ${selected ? "bg-jupiter-green" : "bg-jupiter-dim/50"}`} />
                      {e.name}
                    </span>
                    {!e.available && <span className="text-[10px] text-jupiter-dim italic">{e.note}</span>}
                  </button>
                );
              })}
            </div>
          </div>

          {/* Model selector (button row — native <select> renders white-on-white
              under WebKitGTK, so we use the same button style as the engine list) */}
          <div>
            <h3 className="text-[10px] font-semibold uppercase tracking-wider text-jupiter-dim mb-2">
              Model
            </h3>
            {active.models.length === 0 ? (
              <span className="text-[11px] text-jupiter-dim italic px-1">—</span>
            ) : (
              <div className="flex gap-1.5">
                {active.models.map((m) => {
                  const sel = m === model;
                  return (
                    <button
                      key={m}
                      onClick={() => { void chat.changeModel(m); }}
                      className={`flex-1 px-3 py-2 rounded border text-[12px] font-mono transition-colors ${
                        sel
                          ? "border-jupiter-orange/50 bg-jupiter-orange/10 text-white"
                          : "border-jupiter-orange/15 bg-jupiter-surface/40 text-jupiter-muted hover:bg-jupiter-elevated"
                      }`}
                    >
                      {m}
                    </button>
                  );
                })}
              </div>
            )}
          </div>

          {/* Footer note */}
          <p className="text-[10px] text-jupiter-dim leading-relaxed border-t border-jupiter-orange/10 pt-3">
            More engines (local Ollama, Gemini) are coming. Your Anthropic API key lives in{" "}
            <code className="text-jupiter-muted">credentials.env</code>.
          </p>
        </div>
      </div>
    </div>
  );
}
