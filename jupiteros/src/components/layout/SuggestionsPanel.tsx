// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

interface Props {
  /** Close the panel — the right bar then stays blank until a Moon is selected. */
  onClose: () => void;
  /** Send an example prompt straight to the chat. */
  onUsePrompt: (text: string) => void;
}

interface Section {
  icon: string;
  color: string;
  title: string;
  prompts: string[];
}

// What JupiterOS can do, grouped by Moon, with ready-to-run example prompts.
const SECTIONS: Section[] = [
  {
    icon: "mail",
    color: "#ff6b1a",
    title: "Email · Io",
    prompts: [
      "Leggi le ultime email ricevute oggi",
      "Cerca le email che parlano di fatture",
      "Riassumi le email più recenti",
    ],
  },
  {
    icon: "forum",
    color: "#ff3d8b",
    title: "Messaggi · Europa",
    prompts: [
      "Mostrami gli ultimi messaggi Telegram",
      "Cerca i messaggi su un argomento",
    ],
  },
  {
    icon: "bar_chart",
    color: "#8b5cf6",
    title: "Grafici · Amalthea",
    prompts: [
      "Crea un grafico a barre con dei dati di esempio",
      "Genera un diagramma di flusso di un processo",
    ],
  },
  {
    icon: "travel_explore",
    color: "#10b981",
    title: "Ricerca web · Himalia",
    prompts: ["Cerca sul web le ultime novità sull'AI"],
  },
  {
    icon: "newspaper",
    color: "#ec4899",
    title: "News · Elara",
    prompts: ["Trova notizie recenti su un tema che mi interessa"],
  },
  {
    icon: "menu_book",
    color: "#06b6d4",
    title: "Documenti · Metis",
    prompts: ["Cosa dice un documento indicizzato su un certo tema?"],
  },
];

export function SuggestionsPanel({ onClose, onUsePrompt }: Props) {
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
          <div key={s.title} className="space-y-1.5">
            <div className="flex items-center gap-2">
              <span className="material-symbols-outlined text-[16px]" style={{ color: s.color }}>
                {s.icon}
              </span>
              <h4 className="text-[11px] font-bold text-jupiter-text uppercase tracking-wider">
                {s.title}
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
