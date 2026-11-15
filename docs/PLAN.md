# Work plan

Operational version of the project document
([AC1 Recompiled - Altaïr: piano di progetto](https://claude.ai/artifact/SoPWUhJZPJ45oWk3t4r8bu), in Italian).
Each phase closes with a verifiable **gate**; we do not move on to the next phase without it.
Tick the boxes as you go; every result is written in `analysis/`, `decisions.md` or `journal.md`.

```
0 Foundations → 1 Recompiler → 2 Win32 runtime → 3 Graphics/audio/input
  → 4 PLAYABLE NATIVELY (main gate) → 5 Browser → 6 Targeted decompilation and mods
```

---

## Phase 0 · Foundations ✅

**Gate:** Steam build mapped. Initial list of functions, RTTI and vtables counted and
named, complete map of the imports, one game function run in Unicorn from a script.

### 0.1 Build inventory ✅

- [x] Install the Steam build and compute the hashes → `analysis/executables.md`
- [x] PE header, sections, imports of DX9 and DX10 (`tools/pe.py`)
- [x] DX9 / DX10 comparison and choice of DX9 → `decisions.md`
- [x] Identify the launcher and accessory files (`AssassinsCreed_Game.exe`, `Detection/`)

### 0.2 Working environment ✅

Completed on 2026-11-14 (WSL, 28 cores, 31 GB RAM). Details in `environment.md`.

- [x] `git init` of the repository and first commit
- [x] Rust 1.100.0 (in `~/.local/share/cargo`)
- [x] clang/LLVM 18 + lld
- [x] `.venv/` with unicorn 2.1.4 and capstone 5.0.7
- [x] Java 25 + Ghidra 12.2 in `opt/` (Ghidra project in `private/ghidra/`, to be created)
- [x] Clone scimitar-recomp and EaglePatch into `ref/` (ignored by git)

### 0.3 The specification: scimitar-recomp and EaglePatch ✅

- [x] Read `docs/AC1_WINDOWS.md` and `docs/PLAN.md` → `analysis/scimitar-recomp.md`
- [x] Read `README`, `PC_DATA.md`, `ACR_WINDOWS.md`, `METAL.md` (useful parts),
  `ANVIL_FAMILY.md` (comparison), `recomp/README.md`, `platform/README.md`
- [x] Extract from EaglePatch the already known Steam addresses → `analysis/known-addresses.md`
- [x] Compare the layout of their build with ours (entry point, sections, TLS, cookie)

### 0.4 Steam versus Ubisoft Connect ✅

- [x] Identical layout: same code at the same addresses, Ubisoft Connect only adds `.nidata`
- [x] Outcome and consequences → `decisions.md`
- [ ] (optional) Byte-by-byte comparison of `.text`, if one day we have their build

### 0.5 Types: RTTI and vtables ✅

- [x] `tools/rtti.py`: Complete Object Locator, Type Descriptor, Class Hierarchy Descriptor,
  vtables with address and slots
- [x] Counts identical to scimitar-recomp: 4,376 descriptors, 4,597 vtables, 59,899 slots,
  28,717 functions
- [x] Detail in `private/rtti-dx9.json`; statistics and subsystems in `analysis/rtti.md`

### 0.6 Imports and operating system surface ✅

- [x] `tools/imports.py` and `analysis/imports.md`: 260 functions with IAT cell, references in the
  code, category, runtime module that will implement them and phase
- [x] COM interfaces required, from the GUIDs in `.rdata` (DirectInput 8, DirectSound 8 + 3D, EAX)
- [ ] COM methods actually called: deferred to the runtime (traced stubs, Phase 2–3)

### 0.7 First list of functions ✅ (prototype)

- [x] `tools/functions.py`: strong starts (entry, exports, TLS, direct calls) and weak ones
  (vtables, immediates, words in the data) with trial decoding
- [x] 115,807 starts and 3.19 million instructions (96,781 with `--strict`), against the 98,470
  functions of scimitar-recomp → `analysis/functions.md`
- [x] Cross-check with the EaglePatch addresses and with the vtable slots
- [ ] Import RTTI and functions into Ghidra to have a browsable view (moved: to be done
  when code needs to be read, does not block the gate)

### 0.8 Unicorn harness ✅

- [x] `tools/harness.py`: image at `0x400000`, stack, TEB and `fs:` (through the GDT), imports
  intercepted by trampolines with handlers in Python
- [x] `selftest`: the CRT's `strlen`, `strcmp` and `memcpy` run from the original and compared
  with Python; an engine function with an SEH prologue returns correctly
- [x] `analysis/harness.md`: usage, structure, limits
- [ ] First test on engine code (CRC-32 of the class names): the function has to be found

### 0.10 Game data (reconnaissance) ✅

- [x] `tools/forge.py`: v25 container, tables, records, LZO1X sections, object packages
- [x] Inventory of the 18 archives (15,906 entries, 6.45 GB) → `analysis/forge.md`
- [x] The object classes are CRC-32s of the RTTI names: 100% recognised on 3 archives
- [x] "Missions: data or code?" → structure in the data (27 `MissionItem*` classes, about 950
  objects), behaviour in the code

**Phase 0 closed on 2026-11-14.** Gate satisfied: build mapped, functions, RTTI and vtables,
imports, execution in Unicorn, data format.

---

## Phase 1 · Recompiler (gate reached on 2026-11-15)

**Gate:** all of `.text` translated into C that compiles; differential tests green on a large
sample of functions. Status and numbers in `analysis/translator.md`.

### 1.1 Foundations ✅

- [x] Rust workspace: `crates/pe`, `crates/recomp` (binary `altair`)
- [x] PE parser with verification of the Steam build's hash
- [x] RTTI and function discovery ported to Rust (0.35 s; jump table bound from the `cmp`)
- [x] x86 → C translator for integer instructions, strings, jumps, calls, jump tables
- [x] C support for the generated code (`runtime/altair.h`, `altair.c`)
- [x] First differential test against Unicorn: `strlen`, `strcmp`, `memcpy` identical
- [x] `altair coverage`: 85.9% of the functions (59.5% of the instructions) translate

### 1.2 Instruction coverage

- [x] `lock` prefix, `xadd`, `cmpxchg`, `bts`/`btr`/`btc`: true atomics on host memory
- [x] Scalar and packed SSE/SSE2, MXCSR: coverage at 94.3% of the functions, 2.9% of the
  instructions remain untranslatable
- [x] 80-bit x87 with stack, comparisons, control word: 99.96% of the functions translate
- [x] `shld`/`shrd`, `rcl`/`rcr`, `rdtsc`, `cpuid`, `pushfd`/`popfd`, `sahf`/`lahf`,
  8- and 16-bit `mul`/`div`, `bsf`/`bsr` (coverage: 89.5% of the functions)
- [ ] Targeted scenarios for the instructions translated but not yet verified (`rcl`, `bt*`,
  `bsf`/`bsr`, `pushfd`/`popfd`, `sahf`/`lahf`, `cmpsb`, `lodsb`)
- [x] MMX (audio mixer) and the last rare instructions: **all 116,077 functions translate**

### 1.3 Verification

- [x] Automatic choice of the functions for the differential test (`altair candidates`) and
  generated inputs (`difftest.py --auto`)
- [x] Comparison of XMM registers, heap, stack and global data; tolerance on NaNs only
- [x] Comparison of the x87 state (top of stack and 80-bit registers)
- [ ] Comparison of the flags; non-empty x87 stack on entry; x87 transcendentals
- [x] Large sample: 4,201 game functions identical to the original out of 8,000, 0 different
- [ ] Structured inputs for the functions skipped today (more than half of the sample)

### 1.4 Whole image

- [ ] Resolve the limits of discovery (`analysis/functions.md`): functions that do not return,
  C++ exception tables
- [ ] Flags passed in tail calls between split functions
- [x] Emission into several C files and parallel compilation (`altair image`): 357 MB of C in 242
  files, 49 seconds with clang `-O1` → `analysis/image.md`
- [x] Differential test on the whole-image library, with clang and with gcc: 9,892
  functions identical out of 19,043 candidates, 0 different
- [x] `.forge` reader in Rust (`crates/forge`): container and LZO1X, identical to `forge.py` on three archives

## Phase 2 · Win32 runtime (gate reached on 2026-11-15)

**Gate:** the recompiled game starts, initialises the CRT and opens the `.forge` files. Status in
`analysis/runtime.md`.

- [x] `crates/runtime` (`altair-run`): image loading, IAT, TEB, static TLS
- [x] Guest memory, heap, `VirtualAlloc`
- [x] Threads, TLS, critical sections, events, semaphores, waits
- [x] Files and paths (game folder read-only, the rest in `run/c/`)
- [x] Registry (empty), INI files (default values only), `USER32` without a screen, time
- [x] CRT startup, static initialisers, `WinMain`, 8 engine threads
- [x] COM objects as vtables of host functions
- [x] Silent DirectSound, DirectInput and XInput without devices, Bink absent
- [x] **Opening of the `.forge` files** (the gate): `DataPC_Common`, `DataPC`, `DataPC_StreamedSounds`
- [x] The game creates the window and calls `Direct3DCreate9`: boundary with Phase 3
- [x] Game language: emulated registry, `Language` = `Italian` (`ALTAIR_LANG`)
- [ ] SEH and C++ exceptions; `WaitForMultipleObjects` with wait-all
- [x] Real INI files (`Assassin.ini`, `DARE.INI` in `run/c/`)
- [x] First page of the guest space protected and handler for invalid accesses

## Phase 3 · Graphics, audio, input

**Gate:** menus and tutorial in the Animus playable.

- [x] `IDirect3D9` and "recorder" device: the game presents 3,000 frames; census
  in `analysis/d3d9.md` (79 methods, shader model 3.0 only, DXT, 54 draws per frame)
- [x] GPU from WSL: wgpu reaches it with the OpenGL backend and `GALLIUM_DRIVER=d3d12`; Vulkan is
  software only (`crates/gpu-probe`, `analysis/d3d9.md`)
- [x] Off-screen triangle on the GPU (PNG verified) and window via WSLg (212 frames in 4 s)
- [ ] Simulated input to get past the title screen and take a census of menus and game
- [x] SM3 → WGSL shader translator (`crates/shader`): 1,488 game shaders translated and validated
- [x] D3D9 on wgpu, first off-screen version (`gfx.rs`): **the game draws the title
  screen**, 74,341 draws without errors → `analysis/d3d9.md`
- [x] Simulated input (`ALTAIR_KEYS`, `ALTAIR_TEXT`): title screen → profiles → profile
  creation (saved to disk) → main menu
- [x] `NEW`: the game loads the first level and **draws the Abstergo lab** (917 draws
  per frame, no errors)
- [x] Interaction in the lab (the game was waiting for a key), Animus screen, entry
  into the memory: **Altaïr in the Animus corridor, with HUD and the "Sistema di controllo" (control system) tutorial**;
  he moves with W (10 million draws without errors) → `analysis/runtime.md`, `run/shots/`
- [x] First three steps of the tutorial passed with simulated keyboard and mouse (first-person view,
  pushing passers-by aside, high profile): synchronisation bar from 1 to 4
- [ ] Complete the Animus tutorial and reach Masyaf: this needs actual playing
- [x] Key names in the tutorial messages: `EnumObjects` in UTF-16, keyboard keys included
- [x] Crowd: translated game almost twice as fast (`-O2`, `restrict`): from 9 to 12 fps
- [x] Submission to the GPU on a separate thread and lighter draw recording: lab
  106 fps, DNA helix 64, crowd 23 (they were 55, 30 and 12) → `analysis/d3d9.md`
- [x] Sampling profiler in the runtime (`ALTAIR_PROFILE=1`)
- [x] Runtime ported to Windows (`host.rs`, `tools/build-win.sh`, `tools/play.ps1`): it compiles
  and links from WSL
- [ ] Native Windows: **first run** (needs the user), then measurements with Vulkan and D3D12
- [ ] Crowd: the per-draw cost remains in WSL's OpenGL-on-D3D12 driver; to be re-measured with
  native Vulkan or D3D12. Levers left: constants bound only once, draw recording
- [x] Control channel to drive the game while it runs (`ALTAIR_CTL`, `tools/start.sh`, `tools/ctl.sh`)
- [x] Real window (`ALTAIR_WINDOW=1`, `tools/play.sh`): it opens via Wayland and shows the game
- [ ] Real keyboard and mouse in the window: the whole path tested except the first stretch
  (winit → state); **a test by a person is missing**
- [x] Stencil, scissor, depth bias, `ColorFill`, resource recycling
- [ ] Partial rectangles in `StretchRect`/`Clear` (never seen so far), `GetRenderTargetData`
- [x] Renderer performance: from 2-25 to 27-190 frames per second depending on the scene
- [x] Real Bink videos (logos and intro video), with **our own decoder**
  (`crates/bink`): 3,381 frames identical to ffmpeg, audio within 4 × 10⁻⁷ → `analysis/bink.md`
- [x] DirectSound on our own mixer: the game plays sound (verified on the saved mix)
- [x] Audio output on PulseAudio in real time (100 s, zero dropped blocks); **to be listened to**.
  During the night the WSLg server had broken down → `analysis/runtime.md`, Audio section
- [x] Input diagnosis (`ALTAIR_INPUT_LOG`) and mouse on the control channel: the view rotates
- [ ] 3D sound, EAX, XInput (controller)
- [ ] Visual comparison with the original DX10, which works on current hardware: needs a
  screenshot from the user of the same scene (Abstergo lab)
- [ ] (optional, only if a rendering difference cannot be explained with the DX10) Get
  the original DX9 to start, which today hangs at startup: try DXVK, dgVoodoo2, EaglePatch,
  compatibility mode, core limit. If it starts: a save and a fixed path through the tutorial
  as a reference, and possibly a trace of the D3D9 calls with apitrace

## Phase 4 · Playable natively (main gate)

**Gate:** complete playthrough, from the start to the end credits, saves included.

- [ ] Recording and replaying of play sessions as regression tests
- [ ] Profiling and fixing of the slow spots (x87, indirect calls)
- [ ] List of known bugs closed

## Phase 5 · Browser

**Gate:** tutorial playable in Chrome.

- [ ] Stack depth measured natively before starting
- [ ] wasm32, guest at offset 0, game in a Web Worker, WebGPU from OffscreenCanvas
- [ ] Data in OPFS (folder selection with the File System Access API), AudioWorklet
- [ ] Module size: streaming and compilation caching

## Phase 6 · Targeted decompilation and mods

**Gate:** first working gameplay mod.

- [ ] Replacement of guest functions by address (mod API)
- [ ] Decompilation of the player controller, missions, crowd, entity system
- [ ] Then the "nice to haves" of the project document, in order of value/cost ratio

---

## Concrete next steps

1. **Phase 3, with a person at the keyboard**: `tools/play.sh`, try keyboard, mouse and audio
   in the window, complete the Animus tutorial and note what goes wrong.
   Then: reliable audio output, cost of draw submission, visual comparison with the DX10.
2. `.forge` reader in Rust: the container is done; objects and classes remain.
3. Open points of Phase 1: targeted scenarios for the unverified instructions, limits of
   discovery.
