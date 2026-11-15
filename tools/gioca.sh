#!/usr/bin/env bash
# Avvia il gioco tradotto in una finestra, con tastiera, mouse e audio veri.
# Uso: tools/gioca.sh            (dalla radice del progetto; chiudere la finestra per uscire)
# Variabili utili: ALTAIR_LANG=English|Italian|…  ALTAIR_AUDIO=0  ALTAIR_VIDEO=0  ALTAIR_STATS=1
set -euo pipefail
cd "$(dirname "$0")/.."
# In WSL la GPU vera si raggiunge con OpenGL sopra D3D12 (vedi docs/analisi/d3d9.md).
export GALLIUM_DRIVER="${GALLIUM_DRIVER:-d3d12}" LD_LIBRARY_PATH="${LD_LIBRARY_PATH:-/usr/lib/wsl/lib}"
export ALTAIR_WINDOW=1
exec tools/run.sh "$@"
