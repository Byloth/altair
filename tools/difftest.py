#!/usr/bin/env python3
"""Differential test: the code translated by `altair` against the original run in Unicorn.

Usage (from the project root, with the venv's Python):
    .venv/bin/python tools/difftest.py <AssassinsCreed_Dx9.exe>                 hand-written scenarios
    .venv/bin/python tools/difftest.py <AssassinsCreed_Dx9.exe> --auto N [--cases K]
        N functions chosen by `altair candidates`, K cases each (8) with generated inputs;
    .venv/bin/python tools/difftest.py <AssassinsCreed_Dx9.exe> --funcs 0xA,0xB [--cases K]
        the same on the given functions, even if they are not candidates;
    ... --auto N --lib build/libaltair_ac1-clang.so
        uses the library of the whole image (`altair image` + make) instead of compiling the sample;
        the detail goes to build/difftest-auto.txt

Steps:
  1. `altair translate` translates the scenarios' functions into C (build/gen/difftest.c);
  2. clang compiles them with runtime/altair.c into a library (build/difftest.so);
  3. each scenario runs twice with the same inputs: on the original in Unicorn
     (tools/harness.py) and on the translated code, in an identical guest memory;
  4. the two are compared, call by call: eax, ecx, edx, the bytes popped off the stack, the imports
     called, and at the end the contents of test heap and stack.

A scenario is a function `scenario(g, rng)` that uses only the interface common to `Guest`
(Unicorn) and `Native` (translated): alloc, alloc_str, write, read, call.
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
    ('fpcw', ctypes.c_uint32), ('fpsw', ctypes.c_uint32), ('eflags_id', ctypes.c_uint32)]


class Native:
    """Same interface as harness.Guest, but runs the translated code."""

    def __init__(self, exe, lib):
        self.pe = pe = PE(exe)
        self.lib = lib = ctypes.CDLL(lib)
        lib.altair_mem_create.restype = ctypes.c_void_p
        lib.altair_run.argtypes = [ctypes.POINTER(Ctx), ctypes.c_uint32]
        self.mem = lib.altair_mem_create()
        if not self.mem:
            raise harness.GuestError('cannot reserve the guest memory')
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
            raise harness.GuestError('test heap exhausted')
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
        """Has the translated code written into code or constants? If so, restores them."""
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
            self._fault = f'call to {addr:#010x}: neither a translated function nor an import'
            return 1
        dll, fn = self.imports[addr]
        ret = self.u32(c.esp)
        name = fn.split(' ')[0]
        self._log.append((name, ret))
        h = self.handlers.get(name)
        if h is None:
            self._fault = f'unhandled import: {dll}!{fn} (called from {ret:#010x})'
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
    """Wraps an engine and records what can be observed of each call."""

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
            self.log.append((addr, tuple(args), 'error', str(err)))
            raise

    def snapshot(self):
        heap = self.e.read(HEAP, self.e.heap_top - HEAP)
        stack = self.e.read(STACK_TOP - STACK_CLEAR, STACK_CLEAR)
        return (hashlib.sha1(heap).hexdigest(), hashlib.sha1(stack).hexdigest(),
                hashlib.sha1(self.e.image_state()).hexdigest())


# --- scenarios -------------------------------------------------------------------------------

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
    # Overlapping regions, forwards and backwards (path with the direction flag).
    buf = g.alloc_bytes(bytes(rng.randrange(256) for _ in range(900)))
    for _ in range(200):
        n = rng.randrange(0, 300)
        s = rng.randrange(0, 300)
        d = s + rng.randrange(-40, 41)
        if d >= 0:
            g.call(MEMCPY, [buf + d, buf + s, n])


def sc_floor(g, rng):
    """The CRT's `floor`, x87 branch: sets RC towards minus infinity with `fldcw` and rounds with `frndint`.
    Half-integers tell the right mode apart from round-to-nearest."""
    values = [11.5, 8.5, 2.5, 3.5, -2.5, -3.5, 0.5, -0.5, 0.0, -0.0, 11.4999, 1e10 + 0.5, 4503599627370497.0, 1e300,
              float('inf'), float('-inf')]
    values += [rng.uniform(-1e6, 1e6) for _ in range(200)] + [rng.randrange(-1000, 1000) + 0.5 for _ in range(100)]
    for v in values:
        lo, hi = struct.unpack('<II', struct.pack('<d', v))
        g.call(FLOOR, [lo, hi])


def sc_cpuid(g, rng):
    """The CRT decides whether to use SSE2: it flips the ID bit of EFLAGS (`pushfd`/`popfd`), then `cpuid`."""
    # Only the verdict is compared: the other registers and the stack are left with the bits of `cpuid`, which between
    # the CPU emulated by Unicorn and the one declared by the runtime are bound to differ.
    g.log.append(('sse2', g.e.call(SSE2_AVAILABLE, []).eax))
    g.write(STACK_TOP - STACK_CLEAR, bytes(STACK_CLEAR))    # the bits of `cpuid` remain on the stack too


FLOOR, SSE2_AVAILABLE = 0x00FA1120, 0x00FCA495
SCENARIOS = [('strlen', sc_strlen, [STRLEN]), ('strcmp', sc_strcmp, [STRCMP]), ('memcpy', sc_memcpy, [MEMCPY]),
             ('floor', sc_floor, [FLOOR]), ('cpuid', sc_cpuid, [SSE2_AVAILABLE])]


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
            print(f'ok         {name}: {len(la)} identical calls; heap, stack and global data identical')
            continue
        failed += 1
        print(f'DIFFERENT  {name}')
        if diff is not None:
            print(f'  call {diff}:\n    original   {la[diff]}\n    translated {lb[diff]}')
        elif len(la) != len(lb):
            print(f'  {len(la)} calls in the original, {len(lb)} in the translated')
        else:
            print(f'  final memory: heap {"=" if sa[0] == sb[0] else "DIFFERENT"}, '
                  f'stack {"=" if sa[1] == sb[1] else "DIFFERENT"}, data {"=" if sa[2] == sb[2] else "DIFFERENT"}')
    print('difftest:', 'OK' if not failed else f'{failed} scenarios differ')
    return 1 if failed else 0


SPECIAL_FLOATS = [0.0, -0.0, 1.0, -1.0, 0.5, float('inf'), float('-inf'), float('nan'), 1e-40, 3.4e38, 2147483648.0,
                  -2147483904.0, 16777216.0]


def make_inputs(g, rng):
    """Generic inputs: buffers full of pointers to other buffers, small integers, floats and
    random values; arguments and registers chosen among pointers and integers."""
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
    # Initial XMM registers: four floats each, sometimes two doubles.
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
    """Two blocks of bytes are equal, except for words that are NaN in both.

    When both operands of an SSE operation are NaN, x86 hardware returns
    the first; the emulator (QEMU) applies the x87 rule and may return the other. The translated
    code follows the hardware, so sign and payload of a NaN may differ."""
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
        j = i & ~1                      # the double word that contains it, aligned to 8
        if j + 1 < len(wa) and nan64(wa[j], wa[j + 1]) and nan64(wb[j], wb[j + 1]):
            continue
        return False
    return True


def x87_same_but_nan(a, b):
    """Same top of the stack and same registers, except for registers that are NaN in both."""
    def nan(r):
        return (r[9] & 0x7F) == 0x7F and r[8] == 0xFF

    def mmx(x, y):
        # After an MMX instruction the hardware sets the register's exponent to all ones;
        # the emulator leaves it as it was. The translated code (y) follows the hardware.
        return x[:8] == y[:8] and y[8:] == b'\xff\xff'
    return a[0] == b[0] and all(x == y or (nan(x) and nan(y)) or mmx(x, y) for x, y in zip(a[1], b[1]))


def xmm_bytes(regs):
    return b''.join(v.to_bytes(16, 'little') for v in regs)


FIELDS = ('registers (eax ecx edx ebx ebp esi edi)', 'xmm registers', 'x87 state', 'bytes popped off the stack', 'imports', 'heap', 'stack',
          'global data')


def instruction_report(gen, ok_roots):
    """Which kinds of instruction appear in the functions that turned out identical (with the functions
    they call). Appearing does not guarantee that the test case went through there."""
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


def run_auto(exe, count, cases, only=None, lib=None):
    import signal
    if only:
        roots = only
    else:
        out = subprocess.run([ALTAIR, 'candidates', exe, '--max', str(count)], check=True, capture_output=True,
                             text=True)
        print(out.stderr.strip())
        roots = [int(x, 16) for x in out.stdout.split()]
    if lib is None:
        lib = build(exe, roots, name='difftest-auto', opt='-O1')
        gen = os.path.join(ROOT, 'build', 'gen', 'difftest-auto.c')
    else:
        gen = None                      # library of the whole image: already compiled
        print('library:', lib)
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
                        f.write(f'{addr:#x} case {case}\n')
                    signal.alarm(60)       # an infinite loop in the translated code kills the process
                try:
                    r = engine.call(addr, args, ecx=ecx, edx=edx, max_insns=300_000, xmm=xmm)
                    obs.append(observe(engine, r))
                except harness.GuestError as e:
                    obs.append(('error', str(e)))
                finally:
                    signal.alarm(0)
                if obs[0][0] == 'error':
                    break                   # the original does not run with these inputs: case skipped
            if obs[0][0] == 'error':
                continue
            ran += 1
            if obs[0] != obs[1] and obs[1][0] != 'error':
                # Tolerance on NaNs only, for XMM registers and memory (indices 1, 4, 5, 6).
                o0 = (obs[0][0], xmm_bytes(obs[0][1])) + obs[0][2:]
                o1 = (obs[1][0], xmm_bytes(obs[1][1])) + obs[1][2:]
                if all(same_but_nan(x, y) if k in (1, 5, 6, 7) else x87_same_but_nan(x, y) if k == 2 else x == y
                       for k, (x, y) in enumerate(zip(o0, o1))):
                    nan_only.add(addr)
                    continue
            if obs[0] != obs[1] and bad is None:
                if obs[1][0] == 'error':
                    bad = (case, f'the translated code stops: {obs[1][1]}')
                else:
                    k = next(i for i in range(len(FIELDS)) if obs[0][i] != obs[1][i])
                    detail = (f'{obs[0][k]} versus {obs[1][k]}' if k in (0, 3, 4) else
                              f'top {obs[0][2][0]} versus {obs[1][2][0]}; ' + ' '.join(
                                  f'fp{i}: {a.hex()} versus {b.hex()}' for i, (a, b) in
                                  enumerate(zip(obs[0][2][1], obs[1][2][1])) if a != b) if k == 2 else
                              ' '.join(f'xmm{i}: {a:032x} versus {b:032x}' for i, (a, b) in
                                       enumerate(zip(obs[0][1], obs[1][1])) if a != b) if k == 1 else '')
                    bad = (case, f'{FIELDS[k]} {detail}')
        if native.readonly_damaged() and not bad:
            bad = ('-', 'the translated code wrote into read-only sections')
        if bad:
            differ.append(addr)
            report.append(f'DIFFERENT {addr:#010x} case {bad[0]}: {bad[1]}')
        elif ran:
            same.append(addr)
            report.append(f'ok        {addr:#010x} ({ran} cases)')
        else:
            never.append(addr)
            report.append(f'skipped   {addr:#010x} (the original does not run with the generated inputs)')
    with open(os.path.join(ROOT, 'build', 'difftest-auto.txt'), 'w') as f:
        f.write('\n'.join(report) + '\n')
    print(f'{len(roots)} functions: {len(same)} identical, {len(differ)} different, {len(never)} skipped')
    nan_ok = sorted(nan_only - set(differ))
    if nan_ok:
        print(f'  of which {len(nan_ok)} identical except for a NaN or the exponent of an MMX register: '
              + ' '.join(f'{a:#x}' for a in nan_ok[:8]))
    if gen:
        ndone, verified, missing = instruction_report(gen, same)
        print(f'functions executed in identical cases, including those called: {ndone}; '
              f'{sum(verified.values())} instructions of {len(verified)} kinds')
        if missing:
            print('kinds translated but absent from the verified functions:', ' '.join(missing))
        with open(os.path.join(ROOT, 'build', 'difftest-auto.txt'), 'a') as f:
            f.write('\ninstructions in the verified functions:\n')
            f.write(''.join(f'{n:7} {m}\n' for m, n in verified.most_common()))
    for line in [x for x in report if x.startswith('DIFFERENT')][:25]:
        print(' ', line[:200])
    return 1 if differ else 0


if __name__ == '__main__':
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    if '--auto' in sys.argv:
        n = int(sys.argv[sys.argv.index('--auto') + 1])
        k = int(sys.argv[sys.argv.index('--cases') + 1]) if '--cases' in sys.argv else 8
        lib = os.path.abspath(sys.argv[sys.argv.index('--lib') + 1]) if '--lib' in sys.argv else None
        sys.exit(run_auto(sys.argv[1], n, k, lib=lib))
    if '--funcs' in sys.argv:
        addrs = [int(x, 16) for x in sys.argv[sys.argv.index('--funcs') + 1].split(',')]
        k = int(sys.argv[sys.argv.index('--cases') + 1]) if '--cases' in sys.argv else 8
        sys.exit(run_auto(sys.argv[1], 0, k, only=addrs))
    sys.exit(run(sys.argv[1]))
