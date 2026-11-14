#!/usr/bin/env python3
"""Scoperta delle funzioni di un PE32 senza simboli né rilocazioni (prototipo).

Uso (serve capstone: usare il Python del venv):
    .venv/bin/python tools/functions.py <exe> [--strict] [--out private/functions.json]

Metodo, in ordine di affidabilità delle partenze:
  1. forti: entry point, export, callback TLS e ogni destinazione di `call` diretta nel
     codice raggiunto da queste;
  2. slot delle vtable trovate dall'RTTI;
  3. indirizzi di codice usati come immediati (`push offset f`, `mov [x], offset f`);
  4. parole nelle sezioni dati che puntano nel codice.
Le partenze 2-4 sono "deboli": si accettano solo se la decodifica di prova non incontra
byte non validi, istruzioni che un gioco non esegue mai, o il mezzo di un'istruzione già
nota. Le 3 e 4 devono anche "sembrare un inizio" (allineate a 16 o dopo padding/ret).

Il ricompilatore vero (Fase 1, Rust) rifarà questo lavoro: qui servono i numeri e l'elenco.
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
STRICT = False   # --strict: solo inizi allineati o dopo padding/ret


class Discovery:
    def __init__(self, path):
        self.pe = pe = PE(path)
        self.img = img = rtti.Image(pe)
        self.lo, hi_v, self.raw, self.rs = img.secs['.text']
        self.hi = self.lo + self.rs
        self.text = pe.data[self.raw:self.raw + self.rs]
        # 0 ignoto, 1 inizio di istruzione, 2 interno di istruzione, 3 dati (jump table)
        self.own = bytearray(self.rs)
        self.md = Cs(CS_ARCH_X86, CS_MODE_32)
        self.starts = {}            # indirizzo -> origine
        self.call_targets = []      # da esplorare
        self.imms = set()           # immediati che cadono in .text
        self.table_slots = set()    # indirizzi delle voci di jump table (ovunque)
        self.tables = 0
        self.n_insn = 0
        self.rejected = Counter()

    def in_text(self, a):
        return self.lo <= a < self.hi

    def read_table(self, t):
        """Voci di una jump table a `t`: puntatori consecutivi in .text."""
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
        """Decodifica il codice raggiungibile da `start`. Ritorna None se va scartato."""
        own, lo, text = self.own, self.lo, self.text
        journal = []                # (offset, lunghezza) scritti in own, per annullare
        calls, imms, slots = [], [], []
        ntab = 0
        work = [start]
        fail = None
        while work and not fail:
            a = work.pop()
            if not self.in_text(a):
                fail = 'salto fuori da .text'
                break
            o = a - lo
            if own[o] == 1:
                continue
            if own[o]:
                fail = 'dentro un\'istruzione o una tabella'
                break
            ended = False
            nxt = a
            for addr, size, mn, ops in self.md.disasm_lite(text[o:o + CHUNK], a):
                p = addr - lo
                if own[p] == 1:
                    ended = True
                    break
                if own[p] or any(own[p + 1:p + size]):
                    fail = 'sovrapposizione'
                    break
                if mn in NEVER or (mn == 'int3' and addr == start):
                    fail = f'istruzione improbabile ({mn})'
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
                                        fail = 'tabella sopra codice'
                                        break
                                    own[q:q + n] = b'\x03' * n
                                    journal.append((q, n))
                        ended = True
                        break
                    else:
                        fail = f'salto strano ({mn} {ops})'
                        break
                elif mn == 'call':
                    if ops.startswith('0x'):
                        t = int(ops, 16)
                        if not self.in_text(t):
                            fail = 'call fuori da .text'
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
                    fail = 'byte non decodificabili'
                else:
                    work.append(nxt)
        if fail and tentative:
            for p, n in journal:
                own[p:p + n] = bytes(n)
            self.rejected[fail.split(' (')[0]] += 1
            return None
        if fail:
            self.rejected['[forte] ' + fail.split(' (')[0]] += 1
        self.n_insn += sum(1 for p, n in journal if own[p] == 1)
        self.tables += ntab
        self.table_slots.update(slots)
        self.imms.update(imms)
        return calls

    def add(self, addr, origin, tentative):
        """Prova ad aggiungere una funzione e, a catena, quelle che chiama."""
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
        # allineata, oppure subito dopo padding, `ret`, `ret n`, `jmp rel32`, `jmp rel8`
        return (a % 16 == 0 or t[o - 1] in (0xCC, 0xC3, 0x90) or t[o - 3] == 0xC2
                or t[o - 5] == 0xE9 or t[o - 2] == 0xEB)

    def weak(self, addr, origin, need_look):
        if addr in self.starts or not self.in_text(addr):
            return
        st = self.own[addr - self.lo]
        if st in (2, 3):
            self.rejected['dentro un\'istruzione o una tabella'] += 1
            return
        if need_look and not self.looks_like_start(addr):
            self.rejected['non sembra un inizio'] += 1
            return
        if st == 1:
            # codice già raggiunto (tail call, fall-through): è comunque un punto d'ingresso
            self.starts[addr] = origin
            return
        self.add(addr, origin, tentative=True)


def run(path):
    t0 = time.time()
    d = Discovery(path)
    pe, img = d.pe, d.img

    def log(msg):
        print(f'[{time.time() - t0:6.1f}s] {msg}  funzioni={len(d.starts)} istruzioni={d.n_insn}',
              file=sys.stderr)

    # 1. forti
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
            d.add(f, 'callback TLS', False)
            cb += 4
    log('partenze forti')

    # 2. vtable
    r = rtti.analyze(path)
    vt_slots = set()
    for v in r['vtables']:
        for s in v['slots']:
            vt_slots.add(s)
            d.weak(s, 'vtable', need_look=False)
    log('slot delle vtable')

    # 3-4. immediati e parole nei dati, fino a stabilità
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
            d.weak(v, 'immediato', need_look=True)
        for a, v in data_words:
            if a not in d.table_slots:
                d.weak(v, 'parola nei dati', need_look=True)
        log(f'giro {rnd}: immediati e dati')
        if len(d.starts) == before:
            break

    own = d.own
    stats = {
        'funzioni': len(d.starts),
        'istruzioni': d.n_insn,
        'per origine': dict(Counter(o for o in d.starts.values()).most_common()),
        'jump table': d.tables,
        'byte .text': d.rs,
        'byte di codice': sum(1 for b in own if b in (1, 2)),
        'byte di jump table in .text': own.count(3),
        'byte int3 non raggiunti': sum(1 for i, b in enumerate(own) if b == 0 and d.text[i] == 0xCC),
        'byte ignoti (non int3)': sum(1 for i, b in enumerate(own) if b == 0 and d.text[i] != 0xCC),
        'scarti': dict(d.rejected.most_common()),
        'slot di vtable non accettati': len([s for s in vt_slots if s not in d.starts]),
        'secondi': round(time.time() - t0, 1),
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
        print(f'elenco scritto in {out}', file=sys.stderr)


if __name__ == '__main__':
    main(sys.argv)
