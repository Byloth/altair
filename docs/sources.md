# Sources

## Reference projects

- [scimitar-recomp](https://github.com/T01iK/scimitar-recomp) (GPL-2.0): recompilation of
  AC1 (Ubisoft Connect, DX9) for macOS arm64. **A specification, not code to copy.** Key
  documents: `README`, `docs/AC1_WINDOWS.md`, `docs/PLAN.md`.
- [EaglePatch](https://github.com/Sergeanur/EaglePatch) (no licence in the root): ASI
  plugin for Steam/GOG. Build hashes, function addresses. Knowledge only.
  Also on [Nexus Mods](https://www.nexusmods.com/assassinscreed/mods/29/).
- [Static translation of AC1 with Claude](https://reversedwithai.com/ai-game-decompilation)
  (reversedwithai catalogue), partial, not verified.

## Planned libraries

- [iced-x86](https://github.com/icedland/iced): x86 decoding in Rust.
- [wgpu](https://github.com/gfx-rs/wgpu): graphics on DX12, Vulkan, Metal, WebGPU.
- Unicorn: emulation for differential testing.
- quinn (QUIC) for online play, later on.

## Community mods

- [Assassin's Creed Overhaul 2016](https://www.dsogaming.com/news/assassins-creed-overhaul-2016-released-aiming-to-improve-the-textures-of-the-very-first-ac-game/) (hecumarine): textures.
- [Assassin's Creed Remastered (Community Edition)](https://www.gamepressure.com/download/assassins-creed-assassins-creed-remastered-community-edition-v20/z114e25) (Shazzaam).
- [Definitive AC1 Parkour Mod](https://www.gry-online.pl/download/assassins-creed-wersja-rezyserska-definitive-ac1-parkour-mod-v61/z414d9b) (HenryPDT): Cheat Engine, DX10 only.
- [Nexus Mods, AC1 category](https://www.nexusmods.com/assassinscreed/mods).

## Project document

- [AC1 Recompiled - Altaïr: piano di progetto](https://claude.ai/artifact/SoPWUhJZPJ45oWk3t4r8bu)
  (project plan, in Italian): vision, architecture, web risks, nice-to-haves, legal aspects.

## Bink (2026-11-15)

- MultimediaWiki, "Bink Container", "Bink Video", "Bink Audio": https://wiki.multimedia.cx/index.php/Bink_Container
- FFmpeg n6.1, `libavformat/bink.c`, `libavcodec/bink.c`, `binkdata.h`, `binkdsp.c`,
  `binkaudio.c`, `wma_freqs.c` (LGPL 2.1+), downloaded into `ref/ffmpeg-bink/` (ignored by
  git). Read as a specification; the tables from `binkdata.h` are reproduced in
  `crates/bink/src/tables.rs` as format data.
