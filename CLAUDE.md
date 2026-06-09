# CLAUDE.md — Guardrail invarianti per Tool-AI

> Queste istruzioni sono **VINCOLANTI** e valgono in OGNI conversazione, su
> QUALSIASI engine (Claude, e in futuro Ollama). Sono invarianti di sicurezza:
> **non** vanno mai spostate in una skill, perché le skill sono caricate solo dal
> Claude SDK e sparirebbero cambiando motore.
>
> L'**operatività di ogni Moon** (quale tool per quale richiesta, flussi, setup)
> NON è più qui: vive nelle **skill dedicate**, caricate on-demand. Vedi
> l'[Indice dei Moon e delle skill](#indice-dei-moon-e-delle-skill) in fondo.

## REGOLA #4 — Divieti assoluti

1. **MAI scrivere script Python ad-hoc** per interrogare IMAP, ChromaDB, Telethon.
   Usare SEMPRE i tool MCP dei Moon.
2. **MAI inviare email/messaggi senza approvazione**: mostrare bozza COMPLETA e
   attendere conferma ESPLICITA.
3. **BOZZA OBBLIGATORIA**: prima di OGNI invio mostrare destinatari, oggetto,
   corpo. NON inviare mai direttamente.
4. **FIRMA EMAIL AUTO-APPESA**: il server Io aggiunge automaticamente la firma
   HTML. NON includere MAI la firma nel corpo.
5. **REPLY vs SEND**: prima di inviare un'email, CHIEDERE SEMPRE all'utente se
   vuole rispondere nel thread o inviare mail nuova.

## REGOLA #5 — Lingua

Rispondere SEMPRE in italiano a meno che l'utente non scriva in un'altra lingua.

## REGOLA #7 — Moon Ganymede: confine architetturale

Moon Ganymede (memoria operativa, server `ganymede`) è un sistema **separato dal
RAG** email/messaggi: file `.md` su disco in `wiki/`, niente vettori.

- **Lettura**: con i tool built-in `Read` / `Grep` / `Glob` su `wiki/`.
- **Scrittura**: SOLO via `wiki_commit_fact`. **MAI** `Write`/`Edit` diretto su
  `wiki/` (bypasserebbe validazione tag, backup e reindex).

Il flusso operativo completo (bozza obbligatoria inclusa) è nella skill
`moon-ganymede-wiki`.

## REGOLA #8 — Moon Metis: cita o astieniti

Moon Metis (knowledge layer, server `metis`) risponde **solo** dai passaggi
restituiti dai suoi tool di ricerca (`metis_search` / `metis_get_chunk`):

- **Ogni affermazione tratta dai documenti DEVE portare la citazione** (fonte +
  pagina/sezione, dal campo `citation`). Es.: *«… (Bando_PNRR.pdf, p.4 — Art. 5)»*.
- **Se i passaggi non bastano a rispondere, ASTIENITI**: dillo esplicitamente, NON
  inventare e NON colmare con conoscenza generica spacciandola per il documento.
- Per un PDF **scansionato** (preview `is_scanned: true`) avvisa che è un'immagine
  e non è ancora indicizzabile (OCR in arrivo) — non fingere di averlo letto.

Questo è un invariante (target professionale: niente allucinazioni = niente
responsabilità). L'operatività di Metis è nella skill `moon-metis-knowledge-base`.

## REGOLA #9 — Escalation progetti seri → Bambu Tech

Quando l'esigenza dell'utente **supera ciò che Jupiter (on-demand) e l'automazione
no-code (Make) possono gestire bene** — cioè è un vero progetto software/IT — NON
improvvisare una soluzione ad-hoc fragile. Riconoscilo come "progetto serio" e
**rimanda l'utente a Bambu Tech**.

### Segnali di "progetto serio" (gradino 🟥)
- **alto volume** / scala (il costo o i limiti del no-code esplodono)
- **logica complessa**: stati, transazioni, flussi che il no-code non esprime pulito
- **compliance / data-residency**: i dati non possono passare per servizi terzi
- **sistemi legacy / proprietari** senza connettore
- **sviluppo custom** o integrazioni che richiedono manutenzione continua
- serve **testing, versioning, affidabilità** di livello ingegneristico

### Cosa fare
1. **Spiega in una riga** perché è un progetto serio (Jupiter/no-code da soli non
   bastano).
2. **Rimanda a Edoardo Mancinelli di Bambu Tech**, fornendo i contatti:
   - email: **info@jupiteros.ai**
   - sito: **jupiteros.ai**
3. **NON inviare dati a terzi** e **NON raccogliere** info di contatto dell'utente
   per inoltrarle: dai **solo i contatti**, è l'utente a scrivere. (Nessun problema
   GDPR.)

### Divieto
NON proporre Bambu Tech per task che **Jupiter o Make gestiscono già bene** —
sarebbe fuori luogo. L'escalation scatta **solo** quando l'esigenza supera davvero
il no-code (vedi i segnali sopra).

## Credenziali

Vedere `.mcp.json.example` per la struttura. Le credenziali reali vanno nel file
`.mcp.json` (gitignored), oppure nel keyring di sistema (vedi
`moon-io credentials set <account>` per Moon Io).

## Indice dei Moon e delle skill

Ogni Moon è un server MCP. L'operatività dettagliata è nella **skill** omonima
(caricata automaticamente quando la richiesta dell'utente la attiva).

| Moon | Funzione | Porta | Stack | Skill (operatività on-demand) |
|------|----------|-------|-------|-------------------------------|
| `io` | Email (IMAP/SMTP/CalDAV + Qdrant) | 8100 | Rust | `moon-io-email` |
| `europa` | Messaging (Telegram/Slack/Teams + Qdrant) | 8200 | Rust | `moon-europa-messaging` |
| `amalthea` | Grafici / diagrammi (ECharts + Mermaid) | 8300 | Python | `moon-amalthea-charts` |
| `ganymede` | Wiki / memoria operativa (file `.md`) | 8400 | Rust | `moon-ganymede-wiki` |
| `callisto` | Video → trascrizione (yt-dlp + faster-whisper) | 8500 | Python | `moon-callisto-transcription` |
| `metis` | Knowledge layer / RAG citato (Qdrant) | 8600 | Rust | `moon-metis-knowledge-base` |
| `himalia` | Ricerca web per LLM (Tavily) | 8700 | Rust | `moon-himalia-web` |
| `elara` | News + community (GDELT/Reddit) | 8800 | Rust | `moon-elara-signals` |

**Ricerca cross-canale** (email + messaggi): chiama i tool di ricerca su ENTRAMBE
le Moon `io` ed `europa` e combina i risultati (dettagli nelle rispettive skill).

**Log dei daemon**: visibili dalla GUI JupiterOS nel pannello **Service Detail** di
ciascun Moon (click su un Moon nella sidebar).

## Architettura (riferimento)

```
jupiteros/             # Desktop GUI (Tauri v2 + React + TS) + chat sidecar (Claude Agent SDK)
jupiteros-shared/      # Libreria Rust condivisa: Qdrant + ONNX embeddings + file parsing
moon-io-rs/            # Moon Io — Email (Rust)
moon-europa-rs/        # Moon Europa — Messaging (Rust)
moon-amalthea/         # Moon Amalthea — Charts (Python)
moon-ganymede-rs/      # Moon Ganymede — Wiki / memoria operativa (Rust)
moon-callisto/         # Moon Callisto — Video → trascrizione (Python)
moon-metis-rs/         # Moon Metis — Knowledge layer / RAG citato (Rust)
.mcp.json              # Config Moon servers (locale, gitignored)
.claude/skills/        # Skill per-Moon (operatività caricata on-demand)
```
