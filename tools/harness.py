#!/usr/bin/env python3
"""Esegue funzioni dell'eseguibile originale in Unicorn: l'oracolo dei test differenziali.

Uso da riga di comando (serve unicorn: usare il Python del venv):
    .venv/bin/python tools/harness.py <exe> selftest
    .venv/bin/python tools/harness.py <exe> call 0xADDR [--ecx V] [--trace] [arg ...]
        gli argomenti sono interi (0x..) oppure stringhe: s:testo viene copiato in memoria
        guest e passato come puntatore

Uso da Python:
    from harness import Guest
    g = Guest(exe)
    p = g.alloc_str(b'ShaderPermutation')
    r = g.call(0x00AB1234, [p], ecx=0)        # r.eax, r.insns, r.imports

Memoria guest:
    0x00400000  immagine PE (sezioni ai loro indirizzi; nessuna rilocazione)
    0x10000000  heap di prova per gli argomenti (1 MB)
    0x6FF00000  stack (1 MB), cresce verso il basso
    0x7FFD0000  TEB (fs:[0] = fine catena SEH, fs:[0x18] = self, fs:[0x2C] = array TLS)
    0x7F000000  trampolini degli import (16 byte ciascuno)
    0x7FFE0000  GDT (serve solo a dare a fs la base del TEB)
    0x7FFF0000  indirizzo di ritorno sentinella
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
STACK_CLEAR = 0x10000      # parte alta dello stack azzerata da reset()
HEAP_CLEAR = HEAP_SIZE     # reset() azzera tutto lo heap di prova, anche la parte non allocata


class GuestError(Exception):
    pass


class Result:
    def __init__(self, **kw):
        self.__dict__.update(kw)

    def __repr__(self):
        return (f'eax={self.eax:#010x} edx={self.edx:#010x} tolti={self.popped} '
                f'import={[n for n, _ in self.imports]}'
                + (f' istruzioni={self.insns}' if self.insns is not None else ''))


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

        # Segmenti: in modalità a 32 bit la base di fs si imposta solo tramite una GDT.
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

        # Import: ogni cella IAT punta a un trampolino; l'hook decide cosa fare.
        self.imports = {}                          # indirizzo del trampolino -> (dll, nome)
        self.handlers = {}                         # nome -> funzione(guest, args_reader) -> (eax, byte da togliere)
        for i, (dll, fn, slot) in enumerate(iat_entries(pe)):
            t = TRAMP + 16 * i
            self.imports[t] = (dll, fn)
            uc.mem_write(slot, struct.pack('<I', t))
        self._iat = {slot: TRAMP + 16 * i for i, (_, _, slot) in enumerate(iat_entries(pe))}

        self._log = []
        self._insns = 0
        # L'hook costa una chiamata Python per istruzione: di norma copre solo i trampolini
        # degli import. Con trace=True copre tutto, e conta le istruzioni.
        self.trace = trace
        if trace:
            uc.hook_add(UC_HOOK_CODE, self._on_code)
        else:
            uc.hook_add(UC_HOOK_CODE, self._on_code, begin=TRAMP, end=TRAMP + 0x10000)
        uc.hook_add(UC_HOOK_MEM_UNMAPPED | UC_HOOK_MEM_WRITE_PROT, self._on_unmapped)
        self._fault = None
        # Sezioni che il codice può scrivere: si ripristinano tra un caso e l'altro. Le altre
        # sono protette in scrittura, così un puntatore sbagliato ferma il caso invece di
        # sporcare codice e costanti per i casi successivi.
        self._writable = []
        for name, va, vs, raw, rs in pe.sections:
            size = max(vs, rs)
            if name in ('.text', '.rdata', '.rsrc'):
                uc.mem_protect(base + va, (size + PAGE - 1) & ~(PAGE - 1), UC_PROT_READ | UC_PROT_EXEC)
            else:
                self._writable.append((base + va, pe.data[raw:raw + min(rs, size)].ljust(size, b'\0')))

    # --- memoria ---------------------------------------------------------
    def alloc(self, size, align=16):
        a = (self.heap_top + align - 1) & ~(align - 1)
        if a + size > HEAP + HEAP_SIZE:
            raise GuestError('heap di prova esaurito')
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
        """Riporta sezioni scrivibili, heap di prova e stack allo stato iniziale."""
        for va, data in self._writable:
            self.uc.mem_write(va, data)
        self.uc.mem_write(STACK_TOP - STACK_CLEAR, bytes(STACK_CLEAR))
        for slot, t in self._iat.items():
            self.uc.mem_write(slot, struct.pack('<I', t))
        self.uc.mem_write(HEAP, bytes(max(self.heap_top - HEAP, HEAP_CLEAR)))
        self.heap_top = HEAP

    # --- hook ------------------------------------------------------------
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
                self._fault = f'import non gestito: {dll}!{fn} (chiamato da {ret:#010x})'
                uc.emu_stop()
                return
            eax, pop = h(self, lambda i: self.u32(esp + 4 + 4 * i))
            uc.reg_write(UC_X86_REG_EAX, eax & 0xFFFFFFFF)
            uc.reg_write(UC_X86_REG_ESP, esp + 4 + pop)
            uc.reg_write(UC_X86_REG_EIP, ret)

    def _on_unmapped(self, uc, access, addr, size, value, _):
        eip = uc.reg_read(UC_X86_REG_EIP)
        self._fault = f'accesso a memoria non mappata o protetta {addr:#010x} da eip={eip:#010x}'
        return False

    # --- chiamata --------------------------------------------------------
    def _x87(self):
        """(cima dello stack, gli otto registri fisici come 10 byte ciascuno)."""
        regs = []
        for i in range(8):
            mant, exp = self.uc.reg_read(UC_X86_REG_FP0 + i)
            regs.append(mant.to_bytes(8, 'little') + (exp & 0xFFFF).to_bytes(2, 'little'))
        return (self.uc.reg_read(UC_X86_REG_FPSW) >> 11) & 7, tuple(regs)

    def image_state(self):
        """Contenuto attuale delle sezioni scrivibili (per i confronti)."""
        return b''.join(self.read(va, len(data)) for va, data in self._writable)

    def call(self, addr, args=(), ecx=0, edx=0, max_insns=5_000_000, xmm=None):
        """Chiama `addr` con gli argomenti sullo stack (cdecl/stdcall/thiscall con ecx).
        `xmm`: otto interi a 128 bit con il valore iniziale dei registri XMM (default zero)."""
        uc = self.uc
        teb = bytearray(PAGE * 2)
        struct.pack_into('<I', teb, 0x00, 0xFFFFFFFF)            # fine della catena SEH
        struct.pack_into('<I', teb, 0x04, STACK_TOP)             # base dello stack
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
        # I flag vanno azzerati: una funzione interrotta può lasciare impostato il flag di
        # direzione, e le `rep movs`/`rep stos` successive andrebbero all'indietro.
        uc.reg_write(UC_X86_REG_EFLAGS, 0x202)
        for i in range(8):
            uc.reg_write(UC_X86_REG_XMM0 + i, xmm[i] if xmm else 0)
        # Scrivere il registro MXCSR dall'esterno non aggiorna lo stato interno dell'emulatore
        # (arrotondamento, flush-to-zero, denormali): un `ldmxcsr` eseguito da una funzione
        # resterebbe attivo per le chiamate successive. Lo si reimposta eseguendo l'istruzione.
        # Lo stesso vale per l'x87: `fninit` e `fldcw` con la parola di controllo di Windows
        # (0x027F, precisione a 53 bit), poi registri azzerati.
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
            raise GuestError(self._fault or f'{e} a eip={uc.reg_read(UC_X86_REG_EIP):#010x}')
        if self._fault:
            raise GuestError(self._fault)
        if uc.reg_read(UC_X86_REG_EIP) != SENTINEL:
            raise GuestError(f'limite di {max_insns} istruzioni raggiunto')
        regs = tuple(uc.reg_read(r) for r in (UC_X86_REG_EAX, UC_X86_REG_ECX, UC_X86_REG_EDX, UC_X86_REG_EBX,
                                              UC_X86_REG_EBP, UC_X86_REG_ESI, UC_X86_REG_EDI))
        return Result(eax=regs[0], ecx=regs[1], edx=regs[2], regs=regs,
                      xmm=tuple(uc.reg_read(UC_X86_REG_XMM0 + i) for i in range(8)),
                      x87=self._x87(),
                      st0=uc.reg_read(UC_X86_REG_ST0),
                      popped=(uc.reg_read(UC_X86_REG_ESP) - esp0 - 4) & 0xFFFFFFFF,   # byte tolti dal chiamato
                      insns=self._insns if self.trace else None, imports=list(self._log))


def _selftest(exe):
    import zlib
    g = Guest(exe)
    ok = True
    for name, addr, fn in SELFTESTS:
        try:
            passed = fn(g, addr, zlib)
            print(f'{"ok     " if passed else "FALLITO"} {name} @ {addr:#010x}')
            ok &= passed
            g.reset()
        except GuestError as e:
            print(f'FALLITO {name}: {e}')
            ok = False
    print('selftest:', 'OK' if ok else 'FALLITO')
    return 0 if ok else 1


def _t_strlen(g, addr, zlib):
    for n in (0, 1, 3, 4, 5, 31, 32, 255, 1000):
        for shift in range(4):                      # tutti gli allineamenti
            p = g.alloc(n + 8) + shift
            g.write(p, b'x' * n + b'\0')
            r = g.call(addr, [p])
            if r.eax != n:
                print(f'strlen({n}, allineamento {shift}) = {r.eax}')
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
            print(f'strcmp({x!r}, {y!r}) = {got}, atteso segno {want}')
            return False
    return True


def _t_memcpy(g, addr, zlib):
    # Percorre le jump table del memcpy di MSVC: ogni lunghezza e ogni allineamento.
    src0 = g.alloc(600)
    data = bytes((i * 7 + 3) & 0xFF for i in range(600))
    g.write(src0, data)
    for n in list(range(0, 40)) + [63, 64, 65, 127, 128, 129, 255, 256, 511]:
        for sa in range(4):
            for da in range(4):
                dst = g.alloc(n + 16) + da
                r = g.call(addr, [dst, src0 + sa, n])
                if r.eax != dst or g.read(dst, n) != data[sa:sa + n]:
                    print(f'memcpy(n={n}, src+{sa}, dst+{da}) errato')
                    return False
    return True


# (nome, indirizzo nella build Steam DX9, prova). Gli indirizzi sono stati trovati dalle firme
# di byte delle routine assembly del CRT di MSVC 8; vedi docs/analisi/harness.md.
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
            sys.exit(f'errore: {e}')
        return
    sys.exit(__doc__)


if __name__ == '__main__':
    main(sys.argv)
