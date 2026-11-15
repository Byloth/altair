#!/usr/bin/env python3
"""Runs functions of the original executable in Unicorn: the oracle of the differential tests.

Command-line usage (unicorn is needed: use the venv's Python):
    .venv/bin/python tools/harness.py <exe> selftest
    .venv/bin/python tools/harness.py <exe> call 0xADDR [--ecx V] [--trace] [arg ...]
        the arguments are integers (0x..) or strings: s:text is copied into guest
        memory and passed as a pointer

Usage from Python:
    from harness import Guest
    g = Guest(exe)
    p = g.alloc_str(b'ShaderPermutation')
    r = g.call(0x00AB1234, [p], ecx=0)        # r.eax, r.insns, r.imports

Guest memory:
    0x00400000  PE image (sections at their addresses; no relocations)
    0x10000000  test heap for the arguments (1 MB)
    0x6FF00000  stack (1 MB), grows downwards
    0x7FFD0000  TEB (fs:[0] = end of SEH chain, fs:[0x18] = self, fs:[0x2C] = TLS array)
    0x7F000000  import trampolines (16 bytes each)
    0x7FFE0000  GDT (only needed to give fs the base of the TEB)
    0x7FFF0000  sentinel return address
"""
import os
import struct
import sys

from unicorn import (UC_ARCH_X86, UC_HOOK_CODE, UC_HOOK_MEM_UNMAPPED, UC_HOOK_MEM_WRITE_PROT, UC_MODE_32, UC_PROT_EXEC, UC_PROT_READ, Uc,
                     UcError)
from unicorn.x86_const import (UC_X86_REG_EAX, UC_X86_REG_EBP, UC_X86_REG_EBX, UC_X86_REG_ECX,
                               UC_X86_REG_EDI, UC_X86_REG_EDX, UC_X86_REG_EFLAGS, UC_X86_REG_EIP, UC_X86_REG_ESI,
                               UC_X86_REG_ESP, UC_X86_REG_CS, UC_X86_REG_DS, UC_X86_REG_ES,
                               UC_X86_REG_FP0, UC_X86_REG_FPSW, UC_X86_REG_FS, UC_X86_REG_GDTR, UC_X86_REG_MXCSR, UC_X86_REG_SS,
                               UC_X86_REG_ST0, UC_X86_REG_XMM0)

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from imports import iat_entries  # noqa: E402
from pe import PE  # noqa: E402

HEAP, HEAP_SIZE = 0x10000000, 0x100000
STACK_TOP, STACK_SIZE = 0x70000000, 0x100000
TEB = 0x7FFD0000
TRAMP = 0x7F000000
SENTINEL = 0x7FFF0000
GDT = 0x7FFE0000
PAGE = 0x1000
STACK_CLEAR = 0x10000      # upper part of the stack zeroed by reset()
HEAP_CLEAR = HEAP_SIZE     # reset() zeroes the whole test heap, including the unallocated part


class GuestError(Exception):
    pass


class Result:
    def __init__(self, **kw):
        self.__dict__.update(kw)

    def __repr__(self):
        return (f'eax={self.eax:#010x} edx={self.edx:#010x} popped={self.popped} '
                f'imports={[n for n, _ in self.imports]}'
                + (f' instructions={self.insns}' if self.insns is not None else ''))


class Guest:
    def __init__(self, exe, trace=False):
        self.pe = pe = PE(exe)
        self.uc = uc = Uc(UC_ARCH_X86, UC_MODE_32)
        base = pe.image_base
        end = max(va + max(vs, rs) for _, va, vs, _, rs in pe.sections)
        uc.mem_map(base, (end + PAGE - 1) & ~(PAGE - 1))
        for _, va, vs, raw, rs in pe.sections:
            uc.mem_write(base + va, pe.data[raw:raw + min(rs, max(vs, rs))])

        uc.mem_map(HEAP, HEAP_SIZE)
        uc.mem_map(STACK_TOP - STACK_SIZE, STACK_SIZE)
        uc.mem_map(TEB, PAGE * 2)
        uc.mem_map(TRAMP, 0x10000)
        uc.mem_map(SENTINEL, PAGE)
        self.heap_top = HEAP

        # Segments: in 32-bit mode the base of fs can only be set through a GDT.
        def desc(base, limit, access):
            return struct.pack('<HHBBBB', limit & 0xFFFF, base & 0xFFFF, (base >> 16) & 0xFF,
                               access, 0xC0 | ((limit >> 16) & 0xF), (base >> 24) & 0xFF)
        uc.mem_map(GDT, PAGE)
        uc.mem_write(GDT, bytes(8) + desc(0, 0xFFFFF, 0x9B) + desc(0, 0xFFFFF, 0x93)
                     + desc(TEB, 0xFFFFF, 0x93))
        uc.reg_write(UC_X86_REG_GDTR, (0, GDT, 4 * 8 - 1, 0))
        uc.reg_write(UC_X86_REG_CS, 0x08)
        for reg in (UC_X86_REG_SS, UC_X86_REG_DS, UC_X86_REG_ES):
            uc.reg_write(reg, 0x10)
        uc.reg_write(UC_X86_REG_FS, 0x18)

        # Imports: each IAT cell points to a trampoline; the hook decides what to do.
        self.imports = {}                          # trampoline address -> (dll, name)
        self.handlers = {}                         # name -> function(guest, args_reader) -> (eax, bytes to pop)
        for i, (dll, fn, slot) in enumerate(iat_entries(pe)):
            t = TRAMP + 16 * i
            self.imports[t] = (dll, fn)
            uc.mem_write(slot, struct.pack('<I', t))
        self._iat = {slot: TRAMP + 16 * i for i, (_, _, slot) in enumerate(iat_entries(pe))}

        self._log = []
        self._insns = 0
        # The hook costs one Python call per instruction: normally it covers only the import
        # trampolines. With trace=True it covers everything, and counts the instructions.
        self.trace = trace
        if trace:
            uc.hook_add(UC_HOOK_CODE, self._on_code)
        else:
            uc.hook_add(UC_HOOK_CODE, self._on_code, begin=TRAMP, end=TRAMP + 0x10000)
        uc.hook_add(UC_HOOK_MEM_UNMAPPED | UC_HOOK_MEM_WRITE_PROT, self._on_unmapped)
        self._fault = None
        # Sections the code may write to: they are restored between one case and the next. The others
        # are write-protected, so a wrong pointer stops the case instead of
        # dirtying code and constants for the following cases.
        self._writable = []
        for name, va, vs, raw, rs in pe.sections:
            size = max(vs, rs)
            if name in ('.text', '.rdata', '.rsrc'):
                uc.mem_protect(base + va, (size + PAGE - 1) & ~(PAGE - 1), UC_PROT_READ | UC_PROT_EXEC)
            else:
                self._writable.append((base + va, pe.data[raw:raw + min(rs, size)].ljust(size, b'\0')))

    # --- memory ----------------------------------------------------------
    def alloc(self, size, align=16):
        a = (self.heap_top + align - 1) & ~(align - 1)
        if a + size > HEAP + HEAP_SIZE:
            raise GuestError('test heap exhausted')
        self.heap_top = a + size
        return a

    def alloc_bytes(self, data):
        a = self.alloc(len(data) or 1)
        self.uc.mem_write(a, bytes(data))
        return a

    def alloc_str(self, s):
        return self.alloc_bytes(bytes(s) + b'\0')

    def write(self, addr, data):
        self.uc.mem_write(addr, bytes(data))

    def read(self, addr, size):
        return bytes(self.uc.mem_read(addr, size))

    def u32(self, addr):
        return struct.unpack('<I', self.read(addr, 4))[0]

    def reset(self):
        """Brings writable sections, test heap and stack back to their initial state."""
        for va, data in self._writable:
            self.uc.mem_write(va, data)
        self.uc.mem_write(STACK_TOP - STACK_CLEAR, bytes(STACK_CLEAR))
        for slot, t in self._iat.items():
            self.uc.mem_write(slot, struct.pack('<I', t))
        self.uc.mem_write(HEAP, bytes(max(self.heap_top - HEAP, HEAP_CLEAR)))
        self.heap_top = HEAP

    # --- hooks -----------------------------------------------------------
    def _on_code(self, uc, addr, size, _):
        self._insns += 1
        if self.trace and addr < TRAMP:
            print(f'  {addr:#010x}', file=sys.stderr)
        if addr in self.imports:
            dll, fn = self.imports[addr]
            esp = uc.reg_read(UC_X86_REG_ESP)
            ret = self.u32(esp)
            name = fn.split(' ')[0]
            self._log.append((name, ret))
            h = self.handlers.get(name)
            if h is None:
                self._fault = f'unhandled import: {dll}!{fn} (called from {ret:#010x})'
                uc.emu_stop()
                return
            eax, pop = h(self, lambda i: self.u32(esp + 4 + 4 * i))
            uc.reg_write(UC_X86_REG_EAX, eax & 0xFFFFFFFF)
            uc.reg_write(UC_X86_REG_ESP, esp + 4 + pop)
            uc.reg_write(UC_X86_REG_EIP, ret)

    def _on_unmapped(self, uc, access, addr, size, value, _):
        eip = uc.reg_read(UC_X86_REG_EIP)
        self._fault = f'access to unmapped or protected memory {addr:#010x} from eip={eip:#010x}'
        return False

    # --- call ------------------------------------------------------------
    def _x87(self):
        """(top of the stack, the eight physical registers as 10 bytes each)."""
        regs = []
        for i in range(8):
            mant, exp = self.uc.reg_read(UC_X86_REG_FP0 + i)
            regs.append(mant.to_bytes(8, 'little') + (exp & 0xFFFF).to_bytes(2, 'little'))
        return (self.uc.reg_read(UC_X86_REG_FPSW) >> 11) & 7, tuple(regs)

    def image_state(self):
        """Current contents of the writable sections (for comparisons)."""
        return b''.join(self.read(va, len(data)) for va, data in self._writable)

    def call(self, addr, args=(), ecx=0, edx=0, max_insns=5_000_000, xmm=None):
        """Calls `addr` with the arguments on the stack (cdecl/stdcall/thiscall with ecx).
        `xmm`: eight 128-bit integers with the initial value of the XMM registers (default zero)."""
        uc = self.uc
        teb = bytearray(PAGE * 2)
        struct.pack_into('<I', teb, 0x00, 0xFFFFFFFF)            # end of the SEH chain
        struct.pack_into('<I', teb, 0x04, STACK_TOP)             # stack base
        struct.pack_into('<I', teb, 0x08, STACK_TOP - STACK_SIZE)
        struct.pack_into('<I', teb, 0x18, TEB)                   # self
        struct.pack_into('<I', teb, 0x2C, TEB + PAGE)            # ThreadLocalStoragePointer
        uc.mem_write(TEB, bytes(teb))

        esp = STACK_TOP - 0x1000
        frame = struct.pack(f'<{len(args) + 1}I', SENTINEL, *[a & 0xFFFFFFFF for a in args])
        esp -= len(frame)
        uc.mem_write(esp, frame)
        esp0 = esp
        for reg, v in ((UC_X86_REG_ESP, esp), (UC_X86_REG_EBP, 0), (UC_X86_REG_ECX, ecx),
                       (UC_X86_REG_EDX, edx), (UC_X86_REG_EAX, 0), (UC_X86_REG_EBX, 0),
                       (UC_X86_REG_ESI, 0), (UC_X86_REG_EDI, 0)):
            uc.reg_write(reg, v & 0xFFFFFFFF)
        # The flags must be cleared: an interrupted function can leave the direction flag
        # set, and the following `rep movs`/`rep stos` would run backwards.
        uc.reg_write(UC_X86_REG_EFLAGS, 0x202)
        for i in range(8):
            uc.reg_write(UC_X86_REG_XMM0 + i, xmm[i] if xmm else 0)
        # Writing the MXCSR register from outside does not update the emulator's internal state
        # (rounding, flush-to-zero, denormals): an `ldmxcsr` executed by a function
        # would stay in effect for the following calls. It is reset by executing the instruction.
        # The same goes for the x87: `fninit` and `fldcw` with Windows' control word
        # (0x027F, 53-bit precision), then registers zeroed.
        uc.mem_write(SENTINEL + 0x100, b'\x0f\xae\x15' + struct.pack('<I', SENTINEL + 0x200)
                     + b'\xdb\xe3\xd9\x2d' + struct.pack('<I', SENTINEL + 0x204))
        uc.mem_write(SENTINEL + 0x200, struct.pack('<IH', 0x1F80, 0x027F))
        uc.emu_start(SENTINEL + 0x100, SENTINEL + 0x10F)
        for i in range(8):
            uc.reg_write(UC_X86_REG_FP0 + i, (0, 0))
        self._log, self._insns, self._fault = [], 0, None
        try:
            uc.emu_start(addr, SENTINEL, count=max_insns)
        except UcError as e:
            raise GuestError(self._fault or f'{e} at eip={uc.reg_read(UC_X86_REG_EIP):#010x}')
        if self._fault:
            raise GuestError(self._fault)
        if uc.reg_read(UC_X86_REG_EIP) != SENTINEL:
            raise GuestError(f'limit of {max_insns} instructions reached')
        regs = tuple(uc.reg_read(r) for r in (UC_X86_REG_EAX, UC_X86_REG_ECX, UC_X86_REG_EDX, UC_X86_REG_EBX,
                                              UC_X86_REG_EBP, UC_X86_REG_ESI, UC_X86_REG_EDI))
        return Result(eax=regs[0], ecx=regs[1], edx=regs[2], regs=regs,
                      xmm=tuple(uc.reg_read(UC_X86_REG_XMM0 + i) for i in range(8)),
                      x87=self._x87(),
                      st0=uc.reg_read(UC_X86_REG_ST0),
                      popped=(uc.reg_read(UC_X86_REG_ESP) - esp0 - 4) & 0xFFFFFFFF,   # bytes popped by the callee
                      insns=self._insns if self.trace else None, imports=list(self._log))


def _selftest(exe):
    import zlib
    g = Guest(exe)
    ok = True
    for name, addr, fn in SELFTESTS:
        try:
            passed = fn(g, addr, zlib)
            print(f'{"ok    " if passed else "FAILED"} {name} @ {addr:#010x}')
            ok &= passed
            g.reset()
        except GuestError as e:
            print(f'FAILED {name}: {e}')
            ok = False
    print('selftest:', 'OK' if ok else 'FAILED')
    return 0 if ok else 1


def _t_strlen(g, addr, zlib):
    for n in (0, 1, 3, 4, 5, 31, 32, 255, 1000):
        for shift in range(4):                      # all the alignments
            p = g.alloc(n + 8) + shift
            g.write(p, b'x' * n + b'\0')
            r = g.call(addr, [p])
            if r.eax != n:
                print(f'strlen({n}, alignment {shift}) = {r.eax}')
                return False
    return True


def _t_strcmp(g, addr, zlib):
    cases = [(b'', b''), (b'a', b''), (b'', b'a'), (b'abc', b'abc'), (b'abc', b'abd'),
             (b'abd', b'abc'), (b'Altair', b'Altair!'), (b'scimitar' * 9, b'scimitar' * 9),
             (b'\xff', b'\x01')]
    for x, y in cases:
        r = g.call(addr, [g.alloc_str(x), g.alloc_str(y)])
        got = struct.unpack('<i', struct.pack('<I', r.eax))[0]
        want = (x > y) - (x < y)
        if (got > 0) - (got < 0) != want:
            print(f'strcmp({x!r}, {y!r}) = {got}, expected sign {want}')
            return False
    return True


def _t_memcpy(g, addr, zlib):
    # Walks the jump tables of MSVC's memcpy: every length and every alignment.
    src0 = g.alloc(600)
    data = bytes((i * 7 + 3) & 0xFF for i in range(600))
    g.write(src0, data)
    for n in list(range(0, 40)) + [63, 64, 65, 127, 128, 129, 255, 256, 511]:
        for sa in range(4):
            for da in range(4):
                dst = g.alloc(n + 16) + da
                r = g.call(addr, [dst, src0 + sa, n])
                if r.eax != dst or g.read(dst, n) != data[sa:sa + n]:
                    print(f'memcpy(n={n}, src+{sa}, dst+{da}) wrong')
                    return False
    return True


# (name, address in the Steam DX9 build, test). The addresses were found from the byte
# signatures of the assembly routines of the MSVC 8 CRT; see docs/analysis/harness.md.
SELFTESTS = [
    ('strlen', 0x00FA6790, _t_strlen),
    ('strcmp', 0x00FA9760, _t_strcmp),
    ('memcpy', 0x00F9FB80, _t_memcpy),
]


def main(argv):
    if len(argv) < 3:
        sys.exit(__doc__)
    exe, cmd = argv[1], argv[2]
    if cmd == 'selftest':
        sys.exit(_selftest(exe))
    if cmd == 'call':
        rest = argv[3:]
        g = Guest(exe, trace='--trace' in rest)
        ecx = 0
        if '--ecx' in rest:
            ecx = int(rest[rest.index('--ecx') + 1], 0)
            del rest[rest.index('--ecx'):rest.index('--ecx') + 2]
        rest = [a for a in rest if a != '--trace']
        addr = int(rest[0], 0)
        args = [g.alloc_str(a[2:].encode()) if a.startswith('s:') else int(a, 0) for a in rest[1:]]
        try:
            print(g.call(addr, args, ecx=ecx))
        except GuestError as e:
            sys.exit(f'error: {e}')
        return
    sys.exit(__doc__)


if __name__ == '__main__':
    main(sys.argv)
