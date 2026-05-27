# CLAUDE.md — Regole obbligatorie per Tool-AI

> Queste istruzioni sono VINCOLANTI. Ogni nuova conversazione Claude Code DEVE seguirle.

## REGOLA #1 — Moon Io (server `io`) — Email

**Moon Io gestisce TUTTO ciò che riguarda email**: lettura, invio, allegati, calendario, firme, ricerca email.

### Tool di DEFAULT: `list_emails` (solo header)

Per panoramiche generiche usare SEMPRE `list_emails` che restituisce SOLO header. Leggero e veloce.
Usare `read_recent_emails` SOLO quando l'utente chiede esplicitamente di LEGGERE IL CONTENUTO. Max 5.

### Tabella: quale tool per quale richiesta

| Richiesta utente | Tool OBBLIGATORIO (server `io`) |
|---|---|
| **LETTURA EMAIL** | |
| "le ultime email" / "mostrami le email" | `list_emails(limit=20)` — SOLO HEADER |
| "ultime N email" / "panoramica email" | `list_emails(limit=N)` — SOLO HEADER, max 200. Usa offset per paginazione |
| "le mie email inviate" | `list_emails(contact="yourname")` |
| "lista email da/a [persona]" | `list_emails(contact="persona")` |
| "leggimi l'email di [persona]" | `read_recent_emails(contact="persona", limit=3)` — max 5 |
| "cosa dice l'email su [topic]?" | `read_recent_emails(limit=3)` — max 5 |
| **RICERCA EMAIL** | |
| "cerca email su [argomento]" | `search_emails(query="argomento")` |
| "email con [persona]" | `search_by_contact(contact="persona")` |
| **AZIONI EMAIL** | |
| "invia email a..." | `send_email(to, subject, body)` — BOZZA OBBLIGATORIA |
| "rispondi all'email" | `reply_email(email_id, body)` — BOZZA OBBLIGATORIA |
| "scarica allegato email" / "salva PDF" | `save_attachment(email_id, attachment_index)` |
| "lista allegati dell'email" | `get_email_attachments(email_id)` |
| "leggi questo PDF/DOCX" | `extract_file_text(file_path="path")` |
| **CALENDARIO** | |
| "eventi calendario" | `list_calendar_events()` |
| "crea evento" | `create_calendar_event(summary, start, end)` |
| **FIRME** | |
| "imposta firma" | `set_email_signature(...)` |
| "mostra firma" | `get_email_signature()` |
| **STATISTICHE** | |
| "quante email ho?" | `get_stats()` |

### Differenza tra i tool principali
- **`list_emails`** — SOLO header email. USARE PER DEFAULT. Max 200. Supporta `offset` per paginazione.
- **`read_recent_emails`** — corpo email (troncato). SOLO lettura esplicita. Max 5.
- **`search_emails`** — ricerca SEMANTICA nelle email
- **`search_by_contact`** — tutte le email con una persona
- **`send_email`** / **`reply_email`** — invio email. MAI inviare senza bozza e approvazione!

## REGOLA #2 — Moon Europa (server `europa`) — Messaging

**Moon Europa gestisce TUTTO ciò che riguarda messaggistica**: Telegram (e futuro WhatsApp, Discord).

### Tool di DEFAULT: `list_messages` (solo header)

Per panoramiche generiche usare SEMPRE `list_messages` che restituisce SOLO header.
Usare `read_messages` SOLO per leggere il contenuto su richiesta esplicita. Max 5.

### Tabella: quale tool per quale richiesta

| Richiesta utente | Tool OBBLIGATORIO (server `europa`) |
|---|---|
| **LETTURA MESSAGGI** | |
| "ultimi messaggi Telegram" | `list_messages(channel="telegram", limit=20)` — SOLO HEADER |
| "messaggi di [persona]" | `list_messages(channel="telegram", contact="persona")` |
| "leggimi i messaggi di [persona]" | `read_messages(channel="telegram", contact="persona", limit=3)` — max 5 |
| **RICERCA MESSAGGI** | |
| "cerca messaggi su [argomento]" | `search_messages(query="argomento")` |
| "messaggi con [persona]" | `search_by_contact(contact="persona")` |
| **AZIONI** | |
| "invia messaggio Telegram a..." | `send_message(channel="telegram", to="nome", message="testo")` — MAI senza approvazione! |
| "invia file su Telegram a..." | `send_message(channel="telegram", to="nome", message="caption", attachments="path")` |
| "contatti Telegram" / "a chi posso scrivere?" | `list_contacts(channel="telegram")` |
| "scarica file/foto da Telegram" | `save_media(channel="telegram", chat="nome", last_n=1)` |
| "leggi questo PDF/DOCX" | `extract_file_text(file_path="path")` |
| **STATISTICHE** | |
| "quanti messaggi ho?" | `get_stats()` |

### Differenza tra i tool principali
- **`list_messages`** — SOLO header messaggi. USARE PER DEFAULT. Max 200. Supporta `offset`.
- **`read_messages`** — corpo messaggi (troncato). SOLO lettura esplicita. Max 5.
- **`search_messages`** — ricerca SEMANTICA nei messaggi
- **`search_by_contact`** — tutti i messaggi con una persona
- **`send_message`** — invia messaggio. MAI inviare senza approvazione!

## REGOLA #3 — Ricerca cross-canale

Per cercare su TUTTI i canali (email + messaggi), chiamare i tool di ricerca di ENTRAMBE le Moon:
1. `search_emails(query="topic")` su **io**
2. `search_messages(query="topic")` su **europa**

Idem per `search_by_contact`: chiamare su entrambe le Moon e combinare i risultati.

## REGOLA #4 — Divieti assoluti

1. **MAI scrivere script Python ad-hoc** per interrogare IMAP, ChromaDB, Telethon. Usare SEMPRE i tool MCP.
2. **MAI inviare email/messaggi senza approvazione**: mostrare bozza COMPLETA e attendere conferma ESPLICITA.
3. **BOZZA OBBLIGATORIA**: prima di OGNI invio mostrare destinatari, oggetto, corpo. NON inviare mai direttamente.
4. **FIRMA EMAIL AUTO-APPESA**: il server Io aggiunge automaticamente la firma HTML. NON includere MAI la firma nel corpo.
5. **REPLY vs SEND**: prima di inviare un'email, CHIEDERE SEMPRE all'utente se vuole rispondere nel thread o inviare mail nuova.

## REGOLA #5 — Lingua

Rispondere SEMPRE in italiano a meno che l'utente non scriva in un'altra lingua.

## REGOLA #6 — Log dei daemon

Visibili dalla GUI JupiterOS nel pannello **Service Detail** di ciascun Moon (click su un Moon nella sidebar).

## Architettura

```
jupiteros/             # JupiterOS desktop GUI (Tauri v2 + React + TS)
│                        chat sidecar Claude Agent SDK in sidecar/agent.mjs
│
jupiteros-shared/      # Libreria condivisa Rust:
│                        Qdrant client + ONNX embeddings + file parsing
│
moon-io-rs/            # Moon Io — Email (Rust)
│                        IMAP/SMTP/CalDAV + Qdrant vector search
│                        Multi-account (Aruba, Gmail via OAuth)
│                        Credenziali nel keyring di sistema
│
moon-europa-rs/        # Moon Europa — Messaging (Rust)
│                        Telegram via MTProto nativo (grammers)
│                        Qdrant vector search
│
moon-amalthea/         # Moon Amalthea — Charts (Python)
│                        16 tipi di grafici/diagrammi deterministici
│                        ECharts + Mermaid
│
.mcp.json              # Config Moon servers (locale, gitignored)
.mcp.json.example      # Template generico per nuovi contributor
```

## Server MCP (Moons)

| Moon | Funzione | Porta | Stack |
|------|----------|-------|-------|
| `io` | Email (IMAP/SMTP/CalDAV + Qdrant) | 8100 | Rust |
| `europa` | Messaging (Telegram + Qdrant) | 8200 | Rust |
| `amalthea` | Chart/visualization | 8300 | Python |

## Credenziali

Vedere `.mcp.json.example` per la struttura. Le credenziali reali vanno nel file `.mcp.json` (gitignored), oppure nel keyring di sistema (vedi `moon-io credentials set <account>` per Moon Io).