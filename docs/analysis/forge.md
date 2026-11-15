# `.forge` archives

Verified on 2026-11-14 with `tools/forge.py` on the 18 archives of the Steam build (6.45 GB, 15,906
entries). The complete lists of names and objects are game content and live in `private/`.

```sh
python3 -I tools/forge.py summary "$AC1"
python3 -I tools/forge.py list "$AC1/DataPC.forge"
python3 -I tools/forge.py classes "$AC1/DataPC.forge" private/rtti-dx9.json
```

## The main finding

**Every object in the archives declares its own class with the CRC-32 of the name, and that name is
the same one that appears in the executable's RTTI.** It is the standard CRC-32 (zlib's)
computed over the short name of the class, without namespace: `crc32("ShaderPermutation")` gives
`0x9467F2BB`, as reported by scimitar-recomp.

Cross-referencing the CRCs with `private/rtti-dx9.json`, **100% of the top-level objects have a
recognised class**: 35,020 out of 35,020 in `DataPC.forge`, 7,245 out of 7,245 in
`DataPC_Assassination_Talal.forge`, 806 out of 806 in `DataPC_Extra.forge`.

Consequence: for every type of game data we know which C++ class loads it, and for that
class we have vtable and virtual functions (`rtti.md`). Data and code can be studied together.

## Container format (version 25)

All values are little-endian and unaligned.

| Structure | Fields |
|---|---|
| Header | `char[9]` `"scimitar\0"`, `u32` version (25), `u64` offset of HeaderFat, `u32`, `u8` |
| HeaderFat | `u32` number of files, `u32`, `u64`, `u64` (−1), `u32` max files per table, `u32` number of tables, `u64` offset of the first table |
| Table | `u32` entries, `u32`, `u64` offset of the entries, `u64` next table (−1 = end), `u32`, `u32`, `u64` offset of the name table, `u64` |
| Entry (16 bytes) | `u64` offset of the record, `u32` id, `u32` size of the data |
| Record | `"FILEDATA"`, `char[128]` name, other fields; **0x1B8 bytes** in all, then the data |

An entry's data is either raw (`GlobalMetaFile`, the streamed sounds) or **two compressed
sections**, without the initial word that AC2 has (it is the 4-byte difference noted by
scimitar-recomp).

| Compressed section | Fields |
|---|---|
| Header | `u64` marker `33 AA FB 57 99 FA 04 10`, `u16` version (1), `u8` codec, `u16` (0x8000), `u16` (0x8000), `u16` number of blocks |
| Block table | per block: `u16` decompressed bytes, `u16` bytes on disk |
| Blocks | per block: `u32` checksum, then the data; compressed if the bytes on disk are fewer than the decompressed ones |

The codec is always **1, LZO1X** (compressed with the 999 variant; the decompressor is the one common
to the whole LZO1X family). `forge.py` contains one in pure Python.

### Inside a compressed entry: a pack of objects

- **First section, the index:** `u16` number of objects, then for each one `u32` id and `u32`
  size.
- **Second section, the data:** the objects one after another. Each begins with `u32` CRC-32
  of the class, `u32` size of the body, `u32` length of the name, the name, `u32` id,
  again `u32` CRC-32 of the class, then the fields.

The fields are serialised in order, without tags ("FastLoad" format): interpreting them requires
the class's code. The block checksum has not been identified yet.

## Inventory of the archives

| Archive | Size | Entries | Content |
|---|---|---|---|
| `DataPC.forge` | 194 MB | 37 | startup and global data: `Game Bootstrap Settings`, `Game Fix`, animations, missions |
| `DataPC_Common.forge` | 667 MB | 1,715 | sound banks (`BAO_0x…`) and common data |
| `DataPC_Acre`, `_Damascus`, `_Jerusalem` | 775–809 MB | 869–1,387 | the three cities, divided into cells (`CellNNNNN_DataBlock`) |
| `DataPC_Kingdom`, `_Masyaf`, `_Arsuf` | 369–719 MB | 785–1,404 | the Kingdom, Masyaf, Arsuf, again in cells |
| `DataPC_Assassination_Talal` | 101 MB | 50 | a single assassination mission has an archive of its own |
| `DataPC_SolomonTemple`, `_Flashback_Paradise` | 197–245 MB | 34–122 | prologue and ending |
| `DataPC_Present_Room`, `_Memory_Corridor`, `_LoadingRoom2` | 10–286 MB | 23–90 | Abstergo laboratory, memory corridor, loading room |
| `DataPC_Map_Menu` | 2 MB | 2 | the world of the main menu |
| `DataPC_Extra` | 24 MB | 404 | textures only (403 `TextureMap` + 403 `TextureMapSpec`), including the tutorial ones |
| `DataPC_StreamedSounds` | 450 MB | 477 | streamed audio, uncompressed |
| `DataPC_StreamedSoundsita` | 127 MB | 6,056 | Italian voices, uncompressed |

The worlds are divided into cells loaded by streaming: the same scheme as the menu, where a
`World` object contains `WorldArea`, `WorldAmbiance`, `LayeredSky` and `Entity`.

## Classes in the archives

`DataPC.forge` contains 35,020 objects of 112 classes. The most numerous: `SoundBao` 18,306,
`Animation` 12,387, `FX` 658, `TextureMap` 521, `Entity` 427, `Material` 188, `Mesh` 168,
`MaterialTemplate` 83 (102 MB: it contains the shaders), `AnimSet` 80, `Skeleton` 50.

`DataPC_Assassination_Talal.forge` contains 7,245 objects of 56 classes: sounds, animations,
textures, meshes, plus the world data (`GridCellDataBlock`, `GridPartition`, `RegionLayout`) and
behaviour packs (`CustomActionGamePlayPack`, `CustomActionReactionPack`,
`VocalActionPack`, `ActionBlock`).

## The missions: data or code?

**Both: the structure is in the data, the behaviour is in the code.**

In `DataPC.forge` there are about 950 objects of 27 `MissionItem*` classes. The most numerous:

| Objects | Class | What it seems to be |
|---|---|---|
| 339 | `MissionItemSceneSequencer` | scene sequences |
| 161 | `MissionItemList` | lists of mission items |
| 91 | `MissionItemHighestPoint` | view points |
| 85 | `MissionItemFreeMission` | free missions |
| 66 | `MissionItemVigilantes` | saving citizens |
| 22 | `MissionItemScholars` | scholars |
| 11 / 10 / 10 | `MissionItemPickPocket`, `…EavesDropping`, `…Interrogation` | the three classic investigations |
| 8 / 4 / 4 / 4 / 3 | `MissionItemInformerAssassination`, `…Escort`, `…Flag`, `…FreeRun`, `…Kiosk` | the informer missions |
| 9 + 9 + 9 + 20 | `MissionItemPreBureau`, `…InitBureau`, `…WhileBureau`, `…PostBureau` | the phases around the Assassins' bureau |
| 15 | `MissionItemAlMualim` | the conversations with Al Mualim |
| 8 | `MissionMemoryFile` | the memory blocks |

In the code there are 45 `MissionItem*` classes (including the test ones left in the build).

What this implies for Phase 6:

- **Rearranging the existing missions is work on the data**: which investigations are needed, in
  what order, where, under which conditions (`MissionItemList`, `MissionItemCountOr`). There is no need
  to touch the game's code; what is needed is being able to read and rewrite these objects.
- **A new mission type requires code**: every type is a C++ class. It can however be
  added in our runtime and registered with the engine.
- The next step, when we get there, is to decode the fields of `MissionItemList` and of a
  simple type, such as `MissionItemPickPocket`, by reading their loading function.

Note: the class names suggest the role, but the meaning of the fields is not verified.

## Limits

- Only the top-level objects of each pack are counted; nested ones are not.
- Class inventory done on 3 archives out of 18. The Python decompressor takes almost
  3 minutes on `DataPC.forge`: for the complete inventory it is worth rewriting it in Rust (the
  runtime will need it anyway for the tests).
- Unnamed fields in the tables, name table and block checksum: not decoded.
- The engine function that computes the CRC has not been found (`harness.md`); we do know, however,
  that the result is the standard CRC-32.

## Reader in Rust (`crates/forge`, 2026-11-15)

`altair_forge` (library: `Forge::open`, `entries`, `name`, `sections`, `decompress`, `raw`,
`lzo1x_decompress`, `crc32`) and the `altair-forge` binary (`list`, `digest`). It is the rewrite
of `tools/forge.py`, which remains the reference for comparison.

Verification: `list` and `digest` (per entry: id, number of sections, decompressed bytes, CRC-32 of the
decompressed data) **byte-for-byte identical** between Python and Rust on `DataPC_Map_Menu` (2 entries),
`DataPC_LoadingRoom2` (23) and `DataPC_Extra` (404). `DataPC_Memory_Corridor`: 90 entries, 174 MB
decompressed in 1.5 s.

```sh
cargo build --release -p altair-forge
target/release/altair-forge digest "$AC1/DataPC_Extra.forge" > build/forge-check/rs.txt
python3 -I tools/forge.py digest "$AC1/DataPC_Extra.forge" > build/forge-check/py.txt
cmp build/forge-check/rs.txt build/forge-check/py.txt
```

Not ported yet: the index of the objects inside an entry and class recognition
(`forge.py classes`).

## Interface fonts (2026-11-15)

Verified here on `DataPC.forge`, with throwaway scripts and then with `crates/runtime/src/animus_font.rs`.

The interface belongs to the "magma" library (classes `magma::PixmapFont`, `GlyphFont`, `FontServer`…
in the RTTI). In the "Game Bootstrap Settings" entry there are 4 `MagmaMftFile` objects (the fonts)
and 13 `MagmaMgbFile` (screens and text tables):

| Object | Id | Image (`TextureMap`) |
|---|---|---|
| `AnimusText PC` | `0x4EBA10AB` | `0x47195956`, 256×256 |
| `AnimusText Bold_PC` | `0x4EBA10A0` | through the spec `0x4719595C` |
| `AnimusTechno Regular 22_PC` | `0x4EBA1095` | through the spec `0x4719598E` |
| `Animus Title` (`AnimusTitle Thin 74`) | `0x87E599F1` | `0x87E599EC`, 512×1024 |

All of them cover Latin-1 (from `0x20` to `0xFF`, 199 glyphs; 261 for the titles): **no Greek**.

**`MagmaMftFile`.** After the string `PixmapFont`: `u16` number of glyphs, then a 30-byte
record per glyph: `u16` code, `u16` width − 1, `u16` height − 1, `i16` x offset,
`i16` height of the top edge above the baseline, `u16` advance, four `f32` (u0, v0,
u1, v1 in the image), `u16` set to zero. Every file repeats the font five times, identical
(hypothesis: one per language). The reference to the image is a string, for example
`0x47195953TGA32`: it is the id of a `TextureMapSpec`, which in turn names the `TextureMap`.

**`TextureMap`.** After the common header, name (with terminator), id and class: `u32`
width, `u32` height, other fields; at +71 from the width `u32` size of the data, at +75
the data; 61 bytes at the end. For the fonts the data is **DXT3** (16 bytes per 4×4 block: 8 of
4-bit alpha, then the colour, always white), with the **rows from bottom to top**. The
glyph is entirely in the alpha. Proof: a line of text composed with these metrics reads
correctly (`run/shots/pannello-4.png`). The meaning of the other fields is not verified.

**Texts.** In the `MagmaMgbFile` objects the texts are UTF-16 preceded by a 32-bit identifier and
by the length in characters (`u32`), all the languages one after another. "Versione Animus 1.28"
("Animus Version 1.28") is in `Animus_MGB` (`0x9D8BDA78`), inside `DataPC_Present_Room.forge`, twice per language.
