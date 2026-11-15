#!/usr/bin/env bash
# Starts the translated game in a window, with real keyboard, mouse and audio.
# Usage: tools/play.sh            (from the project root; close the window to quit)
# F10 captures or releases the mouse pointer (captured: camera; released: menus).
# F8 shows or hides the diagnostics overlay (frames per second).
# The opening videos (logo, "presents", warnings) are skipped like the intro one.
# Useful variables: ALTAIR_LANG=English|Italian|…  ALTAIR_AUDIO=0  ALTAIR_VIDEO=0  ALTAIR_STATS=1
set -euo pipefail
cd "$(dirname "$0")/.."
# In WSL the real GPU is reached with OpenGL on top of D3D12 (see docs/analysis/d3d9.md).
export GALLIUM_DRIVER="${GALLIUM_DRIVER:-d3d12}" LD_LIBRARY_PATH="${LD_LIBRARY_PATH:-/usr/lib/wsl/lib}"
export ALTAIR_WINDOW=1
# One instance at a time: those already running are closed.
pkill -f "^target/release/altair-ru[n]" 2>/dev/null && sleep 1 || true
exec tools/run.sh "$@"
