#!/usr/bin/env bash
# Avvio JupiterOS dalla root del repo (così legge il .mcp.json giusto, con Metis)
# usando il binario di produzione appena buildato. Chiude prima ogni istanza aperta.
set -e
cd "$(dirname "$0")"

# 1) chiudi qualsiasi istanza in esecuzione (sia il binario che eventuali nomi simili)
pkill -x jupiteros 2>/dev/null || true
pkill -f 'release/jupiteros' 2>/dev/null || true
sleep 1

BIN="./jupiteros/src-tauri/target/release/jupiteros"
if [ ! -x "$BIN" ]; then
  echo "ERRORE: binario non trovato: $BIN — esegui prima:  (cd jupiteros && npm run tauri build)"
  exit 1
fi

echo "Avvio $BIN  (cwd=$(pwd))"
# Workaround webkit per Fedora/Wayland (innocui altrove)
exec env WEBKIT_DISABLE_DMABUF_RENDERER=1 WEBKIT_DISABLE_COMPOSITING_MODE=1 "$BIN"
