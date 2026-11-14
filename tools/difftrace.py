#!/usr/bin/env python3
"""Rigioca un caso del test automatico sull'originale e stampa gli indirizzi eseguiti.

Uso: .venv/bin/python tools/difftrace.py <exe> 0xADDR <caso>
Serve a capire una differenza segnalata da `difftest.py --auto`: gli ingressi sono gli stessi.
"""
import os
import random
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import difftest  # noqa: E402
import harness  # noqa: E402

exe, addr, case = sys.argv[1], int(sys.argv[2], 16), int(sys.argv[3])
g = harness.Guest(exe, trace=True)
g.reset()
args, ecx, edx, xmm = difftest.make_inputs(g, random.Random(addr * 1000 + case))
print('args', [hex(a) for a in args], 'ecx', hex(ecx), 'edx', hex(edx))
try:
    r = g.call(addr, args, ecx=ecx, edx=edx, xmm=xmm, max_insns=300_000)
    print('regs', [hex(x) for x in r.regs])
    print('xmm ', [f'{x:032x}' for x in r.xmm])
except harness.GuestError as e:
    print('errore:', e)

# Stesso caso sul codice tradotto (build/difftest-auto.so), con le prime differenze di memoria.
lib = os.path.join(difftest.ROOT, 'build', 'difftest-auto.so')
if os.path.exists(lib):
    n = difftest.Native(exe, lib)
    n.reset()
    args, ecx, edx, xmm = difftest.make_inputs(n, random.Random(addr * 1000 + case))
    try:
        r2 = n.call(addr, args, ecx=ecx, edx=edx, xmm=xmm)
        print('tradotto regs', [hex(x) for x in r2.regs])
    except harness.GuestError as e:
        print('tradotto errore:', e)
    for name, base, size in (('heap', harness.HEAP, harness.HEAP_SIZE),
                             ('stack', harness.STACK_TOP - harness.STACK_CLEAR, harness.STACK_CLEAR)):
        a, b = g.read(base, size), n.read(base, size)
        diffs = [i for i in range(0, size, 4) if a[i:i + 4] != b[i:i + 4]]
        print(f'{name}: {len(diffs)} parole diverse', [(hex(base + i), a[i:i + 4].hex(), b[i:i + 4].hex()) for i in diffs[:8]])
