# JupiterOS

[![License: AGPL v3](https://img.shields.io/badge/License-AGPL_v3-orange.svg)](LICENSE)
[![Built with Tauri](https://img.shields.io/badge/Built%20with-Tauri%20v2-black?logo=tauri)](https://tauri.app)
[![Made with Rust](https://img.shields.io/badge/Made%20with-Rust-black?logo=rust)](https://www.rust-lang.org/)

> **Open-source desktop environment for AI — Claude-first, multi-engine, local-first.**

JupiterOS is a Tauri desktop app that gives an AI model the right context at the right
moment: your email, your chats, your documents and the web, through specialised MCP
servers called **Moons**. It runs the Moons for you, shows every tool call as it happens,
and lets you pick the engine — Anthropic Claude, a cloud API, or a fully local model.

<!-- TODO: hero GIF (chat + tool call live) -->

## Features

**Chat**
- Multiple sessions with persistent history and automatic titles
- Live tool-call timeline: which Moon was called, status, elapsed time, result; token usage and cost per turn
- Paste images straight into the chat (`Ctrl+V`)
- Slash commands: `/clear`, `/compact` (summarise and continue in a fresh session), `/model`, `/cost`, `/mcp`, `/plan`
- Italian and English UI

**Engines** — chosen per session

| Engine | Runs | Notes |
|--------|------|-------|
| **Claude** (Anthropic) | Cloud | Recommended. Uses the Claude Agent SDK: loads your Claude Code skills and settings, handles many tools and subagents in one conversation |
| **Gemini** (Google) | Cloud | Your API key |
| **Groq** | Cloud | Your API key |
| **OpenRouter** | Cloud | DeepSeek V4 models, your API key |
| **Ollama** | Local | Any local model served by Ollama |
| **LocalOps** | Local | `llama.cpp` server started by JupiterOS as a daemon |

**Moon management**
- Sidebar with every Moon: status, start/stop, autostart, live logs
- Setup panels inside the app: email accounts, Telegram and WhatsApp pairing, Google/Meta Ads credentials, brand kit, document ingest for Metis
- Add any MCP server from the GUI — local (stdio) or remote (SSE/HTTP)

**PII shield** (optional, per chat) — with a local PII backend configured as a daemon,
personal data is replaced by placeholders before the message leaves your machine; the
model only sees `[TAG_n]`, you see the real values. Current limit: tool results reach the
model unmasked.

## Moons

| Moon | Function | Stack | Port |
|------|----------|-------|------|
| **Io** | Email: IMAP/SMTP, Gmail API, CalDAV calendar, semantic search | Rust + Qdrant | 8100 |
| **Europa** | Messaging: Telegram, Slack, Teams, WhatsApp, semantic search | Rust + Qdrant | 8200 |
| **Amalthea** | Charts and diagrams (16 deterministic types) | Python | 8300 |
| **Ganymede** | Operational memory: Markdown wiki with tagged facts | Rust | 8400 |
| **Callisto** | Video → clean transcript (yt-dlp + faster-whisper) | Python | 8500 |
| **Metis** | Document library: RAG with mandatory citations (source + page), local OCR for scanned PDFs | Rust + Qdrant | 8600 |
| **Himalia** | Web search and page extraction (Tavily), Italian company reports (openapi.it, with spend cap) | Rust | 8700 |
| **Elara** | External signal: news (GDELT) and communities (Reddit) | Rust | 8800 |
| **Thebe** | Brand kit + Markdown → on-brand PDF reports | Python | 8900 |
| **Google Ads** | Keyword Planner and GAQL reporting (changes disabled by default) | Python | stdio |
| **Meta Ads** | Insights, interest research, ad set creation | Python | stdio |

Every Moon is optional: start only the ones you need. Any other MCP server works too.

## Privacy

- **Zero telemetry.** The app does not phone home.
- **Your data stays on disk.** Email and message indexes, embeddings (BGE-M3 via ONNX),
  the Qdrant vector store, the wiki and chat history are all local.
- **What does leave your machine:** the conversation goes to the engine you pick (nothing
  with Ollama or LocalOps), and Moons that use external services talk to them (Tavily,
  openapi.it, GDELT, Reddit, Google/Meta Ads, video sites).
- **Secrets in the OS keyring** — macOS Keychain, Windows Credential Manager, Linux Secret
  Service — instead of plain-text config.

> [!WARNING]
> With the Claude engine, tools currently run **without confirmation prompts** (auto mode).
> The guardrails in [`CLAUDE.md`](CLAUDE.md) tell the model to always show a draft and wait
> for your OK before sending email or messages, but there is no approval dialog in the app yet.

## Installation

There are no pre-built installers: JupiterOS is built from source. The code is
cross-platform (macOS, Linux, Windows).

### Prerequisites

| Tool | Version | Needed for |
|------|---------|------------|
| Rust | nightly, pinned in `moons/rust-toolchain.toml` | Rust Moons and the app. Install [rustup](https://rustup.rs); inside `moons/` the pinned toolchain is picked up automatically |
| sccache | latest | Required for the Moons: compiler wrapper set in `moons/.cargo/config.toml` (`cargo install sccache`) |
| Node.js | >= 18 | GUI, chat sidecar, WhatsApp helper |
| Python | >= 3.10 | Python Moons (Amalthea, Callisto, Thebe, Google Ads, Meta Ads) |
| Tauri prerequisites | per platform | See [Tauri prerequisites](https://tauri.app/start/prerequisites/) |
| poppler | any | Metis (`pdftotext`, `pdftoppm`) |
| yt-dlp | latest | Callisto |
| WeasyPrint system libraries | — | Thebe, see [WeasyPrint install](https://doc.courtbouillon.org/weasyprint/stable/first_steps.html) |

On Debian/Ubuntu, `scripts/setup-linux.sh` installs the system packages and builds the core for you.

### Build

```bash
git clone https://github.com/eGancio/jupiter-os.git
cd jupiter-os

# Rust Moons + shared library → moons/target/release/
(cd moons && cargo build --release)

# Chat sidecar
(cd app/sidecar && npm install)

# Desktop app → app/src-tauri/target/release/
cd app
npm install
npm run tauri build
```

> **Note.** Build the app with `npm run tauri build`: a plain `cargo build` does not bundle
> the frontend and gives an empty window.

Python Moons, each in its own virtual environment (example for Amalthea):

```bash
cd moons/amalthea
python3 -m venv .venv
.venv/bin/pip install -e .      # Windows: .venv\Scripts\pip install -e .
```

For WhatsApp in Europa: `(cd moons/europa/whatsapp-helper && npm install)`.

## Configuration

### Where JupiterOS reads its config

The app looks for `.mcp.json` in this order and uses that folder as its base directory:

1. next to the executable
2. the current working directory
3. `~/.config/jupiteros/` (macOS, Linux) or `%APPDATA%\jupiteros\` (Windows)

The chat sidecar is looked up in the same folder, as `app/sidecar/agent.mjs` or
`sidecar/agent.mjs`. When the config lives in `~/.config/jupiteros/`, link the sidecar there:

```bash
ln -s "$PWD/app/sidecar" ~/.config/jupiteros/sidecar
```

### `.mcp.json`

Start from the example: `cp docs/mcp.json.example .mcp.json`. It has three sections:

- `mcpServers` — what the chat connects to (URL for SSE/HTTP Moons, command for stdio ones)
- `servers` — Moon processes JupiterOS starts and supervises (command, working dir, env, port)
- `daemons` — helper processes that are not MCP servers (e.g. the PII backend, `llama-server`);
  `"autostart": false` keeps one manual

The example uses Windows paths (`moon-io.exe`, `.venv/Scripts/python.exe`). On macOS and
Linux drop `.exe` and use `.venv/bin/python`. Absolute paths are the safest choice.

### Secrets

Keep secrets out of `.mcp.json`. Any `env` value in `servers` or `daemons` written as
`${VAR}` is resolved at startup from, in order:

1. `credentials.env` in the base directory (`VAR=value`, one per line)
2. the OS keyring, service `MoonEnv`, account `VAR` — on macOS:
   `security add-generic-password -U -s MoonEnv -a VAR -w 'value'`

A placeholder that cannot be resolved is dropped, so the Moon falls back to its own
defaults instead of using the literal `${VAR}` string.

Engine keys go in `credentials.env`: `GEMINI_API_KEY`, `GROQ_API_KEY`, `OPENROUTER_API_KEY`
(optional: `OLLAMA_BASE_URL`, `LOCALOPS_BASE_URL`).

Google Ads and Meta Ads tokens are entered from their panel in the app and stored in the
keyring (service `MoonAds`). Email credentials live in the keyring too — use the panel in
the app, or the CLI:

```bash
moons/target/release/moon-io credentials set <account>   # IMAP password
moons/target/release/moon-io oauth connect gmail         # Gmail via OAuth
```

### Claude authentication

The Claude engine uses the Claude Agent SDK, which reuses your **Claude Code sign-in** on
the same machine. To bill a separate account, set `ANTHROPIC_API_KEY` in the environment
that launches the app (on macOS, `scripts/run-macos.sh` loads it from `~/.jupiteros.env`).

## Run

| Platform | Command |
|----------|---------|
| macOS | `scripts/run-macos.sh` — finds `node` even outside a terminal session, reads config from `~/.config/jupiteros/` |
| Linux | `scripts/run-linux.sh` — reads `.mcp.json` from the repo root (`JUPITEROS_SAFE_GFX=1` if the window stays blank) |
| Windows | `app\src-tauri\target\release\jupiteros.exe` |

On first start:

- Rust Moons download what they need: the Qdrant binary (~80 MB) and the BGE-M3 embedding
  model (~570 MB); Metis also fetches its OCR models. Allow a few minutes.
- Europa asks for the Telegram code, or shows a QR code for WhatsApp.

## Architecture

```
┌──────────────────────────────────────────────────────────┐
│                  JupiterOS desktop app                   │
│   React + TypeScript UI   ·   Rust backend (Tauri v2)    │
│   Moon processes · keyring · PII shield · sessions       │
├──────────────────────────────────────────────────────────┤
│                 Chat sidecar (Node.js)                   │
│   Claude Agent SDK · Gemini · Groq · OpenRouter ·        │
│   Ollama · LocalOps                                      │
└───────────────────────────┬──────────────────────────────┘
                            │  MCP  (SSE · HTTP · stdio)
                            ▼
┌──────────┬──────────┬──────────┬──────────┬──────────────┐
│ Io       │ Europa   │ Metis    │ Himalia  │ Amalthea     │
│ email    │ messages │ docs     │ web      │ Thebe        │
│ Ganymede │ Elara    │ Callisto │ Ads      │ …your own    │
└──────────┴──────────┴──────────┴──────────┴──────────────┘
      │
      ▼
 Qdrant + ONNX embeddings (local)
```

```
jupiter-os/
├── app/        desktop app: src/ (React UI), src-tauri/ (Rust backend), sidecar/ (engines)
├── moons/      Rust workspace (Cargo.toml) with one folder per Moon
│               io, europa, ganymede, metis, himalia, elara (Rust)
│               amalthea, callisto, thebe, google-ads, meta-ads (Python)
│               shared/: Rust library for the Moons (Qdrant, ONNX embeddings, document store)
├── scripts/    run-macos.sh, run-linux.sh, setup-linux.sh, localops-eval/ (local-model eval)
├── docs/       config example, sponsors, design notes
└── .github/    contributing and security guides
```

## Build your own Moon

A Moon is any MCP server. See [CONTRIBUTING.md](.github/CONTRIBUTING.md#building-a-new-moon) for the step-by-step guide.

Reference implementations to copy from:
- [`moons/amalthea/`](moons/amalthea/) — minimal Python, no credentials, deterministic
- [`moons/io/`](moons/io/) — production-grade Rust: keyring credentials, background indexer, semantic search

## Contributing

PRs welcome. See [CONTRIBUTING.md](.github/CONTRIBUTING.md) for style, commits, PR process and how to build a new Moon.

## Security

Report vulnerabilities privately. See [SECURITY.md](.github/SECURITY.md).

## License

[AGPL-3.0-or-later](LICENSE). If you build a commercial product on top of JupiterOS, the AGPL's network-use clause applies.

---

Built in Italy by [Edoardo Mancinelli](https://jupiteros.ai) — Bambu Holding S.r.l.s.
