# Security Policy

JupiterOS handles email, messages, and credentials on the user's machine. We take vulnerability reports seriously.

## Supported versions

Security fixes are issued for the latest minor release on the `main` branch. Older versions are not patched.

| Version | Supported |
|---------|-----------|
| `main` / latest release | ✅ |
| Anything older | ❌ |

## Reporting a vulnerability

**Do not** open a public GitHub issue for security problems.

Report privately via one of:

1. **GitHub Security Advisories** — preferred. Go to the [Security tab](../../security/advisories/new) and open a draft advisory. We get a notification immediately.
2. **Email** — `security@jupiteros.ai`. PGP key on request.

Please include:

- Affected component (which Moon, the GUI, the shared library, …)
- JupiterOS version (`jupiteros --version` or commit SHA)
- Operating system and version
- Steps to reproduce, minimum proof-of-concept if you have one
- Impact assessment (data exposure, RCE, privilege escalation, …)
- Whether the issue is already public anywhere

## What to expect

| Stage | Target time |
|-------|-------------|
| Acknowledgement of report | within 72 hours |
| Initial triage & severity | within 7 days |
| Fix & coordinated disclosure plan | within 30 days for high/critical |
| Public advisory & patched release | after fix is shipped |

We follow **coordinated disclosure**: please give us a reasonable window to ship a fix before going public. We credit reporters in the advisory unless they prefer to stay anonymous.

## Scope

In scope:

- Authentication / credential handling (IMAP, SMTP, OAuth2, Telegram, keyring)
- Email / message indexing and storage (Qdrant, local DB files)
- MCP server input validation (every tool in every Moon)
- Tauri IPC commands and the chat sidecar
- Privilege escalation, RCE, data leaks across accounts

Out of scope:

- Bugs in upstream dependencies — please report them upstream (we'll bump versions when patched)
- Security of services you self-host alongside JupiterOS (Qdrant, your IMAP provider, etc.)
- Issues that require physical access to an unlocked machine
- DoS by overwhelming the local machine's resources

## Hardening notes for users

- Keep credentials in the OS keyring. **Never** commit `.mcp.json` with real passwords — use `.mcp.json.example` as a template.
- Qdrant runs on `127.0.0.1`. If you expose it to a network, you are responsible for authentication.
- The Anthropic API key used by the chat sidecar lives in the OS keyring and is sent only to `api.anthropic.com`.
- Email and Telegram data stay local. JupiterOS performs **no telemetry** and has **no opt-out toggle to disable**, because there is nothing to disable.

Thanks for helping keep JupiterOS and its users safe.
