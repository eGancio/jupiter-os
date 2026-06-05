// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

import { useEffect, useState, useCallback } from "react";
import { listEmailsIo, type EmailItem } from "../../lib/tauri";

function fmtDate(iso: string): string {
  const d = new Date(iso);
  if (isNaN(d.getTime())) return iso;
  return d.toLocaleString(undefined, {
    year: "numeric", month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit",
  });
}

/** "Name <email>" → "Name" if present, else the raw string. */
function shortSender(s: string): string {
  const m = s.match(/^\s*"?([^"<]+?)"?\s*<.*>$/);
  return (m ? m[1] : s).trim();
}

export function EmailsPanel() {
  const [emails, setEmails] = useState<EmailItem[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [expanded, setExpanded] = useState<string | null>(null);

  const load = useCallback(() => {
    setLoading(true);
    setError(null);
    listEmailsIo(50)
      .then(setEmails)
      .catch((e) => setError(String(e)))
      .finally(() => setLoading(false));
  }, []);

  useEffect(() => { load(); }, [load]);

  return (
    <div className="flex flex-col min-h-0 flex-1">
      {/* Header */}
      <div className="flex items-center justify-between mb-2 flex-shrink-0">
        <span className="text-[11px] font-semibold uppercase tracking-wider text-jupiter-dim">
          Inbox{emails.length > 0 && <span className="text-jupiter-muted"> ({emails.length})</span>}
        </span>
        <button
          onClick={load}
          disabled={loading}
          className="px-2 py-0.5 text-[10px] rounded border border-jupiter-orange/20 text-jupiter-dim hover:text-white hover:border-jupiter-orange/40 disabled:opacity-40 transition-colors"
        >
          {loading ? "…" : "↻ refresh"}
        </button>
      </div>

      {/* Body */}
      <div className="flex-1 overflow-y-auto bg-jupiter-bg rounded border border-jupiter-orange/25 p-2 min-h-0">
        {error ? (
          <div className="text-[11px] text-jupiter-red break-words p-1 leading-relaxed">{error}</div>
        ) : loading && emails.length === 0 ? (
          <div className="text-[11px] text-jupiter-dim p-1">Caricamento…</div>
        ) : emails.length === 0 ? (
          <div className="text-[11px] text-jupiter-dim p-1 leading-relaxed">
            Nessuna email indicizzata. Avvia Moon Io e attendi l'indicizzazione della INBOX.
          </div>
        ) : (
          <div className="space-y-1.5">
            {emails.map((e) => {
              const key = `${e.account}-${e.uid}`;
              const open = expanded === key;
              return (
                <div
                  key={key}
                  className="rounded border border-jupiter-orange/15 bg-jupiter-elevated/40 overflow-hidden"
                >
                  <button
                    onClick={() => setExpanded(open ? null : key)}
                    className="w-full text-left px-2 py-1.5 hover:bg-jupiter-elevated transition-colors"
                  >
                    <div className="flex items-center justify-between gap-2">
                      <span className="text-[12px] text-white truncate font-medium">
                        {e.subject || "(nessun oggetto)"}
                      </span>
                      <span className="text-[10px] text-jupiter-dim flex-shrink-0">{fmtDate(e.date)}</span>
                    </div>
                    <div className="text-[10px] text-jupiter-muted truncate">
                      {shortSender(e.sender)}
                      <span className="text-jupiter-dim/50"> · {e.account}</span>
                    </div>
                    {!open && e.body && (
                      <div className="text-[10px] text-jupiter-dim truncate mt-0.5">
                        {e.body.replace(/\s+/g, " ").slice(0, 140)}
                      </div>
                    )}
                  </button>
                  {open && (
                    <div className="px-2 py-2 border-t border-jupiter-orange/10 bg-jupiter-bg/60">
                      <div className="text-[10px] text-jupiter-dim mb-1.5 space-y-0.5">
                        <div className="break-words"><span className="text-jupiter-dim/60">Da:</span> {e.sender}</div>
                        <div className="break-words"><span className="text-jupiter-dim/60">A:</span> {e.recipient}</div>
                      </div>
                      <div className="text-[11px] text-jupiter-muted whitespace-pre-wrap break-words leading-relaxed max-h-72 overflow-y-auto">
                        {e.body || "(corpo vuoto)"}
                      </div>
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
}
