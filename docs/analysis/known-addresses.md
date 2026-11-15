# Known addresses

Addresses of the Steam DX9 build (`a9ea476e…`) already identified by others. Every row says where
it comes from. The scimitar-recomp addresses hold because their build has the same code
(see `scimitar-recomp.md`). From EaglePatch only the data is taken, not the code.

## Structure of the image (verified here)

| Address | What | Source |
|---|---|---|
| `0x00400000` | Image base | PE header |
| `0x00F9F445` | Entry point, `WinMainCRTStartup` | PE header; name from scimitar-recomp |
| `0x0168010C` | Pointer to the array of TLS callbacks | TLS directory |
| `0x01A3561C` | Static TLS index | TLS directory |
| `0x019CA838` | `/GS` security cookie | load config |
| `0x017D2F10` | SEHandlerTable field of the load config (scimitar-recomp says "no SafeSEH table": the count is to be checked) | load config |
| `0x011B5239`–`0x01520860` | Block of `int3` (3.5 MB) never referenced | scimitar-recomp |
| `0x0170B880` | `type_info::vftable` | `tools/rtti.py` |
| `0x00F9F7A2` | `_purecall` of the CRT (in 2,721 vtable slots) | `tools/rtti.py` + disassembly |
| `0x016B1E4C`, `0x016B18AC`, `0x016B13FC` | vtables of `scimitar::Pad`, `PadXenon`, `PadProxyPC` (19 slots) | `tools/rtti.py` |

## Input (EaglePatch, `ref/EaglePatch/EaglePatch/src/ac1.cpp`)

| Address | What |
|---|---|
| `0x0093F990` | `scimitar::Pad::UpdateTimeStamps` (thiscall) |
| `0x0093FC80` | `scimitar::Pad::ScaleStickValues` (thiscall) |
| `0x009161A0` | constructor of `scimitar::PadXenon(padId)` |
| `0x0090B2F0` | `scimitar::PadProxyPC::AddPad(pad, type, name, u16, u16)` |
| `0x00909C90` | `scimitar::PadProxyPC::Update` (EaglePatch replaces it) |
| `0x00916979` / `0x00916990` | point where the XInput pad is added / return |
| `0x0098CF98`… `0x0098D092` | table of pad commands (EaglePatch modifies it for the PS3 scheme) |

Structures reconstructed from EaglePatch: `Pad` is `0x500` bytes large, `PadXenon` `0x520`,
`PadProxyPC` `0x15D0`. `PadProxyPC` contains 8 `PadData` (Keyboard1–4, Joy1–4) of 536 bytes each.
Pad types: `MouseKeyboardPad`, `PCPad`, `XenonPad`, `PS2Pad`. There are 16 buttons.

## Memory (EaglePatch)

| Address | What |
|---|---|
| `0x007A4510` | engine allocator (cdecl, 8 arguments) |
| `0x00924070` | `getNewDescriptor(size, align, var)` |
| `0x01A1E680` | global variable passed to `getNewDescriptor` |
| `0x00916440` | engine delete |

## Graphics and miscellaneous (EaglePatch)

| Address | What |
|---|---|
| `0x00E91422`, `0x00E9116D`, `0x00E91178` | checks that disable MSAA at high resolutions |
| `0x00405495` | jump that enables skipping the startup videos |
| `0x017382D8` | telemetry flag (EaglePatch zeroes it) |

## Recognising the build (EaglePatch)

EaglePatch tells DX9 and DX10 apart by reading the dword at `0x00401376`: it is `0x42D6` in the DX9 one.
In the DX10 one `0x004013DF` is read, which is `0x428D`.

## CRT (found by byte signatures, verified with `tools/harness.py selftest`)

| Address | What |
|---|---|
| `0x00FA6790` | `strlen` |
| `0x00FA9760` | `strcmp` |
| `0x00F9FB80` | `memcpy` (or `memmove`: same initial sequence; there is a second one at `0x00FA0120`) |
| `0x00FA8E10` | probably `_decode_pointer` (called by `_purecall`, uses `TlsGetValue`) |
| `0x01915AC0` | standard CRC-32 table in `.data`, without direct references |
