# The Win32 runtime (`altair-run`)

Status as of 2026-11-15, first day of Phase 2. Code in `crates/runtime/`.

```sh
tools/build-image.sh          # regenerates the translated image and recompiles only what changed (about 10 s)
tools/run.sh                  # starts the translated game
tools/run.sh --trace          # with every call to an import (or ALTAIR_TRACE=1)
```

## How it works

`altair-run` is an ordinary Linux program:

1. it checks the SHA-1 of the executable, reads it and copies its sections into a 4 GB guest
   space, at their original addresses;
2. it loads with `dlopen` the library produced by `altair image` (`build/libaltair_ac1-clang.so`);
3. it fills every IAT cell with the address of a trampoline (`0x7F000000 + 16·n`);
4. it sets up the TEB, PEB, static TLS and stack of the main thread;
5. it calls the translated entry point (`0x00F9F445`) on a host thread with a 1 GB stack.

When the translated code calls an address that is not a translated function, the library
hands over to `host_call`, which looks up the **shim** for that import: a Rust function that reads the
arguments from the guest stack and writes the result into `eax`. If the shim does not exist the game
stops, saying which import is needed and where it was called from. This is the work loop of the phase.

| Guest region | Use |
|---|---|
| `0x00100000`–`0x003F0000` | main thread stack |
| `0x00400000`… | the image |
| `0x10000000`–`0x40000000` | heap (`HeapAlloc`) |
| `0x40000000`–`0x70000000` | `VirtualAlloc`, stacks and TEBs of the other threads |
| `0x7F000000`… | import trampolines |
| `0x7FFD0000`, `0x7FFDF000` | main thread TEB, PEB |

| File | Contents |
|---|---|
| `main.rs` | loading and startup |
| `machine.rs` | `Ctx` (identical to `runtime/altair.h`), guest memory access, heap |
| `win32.rs` | shim mechanism; KERNEL32: process, memory, TLS, critical sections, waits, threads |
| `sync.rs` | events, semaphores, threads as waitable objects |
| `fs.rs` | Windows paths and file APIs |
| `com.rs` | COM objects: vtables in guest memory whose slots are host functions |
| `dsound.rs` | silent DirectSound 8 |
| `input.rs` | DirectInput 8 and XInput with no devices |
| `d3d9.rs` | "recorder" Direct3D 9: it counts and does not draw (see `d3d9.md`) |
| `system.rs` | registry, INI files, date and time, network, Bink, random numbers |
| `user32.rs` | a window with no screen |

Choices made:

- **Two kinds of shim.** `L` receives the runtime under a global lock; `F` does not take it:
  it is for the hot paths (`TlsGetValue`, critical sections, `Interlocked*`) and for anything that may
  block waiting (`WaitForSingleObject`).
- **Critical sections with no state on the host**: the lock is the `OwningThread` field of the
  guest structure, taken with a compare-and-swap, waiting by spinning and then with short pauses.
- **Guest threads = host threads**, each with its own context, guest stack, TEB and static TLS.
- **Heap** with the metadata on the host side: an overflow in the game does not corrupt them.
- **File system**: `C:\AC1\…` is the game folder, **read-only** (a write there
  is refused and reported). The rest of `C:\` lives in `run/c/` in the project, ignored by
  git. Names are case-insensitive.
- **Declared Windows version**: XP SP3 (5.1.2600), one processor.
- **COM objects**: `[vtable][references][data]` in guest memory; every interface declares
  all its methods with their argument count, and the unimplemented ones stop the game
  saying which one is needed ("MISSING COM METHOD: IDirectSoundBuffer8::SetFX").
- **Audio**: a DirectSound that accepts everything and plays nothing. The buffers exist in guest memory
  and the play cursor advances with the clock, so the game's mixer keeps pace.
  EAX is reported as absent.
- **Input**: keyboard and mouse exist but are never touched; no controller.
- **Video**: `BinkOpen` fails, the videos are skipped.
- **Virtual memory**: `VirtualAlloc` regions are tracked, because the engine asks
  `VirtualQuery` how large a region is before freeing it.
- **Overlapped reads**: performed immediately; the completion event is signalled.
- **Exceptions**: no SEH. `RaiseException` handles only the exception MSVC uses to name
  threads; any other one stops everything.

## Where it gets to today

With 234 import shims and 87 COM methods, without any change to the translated code:

1. **The CRT startup**, complete: heap, TLS, environment, locale, command line.
2. **All the static initializers** of the engine.
3. **`WinMain`**: Vista's Game Explorer (absent), keyboard hook, special folders.
4. **The engine threads**: `Loading`, `TaskThread1`, `Gear::AsynchDevice` and others, 8 in all.
5. **Audio**: `EAXDirectSoundCreate8` fails, `DirectSoundCreate8` succeeds; the mixer
   thread runs and writes into the buffers (about 90 `Lock`/`Unlock` per second).
6. **The `.forge` files**: it opens `DataPC_Common.forge`, `DataPC.forge` and `DataPC_StreamedSounds.forge`
   with overlapped reads, and reads them from the loading thread. **This is the gate of the phase.**
7. **Input**: it creates DirectInput, mouse and keyboard, and polls them from the main loop.
8. **The window**: it registers the class `ScimitarEngineWindowClass` (procedure `0x004013E3`) and
   creates the "Assassin's Creed" 1280x720 window, from an engine thread.
9. **Direct3D**: it creates the device (1280x720, D24S8), 3,412 shaders, textures and render targets.
10. **The main loop**: it loads the loading room and the menu world, skips the
    opening videos and **presents frames**: 3,000 in a row, with about 54 draws each,
    without stopping. The audio mixer thread uses the translated MMX code.

The game no longer stops on its own: it runs until it is interrupted (`ALTAIR_FRAMES=N`).
It draws nothing: Direct3D is a recorder.

Things noticed along the way:
- The game looks for `DataPC_StreamedSoundsEng.forge`, which does not exist in this installation
  (there is only the Italian one): we declare the English language. To be fixed with the real language.
- It opens `videos\LoadingScreen.bik` directly as a file, as well as through Bink.
- Without an audio device the game shows "Cannot initialize Sound Driver" and then calls a
  null object: the original does not start without a sound card.

## Simulated input

Until there is a real window, keys come from a script tied to frames:

```sh
ALTAIR_KEYS="900:28,1500:28,2300:28"   # frame:DIK code[:duration in frames]
ALTAIR_TEXT="2000:Desmond"              # frame:text, one character every 12 frames
```

The game reads the keyboard in two different ways, both served:

- **Navigation** (Enter, arrows, "press a key"): DirectInput immediate state,
  `GetDeviceState` on the keyboard, 256 bytes indexed by DIK codes (28 Enter, 1 Esc,
  57 Space, 200/208/203/205 arrows).
- **Text** (profile name): window messages. The thread that owns the window
  calls `PeekMessageA` every frame, then `TranslateMessage` and `DispatchMessageA`; the
  window procedure (`0x004013E3`, game code invoked with `call_guest`) on
  `WM_KEYDOWN` asks for the character with `GetKeyboardState` and `ToUnicode`. `WM_CHAR` is not needed.

With the script above the game **creates a profile**: it writes
`C:\Users\Player\AppData\Roaming\Ubisoft\Assassin's Creed\Saved Games\<number>.hdr` and
`.sav`, that is `run/c/Users/Player/…` in the project, and on the next run it finds it again. It also
looks for a `<number>.opt` (the options), which does not exist yet.

The arrows are also needed as `WM_KEYDOWN` messages (the menu lists read them from there); the
script sends them both ways. Timings are in frames, but the game reasons in real
time: a key pressed while a menu is still appearing is ignored. The script that
leads to the Abstergo lab with the "Claude" profile (second in the list):

```sh
ALTAIR_KEYS="900:28,1500:208:6,1700:208:6,1900:28,3300:28,3900:28:30,4500:28:3"
```

### Control channel (since 2026-11-15)

The frame-based scripts remain, but for exploring one uses the control channel: with
`ALTAIR_CTL=folder` the runtime, at every `Present`, reads and deletes `folder/cmd` (one line
per command) and writes the frame number to `folder/status`.

| Command | Effect |
|---|---|
| `key DIK [frames]` | presses a key (default 10 frames) |
| `text TEXT` | types a text as window messages |
| `mouse DX DY [WHEEL]` | moves the mouse (relative, as DirectInput reads it) |
| `button N [frames]` | presses a mouse button (0 left, 1 right, 2 middle) |
| `move X Y`, `click X Y` | moves the pointer to a point on the screen, and left click |
| `stats` | prints the pace of the last interval |
| `shot FILE.png` | saves the back buffer to `folder/FILE.png` |
| `quit` | prints the census and exits |

```sh
tools/start.sh --game          # starts in the background and gets to the game of the "Claude" profile
tools/start.sh --animus           # continues to the Animus corridor with Altaïr (5 minutes)
tools/ctl.sh "key 28" "shot a.png"
tools/ctl.sh quit
```

## From the lab to the Animus (2026-11-15)

Path verified with the control channel, "Claude" profile, Italian language:

1. `CONTINUA` from the main menu loads `DataPC_Present_Room.forge`: a scene in the Abstergo
   lab, Vidic talks to Desmond sitting on the Animus (about two minutes).
2. "PREMERE QUALUNQUE PULSANTE PER INTERAGIRE" (press any button to interact) appears and **the scene waits**: without a key
   it does not continue (verified: stuck for over five minutes). With Space it continues.
3. The Animus screen seen from the couch: "CRONOLOGIA MEMORIA GENETICA", the DNA helix,
   "Ricordo bloccato", the faces of Lucy and Vidic; then "IN ATTESA DI INPUT". Another key, and the
   arrows move the pointer along the helix.
4. The game opens `DataPC_Memory_Corridor.forge` and `DataPC_extra.forge`: **Altaïr in the Animus
   corridor**, with the synchronization bar; the "SISTEMA DI CONTROLLO" tutorial starts
   (Testa, Mano nuda, Mano armata, Gambe: head, empty hand, weapon hand, legs) and two characters appear. With W Altaïr walks.

Screenshots in `run/shots/` (ignored by git: they are images of the game). Over the whole session,
10 million draws, no skipped draw and no validation error.

### The tutorial, step by step (2026-11-15 morning)

With simulated keyboard and mouse commands (`tools/ctl.sh`), after `tools/start.sh --animus`:

| Step | What the game asks | Command | Outcome |
|---|---|---|---|
| First-person view | "Premi Testa da fermo" (press Head while standing still) | `key 18 60` (E) | accepted, the scene changes |
| Gentle push | "Tieni premuto Mano nuda mentre cammini" (hold Empty hand while walking), reach the marker | `key 42 300` + `key 17 300` (Shift + W) | Altaïr walks through the line of monks and reaches the light |
| High/low profile | "Premi senza rilasciare" (press and hold) High profile | `button 1 300` (right button) | accepted, a target appears |

| Grab and throw | "Premi senza rilasciare (Mouse destro) e premi (Maiusc sinistro)" (press and hold right mouse and press left Shift), throw the man onto the marker | — | **not passed**: see below |

`tools/start.sh --tutorial` performs the first three steps by itself (durations in real time, `ms`
suffix in the `key` and `button` commands) and leaves the game in front of the fourth.

The fourth step asks you to approach a man, grab him and aim the throw: steering
with one screenshot every few seconds I lost the target from view on the second
attempt. It is not a defect of the translated game (the commands arrive), it is the limit of this way
of playing: from here on a person is needed. The save does not change during the tutorial (last
write of the `.sav`: the new game), so the tutorial has to be done in a single session.

The synchronization bar goes up one notch at every step (from 1 to 4). So keyboard, buttons and
mouse movement all reach the game and the tutorial can be played through. I stopped at the
target: the following steps (combat) are better played than scripted.

Defects seen:

- **The scene with the crowd is slow**: 3,800 draws, 9 frames per second (51 ms in the renderer,
  of which 35 of submission; the rest of the frame is the game itself).
- ~~The key name in the messages is empty~~ **Fixed.** The game creates `IDirectInput8W`
  (Unicode interface, GUID `BF798031-…`), so `EnumObjects` delivers
  `DIDEVICEOBJECTINSTANCEW` with the name in UTF-16; we were writing it in ASCII. The game's
  callback (`0x009091C0`) copies the name of the buttons (`GUID_Button`, index `dwOfs - 12`) and of the
  keys (`GUID_Key`, index `dwOfs` = DIK code) into a table of 0x208 bytes per entry. We now
  also enumerate the keyboard keys, with Italian names: the message says "Premi senza
  rilasciare (Mouse destro)".

## Registry and language

Verified with `ALTAIR_TRACE`: the game opens a single key, `HKLM\SOFTWARE\Ubisoft\Assassin's Creed`
(from `0x00402289`), and reads its `Language` value (from `0x004022AD`). The accepted names are
in the executable next to the key (`0x1680D24` onwards): `English`, `French`, `Spanish`,
`German`, `Italian`, `Japanese`, `Korean`, `Russian`, `Czech`, `Polish`. Without the key the
game starts in English and looks for `DataPC_StreamedSoundsEng.forge`, which the Italian
installation does not have.

`system.rs` emulates that key: `Language` is `ALTAIR_LANG` (default `Italian`),
`InstallDir` is `C:\AC1\`. With `Italian` the game opens `DataPC_StreamedSoundsIta.forge` and
shows the texts in Italian.

## INI files

`system.rs`. The `GetPrivateProfile*` functions and `WritePrivateProfileStringA` read and write
real files on the writable disk (`run/c/…`); the game folder stays read-only. On the
first run the game creates, in `C:\Users\Player\AppData\Roaming\Ubisoft\Assassin's Creed\`:

- `Assassin.ini`: `[Player] LastUsedProfile=<profile number>`;
- `DARE.INI`: options of the audio engine (`DS3D_CACHE_SIZE`, `NB_AUDIBLE_VOICES=48`, …).

With `LastUsedProfile` the path through the menus at startup stays the same as before (verified:
`tools/start.sh --game` still reaches the lab).

## Null pointers and invalid accesses

The first 64 KB of the guest space are protected, as in Windows (`ALTAIR_NULL=0` removes the
protection), and a `SIGSEGV` handler prints the guest address, the thread and the latest
indirect call targets, then exits with code 3. Before, a bad access gave a
silent core dump, and a read from a null pointer went unnoticed. Full path
to the Animus corridor with the protection on: no invalid access.

## Audio

`dsound.rs`. The game (DARE audio engine) creates dozens of secondary buffers of 38,400 bytes,
48 kHz, with looping `Play`, and fills them progressively following the play cursor: they are
streams, the final mixing is done by DirectSound. So:

- a `mixer` thread every 10 ms sums 480 frames of every playing buffer (8/16 bit, mono or
  stereo, linear resampling if the frequency is not 48 kHz, volume and pan in
  hundredths of a decibel) and **it is the one that advances the cursors**: the game keeps pace even
  with no output;
- the buffers have a lock of their own: the runtime's is almost always held by the game;
- an `audio-out` thread writes to PulseAudio with `pa_simple_write` from `libpulse-simple.so.0`,
  loaded with `dlopen` (present in WSL, nothing to install, no headers). Between the two there is
  a 60 ms queue: if the output does not consume, the blocks are dropped and counted;
- `ALTAIR_AUDIO=0` turns the output off; `ALTAIR_AUDIO_DUMP=file.raw` saves the mix
  (S16LE, 48 kHz, stereo).

Measured: in the menu and in the lab the mix has peaks between 15,000 and 30,000 out of 32,767 for
all 400 seconds of a session, so the game really decodes and plays (music, voices,
ambience). Not done: 3D sound (position, distance), EAX.

**The PulseAudio output works in real time with the 200 ms queue; it has not yet been
listened to by a person.** Measurement of 2026-11-15 morning, with the server back in good health: 100
seconds of game (logos, intro video with its audio), 100.8 seconds of mix,
**zero dropped blocks**.

History, because the problem may come back. During the night the output accepted a fifth of real time,
slowed down the other programs too (`ffmpeg -f pulse`) and in the end the server stopped
responding (`pa_simple_new`: error 8, timeout) for more than an hour; then it recovered by itself.
The first version asked for a 60 ms queue: the hypothesis, consistent with the measurements but not
proven, is that such a short queue throws WSLg's RDP sink into trouble for everyone, and that the
later tests with 200 ms were made with the server already compromised. If it happens again:
`ALTAIR_AUDIO=0` to play without it, `wsl --shutdown` from Windows to restart the server.

`tools/audio-probe.py` measures the output outside the game: it plays 4 seconds of tone in blocks, in
real time like the mixer, and says how long it took. With the server in good health: `pa_simple` with
the default queue or a 200 ms one, and SDL2 (`libSDL2`, present in WSL, which uses PulseAudio in its own
way), all consume 4 seconds in 4.0-4.2. SDL2 remains the ready alternative should `pa_simple`
start giving trouble again.

### The crackling sound: the game's audio thread was arriving late (2026-11-15)

Reported by the user at the first test on Windows: in the menu and in game, on top of the correct audio,
a second "broken" audio; the videos, however, clean. It is the first time anyone has listened.

How the game uses the buffers (seen with `ALTAIR_AUDIO_LOG=1`, which prints Play, Stop and every
`Lock` with cursor, instant and thread): every voice is a buffer of 19,200 bytes, 16-bit mono,
played in a loop at 24-36 kHz (`SetFrequency`), split into five segments of 3,840 bytes, about
60 ms. A dedicated thread (the one that calls `SetThreadPriority`) polls the position and
fills **the segment after the one the cursor is in**: so it has at most 60 ms of
margin. If it arrives late, the mixer plays a segment that is one lap old: correct sound with
wrong pieces on top. The videos do not suffer from it because their audio is already fully decoded.

Why it was arriving late: every one of its calls (`GetCurrentPosition`, `Lock`, `Unlock`, `GetStatus`,
`QueryPerformanceCounter`) took the runtime's global lock, contended by the thread that
draws. Measurement over one minute, from the menu to the lab: **123 fills that arrived more than
100 ms** after the previous one.

Fixes:

- the buffer methods and the clock no longer take the runtime lock (`F` shims);
- the mixer no longer makes up for lost time by advancing the cursors in jumps;
- on Windows, `timeBeginPeriod(1)` at startup: system timers go from 15.6 ms to 1 ms.

After: 70 delays on the same path, concentrated at startup and during loading (there the
audio thread waits for the game's own locks); in steady state, in the menu and in the lab, none.
The mixer now counts them: it prints "N ms of streams played from regions not rewritten in time" every
5 seconds in which it happens. **To be listened to again on Windows.**

### The crackling sound, the real cause: `floor(11.5) = 12` (2026-11-15)

The fix above was real but it was not the one: on Windows the log showed almost
no delays any more and the sound crackled all the same. The user gave the right clue:
**only the menu music and the voices** crackle, not the ambience, not the loading screens, not the videos.

Music and voices are **Ogg Vorbis**, decoded by a libvorbis compiled into the game;
the other sounds are not. The chain, traced backwards from the DirectSound buffer (all verified
here, with temporary probes in the generated C and with `ALTAIR_AUDIO_TRAP`):

| Stage | Address | Outcome |
|---|---|---|
| DirectSound mixing buffer (stereo 48 kHz, 37,888 bytes) | object `0x10013750` | alternating frames of two different signals, peaks |
| DARE's float → int16 conversion (`cvttps2pi`…) | `0x8383A0` | translated correctly; receives floats that are already broken |
| `vorbis_synthesis_blockin`, overlap-add | `0x83FF10` | translated correctly; source already broken |
| `vorbis_synthesis` | `0x841590` | |
| `mapping0_inverse` (table `0x1927740`) | `0x851A60` | healthy after residues, stereo coupling and curve; **broken after the MDCT** |
| `mdct_backward` | `0x84ECB0` | identical to the original when tested on its own |
| `mdct_bitreverse` | `0x84EB40` | reads outside the buffer |
| `mdct_init` | around `0x84F2D8` | **`log2n = 12` for n = 2048** (correct: 11); for n = 256 it gives 8, correct |

`mdct_init` computes `log2n = (int)floor(log(n)/log(2) + 0.5)`. For n = 2048 that is `floor(11.5)`, and
the `floor` of the translated CRT returned 12: `frndint` ignored the rounding mode
(details in `translator.md`, "What the test found", item 7). For n = 256, `floor(8.5)`
gave 8 only because round-to-nearest sends halves to even. With the wrong `log2n`
the bit-reversal table contains indices up to 2046 instead of up to 1022: the MDCT
of the long blocks reads memory beyond its own buffer and mixes it into the signal. This is why
the correct sound could be heard underneath, and why the original in Unicorn with the same tables
gave the same result as the translated code (same neighbouring memory) but not that of the game.

Two more things that came up along the way:

- The game ended up in the x87 branch of `floor` only because the CRT believed the CPU did not have
  SSE2 (ID bit of EFLAGS lost, item 8 of the same list). On a real PC it takes the
  SSE2 branch. This was fixed too: the cause is now removed twice over.
- DARE chooses between two implementations of the MDCT butterflies (`0x84E960` and `0x84EA30`)
  according to the variable `0x1A19ECC`, set at startup by `0x81A250` (CPU detection;
  in the game it is 1). Both tested with real data: identical to the original.

Verification after the fix, on the mixing buffer in the main menu (70 s, samples
above 300 in absolute value): the mean difference between consecutive frames is 27.7 and the one
at distance two is 53.7, as in a continuous signal; before it was the opposite (the distance-two one was
much smaller: two interleaved signals). No saturated sample, maximum 7,076.
**To be listened to again on Windows.**

Lesson: **a function identical to the original in isolation can be broken in the game if
its tables were born wrong.** The comparison must be made with the game's state, not with made-up
inputs; here it took saving the input and output of 600 real calls and re-running them
on the original. One piece of data saved for the test (`private/audio/mdct-2048.raw`) stays out of git.

Tools born from the investigation, all normally off:

- `ALTAIR_AUDIO_LOG=1` now also saves what the game writes into the streaming buffers, in
  `run/ctl/flusso-<object>.raw`, and prints the format and flags of every `CreateSoundBuffer`;
- `ALTAIR_AUDIO_TRAP=N` (Linux): at the N-th `Lock` of the mixing buffer the region becomes
  read-only; with `ALTAIR_TRAP_CONTINUE=1` the invalid-access handler names the
  translated function that writes to it (`dladdr`) and lets execution continue.

## Bink video

`bink.rs`. The game imports 15 functions from `binkw32.dll` and plays five videos from
`videos\`: `LoadingScreen` (looping during loading, 15 fps, silent), `UBI_LOGO`,
`Ubisoft_Presents`, `Warning_Disclaimer` and `DemoIntro` (29.97 fps, 3,020 frames, four
audio tracks: front, centre, bass, rear). All 1280×720.

Sequence observed per frame (counts printed when each video is closed):
`BinkWait` until it is time → `BinkDoFrame` → `BinkCopyToBuffer` (32-bit format, into a
locked texture) → `BinkShouldSkip` → `BinkNextFrame`. `BinkGetFrameBuffersInfo` is called
only once per video. Of the `BINK` structure the first fields are enough (width, height,
frames, current frame, frame rate).

RAD's library is not there. **Since 2026-11-15 the videos are decoded by a decoder
of our own**, the `crates/bink` crate (see `analysis/bink.md`): video identical to the reference,
audio in stereo (front + centre + rear) passed to the mixer progressively, twenty frames
ahead of the video. No external program. `ALTAIR_BINK=ffmpeg` goes back to the previous route
(an `ffmpeg` process read from a pipe), which remains as a comparison; `ALTAIR_VIDEO=0` makes
`BinkOpen` fail and the game skips the video. Enter skips the intro video.

## Real window

`window.rs`, with `ALTAIR_WINDOW=1`. The runtime's main thread, which otherwise only waits
for the game to end, runs the winit event loop; the renderer creates the
wgpu surface on the window and at every `Present` copies the back buffer to it, scaled to the
window size. Events accumulate in a shared state:

- keys → table of DIK codes for the keyboard's `GetDeviceState`, and
  `WM_KEYDOWN`/`WM_KEYUP` messages with the character ready for `ToUnicode`;
- mouse → `DIMOUSESTATE` (movement since the last read, wheel, buttons), `GetCursorPos`
  and `WM_MOUSEMOVE`, `WM_xBUTTONDOWN/UP` messages;
- closing the window → immediate exit of the process;
- **F8** (does not reach the game) shows or hides the diagnostics panel overlay:
  see "Diagnostics panel" further down;
- **F10** captures or releases the pointer (does not reach the game): captured, the mouse moves the
  view without stopping at the edges; released, it is for the menus.

Without the variable everything stays off screen, as before. `tools/play.sh` sets it.

**In a window the pace is tied to the screen.** The surface offers a single presentation
mode, `Fifo` (printed at startup: "presentation modes offered: [Fifo]"): with
wgpu's OpenGL backend on Wayland `Present` always waits for the refresh. So no more than
60 frames per second, and a scene that off screen does 45 drops to 30 in a window
(it misses one refresh out of two). Presenting from another thread would not help: the buffer swap
holds the OpenGL context lock, which the game thread needs too. A limit of WSLg with
OpenGL, not ours; with native Vulkan or D3D12 the non-waiting modes exist.

### Input diagnosis: `ALTAIR_INPUT_LOG=1`

One line for every stage an event goes through: `window:` (winit received it), `message:`
(delivered to the window procedure), `DirectInput keyboard/mouse:` (the game read it
with `GetDeviceState`; printed only when it changes). If a key does nothing, the log
says at which stage it was lost.

The `mouse`, `button`, `move`, `click` commands of the control channel write into the same state
that the window fills, so they exercise the same route. Verified this way (2026-11-15): in the
Animus corridor `mouse 400 0` **rotates the view** around Altaïr, and the click arrives
both at DirectInput and as `WM_LBUTTONDOWN`. Only the first stretch remains untested, from winit
to the state.

Default keys of the game (`DefaultBindings.map`, keyboard + mouse scheme): WASD movement,
arrows view, Space legs, left click weapon hand, E head, left Shift empty hand,
right click high profile, Tab map, Esc pause. The mouse buttons are codes 256 + N.

Status of the checks: the window opens via **Wayland** (WSLg) and shows the game; the keys
of the control channel work with the window open too. The **real** keyboard and mouse
**have not been tested**: during the night there was nobody to press keys, and there is no way to
inject them (on X11, where XTEST would exist, winit lacks `libxkbcommon-x11.so` and the window
does not open: in that case the runtime continues off screen). The game reads the mouse as
immediate state `DIMOUSESTATE2` (20 bytes) every frame, and at startup enumerates the objects
of the devices with `EnumObjects`: for the mouse we list three axes and eight buttons.

### Diagnostics panel (F8, 2026-11-15)

Request from the user: no numbers in the window title, but an indicator drawn
on screen, polished and in the style of the Animus, "that reminds you that the whole game takes place inside
a machine". First an original indicator to switch back on was looked for: there is none
(`hidden-features.md`).

`overlay.rs` builds the panel as a list of coloured triangles, in window
pixels; `gfx.rs` draws it on the surface after copying the back buffer, with a
separate pipeline (alpha, no texture) and a new vertex buffer per frame. The game
does not see it: its back buffer stays intact. Contents: a rotating helix of diamonds,
"ANIMUS / SINCRONIA", the frames per second in large type, milliseconds per frame, draws per
frame, and a history of small bars. Below 30 frames per second numbers and accents
turn to the red of desynchronization. The values update twice a second.

The history: 56 bars, **one every quarter of a second** (14 seconds in all), the oldest on
the left and fainter. The height is the average pace of that interval, on a scale with fixed
steps (30, 60, 120, 240, 480…) chosen on the visible maximum: it changes only when the pace really
changes. A bar is red if the interval was below 30 frames per second or
contained a frame longer than 50 ms; the thresholds are absolute, so it does not change colour
afterwards. The first version had one bar per frame, with height and red relative to the
average of the moment: it scrolled away in half a second, rescaled at every recomputation and the
bars changed colour (reported by the user, unreadable).

**No game assets in the repository:** helix and frame are described in code, and the Abstergo
logo is not there, on purpose: it is a Ubisoft trademark and rule 1 keeps it out.

**The fonts are the Animus ones, read from the user's data.** The first time it is switched on
`animus_font.rs` opens `DataPC.forge` in the game folder, takes from the "Game
Bootstrap Settings" entry the fonts `AnimusText PC` (text) and `Animus Title` (the large number)
with their images, and makes a single atlas out of them (512×1280, one byte per pixel); 209 ms on WSL.
`overlay.rs` draws every glyph as a rectangle with the atlas coordinates. Formats in
`forge.md`, "Interface fonts". If the read fails the panel falls back on
5×7 characters described in code, and writes so in the log.

It is switched on with F8, with the `fps` command of the control channel or with `ALTAIR_FPS=1`. With the
panel on, `shot` saves what is seen in the window (back buffer plus panel, composited
on a copy). Verified on WSL in the lab (`run/shots/pannello-4.png`); to be tested on
Windows.

**F8 and the game.** The key is intercepted in `window.rs` and never reaches DirectInput, like
F10. To find out whether the game loses anything: DIK code 66 (F8) injected from the control
channel in the lab, no visible effect; in the translated code no test of the kind
`[state+42h] & 80h`. It is not proof of absence: the keys can be remapped from the options.

### Skippable opening videos (2026-11-15)

The original lets you skip only `DemoIntro.bik`: Ubisoft logo, "presents" and warnings must be
watched in full. The permission sits in a table in the game's data, that of the
`StartupVideoManager`: 36-byte entries starting at `0x18C2C08`, read by the code around
`0x41D45D-0x41D6A6`. Verified here on the file and in the translated code:

| Offset | Meaning | LoadingScreen | UBI_LOGO | Ubisoft_Presents | Warning_Disclaimer | DemoIntro |
|---|---|---|---|---|---|---|
| +0 | pointer to the name | | | | | |
| +4 | **can be skipped** (byte) | 0 | 0 | 0 | 0 | 1 |
| +8 | fade-in, ms | 0 | 1000 | 1000 | 1000 | 1000 |
| +12 | duration, ms (−1: the whole video) | −1 | −1 | 4000 | 8000 | −1 |
| +16 | fade-out, ms | 1000 | 1000 | 1000 | 1000 | 1000 |
| +20 | fade colour | white | white | white | white | white |
| +24 | other flags (the two still screens have `01 01`) | | | | | |

The meaning of +8, +16 and +24 is deduced from the values (hypothesis); that of +4 is proven by the
result. At startup the runtime writes 1 into byte +4 of entries 1, 2 and 3 (`main.rs`), and from there
the game does everything, as for the intro video: on a key press the fade to
white starts and the video closes. Measurement (WSL, Enter 1.5 s after opening): each of the three
closes 1.2-1.3 s after the key; the logo at frame 82 of 257, without speeding up.
`ALTAIR_INTRO_SKIP=0` leaves the table as it is.

First attempt, discarded the same day: making `BinkWait` answer "do not wait" after
a key. The logo played sped up and the still screens, which have a duration in
milliseconds and not in frames, did not get short enough (reported by the user).
Lesson: before working around the game from outside, check whether it already has the feature and only
the permission is missing.

### Animus version (2026-11-15)

Request from the user: the Animus screen says "Versione Animus 1.28"; show
ours. `2.0.0α` is not possible (the game's fonts cover only Latin-1, `forge.md`), so
`2.0.0-dev.1`.

The text is in `Animus_MGB`, inside `DataPC_Present_Room.forge`, and once loaded it lives in
exactly-sized heap blocks: it cannot be lengthened in place (tried with `find`/`findhex`). How
the place to intervene was found: a temporary hook on `memcpy` (`0xF9FB80`) which, when the
copied bytes contain "1.28" in UTF-16, prints the code addresses on the stack. The
chain: the reader of the interface data (around `0x892C40` and `0x892E80`) reads the
length, allocates a temporary buffer, reads the characters into it and calls **`0x8A1920`**
(`cdecl`: output string, zero-terminated UTF-16 text), which builds the real string.

The hook on `0x8A1920` (`hooks.rs`, `animus_version`) looks at the text: if it contains "Animus"
and ends with "1.28", in any language, it writes a copy of it with the new version into a
guest memory region of our own and points the argument there; the function continues and allocates the
string of the right length. Verified: ten "… Animus 2.0.0-dev.1" strings in memory
and the text on screen in the timeline screen (`run/shots/elica-versione.png`).
`ALTAIR_ANIMUS_VERSION` changes the text; empty, it leaves the original.

### Icon (2026-11-15)

`altair icon EXE --out build/win/altair.ico` extracts from the original executable the first group
icon (`RT_GROUP_ICON` with its `RT_ICON`s: 11 images). `tools/build-win.sh`
compiles it with `llvm-rc` and `llvm-cvtres` and `tools/win-link.sh` links it into `altair-run.exe`
as resource no. 1; `window.rs` gives it to the window and to the taskbar. Like the generated
C, the icon is material derived from the game and stays in `build/`. **Never seen on Windows.**

## Native Windows (2026-11-15)

The same runtime runs on Windows, where wgpu uses Vulkan or Direct3D 12 with no intermediate
steps (in WSL every draw goes through OpenGL → Mesa → D3D12). **It compiles and links; it has not
yet been run**: from WSL I cannot launch Windows programs.

What changes between the two systems is in `host.rs`:

| | Linux | Windows |
|---|---|---|
| Library with the translated code | `dlopen` of a `.so` | `LoadLibraryW` of a DLL |
| Guest memory (in `runtime/altair.c`) | lazy 4 GB `mmap` | `VirtualAlloc` with 4 GB committed up front (see below) |
| Zero page and invalid accesses | `mprotect` + `SIGSEGV` | `VirtualProtect` + vectored exception handler |
| Exit from guest code | `setjmp`/`longjmp` | `__builtin_setjmp`/`__builtin_longjmp` |
| Audio output | PulseAudio (`libpulse-simple`) | `waveOut` (`winmm`) |
| Sampling profiler | signals | absent |

Choices to remember:

- **4 GB committed.** On Linux pages come into being when they are touched. On Windows a region that is only
  reserved would make the system functions that write into it fail (a file read into a
  game buffer), so all of it is committed: it does not take up RAM until needed, but it counts towards the
  limit of RAM plus page file.
- **No system `longjmp`.** On 64-bit Windows it would unwind the stack with the exception
  mechanism, crossing the Rust frames; the compiler intrinsics
  restore only the registers.
- **Exports.** A DLL cannot export more than 65,535 symbols and the translated functions
  are 117,000: only the five the runtime looks for are exported (`ALTAIR_API`).
- The exception handler deals only with accesses to guest memory; the others it
  leaves to whoever is responsible (a driver may handle its own).
- Adapter choice: `ALTAIR_BACKEND=vulkan|dx12|gl`; without it, Vulkan, then Direct3D 12.

```sh
tools/build-win.sh                                             # from WSL
```
```powershell
powershell -ExecutionPolicy Bypass -File tools\play.ps1       # from Windows, in the project folder
```

`play.ps1` opens the window and writes the log to `run\win\log.txt`, readable from
WSL too: it is how what does not work on Windows gets diagnosed. How to build: `environment.md`.

## What it taught

- **A function that discovery did not see.** The first failed indirect call was to
  `0x00FB2451`, in the CRT's math routines: it is reached from a table of pointers in
  `.data` **not aligned to 4** (at `0x019CB64A`), as scimitar-recomp had noted. Discovery
  now also searches `.data` for unaligned words that point to code right after a `ret`
  or a `jmp`: 65 more starts (116,142 functions). Some will be false positives: they cost
  only a little extra code.
- **Null pointers do not crash.** The guest space is mapped in full, so reading
  from address 0 gives 0 instead of an exception. A null object becomes a call
  to address 0, which the runtime reports: convenient, but the error surfaces later than it should.
  To be evaluated: protecting the first page.
- Incremental recompilation works: after the change to discovery 26 files out of 242
  were rewritten, and everything was recompiled in 10 seconds.

- **`VirtualQuery` matters.** With a fixed answer ("one page") the engine, when freeing a
  block, miscalculated which allocator it belonged to and called a non-existent object.
  Finding it required the import trace and a new diagnostic: at every stop the
  runtime prints the thread's last 32 indirect call targets (`altair_trail`),
  which show the path even through tail calls.
- **Overlapped reads.** The loading thread waited forever for the completion event
  of `ReadFile`: it must be signalled even if the read has already finished.

- **More functions that discovery did not see**, which emerged only by running:
  - a second CRT label in an unaligned table: *runs* of
    three or more code pointers at unaligned addresses are now recognized;
  - a function called with a direct `call` but reached by the translator and not by
    discovery: `altair image` now adds every missing `call` target (93 functions);
  - a callback that is just `ret 8`, the tail of another function; and its table neighbour,
    hidden by two **false starts** born from numeric constants. Discovery now processes data
    before immediates, requires two `nop`s (a single `0x90` is often a ModRM byte) and accepts
    pointer table entries only in a second pass;
  - an MMX audio mixer discarded because it contained `pushad`, which discovery considered
    an "instruction a game does not execute".
  In all, the functions went from 116,142 to **117,159**.
- **Callbacks into the game**: `EnumDisplayMonitors` calls a guest function. `call_guest`
  allows it for shims that do not hold the runtime lock.

- **Yet another false start.** With `NEW` chosen, loading stopped at `0x005830F0`: a
  real function, aligned and preceded by padding, discarded because a "round" address
  (`0x00583420`, a word in the data) had been accepted earlier and fell in the middle of one of its
  instructions. Discovery now works in three passes, from the strictest: first only the starts
  preceded by padding or by a `ret`, then those merely aligned, finally the table
  entries. `ALTAIR_WHY=0xADDR altair discover …` explains why a start is accepted or not.

## Next steps

1. Actually play with `tools/play.sh`: real keyboard and mouse have never been tested.
2. Audio output: understand why WSLg's PulseAudio consumes in fits and starts (Audio section).
3. Complete the Animus tutorial and get to Masyaf; check that the autosave
   works along the way.
4. Open points: SEH, real INI files, protected first page, 3D sound, controller.
