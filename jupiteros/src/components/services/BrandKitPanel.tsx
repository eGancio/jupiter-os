// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

import { useEffect, useState } from "react";
import { getBrandKit, setBrandKit } from "../../lib/tauri";

interface Props {
  /** Service name. The panel only renders for "thebe". */
  serviceName: string;
}

const BRAND = "emotion";
const inputCls =
  "w-full px-2.5 py-1.5 text-[11px] rounded-lg bg-jupiter-bg border border-jupiter-border text-jupiter-text placeholder:text-jupiter-dim focus:outline-none focus:border-jupiter-orange";

export function BrandKitPanel({ serviceName }: Props) {
  const enabled = serviceName === "thebe";

  const [name, setName] = useState("");
  const [primary, setPrimary] = useState("#1a1a1a");
  const [accent, setAccent] = useState("#2d7bbf");
  const [vat, setVat] = useState("");
  const [address, setAddress] = useState("");
  const [contacts, setContacts] = useState("");
  const [confidentiality, setConfidentiality] = useState("");
  const [hasLogo, setHasLogo] = useState(false);
  const [logoDataUrl, setLogoDataUrl] = useState<string | null>(null);

  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!enabled) return;
    getBrandKit(BRAND)
      .then((k) => {
        setName(k.name);
        if (k.primary) setPrimary(k.primary);
        if (k.accent) setAccent(k.accent);
        setVat(k.vat);
        setAddress(k.address);
        setContacts(k.contacts);
        setConfidentiality(k.confidentiality);
        setHasLogo(k.hasLogo);
        setError(null);
      })
      .catch((e) => setError(String(e)));
  }, [enabled, serviceName]);

  if (!enabled) return null;

  const onPickLogo = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;
    const reader = new FileReader();
    reader.onload = () => {
      setLogoDataUrl(reader.result as string);
      setMsg("Logo pronto — premi Salva per applicarlo.");
    };
    reader.readAsDataURL(file);
  };

  const handleSave = async () => {
    setBusy(true);
    setMsg(null);
    setError(null);
    try {
      await setBrandKit(BRAND, {
        name,
        primary,
        accent,
        vat,
        address,
        contacts,
        confidentiality,
        logoDataUrl: logoDataUrl || undefined,
      });
      if (logoDataUrl) setHasLogo(true);
      setLogoDataUrl(null);
      setMsg("Salvato ✓ — il prossimo report sarà brandizzato così.");
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="mb-3 rounded-xl p-3 bg-jupiter-elevated/40">
      <div className="flex items-center gap-1.5 text-[10px] text-jupiter-dim mb-2 font-bold uppercase tracking-wider">
        <span className="material-symbols-outlined text-[14px]">palette</span>
        Brand kit — EMotion
      </div>

      <div className="space-y-2">
        {/* Logo */}
        <div className="flex items-center gap-2">
          <label className="w-20 text-[10px] text-jupiter-dim shrink-0">Logo</label>
          <label className="px-2 py-1 text-[10px] rounded-lg bg-jupiter-elevated text-jupiter-orange hover:bg-jupiter-orange hover:text-white cursor-pointer transition-colors">
            {logoDataUrl ? "Logo selezionato" : hasLogo ? "Sostituisci logo" : "Carica logo"}
            <input type="file" accept="image/*" className="hidden" onChange={onPickLogo} />
          </label>
          {logoDataUrl ? (
            <img src={logoDataUrl} alt="anteprima" className="h-6 w-auto rounded-md bg-white/5" />
          ) : (
            <span className="text-[10px] text-jupiter-dim">{hasLogo ? "presente" : "nessuno"}</span>
          )}
        </div>

        {/* Colors */}
        <div className="flex items-center gap-3">
          <div className="flex items-center gap-1.5">
            <label className="w-20 text-[10px] text-jupiter-dim shrink-0">Primario</label>
            <input type="color" value={primary} onChange={(e) => setPrimary(e.target.value)}
              className="h-6 w-8 rounded-lg bg-transparent border border-jupiter-border cursor-pointer" />
            <span className="text-[10px] text-jupiter-dim font-mono">{primary}</span>
          </div>
          <div className="flex items-center gap-1.5">
            <label className="text-[10px] text-jupiter-dim shrink-0">Accent</label>
            <input type="color" value={accent} onChange={(e) => setAccent(e.target.value)}
              className="h-6 w-8 rounded-lg bg-transparent border border-jupiter-border cursor-pointer" />
            <span className="text-[10px] text-jupiter-dim font-mono">{accent}</span>
          </div>
        </div>

        {/* Company fields */}
        <div className="flex items-center gap-2">
          <label className="w-20 text-[10px] text-jupiter-dim shrink-0">Ragione sociale</label>
          <input className={inputCls} value={name} onChange={(e) => setName(e.target.value)} placeholder="EMotion S.r.l." />
        </div>
        <div className="flex items-center gap-2">
          <label className="w-20 text-[10px] text-jupiter-dim shrink-0">P.IVA</label>
          <input className={inputCls} value={vat} onChange={(e) => setVat(e.target.value)} placeholder="P.IVA / C.F." />
        </div>
        <div className="flex items-center gap-2">
          <label className="w-20 text-[10px] text-jupiter-dim shrink-0">Indirizzo</label>
          <input className={inputCls} value={address} onChange={(e) => setAddress(e.target.value)} placeholder="Via …, CAP Città (PR)" />
        </div>
        <div className="flex items-center gap-2">
          <label className="w-20 text-[10px] text-jupiter-dim shrink-0">Contatti</label>
          <input className={inputCls} value={contacts} onChange={(e) => setContacts(e.target.value)} placeholder="tel · email · web" />
        </div>
        <div className="flex items-center gap-2">
          <label className="w-20 text-[10px] text-jupiter-dim shrink-0">Nota footer</label>
          <input className={inputCls} value={confidentiality} onChange={(e) => setConfidentiality(e.target.value)} placeholder="Documento riservato e confidenziale." />
        </div>

        <button
          onClick={handleSave}
          disabled={busy}
          className="w-full px-2 py-1.5 text-[11px] font-bold uppercase tracking-wider rounded-lg bg-jupiter-orange text-white hover:opacity-90 disabled:opacity-40 transition-all"
        >
          {busy ? "Salvataggio…" : "Salva brand kit"}
        </button>

        {msg && <div className="text-[10px] text-jupiter-green">{msg}</div>}
        {error && <div className="text-[10px] text-jupiter-red">{error}</div>}
      </div>
    </div>
  );
}
