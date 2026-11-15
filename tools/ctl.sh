#!/usr/bin/env bash
# Manda comandi al gioco in esecuzione (avviato con ALTAIR_CTL=run/ctl) e aspetta che li esegua.
# Uso: tools/ctl.sh "key 28" "shot a.png"      (dalla radice del progetto)
# Comandi: key DIK [fotogrammi] | text TESTO | shot FILE.png | quit
set -euo pipefail
cd "$(dirname "$0")/.."
dir="${ALTAIR_CTL:-run/ctl}"
rm -f "$dir/stato"
printf '%s\n' "$@" > "$dir/cmd.tmp"
mv "$dir/cmd.tmp" "$dir/cmd"
for _ in $(seq "${ALTAIR_CTL_WAIT:-600}"); do
  [ -f "$dir/stato" ] && { echo "fotogramma $(cat "$dir/stato")"; exit 0; }
  sleep 0.1
done
echo "il gioco non ha risposto" >&2; exit 1
