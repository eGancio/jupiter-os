// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

import { useState, useCallback, useEffect } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import {
  metisIngestPaths,
  metisListDocuments,
  metisDeleteDocument,
  metisOpenFile,
  metisRevealFile,
  type MetisIngestResult,
  type MetisDoc,
} from "../../lib/tauri";

const NATURES = ["generico", "norma"] as const;
const DOC_EXTS = ["pdf", "docx", "xlsx", "xls", "txt", "md", "csv", "html", "json", "xml"];

export function MetisIngestPanel() {
  const [nature, setNature] = useState<string>("generico");
  const [recursive, setRecursive] = useState(true);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [results, setResults] = useState<MetisIngestResult[] | null>(null);
  const [docs, setDocs] = useState<MetisDoc[] | null>(null);

  const loadDocs = useCallback(async () => {
    try {
      setDocs(await metisListDocuments());
    } catch (e) {
      setError(String(e));
    }
  }, []);

  // Load the indexed-documents list when the panel opens.
  useEffect(() => {
    loadDocs();
  }, [loadDocs]);

  const run = useCallback(
    async (paths: string[]) => {
      setLoading(true);
      setError(null);
      try {
        const res = await metisIngestPaths(paths, nature, recursive, false);
        setResults(res);
        await loadDocs(); // refresh the list after ingest
      } catch (e) {
        setError(String(e));
      } finally {
        setLoading(false);
      }
    },
    [nature, recursive, loadDocs]
  );

  const removeDoc = useCallback(
    async (docId: string) => {
      try {
        await metisDeleteDocument(docId);
        await loadDocs();
      } catch (e) {
        setError(String(e));
      }
    },
    [loadDocs]
  );

  const pickFiles = useCallback(async () => {
    const sel = await open({
      multiple: true,
      directory: false,
      filters: [{ name: "Documenti", extensions: DOC_EXTS }],
    });
    if (!sel) return;
    await run(Array.isArray(sel) ? sel : [sel]);
  }, [run]);

  const pickFolder = useCallback(async () => {
    const sel = await open({ directory: true, multiple: false });
    if (!sel || Array.isArray(sel)) return;
    await run([sel]);
  }, [run]);

  const counts = (results ?? []).reduce<Record<string, number>>((acc, r) => {
    acc[r.status] = (acc[r.status] ?? 0) + 1;
    return acc;
  }, {});

  return (
    <div className="flex flex-col min-h-0 flex-1">
      {/* Nature — button row (native <select> renders white-on-white on WebKitGTK) */}
      <div className="text-[10px] font-semibold uppercase tracking-wider text-jupiter-dim mb-1">
        Natura
      </div>
      <div className="grid grid-cols-2 gap-1 mb-2">
        {NATURES.map((n) => (
          <button
            key={n}
            onClick={() => setNature(n)}
            className={`px-2 py-1 text-[10px] rounded border transition-colors ${
              nature === n
                ? "bg-jupiter-orange/20 text-jupiter-orange border-jupiter-orange/40"
                : "text-jupiter-dim border-jupiter-orange/15 hover:border-jupiter-orange/30"
            }`}
          >
            {n}
          </button>
        ))}
      </div>

      <label className="flex items-center gap-1.5 text-[10px] text-jupiter-dim cursor-pointer select-none mb-2">
        <input
          type="checkbox"
          checked={recursive}
          onChange={(e) => setRecursive(e.target.checked)}
        />
        Includi sottocartelle
      </label>

      {/* Picker buttons */}
      <div className="flex gap-2 mb-2 flex-shrink-0">
        <button
          onClick={pickFiles}
          disabled={loading}
          className="flex-1 px-2.5 py-1.5 text-[11px] rounded border bg-jupiter-orange/15 text-jupiter-orange border-jupiter-orange/30 hover:bg-jupiter-orange/25 disabled:opacity-40 transition-colors"
        >
          Seleziona file…
        </button>
        <button
          onClick={pickFolder}
          disabled={loading}
          className="flex-1 px-2.5 py-1.5 text-[11px] rounded border bg-jupiter-elevated text-jupiter-dim border-jupiter-orange/20 hover:text-white hover:border-jupiter-orange/40 disabled:opacity-40 transition-colors"
        >
          Seleziona cartella…
        </button>
      </div>

      <p className="text-[10px] text-jupiter-dim mb-2">
        I documenti vengono indicizzati localmente (estrazione → chunk → embedding → Qdrant).
        Le risposte in chat citeranno fonte e pagina. PDF scansionati: OCR non ancora disponibile.
      </p>

      {error && (
        <div className="text-[11px] text-jupiter-red bg-jupiter-red/10 border border-jupiter-red/30 rounded p-2 mb-2 whitespace-pre-wrap">
          {error}
        </div>
      )}

      {loading && (
        <div className="text-[11px] text-jupiter-amber mb-2">Indicizzazione in corso…</div>
      )}

      {/* Last-run summary (compact) */}
      {results && results.length > 0 && (
        <div className="text-[10px] text-jupiter-dim mb-2" title={results.map((r) => `${r.title}: ${r.status}`).join("\n")}>
          Ultimo ingest:{" "}
          {Object.entries(counts)
            .map(([k, v]) => `${v} ${k}`)
            .join(" · ")}
        </div>
      )}

      {/* Indexed documents */}
      <div className="flex items-center justify-between mb-1 flex-shrink-0">
        <div className="text-[10px] font-semibold uppercase tracking-wider text-jupiter-dim">
          Documenti indicizzati{docs ? ` (${docs.length})` : ""}
        </div>
        <button
          onClick={loadDocs}
          className="text-[10px] text-jupiter-dim hover:text-white transition-colors"
        >
          ⟳ aggiorna
        </button>
      </div>
      <div className="flex-1 overflow-y-auto bg-jupiter-bg rounded border border-jupiter-orange/25 p-2 min-h-0">
        {docs === null ? (
          <div className="text-[11px] text-jupiter-dim">Caricamento…</div>
        ) : docs.length === 0 ? (
          <div className="text-[11px] text-jupiter-dim">Nessun documento nella knowledge base.</div>
        ) : (
          <div className="space-y-1">
            {docs.map((d) => (
              <div
                key={d.doc_id}
                className="flex items-center gap-2 text-[10px] px-2 py-1 rounded bg-jupiter-elevated"
              >
                <button
                  onClick={() => metisOpenFile(d.source_path).catch((e) => setError(String(e)))}
                  title={`Apri: ${d.source_path}`}
                  className="flex-1 min-w-0 text-left group"
                >
                  <div className="text-white truncate group-hover:text-jupiter-orange transition-colors">
                    {d.title || d.source_path}
                  </div>
                  <div className="text-jupiter-dim">
                    {d.nature}
                    {d.pages > 0 ? ` · ${d.pages}p` : ""} · {d.chunks} chunk
                  </div>
                </button>
                <button
                  onClick={() => metisRevealFile(d.source_path).catch((e) => setError(String(e)))}
                  title="Mostra nel file manager"
                  className="text-jupiter-dim hover:text-white px-1 transition-colors"
                >
                  📂
                </button>
                <button
                  onClick={() => removeDoc(d.doc_id)}
                  title="Rimuovi dalla knowledge base"
                  className="text-jupiter-dim hover:text-jupiter-red px-1 transition-colors"
                >
                  ✕
                </button>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
