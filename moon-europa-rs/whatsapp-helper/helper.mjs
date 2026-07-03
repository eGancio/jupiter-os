// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Edoardo Mancinelli
//
// WhatsApp companion helper for Moon Europa, built on Baileys (multi-device).
//
// It links a PERSONAL WhatsApp account as a companion device (same mechanism as
// WhatsApp Web/Desktop) — no Cloud/Business API. It speaks JSON-lines over
// stdio so it can be driven by:
//   - the JupiterOS Tauri backend in `pair` mode (emits the QR string to show
//     the user, persists the session, exits on success);
//   - moon-europa's Rust WhatsApp adapter in `run` mode (streams incoming
//     messages and answers commands: list_contacts / fetch_history / send).
//
// Protocol (one JSON object per line):
//   stdout (helper -> caller):
//     {"type":"qr","qr":"<string to render as QR>"}
//     {"type":"connected"}
//     {"type":"message","chat","id","sender","text","date","fromMe"}
//     {"type":"contacts","reqId","contacts":[{"id","name","type"}]}
//     {"type":"history","reqId","messages":[<message objects>]}
//     {"type":"sent","reqId","id":"<msg id>"}
//     {"type":"error","error":"...","fatal":bool}
//   stdin (caller -> helper, run mode):
//     {"cmd":"list_contacts","reqId"}
//     {"cmd":"fetch_history","reqId","chat","limit"}
//     {"cmd":"send","reqId","to","text"}
//
// Only TEXT is handled for now (read context + reply). Media download is a TODO.

import readline from "node:readline";
import fs from "node:fs";
import path from "node:path";
import makeWASocket, {
  BufferJSON,
  initAuthCreds,
  proto,
  DisconnectReason,
  fetchLatestBaileysVersion,
} from "@whiskeysockets/baileys";
import pino from "pino";

// ── Atomic multi-file auth state ──────────────────────────────────────
// Baileys' built-in `useMultiFileAuthState` writes each file with a bare
// `writeFile` (no tmp+rename). If the process is SIGKILLed mid-write — which the
// Rust adapter used to do on a connect timeout — `creds.json` is left truncated
// to 0 bytes, permanently bricking the companion session. This is a drop-in
// replacement that writes via a temp file + atomic rename, so a kill at any
// instant leaves either the old or the new file, never a torn one.
function useAtomicAuthState(folder) {
  const fixName = (file) => file?.replace(/\//g, "__")?.replace(/:/g, "-");
  const writeData = (data, file) => {
    const target = path.join(folder, fixName(file));
    const tmp = `${target}.tmp`;
    fs.writeFileSync(tmp, JSON.stringify(data, BufferJSON.replacer));
    fs.renameSync(tmp, target); // atomic on POSIX
  };
  const readData = (file) => {
    try {
      const raw = fs.readFileSync(path.join(folder, fixName(file)), "utf8");
      if (!raw.trim()) return null;
      return JSON.parse(raw, BufferJSON.reviver);
    } catch {
      return null;
    }
  };
  const removeData = (file) => {
    try {
      fs.unlinkSync(path.join(folder, fixName(file)));
    } catch {}
  };

  fs.mkdirSync(folder, { recursive: true });
  const creds = readData("creds.json") || initAuthCreds();

  return {
    state: {
      creds,
      keys: {
        get: async (type, ids) => {
          const out = {};
          for (const id of ids) {
            let value = readData(`${type}-${id}.json`);
            if (type === "app-state-sync-key" && value) {
              value = proto.Message.AppStateSyncKeyData.fromObject(value);
            }
            out[id] = value;
          }
          return out;
        },
        set: async (data) => {
          for (const category in data) {
            for (const id in data[category]) {
              const value = data[category][id];
              const file = `${category}-${id}.json`;
              if (value) writeData(value, file);
              else removeData(file);
            }
          }
        },
      },
    },
    saveCreds: () => writeData(creds, "creds.json"),
  };
}

// ── CLI args ──────────────────────────────────────────────────────────
const mode = process.argv[2]; // "pair" | "run"
const sessionIdx = process.argv.indexOf("--session");
const sessionDir = sessionIdx !== -1 ? process.argv[sessionIdx + 1] : "./wa-session";

if (mode !== "pair" && mode !== "run") {
  process.stderr.write("usage: helper.mjs <pair|run> --session <dir>\n");
  process.exit(2);
}

// Baileys logs MUST NOT pollute stdout (we use it for the JSON protocol).
const logger = pino({ level: "silent" });

function send(obj) {
  process.stdout.write(JSON.stringify(obj) + "\n");
}

// ── In-memory state (run mode) ────────────────────────────────────────
// Baileys has no built-in persistent store; we keep a light one for
// list_contacts / fetch_history. It fills from history sync + live events and
// is persisted to disk so the contact book survives restarts (and so
// list_contacts is usable right after a restart, not only after new traffic).
const chatNames = new Map(); // jid -> display name
const history = new Map(); // jid -> [message objects] (recent buffer, capped)
const HISTORY_CAP = 500;
const contactsFile = `${sessionDir}/contacts.json`;

function loadContacts() {
  try {
    const obj = JSON.parse(fs.readFileSync(contactsFile, "utf8"));
    for (const [jid, name] of Object.entries(obj)) chatNames.set(jid, name);
  } catch {
    /* no file yet */
  }
}

let saveTimer = null;
function saveContacts() {
  if (saveTimer) return;
  saveTimer = setTimeout(() => {
    saveTimer = null;
    try {
      fs.mkdirSync(sessionDir, { recursive: true });
      fs.writeFileSync(contactsFile, JSON.stringify(Object.fromEntries(chatNames)));
    } catch {
      /* best effort */
    }
  }, 1000);
}

// Record a chat. Always register the jid (so we can send to it); upgrade to the
// real display name as soon as we learn it. Skip groups/status for the contact book.
function rememberName(jid, name) {
  if (!jid || jid === "status@broadcast") return;
  const cur = chatNames.get(jid);
  if (name && name !== cur) {
    chatNames.set(jid, name);
    saveContacts();
  } else if (!cur) {
    chatNames.set(jid, jid);
    saveContacts();
  }
}

function pushHistory(jid, msg) {
  if (!jid) return;
  const arr = history.get(jid) || [];
  arr.push(msg);
  if (arr.length > HISTORY_CAP) arr.splice(0, arr.length - HISTORY_CAP);
  history.set(jid, arr);
}

// Extract plain text from a Baileys message node.
function extractText(m) {
  const msg = m.message;
  if (!msg) return "";
  return (
    msg.conversation ||
    msg.extendedTextMessage?.text ||
    msg.imageMessage?.caption ||
    msg.videoMessage?.caption ||
    msg.documentMessage?.caption ||
    ""
  );
}

function jidType(jid) {
  if (!jid) return "user";
  if (jid.endsWith("@g.us")) return "group";
  if (jid.endsWith("@broadcast")) return "channel";
  return "user";
}

// Normalize a Baileys message into our flat shape.
function toMessage(m) {
  const chat = m.key?.remoteJid || "";
  const fromMe = !!m.key?.fromMe;
  const sender = fromMe ? "me" : m.key?.participant || m.pushName || chat;
  const tsRaw = m.messageTimestamp;
  const ts = typeof tsRaw === "number" ? tsRaw : Number(tsRaw?.low ?? tsRaw ?? 0);
  const date = ts ? new Date(ts * 1000).toISOString() : new Date().toISOString();
  rememberName(chat, m.pushName);
  return {
    chat,
    id: m.key?.id || "",
    sender,
    text: extractText(m),
    date,
    fromMe,
  };
}

let sock = null;
let stopping = false;
let saveCredsRef = null;

async function start() {
  const { state, saveCreds } = useAtomicAuthState(sessionDir);
  saveCredsRef = saveCreds;
  const { version } = await fetchLatestBaileysVersion().catch(() => ({ version: undefined }));

  sock = makeWASocket({
    version,
    auth: state,
    logger,
    printQRInTerminal: false,
    markOnlineOnConnect: false, // stay invisible; we are a passive companion
    syncFullHistory: true, // pull the backlog WhatsApp will sync to a companion
  });

  sock.ev.on("creds.update", saveCreds);

  sock.ev.on("connection.update", (u) => {
    const { connection, qr, lastDisconnect } = u;

    if (qr) {
      // Baileys rotates the QR every ~20s until scanned; emit each one.
      send({ type: "qr", qr });
    }

    if (connection === "open") {
      if (mode === "pair") {
        // Wait until the LINKED identity is actually on disk before exiting, so
        // run mode finds a usable session (not a half-written one). The right
        // completion signal for a QR link is a provisioned identity — `me.id` +
        // `account` — NOT `creds.registered`: Baileys only sets `registered` for
        // the pairing-CODE flow, so gating on it here hangs forever on QR.
        // (Atomic saveCreds already guarantees creds.json is never torn.)
        const linked = (c) => !!(c?.me?.id && c?.account);
        const finish = async () => {
          try {
            await saveCredsRef?.();
          } catch {}
          send({ type: "connected" });
          setTimeout(() => process.exit(0), 1200);
        };
        if (linked(sock?.authState?.creds)) {
          finish();
        } else {
          const waitReg = setInterval(() => {
            if (linked(sock?.authState?.creds)) {
              clearInterval(waitReg);
              finish();
            }
          }, 300);
        }
      } else {
        send({ type: "connected" });
      }
    }

    if (connection === "close") {
      const code = lastDisconnect?.error?.output?.statusCode;
      if (code === DisconnectReason.loggedOut) {
        send({ type: "error", error: "logged_out", fatal: true });
        process.exit(1);
      }
      if (stopping) return;
      // Any other close reconnects. CRUCIAL for pairing: right after the QR is
      // scanned, WhatsApp closes the socket with 515 (restartRequired) and the
      // login only completes on the RECONNECT (which then fires connection:open).
      // This applies to both pair and run modes (transient drops reconnect too).
      setTimeout(
        () => start().catch((e) => send({ type: "error", error: String(e) })),
        1500,
      );
    }
  });

  if (mode === "run") {
    // Backlog WhatsApp pushes on link/connection (a companion gets a bounded
    // window of history, not the full archive). CRUCIAL: emit each message on the
    // SAME channel as live messages so it reaches Rust/Qdrant. Storing it only in
    // the in-memory map (as before) meant the daemon's bulk_index had to read it
    // at exactly the right moment with the chat already known — so the history
    // never got indexed and only the first *live* message showed up.
    sock.ev.on("messaging-history.set", ({ chats, contacts, messages }) => {
      // Contact display names arrive HERE in bulk on sync (addressbook `name`,
      // their self-set `notify`, or business `verifiedName`) — for 1:1 chats the
      // `chats` array has no name, so without this the contact book is just jids.
      for (const c of contacts || []) {
        rememberName(c.id, c.name || c.notify || c.verifiedName);
      }
      for (const c of chats || []) rememberName(c.id, c.name || c.subject);
      for (const m of messages || []) {
        const norm = toMessage(m);
        if (!norm.chat || norm.chat === "status@broadcast") continue;
        pushHistory(norm.chat, norm);
        if (norm.text) send({ type: "message", ...norm });
      }
    });

    sock.ev.on("chats.upsert", (chats) => {
      for (const c of chats || []) rememberName(c.id, c.name || c.subject);
    });
    const onContacts = (contacts) => {
      for (const c of contacts || []) {
        rememberName(c.id, c.name || c.notify || c.verifiedName);
      }
    };
    sock.ev.on("contacts.upsert", onContacts);
    sock.ev.on("contacts.update", onContacts); // names that arrive/refresh later

    sock.ev.on("messages.upsert", ({ messages, type }) => {
      if (type !== "notify" && type !== "append") return;
      for (const m of messages || []) {
        const jid = m.key?.remoteJid;
        if (!jid || jid === "status@broadcast") continue;
        const norm = toMessage(m);
        pushHistory(jid, norm);
        if (norm.text) send({ type: "message", ...norm });
      }
    });
  }
}

// ── Command loop (run mode) ───────────────────────────────────────────
// Resolve a recipient to a WhatsApp jid. Accepts a raw jid, a phone number, or
// a contact display name (reverse-looked-up from the chats we know).
function jidForTarget(to) {
  if (!to) return "";
  if (to.includes("@")) return to;
  const digits = to.replace(/[^0-9]/g, "");
  if (digits.length >= 6) return `${digits}@s.whatsapp.net`;
  // Maybe a contact name → reverse lookup (case-insensitive).
  const low = to.trim().toLowerCase();
  for (const [jid, name] of chatNames) {
    if (name && name.toLowerCase() === low) return jid;
  }
  return ""; // unresolved
}

// Reject a hung promise after `ms` so a stuck send can't time out the caller.
function withTimeout(promise, ms, msg) {
  return Promise.race([
    promise,
    new Promise((_, reject) => setTimeout(() => reject(new Error(msg)), ms)),
  ]);
}

async function handleCommand(cmd) {
  try {
    switch (cmd.cmd) {
      case "list_contacts": {
        const contacts = [...chatNames.entries()].map(([id, name]) => ({
          id,
          name: name || id,
          type: jidType(id),
        }));
        send({ type: "contacts", reqId: cmd.reqId, contacts });
        break;
      }
      case "fetch_history": {
        const jid = jidForTarget(cmd.chat);
        const all = history.get(jid) || history.get(cmd.chat) || [];
        const limit = cmd.limit && cmd.limit > 0 ? cmd.limit : all.length;
        send({ type: "history", reqId: cmd.reqId, messages: all.slice(-limit) });
        break;
      }
      case "send": {
        if (!sock) throw new Error("socket WhatsApp non connesso");
        const jid = jidForTarget(cmd.to);
        if (!jid || !jid.includes("@")) {
          throw new Error(`destinatario non risolto: "${cmd.to}" (usa il numero con prefisso o il jid)`);
        }
        const res = await withTimeout(
          sock.sendMessage(jid, { text: cmd.text || "" }),
          15000,
          "invio in timeout dal lato WhatsApp (jid valido? sessione attiva?)",
        );
        send({ type: "sent", reqId: cmd.reqId, id: res?.key?.id || "" });
        break;
      }
      default:
        send({ type: "error", reqId: cmd.reqId, error: `unknown cmd: ${cmd.cmd}` });
    }
  } catch (e) {
    send({ type: "error", reqId: cmd.reqId, error: String(e?.message || e) });
  }
}

if (mode === "run") {
  loadContacts();
  const rl = readline.createInterface({ input: process.stdin });
  rl.on("line", (line) => {
    const trimmed = line.trim();
    if (!trimmed) return;
    let cmd;
    try {
      cmd = JSON.parse(trimmed);
    } catch {
      return;
    }
    handleCommand(cmd);
  });
  // Caller closed our stdin → graceful shutdown. The Rust adapter closes stdin
  // (instead of SIGKILL) precisely so we can end the socket cleanly here and let
  // any in-flight creds write finish, rather than being truncated mid-write.
  rl.on("close", () => {
    stopping = true;
    try {
      sock?.end?.();
    } catch {}
    process.exit(0);
  });
}

// Pairing always starts FRESH: wipe any stale/partial creds from a previous
// attempt so Baileys shows a new QR instead of trying to resume a half-linked
// session (which would never emit a QR). Done ONCE here, not inside start(),
// so reconnects after a QR scan keep the creds just saved.
if (mode === "pair") {
  try {
    fs.rmSync(sessionDir, { recursive: true, force: true });
  } catch {}

  // Pairing safety timeout: if nothing happens, don't hang forever.
  setTimeout(() => {
    send({ type: "error", error: "pairing_timeout", fatal: true });
    process.exit(1);
  }, 180000); // 3 min
}

process.on("SIGTERM", () => {
  stopping = true;
  try {
    sock?.end?.();
  } catch {}
  process.exit(0);
});

start().catch((e) => {
  send({ type: "error", error: String(e?.message || e), fatal: true });
  process.exit(1);
});
