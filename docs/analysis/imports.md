# Imports and operating system surface

Verified on 2026-11-14 on `AssassinsCreed_Dx9.exe` (Steam build). The tables from the
"Summary by DLL" section downwards are generated and can be rebuilt with:

```sh
python3 -I tools/imports.py "$AC1/AssassinsCreed_Dx9.exe"          # tables
python3 -I tools/imports.py "$AC1/AssassinsCreed_Dx9.exe" --guids  # COM GUIDs
```

"References" is the number of times the address of the IAT cell appears in `.text`:
`call [IAT]`, `jmp [IAT]` and other uses, such as `mov esi, [IAT]` followed by `call esi`. It is a
static measure of how much code depends on a function, not of how many times it gets called.

## What we learn

- **260 functions from 17 DLLs; all of them have at least one reference in the code.** None can
  be ignored a priori.
- **`TlsGetValue` has 2,644 references**, more than all the other KERNEL32 imports put
  together (4,300 in total). The engine keeps per-thread data through the TLS API, not only through
  static TLS (`fs:[0x2C]`). In the runtime it must be implemented as a fast path, not as a
  generic call.
- **Critical sections**: `EnterCriticalSection` 210, `LeaveCriticalSection` 321. These too
  are a hot path (scimitar-recomp had to make them cheaper).
- **`DebugBreak` has 144 references**: they are the engine's asserts left in the build. In the
  runtime a log with the caller's address is worthwhile: it will tell us when we violate one of
  the game's assumptions.
- **KERNEL32 by category** (functions / references): thread 19 / 2,789; synchronization
  18 / 730; file 32 / 171; process and environment 24 / 172; console and debug 11 / 169; locale and
  strings 21 / 96; time 12 / 66; memory 11 / 53; INI files 5 / 39; exceptions 4 / 15.
- **Network**: 30 Winsock functions by ordinal, 170 references. It is enough for `WSAStartup` to fail.
- **Saves and settings**: `SHGetSpecialFolderPathA` and `SHCreateDirectoryExA` (17
  references each) plus the INI files (`GetPrivateProfile*`, `WritePrivateProfileStringA`).
- **Exceptions**: `RaiseException` and `RtlUnwind` are there (15 references in all). At first
  a `throw` will be a fatal error with a backtrace, as scimitar-recomp does.
- The ordinals were resolved with the documented tables: `dsound` #11 is
  `DirectSoundCreate8`, #2 `DirectSoundEnumerateA`; `eax` #6 `EAXDirectSoundCreate8`;
  `oleaut32` #2 `SysAllocString`; `xinput1_3` #2, #3, #4 are `XInputGetState`,
  `XInputSetState`, `XInputGetCapabilities`.

## COM interfaces

Behind 5 imports (`Direct3DCreate9`, `DirectInput8Create`, `DirectSoundCreate8`,
`EAXDirectSoundCreate8`, `CoCreateInstance`) are the COM interfaces, called through vtables.
GUIDs found in `.rdata`:

| GUID | Address | What it implies |
|---|---|---|
| `IID_IDirectInput8A`, `IID_IDirectInput8W` | `0x0171BFFC`, `0x0171BFEC` | DirectInput 8 |
| `IID_IDirectInputDevice8A` | `0x0171BF7C` | devices |
| `GUID_SysKeyboard`, `GUID_SysMouse`, `GUID_XAxis` | `0x0171BE8C`… | keyboard, mouse, axes (joystick too) |
| `IID_IDirectSound8`, `IID_IDirectSoundBuffer8` | `0x0171BA1C`, `0x0171B9FC` | DirectSound 8 |
| `IID_IDirectSound3DBuffer`, `IID_IDirectSound3DListener` | `0x0171B9CC`, `0x0171B9DC` | 3D audio |
| `IID_IKsPropertySet` | `0x0171B97C` | gateway to EAX |
| EAX 2.0, 3.0 property sets and EAX 4.0 context | `0x016A7AF4`… | the game tries EAX from 4 down to 2 |

The Direct3D 9 IIDs do not appear: the game obtains the interfaces from `Direct3DCreate9` and
`CreateDevice` without `QueryInterface`.

**Not done yet:** the list of COM methods actually called (scimitar-recomp mentions
about 120 for `IDirect3DDevice9`). Statically it would require following the interface pointers;
it is more reliable to obtain it at runtime, with stubs that log the method name on the first
call (Phase 2–3).

## Summary by DLL

| DLL | Functions | References in the code | Our module | Phase | Note |
|---|---|---|---|---|---|
| `kernel32.dll` | 157 | 4300 | `win32/kernel32` | 2 |  |
| `user32.dll` | 32 | 83 | `win32/user32` | 2 | one window, messages, cursor, keyboard hooks |
| `ws2_32.dll` | 30 | 170 | `stub` | 2 | network: fails as unavailable |
| `binkw32.dll` | 14 | 32 | `video` | 3 | stdcall |
| `advapi32.dll` | 7 | 15 | `win32/system` | 2 | registry (read-only), random numbers, user name |
| `ole32.dll` | 3 | 6 | `win32/com` | 2 |  |
| `xinput1_3.dll` | 3 | 3 | `input` | 3 |  |
| `shell32.dll` | 2 | 34 | `win32/system` | 2 | special folders: where the saves go |
| `d3dx9_36.dll` | 2 | 2 | `d3d9` | 3 |  |
| `dsound.dll` | 2 | 2 | `audio` | 3 | buffers and listener via COM |
| `wininet.dll` | 2 | 4 | `stub` | 2 | cookies: fails |
| `oleaut32.dll` | 1 | 2 | `win32/com` | 2 |  |
| `eax.dll` | 1 | 2 | `audio` | 3 | can be declared absent |
| `d3d9.dll` | 1 | 1 | `d3d9` | 3 | everything else goes through COM vtables |
| `dinput8.dll` | 1 | 1 | `input` | 3 | keyboard and mouse via COM |
| `winmm.dll` | 1 | 2 | `win32/system` | 2 |  |
| `iphlpapi.dll` | 1 | 1 | `stub` | 2 | answers "no adapters" |

Total: 260 functions from 17 DLLs.

### `kernel32.dll` (157)

| Function | IAT cell | call | jmp | other uses | Category |
|---|---|---|---|---|---|
| `DebugBreak` | `0x01a47c40` | 143 | 1 | 0 | console and debug |
| `OutputDebugStringA` | `0x01a47bac` | 3 | 1 | 0 | console and debug |
| `GetStdHandle` | `0x01a47a98` | 2 | 1 | 0 | console and debug |
| `IsDebuggerPresent` | `0x01a47aec` | 2 | 1 | 0 | console and debug |
| `SetStdHandle` | `0x01a47a60` | 2 | 1 | 0 | console and debug |
| `FatalAppExitA` | `0x01a47a9c` | 1 | 1 | 0 | console and debug |
| `FormatMessageA` | `0x01a47b60` | 1 | 1 | 0 | console and debug |
| `GetConsoleMode` | `0x01a47a48` | 1 | 1 | 0 | console and debug |
| `SetConsoleCtrlHandler` | `0x01a47a94` | 1 | 1 | 0 | console and debug |
| `WriteConsoleA` | `0x01a47a24` | 1 | 1 | 0 | console and debug |
| `WriteConsoleW` | `0x01a47a1c` | 1 | 1 | 0 | console and debug |
| `RaiseException` | `0x01a47c50` | 4 | 1 | 0 | exceptions |
| `SetUnhandledExceptionFilter` | `0x01a47af0` | 4 | 1 | 0 | exceptions |
| `UnhandledExceptionFilter` | `0x01a47af4` | 3 | 1 | 0 | exceptions |
| `RtlUnwind` | `0x01a47b28` | 0 | 1 | 0 | exceptions |
| `CreateFileA` | `0x01a47c3c` | 10 | 1 | 3 | file |
| `WriteFile` | `0x01a47c4c` | 12 | 1 | 1 | file |
| `ReadFile` | `0x01a47ba4` | 11 | 1 | 0 | file |
| `SetFilePointer` | `0x01a47bec` | 9 | 1 | 2 | file |
| `FindClose` | `0x01a47c60` | 10 | 1 | 0 | file |
| `FindFirstFileA` | `0x01a479fc` | 7 | 1 | 0 | file |
| `GetModuleFileNameA` | `0x01a47a54` | 7 | 1 | 0 | file |
| `GetCurrentDirectoryA` | `0x01a47b24` | 5 | 1 | 1 | file |
| `FindNextFileA` | `0x01a47c68` | 2 | 1 | 3 | file |
| `UnlockFile` | `0x01a47b48` | 4 | 1 | 1 | file |
| `CreateDirectoryA` | `0x01a47c48` | 3 | 1 | 1 | file |
| `DeleteFileA` | `0x01a47c6c` | 4 | 1 | 0 | file |
| `GetFileAttributesA` | `0x01a47b5c` | 3 | 1 | 1 | file |
| `GetFileType` | `0x01a47a70` | 4 | 1 | 0 | file |
| `CopyFileA` | `0x01a47b88` | 2 | 1 | 1 | file |
| `FlushFileBuffers` | `0x01a47b6c` | 3 | 1 | 0 | file |
| `GetDriveTypeA` | `0x01a47ae0` | 3 | 1 | 0 | file |
| `LockFile` | `0x01a47b50` | 2 | 1 | 1 | file |
| `RemoveDirectoryA` | `0x01a47c64` | 3 | 1 | 0 | file |
| `GetFileSize` | `0x01a47bf0` | 2 | 1 | 0 | file |
| `GetLogicalDriveStringsA` | `0x01a47bd0` | 2 | 1 | 0 | file |
| `GetOverlappedResult` | `0x01a47ba8` | 2 | 1 | 0 | file |
| `MoveFileA` | `0x01a47b8c` | 2 | 1 | 0 | file |
| `SetCurrentDirectoryA` | `0x01a47a50` | 2 | 1 | 0 | file |
| `SetEndOfFile` | `0x01a47b54` | 2 | 1 | 0 | file |
| `SetFileAttributesA` | `0x01a47c44` | 2 | 1 | 0 | file |
| `GetFileSizeEx` | `0x01a47c38` | 1 | 1 | 0 | file |
| `GetFileTime` | `0x01a47b34` | 1 | 1 | 0 | file |
| `GetTempPathA` | `0x01a47b58` | 1 | 1 | 0 | file |
| `LockFileEx` | `0x01a47b4c` | 1 | 1 | 0 | file |
| `SetFileTime` | `0x01a47adc` | 1 | 1 | 0 | file |
| `SetHandleCount` | `0x01a47b20` | 1 | 1 | 0 | file |
| `WritePrivateProfileStringA` | `0x01a47a74` | 7 | 1 | 5 | ini |
| `GetPrivateProfileIntA` | `0x01a47b1c` | 2 | 1 | 7 | ini |
| `GetPrivateProfileSectionNamesA` | `0x01a47b18` | 5 | 1 | 0 | ini |
| `GetPrivateProfileStringA` | `0x01a47b14` | 4 | 1 | 1 | ini |
| `GetPrivateProfileSectionA` | `0x01a47b30` | 3 | 1 | 0 | ini |
| `WideCharToMultiByte` | `0x01a47b2c` | 11 | 1 | 7 | locale and strings |
| `MultiByteToWideChar` | `0x01a47a58` | 6 | 1 | 5 | locale and strings |
| `GetLocaleInfoA` | `0x01a47a34` | 6 | 1 | 3 | locale and strings |
| `GetCPInfo` | `0x01a47ac0` | 4 | 1 | 1 | locale and strings |
| `EnumSystemLocalesA` | `0x01a47a30` | 4 | 1 | 0 | locale and strings |
| `LCMapStringW` | `0x01a47a88` | 2 | 1 | 2 | locale and strings |
| `GetStringTypeW` | `0x01a47a64` | 2 | 1 | 1 | locale and strings |
| `LCMapStringA` | `0x01a47a8c` | 1 | 1 | 2 | locale and strings |
| `CompareStringW` | `0x01a47a14` | 2 | 1 | 0 | locale and strings |
| `GetLocaleInfoW` | `0x01a47a28` | 0 | 1 | 2 | locale and strings |
| `GetStringTypeA` | `0x01a47a68` | 2 | 1 | 0 | locale and strings |
| `GetUserDefaultLCID` | `0x01a47a38` | 2 | 1 | 0 | locale and strings |
| `IsValidCodePage` | `0x01a47ab0` | 2 | 1 | 0 | locale and strings |
| `CompareStringA` | `0x01a47a18` | 1 | 1 | 0 | locale and strings |
| `GetACP` | `0x01a47ab8` | 1 | 1 | 0 | locale and strings |
| `GetConsoleCP` | `0x01a47a4c` | 1 | 1 | 0 | locale and strings |
| `GetConsoleOutputCP` | `0x01a47a20` | 1 | 1 | 0 | locale and strings |
| `GetDateFormatA` | `0x01a47a3c` | 0 | 1 | 1 | locale and strings |
| `GetOEMCP` | `0x01a47ab4` | 1 | 1 | 0 | locale and strings |
| `GetTimeFormatA` | `0x01a47a40` | 0 | 1 | 1 | locale and strings |
| `IsValidLocale` | `0x01a47a2c` | 1 | 1 | 0 | locale and strings |
| `HeapAlloc` | `0x01a47b0c` | 8 | 1 | 1 | memory |
| `HeapFree` | `0x01a47b10` | 7 | 1 | 1 | memory |
| `VirtualFree` | `0x01a47bfc` | 2 | 3 | 1 | memory |
| `VirtualQuery` | `0x01a47be8` | 4 | 1 | 1 | memory |
| `HeapReAlloc` | `0x01a47ac4` | 4 | 1 | 0 | memory |
| `GetProcessHeap` | `0x01a47b04` | 2 | 1 | 1 | memory |
| `VirtualAlloc` | `0x01a47bf8` | 3 | 1 | 0 | memory |
| `HeapDestroy` | `0x01a47aa4` | 2 | 1 | 0 | memory |
| `HeapCreate` | `0x01a47aa0` | 1 | 1 | 0 | memory |
| `HeapSize` | `0x01a47aa8` | 1 | 1 | 0 | memory |
| `LocalFree` | `0x01a47b64` | 1 | 1 | 0 | memory |
| `GetLastError` | `0x01a47a04` | 72 | 1 | 2 | process and environment |
| `GetModuleHandleA` | `0x01a47a5c` | 13 | 1 | 0 | process and environment |
| `GetProcAddress` | `0x01a47b78` | 7 | 1 | 4 | process and environment |
| `GetCurrentProcess` | `0x01a47bcc` | 8 | 1 | 0 | process and environment |
| `GetVersionExA` | `0x01a47b74` | 7 | 1 | 0 | process and environment |
| `SetLastError` | `0x01a47be4` | 4 | 1 | 2 | process and environment |
| `LoadLibraryA` | `0x01a47b80` | 4 | 1 | 0 | process and environment |
| `FreeLibrary` | `0x01a47b7c` | 3 | 1 | 0 | process and environment |
| `GetFullPathNameA` | `0x01a47b3c` | 0 | 1 | 3 | process and environment |
| `ExitProcess` | `0x01a47b68` | 2 | 1 | 0 | process and environment |
| `FreeEnvironmentStringsA` | `0x01a47a84` | 2 | 1 | 0 | process and environment |
| `GetStartupInfoA` | `0x01a47b00` | 2 | 1 | 0 | process and environment |
| `TerminateProcess` | `0x01a47afc` | 2 | 1 | 0 | process and environment |
| `FreeEnvironmentStringsW` | `0x01a47a7c` | 1 | 1 | 0 | process and environment |
| `GetCommandLineA` | `0x01a47b08` | 1 | 1 | 0 | process and environment |
| `GetComputerNameExA` | `0x01a47b70` | 1 | 1 | 0 | process and environment |
| `GetCurrentProcessId` | `0x01a47a6c` | 1 | 1 | 0 | process and environment |
| `GetEnvironmentStrings` | `0x01a47a80` | 1 | 1 | 0 | process and environment |
| `GetEnvironmentStringsW` | `0x01a47af8` | 0 | 1 | 1 | process and environment |
| `GetProcessAffinityMask` | `0x01a47bc8` | 1 | 1 | 0 | process and environment |
| `GetSystemInfo` | `0x01a47c5c` | 1 | 1 | 0 | process and environment |
| `GetVersion` | `0x01a47c2c` | 1 | 1 | 0 | process and environment |
| `SetEnvironmentVariableA` | `0x01a47a10` | 1 | 1 | 0 | process and environment |
| `SetErrorMode` | `0x01a47a78` | 1 | 1 | 0 | process and environment |
| `LeaveCriticalSection` | `0x01a47b94` | 294 | 21 | 6 | synchronization |
| `EnterCriticalSection` | `0x01a47b98` | 208 | 1 | 1 | synchronization |
| `DeleteCriticalSection` | `0x01a47c10` | 39 | 1 | 7 | synchronization |
| `InitializeCriticalSectionAndSpinCount` | `0x01a47c08` | 36 | 1 | 5 | synchronization |
| `WaitForSingleObject` | `0x01a47bdc` | 18 | 1 | 3 | synchronization |
| `InterlockedDecrement` | `0x01a47abc` | 11 | 1 | 4 | synchronization |
| `SetEvent` | `0x01a47c20` | 13 | 2 | 1 | synchronization |
| `CreateEventA` | `0x01a47c18` | 5 | 1 | 3 | synchronization |
| `InitializeCriticalSection` | `0x01a47c0c` | 7 | 2 | 0 | synchronization |
| `CreateSemaphoreA` | `0x01a47a00` | 6 | 1 | 0 | synchronization |
| `InterlockedIncrement` | `0x01a47b40` | 3 | 1 | 3 | synchronization |
| `ReleaseSemaphore` | `0x01a47c28` | 4 | 1 | 0 | synchronization |
| `TryEnterCriticalSection` | `0x01a47b9c` | 3 | 1 | 1 | synchronization |
| `InterlockedExchange` | `0x01a47a90` | 2 | 1 | 0 | synchronization |
| `InterlockedExchangeAdd` | `0x01a47a08` | 2 | 1 | 0 | synchronization |
| `ResetEvent` | `0x01a47c24` | 1 | 2 | 0 | synchronization |
| `WaitForMultipleObjects` | `0x01a47c14` | 2 | 1 | 0 | synchronization |
| `PulseEvent` | `0x01a47c1c` | 1 | 1 | 0 | synchronization |
| `QueryPerformanceCounter` | `0x01a47bb4` | 19 | 1 | 1 | time |
| `FileTimeToLocalFileTime` | `0x01a47ae4` | 6 | 1 | 0 | time |
| `FileTimeToSystemTime` | `0x01a47ae8` | 6 | 1 | 0 | time |
| `QueryPerformanceFrequency` | `0x01a47bb8` | 6 | 1 | 0 | time |
| `GetSystemTimeAsFileTime` | `0x01a47b38` | 5 | 1 | 0 | time |
| `GetLocalTime` | `0x01a47ac8` | 2 | 1 | 0 | time |
| `GetProcessTimes` | `0x01a47bf4` | 2 | 1 | 0 | time |
| `GetTickCount` | `0x01a47bb0` | 1 | 2 | 0 | time |
| `LocalFileTimeToFileTime` | `0x01a47ad8` | 2 | 1 | 0 | time |
| `GetSystemTime` | `0x01a47b44` | 1 | 1 | 0 | time |
| `GetTimeZoneInformation` | `0x01a47a44` | 1 | 1 | 0 | time |
| `SystemTimeToFileTime` | `0x01a47ad4` | 0 | 1 | 1 | time |
| `TlsGetValue` | `0x01a47b90` | 2260 | 1 | 383 | thread |
| `CloseHandle` | `0x01a47ba0` | 26 | 2 | 9 | thread |
| `Sleep` | `0x01a47bbc` | 15 | 1 | 3 | thread |
| `GetCurrentThreadId` | `0x01a47bd4` | 12 | 4 | 0 | thread |
| `TlsSetValue` | `0x01a47c04` | 10 | 1 | 1 | thread |
| `TlsAlloc` | `0x01a47c00` | 9 | 2 | 0 | thread |
| `TlsFree` | `0x01a47aac` | 7 | 1 | 1 | thread |
| `GetCurrentThread` | `0x01a47bc4` | 4 | 2 | 0 | thread |
| `OpenThread` | `0x01a47be0` | 5 | 1 | 0 | thread |
| `ResumeThread` | `0x01a47c58` | 3 | 1 | 1 | thread |
| `SetThreadAffinityMask` | `0x01a47bc0` | 3 | 1 | 0 | thread |
| `CreateThread` | `0x01a47acc` | 2 | 1 | 0 | thread |
| `ExitThread` | `0x01a47ad0` | 2 | 1 | 0 | thread |
| `SetThreadPriority` | `0x01a47c30` | 2 | 1 | 0 | thread |
| `SwitchToThread` | `0x01a47bd8` | 0 | 2 | 1 | thread |
| `DuplicateHandle` | `0x01a47b84` | 1 | 1 | 0 | thread |
| `GetExitCodeThread` | `0x01a47a0c` | 1 | 1 | 0 | thread |
| `SetThreadPriorityBoost` | `0x01a47c34` | 1 | 1 | 0 | thread |
| `SuspendThread` | `0x01a47c54` | 1 | 1 | 0 | thread |

### `user32.dll` (32)

| Function | IAT cell | call | jmp | other uses |
|---|---|---|---|---|
| `DefWindowProcA` | `0x01a47dc8` | 8 | 1 | 0 |
| `MessageBoxA` | `0x01a47de0` | 6 | 1 | 0 |
| `GetSystemMetrics` | `0x01a47dac` | 3 | 1 | 0 |
| `SystemParametersInfoA` | `0x01a47d7c` | 0 | 1 | 3 |
| `CallNextHookEx` | `0x01a47dcc` | 2 | 1 | 0 |
| `ClipCursor` | `0x01a47d90` | 2 | 1 | 0 |
| `PeekMessageA` | `0x01a47da0` | 1 | 1 | 1 |
| `AdjustWindowRect` | `0x01a47dec` | 1 | 1 | 0 |
| `CreateWindowExA` | `0x01a47ddc` | 1 | 1 | 0 |
| `DestroyWindow` | `0x01a47dc4` | 1 | 1 | 0 |
| `DispatchMessageA` | `0x01a47d98` | 0 | 1 | 1 |
| `EnumDisplayMonitors` | `0x01a47d94` | 1 | 1 | 0 |
| `GetClientRect` | `0x01a47db0` | 1 | 1 | 0 |
| `GetCursorPos` | `0x01a47d84` | 1 | 1 | 0 |
| `GetDesktopWindow` | `0x01a47df0` | 1 | 1 | 0 |
| `GetKeyboardState` | `0x01a47da4` | 1 | 1 | 0 |
| `GetWindowRect` | `0x01a47db4` | 1 | 1 | 0 |
| `IsIconic` | `0x01a47dbc` | 1 | 1 | 0 |
| `IsWindow` | `0x01a47d88` | 1 | 1 | 0 |
| `IsWindowVisible` | `0x01a47d8c` | 1 | 1 | 0 |
| `LoadCursorA` | `0x01a47de4` | 1 | 1 | 0 |
| `LoadStringA` | `0x01a47df8` | 1 | 1 | 0 |
| `PostQuitMessage` | `0x01a47dc0` | 1 | 1 | 0 |
| `RegisterClassExA` | `0x01a47de8` | 1 | 1 | 0 |
| `ScreenToClient` | `0x01a47d80` | 1 | 1 | 0 |
| `SetWindowsHookExA` | `0x01a47db8` | 1 | 1 | 0 |
| `ShowCursor` | `0x01a47dd0` | 1 | 1 | 0 |
| `ShowWindow` | `0x01a47dd8` | 1 | 1 | 0 |
| `ToUnicode` | `0x01a47da8` | 1 | 1 | 0 |
| `TranslateMessage` | `0x01a47d9c` | 0 | 1 | 1 |
| `UnhookWindowsHookEx` | `0x01a47df4` | 1 | 1 | 0 |
| `UpdateWindow` | `0x01a47dd4` | 1 | 1 | 0 |

### `ws2_32.dll` (30)

| Function | IAT cell | call | jmp | other uses |
|---|---|---|---|---|
| `WSAGetLastError (#111)` | `0x01a47ed4` | 27 | 1 | 0 |
| `closesocket (#3)` | `0x01a47ec0` | 12 | 1 | 0 |
| `htons (#9)` | `0x01a47ee4` | 8 | 1 | 1 |
| `setsockopt (#21)` | `0x01a47ecc` | 8 | 1 | 1 |
| `getsockopt (#7)` | `0x01a47f18` | 8 | 1 | 0 |
| `inet_ntoa (#12)` | `0x01a47ed8` | 7 | 1 | 1 |
| `select (#18)` | `0x01a47ea8` | 7 | 1 | 0 |
| `inet_addr (#11)` | `0x01a47ef4` | 6 | 1 | 0 |
| `ioctlsocket (#10)` | `0x01a47eb0` | 6 | 1 | 0 |
| `socket (#23)` | `0x01a47ea4` | 6 | 1 | 0 |
| `gethostbyname (#52)` | `0x01a47eec` | 5 | 1 | 0 |
| `bind (#2)` | `0x01a47eb8` | 4 | 1 | 0 |
| `WSACleanup (#116)` | `0x01a47ec4` | 2 | 2 | 0 |
| `htonl (#8)` | `0x01a47ee8` | 3 | 1 | 0 |
| `ntohl (#14)` | `0x01a47edc` | 3 | 1 | 0 |
| `send (#19)` | `0x01a47f00` | 3 | 1 | 0 |
| `sendto (#20)` | `0x01a47ed0` | 3 | 1 | 0 |
| `WSAStartup (#115)` | `0x01a47ef8` | 2 | 1 | 0 |
| `accept (#1)` | `0x01a47f10` | 2 | 1 | 0 |
| `connect (#4)` | `0x01a47eb4` | 2 | 1 | 0 |
| `gethostbyaddr (#51)` | `0x01a47ef0` | 2 | 1 | 0 |
| `listen (#13)` | `0x01a47f0c` | 2 | 1 | 0 |
| `recv (#16)` | `0x01a47f08` | 2 | 1 | 0 |
| `recvfrom (#17)` | `0x01a47f04` | 2 | 1 | 0 |
| `gethostname (#57)` | `0x01a47f14` | 1 | 1 | 0 |
| `getpeername (#5)` | `0x01a47efc` | 1 | 1 | 0 |
| `getsockname (#6)` | `0x01a47ebc` | 1 | 1 | 0 |
| `ntohs (#15)` | `0x01a47ee0` | 1 | 1 | 0 |
| `WSAAsyncSelect (#101)` | `0x01a47ec8` | 0 | 1 | 0 |
| `__WSAFDIsSet (#151)` | `0x01a47eac` | 0 | 1 | 0 |

### `binkw32.dll` (14)

| Function | IAT cell | call | jmp | other uses |
|---|---|---|---|---|
| `_BinkGetFrameBuffersInfo@8` | `0x01a47fc4` | 3 | 1 | 0 |
| `_BinkPause@8` | `0x01a47fb8` | 2 | 1 | 0 |
| `_BinkSetVolume@12` | `0x01a47fb4` | 1 | 1 | 1 |
| `_BinkClose@4` | `0x01a47fbc` | 1 | 1 | 0 |
| `_BinkCopyToBuffer@28` | `0x01a47fc8` | 1 | 1 | 0 |
| `_BinkDoFrame@4` | `0x01a47fac` | 0 | 1 | 1 |
| `_BinkNextFrame@4` | `0x01a47fa4` | 0 | 1 | 1 |
| `_BinkOpen@8` | `0x01a47fb0` | 1 | 1 | 0 |
| `_BinkOpenDirectSound@4` | `0x01a47fcc` | 0 | 1 | 1 |
| `_BinkSetMemory@8` | `0x01a47fc0` | 1 | 1 | 0 |
| `_BinkSetSoundSystem@8` | `0x01a47fa8` | 1 | 1 | 0 |
| `_BinkSetSoundTrack@8` | `0x01a47f9c` | 1 | 1 | 0 |
| `_BinkShouldSkip@4` | `0x01a47fa0` | 0 | 1 | 1 |
| `_BinkWait@4` | `0x01a47f98` | 1 | 1 | 0 |

### `advapi32.dll` (7)

| Function | IAT cell | call | jmp | other uses |
|---|---|---|---|---|
| `CryptReleaseContext` | `0x01a47924` | 2 | 1 | 0 |
| `CryptAcquireContextA` | `0x01a47930` | 0 | 1 | 1 |
| `CryptGenRandom` | `0x01a47920` | 1 | 1 | 0 |
| `GetUserNameA` | `0x01a47928` | 1 | 1 | 0 |
| `RegCloseKey` | `0x01a4792c` | 1 | 1 | 0 |
| `RegOpenKeyExA` | `0x01a4791c` | 1 | 1 | 0 |
| `RegQueryValueExA` | `0x01a47934` | 1 | 1 | 0 |

### `ole32.dll` (3)

| Function | IAT cell | call | jmp | other uses |
|---|---|---|---|---|
| `CoCreateInstance` | `0x01a4809c` | 1 | 1 | 0 |
| `CoInitialize` | `0x01a48098` | 1 | 1 | 0 |
| `CoUninitialize` | `0x01a480a0` | 1 | 1 | 0 |

### `xinput1_3.dll` (3)

| Function | IAT cell | call | jmp | other uses |
|---|---|---|---|---|
| `XInputGetCapabilities (#4)` | `0x01a47f60` | 0 | 1 | 0 |
| `XInputGetState (#2)` | `0x01a47f68` | 0 | 1 | 0 |
| `XInputSetState (#3)` | `0x01a47f64` | 0 | 1 | 0 |

### `shell32.dll` (2)

| Function | IAT cell | call | jmp | other uses |
|---|---|---|---|---|
| `SHCreateDirectoryExA` | `0x01a47d4c` | 16 | 1 | 0 |
| `SHGetSpecialFolderPathA` | `0x01a47d48` | 16 | 1 | 0 |

### `d3dx9_36.dll` (2)

| Function | IAT cell | call | jmp | other uses |
|---|---|---|---|---|
| `D3DXCheckTextureRequirements` | `0x01a48038` | 0 | 1 | 0 |
| `D3DXGetShaderConstantTable` | `0x01a48034` | 0 | 1 | 0 |

### `dsound.dll` (2)

| Function | IAT cell | call | jmp | other uses |
|---|---|---|---|---|
| `DirectSoundCreate8 (#11)` | `0x01a47998` | 0 | 1 | 0 |
| `DirectSoundEnumerateA (#2)` | `0x01a4799c` | 0 | 1 | 0 |

### `wininet.dll` (2)

| Function | IAT cell | call | jmp | other uses |
|---|---|---|---|---|
| `InternetGetCookieW` | `0x01a47e40` | 0 | 1 | 1 |
| `InternetSetCookieW` | `0x01a47e44` | 1 | 1 | 0 |

### `oleaut32.dll` (1)

| Function | IAT cell | call | jmp | other uses |
|---|---|---|---|---|
| `SysAllocString (#2)` | `0x01a47d18` | 1 | 1 | 0 |

### `eax.dll` (1)

| Function | IAT cell | call | jmp | other uses |
|---|---|---|---|---|
| `EAXDirectSoundCreate8 (#6)` | `0x01a479cc` | 1 | 1 | 0 |

### `d3d9.dll` (1)

| Function | IAT cell | call | jmp | other uses |
|---|---|---|---|---|
| `Direct3DCreate9` | `0x01a48004` | 0 | 1 | 0 |

### `dinput8.dll` (1)

| Function | IAT cell | call | jmp | other uses |
|---|---|---|---|---|
| `DirectInput8Create` | `0x01a47968` | 0 | 1 | 0 |

### `winmm.dll` (1)

| Function | IAT cell | call | jmp | other uses |
|---|---|---|---|---|
| `waveOutGetDevCapsA` | `0x01a47e74` | 1 | 1 | 0 |

### `iphlpapi.dll` (1)

| Function | IAT cell | call | jmp | other uses |
|---|---|---|---|---|
| `GetAdaptersInfo` | `0x01a48068` | 0 | 1 | 0 |

