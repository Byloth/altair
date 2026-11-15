#!/usr/bin/env bash
# Compila l'immagine tradotta come DLL per Windows, da WSL, con il clang di Linux e le librerie
# di sistema di LLVM-MinGW (installato su Windows con winget: da qui lo si legge soltanto).
# Serve la variante MinGW perché è l'unica in cui `long double` è a 80 bit, come vuole l'x87.
# Uso: tools/build-image-win.sh        → build/win/altair_ac1.dll
# Prima va generato il C: tools/build-image.sh (lo stesso per Linux e Windows).
set -euo pipefail
cd "$(dirname "$0")/.."
SYSROOT="${LLVM_MINGW:-$(ls -d /mnt/c/Users/*/AppData/Local/Microsoft/WinGet/Packages/MartinStorsjo.LLVM-MinGW.UCRT_*/llvm-mingw-*-x86_64 2>/dev/null | head -1)}"
[ -d "$SYSROOT/x86_64-w64-mingw32" ] || { echo "LLVM-MinGW non trovato: impostare LLVM_MINGW" >&2; exit 1; }
BUILTINS="$(ls "$SYSROOT"/lib/clang/*/lib/windows/libclang_rt.builtins-x86_64.a | head -1)"
OBJ=build/obj/full-win
mkdir -p "$OBJ" build/win
export SYSROOT OBJ
CFLAGS="--target=x86_64-w64-mingw32 --sysroot=$SYSROOT -O2 -fwrapv -fno-strict-aliasing -fno-math-errno -ffp-contract=off -w -Iruntime"
export CFLAGS
compile() {
  src="$1"; obj="$OBJ/$(basename "${src%.c}").o"
  # Si ricompila solo ciò che è cambiato (il sorgente o le intestazioni del runtime).
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
