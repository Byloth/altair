#!/usr/bin/env bash
# Starts the translated game in the background with the control channel (run/ctl) and, if asked,
# takes it all the way into the game of the "Claude" profile (third entry of the profile list).
# Usage: tools/start.sh [--game|--animus]   then: tools/ctl.sh "key 28" "shot a.png" … "quit"
# --animus continues past the laboratory to the Animus corridor with Altaïr (about 5 minutes).
# --tutorial also gets through the first three steps of the tutorial (first-person view, gentle push, high profile).
# The log goes to run/ctl/log.txt; the mixed audio to run/ctl/audio.raw.
set -euo pipefail
cd "$(dirname "$0")/.."
export GALLIUM_DRIVER="${GALLIUM_DRIVER:-d3d12}" LD_LIBRARY_PATH="${LD_LIBRARY_PATH:-/usr/lib/wsl/lib}"
mkdir -p run/ctl
# At startup every running instance of the game is closed, even one opened by hand with
# tools/play.sh: two instances together fight over GPU, profiles and saves. The one started
# from here is closed gracefully (census in the log), the others with a signal.
if [ -f run/ctl/pid ] && kill -0 "$(cat run/ctl/pid)" 2>/dev/null; then ALTAIR_CTL_WAIT=100 tools/ctl.sh quit >/dev/null 2>&1 || true; fi
pkill -f "^target/release/altair-ru[n]" 2>/dev/null || true
sleep 1
rm -f run/ctl/status run/ctl/cmd
ALTAIR_STATS=1 ALTAIR_CTL=run/ctl ALTAIR_AUDIO_DUMP="${ALTAIR_AUDIO_DUMP:-run/ctl/audio.raw}" setsid nohup tools/run.sh > run/ctl/log.txt 2>&1 < /dev/null &
echo $! > run/ctl/pid
case "${1:-}" in --game|--animus|--tutorial) ;; *) exit 0 ;; esac
# Logo, "presents", warnings and intro video: each is skipped with Enter as soon as it plays
# (the runtime makes the first three skippable too). Then we wait for the title.
intro=0
until [ "$intro" = 1 ] && tools/ctl.sh 2>/dev/null | grep -q "video=$"; do
  case "$(tools/ctl.sh 2>/dev/null | grep -o 'video=.*')" in
    video=DemoIntro*) intro=1; sleep 1; tools/ctl.sh "key 28" >/dev/null ;;
    video=LoadingScreen*|video=) ;;
    video=*) sleep 1; tools/ctl.sh "key 28" >/dev/null ;;
  esac
  sleep 1
done
sleep 8
tools/ctl.sh "key 28";    sleep 6     # title
tools/ctl.sh "key 208 6"; sleep 1     # profile list: down twice
tools/ctl.sh "key 208 6"; sleep 1
tools/ctl.sh "key 28";    sleep 6     # profile selection
tools/ctl.sh "key 28";    sleep 12    # confirm → main menu
tools/ctl.sh "key 28"                 # CONTINUE (or NEW if there is no save)
[ "$1" = "--game" ] && exit 0
# Lab and Animus: two long dialogue scenes, each ending on "press any button", then the DNA
# helix to scroll. The game clock stays at 8x (F9's fast-forward) until the corridor is loaded.
tools/ctl.sh "speed 8" >/dev/null
until grep -q "Memory_Corridor" run/ctl/log.txt; do
  tools/ctl.sh "key 57 15" >/dev/null; sleep 3
  tools/ctl.sh "key 205 200" >/dev/null; sleep 3
done
sleep 6
tools/ctl.sh "speed 1" >/dev/null
[ "$1" = "--tutorial" ] || exit 0
# Durations in real time (ms suffix): the pace changes a lot from one scene to another.
fast() { tools/ctl.sh "speed 8" >/dev/null; sleep "$1"; tools/ctl.sh "speed 1" >/dev/null; }   # waits, at 8x
fast 6; tools/ctl.sh "key 18 700ms" >/dev/null             # Head (E): first-person view
fast 4; tools/ctl.sh "key 42 25000ms" "key 17 25000ms" >/dev/null   # Empty hand + forward among the monks
fast 4; tools/ctl.sh "mouse -60 0" "key 17 8000ms" >/dev/null       # up to the marker
sleep 25; tools/ctl.sh "button 1 4000ms" >/dev/null          # high profile (right button)
sleep 12
