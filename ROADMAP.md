# JupiterOS — Roadmap

> This document describes **where JupiterOS is going** and the principles that guide it.
> It is intentionally high-level: dates are directions, not promises. Feedback and
> contributions are welcome — see [CONTRIBUTING.md](CONTRIBUTING.md).

JupiterOS is an **open-source environment for AI, optimized for Claude**: a desktop
front-end that gives the model **the right context at the right moment** — your email,
your chats, your documents, your knowledge — through specialised MCP servers
("Moons"), without writing code.

The guiding idea: **conversation is the only interface everyone already knows.**
An AI is only as useful as the context it can reach. Jupiter sits *on top of*
deterministic tooling and feeds the model exactly what the task needs, hiding the
plumbing instead of exposing yet another node-graph GUI.

---

## Current status

**Desktop GUI** — Tauri v2 + React/TS, with an agent chat sidecar, visual MCP
management, in-app account setup (e.g. IMAP email accounts), a transparent tool-call
timeline, and `/compact` to beat the token wall.

**Engine — pick your own:**

- **Anthropic Claude** (recommended) — via the Claude Agent SDK. It is the engine we
  optimize for: it's the only one that loads the **skill layer** (see below) and the
  only one that reliably orchestrates many tools in one conversation.
- **Google Gemini · Groq** — bring your own API key, OpenAI-compatible loop with a
  safety denylist on sensitive tools.
- **Ollama** — fully local inference (Qwen / Llama / Mistral). Your machine, your model.

**Skill layer** (Claude engine): on-demand instructions that make the model an expert
of each Moon, plus document skills (Word / Excel / PowerPoint / PDF creation and
editing) and a **deep-research methodology** — multi-source research over your own
documents *and* the web, every claim verified on the primary source and cited.

**Moons shipped:**

| Moon | Function | Stack | Status |
|------|----------|-------|--------|
| **Io** | Email (IMAP/SMTP, CalDAV, semantic search) | Rust + Qdrant | Stable |
| **Europa** | Messaging (Telegram + multi-channel, semantic search) | Rust + Qdrant | Stable |
| **Amalthea** | Charts & diagrams (16 deterministic types) | Python | Stable |
| **Callisto** | Video → transcription (yt-dlp + faster-whisper) | Python | Experimental |
| **Metis** | Knowledge library — RAG with mandatory citations (source + page) | Rust + Qdrant | Recent |
| **Himalia** | Web search & page extraction for LLMs (Tavily) | Rust | Recent |
| **Elara** | External signal — news (GDELT) + community (Reddit) | Rust | Recent |
| **Thebe** | Brand kit + on-brand document rendering | Python | Recent |

---

## Skills: ours bundled, Anthropic's imported by you

- **Jupiter's own skills** (the per-Moon expertise and the deep-research
  methodology) are part of this project and ship with it, AGPL like everything else.
- The **document skills** (Word / Excel / PowerPoint / PDF) are Anthropic's official
  skills: they are *source-available, not open source*, and their license does not
  allow redistribution. A guided import fetches them **from Anthropic's official
  repository onto your machine** — Jupiter never redistributes them. The import is
  manual today; the upcoming **installer** will make it a one-click step (and handle
  the Python/Node dependencies the skills need).

This keeps the project legally clean by design: nothing in this repo contains
third-party proprietary material.

---

## Roadmap

### 🟢 Now — Adoptable by everyone

The product works daily; the priority is removing every barrier between
"I heard about this" and "it works for me."

In priority order:

1. **Deep-research hardening** *(in progress)* — long parallel runs, verified and
   cited multi-source research, report artifacts (the report can already be
   delivered as Word/PDF via the document skills).
2. **Chat workspace** — concurrent chats (run a long research in one chat while you
   keep working in another) and a smarter chat list: filters by date and keyword.
   Requires a partial layout restructure.
3. **Function-first naming** — moon names are romantic but impractical. The UI moves
   to plain names with proper icons: *Io → Email, Europa → Chats, Metis → Library,
   Himalia → Web, …* (architecture keeps the moon codenames).
4. **OCR for scanned PDFs** (Metis) — professional documents are often scans; the
   knowledge library must read them too.
5. **Frictionless installer** — picks a sensible default engine for your hardware,
   runs the skills import described above, and includes **guided account setup for
   the Moons that need one** (email IMAP, Telegram, web-search key, …).
   Ships together with **approval modes**: today the agent runs in full-auto;
   before wide distribution, chat-level approval prompts for sensitive actions.
6. **Docs, video-first + Moon info** — in-depth walkthroughs on the Jupiter channel
   (the videos *are* the tutorials), plus an information page per Moon: what it does,
   what credentials it needs, what data it touches and where that data stays —
   readable *before* installing, so you can evaluate without trusting blindly.

### 🟡 Next — Editions & ecosystem

- **Vertical editions** — preset Moon bundles for a domain (SMB/freelance, marketing,
  travel, research, dev). Same core, zero duplicated code; the installer offers
  "default or a specialised edition."
- **More Moons** — a secrets *Vault* (bridge to your own Bitwarden/Vaultwarden),
  task/project management, and connectors that broaden everyday coverage.
- **Automation hand-off** — when a task is recurring, multi-app and must run 24/7,
  Jupiter helps you set it up on a deterministic automation platform instead of
  pretending an LLM should be your cron runner. Right tool for the job.

### 🔵 Later — Private Server & scale

- **Jupiter Private Server** — run large open models on hardware **you** control: a
  dedicated, encrypted box (LUKS + WireGuard, no logs) where your laptop is a thin
  client and your data never touches shared infrastructure.
- **Confidential Computing** option (TEE / attestation) for big models on rented GPUs
  without trusting the host.
- **Partner hand-off** — a transparent, opt-in way to route genuinely large projects to
  vetted professionals when no-code and local tooling are no longer the right fit.

---

## Design principles *(these don't change)*

- **Context first.** The product's job is getting the right context to the model at
  the right moment — and never inventing what it doesn't have: Metis answers are
  citation-backed ("cite or abstain").
- **Local-first.** Data stays on your machine by default. Embeddings and vector search
  run locally. Privacy is hygiene, not a marketing pitch — it's simply how this is built.
- **Zero telemetry.** The app does not phone home. Attribution for any referral is done
  at the partner's side, never by tracking you inside the app.
- **Bring your own model & key.** Model-agnostic, no lock-in, run it fully local if you
  want — and we tell you honestly what each engine can and cannot do.
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
