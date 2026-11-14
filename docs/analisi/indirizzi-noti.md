# Indirizzi noti

Indirizzi della build Steam DX9 (`a9ea476e…`) già identificati da altri. Ogni riga dice da
dove viene. Gli indirizzi di scimitar-recomp valgono perché la loro build ha lo stesso codice
(vedi `scimitar-recomp.md`). Da EaglePatch si prende solo il dato, non il codice.

## Struttura dell'immagine (verificata qui)

| Indirizzo | Cosa | Fonte |
|---|---|---|
| `0x00400000` | Image base | header PE |
| `0x00F9F445` | Entry point, `WinMainCRTStartup` | header PE; nome da scimitar-recomp |
| `0x0168010C` | Puntatore all'array dei callback TLS | directory TLS |
| `0x01A3561C` | Indice TLS statico | directory TLS |
| `0x019CA838` | Security cookie `/GS` | load config |
| `0x017D2F10` | Campo SEHandlerTable della load config (scimitar-recomp dice "nessuna tabella SafeSEH": da controllare il conteggio) | load config |
| `0x011B5239`–`0x01520860` | Blocco di `int3` (3,5 MB) mai referenziato | scimitar-recomp |
| `0x0170B880` | `type_info::vftable` | `tools/rtti.py` |
| `0x00F9F7A2` | `_purecall` del CRT (in 2.721 slot di vtable) | `tools/rtti.py` + disassemblato |
| `0x016B1E4C`, `0x016B18AC`, `0x016B13FC` | vtable di `scimitar::Pad`, `PadXenon`, `PadProxyPC` (19 slot) | `tools/rtti.py` |

## Input (EaglePatch, `ref/EaglePatch/EaglePatch/src/ac1.cpp`)

| Indirizzo | Cosa |
|---|---|
| `0x0093F990` | `scimitar::Pad::UpdateTimeStamps` (thiscall) |
| `0x0093FC80` | `scimitar::Pad::ScaleStickValues` (thiscall) |
| `0x009161A0` | costruttore di `scimitar::PadXenon(padId)` |
| `0x0090B2F0` | `scimitar::PadProxyPC::AddPad(pad, type, name, u16, u16)` |
| `0x00909C90` | `scimitar::PadProxyPC::Update` (EaglePatch lo sostituisce) |
| `0x00916979` / `0x00916990` | punto in cui viene aggiunto il pad XInput / ritorno |
| `0x0098CF98`… `0x0098D092` | tabella dei comandi pad (EaglePatch la modifica per lo schema PS3) |

Strutture ricostruite da EaglePatch: `Pad` è grande `0x500` byte, `PadXenon` `0x520`,
`PadProxyPC` `0x15D0`. `PadProxyPC` contiene 8 `PadData` (Keyboard1–4, Joy1–4) da 536 byte.
Tipi di pad: `MouseKeyboardPad`, `PCPad`, `XenonPad`, `PS2Pad`. Ci sono 16 pulsanti.

## Memoria (EaglePatch)

| Indirizzo | Cosa |
|---|---|
| `0x007A4510` | allocatore del motore (cdecl, 8 argomenti) |
| `0x00924070` | `getNewDescriptor(size, align, var)` |
| `0x01A1E680` | variabile globale passata a `getNewDescriptor` |
| `0x00916440` | delete del motore |

## Grafica e varie (EaglePatch)

| Indirizzo | Cosa |
|---|---|
| `0x00E91422`, `0x00E9116D`, `0x00E91178` | controlli che disattivano l'MSAA alle alte risoluzioni |
| `0x00405495` | salto che abilita lo skip dei video iniziali |
| `0x017382D8` | flag della telemetria (EaglePatch lo azzera) |

## Riconoscere la build (EaglePatch)

EaglePatch distingue DX9 e DX10 leggendo il dword in `0x00401376`: vale `0x42D6` nella DX9.
Nella DX10 si legge `0x004013DF`, che vale `0x428D`.

## CRT (trovati dalle firme di byte, verificati con `tools/harness.py selftest`)

| Indirizzo | Cosa |
|---|---|
| `0x00FA6790` | `strlen` |
| `0x00FA9760` | `strcmp` |
| `0x00F9FB80` | `memcpy` (o `memmove`: stessa sequenza iniziale; ce n'è una seconda a `0x00FA0120`) |
| `0x00FA8E10` | probabilmente `_decode_pointer` (chiamata da `_purecall`, usa `TlsGetValue`) |
| `0x01915AC0` | tabella CRC-32 standard in `.data`, senza riferimenti diretti |
