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

JupiterOS ships with 3 reference MCP servers ("Moons"):

| Moon | Function | Stack | Port |
|------|----------|-------|------|
| **Io** | Email (IMAP/SMTP, CalDAV, semantic search) | Rust + Qdrant | 8100 |
| **Europa** | Messaging (Telegram, semantic search) | Rust + Qdrant | 8200 |
| **Amalthea** | Charts & diagrams (16 deterministic types) | Python + ECharts/Mermaid | 8300 |

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

```bash
git clone https://github.com/jupiter-os/jupiteros.git
cd jupiteros

# Rust Moons + shared library
cargo build --release

# GUI
cd jupiteros
npm install
npm run tauri build
```

The desktop binary lands in `jupiteros/src-tauri/target/release/`.

### Configure

1. Copy `.mcp.json.example` to `.mcp.json`
2. Fill in your credentials (email IMAP, Telegram API ID/hash)
3. **Prefer the OS keyring** for the email password: `moon-io credentials set <account>`
4. Launch the JupiterOS binary

### First run

1. Open the app — the sidebar lists the Moons
2. Click **Start** on each Moon you want active
3. On first run, Qdrant and the ONNX model auto-download (~170 MB total)
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
