#!/usr/bin/env python3
"""Test differenziale: il codice tradotto da `acrek` contro l'originale eseguito in Unicorn.

Uso (dalla radice del progetto, con il Python del venv):
    .venv/bin/python tools/difftest.py <AssassinsCreed_Dx9.exe> [--keep]

Passi:
  1. `acrek translate` traduce in C le funzioni degli scenari (build/gen/difftest.c);
  2. clang le compila con runtime/acrek.c in una libreria (build/difftest.so);
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
from harness import GDT, HEAP, HEAP_SIZE, PAGE, SENTINEL, STACK_SIZE, STACK_TOP, TEB, TRAMP  # noqa: E402,F401
from imports import iat_entries  # noqa: E402
from pe import PE  # noqa: E402

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


class Ctx(ctypes.Structure):
    pass


HOST_CALL = ctypes.CFUNCTYPE(ctypes.c_int, ctypes.POINTER(Ctx), ctypes.c_uint32)
Ctx._fields_ = [(r, ctypes.c_uint32) for r in ('eax', 'ecx', 'edx', 'ebx', 'esp', 'ebp', 'esi', 'edi',
                                                'df', 'fs_base')] + [
    ('mem', ctypes.c_void_p), ('host_call', HOST_CALL), ('trap_jmp', ctypes.c_void_p),
    ('trap_msg', ctypes.c_char_p), ('trap_addr', ctypes.c_uint32)]


class Native:
    """Stessa interfaccia di harness.Guest, ma esegue il codice tradotto."""

    def __init__(self, exe, lib):
        self.pe = pe = PE(exe)
        self.lib = lib = ctypes.CDLL(lib)
        lib.acrek_mem_create.restype = ctypes.c_void_p
        lib.acrek_run.argtypes = [ctypes.POINTER(Ctx), ctypes.c_uint32]
        self.mem = lib.acrek_mem_create()
        if not self.mem:
            raise harness.GuestError('impossibile riservare la memoria guest')
        self._sections = [(pe.image_base + va, pe.data[raw:raw + rs]) for _, va, vs, raw, rs in pe.sections]
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

    def reset(self):
        for va, data in self._sections:
            self.write(va, data)
        for slot, t in self._iat.items():
            self.write(slot, struct.pack('<I', t))
        self.write(HEAP, bytes(self.heap_top - HEAP))
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

    def call(self, addr, args=(), ecx=0, edx=0, max_insns=None):
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
                df=0, fs_base=TEB, mem=self.mem, host_call=self._cb)
        self._log, self._fault = [], None
        if self.lib.acrek_run(ctypes.byref(c), addr):
            raise harness.GuestError(self._fault or f'trap: {c.trap_msg.decode()} ({c.trap_addr:#010x})')
        return harness.Result(eax=c.eax, edx=c.edx, ecx=c.ecx, st0=None, popped=c.esp - esp0 - 4,
                              insns=None, imports=list(self._log))


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
            self.log.append((addr, tuple(args), r.eax, r.ecx, r.edx, r.popped, tuple(r.imports)))
            return r
        except harness.GuestError as err:
            self.log.append((addr, tuple(args), 'errore', str(err)))
            raise

    def snapshot(self):
        heap = self.e.read(HEAP, self.e.heap_top - HEAP)
        stack = self.e.read(STACK_TOP - 0x3000, 0x3000)
        return hashlib.sha1(heap).hexdigest(), hashlib.sha1(stack).hexdigest()


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


def build(exe):
    acrek = os.path.join(ROOT, 'target', 'release', 'acrek')
    gen = os.path.join(ROOT, 'build', 'gen', 'difftest.c')
    lib = os.path.join(ROOT, 'build', 'difftest.so')
    os.makedirs(os.path.dirname(gen), exist_ok=True)
    roots = sorted({a for _, _, addrs in SCENARIOS for a in addrs})
    subprocess.run([acrek, 'translate', exe, '--out', gen] + [f'{a:#x}' for a in roots], check=True)
    rt = os.path.join(ROOT, 'runtime')
    subprocess.run(['clang', '-O2', '-g', '-Wall', '-Wno-unused-variable', '-Wno-unused-but-set-variable',
                    '-Wno-unused-label', '-fwrapv', '-shared', '-fPIC', '-I', rt, gen,
                    os.path.join(rt, 'acrek.c'), '-o', lib], check=True)
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
            print(f'ok       {name}: {len(la)} chiamate identiche, heap e stack identici')
            continue
        failed += 1
        print(f'DIVERSO  {name}')
        if diff is not None:
            print(f'  chiamata {diff}:\n    originale {la[diff]}\n    tradotto  {lb[diff]}')
        elif len(la) != len(lb):
            print(f'  {len(la)} chiamate nell\'originale, {len(lb)} nel tradotto')
        else:
            print(f'  memoria finale diversa: heap {sa[0] == sb[0]}, stack {sa[1] == sb[1]}')
    print('difftest:', 'OK' if not failed else f'{failed} scenari diversi')
    return 1 if failed else 0


if __name__ == '__main__':
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    sys.exit(run(sys.argv[1]))
