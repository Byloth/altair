#!/usr/bin/env bash
# Rigenera l'immagine tradotta e ricompila solo i file cambiati.
# Uso: tools/build-image.sh [argomenti per make, es. CC=gcc]
set -euo pipefail
cd "$(dirname "$0")/.."
AC1="${AC1:-/mnt/e/Giochi/Steam/steamapps/common/Assassins Creed}"
[[ -f "$HOME/.bash_exports" ]] && source "$HOME/.bash_exports"
cargo build --release 2>&1 | grep -E '^(error|warning: unused)|Finished' || true
target/release/altair image "$AC1/AssassinsCreed_Dx9.exe" --out build/gen/full --hooks runtime/hooks.txt | grep -E 'funzioni  |riscritti|agganci'
make -C build/gen/full -j"$(nproc)" "$@" 2>&1 | grep -vE '^make|^clang|^gcc' || true
