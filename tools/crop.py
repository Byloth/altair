#!/usr/bin/env python3
"""Crops and enlarges a region of a PNG (8-bit RGBA), to look at a detail up close.

Usage: python3 -I tools/crop.py IN.png OUT.png X Y WIDTH HEIGHT [SCALE]
Negative X: counted from the right edge. Standard library only.
"""
import struct
import sys
import zlib


def read(path):
    d = open(path, 'rb').read()
    o, idat = 8, b''
    while o < len(d):
        n, t = struct.unpack('>I4s', d[o:o + 8])
        if t == b'IHDR':
            w, h = struct.unpack('>II', d[o + 8:o + 16])
        if t == b'IDAT':
            idat += d[o + 8:o + 8 + n]
        o += 12 + n
    raw, st, rows, prev = zlib.decompress(idat), w * 4 + 1, [], bytearray(w * 4)
    for y in range(h):
        f, r = raw[y * st], bytearray(raw[y * st + 1:(y + 1) * st])
        for i in range(len(r)):
            a, b, c = (r[i - 4] if i >= 4 else 0), prev[i], (prev[i - 4] if i >= 4 else 0)
            if f == 1:
                r[i] = (r[i] + a) & 255
            elif f == 2:
                r[i] = (r[i] + b) & 255
            elif f == 3:
                r[i] = (r[i] + (a + b) // 2) & 255
            elif f == 4:
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                r[i] = (r[i] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
        rows.append(r)
        prev = r
    return w, h, rows


def write(path, w, h, rows):
    raw = b''.join(b'\0' + bytes(r) for r in rows)

    def chunk(t, b):
        return struct.pack('>I', len(b)) + t + b + struct.pack('>I', zlib.crc32(t + b))
    open(path, 'wb').write(b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', w, h, 8, 6, 0, 0, 0))
                           + chunk(b'IDAT', zlib.compress(raw)) + chunk(b'IEND', b''))


def main(argv):
    if len(argv) < 7:
        sys.exit(__doc__)
    w, h, rows = read(argv[1])
    x, y, cw, ch = (int(v) for v in argv[3:7])
    k = int(argv[7]) if len(argv) > 7 else 3
    if x < 0:
        x += w
    out = []
    for yy in range(y, min(y + ch, h)):
        r = bytearray()
        for xx in range(x, min(x + cw, w)):
            r += rows[yy][xx * 4:xx * 4 + 4] * k
        out += [r] * k
    write(argv[2], len(out[0]) // 4, len(out), out)


if __name__ == '__main__':
    main(sys.argv)
