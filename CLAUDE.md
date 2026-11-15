# AC1 Recompiled - Altaïr (`altair`)

Static recompilation of Assassin's Creed (2007), Steam DX9 build, targeting native and browser.
Full plan, for outside readers: the document "AC1 Recompiled - Altaïr: piano di progetto"
(https://claude.ai/artifact/SoPWUhJZPJ45oWk3t4r8bu; that online document is in Italian). The
operational work lives in here.

**Names.** "AC1 Recompiled - Altaïr" is the full name of the project (with the diaeresis); `altair`, without
the diaeresis, is the technical name:
binary `altair`, crates `altair-*`, prefix `altair_` in the generated code, folder `altair`.
A possible recompiler for another game would take the name of its protagonist.
Until 2026-11-15 the technical name was `acrek` and the folder `ac-rek`: older commits still
mention them.

## Main rule: everything gets written here

Every discovery, measurement, decision or hypothesis is recorded in this directory **at the
very moment it is made**, not at the end of the session.

- Verified facts about the binaries → `docs/analysis/` (one file per topic).
- Decisions and reasons → `docs/decisions.md` (new entry at the top, with the date).
- Questions still open → `docs/open-questions.md`; when they are answered, move them
  to the right analysis file and leave a pointer.
- What was done in each session → `docs/journal.md`.
- Progress → tick the boxes in `docs/PLAN.md`.
- Links and external material → `docs/sources.md`.

Always distinguish what is **verified here** (with the tool or command that proves it) from
what comes from **external sources** or is a **hypothesis**.

Everything in the repository (docs, comments, messages, commit messages) is written in
English. Until 2026-11-15 the repository was in Italian, so older commit messages are in
Italian.

## Paths

**All work stays inside this directory**, including what must not be committed: in that
case it goes in a folder ignored by `.gitignore`.

| Folder | Contents | In git |
|---|---|---|
| `ref/` | Cloned reference repositories (scimitar-recomp, EaglePatch). To be read, not copied | no |
| `private/` | Material derived from Ubisoft binaries (names, dumps, Ghidra project) | no |
| `build/` | Generated output (translated C, objects) | no |
| `.venv/`, `opt/` | Python venv, downloaded tools (e.g. Ghidra) | no |
| `run/` | The writable `C:` drive of the running game (profiles, saves) | no |

- Game (Windows): `E:\Giochi\Steam\steamapps\common\Assassins Creed`
- From WSL: `/mnt/e/Giochi/Steam/steamapps/common/Assassins Creed` → in commands use `$AC1`
  (`export AC1="/mnt/e/Giochi/Steam/steamapps/common/Assassins Creed"`).
- Reference executable: `AssassinsCreed_Dx9.exe`, SHA-1 `a9ea476e70f076709cca59a802e26ec894432120`.

The game folder is **read-only**: do not write to it, do not copy files from it here.

## Legal rules (from the plan, non-negotiable)

1. No Ubisoft assets and no Ubisoft executables in the repository. No dumps, extracts or long hexdumps.
2. The generated C code stays in `build/` (ignored), never published.
3. Names obtained from Ubisoft binaries (e.g. AC2 Mac) only in `private/` (ignored).
4. No copied GPL code (scimitar-recomp): it is read as a specification and rewritten.
5. No EaglePatch code (unlicensed): knowledge only.

Addresses, hashes, function/class names, counts and short quotations needed to explain a
discovery are fine in the documents.

## Tools

```sh
source ~/.bash_exports                     # cargo is not in the PATH of non-interactive shells
cargo build --release                      # produces target/release/altair
.venv/bin/python tools/harness.py "$AC1/AssassinsCreed_Dx9.exe" selftest
.venv/bin/python tools/difftest.py "$AC1/AssassinsCreed_Dx9.exe"   # rerun after every change to the translator
python3 -I tools/binktest.py "$AC1/videos"    # rerun after every change to crates/bink (oracle: ffmpeg)
tools/build-image.sh                       # regenerates and recompiles the translated image (incremental)
tools/run.sh [--trace]                     # starts the translated game; stops at the first missing import
tools/play.sh                              # the same, in a window with keyboard, mouse and audio
tools/start.sh --game; tools/ctl.sh "shot a.png"   # in the background, driven by commands (run/ctl/)
```

Every new instruction in the translator must come with a scenario in the differential test.

- Python only with `python3 -I` and the standard library, except for dependencies declared in `tools/README.md`.
- Every new tool goes in `tools/` and is documented in `tools/README.md`.
- Planned stack: Rust (recompiler and runtime, `iced-x86`, `wgpu`), generated C compiled with clang.
