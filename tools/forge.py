#!/usr/bin/env python3
"""Reading the .forge archives of Assassin's Creed (2007), container version 25.

Usage:
    python3 -I tools/forge.py summary <game folder>             one line per archive
    python3 -I tools/forge.py list <archive.forge>              entries: id, size, name
    python3 -I tools/forge.py digest <archive.forge>            per entry: id, sections, bytes, CRC-32
    python3 -I tools/forge.py sections <archive.forge> [N]      compressed sections of the first N entries
    python3 -I tools/forge.py head <archive.forge> <name|id>    decompresses the first section (hexdump)
    python3 -I tools/forge.py classes <archive.forge> [rtti.json] [--objects]
                                                                 classes of the objects contained

File names and extracted data are game content: complete outputs go in
private/, never in the repository.

Format (verified on the archives; the unnamed fields are not understood yet):
    Header        char[9] "scimitar\\0", u32 version (25), u64 offset of HeaderFat, u32, u8
    HeaderFat     u32 number of files, u32, u64, u64 (-1), u32 max files per table,
                  u32 number of tables, u64 offset of the first table
    Table         u32 entries, u32, u64 offset of the entries, u64 offset of the next table
                  (-1 = end), u32, u32, u64 offset of the name table, u64
    Entry (16)    u64 offset of the record, u32 id, u32 size of the data
    Record        "FILEDATA", char[128] name, ... for 0x1B8 bytes in all; then the data
    Data          raw, or one or more compressed sections
    Section       u64 marker 33 AA FB 57 99 FA 04 10, u16 version, u8 codec
                  (1 = LZO1X_999), u16, u16, u16 number of chunks,
                  chunks × (u16 decompressed, u16 on disk), then per chunk u32 checksum + data
    Compressed entry   two sections: index and data
    Index         u16 number of objects, then per object u32 id, u32 size
    Object        u32 CRC-32 of the class name, u32 size of the body, u32 length
                  of the name, name, u32 id, u32 CRC-32 of the class (again), fields...
"""
import os
import struct
import sys
from collections import Counter

MARK = bytes.fromhex('33aafb5799fa0410')
REC = 0x1B8


def lzo1x_decompress(src, expect):
    """LZO1X decompressor (public format, like minilzo)."""
    out = bytearray()
    ip = 0

    def copy(dist, n):
        p = len(out) - dist
        if p < 0:
            raise ValueError('LZO: distance outside the buffer')
        for _ in range(n):
            out.append(out[p])
            p += 1

    t = src[ip]
    state = 'top'
    if t > 17:
        ip += 1
        t -= 17
        state = 'match_next' if t < 4 else 'lit_copy'
    while True:
        if state == 'top':
            t = src[ip]; ip += 1
            if t >= 16:
                state = 'match'
                continue
            if t == 0:
                while src[ip] == 0:
                    t += 255; ip += 1
                t += 15 + src[ip]; ip += 1
            t += 3
            state = 'lit_copy'
        if state == 'lit_copy':
            out += src[ip:ip + t]; ip += t
            t = src[ip]; ip += 1
            if t >= 16:
                state = 'match'
            else:
                copy(1 + 0x800 + (t >> 2) + (src[ip] << 2), 3); ip += 1
                state = 'match_done'
        if state == 'match':
            if t >= 64:
                copy(1 + ((t >> 2) & 7) + (src[ip] << 3), (t >> 5) + 1); ip += 1
            elif t >= 32:
                t &= 31
                if t == 0:
                    while src[ip] == 0:
                        t += 255; ip += 1
                    t += 31 + src[ip]; ip += 1
                d = 1 + (src[ip] >> 2) + (src[ip + 1] << 6); ip += 2
                copy(d, t + 2)
            elif t >= 16:
                d = (t & 8) << 11
                t &= 7
                if t == 0:
                    while src[ip] == 0:
                        t += 255; ip += 1
                    t += 7 + src[ip]; ip += 1
                d += (src[ip] >> 2) + (src[ip + 1] << 6); ip += 2
                if d == 0:
                    break                                  # end of stream
                copy(d + 0x4000, t + 2)
            else:
                copy(1 + (t >> 2) + (src[ip] << 2), 2); ip += 1
            state = 'match_done'
        if state == 'match_done':
            t = src[ip - 2] & 3
            state = 'match_next' if t else 'top'
        if state == 'match_next':
            out += src[ip:ip + t]; ip += t
            t = src[ip]; ip += 1
            state = 'match'
    if len(out) != expect:
        raise ValueError(f'LZO: {len(out)} bytes instead of {expect}')
    return bytes(out)


class Forge:
    def __init__(self, path):
        self.path = path
        self.f = open(path, 'rb')
        self.size = os.path.getsize(path)
        h = self.read(0, 0x1A)
        if h[:9] != b'scimitar\0':
            raise ValueError(f'{path}: not a forge archive')
        self.version, fat = struct.unpack_from('<IQ', h, 9)
        hf = struct.unpack('<IIQQIIQ', self.read(fat, 0x28))
        self.file_count, self.tables = hf[0], hf[5]
        self.entries = []                      # (record offset, id, size)
        desc = hf[6]
        while desc not in (0, 0xFFFFFFFFFFFFFFFF):
            n, _, eoff, nxt = struct.unpack('<IIQQ', self.read(desc, 0x18))
            raw = self.read(eoff, 16 * n)
            self.entries += [struct.unpack_from('<QII', raw, 16 * i) for i in range(n)]
            desc = nxt

    def read(self, off, n):
        self.f.seek(off)
        return self.f.read(n)

    def name(self, rec):
        r = self.read(rec, 8 + 128)
        if r[:8] != b'FILEDATA':
            return None
        return r[8:].split(b'\0')[0].decode('latin-1')

    def sections(self, rec, size):
        """Compressed sections of an entry's data: [(codec, [(decompressed, on disk, offset)])]."""
        pos, end = rec + REC, rec + REC + size
        out = []
        while pos + 15 <= end and self.read(pos, 8) == MARK:
            ver, codec, _, _, n = struct.unpack('<HBHHH', self.read(pos + 8, 9))
            sizes = struct.unpack(f'<{2 * n}H', self.read(pos + 17, 4 * n))
            p = pos + 17 + 4 * n
            chunks = []
            for i in range(n):
                dec, sto = sizes[2 * i], sizes[2 * i + 1]
                chunks.append((dec, sto, p + 4))
                p += 4 + sto
            out.append((codec, chunks))
            pos = p
        return out

    def decompress(self, section):
        codec, chunks = section
        out = b''
        for dec, sto, off in chunks:
            raw = self.read(off, sto)
            out += raw if sto == dec else lzo1x_decompress(raw, dec)
        return out


def cmd_summary(folder):
    tot_n = tot_b = 0
    print('| Archive | Size | Entries | Compressed data | Example names |')
    print('|---|---|---|---|---|')
    for fn in sorted(os.listdir(folder)):
        if not fn.endswith('.forge'):
            continue
        fg = Forge(os.path.join(folder, fn))
        names = [fg.name(r) for r, _, _ in fg.entries[:400]]
        comp = sum(1 for r, _, s in fg.entries[:200] if fg.read(r + REC, 8) == MARK)
        sample = min(len(fg.entries), 200)
        ex = ', '.join(f'`{n}`' for n in names[1:4] if n)
        print(f'| `{fn}` | {fg.size / 2**20:.0f} MB | {len(fg.entries)} | {comp}/{sample} | {ex} |')
        tot_n += len(fg.entries)
        tot_b += fg.size
    print(f'\nTotal: {tot_n} entries, {tot_b / 2**30:.2f} GB, container version 25.')


def cmd_list(path):
    fg = Forge(path)
    for rec, fid, size in fg.entries:
        print(f'{fid:#010x} {size:>10} {fg.name(rec)}')


def cmd_digest(path):
    """Per entry: id, sections, decompressed bytes, CRC-32. What `altair-forge` is compared against."""
    import zlib
    fg = Forge(path)
    for rec, fid, size in fg.entries:
        secs = fg.sections(rec, size)
        parts = [fg.decompress(s) for s in secs] if secs else [fg.read(rec + REC, size)]
        crc = 0
        for p in parts:
            crc = zlib.crc32(p, crc)
        print(f'{fid:#010x} {len(secs)} {sum(len(p) for p in parts)} {crc:08x}')


def cmd_sections(path, limit):
    fg = Forge(path)
    codecs = Counter()
    for rec, fid, size in fg.entries[:limit]:
        secs = fg.sections(rec, size)
        desc = '; '.join(f'codec {c}: {len(ch)} chunks, {sum(x[0] for x in ch)} bytes'
                         for c, ch in secs) or 'not compressed'
        for c, _ in secs:
            codecs[c] += 1
        print(f'{fid:#010x} {size:>10} {fg.name(rec)}: {desc}')
    print('codecs seen:', dict(codecs))


def cmd_head(path, key):
    fg = Forge(path)
    for rec, fid, size in fg.entries:
        if fg.name(rec) == key or f'{fid:#x}' == key.lower():
            secs = fg.sections(rec, size)
            data = fg.decompress(secs[0]) if secs else fg.read(rec + REC, min(size, 512))
            print(f'{fg.name(rec)}: id {fid:#x}, {size} bytes on disk, {len(secs)} sections; '
                  f'first section {len(data)} bytes')
            for i in range(0, min(len(data), 256), 16):
                row = data[i:i + 16]
                print(f'{i:04x}  {row.hex(" ")}  {"".join(chr(b) if 32 <= b < 127 else "." for b in row)}')
            return
    sys.exit('entry not found')


def class_names(rtti_json):
    """CRC-32 of the short name -> full name, from the executable's RTTI names."""
    import json
    import zlib
    out = {}
    for t in json.load(open(rtti_json))['types']:
        p = t['pretty']
        if not p.startswith('.?A'):
            out[zlib.crc32(p.split('::')[-1].encode())] = p
    return out


def objects(fg):
    """(entry, id, class CRC, size, name) for each top-level object."""
    for rec, fid, size in fg.entries:
        secs = fg.sections(rec, size)
        if len(secs) != 2:
            continue
        idx = fg.decompress(secs[0])
        n = struct.unpack_from('<H', idx)[0]
        if len(idx) != 2 + 8 * n:
            continue
        data = fg.decompress(secs[1])
        pos = 0
        for i in range(n):
            oid, sz = struct.unpack_from('<II', idx, 2 + 8 * i)
            crc, _, nlen = struct.unpack_from('<III', data, pos)
            name = data[pos + 12:pos + 12 + nlen].split(b'\0')[0].decode('latin-1') if nlen < 256 else ''
            yield fg.name(rec), oid, crc, sz, name
            pos += sz


def cmd_classes(path, rtti_json, show_objects):
    names = class_names(rtti_json) if rtti_json else {}
    fg = Forge(path)
    count, size = Counter(), Counter()
    for entry, oid, crc, sz, name in objects(fg):
        cls = names.get(crc, f'? {crc:#010x}')
        count[cls] += 1
        size[cls] += sz
        if show_objects:
            print(f'{entry}\t{oid:#010x}\t{cls}\t{sz}\t{name}')
    if not show_objects:
        known = sum(n for c, n in count.items() if not c.startswith('?'))
        print(f'{sum(count.values())} objects, {len(count)} classes, '
              f'{known} with a class recognised from the RTTI')
        for cls, n in count.most_common():
            print(f'{n:7} {size[cls]:>11}  {cls}')


def main(argv):
    if len(argv) < 3:
        sys.exit(__doc__)
    cmd = argv[1]
    if cmd == 'summary':
        cmd_summary(argv[2])
    elif cmd == 'list':
        cmd_list(argv[2])
    elif cmd == 'digest':
        cmd_digest(argv[2])
    elif cmd == 'sections':
        cmd_sections(argv[2], int(argv[3]) if len(argv) > 3 else 20)
    elif cmd == 'classes':
        rest = [a for a in argv[3:] if a != '--objects']
        cmd_classes(argv[2], rest[0] if rest else None, '--objects' in argv)
    elif cmd == 'head' and len(argv) > 3:
        cmd_head(argv[2], argv[3])
    else:
        sys.exit(__doc__)


if __name__ == '__main__':
    main(sys.argv)
