#!/usr/bin/env bash
# Benchmark of the game thread in the tutorial crowd (about 3,800 draws): drives the game there
# with the GPU submission thrown away (ALTAIR_BENCH=1), waits, and prints the frame rate.
# Usage: tools/bench-crowd.sh [label]      extra environment (ALTAIR_LIB=...) is passed through
set -euo pipefail
cd "$(dirname "$0")/.."
ALTAIR_BENCH=1 tools/start.sh --animus >/dev/null 2>&1
sleep 45; tools/ctl.sh "key 18 700ms" >/dev/null 2>&1   # Head: first-person view, opens the crowd step
sleep 34
# The busiest threads, to see who limits the frame rate.
top -H -b -n 1 -d 3 -p "$(pgrep -f 'altair-ru[n]' | head -1)" 2>/dev/null | awk 'NR>7 && $9+0 > 5 {printf "   thread %-18s %s%% cpu\n", $12, $9}' | head -6
for _ in 1 2 3; do sleep 5; tools/ctl.sh stats 2>/dev/null | tail -1 | grep -o "([0-9.]* fps, [0-9]* draws, [0-9.]* ms in the renderer, [0-9.]* in shims" || true; done | sed "s/^/${1:-bench}: /"
tools/ctl.sh quit >/dev/null 2>&1 || true
