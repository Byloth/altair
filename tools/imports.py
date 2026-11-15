#!/usr/bin/env python3
"""Map of the imports of a PE32: name, IAT cell, references in the code, category.

Usage:
    python3 -I tools/imports.py <exe>          Markdown tables per DLL
    python3 -I tools/imports.py <exe> --guids  known COM GUIDs present in the image

The references are the occurrences of the IAT cell's address inside .text:
`call [IAT]` (FF 15), `jmp [IAT]` (FF 25) and other uses (e.g. `mov esi, [IAT]`).
"""
import os
import re
import struct
import sys
import uuid

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pe import PE  # noqa: E402

ORDINALS = {
    'ws2_32.dll': {1: 'accept', 2: 'bind', 3: 'closesocket', 4: 'connect', 5: 'getpeername',
                   6: 'getsockname', 7: 'getsockopt', 8: 'htonl', 9: 'htons', 10: 'ioctlsocket',
                   11: 'inet_addr', 12: 'inet_ntoa', 13: 'listen', 14: 'ntohl', 15: 'ntohs',
                   16: 'recv', 17: 'recvfrom', 18: 'select', 19: 'send', 20: 'sendto',
                   21: 'setsockopt', 22: 'shutdown', 23: 'socket', 51: 'gethostbyaddr',
                   52: 'gethostbyname', 57: 'gethostname', 101: 'WSAAsyncSelect',
                   111: 'WSAGetLastError', 115: 'WSAStartup', 116: 'WSACleanup',
                   151: '__WSAFDIsSet'},
    'dsound.dll': {1: 'DirectSoundCreate', 2: 'DirectSoundEnumerateA', 11: 'DirectSoundCreate8'},
    'eax.dll': {6: 'EAXDirectSoundCreate8'},
    'oleaut32.dll': {2: 'SysAllocString'},
    'xinput1_3.dll': {2: 'XInputGetState', 3: 'XInputSetState', 4: 'XInputGetCapabilities'},
}

# (category, regex on the KERNEL32 names); the first that matches wins
KERNEL32 = [
    ('ini', r'PrivateProfile'),
    ('exceptions', r'RaiseException|RtlUnwind|UnhandledExceptionFilter'),
    ('time', r'QueryPerformance|GetTickCount|SystemTime|GetLocalTime|FileTimeTo|ToFileTime|TimeZone|GetProcessTimes'),
    ('file', r'File|Directory|FindClose|GetDriveType|GetLogicalDrive|GetTempPath|GetOverlappedResult|SetHandleCount'),
    ('synchronization', r'CriticalSection|Event|Semaphore|WaitFor|Interlocked'),
    ('thread', r'Thread|^Tls|^Sleep$|DuplicateHandle|CloseHandle'),
    ('memory', r'^Heap|^Virtual|LocalFree|GetProcessHeap'),
    ('locale and strings', r'CompareString|LCMapString|GetStringType|MultiByte|WideChar|CP$|CPInfo|CodePage|Locale|LCID|GetDateFormat|GetTimeFormat'),
    ('console and debug', r'Console|StdHandle|Debug|FatalAppExit|FormatMessage'),
]

# DLL -> (module of our runtime, phase, note)
MODULES = {
    'kernel32.dll': ('win32/kernel32', 2, ''),
    'user32.dll': ('win32/user32', 2, 'one window, messages, cursor, keyboard hooks'),
    'advapi32.dll': ('win32/system', 2, 'registry (read-only), random numbers, user name'),
    'shell32.dll': ('win32/system', 2, 'special folders: where the saves go'),
    'ole32.dll': ('win32/com', 2, ''),
    'oleaut32.dll': ('win32/com', 2, ''),
    'winmm.dll': ('win32/system', 2, ''),
    'iphlpapi.dll': ('stub', 2, 'answers "no adapter"'),
    'wininet.dll': ('stub', 2, 'cookies: fails'),
    'ws2_32.dll': ('stub', 2, 'network: fails as unavailable'),
    'd3d9.dll': ('d3d9', 3, 'everything else goes through COM vtables'),
    'd3dx9_36.dll': ('d3d9', 3, ''),
    'dinput8.dll': ('input', 3, 'keyboard and mouse via COM'),
    'xinput1_3.dll': ('input', 3, ''),
    'dsound.dll': ('audio', 3, 'buffers and listener via COM'),
    'eax.dll': ('audio', 3, 'can be declared absent'),
    'binkw32.dll': ('video', 3, 'stdcall'),
}

GUIDS = {
    '81BDCBCA-64D4-426D-AE8D-AD0147F4275C': 'IID_IDirect3D9',
    'D0223B96-BF7A-43FD-92BD-A43B0D82B9EB': 'IID_IDirect3DDevice9',
    'BF798030-483A-4DA2-AA99-5D64ED369700': 'IID_IDirectInput8A',
    'BF798031-483A-4DA2-AA99-5D64ED369700': 'IID_IDirectInput8W',
    '54D41080-DC15-4833-A41B-748F73A38179': 'IID_IDirectInputDevice8A',
    '6F1D2B61-D5A0-11CF-BFC7-444553540000': 'GUID_SysKeyboard',
    '6F1D2B60-D5A0-11CF-BFC7-444553540000': 'GUID_SysMouse',
    'A36D02E0-C9F3-11CF-BFC7-444553540000': 'GUID_XAxis',
    'C50A7E93-F395-4834-9EF6-7FA99DE50966': 'IID_IDirectSound8',
    '6825A449-7524-4D82-920F-50E36AB3AB1E': 'IID_IDirectSoundBuffer8',
    '279AFA86-4981-11CE-A521-0020AF0BE560': 'IID_IDirectSound3DBuffer',
    '279AFA84-4981-11CE-A521-0020AF0BE560': 'IID_IDirectSound3DListener',
    '31EFAC30-515C-11D0-A9AA-00AA0061BE93': 'IID_IKsPropertySet',
    '0306A6A8-B224-11D2-99E5-0000E8D8C722': 'DSPROPSETID_EAX20_ListenerProperties',
    '0306A6A7-B224-11D2-99E5-0000E8D8C722': 'DSPROPSETID_EAX20_BufferProperties',
    'A8FA6882-B476-11D3-BDB9-00C0F02DDF87': 'DSPROPSETID_EAX30_ListenerProperties',
    'A8FA6881-B476-11D3-BDB9-00C0F02DDF87': 'DSPROPSETID_EAX30_BufferProperties',
    '1D4870AD-0DEF-43C0-A40C-523632296342': 'EAXPROPERTYID_EAX40_Context',
    '57E13437-B932-4AB2-B8BD-5266C1A887EE': 'EAXPROPERTYID_EAX50_Context',
    '00000000-0000-0000-C000-000000000046': 'IID_IUnknown',
}


def iat_entries(pe):
    """[(dll, name, address of the IAT cell)]"""
    out = []
    off = pe.rva_to_off(pe.data_dirs[1][0])
    while True:
        oft, _, _, name, ft = struct.unpack_from('<5I', pe.data, off)
        if name == 0:
            break
        dll = pe.cstr(pe.rva_to_off(name)).lower()
        t = pe.rva_to_off(oft or ft)
        i = 0
        while (e := struct.unpack_from('<I', pe.data, t + 4 * i)[0]):
            if e & 0x80000000:
                o = e & 0xffff
                fn = ORDINALS.get(dll, {}).get(o, f'#{o}') + f' (#{o})'
            else:
                fn = pe.cstr(pe.rva_to_off(e) + 2)
            out.append((dll, fn, pe.image_base + ft + 4 * i))
            i += 1
        off += 20
    return out


def category(dll, fn):
    if dll != 'kernel32.dll':
        return ''
    for cat, rx in KERNEL32:
        if re.search(rx, fn):
            return cat
    return 'process and environment'


def main(argv):
    if len(argv) < 2:
        sys.exit(__doc__)
    pe = PE(argv[1])
    if '--guids' in argv:
        for g, name in GUIDS.items():
            raw = uuid.UUID(g).bytes_le
            n = pe.data.count(raw)
            if n:
                o = pe.data.find(raw)
                sec = next(s for s in pe.sections if s[3] <= o < s[3] + s[4])
                print(f'{name:<40} {n} time(s), first at {pe.image_base + sec[1] + o - sec[3]:#010x} ({sec[0]})')
        return

    _, tva, tvs, traw, trs = next(s for s in pe.sections if s[0] == '.text')
    text = pe.data[traw:traw + trs]
    entries = iat_entries(pe)
    rows = {}
    for dll, fn, slot in entries:
        key = struct.pack('<I', slot)
        total = text.count(key)
        calls = text.count(b'\xff\x15' + key)
        jmps = text.count(b'\xff\x25' + key)
        rows.setdefault(dll, []).append((fn, slot, calls, jmps, total - calls - jmps))

    print('| DLL | Functions | References in the code | Our module | Phase | Note |')
    print('|---|---|---|---|---|---|')
    for dll, fns in sorted(rows.items(), key=lambda kv: -len(kv[1])):
        mod, phase, note = MODULES.get(dll, ('?', '?', ''))
        refs = sum(c + j + o for _, _, c, j, o in fns)
        print(f'| `{dll}` | {len(fns)} | {refs} | `{mod}` | {phase} | {note} |')
    print(f'\nTotal: {len(entries)} functions from {len(rows)} DLLs.\n')

    for dll, fns in sorted(rows.items(), key=lambda kv: -len(kv[1])):
        print(f'### `{dll}` ({len(fns)})\n')
        k32 = dll == 'kernel32.dll'
        print('| Function | IAT cell | call | jmp | other uses |' + (' Category |' if k32 else ''))
        print('|---|---|---|---|---|' + ('---|' if k32 else ''))
        order = sorted(fns, key=lambda r: (category(dll, r[0]), -(r[2] + r[3] + r[4]), r[0]))
        for fn, slot, c, j, o in order:
            print(f'| `{fn}` | `{slot:#010x}` | {c} | {j} | {o} |' + (f' {category(dll, fn)} |' if k32 else ''))
        print()


if __name__ == '__main__':
    main(sys.argv)
