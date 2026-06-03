// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

import { useEffect, useState } from "react";
import {
  europaChannelsList,
  europaChannelRemove,
  telegramSaveApi,
  telegramRequestCode,
  telegramSubmitCode,
  telegramSubmitPassword,
  telegramSaveBot,
  slackSave,
  teamsStartDeviceCode,
  teamsPollDeviceCode,
  stopService,
  restartService,
  type EuropaChannel,
  type DeviceCodeInfo,
} from "../../lib/tauri";
import { useT } from "../../i18n";

interface Props {
  /** Service name. The panel only renders for "europa". */
  serviceName: string;
}

type Tab = "telegram" | "telegram-bot" | "slack" | "teams";
type TgStep = "api" | "phone" | "code" | "2fa" | "done";

const TABS: { id: Tab; label: string }[] = [
  { id: "telegram", label: "Telegram (account)" },
  { id: "telegram-bot", label: "Telegram (bot)" },
  { id: "slack", label: "Slack" },
  { id: "teams", label: "Teams" },
];

const inputCls =
  "w-full px-2 py-1 text-[11px] rounded bg-jupiter-elevated border border-jupiter-orange/30 text-white focus:outline-none focus:border-jupiter-orange placeholder:text-jupiter-dim/60";
const primaryBtn =
  "flex-1 px-2 py-1 text-[10px] rounded bg-jupiter-green/15 text-jupiter-green border border-jupiter-green/30 hover:bg-jupiter-green/25 disabled:opacity-40 transition-colors";
const ghostBtn =
  "px-2 py-1 text-[10px] rounded text-jupiter-dim hover:text-white hover:bg-jupiter-elevated transition-colors";

export function EuropaCredentialsPanel({ serviceName }: Props) {
  const { t } = useT();
  const enabled = serviceName === "europa";

  const [channels, setChannels] = useState<EuropaChannel[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [tab, setTab] = useState<Tab>("telegram");
  const [adding, setAdding] = useState(false);
  // True while a Telegram login is in progress (europa is stopped meanwhile).
  const [loginActive, setLoginActive] = useState(false);

  // Telegram user wizard
  const [tgStep, setTgStep] = useState<TgStep>("api");
  const [apiId, setApiId] = useState("");
  const [apiHash, setApiHash] = useState("");
  const [phone, setPhone] = useState("");
  const [code, setCode] = useState("");
  const [twoFa, setTwoFa] = useState("");

  // Bot / Slack tokens
  const [botToken, setBotToken] = useState("");
  const [slackToken, setSlackToken] = useState("");

  // Teams
  const [teamsClientId, setTeamsClientId] = useState("");
  const [teamsTenant, setTeamsTenant] = useState("");
  const [device, setDevice] = useState<DeviceCodeInfo | null>(null);

  const reload = async () => {
    if (!enabled) return;
    try {
      setChannels(await europaChannelsList());
      setError(null);
    } catch (e) {
      setError(String(e));
      setChannels([]);
    }
  };

  useEffect(() => {
    reload();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [enabled, serviceName]);

  if (!enabled) return null;

  // True if Telegram API id/hash are already saved (skip the API step).
  const tgConfigured =
    channels?.find((c) => c.channel === "telegram")?.configured ?? false;

  const resetForms = () => {
    setAdding(false);
    setTgStep("api");
    setApiId("");
    setApiHash("");
    setPhone("");
    setCode("");
    setTwoFa("");
    setBotToken("");
    setSlackToken("");
    setTeamsClientId("");
    setTeamsTenant("");
    setDevice(null);
    setInfo(null);
  };

  // After (re)starting europa, poll the channel state for a bit so the badge
  // flips to "connected" once moon-europa actually finishes connecting.
  const refreshUntilConnected = async (channel: string) => {
    for (let i = 0; i < 8; i++) {
      await new Promise((r) => setTimeout(r, 2500));
      const list = await europaChannelsList().catch(() => null);
      if (list) {
        setChannels(list);
        if (list.find((c) => c.channel === channel)?.authorized) return;
      }
    }
  };

  // Restart europa if it was stopped for a Telegram login (idempotent).
  const reactivateEuropa = async () => {
    if (loginActive) {
      await restartService("europa").catch(() => {});
      setLoginActive(false);
    }
  };

  // Cancel/abandon the current flow: bring europa back up, then clear the form.
  const cancel = async () => {
    await reactivateEuropa();
    setError(null);
    resetForms();
  };

  // ── Telegram user account (OTP wizard) ─────────────────────────────────
  const handleSaveApi = async () => {
    setBusy(true);
    setError(null);
    try {
      await telegramSaveApi(apiId.trim(), apiHash.trim());
      setTgStep("phone");
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const handleRequestCode = async () => {
    setBusy(true);
    setError(null);
    setInfo(t("europa.info.stopRequest"));
    try {
      // Stop europa so it doesn't hold the SQLite session during login.
      await stopService("europa").catch(() => {});
      setLoginActive(true);
      const res = await telegramRequestCode(phone.trim());
      if (res === "already_authorized") {
        setInfo(t("europa.info.alreadyAuth"));
        await reactivateEuropa();
        await reload();
        resetForms();
      } else {
        setInfo(t("europa.info.codeSent"));
        setTgStep("code");
      }
    } catch (e) {
      setError(String(e));
      await reactivateEuropa();
    } finally {
      setBusy(false);
    }
  };

  const finalizeTelegram = async (msgKey: string) => {
    setInfo(t(msgKey) + " " + t("europa.info.restartVerify"));
    await restartService("europa").catch(() => {});
    setLoginActive(false);
    await reload();
    resetForms();
    void refreshUntilConnected("telegram");
  };

  const handleSubmitCode = async () => {
    setBusy(true);
    setError(null);
    try {
      const res = await telegramSubmitCode(code.trim());
      if (res === "password_required") {
        setInfo(t("europa.info.2faRequired"));
        setTgStep("2fa");
      } else {
        await finalizeTelegram("europa.info.tgLinked");
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const handleSubmit2fa = async () => {
    setBusy(true);
    setError(null);
    try {
      await telegramSubmitPassword(twoFa);
      await finalizeTelegram("europa.info.tgLinked2fa");
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  // ── Token-based channels ───────────────────────────────────────────────
  const handleSaveBot = async () => {
    setBusy(true);
    setError(null);
    try {
      await telegramSaveBot(botToken.trim());
      await finalizeTelegram("europa.info.botConfigured");
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const handleSaveSlack = async () => {
    setBusy(true);
    setError(null);
    try {
      await slackSave(slackToken.trim());
      setInfo(t("europa.info.slackSaved"));
      await restartService("europa").catch(() => {});
      await reload();
      resetForms();
      void refreshUntilConnected("slack");
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  // ── Teams device-code flow ─────────────────────────────────────────────
  const handleTeamsStart = async () => {
    setBusy(true);
    setError(null);
    try {
      const d = await teamsStartDeviceCode(teamsClientId.trim(), teamsTenant.trim());
      setDevice(d);
      setInfo(t("europa.info.teamsConsent"));
      pollTeams();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const pollTeams = () => {
    const tick = async () => {
      try {
        const res = await teamsPollDeviceCode();
        if (res === "done") {
          setInfo(t("europa.info.teamsAuth"));
          await restartService("europa").catch(() => {});
          await reload();
          resetForms();
          void refreshUntilConnected("teams");
          return;
        }
      } catch (e) {
        setError(String(e));
        return;
      }
      setTimeout(tick, 4000);
    };
    setTimeout(tick, 4000);
  };

  const handleRemove = async (channel: string) => {
    if (!confirm(t("europa.removeConfirm", { channel }))) return;
    setBusy(true);
    try {
      await europaChannelRemove(channel);
      await restartService("europa").catch(() => {});
      await reload();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const badge = (c: EuropaChannel) =>
    c.authorized ? (
      <span className="text-jupiter-green text-[10px]">✓ {c.detail}</span>
    ) : c.configured ? (
      <span className="text-jupiter-amber text-[10px]">◐ {c.detail}</span>
    ) : (
      <span className="text-jupiter-dim text-[10px]">× {c.detail}</span>
    );

  return (
    <div className="mb-3 border border-jupiter-orange/25 rounded p-2.5 bg-jupiter-bg/40">
      <div className="text-[10px] text-jupiter-dim mb-2 font-semibold uppercase tracking-wider">
        {t("europa.title")}
        <span className="ml-1 normal-case text-jupiter-dim/70">{t("creds.keyring")}</span>
      </div>

      {error && <div className="text-[11px] text-jupiter-red mb-2 break-words">{error}</div>}

      {/* Configured channels */}
      {channels === null ? (
        <div className="text-[11px] text-jupiter-dim">{t("common.loading")}</div>
      ) : (
        <div className="space-y-2">
          {channels.map((c) => (
            <div
              key={c.channel}
              className="rounded border border-jupiter-orange/15 bg-jupiter-elevated/40 p-2"
            >
              <div className="flex items-center justify-between mb-1">
                <span className="font-mono text-white text-[12px]">
                  {c.channel}
                  <span className="ml-1 text-[9px] text-jupiter-dim">[{c.mode}]</span>
                </span>
                {badge(c)}
              </div>
              {c.configured && (
                <div className="flex gap-1.5">
                  <button onClick={() => handleRemove(c.channel)} disabled={busy} className={ghostBtn}>
                    {t("europa.remove")}
                  </button>
                </div>
              )}
            </div>
          ))}
        </div>
      )}

      {/* Add / configure section */}
      <div className="mt-3 pt-2.5 border-t border-jupiter-orange/15">
        {!adding ? (
          <button
            onClick={() => {
              resetForms();
              setAdding(true);
              setTab("telegram");
              setTgStep(tgConfigured ? "phone" : "api");
            }}
            className="w-full px-2 py-1.5 text-[11px] rounded border border-dashed border-jupiter-orange/30 text-jupiter-orange hover:bg-jupiter-orange/10 transition-colors"
          >
            {t("europa.configure")}
          </button>
        ) : (
          <div className="space-y-2">
            {/* Channel selector */}
            <div className="grid grid-cols-2 gap-1 text-[10px]">
              {TABS.map((t) => (
                <button
                  key={t.id}
                  onClick={() => {
                    setTab(t.id);
                    setInfo(null);
                    setError(null);
                    if (t.id === "telegram") setTgStep(tgConfigured ? "phone" : "api");
                  }}
                  className={`px-2 py-1 rounded border transition-colors ${
                    tab === t.id
                      ? "bg-jupiter-orange/20 text-jupiter-orange border-jupiter-orange/40"
                      : "text-jupiter-dim border-jupiter-orange/15 hover:border-jupiter-orange/30"
                  }`}
                >
                  {t.label}
                </button>
              ))}
            </div>

            {info && <div className="text-[10px] text-jupiter-orange leading-snug">{info}</div>}

            {/* ── Telegram account wizard ── */}
            {tab === "telegram" && (
              <>
                {tgStep === "api" && (
                  <>
                    <div className="text-[10px] text-jupiter-dim leading-snug">
                      {t("europa.apiHelpBefore")}
                      <code className="text-white">my.telegram.org</code>{t("europa.apiHelpAfter")}
                    </div>
                    <input
                      autoFocus
                      value={apiId}
                      onChange={(e) => setApiId(e.target.value)}
                      placeholder={t("europa.apiIdPh")}
                      className={inputCls}
                    />
                    <input
                      value={apiHash}
                      onChange={(e) => setApiHash(e.target.value)}
                      placeholder="API hash"
                      className={inputCls}
                    />
                    <div className="flex gap-1.5 pt-0.5">
                      <button
                        onClick={handleSaveApi}
                        disabled={busy || !apiId.trim() || !apiHash.trim()}
                        className={primaryBtn}
                      >
                        {t("europa.next")}
                      </button>
                      <button onClick={cancel} className={ghostBtn}>
                        {t("common.cancel")}
                      </button>
                    </div>
                  </>
                )}

                {tgStep === "phone" && (
                  <>
                    <div className="text-[10px] text-jupiter-dim leading-snug">
                      {t("europa.phoneHelp")}
                    </div>
                    <input
                      autoFocus
                      value={phone}
                      onChange={(e) => setPhone(e.target.value)}
                      placeholder="+39..."
                      className={inputCls}
                    />
                    <div className="flex gap-1.5 pt-0.5">
                      <button onClick={handleRequestCode} disabled={busy || !phone.trim()} className={primaryBtn}>
                        {busy ? t("europa.sending") : t("europa.sendCode")}
                      </button>
                      <button onClick={cancel} className={ghostBtn}>
                        {t("common.cancel")}
                      </button>
                    </div>
                  </>
                )}

                {tgStep === "code" && (
                  <>
                    <input
                      autoFocus
                      value={code}
                      onChange={(e) => setCode(e.target.value)}
                      placeholder={t("europa.codePh")}
                      className={inputCls}
                    />
                    <div className="flex gap-1.5 pt-0.5">
                      <button onClick={handleSubmitCode} disabled={busy || !code.trim()} className={primaryBtn}>
                        {t("common.confirm")}
                      </button>
                      <button onClick={cancel} className={ghostBtn}>
                        {t("common.cancel")}
                      </button>
                    </div>
                  </>
                )}

                {tgStep === "2fa" && (
                  <>
                    <input
                      type="password"
                      autoFocus
                      value={twoFa}
                      onChange={(e) => setTwoFa(e.target.value)}
                      placeholder={t("europa.twoFaPh")}
                      className={inputCls}
                    />
                    <div className="flex gap-1.5 pt-0.5">
                      <button onClick={handleSubmit2fa} disabled={busy || !twoFa} className={primaryBtn}>
                        {t("common.confirm")}
                      </button>
                      <button onClick={cancel} className={ghostBtn}>
                        {t("common.cancel")}
                      </button>
                    </div>
                  </>
                )}
              </>
            )}

            {/* ── Telegram bot ── */}
            {tab === "telegram-bot" && (
              <>
                <div className="text-[10px] text-jupiter-dim leading-snug">
                  {t("europa.botHelpBefore")}<code className="text-white">@BotFather</code>{t("europa.botHelpMid")}
                  <code className="text-white">/newbot</code>{t("europa.botHelpAfter")}
                </div>
                <input
                  autoFocus
                  value={botToken}
                  onChange={(e) => setBotToken(e.target.value)}
                  placeholder="123456:ABC-DEF..."
                  className={inputCls}
                />
                <div className="flex gap-1.5 pt-0.5">
                  <button onClick={handleSaveBot} disabled={busy || !botToken.trim()} className={primaryBtn}>
                    {t("europa.saveBot")}
                  </button>
                  <button onClick={cancel} className={ghostBtn}>
                    {t("common.cancel")}
                  </button>
                </div>
              </>
            )}

            {/* ── Slack ── */}
            {tab === "slack" && (
              <>
                <div className="text-[10px] text-jupiter-dim leading-snug">
                  {t("europa.slackHelpBefore")}<code className="text-white">api.slack.com/apps</code>{t("europa.slackHelpAfter")}
                </div>
                <input
                  autoFocus
                  value={slackToken}
                  onChange={(e) => setSlackToken(e.target.value)}
                  placeholder="xoxb-..."
                  className={inputCls}
                />
                <div className="flex gap-1.5 pt-0.5">
                  <button onClick={handleSaveSlack} disabled={busy || !slackToken.trim()} className={primaryBtn}>
                    {t("europa.saveSlack")}
                  </button>
                  <button onClick={cancel} className={ghostBtn}>
                    {t("common.cancel")}
                  </button>
                </div>
              </>
            )}

            {/* ── Teams ── */}
            {tab === "teams" && (
              <>
                {!device ? (
                  <>
                    <div className="text-[10px] text-jupiter-dim leading-snug">
                      {t("europa.teamsHelpBefore")}<code className="text-white">portal.azure.com</code>{t("europa.teamsHelpMid")}
                      <code className="text-white">Chat.ReadWrite</code>{t("europa.teamsHelpAnd")}
                      <code className="text-white">offline_access</code>{t("europa.teamsHelpAfter")}
                    </div>
                    <input
                      autoFocus
                      value={teamsClientId}
                      onChange={(e) => setTeamsClientId(e.target.value)}
                      placeholder="Client ID (Application ID)"
                      className={inputCls}
                    />
                    <input
                      value={teamsTenant}
                      onChange={(e) => setTeamsTenant(e.target.value)}
                      placeholder={t("europa.tenantPh")}
                      className={inputCls}
                    />
                    <div className="flex gap-1.5 pt-0.5">
                      <button onClick={handleTeamsStart} disabled={busy || !teamsClientId.trim()} className={primaryBtn}>
                        {t("europa.startMsLogin")}
                      </button>
                      <button onClick={cancel} className={ghostBtn}>
                        {t("common.cancel")}
                      </button>
                    </div>
                  </>
                ) : (
                  <div className="space-y-1.5">
                    <div className="text-[10px] text-jupiter-dim leading-snug">
                      {t("europa.teamsGotoBefore")}
                      <a
                        href={device.verification_uri}
                        target="_blank"
                        rel="noreferrer"
                        className="text-jupiter-orange underline"
                      >
                        {device.verification_uri}
                      </a>{t("europa.teamsGotoAfter")}
                    </div>
                    <div className="text-center text-jupiter-green font-mono text-[15px] tracking-widest py-1">
                      {device.user_code}
                    </div>
                    <div className="text-[10px] text-jupiter-dim">{t("europa.waiting")}</div>
                    <button onClick={cancel} className={ghostBtn}>
                      {t("common.cancel")}
                    </button>
                  </div>
                )}
              </>
            )}
          </div>
        )}
      </div>

      <div className="text-[10px] text-jupiter-dim/80 mt-2.5 leading-snug">
        {t("europa.footerBefore")}<code className="text-white">MoonEuropa</code>{t("europa.footerMid")}
        <code className="text-white">.mcp.json</code>{t("europa.footerAfter")}
        <code className="text-white">europa</code>{t("europa.footerEnd")}
      </div>
    </div>
  );
}
