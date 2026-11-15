#!/usr/bin/env bash
# Linker per compilare il runtime per Windows da WSL: il clang di Linux con il bersaglio MinGW,
# lld, e le librerie di sistema di LLVM-MinGW (installato su Windows, qui solo letto).
# Lo chiama cargo; vedi tools/build-win.sh.
set -euo pipefail
SYSROOT="${LLVM_MINGW:-$(ls -d /mnt/c/Users/*/AppData/Local/Microsoft/WinGet/Packages/MartinStorsjo.LLVM-MinGW.UCRT_*/llvm-mingw-*-x86_64 2>/dev/null | head -1)}"
BUILTINS="$(ls "$SYSROOT"/lib/clang/*/lib/windows/libclang_rt.builtins-x86_64.a | head -1)"
# -nodefaultlibs: il clang di Linux aggiungerebbe libgcc, che in LLVM-MinGW non esiste; al suo
# posto compiler-rt (BUILTINS) e libunwind. Le librerie di sistema le elenca già rustc.
# libunwind si collega in forma statica (`-l:libunwind.a`): altrimenti l'eseguibile cercherebbe
# libunwind.dll, che su un Windows qualunque non c'è.
args=()
for a in "$@"; do
  [ "$a" = "-lunwind" ] && a="-l:libunwind.a"
  args+=("$a")
done
exec clang --target=x86_64-w64-mingw32 --sysroot="$SYSROOT" -fuse-ld=lld -nodefaultlibs "${args[@]}" -l:libunwind.a "$BUILTINS" -lmingw32 -lmingwex -lucrt -lkernel32 "$BUILTINS" 2> >(grep -v "argument unused" >&2)
