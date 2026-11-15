# Tools

All the scripts read the game files and never write into the game's folder.

```sh
export AC1="/mnt/e/Giochi/Steam/steamapps/common/Assassins Creed"
```

## `pe.py`

Static analysis of PE32, standard library only.

```sh
python3 -I tools/pe.py info    "$AC1/AssassinsCreed_Dx9.exe"   # hashes, header, sections, imports, RTTI, build string
python3 -I tools/pe.py imports "$AC1/AssassinsCreed_Dx9.exe"   # list of imported functions per DLL
python3 -I tools/pe.py diff    "$AC1/AssassinsCreed_Dx9.exe" "$AC1/AssassinsCreed_Dx10.exe"
python3 -I tools/pe.py rtti    "$AC1/AssassinsCreed_Dx9.exe"   # decorated RTTI type names
```

`rtti` is a quick string scan (4,376 names in the DX9). For hierarchies and vtables
use `rtti.py`. The output contains Ubisoft class names: if you save it, put it in `private/`.

## `rtti.py`

Parser for 32-bit MSVC RTTI: type descriptors, complete object locators, class
hierarchies, vtables with their slots.

```sh
python3 -I tools/rtti.py "$AC1/AssassinsCreed_Dx9.exe"                               # statistics
python3 -I tools/rtti.py "$AC1/AssassinsCreed_Dx9.exe" --out private/rtti-dx9.json   # detail
```

The JSON has three lists: `types` (address, decorated and readable name), `hierarchies` (bases of
each class with their displacements) and `vtables` (address, class, offset, slots). Results in
`docs/analysis/rtti.md`.

## `imports.py`

Map of the imports: name (ordinals resolved), IAT cell, references in `.text`, category.

```sh
python3 -I tools/imports.py "$AC1/AssassinsCreed_Dx9.exe"          # Markdown tables
python3 -I tools/imports.py "$AC1/AssassinsCreed_Dx9.exe" --guids  # known COM GUIDs present
```

The tables in `docs/analysis/imports.md` come from here.

## `functions.py`

Function discovery without symbols (prototype). It needs Capstone, hence the venv's Python.

```sh
.venv/bin/python tools/functions.py "$AC1/AssassinsCreed_Dx9.exe" --out private/functions-dx9.json
.venv/bin/python tools/functions.py "$AC1/AssassinsCreed_Dx9.exe" --strict   # stricter rule
```

It prints the statistics as JSON; with `--out` it saves the list `[address, origin]`. About 30
seconds. Method, results and limits in `docs/analysis/functions.md`.

## `harness.py`

Runs functions of the original executable in Unicorn: the oracle of the differential tests.

```sh
.venv/bin/python tools/harness.py "$AC1/AssassinsCreed_Dx9.exe" selftest
.venv/bin/python tools/harness.py "$AC1/AssassinsCreed_Dx9.exe" call 0xFA6790 s:Altair
```

Usage from Python, memory map and limits in `docs/analysis/harness.md`.

## `forge.py`

Reader for the `.forge` archives (container version 25), with an LZO1X decompressor in Python.

```sh
python3 -I tools/forge.py summary "$AC1"                                  # one line per archive
python3 -I tools/forge.py digest <archive.forge>   # per entry: id, sections, bytes, CRC-32 (comparison with altair-forge)
python3 -I tools/forge.py list "$AC1/DataPC.forge"                        # id, size, name
python3 -I tools/forge.py sections "$AC1/DataPC.forge" 20                 # compressed sections
python3 -I tools/forge.py head "$AC1/DataPC_Map_Menu.forge" Map_Menu      # hexdump of the index
python3 -I tools/forge.py classes "$AC1/DataPC.forge" private/rtti-dx9.json [--objects]
```

`classes` counts the objects per class, translating the CRC-32s with the names from `rtti.py`. It is slow
on the large archives (almost 3 minutes for `DataPC.forge`). The outputs contain game names and
data: they go in `private/`. Format and results in `docs/analysis/forge.md`.

## `difftest.py`

Differential test: translates with `altair`, compiles with clang, runs each scenario on the original
(Unicorn) and on the translation, compares registers, stack, imports called and memory.

```sh
source ~/.bash_exports && cargo build --release
.venv/bin/python tools/difftest.py "$AC1/AssassinsCreed_Dx9.exe"
```

The scenarios are Python functions at the bottom of the file. Details in `docs/analysis/translator.md`.

## `difftrace.py`

Replays a case of `difftest.py --auto` on the original (with the trace of the addresses
executed) and on the translation, and shows registers and the first differences in heap and stack.

```sh
.venv/bin/python tools/difftrace.py "$AC1/AssassinsCreed_Dx9.exe" 0x99BC40 1
```

## `migrate-folder.sh`

Renames or moves the project folder, taking Claude Code's conversations and memory
along, then recreates the venv and recompiles. It must be launched with the session closed. See `docs/environment.md`.

`difftest.py --funcs 0xA,0xB --cases 40` tests hand-picked functions, even non-candidates.
`difftest.py --auto N --lib build/libaltair_ac1-clang.so` uses the whole-image library.

## `build-image.sh`, `run.sh`

`tools/build-image.sh` regenerates the translated image (`altair image`) and recompiles only the
changed files; it accepts arguments for `make` (e.g. `CC=gcc`). `tools/run.sh [--trace]` starts the
translated game with `altair-run`. See `docs/analysis/runtime.md`.

## `altair-gpu-probe` (crate `crates/gpu-probe`)

Tests of the graphics chain: list of adapters, `triangle FILE.png` (off-screen, with
readback), `window [seconds]` (real window). For the GPU in WSL:
`GALLIUM_DRIVER=d3d12 LD_LIBRARY_PATH=/usr/lib/wsl/lib`. See `docs/analysis/d3d9.md`.

## `altair-shaderc` (crate `crates/shader`)

Translates the game's D3D9 shaders to WGSL and validates them with naga.

```sh
ALTAIR_DUMP_SHADERS=private/shaders ALTAIR_FRAMES=1500 tools/run.sh   # extracts the shaders
target/release/altair-shaderc private/shaders                         # coverage summary
target/release/altair-shaderc private/shaders/vs_….bin                # the WGSL of one shader
```

Renderer variables in `tools/run.sh`: `ALTAIR_SHOT_DIR`, `ALTAIR_SHOT_EVERY`,
`ALTAIR_FRAMES`, `ALTAIR_GFX=0`. See `docs/analysis/d3d9.md`.

## `start.sh`, `ctl.sh`

They drive the translated game while it runs, through the runtime's control channel
(`ALTAIR_CTL`, folder `run/ctl/`, ignored by git).

```sh
tools/start.sh              # starts in the background; log in run/ctl/log.txt
tools/start.sh --game    # in addition: title → profile "Claude" → menu → CONTINUA
tools/start.sh --animus     # in addition: lab → Animus → corridor with Altaïr (5 minutes)
tools/start.sh --tutorial   # in addition: the first three steps of the tutorial
tools/ctl.sh "key 28" "shot a.png"   # Enter key and screenshot in run/ctl/a.png
tools/ctl.sh quit           # census and exit
```

Commands (durations are in frames, or in real time with `ms`: `key 17 3000ms`): `key DIK [duration]`, `text TEXT`, `mouse DX DY`, `button N [frames]`,
`move X Y`, `click X Y`, `shot FILE.png`, `stats`, `profile [reset]`, `fps` (turns the
diagnostics panel on or off, like F8; with the panel on, `shot` includes it), `find TEXT` and `findhex BYTES` (investigation: they search the game's memory
for a UTF-16 text or a byte sequence and print its addresses and surroundings), `speed N` (debug fast-forward of the game clock, like F9; `speed 1` goes back to real time), `hooks` (how many times
each hooked function has been called, see `runtime/hooks.txt`), `quit`. `ctl.sh` waits
until the game has executed them and prints frame, rate and the video in progress. Do not launch `start.sh` under
`timeout`: it would kill the game too. `start.sh` and `play.sh` close at startup every
instance of the game already running: only one runs at a time.

## `play.sh`

Starts the game in a window, with real keyboard, mouse and audio (`ALTAIR_WINDOW=1`).

```sh
tools/play.sh                      # close the window to quit
ALTAIR_AUDIO=0 tools/play.sh       # without audio output
ALTAIR_LANG=English tools/play.sh  # game language (default: Italian)
```

## `audio-probe.py`

Measures whether the audio output consumes sound in real time, outside the game: 4 seconds of a
440 Hz tone written in blocks, with `libpulse-simple` (as the runtime does) or with SDL2.

```sh
python3 -I tools/audio-probe.py all          # all the useful combinations
python3 -I tools/audio-probe.py pulse 10 200   # 10 ms blocks, 200 ms queue
python3 -I tools/audio-probe.py sdl
```

If 4 seconds of sound take much more than 4 seconds, the problem is in the audio server.

## `binktest.py`

Compares the project's Bink decoder (`altair-bink`, crate `crates/bink`) with
`ffmpeg`, which acts as the oracle: every video frame must be byte-for-byte identical; for
the audio the error is measured. The comparison happens in memory, it writes nothing.

```sh
cargo build --release -p altair-bink
python3 -I tools/binktest.py "$AC1/videos"                       # all the videos, video and audio
python3 -I tools/binktest.py "$AC1/videos/UBI_LOGO.bik" --video-only --frames 50
```

## `build-win.sh`, `build-image-win.sh`, `win-link.sh`, `play.ps1`

Native Windows. The first three run in WSL and build for Windows (see `docs/environment.md`):

```sh
tools/build-win.sh     # build/win/altair_ac1.dll + target/x86_64-pc-windows-gnullvm/release/altair-run.exe
```

`build-image-win.sh` compiles the generated C as a DLL, `win-link.sh` is the linker that cargo uses
for the executable. `play.ps1` is launched from PowerShell on Windows:

```powershell
powershell -ExecutionPolicy Bypass -File tools\play.ps1 [-Backend vulkan|dx12] [-SenzaAudio] [-SenzaVideo]
```

It closes the instances already open, starts the game in a window and leaves the log in
`run\win\log.txt`.

## `crop.py`

Crops and enlarges an area of a PNG, to look closely at a detail of a screenshot
(standard library only). Negative X: counted from the right edge.

```sh
python3 -I tools/crop.py run/shots/a.png run/shots/a-zoom.png -310 0 310 110 3
```

## `bench-crowd.sh`

Benchmark of the game thread in the tutorial crowd, the heaviest scene reached so far (about
3,800 draws). Drives the game there with `ALTAIR_BENCH=1` (the runtime throws every batch away
instead of sending it to the GPU, so the frame rate is the cost of the game thread alone:
translated code plus draw recording), prints the busiest threads and three readings.

```sh
tools/bench-crowd.sh "label"
ALTAIR_LIB=build/libaltair_ac1-clang-v3.so tools/bench-crowd.sh "variant"
```
