# Executables of the Steam build

Verified on 2026-11-14 with `tools/pe.py` on the installed Steam copy
(`E:\Giochi\Steam\steamapps\common\Assassins Creed`, 6.8 GB).

## Executable files present

| File | Size | SHA-1 | Role |
|---|---|---|---|
| `AssassinsCreed_Dx9.exe` | 24,666,760 | `a9ea476e70f076709cca59a802e26ec894432120` | **Game, D3D9 renderer. Our reference.** |
| `AssassinsCreed_Dx10.exe` | 24,183,432 | `601475fd5efa02bbae80d1c06dfa25a0a1e7fd2f` | Game, D3D10 renderer |
| `AssassinsCreed_Game.exe` | 438,272 | `08455f18feb6377c747377df29b050d7ba21b6fa` | Launcher: chooses between Dx9 and Dx10 (contains both names and calls `Direct3DCreate9`) |
| `testapp.exe` | 438,272 | identical to `AssassinsCreed_Game.exe` | Copy of the launcher |
| `Detection/Detection.exe` | — | — | Hardware detection (MFC 7.1, DX9/DX10 tests). We do not need it |

DLLs next to the executables: `binkw32.dll` (Bink video), `eax.dll` (EAX audio),
`d3dx9_36.dll`, `d3dx10_36.dll`.

**The hash of the DX9 one matches the one EaglePatch gives for Steam and GOG.** It is therefore
the same build, DRM-free. scimitar-recomp instead uses the Ubisoft Connect build
(SHA-1 `b3fff1737cf77bc3ef5bb0ede6905b40c8f4e476`): how much it differs remains to be verified
(see `open-questions.md`).

## `AssassinsCreed_Dx9.exe`

- PE32 i386, MSVC 8.0 linker, timestamp 2008-04-23 19:45:11 UTC.
- Image base `0x400000`, **no relocations**, entry point `0xF9F445`.
- Build string: `ChangeList:86610; Branch://assassin-playground/dx10_integration/main/;`
  `Time:Wed Apr 23 15:46:38 2008; Exec:scimitar_f.exe`.
- Internal PDB: `d:\assassin\assassin-playground\dx10_integration\main\output\win32\exe\scimitar_f.pdb`
  (the PDB is not distributed). "Scimitar" is the engine's internal name (later Anvil).
- Imports: 17 DLLs, 260 functions (scimitar-recomp counts 261 on the Ubisoft Connect build,
  which additionally has `uplay32.dll`).
  `kernel32, user32, advapi32, shell32, ole32, oleaut32, ws2_32, eax, d3d9, d3dx9_36,
  dinput8, dsound, xinput1_3, winmm, binkw32, wininet, iphlpapi`.
- RTTI: 4,376 type descriptors, 4,597 vtables → `rtti.md`.

| Section | VA | Virtual size | Size on disk |
|---|---|---|---|
| `.text` | `0x00401000` | `0x127A7B3` (18.5 MB) | `0x127B000` |
| `.rdata` | `0x0167C000` | `0x02452F7` | `0x0246000` |
| `.data` | `0x018C2000` | `0x01843B0` (1.5 MB) | `0x014D000` |
| `.idata` | `0x01A47000` | `0x00024CE` | `0x0003000` |
| `.tls` | `0x01A4A000` | `0x00003A2` | `0x0001000` |
| `_RDATA` | `0x01A4B000` | `0x00024FB` | `0x0003000` |
| `.data1` | `0x01A4E000` | `0x00007C0` | `0x0001000` |
| `.rsrc` | `0x01A4F000` | `0x016D765` | `0x016E000` |

No `.nidata` section and no import of `uplay32.dll`: no DRM.

## DX9 / DX10 comparison

Same changelist (86610) and same branch: **same revision of the source code**,
compiled on 16 April (DX10) and on 23 April 2008 (DX9). The game logic should
therefore match; the graphics backend changes. The addresses, however, do not: the two `.text`
sections have different sizes.

| | DX9 | DX10 |
|---|---|---|
| Timestamp | 2008-04-23 | 2008-04-16 |
| `Exec` | `scimitar_f.exe` | `scimitar_dx10_f.exe` |
| Entry point | `0xF9F445` | `0x9BB835` |
| `.text` | `0x127A7B3` | `0x12818EF` |
| Virtual `.data` | 1.5 MB | **17.3 MB** (`0x1149CD0`) |
| Imported DLLs / functions | 17 / 260 | 18 / 262 |
| RTTI type descriptors | **4,376** | **939** (all present in the DX9 one too) |

Import differences:

- DX9 only: `Direct3DCreate9`, `D3DXCheckTextureRequirements`, `D3DXGetShaderConstantTable`,
  `EnumDisplayMonitors`;
- DX10 only: `D3D10CreateDevice`, `D3D10CompileShader`, `D3D10ReflectShader`,
  `D3DX10UnsetAllDeviceObjects`, `CreateDXGIFactory`, `MoveWindow`.

The DX10 one contains embedded DXBC (SM4) shaders and the string
`MSAA hi settings: Count %d, Quality %d`, which is missing in the DX9 one.

**Consequence for us:** the DX9 one has more than 4 times the class names of the DX10 one (and 4,597 vtables against 1,000), including
gameplay and menus (`AssassinMenuManager`, `AwarenessResetEvent`, `EntityRequestEvent`, …) and
the renderer's `DX9Device` classes. Without a PDB they are the main source of names. See
`decisions.md`.

## Runtime behaviour (reported by the user)

- On Windows, with the current hardware, `AssassinsCreed_Dx9.exe` **hangs at startup**;
  `AssassinsCreed_Dx10.exe` starts and plays without problems (2026-11-14). Cause unknown.
