#!/usr/bin/env bash
# Builds for Windows, from WSL, everything tools/play.ps1 needs:
#   build/win/altair_ac1.dll                                   the translated game
#   target/x86_64-pc-windows-gnullvm/release/altair-run.exe    the runtime
# Required: LLVM-MinGW installed on Windows (winget install MartinStorsjo.LLVM-MinGW.UCRT),
# which from here is only read, and the Rust standard library for Windows in opt/rust-win/
# (downloaded from static.rust-lang.org; see docs/environment.md).
# Usage: tools/build-win.sh
set -euo pipefail
cd "$(dirname "$0")/.."
[[ -f "$HOME/.bash_exports" ]] && source "$HOME/.bash_exports"
[ -d opt/rust-win/sysroot ] || { echo "opt/rust-win/sysroot is missing: see docs/environment.md" >&2; exit 1; }
tools/build-image-win.sh
# The icon: extracted from the game's original executable (like the code, it stays in build/) and
# linked into ours as resource no. 1; the window takes it from there (window.rs).
AC1="${AC1:-/mnt/e/Giochi/Steam/steamapps/common/Assassins Creed}"
cargo build --release -p altair-recomp 2>&1 | grep -E '^error' || true
target/release/altair icon "$AC1/AssassinsCreed_Dx9.exe" --out build/win/altair.ico
printf '1 ICON "altair.ico"\n' > build/win/altair.rc
(cd build/win && llvm-rc /fo altair.res altair.rc && llvm-cvtres /machine:x64 /out:altair-res.o altair.res)
export ALTAIR_WIN_RES="$PWD/build/win/altair-res.o"
CARGO_TARGET_X86_64_PC_WINDOWS_GNULLVM_LINKER="$PWD/tools/win-link.sh" \
RUSTFLAGS="--sysroot=$PWD/opt/rust-win/sysroot" \
  cargo build --release --target x86_64-pc-windows-gnullvm -p altair-runtime 2>&1 | grep -E '^(warning|error)|Finished|lld: error' || true
ls -la target/x86_64-pc-windows-gnullvm/release/altair-run.exe build/win/altair_ac1.dll
