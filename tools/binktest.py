#!/usr/bin/env python3
"""Compares the project's Bink decoder (`altair-bink`) with ffmpeg, which acts as the oracle.

Video: every frame in yuv420p must be identical byte for byte (decoding is
integer arithmetic). Audio: the samples are floating point, so the error is measured
(maximum and root mean square) and compared with a threshold.

Usage:
    python3 -I tools/binktest.py <file.bik or folder> [--frames N] [--video-only] [--audio-only]

The videos are game content: the comparison happens in memory, nothing is written.
Requires `ffmpeg` in the PATH and `cargo build --release -p altair-bink`.
"""
import array
import os
import struct
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TOOL = os.path.join(ROOT, 'target', 'release', 'altair-bink')
AUDIO_MAX_ERROR = 2e-3      # on samples in [-1, 1]: less than a tenth of a percentage point


def header(path):
    h = open(path, 'rb').read(44)
    frames, w, hgt, tracks = struct.unpack_from('<I', h, 8)[0], *struct.unpack_from('<II', h, 20), struct.unpack_from('<I', h, 40)[0]
    return frames, w, hgt, tracks


def video(path, limit):
    frames, w, h, _ = header(path)
    n = min(frames, limit) if limit else frames
    size = w * h + 2 * ((w + 1) // 2) * ((h + 1) // 2)
    ours = subprocess.Popen([TOOL, 'video', path, str(n)], stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    ref = subprocess.Popen(['ffmpeg', '-v', 'error', '-nostdin', '-i', path, '-an', '-frames:v', str(n), '-f', 'rawvideo',
                            '-pix_fmt', 'yuv420p', '-'], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
    different, first = 0, None
    done = 0
    for i in range(n):
        a, b = ours.stdout.read(size), ref.stdout.read(size)
        if len(a) != size or len(b) != size:
            break
        done += 1
        if a != b:
            different += 1
            if first is None:
                planes = [('Y', 0, w * h), ('Cb', w * h, size - (size - w * h) // 2), ('Cr', size - (size - w * h) // 2, size)]
                where = [f'{name}: {sum(1 for x, y in zip(a[s:e], b[s:e]) if x != y)} bytes' for name, s, e in planes]
                first = f'frame {i} ({", ".join(where)})'
    ours.stdout.close(); ref.stdout.close()
    err = ours.stderr.read().decode().strip()
    ours.wait(); ref.wait()
    ok = done == n and different == 0
    note = '' if ok else f'; first different: {first}' if first else f'; decoded {done} of {n}{": " + err if err else ""}'
    print(f'{"ok        " if ok else "DIFFERENT "} video {os.path.basename(path)}: {done} frames compared, {different} different{note}')
    return ok


def audio(path, track):
    ours = subprocess.run([TOOL, 'audio', path, str(track)], capture_output=True)
    ref = subprocess.run(['ffmpeg', '-v', 'error', '-nostdin', '-i', path, '-map', f'0:a:{track}', '-vn', '-f', 'f32le', '-'],
                         capture_output=True)
    a, b = array.array('f'), array.array('f')
    a.frombytes(ours.stdout[:len(ours.stdout) // 4 * 4])
    b.frombytes(ref.stdout[:len(ref.stdout) // 4 * 4])
    n = min(len(a), len(b))
    if n == 0:
        print(f'DIFFERENT  audio {os.path.basename(path)} track {track}: no samples ({ours.stderr.decode().strip()})')
        return False
    worst = max(abs(a[i] - b[i]) for i in range(n))
    rms = (sum((a[i] - b[i]) ** 2 for i in range(0, n, 7)) / (n / 7)) ** 0.5
    peak = max(abs(x) for x in b[:n:7])
    ok = worst <= AUDIO_MAX_ERROR and len(a) == len(b)
    print(f'{"ok        " if ok else "DIFFERENT "} audio {os.path.basename(path)} track {track}: {len(a)} samples ours, {len(b)} from ffmpeg; '
          f'maximum error {worst:.2e}, root mean square {rms:.2e} (signal peak {peak:.2f})')
    return ok


def main(argv):
    args = [a for a in argv[1:] if not a.startswith('--')]
    if not args:
        sys.exit(__doc__)
    limit = int(argv[argv.index('--frames') + 1]) if '--frames' in argv else 0
    if limit:
        args = [a for a in args if a != str(limit)]
    target = args[0]
    files = sorted(os.path.join(target, f) for f in os.listdir(target) if f.lower().endswith('.bik')) if os.path.isdir(target) else [target]
    ok = True
    for f in files:
        if '--audio-only' not in argv:
            ok &= video(f, limit)
        if '--video-only' not in argv:
            for t in range(header(f)[3]):
                ok &= audio(f, t)
    print('binktest:', 'OK' if ok else 'DIFFERENCES')
    sys.exit(0 if ok else 1)


main(sys.argv)
