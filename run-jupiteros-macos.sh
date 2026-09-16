#!/usr/bin/env bash
# Avvio JupiterOS su macOS — equivalente di run-jupiteros.sh (Linux).
#
# Differenze rispetto a Linux:
#  1) node non sta nei percorsi di sistema e un'app macOS non eredita il PATH
#     della shell: il sidecar è lanciato con Command::new("node"), quindi il
#     PATH va costruito qui.
#  2) niente workaround WebKitGTK/DMABUF: su macOS il webview è WKWebView.
#
# Autenticazione: NON serve ANTHROPIC_API_KEY. Il Claude Agent SDK usa le
# credenziali di Claude Code dal Portachiavi macOS. Se però esporti la chiave
# (in ~/.jupiteros.env) l'SDK la preferisce — utile per separare la fatturazione.
set -e
cd "$(dirname "$0")"

REL="./jupiteros/src-tauri/target/release"
BIN="$REL/jupiteros"

if ! command -v node >/dev/null 2>&1; then
  for d in "$HOME"/.local/tools/node-*/bin /opt/homebrew/bin /usr/local/bin; do
    [ -x "$d/node" ] && export PATH="$d:$PATH" && break
  done
fi

# Segreti opzionali (GEMINI/GROQ/OPENROUTER stanno già in credentials.env
# accanto al binario, letto dall'app stessa).
[ -f "$HOME/.jupiteros.env" ] && { set -a; . "$HOME/.jupiteros.env"; set +a; }

if ! command -v node >/dev/null 2>&1; then
  echo "ERRORE: node non trovato — il sidecar chat non partirebbe." >&2; exit 1
fi
if [ ! -x "$BIN" ]; then
  echo "ERRORE: binario assente: $BIN" >&2
  echo "        Ricompila con: (cd jupiteros && npm run tauri build)" >&2; exit 1
fi

pkill -x jupiteros 2>/dev/null || true
sleep 1

AUTH="Claude Code (Portachiavi)"
[ -n "${ANTHROPIC_API_KEY:-}" ] && AUTH="ANTHROPIC_API_KEY"
echo "Avvio JupiterOS — node $(node --version), auth: $AUTH"
cd "$REL"
exec ./jupiteros
