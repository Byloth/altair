#!/usr/bin/env bash
# Builds the translated image as a DLL for Windows, from WSL, with the Linux clang and the
# system libraries of LLVM-MinGW (installed on Windows with winget: from here it is only read).
# The MinGW flavour is needed because it is the only one where `long double` is 80 bits, as x87 requires.
# Usage: tools/build-image-win.sh        → build/win/altair_ac1.dll
# The C must be generated first: tools/build-image.sh (the same for Linux and Windows).
set -euo pipefail
cd "$(dirname "$0")/.."
SYSROOT="${LLVM_MINGW:-$(ls -d /mnt/c/Users/*/AppData/Local/Microsoft/WinGet/Packages/MartinStorsjo.LLVM-MinGW.UCRT_*/llvm-mingw-*-x86_64 2>/dev/null | head -1)}"
[ -d "$SYSROOT/x86_64-w64-mingw32" ] || { echo "LLVM-MinGW not found: set LLVM_MINGW" >&2; exit 1; }
BUILTINS="$(ls "$SYSROOT"/lib/clang/*/lib/windows/libclang_rt.builtins-x86_64.a | head -1)"
OBJ=build/obj/full-win
mkdir -p "$OBJ" build/win
export SYSROOT OBJ
CFLAGS="--target=x86_64-w64-mingw32 --sysroot=$SYSROOT -O2 -fwrapv -fno-strict-aliasing -fno-math-errno -ffp-contract=off -w -Iruntime"
export CFLAGS
compile() {
  src="$1"; obj="$OBJ/$(basename "${src%.c}").o"
  # Only what has changed is recompiled (the source or the runtime headers).
  if [ ! -f "$obj" ] || [ "$src" -nt "$obj" ] || [ -n "$(find runtime -name '*.h' -newer "$obj" -print -quit)" ]; then
    clang $CFLAGS -c "$src" -o "$obj"
  fi
}
export -f compile
ls build/gen/full/*.c runtime/altair.c | xargs -P "$(nproc)" -I{} bash -c 'compile {}'
echo "link build/win/altair_ac1.dll"
clang --target=x86_64-w64-mingw32 --sysroot="$SYSROOT" -fuse-ld=lld -shared "$OBJ"/*.o -o build/win/altair_ac1.dll \
  -nodefaultlibs -lmingw32 "$BUILTINS" -lmoldname -lmingwex -lucrt -lkernel32 "$BUILTINS"
ls -la build/win/altair_ac1.dll
