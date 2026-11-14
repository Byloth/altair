#!/usr/bin/env python3
"""Lettura degli archivi .forge di Assassin's Creed (2007), contenitore versione 25.

Uso:
    python3 -I tools/forge.py summary <cartella del gioco>      una riga per archivio
    python3 -I tools/forge.py list <archivio.forge>             voci: id, dimensione, nome
    python3 -I tools/forge.py sections <archivio.forge> [N]     sezioni compresse delle prime N voci
    python3 -I tools/forge.py head <archivio.forge> <nome|id>   decomprime la prima sezione (hexdump)
    python3 -I tools/forge.py classes <archivio.forge> [rtti.json] [--objects]
                                                                 classi degli oggetti contenuti

I nomi dei file e i dati estratti sono contenuti del gioco: gli output completi vanno in
private/, mai nel repository.

Formato (verificato sugli archivi; i campi senza nome non sono ancora capiti):
    Header        char[9] "scimitar\\0", u32 versione (25), u64 offset di HeaderFat, u32, u8
    HeaderFat     u32 numero di file, u32, u64, u64 (-1), u32 max file per tabella,
                  u32 numero di tabelle, u64 offset della prima tabella
    Tabella       u32 voci, u32, u64 offset delle voci, u64 offset della tabella successiva
                  (-1 = fine), u32, u32, u64 offset della tabella dei nomi, u64
    Voce (16)     u64 offset del record, u32 id, u32 dimensione dei dati
    Record        "FILEDATA", char[128] nome, ... per 0x1B8 byte in tutto; poi i dati
    Dati          grezzi, oppure una o più sezioni compresse
    Sezione       u64 marcatore 33 AA FB 57 99 FA 04 10, u16 versione, u8 codec
                  (1 = LZO1X_999), u16, u16, u16 numero di blocchi,
                  blocchi × (u16 decompressi, u16 su disco), poi per blocco u32 checksum + dati
    Voce compressa   due sezioni: indice e dati
    Indice        u16 numero di oggetti, poi per oggetto u32 id, u32 dimensione
    Oggetto       u32 CRC-32 del nome della classe, u32 dimensione del corpo, u32 lunghezza
                  del nome, nome, u32 id, u32 CRC-32 della classe (di nuovo), campi...
"""
import os
import struct
import sys
from collections import Counter

MARK = bytes.fromhex('33aafb5799fa0410')
REC = 0x1B8


def lzo1x_decompress(src, expect):
    """Decompressore LZO1X (formato pubblico, come minilzo)."""
    out = bytearray()
    ip = 0

    def copy(dist, n):
        p = len(out) - dist
        if p < 0:
            raise ValueError('LZO: distanza fuori dal buffer')
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
                    break                                  # fine del flusso
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
        raise ValueError(f'LZO: {len(out)} byte invece di {expect}')
    return bytes(out)


class Forge:
    def __init__(self, path):
        self.path = path
        self.f = open(path, 'rb')
        self.size = os.path.getsize(path)
        h = self.read(0, 0x1A)
        if h[:9] != b'scimitar\0':
            raise ValueError(f'{path}: non è un archivio forge')
        self.version, fat = struct.unpack_from('<IQ', h, 9)
        hf = struct.unpack('<IIQQIIQ', self.read(fat, 0x28))
        self.file_count, self.tables = hf[0], hf[5]
        self.entries = []                      # (offset record, id, dimensione)
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
        """Sezioni compresse dei dati di una voce: [(codec, [(decompressi, su disco, offset)])]."""
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
    print('| Archivio | Dimensione | Voci | Dati compressi | Esempi di nomi |')
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
    print(f'\nTotale: {tot_n} voci, {tot_b / 2**30:.2f} GB, versione del contenitore 25.')


def cmd_list(path):
    fg = Forge(path)
    for rec, fid, size in fg.entries:
        print(f'{fid:#010x} {size:>10} {fg.name(rec)}')


def cmd_sections(path, limit):
    fg = Forge(path)
    codecs = Counter()
    for rec, fid, size in fg.entries[:limit]:
        secs = fg.sections(rec, size)
        desc = '; '.join(f'codec {c}: {len(ch)} blocchi, {sum(x[0] for x in ch)} byte'
                         for c, ch in secs) or 'non compresso'
        for c, _ in secs:
            codecs[c] += 1
        print(f'{fid:#010x} {size:>10} {fg.name(rec)}: {desc}')
    print('codec visti:', dict(codecs))


def cmd_head(path, key):
    fg = Forge(path)
    for rec, fid, size in fg.entries:
        if fg.name(rec) == key or f'{fid:#x}' == key.lower():
            secs = fg.sections(rec, size)
            data = fg.decompress(secs[0]) if secs else fg.read(rec + REC, min(size, 512))
            print(f'{fg.name(rec)}: id {fid:#x}, {size} byte su disco, {len(secs)} sezioni; '
                  f'prima sezione {len(data)} byte')
            for i in range(0, min(len(data), 256), 16):
                row = data[i:i + 16]
                print(f'{i:04x}  {row.hex(" ")}  {"".join(chr(b) if 32 <= b < 127 else "." for b in row)}')
            return
    sys.exit('voce non trovata')


def class_names(rtti_json):
    """CRC-32 del nome breve -> nome completo, dai nomi RTTI dell'eseguibile."""
    import json
    import zlib
    out = {}
    for t in json.load(open(rtti_json))['types']:
        p = t['pretty']
        if not p.startswith('.?A'):
            out[zlib.crc32(p.split('::')[-1].encode())] = p
    return out


def objects(fg):
    """(voce, id, classe CRC, dimensione, nome) per ogni oggetto di primo livello."""
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
        print(f'{sum(count.values())} oggetti, {len(count)} classi, '
              f'{known} con classe riconosciuta dall\'RTTI')
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
