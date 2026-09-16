// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

import { useState } from "react";
import { addLocalMoon, addRemoteMoon } from "../../lib/tauri";
import { useT } from "../../i18n";

interface Props {
  onClose: () => void;
  onAdded: () => void;
  suggestedPort: number;
}

type Mode = "local" | "remote";

export function AddMoonModal({ onClose, onAdded, suggestedPort }: Props) {
  const { t } = useT();
  const [mode, setMode] = useState<Mode>("local");

  // Local fields
  const [name, setName] = useState("");
  const [command, setCommand] = useState("");
  const [cwd, setCwd] = useState("");
  const [port, setPort] = useState(suggestedPort);
  const [envText, setEnvText] = useState("");

  // Remote fields
  const [remoteName, setRemoteName] = useState("");
  const [remoteUrl, setRemoteUrl] = useState("");
  const [remoteType, setRemoteType] = useState<"http" | "sse">("http");

  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  const handleConfirm = async () => {
    setError(null);
    setLoading(true);
    try {
      if (mode === "local") {
        if (!name.trim()) return setError(t("addMoon.errName"));
        if (!command.trim()) return setError(t("addMoon.errCommand"));
        if (port < 1024 || port > 65535) return setError(t("addMoon.errPort"));
        const envVars = envText.split("\n").map((l) => l.trim()).filter((l) => l.includes("="));
        await addLocalMoon({ name: name.trim(), command: command.trim(), args: [], cwd: cwd.trim() || null, envVars, port });
      } else {
        if (!remoteName.trim()) return setError(t("addMoon.errName"));
        if (!remoteUrl.trim()) return setError(t("addMoon.errUrl"));
        await addRemoteMoon(remoteName.trim(), remoteUrl.trim(), remoteType);
      }
      onAdded();
      onClose();
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60"
      onClick={(e) => e.target === e.currentTarget && onClose()}
    >
      <div className="bg-jupiter-surface border border-jupiter-border rounded-xl card-shadow w-[440px] p-6 space-y-4">
        <h2 className="flex items-center gap-2 text-[13px] font-bold text-white uppercase tracking-wider">
          <span className="material-symbols-outlined text-[16px] text-jupiter-orange">
            add_circle
          </span>
          {t("addMoon.title")}
        </h2>

        {/* Mode toggle */}
        <div className="flex gap-1 bg-jupiter-elevated rounded-lg p-1">
          {(["local", "remote"] as Mode[]).map((m) => (
            <button
              key={m}
              onClick={() => { setMode(m); setError(null); }}
              className={`flex-1 py-1 text-[11px] rounded-md transition-colors ${
                mode === m
                  ? "bg-jupiter-orange text-white"
                  : "text-jupiter-dim hover:text-white"
              }`}
            >
              {m === "local" ? t("addMoon.local") : t("addMoon.remote")}
            </button>
          ))}
        </div>

        {mode === "local" ? (
          <div className="space-y-3">
            <label className="block space-y-1">
              <span className="text-[11px] text-jupiter-dim">{t("addMoon.name")}</span>
              <input autoFocus value={name} onChange={(e) => setName(e.target.value)}
                placeholder={t("addMoon.namePh")}
                className="w-full bg-jupiter-bg border border-jupiter-border rounded-lg px-3 py-1.5 text-[12px] text-white placeholder:text-jupiter-dim outline-none focus:border-jupiter-orange" />
            </label>
            <label className="block space-y-1">
              <span className="text-[11px] text-jupiter-dim">{t("addMoon.command")}</span>
              <input value={command} onChange={(e) => setCommand(e.target.value)}
                placeholder={t("addMoon.commandPh")}
                className="w-full bg-jupiter-bg border border-jupiter-border rounded-lg px-3 py-1.5 text-[12px] text-white placeholder:text-jupiter-dim outline-none focus:border-jupiter-orange font-mono" />
            </label>
            <div className="flex gap-3">
              <label className="flex-1 block space-y-1">
                <span className="text-[11px] text-jupiter-dim">{t("addMoon.cwd")}</span>
                <input value={cwd} onChange={(e) => setCwd(e.target.value)}
                  placeholder={t("addMoon.cwdPh")}
                  className="w-full bg-jupiter-bg border border-jupiter-border rounded-lg px-3 py-1.5 text-[12px] text-white placeholder:text-jupiter-dim outline-none focus:border-jupiter-orange font-mono" />
              </label>
              <label className="w-20 block space-y-1">
                <span className="text-[11px] text-jupiter-dim">{t("addMoon.port")}</span>
                <input type="number" value={port} onChange={(e) => setPort(Number(e.target.value))}
                  className="w-full bg-jupiter-bg border border-jupiter-border rounded-lg px-3 py-1.5 text-[12px] text-white outline-none focus:border-jupiter-orange font-mono" />
              </label>
            </div>
            <label className="block space-y-1">
              <span className="text-[11px] text-jupiter-dim">{t("addMoon.env")}</span>
              <textarea value={envText} onChange={(e) => setEnvText(e.target.value)} rows={3}
                placeholder={"DATA_DIR=./data\nQDRANT_URL=http://127.0.0.1:6334"}
                className="w-full bg-jupiter-bg border border-jupiter-border rounded-lg px-3 py-1.5 text-[12px] text-white placeholder:text-jupiter-dim outline-none focus:border-jupiter-orange font-mono resize-none" />
            </label>
          </div>
        ) : (
          <div className="space-y-3">
            <label className="block space-y-1">
              <span className="text-[11px] text-jupiter-dim">{t("addMoon.name")}</span>
              <input autoFocus value={remoteName} onChange={(e) => setRemoteName(e.target.value)}
                placeholder={t("addMoon.namePh")}
                className="w-full bg-jupiter-bg border border-jupiter-border rounded-lg px-3 py-1.5 text-[12px] text-white placeholder:text-jupiter-dim outline-none focus:border-jupiter-orange" />
            </label>
            <label className="block space-y-1">
              <span className="text-[11px] text-jupiter-dim">{t("addMoon.url")}</span>
              <input value={remoteUrl} onChange={(e) => setRemoteUrl(e.target.value)}
                placeholder={t("addMoon.urlPh")}
                className="w-full bg-jupiter-bg border border-jupiter-border rounded-lg px-3 py-1.5 text-[12px] text-white placeholder:text-jupiter-dim outline-none focus:border-jupiter-orange font-mono" />
            </label>
            <label className="block space-y-1">
              <span className="text-[11px] text-jupiter-dim">{t("addMoon.transport")}</span>
              <div className="flex gap-2">
                {(["http", "sse"] as const).map((t) => (
                  <button key={t} onClick={() => setRemoteType(t)}
                    className={`px-4 py-1.5 text-[12px] rounded-lg border transition-colors font-mono ${
                      remoteType === t
                        ? "border-jupiter-orange text-jupiter-orange bg-jupiter-orange/10"
                        : "border-jupiter-border text-jupiter-dim hover:text-white"
                    }`}>
                    {t}
                  </button>
                ))}
              </div>
            </label>
          </div>
        )}

        {error && <p className="text-[11px] text-jupiter-red">{error}</p>}

        <div className="flex justify-end gap-2 pt-1">
          <button onClick={onClose}
            className="px-3 py-1.5 text-[12px] rounded-lg text-jupiter-dim hover:text-white hover:bg-jupiter-elevated transition-colors">
            {t("common.cancel")}
          </button>
          <button onClick={handleConfirm} disabled={loading}
            className="px-4 py-1.5 text-[12px] rounded-lg bg-jupiter-orange text-white hover:opacity-90 glow-orange disabled:opacity-50 transition-all">
            {loading ? "..." : t("addMoon.add")}
          </button>
        </div>
      </div>
    </div>
  );
}
