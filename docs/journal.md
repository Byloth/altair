# Journal

## 2026-11-14

- Read the project document "AC1 Recompiled: piano di progetto".
- Installed the Steam build in `E:\Giochi\Steam\steamapps\common\Assassins Creed`.
  The DX9 hangs at startup; the DX10 works.
- Verified the SHA-1 of the DX9 (`a9ea476e…432120`): it matches the one EaglePatch
  gives for Steam and GOG.
- Compared DX9 and DX10: same changelist 86610, DX9 with 4,043 RTTI names against 937 (count later corrected: 4,376 against 939).
  → `analysis/executables.md`, `decisions.md`.
- Created the repository layout, `tools/pe.py` and the work plan (`PLAN.md`).
- `git init`; cloned scimitar-recomp (`4f36655`) and EaglePatch (`a39dc87`) into `ref/`, ignored by git.
  Correction: no more folders outside the project.
- Read scimitar-recomp's `AC1_WINDOWS.md` and `PLAN.md` → `analysis/scimitar-recomp.md`.
- **Discovery:** scimitar-recomp's Ubisoft Connect build has the same layout as ours
  (entry, sections, TLS `0x1A3561C`, cookie `0x19CA838`, 178 exports). Their addresses hold
  for us → `decisions.md`.
- Collected the DX9 addresses from EaglePatch → `analysis/known-addresses.md`.
- Toolchain to be installed by the user: Rust, clang/lld, build-essential, python3-venv,
  JDK 21, Ghidra 12.2 in `opt/`.
- Checked the environment: clang 18, lld, cmake, Java 21 present; Rust not found in WSL.
  Created `.venv/` (unicorn 2.1.4, capstone 5.0.7). Downloaded Ghidra 12.2 into `opt/`
  (SHA-256 verified). Ghidra 12.2 requires **Java 25**, not 21 → `environment.md`.
- Rust found: 1.100.0 in `~/.local/share/cargo` (PATH in `~/.bash_exports`, not in `~/.cargo`).
  Java 25 still to be installed.
- Installed Java 25 (removed 21). Headless Ghidra starts. Environment for 0.2 complete.
- Completed 0.3: also read `PC_DATA.md`, `ACR_WINDOWS.md`, `ANVIL_FAMILY.md`, parts of
  `METAL.md` and `README.md`, `recomp/README.md`, `platform/README.md`. Added to
  `analysis/scimitar-recomp.md`: AC1's `.forge` format (v25, LZO1X_999), 8 shared D3D9 bugs,
  pitfalls of the Win32 runtime (QPC at 10 MHz, multi-CPU race), map of their Windows layer.
- **0.5 closed.** `tools/rtti.py`: 4,376 type descriptors, 4,597 vtables, 59,899 slots, 28,717
  functions, identical to scimitar-recomp → `analysis/rtti.md`. Fixed the regex of `pe.py rtti`
  (it truncated templates). `_purecall` is at `0xF9F7A2`.
- RTTI of the DX10: no `scimitar` classes, only libraries; 5,059 pairs of DX9 ↔ DX10
  functions from the vtables with the same name → `analysis/rtti.md`.
- **0.6 closed.** `tools/imports.py` → `analysis/imports.md`: 260 imports, all referenced;
  `TlsGetValue` has 2,644 references, `DebugBreak` 144 (asserts). COM GUIDs of DirectInput 8,
  DirectSound 8/3D and EAX 2–4 found in `.rdata`. COM methods deferred to the runtime.
- **0.7 closed (prototype).** `tools/functions.py`: 115,807 starts, 3.19 M instructions
  (96,781 with `--strict`; scimitar-recomp 98,470). From the entry, direct calls reach
  only 7,216 functions: the rest goes through vtables and pointers. 3.7 MB of `.text`
  are unreferenced code → `analysis/functions.md`. Import into Ghidra deferred.
- **0.8 closed.** `tools/harness.py`: the original's `strlen`, `strcmp`, `memcpy` run in
  Unicorn and verified; `fs:` set through the GDT (32-bit Unicorn does not accept FS_BASE).
  Phase 0 gate satisfied → `analysis/harness.md`.
- Moved the old 0.9 (original DX9 on Windows) to Phase 3 as an optional task → `decisions.md`.
- **0.10 closed, Phase 0 closed.** `tools/forge.py` reads the 18 archives (15,906 entries). Every
  object carries the (standard) CRC-32 of the name of its class: cross-checked with the RTTI, 100%
  of the objects are recognised. The missions are `MissionItem*` objects in the data (27 classes,
  about 950 objects in `DataPC.forge`) → `analysis/forge.md`.
- **Phase 1 opened.** Rust workspace (`crates/pe`, `crates/recomp`), binary `altair` with
  `info`, `rtti`, `discover`, `translate`, `coverage`. Discovery in 0.35 s: 116,090 functions.
- x86 → C translator for the integer instructions and C support in `runtime/`. 85.9% of
  the functions translate (59.5% of the instructions); `lock`, SSE, x87 are missing.
- `tools/difftest.py`: translated `strlen`, `strcmp`, `memcpy` identical to the original in Unicorn
  (1,717 calls). The test found three forms of `memcpy` jump table that were not recognised,
  now handled → `analysis/translator.md`.
- `altair candidates` + `difftest.py --auto`: differential test on functions chosen by the
  recompiler, with generated inputs. The first 6 differences were memory left over between
  cases, not bugs: now every case restarts from a restored heap (1 MB), stack and global data.
- Added atomics (`lock`, `xchg` with memory), `xadd`, `cmpxchg`, `shld`/`shrd`,
  `rcl`/`rcr`, `bts`/`btr`/`btc`, `bsf`/`bsr`, `pushfd`/`popfd`, `sahf`/`lahf`, 8- and
  16-bit `mul`/`div`, `rdtsc`, `cpuid`. Coverage: 89.5% of the functions, 65.1% of the instructions.
- Sample of 4,000 functions: **1,753 identical to the original, 0 different**, 2,247 skipped.
  Only SSE and x87 remain → `analysis/translator.md`.

## 2026-11-15

- `altair census`: instructions by type, with how many are untranslatable.
- **SSE/SSE2** in the translator (`runtime/altair_sse.h`, in portable C) and XMM state in the context.
  Coverage: 94.3% of the functions; only 2.9% of the instructions untranslatable (x87, MMX).
- The test also compares the XMM registers, with float inputs (signed zeros, infinities, NaNs,
  denormals). Sample of 6,000 functions: **3,042 identical, 0 different**.
- **Found a bug in clang 18.1.3** on writes to partial registers; reproduced in
  `tests/compiler/clang18-partial-register.c` and worked around with narrow writes.
- NaN differences between hardware and emulator (x87 rule applied to SSE): tolerated and counted.
- The emulator was carrying the direction flag and MXCSR over from one case to the next: fixed
  in the harness → `analysis/translator.md`, `analysis/harness.md`.
- **Renamed the technical project from `acrek` to `altair`** (binary, crates, runtime files and
  prefixes, documents). The earlier journal entries have been updated to the new name.
  Tests rerun after the change: selftest and scenarios OK, 820 functions identical out of 1,500.
  The folder has to be renamed by the user with the session closed → `environment.md`.
- Extended project title changed to "AC1 Recompiled - Altaïr" (repository and online document) → `decisions.md`.
- **x87** in the translator (`runtime/altair_x87.h`): 80-bit `long double` registers, host
  control word aligned with the guest's. Coverage: **116,048 functions out of 116,090
  (99.96%)**, 336 instructions out of 3.2 million untranslatable.
- The test also compares the x87 state. Sample of 8,000 functions: **4,201 identical, 0
  different** (6 up to a NaN). Fixed one case: code that after a `call` continues inside
  another function (CRT math routines) → `analysis/translator.md`, `decisions.md`.
- **The last 42 functions.** 17 were data (switch index tables, round constants):
  discovery now discards them. 25 were real code: MMX in the audio mixers, segment registers,
  `xlatb`, `fnstenv`/`fnsave`, `bts` on a bit string and a few others. **All 116,077
  functions now translate.** Verification of the 25: 17 identical, 7 skipped, 1 different only in the
  unmodelled fields of `fnstenv`. Sample of 8,000: 4,199 identical, 0 different.
  `altair coverage --list`, `difftest.py --funcs` → `analysis/translator.md`.
- **`altair image`: the whole image.** 116,077 functions in 242 C files (one per 64 KB window
  of addresses, 357 MB), generated in 3 seconds. Compilation: 49 s with clang `-O1`, 55 s
  with `-O2`, 50 s with gcc. Test on the whole library: 9,892 functions identical out of 19,043
  candidates, 0 different, with both compilers. **Phase 1 gate reached** →
  `analysis/image.md`. (First attempt compiled with gcc by mistake: `CC ?=` in the Makefile
  does not override make's default `cc`.)
- **Phase 2 opened: `altair-run`.** Win32 runtime in Rust (`crates/runtime`), 170 shims. On the first
  run the translated game executed the entire CRT startup (588 calls to imports), stopping
  at `CreateEventA`. With threads, events, files, registry, INI and a window without a screen
  it gets to: static initialisers, `WinMain`, 8 engine threads (`Loading`,
  `TaskThread1`…), audio initialisation.
- It stops at the audio: without a device the game shows "Cannot initialize Sound Driver" and
  calls a null object. A silent DirectSound is needed. No `.forge` opened so far.
- The runtime found an undiscovered function (`0xFB2451`, unaligned table in `.data`):
  discovery now sees it, +65 functions. Incremental recompilation: 26 files, 10 seconds.
  Differential test after the change: 2,110 identical out of 4,000, 0 different → `analysis/runtime.md`.
- **COM objects** (`com.rs`) and on top of them a **silent DirectSound**, DirectInput and XInput without
  devices, Bink absent. The game gets past the audio and **opens the `.forge` files: Phase 2 gate**.
- Two hangs solved with the trace: overlapped reads (completion event never
  signalled) and `VirtualQuery` (fixed answer → the engine picked the wrong allocator and called a
  null object). New diagnostic: `altair_trail`, the last indirect destinations.
- The game now creates the "Assassin's Creed" window and calls **`Direct3DCreate9`**: it stops there,
  three runs out of three. That is the boundary with Phase 3. Differential test: 1,616 identical out of 3,000,
  0 different → `analysis/runtime.md`.
- **Recorder Direct3D 9** (`d3d9.rs`): 17 + 119 methods declared, plus resources and the D3DX
  constant table. **The game runs in its main loop and presents 3,000 frames**
  (54 draws each) without stopping: it loads the loading room and the menu world, skips the videos.
  Census in `analysis/d3d9.md`: 79 methods used, 3,412 shaders all SM 3.0, DXT textures.
- Five undiscovered functions surfaced while running, and discovery improved accordingly
  (unaligned runs, safety net on `call`s, `ret`-only callbacks, false starts from
  immediates, `pushad`): 117,159 functions. `call_guest` for callbacks into the game.
- GPU: `/dev/dxg` present, RTX 4070 Ti visible from WSL; wgpu still to be tried.
- Differential test: 3,084 identical out of 6,000, 0 different.
- GPU in WSL: wgpu sees the RTX 4070 Ti through OpenGL on D3D12 (`GALLIUM_DRIVER=d3d12`); Vulkan software only. No installation needed. `crates/gpu-probe` → `analysis/d3d9.md`, `environment.md`.
- Triangle with wgpu on the RTX 4070 Ti from WSL: off-screen (PNG checked) and in a window via WSLg (212 frames in 4 s). Core dump when the window closes, to be fixed.
- **SM3 → WGSL shader translator** (`crates/shader`): all 1,488 distinct shaders extracted
  from the game translate and pass naga, at the first attempt.
- **Renderer on wgpu** (`gfx.rs`), off-screen on the RTX 4070 Ti. **The game draws the
  Animus loading screen and the title screen** ("Assassin's Creed", "Press
  Any Key To Continue"): 74,341 draws in 1,500 frames, no validation errors.
  First round: errors on vertex buffer slots without attributes and on 3D textures, fixed.
  A light-blue box remains at the top left → `analysis/d3d9.md`.
- **Viewport** fixed (reported by the user): the background of the title screen is the
  Animus fog, not white.
- **Simulated input**: `ALTAIR_KEYS` (DirectInput) and `ALTAIR_TEXT` (window messages,
  with `DispatchMessageA` calling back the game's procedure, and `ToUnicode`). The game goes
  from the title screen to the profile list, **creates the profile "Matteo"** writing
  `.hdr` and `.sav` in `run/c/`, and reaches the **main menu** (NEW, CONTINUE, OPTIONS).
  On the next run it finds the profile again → `analysis/runtime.md`, `analysis/d3d9.md`.
- Profile "Claude" created next to "Matteo"; list navigation with the arrow keys (they are needed
  as `WM_KEYDOWN` too).
- `NEW`: loading stopped at a function hidden by a false start (`0x5830F0`).
  Discovery in three passes and `ALTAIR_WHY` diagnostic. 117,106 functions; differential test
  2,081 identical out of 4,000, 0 different.
- **First 3D scene: the Abstergo lab**, with Vidic, Desmond and Lucy. 917 draws per
  frame, 4.7 million draws without errors after adding the blend constant
  → `analysis/d3d9.md`, `analysis/runtime.md`.

## 2026-11-15, night (autonomous work)

The user's brief: go ahead alone, in the order game → renderer → audio/video → window;
Italian language; test windows allowed; at the end of the work update the online plan.

- **Language**: the game reads it from the registry, `HKLM\SOFTWARE\Ubisoft\Assassin's Creed`,
  value `Language` (string: `English`, `Italian`, …; written by the Steam installer,
  see `installscript.vdf`). Emulated registry with that single key (`ALTAIR_LANG`,
  default `Italian`). The game opens `DataPC_StreamedSoundsIta.forge` and the menus are in
  Italian → `analysis/runtime.md`.
- **Control channel** `ALTAIR_CTL` (`tools/start.sh`, `tools/ctl.sh`): keys, text,
  screenshots and quitting while the game runs. It replaces the frame-based scripts, fragile because
  the game reasons in real time.
- `CONTINUA` (continue) loads the save of the "Claude" profile and the **lab scene proceeds**:
  Vidic walks and talks, "PREMERE QUALUNQUE PULSANTE PER INTERAGIRE" (press any button to interact) appears.
- **Renderer**: stencil (two-sided too), scissor, depth bias. `StretchRect` with partial
  rectangles and `ColorFill` are counted in the final census → `analysis/d3d9.md`.
- **Real audio**: DirectSound on our own mixer (48 kHz stereo, thread every 10 ms) with output
  on PulseAudio through `libpulse-simple` loaded with `dlopen` (no installation). The
  game's buffers are 38,400-byte streams at 48 kHz that DARE fills continuously. The
  mix is not silent (peaks at ~15,000 out of 32,767, `ALTAIR_AUDIO_DUMP`). Without an open
  window WSLg's PulseAudio server consumes in fits and starts: the output sits behind a queue that
  drops, so the game does not slow down → `analysis/runtime.md`.
- A mistake of mine to remember: `timeout` kills its whole process group, including the game
  launched in the background by the script; `start.sh` now uses `setsid`.
- **The game was waiting for a key**: "PREMERE QUALUNQUE PULSANTE PER INTERAGIRE" in the lab
  is not optional. With Space the scene continues and the **Animus screen** arrives
  ("CRONOLOGIA MEMORIA GENETICA", the DNA helix, "Ricordo bloccato" (memory locked), the faces of Lucy and Vidic
  seen from the bed).
- **Renderer performance.** In the Animus the game dropped to 2 frames per second. Three
  causes, all ours, found with the new statistics (`stats` on the control channel,
  `ALTAIR_STATS`): every rewritten dynamic buffer forced a submission to the GPU; every rewrite
  re-uploaded the whole buffer (43 MB per frame); every draw created two bind groups and
  rewrote 9 KB of constants. Now: `Lock` range and `NOOVERWRITE`/`DISCARD` flags
  honoured, bind groups reused, constants written only if changed. Menus from ~25 to 130-250
  frames per second → `analysis/d3d9.md`.
- Full-surface `ColorFill` executed (85 calls in the session: black and `0xff203050`);
  no `StretchRect` with partial rectangles seen so far.
- **Real Bink videos** (`bink.rs`): decoded by `ffmpeg` (already present in WSL) as a process,
  read from a pipe; the audio goes into the mixer. The game uses 32-bit `BinkCopyToBuffer`, 15 functions
  of `binkw32.dll` in all. You can see the Ubisoft logo, the warnings and the intro video (the eagle);
  Enter skips it → `analysis/runtime.md`.
- **Real window** (`window.rs`, `ALTAIR_WINDOW=1`): winit on the main thread, wgpu
  surface, real keyboard and mouse towards DirectInput and the window messages.
- **Altaïr.** From the Animus screen the game loads `DataPC_Memory_Corridor.forge` and
  shows Altaïr in the Animus corridor, with the synchronisation bar and the tutorial
  ("SISTEMA DI CONTROLLO", "BARRA DI SINCRONIA"); with W he walks. 11.5 million draws in the
  longest session, no draws skipped, no errors → `analysis/runtime.md`.
- Renderer, second round: writes of constants and staging data grouped into two per
  submission. Lab 22 → 45 fps, corridor 70-117 fps, DNA helix 2 → 27 fps. Per-phase
  measurements in `analysis/d3d9.md`.
- **Resource recycling**: `Release` to zero + freeing at the end of the frame if the device no
  longer uses them. In one session: 25,967 resources freed, 760 live wgpu textures instead of 1,231,
  image unchanged.
- **Window**: it opens via Wayland and shows the game (no more than ~60 fps: `Present` waits
  for the screen even when asked not to). On X11 winit is missing `libxkbcommon-x11.so`: in
  that case we continue off-screen. `EnumObjects` of the mouse (3 axes, 8 buttons), F10 to
  capture the pointer, `tools/play.sh`. **Real keyboard and mouse not tested**: there is no
  way to inject keys into a Wayland window from here.
- **Audio, open problem**: the mixer works (mix not silent for 400 s), but the output on
  WSLg's PulseAudio accepts a fifth of real time, and at some point the server stopped
  responding to other programs as well (error 8, timeout). I do not know whether I broke it
  with my tests or whether it was already like that. From Windows, `wsl --shutdown` restarts it → `analysis/runtime.md`.
- Mistakes of mine to remember: `pkill -f text` and `pgrep -f text` also find the shell that
  launches them, if the text appears on its command line (I killed my own command twice):
  use `"altair-ru[n]"`. Long foreground `sleep`s are blocked: use an `until` loop.

## 2026-11-15, morning

Plan approved by the user: close the Phase 3 gate (real input, audio, tutorial), then
renderer for the cities and robustness.

- **Verifiable input**: `ALTAIR_INPUT_LOG=1` and the commands `mouse`, `button`, `move`, `click` on the
  control channel, which go through the same state as the window. In the Animus corridor
  the simulated mouse rotates the view: mouse DirectInput works. `tools/start.sh --animus`
  gets there on its own.
- **Buffers uploaded by the range used** by the draw: DNA helix from 20 MB to 0.3 MB per
  frame.
- **Audio**: the PulseAudio server recovered on its own. With the 200 ms queue the game plays in
  real time: 100 s, zero dropped blocks. `tools/audio-probe.py` to measure outside the
  game (SDL2 too, an alternative ready to use). The hypothesis about the night-time breakdown (60 ms queue) remains
  unproven → `analysis/runtime.md`.
- **Robustness**: first guest page protected and `SIGSEGV` handler with diagnosis; path
  up to the corridor without invalid accesses. **Real INI files**: the game writes
  `Assassin.ini` (last profile) and `DARE.INI` → `analysis/runtime.md`.
- Six minutes of play with the audio on: zero dropped blocks.
- **Tutorial**: three steps passed with simulated commands (E for the first-person view, Shift + W
  among the monks up to the marker, right button for high profile). Synchronisation bar at 4.
  Defects: crowd at 9 fps, empty key name in the messages → `analysis/runtime.md`.

## 2026-11-15, afternoon

Second plan approved: key names, crowd, tutorial and saves, window, Phase 1
debts, `.forge` reader, and **our own Bink decoder** in place of `ffmpeg`
(`~/.claude/plans/eager-plotting-clarke.md`).

- **Key names**: the cause was not the one hypothesised. The game uses `IDirectInput8W`; the
  names go in UTF-16. Read the callback `0x009091C0` in the generated C to understand it. Now
  "(Mouse destro)" (right mouse) appears in the tutorial message → `analysis/runtime.md`.
- **Crowd**: measured who pays. Half was the translated game: with `-O2` and `Ctx *restrict` it runs
  almost twice as fast (59 → 31 ms), differential test green, now the defaults. The other half is
  Mesa's OpenGL submission on D3D12: it does not budge either by removing repeated states or with
  `mesa_glthread`. 9 → 12 fps in the crowd, 45 → 55 in the lab → `analysis/d3d9.md`.
  Target of 25 fps not reached in WSL.
- **Tutorial**: `tools/start.sh --tutorial` redoes the three known steps on its own, with real-time
  durations (`key 17 3000ms`). Fourth step (grab and throw) not passed: it requires aiming, and
  I cannot do that from screenshots. The `.sav` does not change during the tutorial: no checkpoint to
  verify before the end of the block. Steps 3 and 4 of the plan stop here, waiting for a
  person.
- **Window and 60 fps**: the surface offers only `Fifo`. A limit of OpenGL on Wayland in WSLg;
  documented, no solution on our side → `analysis/runtime.md`.
- **Phase 1 debts**: the only item noted was the x87 transcendentals; Unicorn is not a
  valid oracle for those (hardware is needed) → `open-questions.md`. Nothing else to do now.
- **`.forge` reader in Rust** (`crates/forge`, `altair-forge`): v25 container and LZO1X.
  `list` and `digest` identical to `forge.py` on three archives, 429 entries → `analysis/forge.md`.
- **Our own Bink decoder** (`crates/bink`, `altair-bink`, `tools/binktest.py`). Read
  FFmpeg's container, video, transform and audio as a specification; written in Rust without
  dependencies. Video: the first two videos identical at the first attempt, then all 3,381
  frames. Audio: only the 2/N factor of the DCT was missing, found by comparing; then maximum
  error 4 × 10⁻⁷ on eight tracks. Integrated into the runtime: the game shows logos and intro without
  `ffmpeg` (`ALTAIR_BINK=ffmpeg` to go back) → `analysis/bink.md`, `decisions.md`.

## 2026-11-15, evening: optimising rendering

Request from the user while trying `tools/play.sh`.

- `start.sh` was closing any instance of the game: now only its own (`run/ctl/pid`).
- **Sampling profiler** in the runtime (`profile.rs`): 42% of the rendering thread is
  inside the OpenGL draw call, 30% at one spot in the NVIDIA driver; GPU at 40%.
- Four hypotheses tested with measurements, three without effect (bytes of constants, rotating
  per-frame buffers, vertex buffer in use) and one partial (switching the constants
  buffer: a quarter of the submission).
- **Submit thread**: the game no longer waits for the driver. Lab 59 → 106 fps, helix
  30 → 65, crowd 12 → 20. To get there: a gate between the threads (wgpu on OpenGL errors out
  or hangs if one thread creates resources while the other uses the queue) and a pool of reused
  buffers → `analysis/d3d9.md`.
- Lighter draw recording (crowd: 17 → 11 ms on the game thread) and vertex and pixel
  shader constants in separate groups (no effect on submission). Crowd at 23 fps.
- At the user's direction `start.sh` goes back to closing all instances at startup, and
  `play.sh` does so too: only one instance at a time.
- `start.sh` and `play.sh`: only one instance at a time (the user's direction).
- **Port to Windows.** In WSL there is no direct Vulkan (the GPU is exposed only as D3D12),
  so the route without a double hop is Windows. The user installed Rust and LLVM-MinGW on
  Windows. From WSL I cannot run them (interoperability off in their `wsl.conf`): I compile
  for Windows from here, with the Linux clang and the LLVM-MinGW libraries read from `/mnt/c`.
  - Generated C → `build/win/altair_ac1.dll` in 65 s (`tools/build-image-win.sh`);
    `runtime/altair.c` with `VirtualAlloc`, `QueryPerformanceCounter`, intrinsic jumps.
  - Runtime → `altair-run.exe` with the target `x86_64-pc-windows-gnullvm`. `rust-std` in
    `opt/rust-win/` and a sysroot of links:
    `ln -s <Linux toolchain>/lib/* …`, `ln -s <extracted rust-std>/lib/rustlib/x86_64-pc-windows-gnullvm …`.
    Two stumbles at link time: the Linux clang added `-lgcc` (`-nodefaultlibs` is
    needed) and the executable looked for `libunwind.dll` (now static).
  - `host.rs`: the Linux/Windows layer. Audio on `waveOut`.
  - `tools/play.ps1` to launch it. **Never run**: I am waiting for the user's test and the
    log in `run\win\log.txt` → `analysis/runtime.md`, `environment.md`.
- **First run on Windows** (user): everything starts at the first attempt (DLL, 4 GB of
  memory, `waveOut` audio, video, Vulkan on the RTX 4070 Ti, 180-195 fps on the logos), but the
  window stayed white: `create_surface` from the game thread failed with "the underlying
  handle is not available". On Windows the window handle can only be obtained from the thread that
  created it. Now the wgpu instance and the surface are created in `window.rs`, on the window
  thread, and the renderer receives them (`take_graphics`). Verified on Linux that the window
  still works; to be retried on Windows.
- **Second test on Windows** (user): the window works, it gets into the game. Vulkan: the
  submit thread costs 0.5-1.5 ms against the 9-38 of WSL. Reported: a crackling sound on top of
  the right one (menu and game, not in the videos), striped shadows on lit surfaces,
  camera too close; F8 requested for frames per second.
  - Shadows: `R32F` shadow maps created at 16 bits → now at 32 bits where the card filters them.
  - Sound: the game's audio thread has 60 ms to fill each segment and was waiting for the
    runtime's global lock; 123 delays per minute. Buffer methods and clock outside the lock,
    no jerky catch-up in the mixer, `timeBeginPeriod(1)` on Windows. Delay counter
    in the mixer, `ALTAIR_AUDIO_LOG` → `analysis/runtime.md`.
  - F8: frames per second in the window title.
  - Camera: not understood yet, details are needed.
- **Third test on Windows** (user): the sound still crackles, but only menu music and
  voices. Decisive clue: those are the Ogg Vorbis streams. Traced the chain back from the DirectSound buffer
  to `mdct_init`: the translated CRT's `floor(11.5)` returned 12, because `frndint` was translated with
  `rintl` and clang reduces it to `rint`, which ignores the x87 rounding. MDCT table
  wrong for the 2048 blocks → reads outside the buffer. Fixed `frndint` and `fistp`
  (`x87_rnd`) and the ID bit of EFLAGS (the CRT did not see SSE2). `floor` and `cpuid` scenarios in the
  differential test. Verified on the menu stream → `analysis/runtime.md`,
  `analysis/translator.md`. Linux image and Windows binaries recompiled.
- Reconnaissance of the development leftovers in the binary (the user's request, for the FPS
  indicator): there is an interpreter for 60 command-line options, `/nofps` included, but it is orphan
  code and the indicator's text is missing → `analysis/hidden-features.md`.
- FPS indicator: looked again for the original switch, no reader → made our own.
  `overlay.rs`: an overlay panel in Animus style (animated helix, numbers, graph of
  frame times), F8 / `fps` / `ALTAIR_FPS=1`; the numbers are gone from the window title. Verified on
  WSL with a screenshot.
- Startup videos: logo, "presenta" (presents) and warnings now go by with a key or a click
  (the logo in one second, the two fixed screens in half the time: the game counts them at 30 per
  second) → `analysis/runtime.md`.
- Startup videos, second version (the user: the warnings could not be skipped and the logo
  sped up): found the table of the `StartupVideoManager` (`0x18C2C08`) with the "can be
  skipped" byte; the runtime sets it to 1 for logo, "presenta" and warnings and the game skips them by
  itself with its own fade. Removed the fast-forward of `BinkWait` → `analysis/runtime.md`.
- F8 panel: the history was one bar per frame with scale and red relative to the
  average, unreadable (user). Now one bar every quarter of a second, scale in fixed steps,
  absolute thresholds for red → `analysis/runtime.md`.
- F8 panel with the Animus fonts: deciphered `MagmaMftFile` (glyph metrics) and the
  DXT3 `TextureMap` of the fonts; `animus_font.rs` reads them at startup from the user's
  `DataPC.forge` (nothing in the repository), falling back on the bitmap font → `analysis/forge.md`.
- Animus version ("1.28" → `2.0.0-dev.1`): found the text (`Animus_MGB`), no `α`
  in the fonts, and in memory there is no room for a longer text. A hook in the
  translator is needed → `open-questions.md`. Commands `find`/`findhex`, `tools/crop.py`.
- Wrong camera in the lab: gone after today's x87/EFLAGS fixes
  (user on Windows, and eight clicks here on WSL). Glitches and the special
  shot remain to be retried → `open-questions.md`.
- **Hooks** in the translator (`--hooks runtime/hooks.txt`, `altair_hook`, `hooks.rs`): a
  game function can be observed or replaced by the runtime → `analysis/translator.md`.
- Animus version: with a hook on `0x8A1920` (construction of the interface
  strings) the game now shows "Versione Animus 2.0.0-dev.1" → `analysis/runtime.md`.
- Icon: `altair icon` extracts it from the original executable, `build-win.sh` links it in.
- Glitches in the lab: the effect runs only at the awakening; the glitch clips
  are never constructed. Confirmation on the original is needed → `open-questions.md`.
- Reported by the user, to be tackled: tutorial crowd at 21-23 frames per second on
  Windows.
- **x87 rounding without library calls.** Profiling the tutorial crowd showed 27% of the
  render thread inside libm's `truncl`: the rounding fix of this morning (`x87_rnd`) had
  turned every `fistp` under the CRT's truncating control word into a library call (crowd on
  WSL down from 23 to 17 fps). The conversions now go through an inline 64-bit integer
  conversion (`x87_trunc` in `runtime/altair_x87.h`); differential test green. Re-measured
  on WSL: 24.0 fps at 3,791 draws.
- **The repository is now in English** (the user's request: it is public on GitHub). Docs,
  comments, log and CLI messages translated; Italian file names renamed (`docs/analysis/`,
  `PLAN.md`, `journal.md`, `decisions.md`, `open-questions.md`, `sources.md`,
  `environment.md`, `tools/start.sh`, `play.sh`, `play.ps1`, `crop.py`); options renamed
  (`start.sh --game`, `binktest.py --frames/--video-only/--audio-only`, `play.ps1 -Game
  -NoAudio -NoVideo`); log tags `[window]`, `[profile]`, `[net]`, `[overlay]`, `[hook]`. Still
  Italian on purpose: text the game itself shows (key names handed to the game, quoted
  dialogue). Commit history rewritten in English, Conventional Commits, same dates.
- **Debug fast-forward.** The user's idea, to avoid waiting through the lab scene: the game
  clock can run faster (`clock.rs`; F9 toggles 8x, `speed N`, `ALTAIR_SPEED`). The lab scene
  takes 16 seconds instead of about 100; `tools/start.sh` uses it → `analysis/runtime.md`.
- **Rendering performance, first round on the game thread** (the user: 21-23 fps in the
  tutorial crowd on Windows, where the driver is fast and the game thread is the limit).
  `ALTAIR_BENCH=1` and `tools/bench-crowd.sh` measure that thread alone: 25.5 fps, one thread
  at 100%, 26.8 ms of translated code and 12.4 ms in the runtime per frame; the profile is
  flat. Hash table for indirect calls (+4%); guest integer registers and memory base in
  locals of each generated function (+7%, the function looked at went from 591 to 488 host
  instructions). 28.4 fps now. Next: XMM and x87 state in locals, draw recording →
  `analysis/translator.md`.
