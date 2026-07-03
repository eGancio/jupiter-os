// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli

import { invoke } from "@tauri-apps/api/core";
import type { ChatMessage, ChatMsgBackend, ChatSessionInfo, ServiceInfo, LogLines } from "../types";

export type { ServiceInfo, LogLines };

// ── Additional types ──────────────────────────────────────────────────────────

export interface ClaudeMdStatus {
  exists: boolean;
  path: string;
  size_bytes: number;
}

export interface CredentialEntry {
  account: string;
  username: string;
  has_password: boolean;
  has_oauth: boolean;
}

// ── Chat commands ─────────────────────────────────────────────────────────────

export function createChatSession(): Promise<string> {
  return invoke("create_chat_session");
}

export function sendChatMessage(
  sessionId: string,
  message: string,
  images?: string[]
): Promise<void> {
  return invoke("send_chat_message", { sessionId, message, images: images ?? [] });
}

export function stopChat(sessionId: string): Promise<void> {
  return invoke("stop_chat", { sessionId });
}

export function getChatMessages(sessionId: string): Promise<ChatMsgBackend[]> {
  return invoke("get_chat_messages", { sessionId });
}

export function listChatSessions(): Promise<ChatSessionInfo[]> {
  return invoke("list_chat_sessions");
}

export function deleteChatSession(sessionId: string): Promise<void> {
  return invoke("delete_chat_session", { sessionId });
}

export function renameChatSession(sessionId: string, title: string): Promise<void> {
  return invoke("rename_chat_session", { sessionId, title });
}

export function compactChatSession(sessionId: string): Promise<void> {
  return invoke("compact_chat_session", { sessionId });
}

export function getChatModel(): Promise<string> {
  return invoke("get_chat_model");
}

export function setChatModel(model: string): Promise<void> {
  return invoke("set_chat_model", { model });
}

/** Change the model of ONE session (per-tab); also becomes the default for new sessions. */
export function setChatSessionModel(sessionId: string, model: string): Promise<void> {
  return invoke("set_chat_session_model", { sessionId, model });
}

export type ChatPermissionMode = "auto" | "plan" | "bypass";

export function setChatPermissionMode(mode: ChatPermissionMode): Promise<void> {
  return invoke("set_chat_permission_mode", { mode });
}

export function getChatEngine(): Promise<string> {
  return invoke("get_chat_engine");
}

export function setChatEngine(engine: string): Promise<void> {
  return invoke("set_chat_engine", { engine });
}

export interface EmailItem {
  uid: number;
  subject: string;
  sender: string;
  recipient: string;
  date: string;
  body: string;
  folder: string;
  account: string;
}

export function listEmailsIo(limit?: number): Promise<EmailItem[]> {
  return invoke("list_emails_io", { limit });
}

// ── Moon Metis (knowledge base ingestion) ──────────────────────────────────────

export interface MetisIngestResult {
  source_path: string;
  /** "indexed" | "skipped" | "scanned" | "empty" | "error" */
  status: string;
  doc_id: string;
  title: string;
  nature: string;
  pages: number;
  chunks: number;
  message: string;
}

/** Ingest files/folders into Moon Metis (deterministic, no chat/LLM). */
export function metisIngestPaths(
  paths: string[],
  nature?: string,
  recursive = true,
  force = false
): Promise<MetisIngestResult[]> {
  return invoke("metis_ingest_paths", { paths, nature, recursive, force });
}

export interface MetisDoc {
  doc_id: string;
  title: string;
  source_path: string;
  nature: string;
  pages: number;
  chunks: number;
}

/** List documents currently indexed in the Metis knowledge base. */
export function metisListDocuments(): Promise<MetisDoc[]> {
  return invoke("metis_list_documents");
}

/** Remove a document (all its chunks) from the knowledge base by doc_id. */
export function metisDeleteDocument(docId: string): Promise<void> {
  return invoke("metis_delete_document", { docId });
}

/** Open a document with the OS default application. */
export function metisOpenFile(path: string): Promise<void> {
  return invoke("metis_open_file", { path });
}

/** Reveal a document in the OS file manager (or open its folder). */
export function metisRevealFile(path: string): Promise<void> {
  return invoke("metis_reveal_file", { path });
}

export function flushSessionMessages(
  sessionId: string,
  messages: ChatMessage[]
): Promise<void> {
  const payload = messages.map((m) => ({
    id: m.id,
    role: m.role,
    content: m.content,
    thinking: m.thinking ?? "",
    // `children` is a rendering index of refs into the same array — never
    // serialized (it would duplicate entries); nesting is rebuilt on load
    // from parent_tool_id.
    tool_calls: m.toolCalls.map((t) => ({
      name: t.name,
      id: t.id ?? "",
      input: t.input,
      result: t.result ?? null,
      parent_tool_id: t.parentToolUseId ?? null,
    })),
    timestamp: m.timestamp,
    streaming: m.streaming ?? false,
  }));
  return invoke("flush_session_messages", { sessionId, messages: payload });
}

export function addLocalMoon(params: import("../types").AddLocalMoonParams): Promise<void> {
  return invoke("add_local_moon", {
    name: params.name,
    command: params.command,
    args: params.args,
    cwd: params.cwd,
    envVars: params.envVars,
    port: params.port,
  });
}

export function addRemoteMoon(name: string, url: string, moonType: string): Promise<void> {
  return invoke("add_remote_moon", { name, url, moonType });
}

export function removeMoon(name: string): Promise<void> {
  return invoke("remove_moon", { name });
}

export function removeLocalMoon(name: string): Promise<void> {
  return invoke("remove_local_moon", { name });
}

export function getClaudeMdStatus(): Promise<ClaudeMdStatus> {
  return invoke("get_claude_md_status");
}

export function readChartFile(path: string): Promise<string> {
  return invoke("read_chart_file", { path });
}

// ── Service management ────────────────────────────────────────────────────────

export function getServices(): Promise<ServiceInfo[]> {
  return invoke("get_services");
}

export function getVirtualMoons(): Promise<ServiceInfo[]> {
  return invoke("get_virtual_moons");
}

export function startService(name: string): Promise<void> {
  return invoke("start_service", { name });
}

export function stopService(name: string): Promise<void> {
  return invoke("stop_service", { name });
}

export function restartService(name: string): Promise<void> {
  return invoke("restart_service", { name });
}

export function startAll(): Promise<void> {
  return invoke("start_all");
}

export function stopAll(): Promise<void> {
  return invoke("stop_all");
}

export function getLogs(name: string): Promise<LogLines> {
  return invoke("get_logs", { name });
}

export function clearLogs(name: string): Promise<void> {
  return invoke("clear_logs", { name });
}

// ── Autostart ─────────────────────────────────────────────────────────────────

export function isAutostartEnabled(): Promise<boolean> {
  return invoke("is_autostart_enabled");
}

export function setAutostart(enabled: boolean): Promise<void> {
  return invoke("set_autostart", { enabled });
}

// ── Credentials ───────────────────────────────────────────────────────────────

export function credentialsList(): Promise<CredentialEntry[]> {
  return invoke("credentials_list");
}

export function credentialsSet(account: string, password: string): Promise<void> {
  return invoke("credentials_set", { account, password });
}

export interface AddAccountParams {
  account: string;
  email: string;
  password: string;
  preset?: string;
  imapHost?: string;
}

export function credentialsAddAccount(params: AddAccountParams): Promise<void> {
  return invoke("credentials_add_account", {
    account: params.account,
    email: params.email,
    password: params.password,
    preset: params.preset,
    imapHost: params.imapHost,
  });
}

export function oauthConnectGmail(email: string): Promise<string> {
  return invoke("oauth_connect_gmail", { email });
}

export function accountRemove(account: string): Promise<void> {
  return invoke("account_remove", { account });
}

// ── Ads integration (Google Ads / Meta Ads) ─────────────────────────────────
// Tokens are stored in the OS keyring (service "MoonAds"), never on disk in
// plaintext. The status call never returns secret values, only whether each is set.

export interface AdsKeyStatus {
  key: string;
  set: boolean;
  required: boolean;
}

export interface AdsPlatformStatus {
  platform: string;
  configured: boolean;
  keys: AdsKeyStatus[];
}

export function adsCredentialsStatus(): Promise<AdsPlatformStatus[]> {
  return invoke("ads_credentials_status");
}

export function adsCredentialsSet(
  platform: string,
  values: Record<string, string>
): Promise<void> {
  return invoke("ads_credentials_set", { platform, values });
}

export function adsCredentialsDelete(platform: string): Promise<void> {
  return invoke("ads_credentials_delete", { platform });
}

/** Restart the chat sidecar so freshly-saved credentials are injected. */
export function restartSidecar(): Promise<void> {
  return invoke("restart_sidecar");
}

// ── Europa messaging channels (Telegram / Slack / Teams) ────────────────────

export interface EuropaChannel {
  channel: string;
  mode: string;
  configured: boolean;
  authorized: boolean;
  detail: string;
}

export function europaChannelsList(): Promise<EuropaChannel[]> {
  return invoke("europa_channels_list");
}

export function telegramSaveApi(apiId: string, apiHash: string): Promise<void> {
  return invoke("telegram_save_api", { apiId, apiHash });
}

/** Returns "code_sent" or "already_authorized". */
export function telegramRequestCode(phone: string): Promise<string> {
  return invoke("telegram_request_code", { phone });
}

/** Returns "done" or "password_required". */
export function telegramSubmitCode(code: string): Promise<string> {
  return invoke("telegram_submit_code", { code });
}

/** Returns "done". */
export function telegramSubmitPassword(password: string): Promise<string> {
  return invoke("telegram_submit_password", { password });
}

export function telegramSaveBot(token: string): Promise<void> {
  return invoke("telegram_save_bot", { token });
}

export function slackSave(token: string): Promise<void> {
  return invoke("slack_save", { token });
}

export interface DeviceCodeInfo {
  user_code: string;
  verification_uri: string;
  message: string;
}

export function teamsStartDeviceCode(
  clientId: string,
  tenantId: string
): Promise<DeviceCodeInfo> {
  return invoke("teams_start_device_code", { clientId, tenantId });
}

/** Returns "pending" or "done". */
export function teamsPollDeviceCode(): Promise<string> {
  return invoke("teams_poll_device_code");
}

export function europaChannelRemove(channel: string): Promise<void> {
  return invoke("europa_channel_remove", { channel });
}

// ── WhatsApp (personal account, Baileys companion device — QR pairing) ──────

export interface WhatsappStatus {
  /** "idle" | "waiting" | "connected" | "error" */
  status: string;
  /** QR string to render (rotates until scanned). */
  qr: string | null;
  error: string | null;
}

/** Start pairing: spawns the Baileys helper. Poll whatsappPairingStatus for the QR. */
export function whatsappStartPairing(): Promise<void> {
  return invoke("whatsapp_start_pairing");
}

export function whatsappPairingStatus(): Promise<WhatsappStatus> {
  return invoke("whatsapp_pairing_status");
}

export function whatsappCancelPairing(): Promise<void> {
  return invoke("whatsapp_cancel_pairing");
}

// ── Moon Thebe: brand kit ─────────────────────────────────────────────────────

export interface BrandKit {
  name: string;
  primary: string;
  accent: string;
  vat: string;
  address: string;
  contacts: string;
  confidentiality: string;
  hasLogo: boolean;
}

export interface BrandKitInput {
  name: string;
  primary: string;
  accent: string;
  vat: string;
  address: string;
  contacts: string;
  confidentiality: string;
  /** New logo as a data URL ("data:image/png;base64,..."); omit to keep the current one. */
  logoDataUrl?: string | null;
}

export function getBrandKit(brand = "emotion"): Promise<BrandKit> {
  return invoke("get_brand_kit", { brand });
}

export function setBrandKit(brand: string, kit: BrandKitInput): Promise<void> {
  return invoke("set_brand_kit", { brand, kit });
}
