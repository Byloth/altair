#!/usr/bin/env bash
# Regenerates the translated image and recompiles only the files that changed.
# Usage: tools/build-image.sh [arguments for make, e.g. CC=gcc]
set -euo pipefail
cd "$(dirname "$0")/.."
AC1="${AC1:-/mnt/e/Giochi/Steam/steamapps/common/Assassins Creed}"
[[ -f "$HOME/.bash_exports" ]] && source "$HOME/.bash_exports"
cargo build --release 2>&1 | grep -E '^(error|warning: unused)|Finished' || true
target/release/altair image "$AC1/AssassinsCreed_Dx9.exe" --out build/gen/full --hooks runtime/hooks.txt | grep -E 'functions  |rewritten|hooks'
make -C build/gen/full -j"$(nproc)" "$@" 2>&1 | grep -vE '^make|^clang|^gcc' || true
