# First list of functions

Verified on 2026-11-14 with `tools/functions.py` (Python prototype with Capstone) on
`AssassinsCreed_Dx9.exe`. The complete list, with the origin of every start, is in
`private/functions-dx9.json` (30 seconds to regenerate it):

```sh
.venv/bin/python tools/functions.py "$AC1/AssassinsCreed_Dx9.exe" --out private/functions-dx9.json
```

It is a working estimate, not the definitive list: that will be produced by the Rust recompiler
(Phase 1), which will also have to resolve the limits listed at the bottom.

## Result

| | Loose rule (default) | `--strict` | scimitar-recomp |
|---|---|---|---|
| Functions | **115,807** | 96,781 | 98,470 |
| Instructions reached (distinct) | **3,191,741** | 3,121,093 | 3,484,935 |
| Jump tables | 570 | 567 | 728 (count from linear sweep) |

The numbers are not comparable one to one with those of scimitar-recomp: they count
instructions per function (code shared between two functions counts twice), we count distinct
instructions. The order of magnitude matches.

The two rules differ on when a word in the data, or an immediate, "looks like a beginning":

- `--strict`: address aligned to 16, or right after padding (`int3`, `nop`) or `ret`;
- loose: also right after a `jmp`. It adds 19,000 starts but only 70,000 instructions: they are
  almost all small pieces of exception handling (thunks `mov eax, table; jmp
  __CxxFrameHandler`, unwind funclets and `catch` blocks), reached only by the exception
  runtime. They are real entry points, but are needed only if the game throws exceptions.

## Where the starts come from (loose rule)

| Origin | Functions |
|---|---|
| Entry point and what it calls, directly or in a chain | 7,116 |
| Exports (DARE audio engine) and what they call | 100 |
| Vtable slots | 28,304 |
| Direct calls from the vtable functions | 23,400 |
| Words in the data | 36,254 (+3,401 called from these) |
| Immediates in the code (`push offset f`) | 16,087 (+1,145) |

**The most important fact: from the entry point, following only direct calls, one reaches
7,216 functions out of more than 100,000.** All the rest of the game is reached through vtables and
function pointers. One cannot translate "only what is needed" starting from `main`: the
whole image must be translated, as scimitar-recomp also concluded.

After entry, exports and vtables (the most reliable sources) there are already 58,920 functions and 2.5 million
instructions: 78% of the code reached.

## What `.text` is made of (19.4 MB)

| Content | Bytes | Share |
|---|---|---|
| Code reached | 10,197,343 | 52.6% |
| Jump tables inside `.text` | 20,024 | 0.1% |
| `int3` not reached (padding and the 3.5 MB dead block) | 5,440,558 | 28.1% |
| Unclassified | 3,720,251 | 19.2% |

The count of `int3` (5.44 million) matches that of scimitar-recomp.

**The 3.7 MB unclassified are largely real code that nobody references.** They are
27,000 blocks, 22,000 of which start at an address aligned to 16 after padding and
decode as normal functions (prologues `push esi; mov esi, ecx`…). Neither a call, nor
a vtable, nor a word in the data points to them: dead code that the linker did not discard.
This agrees with scimitar-recomp's figures: about 4.4 million real instructions in `.text`, of which
3.5 reached. They need not be translated, but must be kept an eye on: if at runtime an indirect call
points in there, it is a function that discovery missed.

## Checks

- All the addresses known from EaglePatch are in the list: `PadProxyPC::Update` (vtable),
  `PadProxyPC::AddPad`, the allocator and `getNewDescriptor` (from the entry), the constructor of
  `PadXenon` and the engine's `delete` (from immediates), `Pad::UpdateTimeStamps` (from a vtable).
- Only **13 vtable slots out of 28,717** do not decode as a function: the heuristic with which
  `rtti.py` counts the slots is reliable.

## Rejections and anomalies

| Reason for rejecting a weak start | How many |
|---|---|
| Falls inside an already known instruction or table | 41,253 |
| Does not look like a beginning | 36,496 |
| Decodes into instructions that a game does not execute (`in`, `out`, `les`, `retf`…) | 12,529 |
| Jump table overlapping code | 534 |
| Overlap with other instructions | 184 |
| Undecodable bytes | 44 |
| Jump or call outside `.text` | 20 |

This is the effect of the absence of relocations: tens of thousands of words in the data fall by chance
within the code range and must be filtered out.

Anomalies in the **strong** starts (code that is certainly real), to be resolved in Phase 1:

- 10 jump tables that overlap already decoded code and 2 overlaps: the
  "greedy" reading of the tables goes past the end, or decoding went past a call that does not
  return;
- 3 improbable instructions and 1 landing inside an instruction: same probable cause, the
  code after a `call` to a function that does not return (`exit`, `_amsg_exit`) is not code.

## Limits of the prototype

1. **Jump tables**: it recognises only `jmp [reg*4 + T]` and reads entries as long as they are code
   addresses, without using the bound of the preceding `cmp`. The two-level forms with an
   explicit bound and the negative indices of `memcpy` are missing.
2. **Functions that do not return**: they are not recognised, so what follows the call
   gets decoded too.
3. **No function boundaries**: we know where a function starts, not which instructions
   belong to it. The recompiler will have to compute this (shared code, tail calls).
4. **Entry points in already reached code** counted as functions: some of them are internal
   labels (`catch` blocks), not callable functions.
5. It does not use the C++ exception tables (`FuncInfo`, unwind map), which would give funclets and
   handlers with certainty rather than heuristically.

## Update of 2026-11-15: discovery in Rust

The numbers above are from the Python prototype. `altair discover` today gives **116,077 functions and
3,221,462 instructions**, with 641 jump tables. The discarded starts now also include the
index tables of two-level switches, which decoded as `push es`/`pop es`:
see `translator.md`, "The last 42 functions". The runtime then revealed an unaligned
pointer table in `.data` (CRT math routines): with those the functions are 116,142.
Running the game revealed more cases (unaligned runs, `ret`-only callbacks, false
starts from immediates, `pushad`): after that the functions were 117,159.
With discovery in three passes (certain boundaries, then aligned addresses, then tables) they are
**117,106**, 56 of which added by `altair image` because they are called but not discovered (`runtime.md`).
