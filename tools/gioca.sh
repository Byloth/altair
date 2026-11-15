#!/usr/bin/env bash
# Avvia il gioco tradotto in una finestra, con tastiera, mouse e audio veri.
# Uso: tools/gioca.sh            (dalla radice del progetto; chiudere la finestra per uscire)
# F10 cattura o libera il puntatore del mouse (catturato: visuale; libero: menu).
# F8 mostra o nasconde i fotogrammi al secondo nel titolo della finestra.
# Variabili utili: ALTAIR_LANG=English|Italian|…  ALTAIR_AUDIO=0  ALTAIR_VIDEO=0  ALTAIR_STATS=1
set -euo pipefail
cd "$(dirname "$0")/.."
# In WSL la GPU vera si raggiunge con OpenGL sopra D3D12 (vedi docs/analisi/d3d9.md).
export GALLIUM_DRIVER="${GALLIUM_DRIVER:-d3d12}" LD_LIBRARY_PATH="${LD_LIBRARY_PATH:-/usr/lib/wsl/lib}"
export ALTAIR_WINDOW=1
# Una sola istanza alla volta: si chiudono quelle già in esecuzione.
pkill -f "^target/release/altair-ru[n]" 2>/dev/null && sleep 1 || true
exec tools/run.sh "$@"
