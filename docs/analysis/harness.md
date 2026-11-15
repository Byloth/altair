# Unicorn harness

`tools/harness.py` runs functions of the **original** executable in an x86 emulator (Unicorn).
It is the oracle of the differential tests: when the recompiler translates a function, the same
call with the same inputs must give the same results here and in the translated code.

Verified on 2026-11-14 on the Steam DX9 build with Unicorn 2.1.4.

## Usage

```sh
.venv/bin/python tools/harness.py "$AC1/AssassinsCreed_Dx9.exe" selftest
.venv/bin/python tools/harness.py "$AC1/AssassinsCreed_Dx9.exe" call 0xFA6790 s:Altair
#   eax=0x00000006 edx=... instructions=30 imports=[]
```

Options of `call`: `--ecx V` (the `this` of thiscall functions), `--trace` (prints every
address executed). The arguments are integers or `s:text`, copied into guest memory and
passed as a pointer.

From Python:

```python
from harness import Guest
g = Guest(exe)
p = g.alloc_str(b'Altair')
r = g.call(0x00FA6790, [p])      # r.eax, r.edx, r.st0, r.popped, r.insns, r.imports
g.reset()                        # image and test heap go back to the initial state
```

`r.popped` is the number of bytes of arguments removed from the stack by the callee: 0 for cdecl,
greater than 0 for stdcall and thiscall. It is used to find out the calling convention.

## How it is built

| Address | What |
|---|---|
| `0x00400000` | the PE image, sections at their addresses (no relocations needed) |
| `0x10000000` | test heap for the arguments, 1 MB, zeroed in full at every `reset()` |
| `0x6FF00000`–`0x70000000` | stack, 1 MB |
| `0x7F000000` | import trampolines, 16 bytes each |
| `0x7FFD0000` | TEB: `fs:[0]` end of the SEH chain, `fs:[0x18]` pointer to itself, `fs:[0x2C]` TLS array |
| `0x7FFE0000` | GDT |
| `0x7FFF0000` | sentinel return address: getting there means "the function has returned" |

- **`fs:`** In 32-bit mode Unicorn does not allow writing the base of `fs` directly:
  a GDT is needed with a descriptor whose base is the TEB, and `fs` loaded with that selector.
- **Imports.** Every IAT cell points to a trampoline. When execution gets there, a hook
  looks in `Guest.handlers` for a Python function with that name, which receives the arguments and
  returns `(eax, bytes to remove from the stack)`. If there is none, the call stops with
  `unhandled import: kernel32.dll!TlsGetValue (called from 0x00fa8e1f)`. All the calls
  to imports are recorded in `r.imports`: in the differential tests those must be compared
  too.
- **Instruction limit** (5 million by default) against infinite loops.

## Automatic checks (`selftest`)

Three assembly routines of the MSVC 8 CRT, found by searching `.text` for their initial byte
sequences and compared with the result computed in Python:

| Function | Address | Check |
|---|---|---|
| `strlen` | `0x00FA6790` | 9 lengths × 4 alignments |
| `strcmp` | `0x00FA9760` | 9 pairs, sign of the result |
| `memcpy` | `0x00F9FB80` | 49 lengths × 4 alignments of the source × 4 of the destination: it walks the `memcpy` jump tables, one of the delicate points for the recompiler |

All pass, in 0.4 seconds. Two more control calls:

- `0x007A56F0` (an engine function with an SEH prologue, which reads and writes `fs:[0]`) returns
  correctly: the TEB works.
- `_purecall` (`0x00F9F7A2`) stops on `TlsGetValue`, as expected: it calls a function at
  `0x00FA8E10` (probably the CRT's `_decode_pointer`) which uses TLS.

## State restored at every call

Learned the hard way from the differential test (2026-11-15): the emulator keeps state between one
call and the next, and a function interrupted halfway leaves it dirty.

- **EFLAGS** is rewritten to `0x202`: otherwise the direction flag stays set.
- **MXCSR** is reset by *executing* an `ldmxcsr` in a small service page: writing the
  register from outside does not update the internal rounding, flush-to-zero and denormals.
- `reset()` restores the writable sections, the whole test heap and 64 KB of stack.
- `.text`, `.rdata` and `.rsrc` are write-protected: a wrong pointer stops the case.
- The initial XMM registers are passed with `call(..., xmm=[...])`.

## Limits and next steps

- **No initialised global state.** The image is the one on disk: the static
  initialisers of the CRT and of the engine have not been run. Pure functions are fine; for the
  others it is first necessary to run the startup (Phase 2) or prepare the state by hand.
- **No default import handlers.** They are added as needed (`g.handlers['TlsGetValue'] = …`).
- **A single thread.**
- The real differential tests still need: input generation, comparison of
  registers, x87 state and written memory, and a selection of "leaf" functions (without imports,
  without global state). It is Phase 1 work, together with the recompiler.
- Unicorn (QEMU) differs from real hardware in two points known to scimitar-recomp: `fisttp`
  overflow and conversions of signalling NaNs. `rcpps`/`rsqrtps` are exact in Unicorn and
  approximate on real CPUs.

## Note: the CRC-32 table

In `.data` at `0x01915AC0` there is the standard CRC-32 table (polynomial `0xEDB88320`), but no
instruction and no word in the data references it directly. The engine identifies the classes
in the archives by the CRC-32 of the name, so a function that uses it must exist: it has to be looked for
(access through a computed pointer, or a second table generated at runtime). It would be the
first "engine" check to add to the selftest: `CRC32("ShaderPermutation")` must give
`0x9467F2BB`.
