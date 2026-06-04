# JupiterOS

[![License: AGPL v3](https://img.shields.io/badge/License-AGPL_v3-orange.svg)](LICENSE)
[![Built with Tauri](https://img.shields.io/badge/Built%20with-Tauri%20v2-black?logo=tauri)](https://tauri.app)
[![Made with Rust](https://img.shields.io/badge/Made%20with-Rust-black?logo=rust)](https://www.rust-lang.org/)

> **Open-source desktop chat for Claude + MCP servers.**

JupiterOS is a Tauri-based desktop application that gives you a native chat interface for Anthropic Claude with first-class support for MCP (Model Context Protocol) servers — local (stdio) and remote (SSE/HTTP). Manage your servers visually, see every tool call in real time, never hit the token wall.

<!-- TODO: hero GIF (chat + tool call live) -->

## Why JupiterOS

- **Visual MCP management** — see every server in your sidebar, with live status, start/stop, log tail, and credentials. No more hand-editing `.mcp.json`.
- **Transparent tool timeline** — every Claude tool call appears inline with the server it hits, a live status, elapsed time and the actual result. Token usage and cost tracked per turn.
- **No token wall** — type `/compact` and JupiterOS summarises the conversation so far and hands it to a fresh session. Multi-session, slash commands (`/clear`, `/model`, `/cost`, `/mcp`, `/plan`), persistent history.

## Pre-configured Moons

JupiterOS ships with 5 reference MCP servers ("Moons"):

| Moon | Function | Stack | Port |
|------|----------|-------|------|
| **Io** | Email (IMAP/SMTP, CalDAV, semantic search) | Rust + Qdrant | 8100 |
| **Europa** | Messaging (Telegram + multi-channel, semantic search) | Rust + Qdrant | 8200 |
| **Amalthea** | Charts & diagrams (16 deterministic types) | Python + ECharts/Mermaid | 8300 |
| **Ganymede** | Operational wiki / memory (Markdown + YAML) | Rust | 8400 |
| **Callisto** | Video → transcription (yt-dlp + faster-whisper) | Python | 8500 |

Bring your own — any stdio, SSE or HTTP MCP server works out of the box.

## Privacy by design

- **Zero telemetry.** Nothing leaves your machine except calls to your own Anthropic API endpoint.
- **Data stays local.** Embeddings run locally via ONNX. Vector store (Qdrant) runs locally.
- **Bring your own key.** Your Anthropic API key, your billing, your control.

## Installation

### Prerequisites

| Tool | Version | Notes |
|------|---------|-------|
| Rust | nightly (pinned in `rust-toolchain.toml`) | Installed via [rustup](https://rustup.rs) |
| sccache | latest | `cargo install sccache` — used by the workspace config |
| Node.js | >= 18 | For the Tauri GUI |
| Python | >= 3.10 | Only if you want Moon Amalthea |
| Tauri prerequisites | platform-specific | See [Tauri prerequisites](https://tauri.app/start/prerequisites/) |

### Build

> **Platforms.** The Rust/Tauri code is cross-platform (Linux, macOS, Windows). The
> `setup-linux.sh` helper is **Debian/Ubuntu only** (`apt-get`); on Fedora, macOS or
> Windows install the [Tauri prerequisites](https://tauri.app/start/prerequisites/) and
> the tools in the table above by hand, then run the build steps below.

```bash
git clone https://github.com/eGancio/jupiter-os.git
cd jupiter-os

# Rust Moons + shared library
cargo build --release

# GUI
cd jupiteros
npm install
npm run tauri build
```

The desktop binary lands in `jupiteros/src-tauri/target/release/`.

> **Note.** The GUI must be built with `npm run tauri build` — a bare `cargo build
> --release` does not bundle the frontend and produces a non-functional window.

### Authentication (Anthropic API key)

JupiterOS is **bring-your-own-key** — it never ships or proxies a key. Export your
Anthropic API key in the environment that launches the app:

```bash
export ANTHROPIC_API_KEY=sk-ant-...
```

The chat sidecar reads it from the environment at startup; nothing is stored by JupiterOS.

### Configure

1. Copy `.mcp.json.example` to `.mcp.json`
2. Fill in your credentials (email IMAP, Telegram API ID/hash)
3. **Prefer the OS keyring** for the email password: `moon-io credentials set <account>`
4. Launch the JupiterOS binary

### First run

1. Open the app — the sidebar lists the Moons
2. Click **Start** on each Moon you want active
3. On first run, each Rust Moon downloads its runtime dependencies on demand: the Qdrant
   binary (~80 MB, from GitHub releases) and the BGE-M3 ONNX embedding model (~570 MB,
   from Hugging Face). Allow a few minutes and a stable connection for the first start.
4. For Moon Europa: enter the Telegram OTP when prompted

## Architecture

```
┌────────────────────────────────────────────────┐
│            JupiterOS Desktop GUI               │
│            Tauri v2 + React + TS               │
├────────────────────────────────────────────────┤
│       Claude Agent SDK (Node sidecar)          │
└────────┬───────────────────────────────────────┘
         │  MCP protocol  (stdio · SSE · HTTP)
         ▼
┌────────────────┬────────────────┬──────────────┐
│   Moon Io      │  Moon Europa   │ Moon Amalth. │
│   (Email)      │  (Telegram)    │  (Charts)    │
│                │                │              │
│ IMAP/SMTP +    │ Telegram MTP + │ ECharts +    │
│ Qdrant + ONNX  │ Qdrant + ONNX  │ Mermaid      │
└────────────────┴────────────────┴──────────────┘
```

- **GUI**: Tauri v2 (Rust backend) + React (TypeScript frontend)
- **Chat sidecar**: Anthropic [Claude Agent SDK](https://github.com/anthropics/claude-agent-sdk) for streaming + tool orchestration
- **MCP transports**: stdio (local processes), SSE and HTTP (remote)
- **Shared library** (`jupiteros-shared`): Qdrant client, ONNX embeddings, file parsing utilities

## Build your own Moon

A Moon is any MCP server. See [CONTRIBUTING.md](CONTRIBUTING.md#building-a-new-moon) for the step-by-step guide.

Reference implementations to copy from:
- [`moon-amalthea/`](moon-amalthea/) — minimal, no credentials, deterministic
- [`moon-io-rs/`](moon-io-rs/) — production-grade Rust, credentials in keyring, background daemon, semantic search

## Roadmap

| Moon | Function | Status |
|------|----------|--------|
| **Io** | Email | Stable |
| **Europa** | Messaging | Stable |
| **Amalthea** | Charts | Stable |
| Pandora | Filesystem (local + SSH) | Planned |
| Hyperion | System Monitor | Planned |
| Callisto | Browser AI | Planned |
| Titan | Predictions & Analytics | Planned |

## Contributing

PRs welcome. See [CONTRIBUTING.md](CONTRIBUTING.md) for the full guide (style, commits, PR process, how to build a new Moon).

## Security

Report vulnerabilities privately. See [SECURITY.md](SECURITY.md).

## License

[AGPL-3.0-or-later](LICENSE). If you build a commercial product on top of JupiterOS, the AGPL's network-use clause applies.

---

Built in Italy by [Edoardo Mancinelli](https://jupiteros.ai) — Bambu Holding S.r.l.s.
