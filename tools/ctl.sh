#!/usr/bin/env bash
# Sends commands to the running game (started with ALTAIR_CTL=run/ctl) and waits for it to execute them.
# Usage: tools/ctl.sh "key 28" "shot a.png"      (from the project root)
# Commands: key DIK [frames] | text TEXT | shot FILE.png | quit
set -euo pipefail
cd "$(dirname "$0")/.."
dir="${ALTAIR_CTL:-run/ctl}"
rm -f "$dir/status"
printf '%s\n' "$@" > "$dir/cmd.tmp"
mv "$dir/cmd.tmp" "$dir/cmd"
for _ in $(seq "${ALTAIR_CTL_WAIT:-600}"); do
  [ -f "$dir/status" ] && { echo "frame $(cat "$dir/status")"; exit 0; }
  sleep 0.1
done
echo "the game did not respond" >&2; exit 1
