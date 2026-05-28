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

export function flushSessionMessages(
  sessionId: string,
  messages: ChatMessage[]
): Promise<void> {
  const payload = messages.map((m) => ({
    id: m.id,
    role: m.role,
    content: m.content,
    thinking: m.thinking ?? "",
    tool_calls: m.toolCalls.map((t) => ({
      name: t.name,
      input: t.input,
      result: t.result ?? null,
    })),
    timestamp: m.timestamp,
    streaming: m.streaming ?? false,
  }));
  return invoke("flush_session_messages", { sessionId, messages: payload });
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
