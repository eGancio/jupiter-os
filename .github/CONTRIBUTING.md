# Contributing to JupiterOS

Thanks for your interest in JupiterOS — an open, self-hosted AI operating system where every **Moon** is a capability (MCP server) orbiting a single desktop **Jupiter** GUI.

Whether you want to fix a bug, polish a Moon, build a new one, or improve docs: this guide is the fastest path in.

## Table of contents

- [Code of Conduct](#code-of-conduct)
- [Ways to contribute](#ways-to-contribute)
- [Development setup](#development-setup)
- [Project layout](#project-layout)
- [Building a new Moon](#building-a-new-moon)
- [Coding style](#coding-style)
- [Commit messages](#commit-messages)
- [Pull request process](#pull-request-process)
- [Referral & sponsorship transparency](#referral--sponsorship-transparency)
- [Reporting bugs & security issues](#reporting-bugs--security-issues)
- [License of your contribution](#license-of-your-contribution)

## Code of Conduct

This project follows the [Contributor Covenant 2.1](CODE_OF_CONDUCT.md). By participating you agree to uphold it. Report unacceptable behaviour to **conduct@jupiteros.ai**.

## Ways to contribute

- **Bug reports** — use the issue template, include OS, JupiterOS version, and reproduction steps.
- **Feature requests** — open a discussion first if the scope is unclear; once aligned, an issue.
- **Code** — pick a [`good first issue`](https://github.com/eGancio/jupiter-os/labels/good%20first%20issue) or propose your own. Always link the issue from the PR.
- **Docs** — typos, clarifications, missing pieces in the README or per-Moon docs. These are very welcome PRs.
- **New Moon** — see [Building a new Moon](#building-a-new-moon) below.

## Development setup

### Prerequisites

| Tool | Version | Notes |
|------|---------|-------|
| Rust | nightly (pinned in `moons/rust-toolchain.toml`) | Installed via [rustup](https://rustup.rs) |
| Node.js | >= 18 | For the Tauri GUI |
| Python | >= 3.10 | For the Python Moons (`moons/amalthea`, `moons/callisto`, …) |
| Tauri prerequisites | platform-specific | See [Tauri prerequisites](https://tauri.app/start/prerequisites/) |

### First build

```bash
git clone https://github.com/eGancio/jupiter-os.git
cd jupiter-os

# 1. Build the Rust Moons + shared library
(cd moons && cargo build --release)

# 2. Build the GUI
cd app
npm install
npm run tauri build
```

The desktop binary is in `app/src-tauri/target/release/`.

### Running tests

```bash
# Rust
(cd moons && cargo test --workspace)

# TypeScript / frontend
(cd app && npm run typecheck)
```

CI runs the same commands on `windows-latest`, `macos-latest`, and `ubuntu-latest`. Your PR must keep CI green.

### Running locally without packaging

```bash
cd app
npm run tauri dev
```

This rebuilds on save and gives you devtools.

## Project layout

```
app/                Desktop app: src/ (React + TypeScript), src-tauri/ (Tauri v2, Rust), sidecar/ (engines)
moons/              Rust workspace (Cargo.toml, rust-toolchain.toml, .cargo/) + one folder per Moon
  io/ europa/ ganymede/ metis/ himalia/ elara/        Rust (workspace members)
  amalthea/ callisto/ thebe/ google-ads/ meta-ads/    Python
  shared/                                             Shared Rust library: Qdrant, ONNX embeddings, document store
scripts/            run-macos.sh, run-linux.sh, setup-linux.sh, localops-eval/ (tool-calling eval)
docs/               mcp.json.example, SPONSORS.md, design notes
.github/            CONTRIBUTING.md, SECURITY.md
```

Each Moon is a self-contained MCP server. Adding one does **not** require touching the `app/` core.

## Building a new Moon

A Moon is just an MCP server that the JupiterOS GUI can spawn and talk to. To create one:

1. **Pick a name** from Jupiter's moons (Adrastea, Leda, Carme, Sinope, …). Check the [Moons in the README](../README.md#moons) so you don't reuse one already taken.
2. **Create the crate / package** in `moons/<name>/`: a Rust crate added to `members` in `moons/Cargo.toml`, or a Python package with a `pyproject.toml`.
3. **Implement the MCP server** exposing a small, focused set of tools. Keep tool inputs/outputs typed and documented.
4. **Add a settings entry** the GUI can show in the *Services* panel so users can start/stop your Moon.
5. **Document it**: a per-Moon README with what tools it exposes, what credentials it needs, and a config example.
6. **Write tests**: at minimum, one unit test per tool and one integration test for the end-to-end MCP call.

A reference implementation is `moons/amalthea/` (small, no credentials, deterministic). For something bigger with credentials and a background daemon, see `moons/io/`.

## Coding style

### Rust

- `cargo fmt --all` before every commit. CI enforces this.
- `cargo clippy --all-targets --all-features -- -D warnings`. Treat clippy lints as build errors.
- Prefer `?` and `thiserror` over `unwrap()` in library code. `unwrap()` is fine in tests.
- Public items get a `///` doc comment with at least one sentence.

### TypeScript / React

- Strict TypeScript (`tsc --noEmit` is part of CI).
- Components in `app/src/components/`, hooks in `app/src/hooks/`, IPC wrappers in `app/src/lib/tauri.ts`.
- No `any` unless explicitly justified in a comment.

### Python (Amalthea)

- `ruff check` and `ruff format`.
- Type hints on public functions.

## Commit messages

We use **[Conventional Commits](https://www.conventionalcommits.org/)**:

```
<type>(<scope>): <short summary>

<optional body>

<optional footer>
```

Types: `feat`, `fix`, `docs`, `style`, `refactor`, `test`, `chore`, `ci`, `build`, `perf`.

Examples:

```
feat(moon-io): add OAuth2 backend for Google Workspace
fix(moon-europa): handle empty TELEGRAM_FOLDER as "all chats"
docs(readme): clarify Qdrant first-run download
```

A clean Conventional Commit history powers our generated `CHANGELOG.md`.

## Pull request process

1. **Fork** the repo and create a topic branch off `main` (`git checkout -b feat/my-thing`).
2. **Write tests** for new behaviour, or a regression test for a bug fix.
3. **Run** `cargo fmt && cargo clippy && cargo test` in `moons/` locally before pushing.
4. **Push** and open a PR using the template. Link the issue you're solving.
5. **CI must be green**. If it goes red, fix it; don't ask a maintainer to ignore it.
6. **One reviewer approval** is required to merge. Squash-merge by default — your branch's commit history is rewritten into one Conventional Commit on `main`.

PRs that touch the **chat sidecar** (`app/sidecar/agent.mjs`) or the **agent integration** need extra care — that path is load-bearing for the GUI. Include a screenshot or short clip showing the GUI still works.

## Referral & sponsorship transparency

JupiterOS is funded by disclosed integration referrals and professional services
(see [SPONSORS.md](../docs/SPONSORS.md)). To keep that promise enforceable, one rule is
**non-negotiable**:

> **No referral link enters the codebase without its row in `SPONSORS.md` in the same
> commit, and an inline disclosure shown to the user in the product.**

A PR that adds or changes a referral/affiliate link must, in the same PR:

1. Add (or update) the partner's row in `SPONSORS.md`.
2. **Visibly disclose** the referral at the point of use in the UI.
3. Respect the global opt-out (Settings → Privacy) — a user who opted out must never be
   routed through a referral.
4. Use **partner-side attribution only** (a referral link/code). Client-side tracking or
   telemetry to attribute conversions is never accepted.

PRs that add a referral without all four will not be merged. This isn't bureaucracy —
it's the whole reason users can trust the recommendations.

## Reporting bugs & security issues

- **Bugs** → [open an issue](https://github.com/eGancio/jupiter-os/issues/new/choose) using the bug template.
- **Security vulnerabilities** → see [SECURITY.md](SECURITY.md). **Do not** open public issues for security problems.

## License of your contribution

JupiterOS core is licensed under **AGPL-3.0-or-later**. By submitting a contribution you agree that:

1. Your contribution is licensed under AGPL-3.0-or-later.
2. You have the right to submit it (you wrote it, or you have permission from the rights-holder).
3. You're fine with your name appearing in the Git history and `CHANGELOG.md`.

We don't require a CLA. The DCO (Developer Certificate of Origin) is implied by your commit and PR — keep your commits signed if you can (`git commit -s`).

---

Welcome on board. Questions? [Start a discussion](https://github.com/eGancio/jupiter-os/discussions).
