import { useEffect, useState } from "react";
import {
  credentialsList,
  credentialsSet,
  credentialsAddAccount,
  oauthConnectGmail,
  accountRemove,
  type CredentialEntry,
} from "../../lib/tauri";

interface Props {
  /** Service name. The panel only renders for "io". */
  serviceName: string;
}

type AddPreset = "gmail-oauth" | "custom";

export function CredentialsPanel({ serviceName }: Props) {
  const [entries, setEntries] = useState<CredentialEntry[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [editing, setEditing] = useState<string | null>(null);
  const [draftPassword, setDraftPassword] = useState("");
  const [busy, setBusy] = useState(false);

  // Add-account form
  const [adding, setAdding] = useState(false);
  const [addPreset, setAddPreset] = useState<AddPreset>("gmail-oauth");
  const [addAccountName, setAddAccountName] = useState("");
  const [addEmail, setAddEmail] = useState("");
  const [addImapHost, setAddImapHost] = useState("");
  const [addPassword, setAddPassword] = useState("");

  // OAuth flow state
  const [oauthBusy, setOauthBusy] = useState(false);
  const [oauthMessage, setOauthMessage] = useState<string | null>(null);

  const enabled = serviceName === "io";

  const reload = async () => {
    if (!enabled) return;
    try {
      const list = await credentialsList();
      setEntries(list);
      setError(null);
    } catch (e) {
      setError(String(e));
      setEntries([]);
    }
  };

  useEffect(() => {
    reload();
  }, [enabled, serviceName]);

  if (!enabled) return null;

  const handleSavePassword = async (account: string) => {
    if (!draftPassword.trim()) return;
    setBusy(true);
    try {
      await credentialsSet(account, draftPassword);
      setDraftPassword("");
      setEditing(null);
      await reload();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const resetAddForm = () => {
    setAdding(false);
    setAddPreset("gmail-oauth");
    setAddAccountName("");
    setAddEmail("");
    setAddImapHost("");
    setAddPassword("");
    setOauthMessage(null);
  };

  const handleAddCustom = async () => {
    if (!addEmail.trim() || !addPassword || !addAccountName.trim()) return;
    setBusy(true);
    try {
      await credentialsAddAccount({
        account: addAccountName.trim(),
        email: addEmail.trim(),
        password: addPassword,
        imapHost: addImapHost.trim() || undefined,
      });
      resetAddForm();
      await reload();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const handleOAuthConnect = async () => {
    if (!addEmail.trim()) return;
    setOauthBusy(true);
    setOauthMessage("Apertura browser per consenso Google… completa il login e torna qui.");
    setError(null);
    try {
      const msg = await oauthConnectGmail(addEmail.trim());
      setOauthMessage(msg);
      setTimeout(() => {
        resetAddForm();
        reload();
      }, 1500);
    } catch (e) {
      setOauthMessage(null);
      setError(String(e));
    } finally {
      setOauthBusy(false);
    }
  };

  /// Total account removal: keyring (password + OAuth) + .mcp.json env.
  /// The account disappears from the list.
  const handleRemove = async (account: string) => {
    if (
      !confirm(
        `Rimuovere completamente l'account '${account}'?\n` +
          `Cancellerà credenziali (password + OAuth) e l'entry dal .mcp.json.\n` +
          `Le email già indicizzate in Qdrant restano (non vengono toccate).`
      )
    )
      return;
    setBusy(true);
    try {
      await accountRemove(account);
      await reload();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  // Shared input class
  const inputCls =
    "w-full px-2 py-1 text-[11px] rounded bg-jupiter-elevated border border-jupiter-orange/30 text-white focus:outline-none focus:border-jupiter-orange placeholder:text-jupiter-dim/60";

  return (
    <div className="mb-3 border border-jupiter-orange/25 rounded p-2.5 bg-jupiter-bg/40">
      <div className="text-[10px] text-jupiter-dim mb-2 font-semibold uppercase tracking-wider">
        Account credentials
        <span className="ml-1 normal-case text-jupiter-dim/70">(Windows Credential Manager)</span>
      </div>

      {error && (
        <div className="text-[11px] text-jupiter-red mb-2 break-words">{error}</div>
      )}

      {entries === null ? (
        <div className="text-[11px] text-jupiter-dim">Loading…</div>
      ) : entries.length === 0 ? (
        <div className="text-[11px] text-jupiter-dim">
          Nessun account configurato. Click <span className="text-white">+ Add account</span> qui sotto.
        </div>
      ) : (
        <div className="space-y-2">
          {entries.map((e) => (
            <div
              key={e.account}
              className="rounded border border-jupiter-orange/15 bg-jupiter-elevated/40 p-2"
            >
              {/* Account header: name + status */}
              <div className="flex items-center justify-between mb-1">
                <span className="font-mono text-white text-[12px]">{e.account}</span>
                {e.has_oauth ? (
                  <span className="text-jupiter-green text-[10px]">✓ OAuth</span>
                ) : e.has_password ? (
                  <span className="text-jupiter-green text-[10px]">✓ password</span>
                ) : (
                  <span className="text-jupiter-amber text-[10px]">× missing</span>
                )}
              </div>

              {/* Email */}
              <div
                className="text-[10px] text-jupiter-dim mb-1.5 truncate"
                title={e.username}
              >
                {e.username}
              </div>

              {/* Action row OR password editor */}
              {editing === e.account ? (
                <div className="space-y-1.5">
                  <input
                    type="password"
                    autoFocus
                    value={draftPassword}
                    onChange={(ev) => setDraftPassword(ev.target.value)}
                    onKeyDown={(ev) => {
                      if (ev.key === "Enter") handleSavePassword(e.account);
                      if (ev.key === "Escape") {
                        setEditing(null);
                        setDraftPassword("");
                      }
                    }}
                    placeholder="new password"
                    className={inputCls}
                  />
                  <div className="flex gap-1.5">
                    <button
                      onClick={() => handleSavePassword(e.account)}
                      disabled={busy || !draftPassword.trim()}
                      className="flex-1 px-2 py-1 text-[10px] rounded bg-jupiter-green/15 text-jupiter-green border border-jupiter-green/30 hover:bg-jupiter-green/25 disabled:opacity-40 transition-colors"
                    >
                      Save
                    </button>
                    <button
                      onClick={() => {
                        setEditing(null);
                        setDraftPassword("");
                      }}
                      className="px-2 py-1 text-[10px] rounded text-jupiter-dim hover:text-white hover:bg-jupiter-elevated transition-colors"
                    >
                      Cancel
                    </button>
                  </div>
                </div>
              ) : e.has_oauth ? (
                <div className="flex gap-1.5">
                  <span className="flex-1 px-2 py-1 text-[10px] text-jupiter-dim italic">
                    Gestito da Google (refresh token nel keyring)
                  </span>
                  <button
                    onClick={() => handleRemove(e.account)}
                    disabled={busy}
                    className="px-2 py-1 text-[10px] rounded text-jupiter-red hover:bg-jupiter-red/15 transition-colors"
                  >
                    Delete
                  </button>
                </div>
              ) : (
                <div className="flex gap-1.5">
                  <button
                    onClick={() => {
                      setEditing(e.account);
                      setDraftPassword("");
                      setError(null);
                    }}
                    className="flex-1 px-2 py-1 text-[10px] rounded bg-jupiter-orange/15 text-jupiter-orange border border-jupiter-orange/30 hover:bg-jupiter-orange/25 transition-colors"
                  >
                    {e.has_password ? "Update password" : "Set password"}
                  </button>
                  <button
                    onClick={() => handleRemove(e.account)}
                    disabled={busy}
                    className="px-2 py-1 text-[10px] rounded text-jupiter-red hover:bg-jupiter-red/15 transition-colors"
                  >
                    Delete
                  </button>
                </div>
              )}
            </div>
          ))}
        </div>
      )}

      {/* Add account section */}
      <div className="mt-3 pt-2.5 border-t border-jupiter-orange/15">
        {!adding ? (
          <button
            onClick={() => setAdding(true)}
            className="w-full px-2 py-1.5 text-[11px] rounded border border-dashed border-jupiter-orange/30 text-jupiter-orange hover:bg-jupiter-orange/10 transition-colors"
          >
            + Add account
          </button>
        ) : (
          <div className="space-y-2">
            <div className="text-[10px] text-jupiter-dim uppercase tracking-wider font-semibold">
              Add new account
            </div>

            {/* Preset selector — solo 2 opzioni */}
            <div className="grid grid-cols-2 gap-1 text-[10px]">
              <button
                onClick={() => setAddPreset("gmail-oauth")}
                className={`px-2 py-1 rounded border transition-colors ${
                  addPreset === "gmail-oauth"
                    ? "bg-jupiter-orange/20 text-jupiter-orange border-jupiter-orange/40"
                    : "text-jupiter-dim border-jupiter-orange/15 hover:border-jupiter-orange/30"
                }`}
              >
                Gmail (OAuth)
              </button>
              <button
                onClick={() => setAddPreset("custom")}
                className={`px-2 py-1 rounded border transition-colors ${
                  addPreset === "custom"
                    ? "bg-jupiter-orange/20 text-jupiter-orange border-jupiter-orange/40"
                    : "text-jupiter-dim border-jupiter-orange/15 hover:border-jupiter-orange/30"
                }`}
              >
                Custom IMAP
              </button>
            </div>

            {addPreset === "gmail-oauth" ? (
              <>
                {/* Email only — client_id/secret hardcoded in the binary */}
                <div className="space-y-1">
                  <label className="block text-[10px] text-jupiter-dim uppercase tracking-wider">
                    Email Gmail
                  </label>
                  <input
                    type="email"
                    autoFocus
                    value={addEmail}
                    onChange={(ev) => setAddEmail(ev.target.value)}
                    onKeyDown={(ev) => {
                      if (ev.key === "Enter") handleOAuthConnect();
                      if (ev.key === "Escape") resetAddForm();
                    }}
                    placeholder="you@gmail.com  o  you@yourworkspace.com"
                    className={inputCls}
                  />
                </div>

                <div className="text-[10px] text-jupiter-dim leading-snug">
                  Click <span className="text-white">Connect with Google</span> →
                  si apre il browser → fai login + accetta consenso. Niente
                  password salvata: solo un refresh token cifrato nel keyring.
                </div>

                {oauthMessage && (
                  <div className="text-[10px] text-jupiter-orange leading-snug">
                    {oauthMessage}
                  </div>
                )}

                <div className="flex gap-1.5 pt-0.5">
                  <button
                    onClick={handleOAuthConnect}
                    disabled={oauthBusy || !addEmail.trim()}
                    className="flex-1 px-2 py-1 text-[10px] rounded bg-jupiter-green/15 text-jupiter-green border border-jupiter-green/30 hover:bg-jupiter-green/25 disabled:opacity-40 transition-colors"
                  >
                    {oauthBusy ? "Apertura browser…" : "Connect with Google"}
                  </button>
                  <button
                    onClick={resetAddForm}
                    disabled={oauthBusy}
                    className="px-2 py-1 text-[10px] rounded text-jupiter-dim hover:text-white hover:bg-jupiter-elevated transition-colors"
                  >
                    Cancel
                  </button>
                </div>
              </>
            ) : (
              <>
                {/* Custom IMAP */}
                <div className="space-y-1">
                  <label className="block text-[10px] text-jupiter-dim uppercase tracking-wider">
                    Account name
                  </label>
                  <input
                    autoFocus
                    value={addAccountName}
                    onChange={(ev) => setAddAccountName(ev.target.value)}
                    placeholder="e.g. outlook, work"
                    className={inputCls}
                  />
                </div>

                <div className="space-y-1">
                  <label className="block text-[10px] text-jupiter-dim uppercase tracking-wider">
                    Email address
                  </label>
                  <input
                    type="email"
                    value={addEmail}
                    onChange={(ev) => setAddEmail(ev.target.value)}
                    placeholder="you@example.com"
                    className={inputCls}
                  />
                </div>

                <div className="space-y-1">
                  <label className="block text-[10px] text-jupiter-dim uppercase tracking-wider">
                    IMAP host
                  </label>
                  <input
                    value={addImapHost}
                    onChange={(ev) => setAddImapHost(ev.target.value)}
                    placeholder="imap.example.com"
                    className={inputCls}
                  />
                </div>

                <div className="space-y-1">
                  <label className="block text-[10px] text-jupiter-dim uppercase tracking-wider">
                    Password
                  </label>
                  <input
                    type="password"
                    value={addPassword}
                    onChange={(ev) => setAddPassword(ev.target.value)}
                    onKeyDown={(ev) => {
                      if (ev.key === "Enter") handleAddCustom();
                      if (ev.key === "Escape") resetAddForm();
                    }}
                    placeholder="password"
                    className={inputCls}
                  />
                </div>

                <div className="flex gap-1.5 pt-0.5">
                  <button
                    onClick={handleAddCustom}
                    disabled={
                      busy ||
                      !addEmail.trim() ||
                      !addPassword ||
                      !addAccountName.trim()
                    }
                    className="flex-1 px-2 py-1 text-[10px] rounded bg-jupiter-green/15 text-jupiter-green border border-jupiter-green/30 hover:bg-jupiter-green/25 disabled:opacity-40 transition-colors"
                  >
                    Add account
                  </button>
                  <button
                    onClick={resetAddForm}
                    className="px-2 py-1 text-[10px] rounded text-jupiter-dim hover:text-white hover:bg-jupiter-elevated transition-colors"
                  >
                    Cancel
                  </button>
                </div>
              </>
            )}
          </div>
        )}
      </div>

      <div className="text-[10px] text-jupiter-dim/80 mt-2.5 leading-snug">
        Le credenziali sono cifrate dal Windows Credential Manager (servizio{" "}
        <code className="text-white">MoonIo</code>). Dopo aver salvato, fai{" "}
        <span className="text-white">Restart</span> del server `io`.
      </div>
    </div>
  );
}
