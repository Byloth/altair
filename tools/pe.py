#!/usr/bin/env python3
"""Analisi statica minima di eseguibili PE32 (solo libreria standard).

Uso:
    python3 -I tools/pe.py info    <exe>...        header, sezioni, DLL importate
    python3 -I tools/pe.py imports <exe>           funzioni importate, per DLL
    python3 -I tools/pe.py diff    <exe_a> <exe_b> differenze di import, build string, RTTI
    python3 -I tools/pe.py rtti    <exe>           nomi RTTI (.?AV / .?AU), uno per riga

Legge solo i file indicati: non scrive nulla accanto al gioco.
"""
import hashlib
import re
import struct
import sys
from datetime import datetime, timezone


class PE:
    def __init__(self, path):
        self.path = path
        with open(path, 'rb') as f:
            self.data = f.read()
        d = self.data
        pe = struct.unpack_from('<I', d, 0x3c)[0]
        if d[pe:pe + 4] != b'PE\0\0':
            raise ValueError(f'{path}: non è un PE')
        self.machine, nsec, self.timestamp = struct.unpack_from('<HHI', d, pe + 4)
        opt_size, self.characteristics = struct.unpack_from('<HH', d, pe + 20)
        opt = pe + 24
        if struct.unpack_from('<H', d, opt)[0] != 0x10b:
            raise ValueError(f'{path}: solo PE32 supportato')
        self.linker = (d[opt + 2], d[opt + 3])
        self.entry = struct.unpack_from('<I', d, opt + 16)[0]
        self.image_base = struct.unpack_from('<I', d, opt + 28)[0]
        dirs = opt + 96
        self.data_dirs = [struct.unpack_from('<II', d, dirs + 8 * i) for i in range(16)]
        self.sections = []
        for i in range(nsec):
            name, vsize, va, rsize, raw = struct.unpack_from('<8sIIII', d, opt + opt_size + 40 * i)
            self.sections.append((name.rstrip(b'\0').decode(errors='replace'), va, vsize, raw, rsize))

    def rva_to_off(self, rva):
        for _, va, vsize, raw, rsize in self.sections:
            if va <= rva < va + max(vsize, rsize):
                return rva - va + raw
        raise ValueError(f'RVA {rva:#x} fuori dalle sezioni')

    def cstr(self, off):
        return self.data[off:self.data.index(b'\0', off)].decode(errors='replace')

    def sha1(self):
        return hashlib.sha1(self.data).hexdigest()

    def imports(self):
        """{dll (minuscolo): set di nomi funzione o '#ordinale'}"""
        out = {}
        off = self.rva_to_off(self.data_dirs[1][0])
        while True:
            oft, _, _, name, ft = struct.unpack_from('<5I', self.data, off)
            if name == 0:
                break
            funcs = set()
            t = self.rva_to_off(oft or ft)
            while (e := struct.unpack_from('<I', self.data, t)[0]):
                funcs.add(f'#{e & 0xffff}' if e & 0x80000000 else self.cstr(self.rva_to_off(e) + 2))
                t += 4
            out[self.cstr(self.rva_to_off(name)).lower()] = funcs
            off += 20
        return out

    def rtti_names(self):
        return sorted(set(m.decode() for m in re.findall(rb'\.\?A[VU][\x21-\x7e]+', self.data)))

    def build_strings(self):
        return sorted(set(m.decode() for m in re.findall(rb'ChangeList:[\x20-\x7e]+', self.data)))


def cmd_info(paths):
    for p in paths:
        pe = PE(p)
        ts = datetime.fromtimestamp(pe.timestamp, timezone.utc).strftime('%Y-%m-%d %H:%M:%S UTC')
        reloc = pe.data_dirs[5]
        print(f'== {p}')
        print(f'sha1        {pe.sha1()}')
        print(f'dimensione  {len(pe.data)} byte')
        print(f'timestamp   {ts}')
        print(f'linker      {pe.linker[0]}.{pe.linker[1]}')
        print(f'image base  {pe.image_base:#x}   entry {pe.image_base + pe.entry:#x}')
        print(f'flag        {pe.characteristics:#06x}   rilocazioni {"no" if reloc == (0, 0) else reloc}')
        print('sezioni:')
        for name, va, vsize, raw, rsize in pe.sections:
            print(f'  {name:<8} va={pe.image_base + va:#010x} vsize={vsize:#09x} rsize={rsize:#09x}')
        imps = pe.imports()
        print(f'import      {len(imps)} DLL, {sum(map(len, imps.values()))} funzioni')
        print('  ' + ', '.join(imps))
        print(f'RTTI        {len(pe.rtti_names())} nomi di tipo')
        for b in pe.build_strings():
            print(f'build       {b}')
        print()


def cmd_imports(path):
    for dll, funcs in sorted(PE(path).imports().items()):
        print(f'{dll} ({len(funcs)})')
        for f in sorted(funcs):
            print(f'  {f}')


def cmd_diff(a_path, b_path):
    a, b = PE(a_path), PE(b_path)
    ia, ib = a.imports(), b.imports()
    print(f'A = {a_path}\nB = {b_path}\n')
    print(f'funzioni importate: A={sum(map(len, ia.values()))} B={sum(map(len, ib.values()))}')
    for dll in sorted(set(ia) | set(ib)):
        x, y = ia.get(dll, set()), ib.get(dll, set())
        if x != y:
            print(f'  {dll}: A={len(x)} B={len(y)}')
            for f in sorted(x - y):
                print(f'    solo A  {f}')
            for f in sorted(y - x):
                print(f'    solo B  {f}')
    print('\nbuild string:')
    for b_ in a.build_strings():
        print(f'  A  {b_}')
    for b_ in b.build_strings():
        print(f'  B  {b_}')
    ra, rb = set(a.rtti_names()), set(b.rtti_names())
    print(f'\nRTTI: A={len(ra)} B={len(rb)} comuni={len(ra & rb)} solo A={len(ra - rb)} solo B={len(rb - ra)}')


def main(argv):
    if len(argv) < 3:
        sys.exit(__doc__)
    cmd, args = argv[1], argv[2:]
    if cmd == 'info':
        cmd_info(args)
    elif cmd == 'imports' and len(args) == 1:
        cmd_imports(args[0])
    elif cmd == 'diff' and len(args) == 2:
        cmd_diff(*args)
    elif cmd == 'rtti' and len(args) == 1:
        print('\n'.join(PE(args[0]).rtti_names()))
    else:
        sys.exit(__doc__)


if __name__ == '__main__':
    main(sys.argv)
