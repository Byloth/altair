#!/usr/bin/env python3
"""RTTI di MSVC (32 bit) e vtable di un eseguibile PE32.

Uso:
    python3 -I tools/rtti.py <exe>                  statistiche
    python3 -I tools/rtti.py <exe> --out FILE.json  anche il dettaglio completo

Il JSON contiene nomi di classi del gioco: va salvato in private/.

Strutture lette (tutti i puntatori sono indirizzi assoluti a 32 bit):
    TypeDescriptor            { pVFTable(type_info), spare, name[] ".?AV...@@" }
    CompleteObjectLocator     { signature=0, offset, cdOffset, pTypeDescriptor, pClassHierarchy }
    ClassHierarchyDescriptor  { signature, attributes, numBaseClasses, pBaseClassArray }
    BaseClassDescriptor       { pTypeDescriptor, numContainedBases, mdisp, pdisp, vdisp, attributes }
    vtable                    [-1] = puntatore al CompleteObjectLocator, poi gli slot
"""
import json
import os
import struct
import sys
from collections import Counter

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pe import PE  # noqa: E402


def undecorate(name):
    """'.?AVFoo@bar@@' -> 'bar::Foo'. I template restano decorati."""
    if len(name) < 6 or not name.startswith('.?A') or not name.endswith('@@'):
        return name
    body = name[4:-2]
    if '?' in body:
        return name
    return '::'.join(reversed(body.split('@')))


class Image:
    def __init__(self, pe):
        self.pe = pe
        self.base = pe.image_base
        self.secs = {n: (self.base + va, self.base + va + max(vs, rs), raw, rs)
                     for n, va, vs, raw, rs in pe.sections}

    def sec_of(self, addr):
        for n, (lo, hi, _, _) in self.secs.items():
            if lo <= addr < hi:
                return n
        return None

    def off(self, addr):
        for lo, hi, raw, rs in self.secs.values():
            if lo <= addr < lo + rs:
                return addr - lo + raw
        return None

    def u32(self, addr):
        o = self.off(addr)
        return None if o is None else struct.unpack_from('<I', self.pe.data, o)[0]

    def words(self, sec):
        """(indirizzo, valore) per ogni dword allineata della sezione."""
        lo, _, raw, rs = self.secs[sec]
        vals = struct.unpack_from(f'<{rs // 4}I', self.pe.data, raw)
        return lo, vals


def analyze(path):
    img = Image(PE(path))
    d = img.pe.data
    text_lo, text_hi = img.secs['.text'][:2]
    data_secs = [n for n in img.secs if n not in ('.text', '.rsrc', '.idata')]

    # 1. TypeDescriptor: stringhe ".?A" precedute da {puntatore a type_info::vftable, 0}
    cands = []
    for sec in data_secs:
        lo, _, raw, rs = img.secs[sec]
        blob = d[raw:raw + rs]
        i = blob.find(b'.?A')
        while i != -1:
            if i >= 8:
                vft, spare = struct.unpack_from('<II', blob, i - 8)
                end = blob.find(b'\0', i)
                cands.append((lo + i - 8, vft, spare, blob[i:end].decode('latin-1')))
            i = blob.find(b'.?A', i + 1)
    type_info_vft = Counter(c[1] for c in cands).most_common(1)[0][0]
    tds = {addr: name for addr, vft, spare, name in cands if vft == type_info_vft}

    # 2. CompleteObjectLocator: signature 0 e puntatore a un TypeDescriptor noto
    cols = {}
    hierarchies = {}
    for sec in data_secs:
        lo, vals = img.words(sec)
        for i in range(len(vals) - 4):
            if vals[i] == 0 and vals[i + 3] in tds and img.sec_of(vals[i + 4]) in data_secs:
                chd = vals[i + 4]
                o = img.off(chd)
                if o is None:
                    continue
                sig, attrs, nbases, parr = struct.unpack_from('<4I', d, o)
                if sig != 0 or not 0 < nbases < 256 or img.off(parr) is None:
                    continue
                bases = []
                for k in range(nbases):
                    bcd = img.u32(parr + 4 * k)
                    bo = img.off(bcd) if bcd else None
                    if bo is None:
                        break
                    ptd, ncont, mdisp, pdisp, vdisp, battr = struct.unpack_from('<IIiiiI', d, bo)
                    if ptd not in tds:
                        break
                    bases.append({'type': tds[ptd], 'mdisp': mdisp, 'pdisp': pdisp, 'vdisp': vdisp})
                else:
                    cols[lo + 4 * i] = {'offset': vals[i + 1], 'cd_offset': vals[i + 2],
                                        'td': vals[i + 3], 'chd': chd}
                    hierarchies[chd] = {'attributes': attrs, 'bases': bases}

    # 3. vtable: una dword che punta a un COL, seguita da puntatori a codice
    vtables = []
    for sec in data_secs:
        lo, vals = img.words(sec)
        ptr_to_col = {i for i, v in enumerate(vals) if v in cols}
        for i in sorted(ptr_to_col):
            slots = []
            j = i + 1
            while j < len(vals) and text_lo <= vals[j] < text_hi and j not in ptr_to_col:
                slots.append(vals[j])
                j += 1
            if slots:
                col = cols[vals[i]]
                vtables.append({'addr': lo + 4 * (i + 1), 'col': vals[i], 'type': tds[col['td']],
                                'offset': col['offset'], 'slots': slots})

    return {'type_info_vftable': type_info_vft, 'tds': tds, 'cols': cols,
            'hierarchies': hierarchies, 'vtables': vtables}


def namespace_of(name):
    u = undecorate(name)
    if u.startswith('.?A'):
        # template: l'ultimo componente prima di '@@' finale è il namespace più esterno
        parts = [p for p in name[4:-2].split('@') if p and '?' not in p and '$' not in p]
        return parts[-1] if len(parts) > 1 else '(globale)'
    return u.split('::')[0] if '::' in u else '(globale)'


def main(argv):
    if len(argv) < 2:
        sys.exit(__doc__)
    r = analyze(argv[1])
    tds, cols, vts = r['tds'], r['cols'], r['vtables']
    slots = [s for v in vts for s in v['slots']]
    kinds = Counter(n[3] for n in tds.values())
    print(f'type_info::vftable       {r["type_info_vftable"]:#x}')
    print(f'TypeDescriptor           {len(tds)}  (nomi distinti: {len(set(tds.values()))})')
    print(f'  per tipo               ' + ', '.join(f'{k}={v}' for k, v in sorted(kinds.items()))
          + '   (V=class U=struct W=enum T=union)')
    print(f'  template               {sum("?$" in n for n in tds.values())}')
    print(f'CompleteObjectLocator    {len(cols)}')
    print(f'ClassHierarchyDescriptor {len(r["hierarchies"])}')
    print(f'vtable                   {len(vts)}')
    print(f'  slot totali            {len(slots)}')
    print(f'  funzioni distinte      {len(set(slots))}')
    print(f'  classi con vtable      {len(set(v["type"] for v in vts))}')
    print(f'  vtable secondarie      {sum(v["offset"] != 0 for v in vts)}  (offset != 0: ereditarietà multipla)')
    sizes = sorted(len(v['slots']) for v in vts)
    print(f'  slot per vtable        min {sizes[0]}, mediana {sizes[len(sizes) // 2]}, max {sizes[-1]}')
    multi = sum(len(h['bases']) > 1 for h in r['hierarchies'].values())
    print(f'classi con almeno una base: {multi}')
    print('namespace principali (per TypeDescriptor):')
    for ns, n in Counter(namespace_of(n) for n in tds.values()).most_common(12):
        print(f'  {n:5}  {ns}')

    if '--out' in argv:
        out = argv[argv.index('--out') + 1]
        doc = {
            'type_info_vftable': r['type_info_vftable'],
            'types': [{'td': a, 'name': n, 'pretty': undecorate(n)} for a, n in sorted(tds.items())],
            'hierarchies': [{'chd': a, **h} for a, h in sorted(r['hierarchies'].items())],
            'vtables': [{**v, 'pretty': undecorate(v['type'])} for v in vts],
        }
        with open(out, 'w') as f:
            json.dump(doc, f, indent=1)
        print(f'\ndettaglio scritto in {out}')


if __name__ == '__main__':
    main(sys.argv)
