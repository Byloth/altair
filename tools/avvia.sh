#!/usr/bin/env bash
# Avvia il gioco tradotto in background con il canale di comando (run/ctl) e, se richiesto,
# lo porta fino alla partita del profilo "Claude" (terza voce dell'elenco profili).
# Uso: tools/avvia.sh [--partita|--animus]   poi: tools/ctl.sh "key 28" "shot a.png" … "quit"
# --animus prosegue oltre il laboratorio fino al corridoio dell'Animus con Altaïr (circa 5 minuti).
# --tutorial supera anche i primi tre passi del tutorial (soggettiva, scostare, alto profilo).
# Il registro va in run/ctl/log.txt; l'audio miscelato in run/ctl/audio.raw.
set -euo pipefail
cd "$(dirname "$0")/.."
export GALLIUM_DRIVER="${GALLIUM_DRIVER:-d3d12}" LD_LIBRARY_PATH="${LD_LIBRARY_PATH:-/usr/lib/wsl/lib}"
mkdir -p run/ctl
if pgrep -f "^target/release/altair-run" >/dev/null; then ALTAIR_CTL_WAIT=100 tools/ctl.sh quit >/dev/null 2>&1 || pkill -f "^target/release/altair-run" || true; sleep 1; fi
rm -f run/ctl/stato run/ctl/cmd
ALTAIR_STATS=1 ALTAIR_CTL=run/ctl ALTAIR_AUDIO_DUMP="${ALTAIR_AUDIO_DUMP:-run/ctl/audio.raw}" setsid nohup tools/run.sh > run/ctl/log.txt 2>&1 < /dev/null &
case "${1:-}" in --partita|--animus|--tutorial) ;; *) exit 0 ;; esac
# Dopo i loghi parte il filmato introduttivo: lo si salta con Invio, poi si aspetta il titolo.
until tools/ctl.sh 2>/dev/null | grep -q "video=DemoIntro"; do sleep 1; done
sleep 2
tools/ctl.sh "key 28"
until tools/ctl.sh 2>/dev/null | grep -q "video=$"; do sleep 1; done
sleep 8
tools/ctl.sh "key 28";    sleep 6     # titolo
tools/ctl.sh "key 208 6"; sleep 1     # elenco profili: giù due volte
tools/ctl.sh "key 208 6"; sleep 1
tools/ctl.sh "key 28";    sleep 6     # scelta del profilo
tools/ctl.sh "key 28";    sleep 12    # conferma → menu principale
tools/ctl.sh "key 28"                 # CONTINUA (o NUOVA se non c'è un salvataggio)
[ "$1" = "--partita" ] && exit 0
# Laboratorio: la scena dura circa due minuti, poi aspetta un tasto; lo stesso nell'Animus.
# Si preme Spazio e si scorre l'elica finché il gioco non carica il corridoio.
sleep 100
until grep -q "Memory_Corridor" run/ctl/log.txt; do
  tools/ctl.sh "key 57 15" >/dev/null; sleep 12
  tools/ctl.sh "key 205 200" >/dev/null; sleep 12
done
sleep 20
[ "$1" = "--tutorial" ] || exit 0
# Durate in tempo reale (suffisso ms): il ritmo cambia molto da una scena all'altra.
sleep 45; tools/ctl.sh "key 18 700ms" >/dev/null             # Testa (E): visuale soggettiva
sleep 30; tools/ctl.sh "key 42 25000ms" "key 17 25000ms" >/dev/null   # Mano nuda + avanti tra i monaci
sleep 30; tools/ctl.sh "mouse -60 0" "key 17 8000ms" >/dev/null       # fino all'indicatore
sleep 25; tools/ctl.sh "button 1 4000ms" >/dev/null          # alto profilo (tasto destro)
sleep 12
