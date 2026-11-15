# The whole translated image

Measured on 2026-11-15 on this machine (28 cores, 31 GB), with clang 18.1.3 and gcc 13.3.0.
It is the gate of Phase 1: all of `.text` translated to C that compiles, verified by sampling.

```sh
source ~/.bash_exports && cargo build --release
target/release/altair image "$AC1/AssassinsCreed_Dx9.exe" --out build/gen/full    # from the project root
make -C build/gen/full -j28                        # clang -O1 → build/libaltair_ac1-clang.so
make -C build/gen/full -j28 CC=gcc                 # → build/libaltair_ac1-gcc.so
make -C build/gen/full -j28 OPT=-O2 TAG=clang-O2   # → build/libaltair_ac1-clang-O2.so
.venv/bin/python tools/difftest.py "$AC1/AssassinsCreed_Dx9.exe" --auto 20000 --lib build/libaltair_ac1-clang.so
```

## How the files are organised

Everything is in `build/gen/full/`, outside git.

- **One C file per 64 KB window of addresses**: `part_00401000.c` contains the functions that
  start between `0x401000` and `0x410FFF`. There are 241 files; the windows without functions (the
  3.5 MB `int3` block) produce no file.
- Every file declares at the top only the functions it defines or calls: there is no global header.
- `table.c`: the address → function table for indirect calls, sorted.
- `Makefile`: one object per file, plus the runtime, and the link into a shared library. The
  variables `CC`, `OPT` and `TAG` choose compiler, optimisation and output name.

Why by address: a change touches only its own file. Splitting "every N functions", one
extra discovered function would shift all the following files. Moreover the generator does not
rewrite a file identical to the existing one, so `make` recompiles only what has changed:
a second generation without changes rewrites 0 files.

Functions with more than 5,000 instructions are marked `ALTAIR_BIG` and the compiler does not
optimise them. Today there is only one: `0x00A33AF0`, with 25,075 instructions (scimitar-recomp reports one
of 29,000 that keeps clang busy for over 15 minutes). The threshold has not been tuned: with 5,000 the
compile time is already good.

## Numbers

| | |
|---|---|
| Functions | 116,077 |
| Instructions (counted per function) | 3,786,982 |
| Generation | 3.1 seconds |
| Generated C | 357 MB in 242 files; the largest 6.6 MB |

| Compiler | Wall time | CPU time | Peak memory per process | Objects | Library |
|---|---|---|---|---|---|
| clang `-O1` | 49 s | 956 s | 0.4 GB | 81 MB | 75 MB |
| clang `-O2` | 55 s | 1,109 s | 0.4 GB | — | 71 MB |
| gcc `-O1` | 50 s | 986 s | 0.7 GB | 83 MB | 78 MB |

For comparison, scimitar-recomp estimates about 1 GB of C++ for the same executable.

## Verification

Differential test against the original in Unicorn, using the whole-image library
directly (`difftest.py --lib`):

| Library | Functions tried | Identical | Different | Skipped |
|---|---|---|---|---|
| clang `-O1` | 8,000 | 4,199 | 0 | 3,801 |
| gcc `-O1` | 8,000 | 4,199 | 0 | 3,801 |
| clang `-O2` | **19,043** (all the candidates) | **9,892** | **0** | 9,151 |

Of the 9,892, 12 are identical up to a NaN or the exponent of an MMX register
(`translator.md`). Two different compilers give the same results: the clang bug found
the day before is worked around, and no others emerge in this sample.

## What this does not say

- **Verified are 9,892 functions out of 116,077**, plus the ones they call: 8.5%. They are the ones
  without imports, without indirect calls and that run with random inputs. The rest compiles but
  has not been executed.
- **Nothing has yet been executed "as a game"**: the Win32 runtime is missing (Phase 2). The whole
  image today serves only as a library for the test.
- Performance not measured. The code reads and writes the registers in the context at every instruction.
- The limits of discovery remain (`functions.md`): functions that do not return, exception
  tables, flags in tail calls between split functions.

## Since 2026-11-15: `-O2` and `restrict` by default

The generated `Makefile` uses `OPT = -O2`, and every function receives `Ctx *ALTAIR_RESTRICT c`
(`restrict`, unless `-DALTAIR_NO_RESTRICT`). Full compilation: 71 s with 28 processes.
Effect measured in the game: the translated code costs about half (`d3d9.md`, "The crowd").
Differential test (`tools/difftest.py`): OK.
