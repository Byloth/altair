#!/usr/bin/env bash
# Linker for building the runtime for Windows from WSL: the Linux clang with the MinGW target,
# lld, and the system libraries of LLVM-MinGW (installed on Windows, only read here).
# cargo calls it; see tools/build-win.sh.
set -euo pipefail
SYSROOT="${LLVM_MINGW:-$(ls -d /mnt/c/Users/*/AppData/Local/Microsoft/WinGet/Packages/MartinStorsjo.LLVM-MinGW.UCRT_*/llvm-mingw-*-x86_64 2>/dev/null | head -1)}"
BUILTINS="$(ls "$SYSROOT"/lib/clang/*/lib/windows/libclang_rt.builtins-x86_64.a | head -1)"
# -nodefaultlibs: the Linux clang would add libgcc, which does not exist in LLVM-MinGW; in its
# place compiler-rt (BUILTINS) and libunwind. The system libraries are already listed by rustc.
# libunwind is linked statically (`-l:libunwind.a`): otherwise the executable would look for
# libunwind.dll, which is not there on an arbitrary Windows.
args=()
for a in "$@"; do
  [ "$a" = "-lunwind" ] && a="-l:libunwind.a"
  args+=("$a")
done
# The resources (the icon), if tools/build-win.sh has prepared them.
[ -n "${ALTAIR_WIN_RES:-}" ] && [ -f "$ALTAIR_WIN_RES" ] && args+=("$ALTAIR_WIN_RES")
exec clang --target=x86_64-w64-mingw32 --sysroot="$SYSROOT" -fuse-ld=lld -nodefaultlibs "${args[@]}" -l:libunwind.a "$BUILTINS" -lmingw32 -lmingwex -lucrt -lkernel32 "$BUILTINS" 2> >(grep -v "argument unused" >&2)
