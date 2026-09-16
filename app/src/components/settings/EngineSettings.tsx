// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

import { useEffect, useState } from "react";
import type { useChat } from "../../hooks/useChat";
import { getChatEngine, setChatEngine } from "../../lib/tauri";
import { ENGINE_MODELS, modelDisplayLabel } from "../../lib/models";

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

// Claude + local Ollama are wired. Gemini is shown disabled so the structure
// is visible. The engine binds at session creation, so switching engine starts
// a new chat (see handleSelectEngine). Ollama Phase A is chat-only (no Moons).
// Model lists live in src/lib/models.ts (single source, shared with the in-chat
// ModelDropdown). qwen3 30B-A3B instruct (MoE, 3B attivi) è il miglior locale su
// CPU; la variante IBRIDA (qwen3:30b) è esclusa di proposito (thinking inusabile
// su CPU).
const ENGINES: EngineDef[] = [
  { id: "claude", name: "Claude (Anthropic)", models: ENGINE_MODELS.claude, available: true },
  { id: "ollama", name: "Ollama (local)", models: ENGINE_MODELS.ollama, available: true },
  { id: "gemini", name: "Gemini (Google)", models: ENGINE_MODELS.gemini, available: true },
  { id: "groq", name: "Groq (free, fast)", models: ENGINE_MODELS.groq, available: true },
  { id: "dwarfstar", name: "DwarfStar (local DeepSeek V4)", models: ENGINE_MODELS.dwarfstar, available: true },
  { id: "localops", name: "LocalOps (llama.cpp, local)", models: ENGINE_MODELS.localops, available: true },
  { id: "openrouter", name: "OpenRouter (DeepSeek V4 API)", models: ENGINE_MODELS.openrouter, available: true },
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

  // The engine is fixed at session creation, so switching it starts a NEW chat.
  // The model is fixed AFTER newSession() because changeModel is per-session:
  // it must target the freshly created session, not the previous tab's one.
  const handleSelectEngine = async (id: string) => {
    if (id === engine) return;
    const target = ENGINES.find((x) => x.id === id);
    if (!target || !target.available) return;
    const nextModel = target.models.includes(model) ? model : target.models[0];
    try {
      await setChatEngine(id);
      const newId = await chat.newSession();
      if (!newId) {
        // Tab cap: nessuna sessione creata — rollback dell'engine globale,
        // altrimenti resta puntato al nuovo engine mentre la sessione attiva
        // (e le prossime "NEW CHAT") sono ancora su quello vecchio.
        await setChatEngine(engine);
        return;
      }
      if (nextModel && nextModel !== model) await chat.changeModel(nextModel);
      setEngine(id);
    } catch {
      /* ignore */
    }
  };

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
                    onClick={() => void handleSelectEngine(e.id)}
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
                      {modelDisplayLabel(active.id, m, sel ? chat.resolvedModel : null)}
                    </button>
                  );
                })}
              </div>
            )}
          </div>

          {/* Footer note */}
          <p className="text-[10px] text-jupiter-dim leading-relaxed border-t border-jupiter-orange/10 pt-3">
            Ollama runs a local model on your machine (offline; chat only for now — Moon
            tools coming). Gemini is coming. Your Anthropic API key lives in{" "}
            <code className="text-jupiter-muted">credentials.env</code>.
          </p>
        </div>
      </div>
    </div>
  );
}
