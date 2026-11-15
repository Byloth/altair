#!/usr/bin/env bash
# Avvia il gioco tradotto. Uso: tools/run.sh [--trace]   (dalla radice del progetto)
set -euo pipefail
cd "$(dirname "$0")/.."
AC1="${AC1:-/mnt/e/Giochi/Steam/steamapps/common/Assassins Creed}"
exec target/release/altair-run "$AC1/AssassinsCreed_Dx9.exe" --lib "${ALTAIR_LIB:-build/libaltair_ac1-clang.so}" "$@"
