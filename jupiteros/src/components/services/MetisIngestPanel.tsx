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
import { useT } from "../../i18n";

const NATURES = ["generico", "norma"] as const;
const DOC_EXTS = ["pdf", "docx", "xlsx", "xls", "txt", "md", "csv", "html", "json", "xml"];

export function MetisIngestPanel() {
  const { t } = useT();
  // Translate a nature value, falling back to the raw value for unknown natures.
  const natureLabel = (n: string) => {
    const key = `metis.nature.${n}`;
    const s = t(key);
    return s === key ? n : s;
  };
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
      filters: [{ name: t("metis.dialogFilterName"), extensions: DOC_EXTS }],
    });
    if (!sel) return;
    await run(Array.isArray(sel) ? sel : [sel]);
  }, [run, t]);

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
        {t("metis.nature")}
      </div>
      <div className="grid grid-cols-2 gap-1 bg-jupiter-elevated rounded-lg p-1 mb-2">
        {NATURES.map((n) => (
          <button
            key={n}
            onClick={() => setNature(n)}
            className={`px-2 py-1 text-[10px] rounded-md transition-colors ${
              nature === n
                ? "bg-jupiter-orange text-white"
                : "text-jupiter-dim hover:text-white"
            }`}
          >
            {natureLabel(n)}
          </button>
        ))}
      </div>

      <label className="flex items-center gap-1.5 text-[10px] text-jupiter-dim cursor-pointer select-none mb-2">
        <input
          type="checkbox"
          checked={recursive}
          onChange={(e) => setRecursive(e.target.checked)}
        />
        {t("metis.includeSubfolders")}
      </label>

      {/* Picker buttons */}
      <div className="flex gap-2 mb-2 flex-shrink-0">
        <button
          onClick={pickFiles}
          disabled={loading}
          className="flex-1 flex items-center justify-center gap-1.5 px-2.5 py-1.5 text-[11px] font-bold uppercase tracking-wider rounded-lg bg-jupiter-orange text-white hover:opacity-90 disabled:opacity-40 transition-all"
        >
          <span className="material-symbols-outlined text-[14px]">upload_file</span>
          {t("metis.pickFiles")}
        </button>
        <button
          onClick={pickFolder}
          disabled={loading}
          className="flex-1 flex items-center justify-center gap-1.5 px-2.5 py-1.5 text-[11px] rounded-lg bg-jupiter-elevated text-jupiter-muted hover:text-white disabled:opacity-40 transition-colors"
        >
          <span className="material-symbols-outlined text-[14px]">folder_open</span>
          {t("metis.pickFolder")}
        </button>
      </div>

      {error && (
        <div className="text-[11px] text-jupiter-red bg-jupiter-red/10 border border-jupiter-red/30 rounded-lg p-2 mb-2 whitespace-pre-wrap">
          {error}
        </div>
      )}

      {loading && (
        <div className="text-[11px] text-jupiter-amber mb-2">{t("metis.indexing")}</div>
      )}

      {/* Last-run summary (compact) */}
      {results && results.length > 0 && (
        <div className="text-[10px] text-jupiter-dim mb-2" title={results.map((r) => `${r.title}: ${r.status}`).join("\n")}>
          {t("metis.lastIngest")}{" "}
          {Object.entries(counts)
            .map(([k, v]) => `${v} ${k}`)
            .join(" · ")}
        </div>
      )}

      {/* Indexed documents */}
      <div className="flex items-center justify-between mb-1 flex-shrink-0">
        <div className="text-[10px] font-semibold uppercase tracking-wider text-jupiter-dim">
          {t("metis.indexedDocs")}{docs ? ` (${docs.length})` : ""}
        </div>
        <button
          onClick={loadDocs}
          className="flex items-center gap-1 text-[10px] text-jupiter-dim hover:text-white transition-colors"
        >
          <span className="material-symbols-outlined text-[12px]">refresh</span>
          {t("metis.refresh")}
        </button>
      </div>
      <div className="flex-1 overflow-y-auto custom-scrollbar bg-jupiter-bg rounded-xl p-2 min-h-0">
        {docs === null ? (
          <div className="text-[11px] text-jupiter-dim">{t("metis.loading")}</div>
        ) : docs.length === 0 ? (
          <div className="text-[11px] text-jupiter-dim">{t("metis.empty")}</div>
        ) : (
          <div className="space-y-0.5">
            {docs.map((d) => (
              <div
                key={d.doc_id}
                className="flex items-center gap-1.5 text-[10px] px-1.5 py-0.5 rounded-md hover:bg-jupiter-elevated transition-colors group"
              >
                <button
                  onClick={() => metisOpenFile(d.source_path).catch((e) => setError(String(e)))}
                  title={t("metis.openFile", { path: d.source_path })}
                  className="flex-1 min-w-0 text-left text-white truncate hover:text-jupiter-orange transition-colors"
                >
                  {d.title || d.source_path}
                </button>
                <span className="text-jupiter-dim whitespace-nowrap flex-shrink-0">
                  {natureLabel(d.nature)}
                  {d.pages > 0 ? ` · ${d.pages}p` : ""} · {d.chunks}
                </span>
                <button
                  onClick={() => metisRevealFile(d.source_path).catch((e) => setError(String(e)))}
                  title={t("metis.revealFile")}
                  className="text-jupiter-dim hover:text-white px-0.5 transition-colors opacity-0 group-hover:opacity-100"
                >
                  📂
                </button>
                <button
                  onClick={() => removeDoc(d.doc_id)}
                  title={t("metis.removeDoc")}
                  className="text-jupiter-dim hover:text-jupiter-red px-0.5 transition-colors opacity-0 group-hover:opacity-100"
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
