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
# Rendering: di default usiamo il percorso ACCELERATO di WebKitGTK (DMABUF).
# Su Intel/Mesa (UHD 630, driver Iris) è il percorso giusto: disabilitarlo
# scarica il rendering sulla CPU e la GUI diventa pesante.
# Il vecchio workaround anti schermata-bianca (nato per bug NVIDIA) resta
# disponibile opt-in:  JUPITEROS_SAFE_GFX=1 ./run-jupiteros.sh
if [ "${JUPITEROS_SAFE_GFX:-0}" = "1" ]; then
  echo "(SAFE_GFX: renderer DMABUF disabilitato — modalità compatibilità)"
  exec env WEBKIT_DISABLE_DMABUF_RENDERER=1 "$BIN"
fi
exec "$BIN"
