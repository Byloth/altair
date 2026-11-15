#!/usr/bin/env bash
# Compila per Windows, da WSL, tutto ciò che serve a tools/gioca.ps1:
#   build/win/altair_ac1.dll                                   il gioco tradotto
#   target/x86_64-pc-windows-gnullvm/release/altair-run.exe    il runtime
# Servono: LLVM-MinGW installato su Windows (winget install MartinStorsjo.LLVM-MinGW.UCRT),
# che da qui si legge soltanto, e la libreria standard di Rust per Windows in opt/rust-win/
# (scaricata da static.rust-lang.org; vedi docs/ambiente.md).
# Uso: tools/build-win.sh
set -euo pipefail
cd "$(dirname "$0")/.."
[[ -f "$HOME/.bash_exports" ]] && source "$HOME/.bash_exports"
[ -d opt/rust-win/sysroot ] || { echo "manca opt/rust-win/sysroot: vedi docs/ambiente.md" >&2; exit 1; }
tools/build-image-win.sh
CARGO_TARGET_X86_64_PC_WINDOWS_GNULLVM_LINKER="$PWD/tools/win-link.sh" \
RUSTFLAGS="--sysroot=$PWD/opt/rust-win/sysroot" \
  cargo build --release --target x86_64-pc-windows-gnullvm -p altair-runtime 2>&1 | grep -E '^(warning|error)|Finished|lld: error' || true
ls -la target/x86_64-pc-windows-gnullvm/release/altair-run.exe build/win/altair_ac1.dll
