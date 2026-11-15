#!/usr/bin/env python3
"""Discovery of the functions of a PE32 without symbols or relocations (prototype).

Usage (capstone is needed: use the venv's Python):
    .venv/bin/python tools/functions.py <exe> [--strict] [--out private/functions.json]

Method, in order of reliability of the starting points:
  1. strong: entry point, exports, TLS callbacks and every direct `call` target in the
     code reached from these;
  2. slots of the vtables found by the RTTI;
  3. code addresses used as immediates (`push offset f`, `mov [x], offset f`);
  4. words in the data sections that point into the code.
Starting points 2-4 are "weak": they are accepted only if the trial decoding does not run into
invalid bytes, instructions a game never executes, or the middle of an instruction already
known. 3 and 4 must also "look like a start" (aligned to 16 or after padding/ret).

The real recompiler (Phase 1, Rust) will redo this work: what is needed here are the numbers and the list.
"""
import json
import os
import re
import struct
import sys
import time
from collections import Counter

from capstone import CS_ARCH_X86, CS_MODE_32, Cs

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pe import PE  # noqa: E402
import rtti  # noqa: E402

NEVER = {'in', 'out', 'insb', 'insd', 'insw', 'outsb', 'outsd', 'outsw', 'les', 'lds', 'aaa',
         'aas', 'aam', 'aad', 'daa', 'das', 'iretd', 'iret', 'hlt', 'cli', 'sti', 'salc', 'int1',
         'into', 'bound', 'arpl', 'lcall', 'ljmp', 'retf', 'int', 'lgdt', 'lidt', 'xlatb',
         'popal', 'pushal', 'popfw', 'pushfw', 'retfw', 'iretw', 'jcxz', 'lahf_'}
HEX = re.compile(r'0x[0-9a-f]{6,7}\b')
TABLE = re.compile(r'^dword ptr \[(?:\w+ \+ )?\w+\*4 \+ (0x[0-9a-f]+)\]$')
CHUNK = 2048
STRICT = False   # --strict: only starts that are aligned or after padding/ret


class Discovery:
    def __init__(self, path):
        self.pe = pe = PE(path)
        self.img = img = rtti.Image(pe)
        self.lo, hi_v, self.raw, self.rs = img.secs['.text']
        self.hi = self.lo + self.rs
        self.text = pe.data[self.raw:self.raw + self.rs]
        # 0 unknown, 1 start of instruction, 2 inside an instruction, 3 data (jump table)
        self.own = bytearray(self.rs)
        self.md = Cs(CS_ARCH_X86, CS_MODE_32)
        self.starts = {}            # address -> origin
        self.call_targets = []      # to be explored
        self.imms = set()           # immediates that fall in .text
        self.table_slots = set()    # addresses of the jump table entries (anywhere)
        self.tables = 0
        self.n_insn = 0
        self.rejected = Counter()

    def in_text(self, a):
        return self.lo <= a < self.hi

    def read_table(self, t):
        """Entries of a jump table at `t`: consecutive pointers into .text."""
        out = []
        while len(out) < 1024:
            a = t + 4 * len(out)
            v = self.img.u32(a)
            if v is None or not self.in_text(v):
                break
            if self.in_text(a) and self.own[a - self.lo] in (1, 2):
                break
            out.append(v)
        return out

    def explore(self, start, tentative):
        """Decodes the code reachable from `start`. Returns None if it must be discarded."""
        own, lo, text = self.own, self.lo, self.text
        journal = []                # (offset, length) written into own, to undo
        calls, imms, slots = [], [], []
        ntab = 0
        work = [start]
        fail = None
        while work and not fail:
            a = work.pop()
            if not self.in_text(a):
                fail = 'jump outside .text'
                break
            o = a - lo
            if own[o] == 1:
                continue
            if own[o]:
                fail = 'inside an instruction or a table'
                break
            ended = False
            nxt = a
            for addr, size, mn, ops in self.md.disasm_lite(text[o:o + CHUNK], a):
                p = addr - lo
                if own[p] == 1:
                    ended = True
                    break
                if own[p] or any(own[p + 1:p + size]):
                    fail = 'overlap'
                    break
                if mn in NEVER or (mn == 'int3' and addr == start):
                    fail = f'unlikely instruction ({mn})'
                    break
                own[p] = 1
                for k in range(1, size):
                    own[p + k] = 2
                journal.append((p, size))
                nxt = addr + size
                c = mn[0]
                if c == 'j':
                    if ops.startswith('0x'):
                        work.append(int(ops, 16))
                        if mn == 'jmp':
                            ended = True
                            break
                    elif mn == 'jmp':
                        m = TABLE.match(ops)
                        if m:
                            t = int(m.group(1), 16)
                            entries = self.read_table(t)
                            if entries:
                                ntab += 1
                                work.extend(entries)
                                slots.extend(t + 4 * i for i in range(len(entries)))
                                if self.in_text(t):
                                    q = t - lo
                                    n = 4 * len(entries)
                                    if any(own[q:q + n]):
                                        fail = 'table over code'
                                        break
                                    own[q:q + n] = b'\x03' * n
                                    journal.append((q, n))
                        ended = True
                        break
                    else:
                        fail = f'odd jump ({mn} {ops})'
                        break
                elif mn == 'call':
                    if ops.startswith('0x'):
                        t = int(ops, 16)
                        if not self.in_text(t):
                            fail = 'call outside .text'
                            break
                        calls.append(t)
                elif mn == 'ret' or mn == 'int3':
                    ended = True
                    break
                elif mn.startswith('loop'):
                    work.append(int(ops, 16))
                elif '0x' in ops:
                    for h in HEX.findall(ops):
                        v = int(h, 16)
                        if lo <= v < self.hi:
                            imms.append(v)
            if fail:
                break
            if not ended:
                if nxt - a < CHUNK - 16:
                    fail = 'undecodable bytes'
                else:
                    work.append(nxt)
        if fail and tentative:
            for p, n in journal:
                own[p:p + n] = bytes(n)
            self.rejected[fail.split(' (')[0]] += 1
            return None
        if fail:
            self.rejected['[strong] ' + fail.split(' (')[0]] += 1
        self.n_insn += sum(1 for p, n in journal if own[p] == 1)
        self.tables += ntab
        self.table_slots.update(slots)
        self.imms.update(imms)
        return calls

    def add(self, addr, origin, tentative):
        """Tries to add a function and, in a chain, the ones it calls."""
        if addr in self.starts:
            return True
        calls = self.explore(addr, tentative)
        if calls is None:
            return False
        self.starts[addr] = origin
        pending = calls
        while pending:
            t = pending.pop()
            if t in self.starts:
                continue
            more = self.explore(t, tentative)
            if more is None:
                continue
            self.starts[t] = origin + ' → call' if not origin.endswith('call') else origin
            pending.extend(more)
        return True

    def looks_like_start(self, a):
        o = a - self.lo
        if o <= 3:
            return True
        t = self.text
        if STRICT:
            return a % 16 == 0 or t[o - 1] in (0xCC, 0xC3, 0x90) or t[o - 3] == 0xC2
        # aligned, or right after padding, `ret`, `ret n`, `jmp rel32`, `jmp rel8`
        return (a % 16 == 0 or t[o - 1] in (0xCC, 0xC3, 0x90) or t[o - 3] == 0xC2
                or t[o - 5] == 0xE9 or t[o - 2] == 0xEB)

    def weak(self, addr, origin, need_look):
        if addr in self.starts or not self.in_text(addr):
            return
        st = self.own[addr - self.lo]
        if st in (2, 3):
            self.rejected['inside an instruction or a table'] += 1
            return
        if need_look and not self.looks_like_start(addr):
            self.rejected['does not look like a start'] += 1
            return
        if st == 1:
            # code already reached (tail call, fall-through): it is an entry point all the same
            self.starts[addr] = origin
            return
        self.add(addr, origin, tentative=True)


def run(path):
    t0 = time.time()
    d = Discovery(path)
    pe, img = d.pe, d.img

    def log(msg):
        print(f'[{time.time() - t0:6.1f}s] {msg}  functions={len(d.starts)} instructions={d.n_insn}',
              file=sys.stderr)

    # 1. strong
    d.add(pe.image_base + pe.entry, 'entry', False)
    exp_rva = pe.data_dirs[0][0]
    if exp_rva:
        o = pe.rva_to_off(exp_rva)
        nfunc, _, afunc = struct.unpack_from('<III', pe.data, o + 20)
        for i in range(nfunc):
            rva = struct.unpack_from('<I', pe.data, pe.rva_to_off(afunc) + 4 * i)[0]
            if rva:
                d.add(pe.image_base + rva, 'export', False)
    tls_rva = pe.data_dirs[9][0]
    if tls_rva:
        cb = struct.unpack_from('<I', pe.data, pe.rva_to_off(tls_rva) + 12)[0]
        while cb and (f := img.u32(cb)):
            d.add(f, 'TLS callback', False)
            cb += 4
    log('strong starting points')

    # 2. vtable
    r = rtti.analyze(path)
    vt_slots = set()
    for v in r['vtables']:
        for s in v['slots']:
            vt_slots.add(s)
            d.weak(s, 'vtable', need_look=False)
    log('vtable slots')

    # 3-4. immediates and words in the data, until stable
    data_words = []
    for sec in img.secs:
        if sec in ('.text', '.rsrc', '.idata'):
            continue
        base, vals = img.words(sec)
        data_words += [(base + 4 * i, v) for i, v in enumerate(vals) if d.lo <= v < d.hi]
    seen_imm = set()
    for rnd in range(1, 8):
        before = len(d.starts)
        for v in sorted(d.imms - seen_imm):
            seen_imm.add(v)
            d.weak(v, 'immediate', need_look=True)
        for a, v in data_words:
            if a not in d.table_slots:
                d.weak(v, 'data word', need_look=True)
        log(f'round {rnd}: immediates and data')
        if len(d.starts) == before:
            break

    own = d.own
    stats = {
        'functions': len(d.starts),
        'instructions': d.n_insn,
        'by origin': dict(Counter(o for o in d.starts.values()).most_common()),
        'jump tables': d.tables,
        '.text bytes': d.rs,
        'code bytes': sum(1 for b in own if b in (1, 2)),
        'jump table bytes in .text': own.count(3),
        'unreached int3 bytes': sum(1 for i, b in enumerate(own) if b == 0 and d.text[i] == 0xCC),
        'unknown bytes (not int3)': sum(1 for i, b in enumerate(own) if b == 0 and d.text[i] != 0xCC),
        'rejected': dict(d.rejected.most_common()),
        'vtable slots not accepted': len([s for s in vt_slots if s not in d.starts]),
        'seconds': round(time.time() - t0, 1),
    }
    return d, stats


def main(argv):
    global STRICT
    if len(argv) < 2:
        sys.exit(__doc__)
    STRICT = '--strict' in argv
    d, stats = run(argv[1])
    print(json.dumps(stats, indent=1, ensure_ascii=False))
    if '--out' in argv:
        out = argv[argv.index('--out') + 1]
        with open(out, 'w') as f:
            json.dump({'stats': stats,
                       'functions': [[a, o] for a, o in sorted(d.starts.items())]}, f)
        print(f'list written to {out}', file=sys.stderr)


if __name__ == '__main__':
    main(sys.argv)
