# scimitar-recomp: what it tells us about AC1

Reading notes (2026-11-14) on `ref/scimitar-recomp` at commit `4f36655` (2026-11-10):
`docs/AC1_WINDOWS.md` and `docs/PLAN.md`. Summarised in our own words; the code is GPL-2.0 and is
not to be copied. Where they write "estimate", it is their estimate.

## Their build and ours are the same code

Comparison done here with `tools/pe.py` and by reading the PE directories. All the values they
report for the Ubisoft Connect build match our Steam build:

| | Ubisoft Connect (theirs) | Steam (ours) |
|---|---|---|
| PE timestamp | 2008-04-23 | 2008-04-23 19:45:11 UTC |
| Entry point | `0xF9F445` (`WinMainCRTStartup`) | `0xF9F445` |
| Sections `.text` … `.rsrc` | same addresses | same addresses |
| TLS template | 673 bytes, index at `0x1A3561C` | same |
| `/GS` cookie | `0x19CA838` | `0x19CA838` |
| Exports | 178 (DARE audio engine) | 178 |
| Imports | 261 from 17 DLLs, including `uplay32` | 260 from 17 DLLs, without `uplay32` (there is `iphlpapi`) |
| Extra section | `.nidata` at `0x1BBD000` (4 KB) | none |
| Size | 24,671,912 bytes | 24,666,760 bytes (4,744-byte Authenticode signature at the end) |

**Conclusion.** Ubisoft Connect only added a `.nidata` section, with a copy of the
import table plus `uplay32.dll!Test`, which nobody calls. The code has the same addresses.
The addresses and counts of scimitar-recomp therefore hold for our build too.
A byte-for-byte confirmation of `.text` remains to be done (see `open-questions.md`), but by now it is a
formality.

Note: we count 17 DLLs and 260 functions. Their list names 17 including `uplay32`;
`iphlpapi` appears in both. The one-function difference is `uplay32!Test`.

## The executable, from their numbers

- **`.text` (19.4 MB)**: contains a 3.5 MB block of `int3` that is never referenced
  (`0x11B5239`–`0x1520860`), followed by the C++ unwind funclets.
- **Instructions**: the linear sweep decodes 9.85 million, 5.44 million of them `int3`.
  Following the flow from the functions found: 91,969 functions and 3.31 million instructions.
  With the final discovery: **98,470 functions, 3,484,935 instructions**. They translate 98,453;
  the remaining 17 are weak starts that are not code.
- **RTTI**: 4,376 type descriptors (3,407 in the `scimitar` namespace), 4,597 vtables found
  from the Complete Object Locators, 59,899 slots, 28,717 distinct functions.
- **Imports by DLL**: KERNEL32 157, USER32 32, WS2_32 30 (by ordinal), binkw32 14,
  ADVAPI32 7, ole32 3, XINPUT1_3 3, SHELL32 2, DSOUND 2, d3dx9_36 2, WININET 2, and one
  each from d3d9, DINPUT8, EAX, OLEAUT32, WINMM, iphlpapi. CRT statically linked.
- **Ubisoft's DARE audio engine** is linked in and exports 178 functions
  (`DareObjectFactory`, `SND_fn_*`). It runs on DirectSound + EAX.

## Census of the difficult instructions (linear sweep)

| What | How many | Note |
|---|---|---|
| `fs:` accesses (SEH chain, TEB) | 46,844 | on their own they blocked 12,905 functions |
| `ret imm` (stdcall/thiscall) | 76,904 | the callee removes the arguments from the guest stack |
| x87 comparisons (`fcom*`, `fucom*`, `fnstsw`, `sahf`…) | ~7,000 | |
| x87 functions (`fsqrt`, `fsin`, `fpatan`…) | ~60 | in the CRT math |
| `lock xadd` / `lock cmpxchg` | 9,188 | inlined `Interlocked*` |
| `pushfd`/`popfd`, `bt*`, `rcl`/`rcr`, `loop*` | a few hundred | CRT, CPUID checks |
| MMX/SSE2 on integer words | ~300 | DARE mixer |
| Jump tables | 728 | two-level ones too (byte table first) |

## Function discovery without symbols

- **Strong starts**: entry point, exports, TLS callbacks, and every destination of a direct
  `call` in the code reached from these.
- **Weak starts**: vtable slots; words in the data that point into the code and look like
  a beginning (MSVC aligns 96% of destinations to 16 bytes after `int3` padding);
  immediates with code addresses (`push offset callback`); addresses after `ret` and
  the linker's `jmp rel32` thunks.
- A function is the code reachable from its start without passing through another
  start (passing through one is a tail call). There are no fixed extents: code can be
  shared, and a wrong weak start costs duplicated code, not incorrect code.
- Weak starts are discarded, iteratively, if they fall inside an instruction of another
  function, if they are the destination of a conditional jump elsewhere, or if they do not decode.
  Only strong functions count as evidence against weak ones.
- `call [IAT]` and `jmp [IAT]` are import calls. The IAT cells must also be filled
  with host functions, for the MSVC pattern `mov esi, [__imp_X]; call esi`.
- Jump tables: one-level form, two-level form, `memcpy` tables indexed
  from 1 and from −3.

## Pitfalls found at runtime (not to be repeated)

1. **Code below a function's entry.** A PE function can jump to code before its
   start. The body emitted in address order must begin with a jump to the entry.
2. **Flags between split functions.** If discovery makes a function start inside another,
   the transition is a tail call and the flags must be passed in the context.
3. **MSVC jump tables.** Negative indices in `memcpy`/`memmove`; the comparison can sit
   several instructions before the jump; the bound of a two-level table is obtained
   from the byte table.
4. **Threads.** The window lives on an engine thread while the main thread waits.
   Translated frames require large host stacks: **256 MB** for the game threads.
5. **New render targets must be cleared.** The game reads targets it has not fully
   drawn (edges of the bloom chain).
6. **Draws that sample their own render target** (post-processing): a copy
   taken just before is needed.
7. **L8 render targets read as textures**: a swizzle on the view is needed, otherwise the
   menus turn pink.
8. **Compile time.** A 29,000-instruction x87 function kept clang's optimiser
   busy for more than 15 minutes. Above 4,000 x87 instructions they do not optimise.
   They generate in parts chosen by address; unchanged files are not recompiled.

## Status of their port (2026-11-10)

- It reaches the title screen, profiles, menus, Abstergo laboratory and the **tutorial
  in the Animus**, with 4096x1024 shadow map, depth pass, bloom and colour grading.
  900–2,000 draws per frame, about 50 fps at 2560x1440.
- The frame runs on **a single thread**; the four workers get little work. The main
  cost is the translated code (about half) and then the x87 runtime.
- Optimisations that mattered: constant upload without a per-parameter scan,
  `com::as<T>` with type tags instead of `dynamic_cast`, pad read as a snapshot, bit-exact
  fast paths for x87 (2x).
- **Not verified**: playing by hand, memories after the tutorial (Solomon's Temple, Masyaf),
  saving and loading, fixed-function/FVF draws, sRGB, audio actually heard, missing D3D9
  methods (each one stops execution with its name).

## Estimates for one person, made before the work

| Piece | Estimate |
|---|---|
| Difftest coverage of the new instructions | 2–3 days |
| Compiling the whole image | 1–2 days (about 1 GB of C++, generated in parts) |
| Windows runtime (TEB, TLS, stdcall, COM) | 3–5 days |
| KERNEL32, USER32 and minor DLLs | 2–3 weeks |
| Direct3D 9 (about 120 device methods, shaders, D3DX constant table) | 6–10 weeks |
| DirectInput 8 + XInput | 1 week |
| DirectSound 8 under DARE | 2 weeks |
| Bink (14 functions, via FFmpeg) | 2 days |
| Winsock: fail as unavailable | 1 day |

## Useful behavioural details

- The game finds pads through DirectInput and hands an Xbox 360 controller
  (vendor `0x045E`, product `0x028E`) to the XInput path (`PadXenon`).
- The game sees 4 processors by default in their port (`SCIMITAR_WIN32_CPUS`).
- Saves: ordinary files through `SHGetSpecialFolderPathA` and CRT I/O. The profile is a `.hdr`.
- Winsock: it is enough for it to fail as "unavailable". Uplay: nothing.
- **Names from AC2 Mac**: 4,978 functions named by matching, including `MainCommon::Initialize`,
  `EngineLoop::Enter`, `Engine::RegisterClasses`. Another 19,664 get class and slot from
  AC1's RTTI. About 85,000 remain unnamed.

## AC1 `.forge` archives (from `AC1_WINDOWS.md` and `PC_DATA.md`)

- The container has the same header as AC2: magic `scimitar`, **version 25**. Index and
  block layout are like AC2's.
- The FILEDATA record before each file's data is **4 bytes shorter** than in AC2: the magic
  of the compressed block is at `+0x9B8`, in AC2 at `+0x9BC`.
- AC1's blocks are **LZO1X_999 (type 1)**. For comparison: AC2 Mac uses LZO1X_1 (type 0),
  AC2 PC LZO2A (type 2).
- Classes in the archives are identified by the **CRC-32 of the class name**
  (e.g. `ShaderPermutation` = `0x9467F2BB`).
- FastLoad records are the fields in order, **without tags or versions**: a class's layout
  changes from game to game, so one game's data cannot be used with another's
  executable. For the same reason AC1 cannot be made to run on AC2's Mac executable.
- The first file of the first archive is `GlobalMetaFile`, followed by `Game Bootstrap Settings` and
  `Game Fix`.
- The shaders in the PC archives are **D3D9 bytecode** with a length prefix, inside the
  `ShaderPermutation` / `LightingShaderPermutation` records. The PC engine builds the constant
  map at load time with `D3DXGetShaderConstantTable`: that is why it is one of the two imported
  d3dx9 functions.

## Graphics: lessons from AC2 PC data and Revelations (`PC_DATA.md`, `ACR_WINDOWS.md`)

They apply to our D3D9 on wgpu, because the D3D9 layer is shared between AC1, ACR and PoP.

**Shader translation.** They go from D3D9 to ARB and then to MSL. ARB has no flow
control, so they have to flatten `if`/`else` (executing both branches) and unroll the `rep`s.
Flattening caused NaNs: an infinity in the branch not taken, multiplied by a 0 mask,
gives NaN, and half of Florence disappeared. **We translate D3D9 → WGSL directly**, which has real
flow control, and the problem does not arise. Other things that apply to us as well:

- column-major matrices in the constant table, to be read transposed;
- bool registers: they must be supplied as separate float constants (the bools used in ACR are
  `g_VolumeFogEnabled`, `g_UseDepthTextureShadows`, `g_DeconstructionEnabled`, `g_HasSun`, …);
- `texldl` with explicit LOD, `vFace`, large relative offsets;
- bool constants must be placed after the float registers (bone palettes go up to c255).

**Direct3D 9 bugs found in Revelations and shared with AC1:**

1. **`vs_3_0` colour outputs must not be saturated.** The game writes linear depth to
   an R32F render target through a `COLOR` output. The clamp to 0..1 applies only below SM3.
2. **`vPos` is the integer pixel coordinate.** In modern APIs the position is the centre
   of the pixel (k + 0.5): 0.5 must be subtracted.
3. **Half pixel.** D3D9's pixel k is k + 0.5 in modern APIs: positions must be
   shifted by half a pixel *right and down* in clip space. They use 63/64 of half a pixel,
   like Wine. With the error, the bloom chain (from 320x180 to 20x11) produced a bright edge.
4. **Depth buffer larger than the render target**: in D3D9 it is legal; it must be rejected only if it is
   smaller.
5. **Recorded state blocks**: the game uses two to reset the states, thousands of times
   a minute.
6. **Renaming buffers and textures** rewritten after a recorded draw has used them.
7. **Fullscreen device**: a fullscreen `CreateDevice` or `Reset` resizes the
   window to the back buffer, and the game receives `WM_SIZE`. Without this, ACR stayed black.
8. Other things solved for AC1: `StretchRect` scaling, NULL render targets, `Reset` preserving
   the back buffer, the device holding references to what it has bound.

**Win32 runtime:**

- `QueryPerformanceCounter` at **10 MHz** from process start: nanoseconds since system
  boot overflowed the game's `ticks * 1000000 / frequency` computation.
- With a pad connected the game asks DirectInput for vibration effects: `CreateEffect`
  must answer "not supported" instead of aborting.
- **Number of processors.** In ACR, declaring 2 or 4 CPUs makes the menu background flicker between two
  projections: it is a race in the game's multiprocessor path, which their timing makes
  visible. ACR now declares 1 CPU; AC1 stays at 4 (`SCIMITAR_WIN32_CPUS`). To keep an eye on.
- The game creates and manages the window on an engine thread, not on the main thread.

## Function discovery: why AC1 is harder (`ACR_WINDOWS.md`)

- ACR has relocations (862,811) and a SafeSEH table of 34,150 handlers: relocations tell with
  certainty which words are addresses. **AC1 has no relocations**: every word in the data that falls
  within the code range is a candidate start, and that is where the false `in`/`out`/`les`
  "instructions" come from.
- With relocations ACR goes from 188,802 to 274,540 functions. Without them, functions reached
  only through vtables were missing; for example the CRT stores the address of a `TlsAlloc` wrapper that follows
  a `ret`, without padding.
- The discovery "by runs of code addresses in the data" used for AC1 misses the labels of the
  CRT's x87 dispatch, stored unaligned in `.data`.
- Source of names for AC1 from their comparison: **4,323 RTTI names** with a string scan,
  2,582 of them `scimitar`. Our `pe.py rtti` finds 4,043: we need to understand why
  (different regex? nested names?) when we write the real parser.

## Structure of their Windows layer (`platform/README.md`, `platform/win32/`)

Useful as a map of the modules of our runtime, not as code:

| File | What it covers |
|---|---|
| `core.cpp` | handles, waits, TEB and static TLS, IAT |
| `kernel32.cpp`, `kernel32_sync.cpp`, `kernel32_file.cpp` | KERNEL32: base, synchronization, files |
| `user32.mm`, `win32_window.h` | window and messages |
| `system.cpp`, `registry.cpp` | small DLLs, emulated registry |
| `com.cpp/h` | COM objects in guest memory |
| `d3d9.mm`, `d3d9_metal.mm`, `d3d9_internal.h` | D3D9 API and renderer |
| `dinput.cpp`, `input.cpp`, `xinput.mm` | input |
| `dsound.cpp` | DirectSound |
| `bink_win.cpp` | Bink (stdcall wrapper over FFmpeg) |
| `games/ac1/main.cpp`, `game.cpp` | startup and AC1-specific code |

Host functions callable from the guest have a guest address of their own (`kHostFnBase`); from the
host side the guest is called back with `call_guest` / `call_virtual`.

## Translator (`recomp/README.md`)

- Every function becomes `void f_XXXXXXXX(Context& c)`: registers in the context, guest memory
  through base plus offset, internal jumps as `goto`, calls that push the real return
  address on the guest stack, tail calls as `f_target(c); return;`.
- Indirect calls go through a registry of translated and host functions. Jump tables
  become a `switch` on the table's labels. Flags are local booleans, kept only
  where a later instruction reads them.
- Exact 80-bit x87: **450,000 results compared with Unicorn without differences**, except for two
  points where QEMU differs from hardware (`fisttp` overflow, conversions of signalling
  NaNs). Hardware is followed.
- SSE with x86 semantics on any host: negative default NaN, float→int conversions
  that give `0x80000000`, the min/max operand rule. `rcp`/`rsqrt` are exact as in
  Unicorn, while real CPUs give 12-bit approximations: the tests need tolerance or emulation.
- `rdtsc` as a 1 GHz counter from host time; `cpuid` as a fixed SSE3-class CPU.
- Their difftest on AC2 compared 1,500 functions chosen to cover every type of
  instruction, with 8 cases each: 8,889 cases identical to Unicorn.
- Whole AC2 image: 947 MB of C++ in 64 files, compiled in 125 s on 10 cores at `-O1`.

## Graphics backend (`METAL.md`, only the points that apply to wgpu)

- The graphics surface used by the game is narrow; only that is implemented. The census of
  a frame (calls by type) is the first step, and must be redone for D3D9 on AC1.
- **Pipeline precompilation and caching**: without it, every new shader pair costs about
  30 ms the first time.
- Depth: a format different from the original D24 changes the scale of the polygon offset.
- Occlusion queries: beware of waits if the GPU is more than one frame behind.

## Improvements in their port (`README.md`)

A useful list for our "nice to have"s, because it shows what the translation layer makes possible:
any screen aspect ratio with the HUD at the edges, draw distance up to 6x, 4096x4096
shadows, 16x anisotropic filtering, native resolutions, launcher with settings,
shader cache, some hot math routines replaced by equivalent native code.

They themselves write that reconstructing readable engine sources would take years, and that
they could not be published because they are derived from Ubisoft code. This is consistent with our choice
of targeted decompilation.
