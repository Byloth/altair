# Eseguibili della build Steam

Verificato il 2026-11-14 con `tools/pe.py` sulla copia Steam installata
(`E:\Giochi\Steam\steamapps\common\Assassins Creed`, 6,8 GB).

## File eseguibili presenti

| File | Dimensione | SHA-1 | Ruolo |
|---|---|---|---|
| `AssassinsCreed_Dx9.exe` | 24.666.760 | `a9ea476e70f076709cca59a802e26ec894432120` | **Gioco, renderer D3D9. Nostro riferimento.** |
| `AssassinsCreed_Dx10.exe` | 24.183.432 | `601475fd5efa02bbae80d1c06dfa25a0a1e7fd2f` | Gioco, renderer D3D10 |
| `AssassinsCreed_Game.exe` | 438.272 | `08455f18feb6377c747377df29b050d7ba21b6fa` | Launcher: sceglie tra Dx9 e Dx10 (contiene entrambi i nomi e chiama `Direct3DCreate9`) |
| `testapp.exe` | 438.272 | identico ad `AssassinsCreed_Game.exe` | Copia del launcher |
| `Detection/Detection.exe` | — | — | Rilevamento hardware (MFC 7.1, test DX9/DX10). Non ci serve |

DLL accanto agli eseguibili: `binkw32.dll` (video Bink), `eax.dll` (audio EAX),
`d3dx9_36.dll`, `d3dx10_36.dll`.

**L'hash della DX9 coincide con quello che EaglePatch indica per Steam e GOG.** È quindi
la stessa build, priva di DRM. scimitar-recomp usa invece la build Ubisoft Connect
(SHA-1 `b3fff1737cf77bc3ef5bb0ede6905b40c8f4e476`): resta da verificare quanto differisce
(vedi `domande-aperte.md`).

## `AssassinsCreed_Dx9.exe`

- PE32 i386, linker MSVC 8.0, timestamp 2008-04-23 19:45:11 UTC.
- Image base `0x400000`, **nessuna rilocazione**, entry point `0xF9F445`.
- Build string: `ChangeList:86610; Branch://assassin-playground/dx10_integration/main/;`
  `Time:Wed Apr 23 15:46:38 2008; Exec:scimitar_f.exe`.
- PDB interno: `d:\assassin\assassin-playground\dx10_integration\main\output\win32\exe\scimitar_f.pdb`
  (il PDB non è distribuito). "Scimitar" è il nome interno del motore (poi Anvil).
- Import: 17 DLL, 260 funzioni (scimitar-recomp ne conta 261 sulla build Ubisoft Connect,
  che ha in più `uplay32.dll`).
  `kernel32, user32, advapi32, shell32, ole32, oleaut32, ws2_32, eax, d3d9, d3dx9_36,
  dinput8, dsound, xinput1_3, winmm, binkw32, wininet, iphlpapi`.
- RTTI: 4.043 nomi di tipo distinti.

| Sezione | VA | Dim. virtuale | Dim. su disco |
|---|---|---|---|
| `.text` | `0x00401000` | `0x127A7B3` (18,5 MB) | `0x127B000` |
| `.rdata` | `0x0167C000` | `0x02452F7` | `0x0246000` |
| `.data` | `0x018C2000` | `0x01843B0` (1,5 MB) | `0x014D000` |
| `.idata` | `0x01A47000` | `0x00024CE` | `0x0003000` |
| `.tls` | `0x01A4A000` | `0x00003A2` | `0x0001000` |
| `_RDATA` | `0x01A4B000` | `0x00024FB` | `0x0003000` |
| `.data1` | `0x01A4E000` | `0x00007C0` | `0x0001000` |
| `.rsrc` | `0x01A4F000` | `0x016D765` | `0x016E000` |

Nessuna sezione `.nidata` e nessun import di `uplay32.dll`: niente DRM.

## Confronto DX9 / DX10

Stesso changelist (86610) e stesso branch: **stessa revisione del codice sorgente**,
compilata il 16 aprile (DX10) e il 23 aprile 2008 (DX9). La logica di gioco dovrebbe
quindi coincidere; cambia il backend grafico. Gli indirizzi invece no: le due `.text`
hanno dimensioni diverse.

| | DX9 | DX10 |
|---|---|---|
| Timestamp | 2008-04-23 | 2008-04-16 |
| `Exec` | `scimitar_f.exe` | `scimitar_dx10_f.exe` |
| Entry point | `0xF9F445` | `0x9BB835` |
| `.text` | `0x127A7B3` | `0x12818EF` |
| `.data` virtuale | 1,5 MB | **17,3 MB** (`0x1149CD0`) |
| DLL / funzioni importate | 17 / 260 | 18 / 262 |
| Nomi RTTI | **4.043** | **937** (tutti presenti anche nella DX9) |

Differenze di import:

- solo DX9: `Direct3DCreate9`, `D3DXCheckTextureRequirements`, `D3DXGetShaderConstantTable`,
  `EnumDisplayMonitors`;
- solo DX10: `D3D10CreateDevice`, `D3D10CompileShader`, `D3D10ReflectShader`,
  `D3DX10UnsetAllDeviceObjects`, `CreateDXGIFactory`, `MoveWindow`.

La DX10 contiene shader DXBC (SM4) incorporati e la stringa
`MSAA hi settings: Count %d, Quality %d`, che nella DX9 manca.

**Conseguenza per noi:** la DX9 ha più di 4 volte i nomi di classe della DX10, inclusi
gameplay e menu (`AssassinMenuManager`, `AwarenessResetEvent`, `EntityRequestEvent`, …) e
le classi `DX9Device` del renderer. Senza PDB sono la fonte principale di nomi. Vedi
`decisioni.md`.

## Comportamento a runtime (riferito dall'utente)

- Su Windows, con l'hardware attuale, `AssassinsCreed_Dx9.exe` **si blocca all'avvio**;
  `AssassinsCreed_Dx10.exe` parte e gioca senza problemi (2026-11-14). Causa ignota.
