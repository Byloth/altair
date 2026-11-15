# Development leftovers in the DX9 executable

Question from the user (2026-11-15): before inventing an FPS indicator of our own, is there
something ready-made in the game (development functions, hidden commands) to exploit?

First survey, only on the strings and on the references in the code. Everything **verified here**
(throwaway Python scripts on `AssassinsCreed_Dx9.exe`, Capstone for the disassembly), except
where it says "hypothesis".

## What is there

**A complete command-line option parser.** About 60 options compared one by
one with `stricmp` (`0xF9ED8E`) in a function that begins around `0x4089D0`, plus the help
text ("Command Line Options", `/help`). Among others: `/windowed`, `/nosound`, `/nobink`,
`/nofps`, `/disablesave`, `/quit`, `/walkthrough`, `/walkthroughloading`, `/logtime`,
`/consoleout`, `/noconsole`, `/waitfordebugger`, `/preloadshaders`, `/remotepad`,
`/soundsystemdebug`, `/darespy`, a `/mem…` family (memory statistics) and production-tool
options (`/computeao`, `/compresstextures`, `/buildloadinggrid`, `/startanvil`).

**`/nofps` implies an FPS indicator normally on in development builds.** The option
zeroes byte `+0x21EB` of the options object; the constructor (`0x408768`) initialises it;
`0x406341` copies it into byte `+0x92` of the global object pointed to by `0x1A0F058` (the
engine settings). A second place writes the same byte (`0x4077AC`).

**Debug classes left in the RTTI:** `DebugCameraToggleEvent`, `GoDebugInputInterpreter`,
`DebugContext`, `DebugPath`, `IHumanDebug`, `QuadrupedDebug`, `SoundDebugOptions`,
`ShaderDebugInfoPack`, `LoadWorldMissionDebugHandler_m`, `QuickTeleportHandler` (with its
buttons for world and portal: a teleport menu), `hkDebugDisplay` (Havok).

**Display enumerations:** `DisplayOptions::ProfilerStats` with 24 entries
(`ProfilerStatsCalls`, `…Memory`, `…Shadow0-3`, `…PostEffects`…), `DisplayOptions::FillMode`
(`WireFrame`), `RenderMode`, `LightingMode`, `ObjectSelectionMode`.

## What is missing

- **The option parser is orphaned.** No direct `call` to `0x4089D0`, no
  pointer to that address in the data; neither the parser, nor the options constructor, nor
  `0x406341` are among the 117,106 functions the translator reaches, and the game runs without
  ever asking for them. Of the roughly 3,000 instructions between `0x408000` and `0x40AF1D`, 403 turn out
  to be reachable. It is code left in the binary but disconnected.
- **No text for the indicator.** Among the 43,067 strings (ASCII and UTF-16) there is no
  format such as "FPS: %d" or the like. Hypothesis: the drawing of the debug text was
  compiled out and only the switch is left.
- **No reader found for byte `+0x92` of the settings** (second search, same
  day). In the translated code no function loads `[0x1A0F058]` and then reads `+92h`; the
  16 reads of a byte at `+92h` in all the code belong to other classes (the ones near the engine
  were looked at one by one: `0xA07A46` is the computation of a light's colour, `0xA05AB4`
  and `0xA05C1F` copy a structure, `0x9CF982` is library code). The object is created in
  `0x4066B9` (`call 0x9E5090`). It is not definitive proof, but together with the absence of the text it is enough
  not to count on it: we built our own indicator (`runtime.md`, "Diagnostics panel").

## What can be done with it

- With a recompiler, reconnecting orphan code is possible: it is enough to add the address
  to the roots and call it. But a switch without the code that draws does not give an
  FPS indicator: **for that there is, as of today, nothing ready-made to exploit.**
- The leads that could yield something working, in order of promise: the
  `QuickTeleportHandler` menu (it is a menu handler like the others, so it has data and code), the
  debug camera (`DebugCameraToggleEvent`), wireframe `FillMode`, the profiler
  statistics. For each one it must be verified whether the code is reachable and whether the data (`.forge`)
  still contains the objects. Not done.

See `open-questions.md`.
