# Import e superficie del sistema operativo

Verificato il 2026-11-14 su `AssassinsCreed_Dx9.exe` (build Steam). Le tabelle dalla sezione
"Riepilogo per DLL" in giù sono generate e si rifanno con:

```sh
python3 -I tools/imports.py "$AC1/AssassinsCreed_Dx9.exe"          # tabelle
python3 -I tools/imports.py "$AC1/AssassinsCreed_Dx9.exe" --guids  # GUID COM
```

"Riferimenti" è il numero di volte in cui l'indirizzo della cella IAT compare in `.text`:
`call [IAT]`, `jmp [IAT]` e altri usi, come `mov esi, [IAT]` seguito da `call esi`. È una
misura statica di quanto codice dipende da una funzione, non di quante volte viene chiamata.

## Cosa si ricava

- **260 funzioni da 17 DLL; tutte hanno almeno un riferimento nel codice.** Nessuna si può
  ignorare a priori.
- **`TlsGetValue` ha 2.644 riferimenti**, più di tutti gli altri import di KERNEL32 messi
  insieme (4.300 in totale). Il motore tiene dati per thread tramite l'API TLS, non solo tramite
  la TLS statica (`fs:[0x2C]`). Nel runtime va implementata come percorso veloce, non come
  chiamata generica.
- **Sezioni critiche**: `EnterCriticalSection` 210, `LeaveCriticalSection` 321. Anche queste
  sono percorso caldo (scimitar-recomp le ha dovute rendere più economiche).
- **`DebugBreak` ha 144 riferimenti**: sono gli assert del motore rimasti nella build. Nel
  runtime conviene un log con l'indirizzo del chiamante: ci dirà quando violiamo un'ipotesi
  del gioco.
- **KERNEL32 per categoria** (funzioni / riferimenti): thread 19 / 2.789; sincronizzazione
  18 / 730; file 32 / 171; processo e ambiente 24 / 172; console e debug 11 / 169; locale e
  stringhe 21 / 96; tempo 12 / 66; memoria 11 / 53; file INI 5 / 39; eccezioni 4 / 15.
- **Rete**: 30 funzioni Winsock per ordinale, 170 riferimenti. Basta che `WSAStartup` fallisca.
- **Salvataggi e impostazioni**: `SHGetSpecialFolderPathA` e `SHCreateDirectoryExA` (17
  riferimenti ciascuna) più i file INI (`GetPrivateProfile*`, `WritePrivateProfileStringA`).
- **Eccezioni**: `RaiseException` e `RtlUnwind` ci sono (15 riferimenti in tutto). All'inizio
  un `throw` sarà un errore fatale con backtrace, come fa scimitar-recomp.
- Gli ordinali sono stati risolti con le tabelle documentate: `dsound` #11 è
  `DirectSoundCreate8`, #2 `DirectSoundEnumerateA`; `eax` #6 `EAXDirectSoundCreate8`;
  `oleaut32` #2 `SysAllocString`; `xinput1_3` #2, #3, #4 sono `XInputGetState`,
  `XInputSetState`, `XInputGetCapabilities`.

## Interfacce COM

Dietro 5 import (`Direct3DCreate9`, `DirectInput8Create`, `DirectSoundCreate8`,
`EAXDirectSoundCreate8`, `CoCreateInstance`) ci sono le interfacce COM, chiamate tramite vtable.
GUID trovati in `.rdata`:

| GUID | Indirizzo | Cosa implica |
|---|---|---|
| `IID_IDirectInput8A`, `IID_IDirectInput8W` | `0x0171BFFC`, `0x0171BFEC` | DirectInput 8 |
| `IID_IDirectInputDevice8A` | `0x0171BF7C` | dispositivi |
| `GUID_SysKeyboard`, `GUID_SysMouse`, `GUID_XAxis` | `0x0171BE8C`… | tastiera, mouse, assi (anche joystick) |
| `IID_IDirectSound8`, `IID_IDirectSoundBuffer8` | `0x0171BA1C`, `0x0171B9FC` | DirectSound 8 |
| `IID_IDirectSound3DBuffer`, `IID_IDirectSound3DListener` | `0x0171B9CC`, `0x0171B9DC` | audio 3D |
| `IID_IKsPropertySet` | `0x0171B97C` | porta d'accesso a EAX |
| Property set EAX 2.0, 3.0 e contesto EAX 4.0 | `0x016A7AF4`… | il gioco prova EAX dalla 4 alla 2 |

Gli IID di Direct3D 9 non compaiono: il gioco ottiene le interfacce da `Direct3DCreate9` e
`CreateDevice` senza `QueryInterface`.

**Non ancora fatto:** l'elenco dei metodi COM effettivamente chiamati (scimitar-recomp parla di
circa 120 per `IDirect3DDevice9`). Staticamente servirebbe seguire i puntatori alle interfacce;
è più affidabile ricavarlo a runtime, con stub che registrano il nome del metodo alla prima
chiamata (Fase 2–3).

## Riepilogo per DLL

| DLL | Funzioni | Riferimenti nel codice | Modulo nostro | Fase | Nota |
|---|---|---|---|---|---|
| `kernel32.dll` | 157 | 4300 | `win32/kernel32` | 2 |  |
| `user32.dll` | 32 | 83 | `win32/user32` | 2 | una finestra, messaggi, cursore, hook di tastiera |
| `ws2_32.dll` | 30 | 170 | `stub` | 2 | rete: fallisce come non disponibile |
| `binkw32.dll` | 14 | 32 | `video` | 3 | stdcall |
| `advapi32.dll` | 7 | 15 | `win32/system` | 2 | registro (sola lettura), numeri casuali, nome utente |
| `ole32.dll` | 3 | 6 | `win32/com` | 2 |  |
| `xinput1_3.dll` | 3 | 3 | `input` | 3 |  |
| `shell32.dll` | 2 | 34 | `win32/system` | 2 | cartelle speciali: dove vanno i salvataggi |
| `d3dx9_36.dll` | 2 | 2 | `d3d9` | 3 |  |
| `dsound.dll` | 2 | 2 | `audio` | 3 | buffer e listener via COM |
| `wininet.dll` | 2 | 4 | `stub` | 2 | cookie: fallisce |
| `oleaut32.dll` | 1 | 2 | `win32/com` | 2 |  |
| `eax.dll` | 1 | 2 | `audio` | 3 | si può dichiarare assente |
| `d3d9.dll` | 1 | 1 | `d3d9` | 3 | tutto il resto passa da vtable COM |
| `dinput8.dll` | 1 | 1 | `input` | 3 | tastiera e mouse via COM |
| `winmm.dll` | 1 | 2 | `win32/system` | 2 |  |
| `iphlpapi.dll` | 1 | 1 | `stub` | 2 | risponde "nessuna scheda" |

Totale: 260 funzioni da 17 DLL.

### `kernel32.dll` (157)

| Funzione | Cella IAT | call | jmp | altri usi | Categoria |
|---|---|---|---|---|---|
| `DebugBreak` | `0x01a47c40` | 143 | 1 | 0 | console e debug |
| `OutputDebugStringA` | `0x01a47bac` | 3 | 1 | 0 | console e debug |
| `GetStdHandle` | `0x01a47a98` | 2 | 1 | 0 | console e debug |
| `IsDebuggerPresent` | `0x01a47aec` | 2 | 1 | 0 | console e debug |
| `SetStdHandle` | `0x01a47a60` | 2 | 1 | 0 | console e debug |
| `FatalAppExitA` | `0x01a47a9c` | 1 | 1 | 0 | console e debug |
| `FormatMessageA` | `0x01a47b60` | 1 | 1 | 0 | console e debug |
| `GetConsoleMode` | `0x01a47a48` | 1 | 1 | 0 | console e debug |
| `SetConsoleCtrlHandler` | `0x01a47a94` | 1 | 1 | 0 | console e debug |
| `WriteConsoleA` | `0x01a47a24` | 1 | 1 | 0 | console e debug |
| `WriteConsoleW` | `0x01a47a1c` | 1 | 1 | 0 | console e debug |
| `RaiseException` | `0x01a47c50` | 4 | 1 | 0 | eccezioni |
| `SetUnhandledExceptionFilter` | `0x01a47af0` | 4 | 1 | 0 | eccezioni |
| `UnhandledExceptionFilter` | `0x01a47af4` | 3 | 1 | 0 | eccezioni |
| `RtlUnwind` | `0x01a47b28` | 0 | 1 | 0 | eccezioni |
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
| `WideCharToMultiByte` | `0x01a47b2c` | 11 | 1 | 7 | locale e stringhe |
| `MultiByteToWideChar` | `0x01a47a58` | 6 | 1 | 5 | locale e stringhe |
| `GetLocaleInfoA` | `0x01a47a34` | 6 | 1 | 3 | locale e stringhe |
| `GetCPInfo` | `0x01a47ac0` | 4 | 1 | 1 | locale e stringhe |
| `EnumSystemLocalesA` | `0x01a47a30` | 4 | 1 | 0 | locale e stringhe |
| `LCMapStringW` | `0x01a47a88` | 2 | 1 | 2 | locale e stringhe |
| `GetStringTypeW` | `0x01a47a64` | 2 | 1 | 1 | locale e stringhe |
| `LCMapStringA` | `0x01a47a8c` | 1 | 1 | 2 | locale e stringhe |
| `CompareStringW` | `0x01a47a14` | 2 | 1 | 0 | locale e stringhe |
| `GetLocaleInfoW` | `0x01a47a28` | 0 | 1 | 2 | locale e stringhe |
| `GetStringTypeA` | `0x01a47a68` | 2 | 1 | 0 | locale e stringhe |
| `GetUserDefaultLCID` | `0x01a47a38` | 2 | 1 | 0 | locale e stringhe |
| `IsValidCodePage` | `0x01a47ab0` | 2 | 1 | 0 | locale e stringhe |
| `CompareStringA` | `0x01a47a18` | 1 | 1 | 0 | locale e stringhe |
| `GetACP` | `0x01a47ab8` | 1 | 1 | 0 | locale e stringhe |
| `GetConsoleCP` | `0x01a47a4c` | 1 | 1 | 0 | locale e stringhe |
| `GetConsoleOutputCP` | `0x01a47a20` | 1 | 1 | 0 | locale e stringhe |
| `GetDateFormatA` | `0x01a47a3c` | 0 | 1 | 1 | locale e stringhe |
| `GetOEMCP` | `0x01a47ab4` | 1 | 1 | 0 | locale e stringhe |
| `GetTimeFormatA` | `0x01a47a40` | 0 | 1 | 1 | locale e stringhe |
| `IsValidLocale` | `0x01a47a2c` | 1 | 1 | 0 | locale e stringhe |
| `HeapAlloc` | `0x01a47b0c` | 8 | 1 | 1 | memoria |
| `HeapFree` | `0x01a47b10` | 7 | 1 | 1 | memoria |
| `VirtualFree` | `0x01a47bfc` | 2 | 3 | 1 | memoria |
| `VirtualQuery` | `0x01a47be8` | 4 | 1 | 1 | memoria |
| `HeapReAlloc` | `0x01a47ac4` | 4 | 1 | 0 | memoria |
| `GetProcessHeap` | `0x01a47b04` | 2 | 1 | 1 | memoria |
| `VirtualAlloc` | `0x01a47bf8` | 3 | 1 | 0 | memoria |
| `HeapDestroy` | `0x01a47aa4` | 2 | 1 | 0 | memoria |
| `HeapCreate` | `0x01a47aa0` | 1 | 1 | 0 | memoria |
| `HeapSize` | `0x01a47aa8` | 1 | 1 | 0 | memoria |
| `LocalFree` | `0x01a47b64` | 1 | 1 | 0 | memoria |
| `GetLastError` | `0x01a47a04` | 72 | 1 | 2 | processo e ambiente |
| `GetModuleHandleA` | `0x01a47a5c` | 13 | 1 | 0 | processo e ambiente |
| `GetProcAddress` | `0x01a47b78` | 7 | 1 | 4 | processo e ambiente |
| `GetCurrentProcess` | `0x01a47bcc` | 8 | 1 | 0 | processo e ambiente |
| `GetVersionExA` | `0x01a47b74` | 7 | 1 | 0 | processo e ambiente |
| `SetLastError` | `0x01a47be4` | 4 | 1 | 2 | processo e ambiente |
| `LoadLibraryA` | `0x01a47b80` | 4 | 1 | 0 | processo e ambiente |
| `FreeLibrary` | `0x01a47b7c` | 3 | 1 | 0 | processo e ambiente |
| `GetFullPathNameA` | `0x01a47b3c` | 0 | 1 | 3 | processo e ambiente |
| `ExitProcess` | `0x01a47b68` | 2 | 1 | 0 | processo e ambiente |
| `FreeEnvironmentStringsA` | `0x01a47a84` | 2 | 1 | 0 | processo e ambiente |
| `GetStartupInfoA` | `0x01a47b00` | 2 | 1 | 0 | processo e ambiente |
| `TerminateProcess` | `0x01a47afc` | 2 | 1 | 0 | processo e ambiente |
| `FreeEnvironmentStringsW` | `0x01a47a7c` | 1 | 1 | 0 | processo e ambiente |
| `GetCommandLineA` | `0x01a47b08` | 1 | 1 | 0 | processo e ambiente |
| `GetComputerNameExA` | `0x01a47b70` | 1 | 1 | 0 | processo e ambiente |
| `GetCurrentProcessId` | `0x01a47a6c` | 1 | 1 | 0 | processo e ambiente |
| `GetEnvironmentStrings` | `0x01a47a80` | 1 | 1 | 0 | processo e ambiente |
| `GetEnvironmentStringsW` | `0x01a47af8` | 0 | 1 | 1 | processo e ambiente |
| `GetProcessAffinityMask` | `0x01a47bc8` | 1 | 1 | 0 | processo e ambiente |
| `GetSystemInfo` | `0x01a47c5c` | 1 | 1 | 0 | processo e ambiente |
| `GetVersion` | `0x01a47c2c` | 1 | 1 | 0 | processo e ambiente |
| `SetEnvironmentVariableA` | `0x01a47a10` | 1 | 1 | 0 | processo e ambiente |
| `SetErrorMode` | `0x01a47a78` | 1 | 1 | 0 | processo e ambiente |
| `LeaveCriticalSection` | `0x01a47b94` | 294 | 21 | 6 | sincronizzazione |
| `EnterCriticalSection` | `0x01a47b98` | 208 | 1 | 1 | sincronizzazione |
| `DeleteCriticalSection` | `0x01a47c10` | 39 | 1 | 7 | sincronizzazione |
| `InitializeCriticalSectionAndSpinCount` | `0x01a47c08` | 36 | 1 | 5 | sincronizzazione |
| `WaitForSingleObject` | `0x01a47bdc` | 18 | 1 | 3 | sincronizzazione |
| `InterlockedDecrement` | `0x01a47abc` | 11 | 1 | 4 | sincronizzazione |
| `SetEvent` | `0x01a47c20` | 13 | 2 | 1 | sincronizzazione |
| `CreateEventA` | `0x01a47c18` | 5 | 1 | 3 | sincronizzazione |
| `InitializeCriticalSection` | `0x01a47c0c` | 7 | 2 | 0 | sincronizzazione |
| `CreateSemaphoreA` | `0x01a47a00` | 6 | 1 | 0 | sincronizzazione |
| `InterlockedIncrement` | `0x01a47b40` | 3 | 1 | 3 | sincronizzazione |
| `ReleaseSemaphore` | `0x01a47c28` | 4 | 1 | 0 | sincronizzazione |
| `TryEnterCriticalSection` | `0x01a47b9c` | 3 | 1 | 1 | sincronizzazione |
| `InterlockedExchange` | `0x01a47a90` | 2 | 1 | 0 | sincronizzazione |
| `InterlockedExchangeAdd` | `0x01a47a08` | 2 | 1 | 0 | sincronizzazione |
| `ResetEvent` | `0x01a47c24` | 1 | 2 | 0 | sincronizzazione |
| `WaitForMultipleObjects` | `0x01a47c14` | 2 | 1 | 0 | sincronizzazione |
| `PulseEvent` | `0x01a47c1c` | 1 | 1 | 0 | sincronizzazione |
| `QueryPerformanceCounter` | `0x01a47bb4` | 19 | 1 | 1 | tempo |
| `FileTimeToLocalFileTime` | `0x01a47ae4` | 6 | 1 | 0 | tempo |
| `FileTimeToSystemTime` | `0x01a47ae8` | 6 | 1 | 0 | tempo |
| `QueryPerformanceFrequency` | `0x01a47bb8` | 6 | 1 | 0 | tempo |
| `GetSystemTimeAsFileTime` | `0x01a47b38` | 5 | 1 | 0 | tempo |
| `GetLocalTime` | `0x01a47ac8` | 2 | 1 | 0 | tempo |
| `GetProcessTimes` | `0x01a47bf4` | 2 | 1 | 0 | tempo |
| `GetTickCount` | `0x01a47bb0` | 1 | 2 | 0 | tempo |
| `LocalFileTimeToFileTime` | `0x01a47ad8` | 2 | 1 | 0 | tempo |
| `GetSystemTime` | `0x01a47b44` | 1 | 1 | 0 | tempo |
| `GetTimeZoneInformation` | `0x01a47a44` | 1 | 1 | 0 | tempo |
| `SystemTimeToFileTime` | `0x01a47ad4` | 0 | 1 | 1 | tempo |
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

| Funzione | Cella IAT | call | jmp | altri usi |
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

| Funzione | Cella IAT | call | jmp | altri usi |
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

| Funzione | Cella IAT | call | jmp | altri usi |
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

| Funzione | Cella IAT | call | jmp | altri usi |
|---|---|---|---|---|
| `CryptReleaseContext` | `0x01a47924` | 2 | 1 | 0 |
| `CryptAcquireContextA` | `0x01a47930` | 0 | 1 | 1 |
| `CryptGenRandom` | `0x01a47920` | 1 | 1 | 0 |
| `GetUserNameA` | `0x01a47928` | 1 | 1 | 0 |
| `RegCloseKey` | `0x01a4792c` | 1 | 1 | 0 |
| `RegOpenKeyExA` | `0x01a4791c` | 1 | 1 | 0 |
| `RegQueryValueExA` | `0x01a47934` | 1 | 1 | 0 |

### `ole32.dll` (3)

| Funzione | Cella IAT | call | jmp | altri usi |
|---|---|---|---|---|
| `CoCreateInstance` | `0x01a4809c` | 1 | 1 | 0 |
| `CoInitialize` | `0x01a48098` | 1 | 1 | 0 |
| `CoUninitialize` | `0x01a480a0` | 1 | 1 | 0 |

### `xinput1_3.dll` (3)

| Funzione | Cella IAT | call | jmp | altri usi |
|---|---|---|---|---|
| `XInputGetCapabilities (#4)` | `0x01a47f60` | 0 | 1 | 0 |
| `XInputGetState (#2)` | `0x01a47f68` | 0 | 1 | 0 |
| `XInputSetState (#3)` | `0x01a47f64` | 0 | 1 | 0 |

### `shell32.dll` (2)

| Funzione | Cella IAT | call | jmp | altri usi |
|---|---|---|---|---|
| `SHCreateDirectoryExA` | `0x01a47d4c` | 16 | 1 | 0 |
| `SHGetSpecialFolderPathA` | `0x01a47d48` | 16 | 1 | 0 |

### `d3dx9_36.dll` (2)

| Funzione | Cella IAT | call | jmp | altri usi |
|---|---|---|---|---|
| `D3DXCheckTextureRequirements` | `0x01a48038` | 0 | 1 | 0 |
| `D3DXGetShaderConstantTable` | `0x01a48034` | 0 | 1 | 0 |

### `dsound.dll` (2)

| Funzione | Cella IAT | call | jmp | altri usi |
|---|---|---|---|---|
| `DirectSoundCreate8 (#11)` | `0x01a47998` | 0 | 1 | 0 |
| `DirectSoundEnumerateA (#2)` | `0x01a4799c` | 0 | 1 | 0 |

### `wininet.dll` (2)

| Funzione | Cella IAT | call | jmp | altri usi |
|---|---|---|---|---|
| `InternetGetCookieW` | `0x01a47e40` | 0 | 1 | 1 |
| `InternetSetCookieW` | `0x01a47e44` | 1 | 1 | 0 |

### `oleaut32.dll` (1)

| Funzione | Cella IAT | call | jmp | altri usi |
|---|---|---|---|---|
| `SysAllocString (#2)` | `0x01a47d18` | 1 | 1 | 0 |

### `eax.dll` (1)

| Funzione | Cella IAT | call | jmp | altri usi |
|---|---|---|---|---|
| `EAXDirectSoundCreate8 (#6)` | `0x01a479cc` | 1 | 1 | 0 |

### `d3d9.dll` (1)

| Funzione | Cella IAT | call | jmp | altri usi |
|---|---|---|---|---|
| `Direct3DCreate9` | `0x01a48004` | 0 | 1 | 0 |

### `dinput8.dll` (1)

| Funzione | Cella IAT | call | jmp | altri usi |
|---|---|---|---|---|
| `DirectInput8Create` | `0x01a47968` | 0 | 1 | 0 |

### `winmm.dll` (1)

| Funzione | Cella IAT | call | jmp | altri usi |
|---|---|---|---|---|
| `waveOutGetDevCapsA` | `0x01a47e74` | 1 | 1 | 0 |

### `iphlpapi.dll` (1)

| Funzione | Cella IAT | call | jmp | altri usi |
|---|---|---|---|---|
| `GetAdaptersInfo` | `0x01a48068` | 0 | 1 | 0 |

