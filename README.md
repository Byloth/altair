# AC1 Recompiled - Altaïr

Unofficial project, not affiliated with Ubisoft: a static recompilation of
**Assassin's Creed (2007)**, Steam DX9 build, to run it natively and in the browser
and to use it as the basis for an improved game. You need your own copy of the game; this
repository contains nothing from Ubisoft.

**Altair** is the technical name of the project: the recompiler (`altair`), its crates and the
support code for the generated code (`runtime/altair.h`). The full name is "AC1 Recompiled - Altaïr".

## Layout

| Path | Contents |
|---|---|
| `CLAUDE.md` | Working rules and legal rules of the project |
| `docs/PLAN.md` | Work plan with phases, tasks and gates |
| `docs/decisions.md` | Decision log, with the reasons |
| `docs/open-questions.md` | Things still to find out |
| `docs/journal.md` | Session journal |
| `docs/sources.md` | External projects, mods and references |
| `docs/environment.md` | Installed tools and how to start them |
| `docs/analysis/` | Verified facts about the game files |
| `crates/` | The recompiler in Rust: `altair` (see `docs/analysis/translator.md`) |
| `runtime/` | C support for the generated code |
| `crates/shader/` | Translator from Direct3D 9 shaders to WGSL (`altair-shaderc`) |
| `crates/gpu-probe/` | Tests of the graphics chain |
| `crates/runtime/` | `altair-run`: the Win32 runtime that runs the translated game (see `docs/analysis/runtime.md`) |
| `tools/` | Analysis and test scripts (see `tools/README.md`) |
| `ref/`, `build/`, `private/`, `.venv/`, `opt/` | Cloned references, generated output, unpublishable material, local tools: ignored by git |

## Status

Phase 0 (foundations) closed on 2026-11-14. Phase 1 (the recompiler): gate reached on 2026-11-15, all of the game's code translates to C and compiles.
Phase 2 (the Win32 runtime): gate reached on 2026-11-15. The translated game runs its startup, creates its
threads, opens the `.forge` files and creates the window.
Phase 3 (graphics) in progress: the translated game shows menus and profiles, starts a new game and draws the
first 3D scene, the Abstergo lab, with our Direct3D 9 on wgpu (off-screen, scripted input). See `docs/PLAN.md`.
