// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

import { useEffect, useState } from "react";
import {
  adsCredentialsStatus,
  adsCredentialsSet,
  adsCredentialsDelete,
  restartSidecar,
  type AdsPlatformStatus,
} from "../../lib/tauri";
import { useT } from "../../i18n";

interface Props {
  /** Service name. The panel only renders for "google-ads" / "meta-ads". */
  serviceName: string;
}

interface FieldDef {
  /** Canonical env-var key (matches the Rust side). */
  key: string;
  labelKey: string;
  optional?: boolean;
}

const FIELDS: Record<string, FieldDef[]> = {
  "google-ads": [
    { key: "GOOGLE_ADS_DEVELOPER_TOKEN", labelKey: "ads.f.developerToken" },
    { key: "GOOGLE_ADS_CLIENT_ID", labelKey: "ads.f.clientId" },
    { key: "GOOGLE_ADS_CLIENT_SECRET", labelKey: "ads.f.clientSecret" },
    { key: "GOOGLE_ADS_REFRESH_TOKEN", labelKey: "ads.f.refreshToken" },
    { key: "GOOGLE_ADS_LOGIN_CUSTOMER_ID", labelKey: "ads.f.loginCustomerId", optional: true },
  ],
  "meta-ads": [
    { key: "META_ACCESS_TOKEN", labelKey: "ads.f.accessToken" },
    { key: "META_AD_ACCOUNT_ID", labelKey: "ads.f.adAccountId", optional: true },
  ],
};

export function AdsCredentialsPanel({ serviceName }: Props) {
  const { t } = useT();
  const enabled = serviceName === "google-ads" || serviceName === "meta-ads";

  const [status, setStatus] = useState<AdsPlatformStatus | null>(null);
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const fields = FIELDS[serviceName] ?? [];

  const reload = async () => {
    if (!enabled) return;
    try {
      const all = await adsCredentialsStatus();
      setStatus(all.find((p) => p.platform === serviceName) ?? null);
      setError(null);
    } catch (e) {
      setError(String(e));
      setStatus(null);
    }
  };

  useEffect(() => {
    setDrafts({});
    setInfo(null);
    reload();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [enabled, serviceName]);

  if (!enabled) return null;

  const isSet = (key: string) => status?.keys.find((k) => k.key === key)?.set ?? false;

  const handleSave = async () => {
    // Only send fields the user actually typed into; blanks leave existing
    // secrets untouched (you don't have to re-enter a saved key to edit another).
    const values: Record<string, string> = {};
    for (const f of fields) {
      const v = drafts[f.key];
      if (v !== undefined && v.trim() !== "") values[f.key] = v.trim();
    }
    if (Object.keys(values).length === 0) {
      setError(t("ads.nothingToSave"));
      return;
    }
    setBusy(true);
    setError(null);
    setInfo(null);
    try {
      await adsCredentialsSet(serviceName, values);
      setDrafts({});
      await restartSidecar();
      setInfo(t("ads.saved"));
      await reload();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const handleDelete = async () => {
    if (!confirm(t("ads.removeConfirm"))) return;
    setBusy(true);
    setError(null);
    setInfo(null);
    try {
      await adsCredentialsDelete(serviceName);
      setDrafts({});
      await restartSidecar();
      await reload();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const inputCls =
    "w-full px-2.5 py-1.5 text-[11px] rounded-lg bg-jupiter-bg border border-jupiter-border text-white focus:outline-none focus:border-jupiter-orange placeholder:text-jupiter-dim/60";

  return (
    <div className="mb-3 rounded-xl p-3 bg-jupiter-elevated/40">
      <div className="flex items-center gap-1.5 text-[10px] text-jupiter-dim mb-2 font-bold uppercase tracking-wider">
        <span className="material-symbols-outlined text-[14px]">key</span>
        {t("ads.title")}
        <span className="normal-case font-normal text-jupiter-dim/70">{t("ads.keyring")}</span>
        {status?.configured && (
          <span className="ml-auto normal-case text-jupiter-green text-[10px]">
            ✓ {t("ads.configured")}
          </span>
        )}
      </div>

      {error && <div className="text-[11px] text-jupiter-red mb-2 break-words">{error}</div>}
      {info && <div className="text-[11px] text-jupiter-orange mb-2">{info}</div>}

      <div className="space-y-2">
        {fields.map((f) => (
          <div key={f.key} className="space-y-1">
            <label className="flex items-center gap-1.5 text-[10px] text-jupiter-dim uppercase tracking-wider">
              {t(f.labelKey)}
              {f.optional && <span className="text-jupiter-dim/60 normal-case">{t("ads.optional")}</span>}
              {isSet(f.key) ? (
                <span className="ml-auto text-jupiter-green normal-case">✓ {t("ads.set")}</span>
              ) : (
                !f.optional && (
                  <span className="ml-auto text-jupiter-amber normal-case">{t("ads.missing")}</span>
                )
              )}
            </label>
            <input
              type="password"
              autoComplete="off"
              value={drafts[f.key] ?? ""}
              onChange={(ev) => setDrafts({ ...drafts, [f.key]: ev.target.value })}
              placeholder={isSet(f.key) ? t("ads.savedPlaceholder") : t("ads.enterPlaceholder")}
              className={inputCls}
            />
          </div>
        ))}
      </div>

      <div className="flex gap-1.5 pt-2.5">
        <button
          onClick={handleSave}
          disabled={busy}
          className="flex-1 px-2 py-1 text-[10px] font-bold uppercase tracking-wider rounded-lg bg-jupiter-orange text-white hover:opacity-90 disabled:opacity-40 transition-all"
        >
          {busy ? t("ads.savingRestart") : t("common.save")}
        </button>
        {status?.keys.some((k) => k.set) && (
          <button
            onClick={handleDelete}
            disabled={busy}
            className="px-2 py-1 text-[10px] rounded-lg text-jupiter-red hover:bg-jupiter-red/15 transition-colors"
          >
            {t("common.delete")}
          </button>
        )}
      </div>

      <div className="text-[10px] text-jupiter-dim/80 mt-2.5 leading-snug">
        {t("ads.footerBefore")}
        <code className="text-white">MoonAds</code>
        {t("ads.footerAfter")}
      </div>
    </div>
  );
}
