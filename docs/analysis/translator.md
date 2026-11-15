# The recompiler (`altair`)

Status as of 2026-11-14, first day of Phase 1 (updated after x87). Code in `crates/` (Rust) and `runtime/` (C).

```sh
source ~/.bash_exports && cargo build --release
A=target/release/altair; E="$AC1/AssassinsCreed_Dx9.exe"
$A info $E                         # hash, sections, imports
$A rtti $E                         # RTTI and vtable counts
$A discover $E [--strict] [--out private/functions.json]
$A translate $E --out build/gen/x.c 0xFA6790 ...   # translates the functions, and what they call, to C
$A coverage $E [--list]            # how many functions translate, and what blocks the others
$A census $E                       # every instruction: how many times it appears, how many are untranslatable
$A candidates $E --max 500         # functions suitable for the differential test
.venv/bin/python tools/difftest.py $E              # hand-written scenarios
.venv/bin/python tools/difftest.py $E --auto 8000  # automatic sample, about 3 minutes
.venv/bin/python tools/difftrace.py $E 0xADDR 0    # replays a case and shows the differences
```

`altair` rejects any executable that does not have the SHA-1 of the Steam DX9 build
(`--any-exe` to force it): this is project rule 4, enforced in the code.

## Structure

| Path | What |
|---|---|
| `crates/pe` | PE32 parser: sections, imports with IAT cells, exports, TLS callbacks, SHA-1 |
| `crates/recomp/src/rtti.rs` | RTTI and vtables (port of `tools/rtti.py`) |
| `crates/recomp/src/discover.rs` | Function discovery (port of `tools/functions.py`) |
| `crates/recomp/src/translate.rs` | Decoding of a function and translation to C |
| `runtime/altair.h`, `altair.c` | Support for the generated code: context, guest memory, flags, function registry, traps |
| `runtime/altair_sse.h` | SSE/SSE2 operations in portable C |
| `runtime/altair_x87.h` | x87 on 80-bit `long double` |
| `build/gen/full/` (generated) | The whole translated image: see `image.md` |
| `tests/compiler/` | Reproductions of C compiler bugs found by the test |
| `tools/difftest.py` | Differential test against `tools/harness.py` |

## Function discovery in Rust

Same RTTI counts as the Python scripts (4,376 / 4,597 / 59,899 / 28,717). Discovery takes
0.35 seconds instead of 30 and gives **116,090 functions, 3,222,078 instructions** (97,045 with `--strict`).
Compared with the prototype:

- the number of entries of a jump table comes from the `cmp x, N; ja` that precedes it, including
  two-level tables (byte table plus address table): 497 tables out of 641;
- the anomalies in the certain code drop from 16 to 7.

Limits 2–5 of `functions.md` remain (functions that do not return, boundaries, exception
tables).

## What the generated code looks like

Every guest function becomes `void f_XXXXXXXX(Ctx *c)`.

- **Registers** in the context (`c->eax`…); the 8- and 16-bit registers are masks over the 32-bit ones.
- **Guest memory**: `rd8/16/32(c, address)` and `wr…`, that is `c->mem + address`. The runtime
  reserves 4 GB of virtual space, so every guest address is valid without checks.
  `fs:` adds `c->fs_base` (the guest address of the TEB).
- **Flags**: six local variables (`cf zf sf of pf af`) computed after every instruction that
  writes them. The C compiler removes the ones nobody reads. Flags do not carry over from one
  function to the next.
- **Internal jumps**: `goto`. **Jump tables**: a `switch` on the destination addresses, which
  the code computes by reading the table from guest memory as the original does.
- **Calls**: a `call` writes the real return address on the guest stack and calls the
  C function; `ret` removes it. The guest stack thus stays byte-for-byte identical to the original.
  A jump to another function is a tail call (`f_x(c); return;`).
- **Indirect calls** and functions not present in the file: `altair_call_indirect(c, address)`,
  which looks in the registry of translated functions and otherwise falls through to `c->host_call` (imports and
  host functions). If nobody handles the address a trap fires.
- **Traps** (`altair_trap`): `int3`, invalid divisions, unknown destinations. They return
  control to `altair_run` with reason and address, without aborting the process.
- Every translated instruction carries the address and text of the original as a comment.

Rules for decoding a function:

- a jump to a **certain** start (entry, export, vtable, `call` destination) is a
  tail call; weak starts remain internal code, at most duplicated in two functions;
- if the body contains code at addresses lower than the entry, the function begins with a
  `goto` to the entry (pitfall no. 1 of scimitar-recomp);
- jump tables: simple form, two-level, indexed from 1, with negative indices (after `neg`)
  and with a shifted base. The last three are those of MSVC's `memcpy`.

## Supported instructions

**Integer:** `mov movzx movsx lea push pop leave xchg`, `add adc sub sbb cmp and or xor test inc
dec neg not`, `shl shr sar rol ror rcl rcr shld shrd`, `imul` (1, 2, 3 operands), `mul div idiv`
(8, 16, 32 bit), `cdq cwde cbw cwd bswap`, `bt bts btr btc bsf bsr`, all the `jcc`, `jecxz`,
`loop`, `setcc`, `cmovcc`, direct and indirect `jmp`/`call`, `ret`/`ret n`, `movs stos lods scas
cmps` with `rep`/`repe`/`repne` and direction flag, `cld std clc stc cmc`, `pushfd popfd sahf
lahf`, `xadd cmpxchg`, `nop pause wait prefetch*`.

**Atomic:** `lock` prefix on `add sub and or xor inc dec not xadd cmpxchg bts btr btc`, and
`xchg` with a memory operand. They become true atomic operations on host memory
(`__atomic_*`, sequential ordering).

**SSE and SSE2** (XMM registers; `runtime/altair_sse.h`):
- moves: `movaps movups movapd movupd movdqa movdqu movss movsd movd movq movlps movlpd
  movhps movhpd movhlps movlhps movmskps movmskpd pmovmskb`;
- packed and scalar arithmetic: `add sub mul div min max sqrt` in the `ps pd ss sd` versions,
  `rsqrtps rsqrtss rcpps rcpss`;
- logic: `and andn or xor` in the `ps`, `pd` and integer (`pand pandn por pxor`) versions;
- comparisons: `cmpps cmppd cmpss cmpsd` (eight predicates), `comiss ucomiss comisd ucomisd`;
- conversions: `cvtss2sd cvtsd2ss cvtps2pd cvtpd2ps cvtsi2ss cvtsi2sd cvttss2si cvttsd2si
  cvtss2si cvtsd2si cvtdq2ps cvtps2dq cvttps2dq`;
- shuffles: `shufps shufpd pshufd unpcklps unpckhps unpcklpd unpckhpd punpcklbw punpcklwd
  punpckldq punpckhdq punpcklqdq punpckhqdq pextrw pinsrw`;
- packed integers: `padd psub` (`b w d q`), `pcmpeq` (`b w d`), `pcmpgt` (`w d`), `pmullw
  pmulhw pmuludq`, shifts `psll psrl psra` (`w d q`), `pslldq psrldq`;
- control: `ldmxcsr stmxcsr`.

The SSE operations are written in portable C, lane by lane. On an x86-64 host the
compiler turns them into the same instructions as the original; rounding, flush-to-zero and
denormals follow the guest because during execution the host's MXCSR is the guest's.
`rsqrt` and `rcp` are computed exactly, as in the reference emulator (real CPUs give
12 bits).

**x87** (`runtime/altair_x87.h`): stack of eight 80-bit registers.
- loads and stores: `fld fst fstp` (32, 64, 80 bit and registers), `fild fist fistp fisttp`
  (16, 32, 64 bit), `fxch`, `fcmovcc`, constants `fld1 fldz fldpi fldl2t fldl2e fldlg2 fldln2`;
- arithmetic: `fadd fsub fsubr fmul fdiv fdivr` with the `p` (pop) and `fi` (integer) variants,
  `fchs fabs fsqrt frndint fscale fprem fprem1`;
- comparisons: `fcom fcomp fcompp fucom fucomp fucompp ficom ficomp ftst fxam` (condition
  codes in the status word) and `fcomi fcomip fucomi fucomip` (CPU flags);
- transcendentals: `fsin fcos fsincos fptan fpatan fyl2x fyl2xp1 f2xm1`, through libm;
- control: `fnstsw fnstcw fldcw fninit fnclex ffree fincstp fdecstp`.

The registers are `long double`, which on x86 hosts is exactly the x87 80-bit format: the
compiler emits the same instructions as the original. Precision (24, 53 or 64 bit) and
rounding follow the guest because the host's x87 control word, during
guest execution, is the guest's. The game starts from the Windows control word,
`0x027F` (53 bit). Register tags and the exception flags in the status word
are not modelled.

**Non-deterministic:** `rdtsc` is a 1 GHz counter from host time; `cpuid` describes a fixed
SSE3-class CPU. The functions that use them are excluded from the differential test, as are
those with transcendental x87 instructions (the emulator computes them in double precision).

## Coverage (`altair coverage`, `altair census`)

| | Integer only | With atomics | With SSE | **With x87** |
|---|---|---|---|---|
| Functions that translate in full | 85.9% | 89.5% | 94.3% | 116,048 out of 116,090 (99.96%), then **all** (see below) |
| Instructions in those functions, counted per function | 59.5% | 65.1% | 74.5% | **99.9%** |
| Distinct untranslatable instructions | — | 10.9% | 2.9% | **336 out of 3,223,348** |

"Translates" means that the translator emits C for every instruction, **not** that the result
is verified: for that there is the differential test below.

### The last 42 functions (2026-11-15)

After x87, 42 untranslatable functions remained. `altair coverage --list` lists them with origin
and reason. They were of two kinds.

**17 were not functions.** They were data mistaken for code by discovery:
- 14 index tables of two-level switches. Their bytes (`00 01 02 03 04 05 06 07…`)
  decode as `add`, `push es`, `pop es`, `pop ds`. They sit right after the address
  table, at an aligned address, and the code loads their address as an immediate;
- 3 "round" constants in the data (`0xD20000`, `0x1640000`, `0x113F9F0`).

Discovery now discards starts that contain a `push`/`pop` of a segment register,
`enter`, `clts`, `loope`/`loopne`. The functions drop from 116,090 to **116,077**.

**25 were real code**, now translated:

| Functions | What | Where |
|---|---|---|
| 13 | MMX: `movq movd emms pshufw punpck* pcmpgtw pmulhw padd* psub* pack* psll* psra* cvttps2pi` | audio mixer (`0x8146F0`…, `0x833DF0`…) and two at `0x1023410` |
| 3 | `mov [mem], ss` and other segment registers | the CRT capturing the context of an exception |
| 2 | `xlatb` | CRT |
| 2 | `fnstenv`, `fnsave` (with `fldenv`, `frstor`) | CRT, math error handling |
| 5 | 16-bit `bsr`, `bts` on a bit string, `cvtdq2pd`, `movntps` | various |

The MM registers are the mantissas of the physical x87 registers, as on hardware; the operations
reuse the lane-based SSE ones. The segment registers return the selectors of a 32-bit
process on 64-bit Windows (`cs` 0x23, `ds`/`es`/`ss` 0x2B, `fs` 0x53).

**Now `altair coverage` gives 116,077 functions out of 116,077 and `altair census` zero
untranslatable instructions.**

Verification of those 25 (`difftest.py --funcs …`, 40 cases each): 17 identical, 9 of them
up to the exponent of the MMX registers (see below); 7 skipped because they do not run with random
inputs (the three for the exception context, the two with `xlatb`, the one with 16-bit `bsr`, one
MMX); 1 different, `0xFB3476`, only in the x87 environment fields written by `fnstenv` that we do not
model (tags, pointers to the last instruction, reserved bits).

## Differential test (`tools/difftest.py`)

It translates the functions, compiles them with clang into a library, then runs every case twice with
the same inputs: on the original in Unicorn and on the translated code, in identical guest
memories. It compares the seven general registers, the eight XMM registers, the x87 state (top
of stack and the eight physical 80-bit registers), the bytes removed from the stack, the imports
called and the final contents of the test heap, the stack and the writable sections of the image.

### Hand-written scenarios

| Scenario | Calls | Outcome |
|---|---|---|
| `strlen` (`0xFA6790`), random contents and alignments | 300 | identical |
| `strcmp` (`0xFA9760`), equal, different, truncated strings | 300 | identical |
| `memcpy` (`0xF9FB80`), every length and alignment, plus overlapping regions (backward copy with `std`) | 1,117 | identical |
| `floor` (`0xFA1120`), x87 branch: half-integers, signed zeros, infinities, random values | 316 | identical (before the fix: different at the first call) |
| `cpuid` (`0xFCA495`), the CRT looks for SSE2 by way of the ID bit of EFLAGS | 1 | same verdict |

### Automatic sample (`--auto N`)

`altair candidates` picks the suitable functions: the whole closure over direct calls is
translatable, small (at most 12 functions and 3,000 instructions), without imports, without indirect calls or
jumps to an unknown destination, without non-comparable instructions. There are 19,028 out of
116,048. It first takes a few for every instruction type, starting from the rarest, then the
others in a repeatable pseudo-random order.

For every function, 8 cases with generated inputs: six buffers full of pointers to other buffers,
small integers, floats (including signed zeros, infinities, NaNs, denormals, integer limits) and
random values; arguments, `ecx` and `edx` chosen among pointers and integers; random initial XMM
registers; empty x87 with control word `0x027F`. If the original does not run with those
inputs the case is skipped.

| | |
|---|---|
| Functions tried | 8,000 |
| **Identical to the original** in all the cases run | **4,199** |
| of which identical up to a NaN or the exponent of an MMX register | 9 |
| Different | **0** |
| Skipped (no case runnable with random inputs) | 3,801 |
| Functions run in identical cases, including the called ones | 4,935 |
| Instructions in those functions | 142,138, of 213 types |
| Time | about 3 minutes |

x87 instructions present in the verified functions: `fld` 3,051, `fstp` 2,305, `fmul` 545, `fxch`
285, `fadd` 275, `fnstsw` 250, `fmulp` 195, `fldcw` 176, `fsub` 170, `fst` 136, `faddp` 130,
`fild` 125, `fistp` 109, `fldz` 98, `fnstcw` 90, `fsubp` 86, `fcom` 81, `fcomip` 70, `fcomp` 67,
`fcompp` 60, `fld1` 58, `fdiv` 47, and 20 more with smaller counts. The 176 `fldcw` show that
the rounding change (MSVC's sequence for converting a float to an integer) is exercised.

SSE instructions in the verified functions (previous sample, of 6,000): `movaps` 5,142,
`movss` 4,395, `shufps` 1,208, `mulps` 1,173, `cvtps2pd` 892, `addps` 830, and 60 more.

**Translated but absent from the verified functions:** `fisubr`, `paddq`, `pmuludq`, `paddsw`,
`pshufw`, `punpckhbw`, `cvtpi2ps`, `xlatb`, `fldenv`, `frstor`, `mov` from a segment register, and among
the integer ones `rcl`, `bts`/`btr`/`btc`, `bsf`/`bsr`, `pushfd`/`popfd`, `sahf`/`lahf`, `cmpsb`,
`lodsb`. **Translated and deliberately excluded from the comparison:** the transcendental x87 ones, `fprem`,
`fprem1`. Few occurrences (fewer than 5): `frndint`, `fldpi`, `fucom`, `fiadd`, `fimul`, `fidiv`.

Limits of the sample: appearing in a verified function does not guarantee that the case
went through that instruction; half of the functions are skipped because they need structured
inputs that the generator cannot build; the x87 stack always starts empty.

### What the test found

1. **Three forms of `memcpy` jump table** not recognised (shifted base, index from 1,
   negative index). Fixed in the translator.
2. **A clang 18.1.3 bug.** `x = (x & 0xffffff00) | (u8)mem[x]`, with `x` just loaded
   from memory, is compiled from `-O1` upwards losing the high 24 bits; gcc is correct.
   20-line reproduction: `tests/compiler/clang18-partial-register.c`. The generated code
   now writes the 8- and 16-bit registers with direct narrow writes (`SET8L`, `SET8H`, `SET16`).
   **Consequence: the differential test must be rerun on the code compiled with the final
   compiler and options**, not only on the translator.
3. **NaN.** When the two operands of an SSE operation are both NaN, x86 hardware
   returns the first; the emulator (QEMU) applies the x87 rule. The translated code
   follows the hardware. The test tolerates differences in which both sides are NaN, and counts them
   separately.
4. **State that the emulator carries over from one case to the next**, and that skewed the comparisons:
   leftover memory in the heap, direction flag left set by an interrupted function,
   MXCSR (writing it from outside does not update the internal rounding and flush-to-zero: an
   `ldmxcsr` must be executed). Now every case starts from a restored state, and the read-only
   sections are protected in the emulator and checked in the translated code.

5. **MMX in the emulator.** After an MMX instruction the hardware sets to all ones the exponent
   of the x87 register that hosts it; the emulator leaves it unchanged. The translated code follows
   the hardware; the test tolerates the difference and counts it with the NaN ones.
6. **Code that continues into another function.** After a `call`, the CRT's math
   routines continue into code that is also the start of another function. The translator
   treated it as a tail call without including it among the functions to translate. Fixed.
7. **`frndint` and `fistp` ignored the x87 rounding mode** (2026-11-15, found
   through the crackling audio, see `runtime.md`). They were translated with `rintl`, relying on
   the fact that the host's x87 control word is the guest's. But when the operand
   originates from a `double` (`fld qword; frndint; fstp qword`), clang reduces
   `(double)rintl((long double)x)` to `rint(x)`: a double-precision call, which rounds
   according to **MXCSR** and not according to the x87 control word. The CRT's `floor` (x87 branch,
   `0xFB2B93`: sets RC to round down with `_controlfp`, then `frndint` in `0xFCA5FE`)
   thus returned `floor(11.5) = 12` and `floor(3.5) = 4`. Verified with `objdump` on the
   library (`call rint@plt`) and with the new `floor` scenario. Now `x87_rnd` (in
   `runtime/altair_x87.h`) rounds by reading the RC field of `c->fpcw` and using only `floorl`,
   `ceill`, `truncl`, which do not depend on any mode. The automatic sample had not
   seen it: it generates random inputs and never sets a mode other than round-to-nearest.
8. **The ID bit of EFLAGS was being lost.** The CRT (`0xFCA495`) decides whether to use the SSE2 branches of the
   math functions by flipping bit 21 with `pushfd`/`popfd` and reading it back, as was
   once done to find out whether the CPU has `cpuid`. `SET_EFLAGS` discarded it: the verdict was "no
   SSE2" and the whole CRT took the x87 branches, unlike any real PC. Now the bit
   lives in `Ctx.eflags_id` (it occupies four bytes that used to be padding: the size of
   `Ctx` does not change). `cpuid` scenario, which compares only the verdict.

`tools/difftrace.py <exe> 0xADDR <case>` replays a case on the two engines and shows the addresses
executed, registers and first memory differences.

## Hooks (2026-11-15)

For observing or changing the game's internal behaviour without touching the generated code by
hand. `altair image --hooks runtime/hooks.txt`: the listed functions (one address per line)
begin with `if (altair_hook(c, address)) return;`. `altair_hook` (in `runtime/altair.c`)
calls the function that the runtime registered with `altair_set_hook`; with no registration
it returns zero. Zero means "carry on": the hook only looked, or tweaked the
arguments on the stack. Non-zero means "I handled it": the hook must have left
registers and stack as they are after the function's `ret`.

Runtime side (`crates/runtime/src/hooks.rs`): every hook counts its calls (`hooks` command
of the control channel), `ALTAIR_HOOK_LOG=N` writes the first N of each with timestamp, `ecx`,
three arguments, return address and thread; the `REPLACEMENTS` table maps an address to
whoever steps in. Changing the list rewrites only the files of the affected functions: a few seconds
of recompilation. Steady-state cost: one indirect call per entry into the function.

First real use: the Animus version (`runtime.md`). First investigative use: the glitches
(`open-questions.md`), with 47 hooks switched on and then off.

## Known limits

- Flags not compared directly by the test (only through their effects).
- `fnstenv`/`fnsave` write only control word, status word and registers.
- x87 with `long double`: works only on hosts with the 80-bit extended format (x86). ARM and
  wasm will need a software implementation of x87. It is the heaviest constraint on the way to the web.
- The host's x87 control word is the guest's throughout guest execution: the
  host functions called by the game (the future runtime) will have to restore it and set it again.
- x87: no register tags, no exception flags; transcendentals not verified.
- x87 rounding: `frndint` and `fist(p)` follow the guest's RC field by construction
  (`x87_rnd`); the four basic operations instead follow the host's control word, which is
  kept equal to the guest's. If the compiler reduces them to `float` or `double` the
  result does not change with round-to-nearest; with another mode it does (not verified).
- SSE in portable C: on non-x86 hosts (ARM, wasm) NaN propagation,
  conversions and rounding control need revisiting.
- `stmxcsr` returns the last control word set: the exception flags are not tracked.
- Atomics use unaligned host pointers: fine on x86-64, to be revisited on ARM and wasm.
- The registers are read and written in the context at every instruction: correct but slow.
  The optimisation (registers in local variables) comes after correctness.
- C++ exceptions and SEH: not handled.
- A single C file per translation: the whole image will need splitting into parts.
