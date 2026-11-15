# Decisions

Most recent entries at the top. Each entry: decision, reason, what would make us revisit it.

## 2026-11-15 · Our own Bink decoder in place of ffmpeg

**Decision:** the videos are decoded by `crates/bink`, written by us; `ffmpeg` remains only as
the test oracle (`tools/binktest.py`) and as a fallback (`ALTAIR_BINK=ffmpeg`).

**Why:** `ffmpeg` as an external process was a system dependency and does not exist in the
browser. The Bink 1 format is publicly documented and the game's files use a small
subset of it (revision `i`, no alpha, DCT audio).

**Rule followed:** as with scimitar-recomp, the FFmpeg code (LGPL) was read as a
specification and not copied. The format's tables are data: reproduced with their provenance.

**Verification:** 3,381 frames byte-for-byte identical to ffmpeg; audio within 4 × 10⁻⁷.

## 2026-11-15 · x87 on the host's `long double`

**Decision.** The x87 registers are `long double` and the operations are C arithmetic; the host's
x87 control word follows the guest's. No software emulation.

**Why.** On x86 hosts `long double` is the 80-bit x87 format: results identical
to the original without writing an emulator, verified on 4,201 functions. Native comes before
the web (decision of 2026-11-14).

**Accepted cost.** It does not work on ARM or on wasm, where `long double` is a different format:
for the browser (Phase 5) a software x87 behind the same interface (`f80`, `ST(i)`) will be needed.

**To be revisited** when Phase 5 opens, or if a native build for ARM is needed.

## 2026-11-15 · Full name: "AC1 Recompiled - Altaïr"

**Decision.** The extended title of the project becomes "AC1 Recompiled - Altaïr" (previously
"AC1 Recompiled"). In code, commands and paths it remains `altair`, without the diaeresis.

**Why.** The user's choice: the technical name enters the title. The entry below, from the same
day, still gives the previous title.

## 2026-11-15 · Technical name of the project: Altair

**Decision.** The recompiler and everything that is code are called `altair` (previously `acrek`);
the project folder becomes `altair` (previously `ac-rek`). The full name remains
"AC1 Recompiled". A recompiler for another game would take the name of its protagonist.

**Why.** The user's choice. Altair is a proper name and a star, already used in computing
(the Altair 8800 of 1975): as a technical name it does not evoke an official product.

**Caution.** There is a very widespread Python library with the same name: to be kept in mind
if packages are published one day. Externally we use "AC1 Recompiled".

## 2026-11-15 · SSE in portable C; partial registers with narrow writes

**Decision.** The SSE operations are lane-by-lane C functions (`runtime/altair_sse.h`),
without processor intrinsics. The 8- and 16-bit registers are written with `SET8L`/`SET8H`/`SET16`
(narrow writes into the 32-bit field), not with mask and OR.

**Why.** Portable C compiles on every host, and on x86-64 it produces the same instructions
as the original: 3,042 identical functions out of 6,000 confirm it. The narrow writes work around
a miscompilation in clang 18.1.3 (`tests/compiler/clang18-partial-register.c`).

**To be revisited if** on ARM or wasm intrinsics are needed for performance or for NaN semantics,
or if a more recent clang no longer has the bug.

## 2026-11-14 · Shape of the generated code

**Decision.** One C function per guest function, `void f_XXXXXXXX(Ctx *c)`; guest stack
identical to the original (real return addresses); flags as local variables; guest memory
as base plus address over 4 GB reserved; indirect calls through a function registry.
The support is in C (`runtime/altair.h`), the recompiler in Rust. Details in
`analysis/translator.md`.

**Why.** It is the model of scimitar-recomp, which has been shown to work on this
executable; the identical stack makes the byte-for-byte comparison with Unicorn possible; local
flags leave optimisation to the C compiler.

**To be revisited if** performance requires it (registers in local variables) or if on wasm32 the
C frames turn out to be too deep.

## 2026-11-14 · The running original DX9 is not needed now: moved to Phase 3, optional

**Decision.** We do not work on getting the original `AssassinsCreed_Dx9.exe` to start on Windows
(it hangs at startup). The task leaves Phase 0 and becomes optional in Phase 3.

**Why.** The correctness of the translated code is verified with the Unicorn harness, without
Windows. For the visual comparison there is the original DX10, which works. The COM methods used are
obtained from our runtime with traced stubs.

**To be revisited if** in Phase 3 a rendering difference cannot be explained with the DX10.

## 2026-11-14 · scimitar-recomp's addresses hold for our build

**Decision.** We use scimitar-recomp's addresses, counts and pitfalls as valid for the
Steam build, without remapping anything.

**Why.** Entry point, section addresses, TLS, `/GS` cookie and exports of their
Ubisoft Connect build match ours. Ubisoft Connect only adds `.nidata` at the end.
Details in `analysis/scimitar-recomp.md`.

**To be revisited if** an address taken from them does not match the code we see.

## 2026-11-14 · DX12 is obtained through wgpu, not by converting the renderer

**Decision.** The game keeps calling D3D9; our runtime implements D3D9 on top of
wgpu, which uses DX12 (or Vulkan) on Windows, Metal on macOS, WebGPU in the browser.

**Why.** Porting the engine to call DX12 directly would mean decompiling the whole
renderer: months of work for the same result. The planned graphics improvements (resolution,
MSAA, LOD, texture replacement, post-processing) all go through the translation layer.

**To be revisited if** features are needed that wgpu does not expose, such as ray tracing (today only
experimental).

## 2026-11-14 · Reference executable: `AssassinsCreed_Dx9.exe` from the Steam build

**Decision.** We recompile the DX9 (SHA-1 `a9ea476e…432120`), not the DX10.

**Why.**
1. It has 4,376 RTTI names against 939: it is the main source of names without a PDB.
2. scimitar-recomp (our specification) and EaglePatch (Steam addresses) work on the DX9.
3. We reimplement D3D9 ourselves, so the original's hang at startup on Windows does not
   affect the recompiled version.

**Accepted cost.** To compare with the running original, the original DX9 has to be made to
start (DXVK or dgVoodoo2, to be tried). Per-function differential testing with Unicorn does not
need it. The DX10 renderer, with its better MSAA, is of no use to us: the graphics are ours.

**To be revisited if** scimitar-recomp's Ubisoft Connect build turns out to be very different from the Steam one,
taking value away from the specification.

## 2026-11-14 · Rewrite, not a fork of scimitar-recomp

See the project document, section "Decisione: riuso o riscrittura" (Decision: reuse or rewrite). In short: about
70% of their code (Metal, Cocoa, arm64) would have to be rewritten anyway, and a fork would make
everything GPL-2.0.

## 2026-11-14 · Order: native → web → modifications

See the project document, sections "Sintesi" (Summary) and "Fasi" (Phases). The gate is Phase 4: game finished
natively.
