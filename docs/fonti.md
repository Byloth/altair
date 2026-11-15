# Fonti

## Progetti di riferimento

- [scimitar-recomp](https://github.com/T01iK/scimitar-recomp) (GPL-2.0): ricompilazione di
  AC1 (Ubisoft Connect, DX9) per macOS arm64. **Specifica, non codice da copiare.** Documenti
  chiave: `README`, `docs/AC1_WINDOWS.md`, `docs/PLAN.md`.
- [EaglePatch](https://github.com/Sergeanur/EaglePatch) (nessuna licenza in radice): plugin
  ASI per Steam/GOG. Hash della build, indirizzi di funzioni. Solo conoscenza.
  Anche su [Nexus Mods](https://www.nexusmods.com/assassinscreed/mods/29/).
- [Traduzione statica di AC1 con Claude](https://reversedwithai.com/ai-game-decompilation)
  (catalogo reversedwithai), parziale, non verificata.

## Librerie previste

- [iced-x86](https://github.com/icedland/iced): decodifica x86 in Rust.
- [wgpu](https://github.com/gfx-rs/wgpu): grafica su DX12, Vulkan, Metal, WebGPU.
- Unicorn: emulazione per il differential testing.
- quinn (QUIC) per l'online, più avanti.

## Mod della community

- [Assassin's Creed Overhaul 2016](https://www.dsogaming.com/news/assassins-creed-overhaul-2016-released-aiming-to-improve-the-textures-of-the-very-first-ac-game/) (hecumarine): texture.
- [Assassin's Creed Remastered (Community Edition)](https://www.gamepressure.com/download/assassins-creed-assassins-creed-remastered-community-edition-v20/z114e25) (Shazzaam).
- [Definitive AC1 Parkour Mod](https://www.gry-online.pl/download/assassins-creed-wersja-rezyserska-definitive-ac1-parkour-mod-v61/z414d9b) (HenryPDT): Cheat Engine, solo DX10.
- [Nexus Mods, categoria AC1](https://www.nexusmods.com/assassinscreed/mods).

## Documento di progetto

- [AC1 Recompiled - Altaïr: piano di progetto](https://claude.ai/artifact/SoPWUhJZPJ45oWk3t4r8bu):
  visione, architettura, rischi del web, nice to have, aspetti legali.

## Bink (2026-11-15)

- MultimediaWiki, "Bink Container", "Bink Video", "Bink Audio": https://wiki.multimedia.cx/index.php/Bink_Container
- FFmpeg n6.1, `libavformat/bink.c`, `libavcodec/bink.c`, `binkdata.h`, `binkdsp.c`,
  `binkaudio.c`, `wma_freqs.c` (LGPL 2.1+), scaricati in `ref/ffmpeg-bink/` (ignorata da
  git). Letti come specifica; le tabelle di `binkdata.h` sono riportate in
  `crates/bink/src/tables.rs` come dati del formato.
