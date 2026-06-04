# JupiterOS — Roadmap

> This document describes **where JupiterOS is going** and the principles that guide it.
> It is intentionally high-level: dates are directions, not promises. Feedback and
> contributions are welcome — see [CONTRIBUTING.md](CONTRIBUTING.md).

JupiterOS is an **open-source, local-first desktop assistant**: a natural-language
front-end over MCP "Moons" (specialised servers) that simplifies everyday operations —
email, messaging, documents, charts, knowledge — without writing code and without
sending your data to anyone.

The guiding idea: **conversation is the only interface everyone already knows.**
Jupiter sits *on top of* deterministic tooling, hiding its complexity rather than
exposing yet another node-graph GUI.

---

## Current status

**Desktop GUI** — Tauri v2 + React/TS, with a Claude-driven chat sidecar, visual MCP
management, a transparent tool-call timeline, and `/compact` to beat the token wall.

**Moons shipped:**

| Moon | Function | Stack | Status |
|------|----------|-------|--------|
| **Io** | Email (IMAP/SMTP, CalDAV, semantic search) | Rust + Qdrant | Stable |
| **Europa** | Messaging (Telegram + multi-channel, semantic search) | Rust + Qdrant | Stable |
| **Amalthea** | Charts & diagrams (16 deterministic types) | Python | Stable |
| **Ganymede** | Operational wiki / memory (Markdown + YAML) | Rust | Recent |
| **Callisto** | Video → transcription (yt-dlp + faster-whisper) | Python | Experimental |

**Engine:** Anthropic Claude via the Claude Agent SDK (bring-your-own-key).

---

## Roadmap

### 🟢 Now — Foundations *(make Jupiter adoptable by everyone)*

The priority is removing every barrier between "I heard about this" and "it works for me."

- **Pluggable model engine** — abstract the agent loop behind a provider interface
  (Anthropic · OpenAI-compatible · Google Gemini · local). The Moons are MCP, so they
  are already model-agnostic; the engine should be too. **No lock-in.**
- **Local-first inference** — run fully offline via [Ollama](https://ollama.com)
  (Llama / Qwen / Mistral). Your machine, your model, nothing leaves.
- **Frictionless onboarding** — an installer that picks a sensible default model for
  your hardware, sets it up in one step, and offers a clear "change model" path
  (local · free tier · your own key).
- **Docs & tutorials** — task-first guides: real workflows, not menu tours.

### 🟡 Next — Editions & ecosystem

- **Vertical editions** — preset Moon bundles for a domain (SMB/freelance, marketing,
  travel, research, dev). Same core, zero duplicated code; the installer offers
  "default or a specialised edition."
- **More Moons** — a secrets *Vault* (bridge to your own Bitwarden/Vaultwarden), and
  connectors that broaden everyday coverage.
- **Automation hand-off** — when a task is recurring, multi-app and must run 24/7,
  Jupiter helps you set it up on a deterministic automation platform instead of
  pretending an LLM should be your cron runner. Right tool for the job.

### 🔵 Later — Private Server & scale

- **Jupiter Private Server** — run large open models on hardware **you** control: a
  dedicated, encrypted box (LUKS + WireGuard, no logs) where your laptop is a thin
  client and your data never touches shared infrastructure. Power *and* privacy.
- **Confidential Computing** option (TEE / attestation) for big models on rented GPUs
  without trusting the host.
- **Partner hand-off** — a transparent, opt-in way to route genuinely large projects to
  vetted professionals when no-code and local tooling are no longer the right fit.

---

## Design principles *(these don't change)*

- **Local-first.** Data stays on your machine by default. Embeddings and vector search
  run locally.
- **Zero telemetry.** The app does not phone home. Attribution for any referral is done
  at the partner's side, never by tracking you inside the app.
- **Bring your own model & key.** Model-agnostic, no lock-in, run it fully local if you
  want.
- **Transparent by design.** The code is auditable. Any affiliate/referral link is
  disclosed inline, can be turned off, and is listed publicly (see `SPONSORS.md`).
  Recommendations are never biased toward partners — read the source and check.
- **You're always in control.** Anything Jupiter can do for you, you can do yourself via
  plain MCP. No walled garden.

---

## Sustainability

JupiterOS is **free and open** (AGPL-3.0) and intends to stay that way. Development is
sustained through **optional, clearly-disclosed integration referrals** and
**professional services** — never through telemetry, selling your data, or paywalling
core features. The goal is a project that stays independent: no VC pressure, no rug-pull.

---

*Have an idea or a need we're not covering? Open an issue — the roadmap is shaped by
real use.*
