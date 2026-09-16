// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

import { getMoonLabel, getMoonIcon, getMoonColor } from "../../lib/moonInfo";
import { useT } from "../../i18n";

interface Props {
  /** Close the panel — the right bar then stays blank until a Moon is selected. */
  onClose: () => void;
  /** Send an example prompt straight to the chat. */
  onUsePrompt: (text: string) => void;
}

// Example prompts grouped by Moon (codename). Title/icon/color come from the
// single moonInfo display layer — only the prompts live here (content).
const SECTIONS: Array<{ moon: string; prompts: string[] }> = [
  {
    moon: "io",
    prompts: [
      "Leggi le ultime email ricevute oggi",
      "Cerca le email che parlano di fatture",
      "Riassumi le email più recenti",
    ],
  },
  {
    moon: "europa",
    prompts: [
      "Mostrami gli ultimi messaggi Telegram",
      "Cerca i messaggi su un argomento",
    ],
  },
  {
    moon: "amalthea",
    prompts: [
      "Crea un grafico a barre con dei dati di esempio",
      "Genera un diagramma di flusso di un processo",
    ],
  },
  {
    moon: "himalia",
    prompts: ["Cerca sul web le ultime novità sull'AI"],
  },
  {
    moon: "elara",
    prompts: ["Trova notizie recenti su un tema che mi interessa"],
  },
  {
    moon: "metis",
    prompts: ["Cosa dice un documento indicizzato su un certo tema?"],
  },
];

export function SuggestionsPanel({ onClose, onUsePrompt }: Props) {
  const { t } = useT();
  return (
    <div className="w-[320px] min-w-[280px] border-l border-jupiter-orange/25 bg-jupiter-surface flex flex-col min-h-0">
      {/* Header with close (X) */}
      <div className="h-9 border-b border-jupiter-orange/25 flex items-center justify-between px-3 bg-jupiter-surface/50 flex-shrink-0">
        <span className="flex items-center gap-1.5 text-[11px] font-semibold text-white uppercase tracking-wider">
          <span className="material-symbols-outlined text-jupiter-orange text-[15px] fill">
            auto_awesome
          </span>
          Cosa posso fare
        </span>
        <button
          onClick={onClose}
          title="Chiudi"
          className="flex items-center text-jupiter-dim hover:text-white transition-colors"
        >
          <span className="material-symbols-outlined text-[18px]">close</span>
        </button>
      </div>

      {/* Body */}
      <div className="flex-1 overflow-y-auto custom-scrollbar p-4 space-y-5">
        <p className="text-[12px] text-jupiter-muted leading-relaxed">
          JupiterOS collega l'AI alle tue{" "}
          <span className="text-jupiter-text font-medium">Moon</span>: email, messaggi, grafici,
          ricerca web, documenti e altro. Scrivi in chat in linguaggio naturale, oppure prova uno
          degli esempi qui sotto.
        </p>

        {SECTIONS.map((s) => (
          <div key={s.moon} className="space-y-1.5">
            <div className="flex items-center gap-2">
              <span className="material-symbols-outlined text-[16px]" style={{ color: getMoonColor(s.moon) }}>
                {getMoonIcon(s.moon)}
              </span>
              <h4 className="text-[11px] font-bold text-jupiter-text uppercase tracking-wider">
                {getMoonLabel(s.moon, t)}
              </h4>
            </div>
            <div className="space-y-1">
              {s.prompts.map((p) => (
                <button
                  key={p}
                  onClick={() => onUsePrompt(p)}
                  className="group w-full text-left text-[12px] text-jupiter-muted bg-jupiter-elevated/60 hover:bg-jupiter-elevated hover:text-white border border-jupiter-border hover:border-jupiter-orange/40 rounded-lg px-3 py-2 transition-colors flex items-center gap-2"
                >
                  <span className="material-symbols-outlined text-[14px] text-jupiter-dim group-hover:text-jupiter-orange flex-shrink-0">
                    arrow_forward
                  </span>
                  <span className="flex-1">{p}</span>
                </button>
              ))}
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}
