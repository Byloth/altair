#!/usr/bin/env python3
"""Test differenziale: il codice tradotto da `altair` contro l'originale eseguito in Unicorn.

Uso (dalla radice del progetto, con il Python del venv):
    .venv/bin/python tools/difftest.py <AssassinsCreed_Dx9.exe>                 scenari scritti a mano
    .venv/bin/python tools/difftest.py <AssassinsCreed_Dx9.exe> --auto N [--cases K]
        N funzioni scelte da `altair candidates`, K casi ciascuna (8) con ingressi generati;
    .venv/bin/python tools/difftest.py <AssassinsCreed_Dx9.exe> --funcs 0xA,0xB [--cases K]
        lo stesso sulle funzioni indicate, anche se non sono candidate;
        il dettaglio va in build/difftest-auto.txt

Passi:
  1. `altair translate` traduce in C le funzioni degli scenari (build/gen/difftest.c);
  2. clang le compila con runtime/altair.c in una libreria (build/difftest.so);
  3. ogni scenario gira due volte con gli stessi ingressi: sull'originale in Unicorn
     (tools/harness.py) e sul codice tradotto, in una memoria guest identica;
  4. si confrontano, chiamata per chiamata, eax, ecx, edx, i byte tolti dallo stack, gli import
     chiamati, e alla fine il contenuto di heap di prova e stack.

Uno scenario è una funzione `scenario(g, rng)` che usa solo l'interfaccia comune a `Guest`
(Unicorn) e `Native` (tradotto): alloc, alloc_str, write, read, call.
"""
import ctypes
import hashlib
import os
import random
import struct
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import harness  # noqa: E402
from harness import (HEAP, HEAP_CLEAR, HEAP_SIZE, PAGE, SENTINEL, STACK_CLEAR, STACK_SIZE, STACK_TOP, TEB,  # noqa: E402
                     TRAMP)
from imports import iat_entries  # noqa: E402
from pe import PE  # noqa: E402

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


class Ctx(ctypes.Structure):
    pass


HOST_CALL = ctypes.CFUNCTYPE(ctypes.c_int, ctypes.POINTER(Ctx), ctypes.c_uint32)
Ctx._fields_ = [(r, ctypes.c_uint32) for r in ('eax', 'ecx', 'edx', 'ebx', 'esp', 'ebp', 'esi', 'edi',
                                                'df', 'fs_base')] + [
    ('mem', ctypes.c_void_p), ('host_call', HOST_CALL), ('trap_jmp', ctypes.c_void_p),
    ('trap_msg', ctypes.c_char_p), ('trap_addr', ctypes.c_uint32), ('mxcsr', ctypes.c_uint32),
    ('xmm', ctypes.c_uint8 * 128), ('fpr', ctypes.c_longdouble * 8), ('fptop', ctypes.c_uint32),
    ('fpcw', ctypes.c_uint32), ('fpsw', ctypes.c_uint32)]


class Native:
    """Stessa interfaccia di harness.Guest, ma esegue il codice tradotto."""

    def __init__(self, exe, lib):
        self.pe = pe = PE(exe)
        self.lib = lib = ctypes.CDLL(lib)
        lib.altair_mem_create.restype = ctypes.c_void_p
        lib.altair_run.argtypes = [ctypes.POINTER(Ctx), ctypes.c_uint32]
        self.mem = lib.altair_mem_create()
        if not self.mem:
            raise harness.GuestError('impossibile riservare la memoria guest')
        self._writable, self._readonly = [], []
        for name, va, vs, raw, rs in pe.sections:
            size = max(vs, rs)
            data = pe.data[raw:raw + min(rs, size)].ljust(size, b'\0')
            self.write(pe.image_base + va, data)
            (self._readonly if name in ('.text', '.rdata', '.rsrc') else self._writable).append(
                (pe.image_base + va, data))
        self.imports, self._iat = {}, {}
        for i, (dll, fn, slot) in enumerate(iat_entries(pe)):
            self.imports[TRAMP + 16 * i] = (dll, fn)
            self._iat[slot] = TRAMP + 16 * i
        self.handlers = {}
        self.heap_top = HEAP
        self._log = []
        self._fault = None
        self._cb = HOST_CALL(self._host_call)
        self.reset()

    def write(self, addr, data):
        data = bytes(data)
        ctypes.memmove(self.mem + addr, data, len(data))

    def read(self, addr, size):
        return ctypes.string_at(self.mem + addr, size)

    def u32(self, addr):
        return struct.unpack('<I', self.read(addr, 4))[0]

    def alloc(self, size, align=16):
        a = (self.heap_top + align - 1) & ~(align - 1)
        if a + size > HEAP + HEAP_SIZE:
            raise harness.GuestError('heap di prova esaurito')
        self.heap_top = a + size
        return a

    def alloc_bytes(self, data):
        a = self.alloc(len(data) or 1)
        self.write(a, data)
        return a

    def alloc_str(self, s):
        return self.alloc_bytes(bytes(s) + b'\0')

    def image_state(self):
        return b''.join(self.read(va, len(data)) for va, data in self._writable)

    def readonly_damaged(self):
        """Il codice tradotto ha scritto in codice o costanti? Se sì le ripristina."""
        damaged = False
        for va, data in self._readonly:
            if self.read(va, len(data)) != data:
                damaged = True
                self.write(va, data)
        return damaged

    def reset(self):
        for va, data in self._writable:
            self.write(va, data)
        self.write(STACK_TOP - STACK_CLEAR, bytes(STACK_CLEAR))
        for slot, t in self._iat.items():
            self.write(slot, struct.pack('<I', t))
        self.write(HEAP, bytes(max(self.heap_top - HEAP, HEAP_CLEAR)))
        self.heap_top = HEAP

    def _host_call(self, cp, addr):
        c = cp.contents
        if addr not in self.imports:
            self._fault = f'chiamata a {addr:#010x}: né funzione tradotta né import'
            return 1
        dll, fn = self.imports[addr]
        ret = self.u32(c.esp)
        name = fn.split(' ')[0]
        self._log.append((name, ret))
        h = self.handlers.get(name)
        if h is None:
            self._fault = f'import non gestito: {dll}!{fn} (chiamato da {ret:#010x})'
            return 1
        esp = c.esp
        eax, pop = h(self, lambda i: self.u32(esp + 4 + 4 * i))
        c.eax = eax & 0xFFFFFFFF
        c.esp = esp + 4 + pop
        return 0

    def call(self, addr, args=(), ecx=0, edx=0, max_insns=None, xmm=None):
        teb = bytearray(PAGE * 2)
        struct.pack_into('<I', teb, 0x00, 0xFFFFFFFF)
        struct.pack_into('<I', teb, 0x04, STACK_TOP)
        struct.pack_into('<I', teb, 0x08, STACK_TOP - STACK_SIZE)
        struct.pack_into('<I', teb, 0x18, TEB)
        struct.pack_into('<I', teb, 0x2C, TEB + PAGE)
        self.write(TEB, teb)
        frame = struct.pack(f'<{len(args) + 1}I', SENTINEL, *[a & 0xFFFFFFFF for a in args])
        esp0 = STACK_TOP - 0x1000 - len(frame)
        self.write(esp0, frame)
        c = Ctx(eax=0, ecx=ecx & 0xFFFFFFFF, edx=edx & 0xFFFFFFFF, ebx=0, esp=esp0, ebp=0, esi=0, edi=0,
                df=0, fs_base=TEB, mem=self.mem, host_call=self._cb, mxcsr=0x1F80, fpcw=0x027F)
        if xmm:
            ctypes.memmove(c.xmm, b''.join(v.to_bytes(16, 'little') for v in xmm), 128)
        self._log, self._fault = [], None
        if self.lib.altair_run(ctypes.byref(c), addr):
            raise harness.GuestError(self._fault or f'trap: {c.trap_msg.decode()} ({c.trap_addr:#010x})')
        regs = (c.eax, c.ecx, c.edx, c.ebx, c.ebp, c.esi, c.edi)
        xmm_out = tuple(int.from_bytes(bytes(c.xmm[16 * i:16 * i + 16]), 'little') for i in range(8))
        raw = ctypes.string_at(ctypes.addressof(c.fpr), 128)
        x87 = (c.fptop, tuple(raw[16 * i:16 * i + 10] for i in range(8)))
        return harness.Result(eax=c.eax, edx=c.edx, ecx=c.ecx, regs=regs, st0=None, xmm=xmm_out, x87=x87,
                              popped=(c.esp - esp0 - 4) & 0xFFFFFFFF, insns=None, imports=list(self._log))


class Recorder:
    """Avvolge un motore e registra ciò che si può osservare di ogni chiamata."""

    def __init__(self, engine):
        self.e = engine
        self.log = []

    def __getattr__(self, name):
        return getattr(self.e, name)

    def call(self, addr, args=(), ecx=0, edx=0):
        try:
            r = self.e.call(addr, args, ecx=ecx, edx=edx)
            self.log.append((addr, tuple(args), r.regs, r.xmm, r.x87, r.popped, tuple(r.imports)))
            return r
        except harness.GuestError as err:
            self.log.append((addr, tuple(args), 'errore', str(err)))
            raise

    def snapshot(self):
        heap = self.e.read(HEAP, self.e.heap_top - HEAP)
        stack = self.e.read(STACK_TOP - STACK_CLEAR, STACK_CLEAR)
        return (hashlib.sha1(heap).hexdigest(), hashlib.sha1(stack).hexdigest(),
                hashlib.sha1(self.e.image_state()).hexdigest())


# --- scenari ---------------------------------------------------------------------------------

STRLEN, STRCMP, MEMCPY = 0x00FA6790, 0x00FA9760, 0x00F9FB80


def sc_strlen(g, rng):
    for _ in range(300):
        n = rng.choice([0, 1, 2, 3, 4, 5, 7, 8, 15, 16, 31, 33, 100, 1000])
        p = g.alloc(n + 8) + rng.randrange(4)
        g.write(p, bytes(rng.randrange(1, 256) for _ in range(n)) + b'\0')
        g.call(STRLEN, [p])


def sc_strcmp(g, rng):
    for _ in range(300):
        n = rng.randrange(0, 40)
        a = bytes(rng.randrange(1, 256) for _ in range(n))
        b = bytearray(a)
        if b and rng.random() < 0.7:
            b[rng.randrange(len(b))] = rng.randrange(1, 256)
        if rng.random() < 0.3:
            b = b[:rng.randrange(len(b) + 1)]
        pa = g.alloc(len(a) + 8) + rng.randrange(4)
        pb = g.alloc(len(b) + 8) + rng.randrange(4)
        g.write(pa, a + b'\0')
        g.write(pb, bytes(b) + b'\0')
        g.call(STRCMP, [pa, pb])


def sc_memcpy(g, rng):
    src = g.alloc_bytes(bytes(rng.randrange(256) for _ in range(700)))
    for n in list(range(0, 48)) + [63, 64, 65, 127, 128, 129, 255, 256, 257, 511]:
        for sa in range(4):
            for da in range(4):
                g.call(MEMCPY, [g.alloc(n + 16) + da, src + sa, n])
    # Regioni sovrapposte, in avanti e all'indietro (percorso con il flag di direzione).
    buf = g.alloc_bytes(bytes(rng.randrange(256) for _ in range(900)))
    for _ in range(200):
        n = rng.randrange(0, 300)
        s = rng.randrange(0, 300)
        d = s + rng.randrange(-40, 41)
        if d >= 0:
            g.call(MEMCPY, [buf + d, buf + s, n])


SCENARIOS = [('strlen', sc_strlen, [STRLEN]), ('strcmp', sc_strcmp, [STRCMP]), ('memcpy', sc_memcpy, [MEMCPY])]


ALTAIR = os.path.join(ROOT, 'target', 'release', 'altair')


def build(exe, roots=None, name='difftest', opt='-O2'):
    gen = os.path.join(ROOT, 'build', 'gen', name + '.c')
    lib = os.path.join(ROOT, 'build', name + '.so')
    os.makedirs(os.path.dirname(gen), exist_ok=True)
    if roots is None:
        roots = sorted({a for _, _, addrs in SCENARIOS for a in addrs})
    roots_file = os.path.join(ROOT, 'build', name + '.roots')
    with open(roots_file, 'w') as f:
        f.write(''.join(f'{a:#x}\n' for a in roots))
    subprocess.run([ALTAIR, 'translate', exe, '--out', gen, '--roots', roots_file], check=True,
                   stderr=subprocess.DEVNULL if name != 'difftest' else None)
    rt = os.path.join(ROOT, 'runtime')
    subprocess.run(['clang', opt, '-g', '-Wall', '-Wno-unused-variable', '-Wno-unused-but-set-variable',
                    '-Wno-unused-label', '-Wno-parentheses-equality', '-fwrapv', '-fno-math-errno', '-ffp-contract=off', '-shared', '-fPIC', '-I', rt, gen,
                    os.path.join(rt, 'altair.c'), '-lm', '-o', lib], check=True)
    return lib


def run(exe):
    lib = build(exe)
    oracle, native = harness.Guest(exe), Native(exe, lib)
    failed = 0
    for name, scenario, _ in SCENARIOS:
        logs = []
        for engine in (oracle, native):
            rec = Recorder(engine)
            try:
                scenario(rec, random.Random(1234))
            except harness.GuestError:
                pass
            logs.append((rec.log, rec.snapshot()))
            engine.reset()
        (la, sa), (lb, sb) = logs
        diff = next((i for i, (x, y) in enumerate(zip(la, lb)) if x != y), None)
        if diff is None and len(la) == len(lb) and sa == sb:
            print(f'ok       {name}: {len(la)} chiamate identiche; heap, stack e dati globali identici')
            continue
        failed += 1
        print(f'DIVERSO  {name}')
        if diff is not None:
            print(f'  chiamata {diff}:\n    originale {la[diff]}\n    tradotto  {lb[diff]}')
        elif len(la) != len(lb):
            print(f'  {len(la)} chiamate nell\'originale, {len(lb)} nel tradotto')
        else:
            print(f'  memoria finale: heap {"=" if sa[0] == sb[0] else "DIVERSO"}, '
                  f'stack {"=" if sa[1] == sb[1] else "DIVERSO"}, dati {"=" if sa[2] == sb[2] else "DIVERSI"}')
    print('difftest:', 'OK' if not failed else f'{failed} scenari diversi')
    return 1 if failed else 0


SPECIAL_FLOATS = [0.0, -0.0, 1.0, -1.0, 0.5, float('inf'), float('-inf'), float('nan'), 1e-40, 3.4e38, 2147483648.0,
                  -2147483904.0, 16777216.0]


def make_inputs(g, rng):
    """Ingressi generici: buffer pieni di puntatori ad altri buffer, interi piccoli, float e
    valori casuali; argomenti e registri scelti tra puntatori e interi."""
    bufs = [g.alloc(512) for _ in range(6)]

    def value(p_ptr):
        r = rng.random()
        if r < p_ptr:
            return rng.choice(bufs) + 4 * rng.randrange(0, 96)
        if r < p_ptr + 0.25:
            return rng.randrange(0, 17)
        if r < p_ptr + 0.50:
            return f32()
        return rng.getrandbits(32)

    def f32():
        r = rng.random()
        x = (rng.choice(SPECIAL_FLOATS) if r < 0.15 else rng.uniform(-4, 4) if r < 0.6
             else rng.uniform(-1e4, 1e4) if r < 0.9 else rng.uniform(-1e30, 1e30))
        return struct.unpack('<I', struct.pack('<f', x))[0]

    for b in bufs:
        g.write(b, struct.pack('<128I', *[value(0.35) for _ in range(128)]))
    # Registri XMM iniziali: quattro float ciascuno, a volte due double.
    xmm = []
    for _ in range(8):
        if rng.random() < 0.25:
            raw = struct.pack('<2d', rng.uniform(-1e3, 1e3), rng.uniform(-1e3, 1e3))
        else:
            raw = struct.pack('<4I', f32(), f32(), f32(), f32())
        xmm.append(int.from_bytes(raw, 'little'))
    return [value(0.6) for _ in range(6)], rng.choice(bufs), value(0.5), xmm


def observe(g, r):
    return (r.regs, r.xmm, r.x87, r.popped, tuple(r.imports), g.read(HEAP, max(g.heap_top - HEAP, HEAP_CLEAR)),
            g.read(STACK_TOP - STACK_CLEAR, STACK_CLEAR), g.image_state())


def same_but_nan(a, b):
    """Due blocchi di byte sono uguali, a meno di parole che sono NaN in entrambi.

    Quando i due operandi di un'operazione SSE sono entrambi NaN, l'hardware x86 restituisce
    il primo; l'emulatore (QEMU) applica la regola dell'x87 e può restituire l'altro. Il codice
    tradotto segue l'hardware, quindi segno e payload di un NaN possono differire."""
    if a == b:
        return True
    if len(a) != len(b):
        return False
    wa, wb = struct.unpack(f'<{len(a) // 4}I', a), struct.unpack(f'<{len(b) // 4}I', b)

    def nan32(x):
        return x & 0x7F800000 == 0x7F800000 and x & 0x007FFFFF

    def nan64(lo, hi):
        return hi & 0x7FF00000 == 0x7FF00000 and (hi & 0x000FFFFF or lo)

    for i, (x, y) in enumerate(zip(wa, wb)):
        if x == y or (nan32(x) and nan32(y)):
            continue
        j = i & ~1                      # la parola doppia che la contiene, allineata a 8
        if j + 1 < len(wa) and nan64(wa[j], wa[j + 1]) and nan64(wb[j], wb[j + 1]):
            continue
        return False
    return True


def x87_same_but_nan(a, b):
    """Stessa cima dello stack e stessi registri, a meno di registri che sono NaN in entrambi."""
    def nan(r):
        return (r[9] & 0x7F) == 0x7F and r[8] == 0xFF

    def mmx(x, y):
        # Dopo un'istruzione MMX l'hardware porta l'esponente del registro a tutti uno;
        # l'emulatore lo lascia com'era. Il tradotto (y) segue l'hardware.
        return x[:8] == y[:8] and y[8:] == b'\xff\xff'
    return a[0] == b[0] and all(x == y or (nan(x) and nan(y)) or mmx(x, y) for x, y in zip(a[1], b[1]))


def xmm_bytes(regs):
    return b''.join(v.to_bytes(16, 'little') for v in regs)


FIELDS = ('registri (eax ecx edx ebx ebp esi edi)', 'registri xmm', 'stato x87', 'byte tolti dallo stack', 'import', 'heap', 'stack',
          'dati globali')


def instruction_report(gen, ok_roots):
    """Quali tipi di istruzione compaiono nelle funzioni risultate identiche (con le funzioni
    che chiamano). Comparire non garantisce che il caso di prova sia passato di lì."""
    import collections
    import re
    src = open(gen).read()
    funcs = {int(m.group(1), 16): m.group(2)
             for m in re.finditer(r'^void f_([0-9a-f]{8})\(Ctx \*c\) \{\n(.*?)^\}', src, re.S | re.M)}
    done, stack = set(), list(ok_roots)
    while stack:
        a = stack.pop()
        if a in done or a not in funcs:
            continue
        done.add(a)
        stack.extend(int(x, 16) for x in re.findall(r'f_([0-9a-f]{8})\(c\);', funcs[a]))
    pat = re.compile(r'/\* [0-9a-f]{8}  ((?:lock |rep |repe |repne )?)(\w+)')
    verified, present = collections.Counter(), set()
    for a, text in funcs.items():
        for prefix, mn in pat.findall(text):
            key = 'lock' if prefix == 'lock ' else mn
            present.add(key)
            present.add(mn)
            if a in done:
                verified[key] += 1
                if prefix == 'lock ':
                    verified[mn] += 1
    return len(done), verified, sorted(present - set(verified))


def run_auto(exe, count, cases, only=None):
    import signal
    if only:
        roots = only
    else:
        out = subprocess.run([ALTAIR, 'candidates', exe, '--max', str(count)], check=True, capture_output=True,
                             text=True)
        print(out.stderr.strip())
        roots = [int(x, 16) for x in out.stdout.split()]
    lib = build(exe, roots, name='difftest-auto', opt='-O1')
    oracle, native = harness.Guest(exe), Native(exe, lib)
    same, differ, never = [], [], []
    nan_only = set()
    report = []
    current = os.path.join(ROOT, 'build', 'difftest-current.txt')
    for addr in roots:
        ran, bad = 0, None
        for case in range(cases):
            obs = []
            for engine in (oracle, native):
                engine.reset()
                args, ecx, edx, xmm = make_inputs(engine, random.Random(addr * 1000 + case))
                if engine is native:
                    with open(current, 'w') as f:
                        f.write(f'{addr:#x} caso {case}\n')
                    signal.alarm(60)       # un ciclo infinito nel tradotto uccide il processo
                try:
                    r = engine.call(addr, args, ecx=ecx, edx=edx, max_insns=300_000, xmm=xmm)
                    obs.append(observe(engine, r))
                except harness.GuestError as e:
                    obs.append(('errore', str(e)))
                finally:
                    signal.alarm(0)
                if obs[0][0] == 'errore':
                    break                   # l'originale non gira con questi ingressi: caso saltato
            if obs[0][0] == 'errore':
                continue
            ran += 1
            if obs[0] != obs[1] and obs[1][0] != 'errore':
                # Tolleranza sui soli NaN, per registri XMM e memoria (indici 1, 4, 5, 6).
                o0 = (obs[0][0], xmm_bytes(obs[0][1])) + obs[0][2:]
                o1 = (obs[1][0], xmm_bytes(obs[1][1])) + obs[1][2:]
                if all(same_but_nan(x, y) if k in (1, 5, 6, 7) else x87_same_but_nan(x, y) if k == 2 else x == y
                       for k, (x, y) in enumerate(zip(o0, o1))):
                    nan_only.add(addr)
                    continue
            if obs[0] != obs[1] and bad is None:
                if obs[1][0] == 'errore':
                    bad = (case, f'il tradotto si ferma: {obs[1][1]}')
                else:
                    k = next(i for i in range(len(FIELDS)) if obs[0][i] != obs[1][i])
                    detail = (f'{obs[0][k]} contro {obs[1][k]}' if k in (0, 3, 4) else
                              f'cima {obs[0][2][0]} contro {obs[1][2][0]}; ' + ' '.join(
                                  f'fp{i}: {a.hex()} contro {b.hex()}' for i, (a, b) in
                                  enumerate(zip(obs[0][2][1], obs[1][2][1])) if a != b) if k == 2 else
                              ' '.join(f'xmm{i}: {a:032x} contro {b:032x}' for i, (a, b) in
                                       enumerate(zip(obs[0][1], obs[1][1])) if a != b) if k == 1 else '')
                    bad = (case, f'{FIELDS[k]} {detail}')
        if native.readonly_damaged() and not bad:
            bad = ('-', 'il tradotto ha scritto in sezioni di sola lettura')
        if bad:
            differ.append(addr)
            report.append(f'DIVERSO {addr:#010x} caso {bad[0]}: {bad[1]}')
        elif ran:
            same.append(addr)
            report.append(f'ok      {addr:#010x} ({ran} casi)')
        else:
            never.append(addr)
            report.append(f'saltata {addr:#010x} (l\'originale non gira con gli ingressi generati)')
    with open(os.path.join(ROOT, 'build', 'difftest-auto.txt'), 'w') as f:
        f.write('\n'.join(report) + '\n')
    print(f'{len(roots)} funzioni: {len(same)} identiche, {len(differ)} diverse, {len(never)} saltate')
    nan_ok = sorted(nan_only - set(differ))
    if nan_ok:
        print(f'  di cui {len(nan_ok)} identiche a meno di un NaN o dell\'esponente di un registro MMX: '
              + ' '.join(f'{a:#x}' for a in nan_ok[:8]))
    ndone, verified, missing = instruction_report(os.path.join(ROOT, 'build', 'gen', 'difftest-auto.c'), same)
    print(f'funzioni eseguite in casi identici, comprese quelle chiamate: {ndone}; '
          f'{sum(verified.values())} istruzioni di {len(verified)} tipi')
    if missing:
        print('tipi tradotti ma assenti dalle funzioni verificate:', ' '.join(missing))
    with open(os.path.join(ROOT, 'build', 'difftest-auto.txt'), 'a') as f:
        f.write('\nistruzioni nelle funzioni verificate:\n')
        f.write(''.join(f'{n:7} {m}\n' for m, n in verified.most_common()))
    for line in [x for x in report if x.startswith('DIVERSO')][:25]:
        print(' ', line[:200])
    return 1 if differ else 0


if __name__ == '__main__':
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    if '--auto' in sys.argv:
        n = int(sys.argv[sys.argv.index('--auto') + 1])
        k = int(sys.argv[sys.argv.index('--cases') + 1]) if '--cases' in sys.argv else 8
        sys.exit(run_auto(sys.argv[1], n, k))
    if '--funcs' in sys.argv:
        addrs = [int(x, 16) for x in sys.argv[sys.argv.index('--funcs') + 1].split(',')]
        k = int(sys.argv[sys.argv.index('--cases') + 1]) if '--cases' in sys.argv else 8
        sys.exit(run_auto(sys.argv[1], 0, k, only=addrs))
    sys.exit(run(sys.argv[1]))
