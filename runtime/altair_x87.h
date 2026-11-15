/* x87 for the generated code.
 *
 * The registers are `long double`, which on x86 hosts is the x87's 80-bit extended format: the
 * compiler translates the operations into the same instructions as the original. Precision and
 * rounding follow the guest because, during guest execution, the host's x87 control word
 * is the guest's one (altair_set_fpcw). It must be compiled without -ffast-math.
 *
 * Not modelled: the register tags (an empty stack reads as the value left in the
 * register) and the exception flags in the status word (always zero). The transcendental
 * functions go through libm in long double: close to the hardware, not identical.
 * On hosts without an 80-bit format (ARM, wasm) a software implementation will be needed.
 */
#ifndef ALTAIR_X87_H
#define ALTAIR_X87_H

_Static_assert(sizeof(long double) == 16 && __LDBL_MANT_DIG__ == 64, "a host with an 80-bit long double is required");

#define ST(i) (c->fpr[(c->fptop + (i)) & 7])
static inline void fpush(Ctx *c, f80 v) { c->fptop = (c->fptop - 1) & 7; c->fpr[c->fptop] = v; }
static inline void fpop(Ctx *c) { c->fptop = (c->fptop + 1) & 7; }

static inline f80 rdf80(Ctx *c, u32 a) { f80 v = 0; memcpy(&v, c->mem + a, 10); return v; }
static inline void wrf80(Ctx *c, u32 a, f80 v) { memcpy(c->mem + a, &v, 10); }
static inline void wrf32(Ctx *c, u32 a, float v) { memcpy(c->mem + a, &v, 4); }
static inline void wrf64(Ctx *c, u32 a, double v) { memcpy(c->mem + a, &v, 8); }

/* Status word as `fnstsw` reads it: condition codes plus the top of the stack. */
#define X87_SW() ((c->fpsw & ~0x3800u) | (c->fptop << 11))
/* `fcom` and the like: C3 C2 C0 = 111 unordered, 001 less, 100 equal, 000 greater. */
#define X87_COM(a, b) \
    c->fpsw = (c->fpsw & ~0x4500u) | (((a) != (a) || (b) != (b)) ? 0x4500u : (a) < (b) ? 0x100u : (a) == (b) ? 0x4000u : 0u);

/* Conversion to integer like `fistp` (rounding from the control word) or `fisttp`
 * (truncation). Out of range or NaN: the "integer indefinite", the most negative value. */
/* Rounding to integer according to the RC field of the guest's control word (`frndint`,
 * `fist`). No `rintl`: when the operand originates from a double the compiler reduces it to
 * `rint`, which rounds according to MXCSR and ignores the x87 mode; the CRT's `floor` (which sets
 * RC to round down and then uses `frndint`) thus returned 12 for 11.5. Here the decision is made by
 * reading RC and the rounding uses operations that do not depend on any mode. */
/* The library versions (`truncl`, `floorl`…) are real, slow calls: in the tutorial
 * crowd `truncl` alone was 27% of the rendering thread (the CRT's `_ftol` sets RC to
 * truncation and does `fistp`, thousands of times per frame). Here the conversion goes through a
 * 64-bit integer, which the compiler does in place; outside that range an 80-bit
 * value is already an integer (or infinite, or NaN) and stays as it is. */
static inline f80 x87_trunc(f80 x) {
    if (x > -9223372036854775808.0L && x < 9223372036854775808.0L) {
        s64 i = (s64)x;
        return i == 0 ? __builtin_copysignl(0.0L, x) : (f80)i;
    }
    return x;
}

static inline f80 x87_rnd(Ctx *c, f80 x) {
    f80 t = x87_trunc(x);
    switch ((c->fpcw >> 10) & 3) {
    case 1: return t > x ? t - 1.0L : t;
    case 2: return t < x ? t + 1.0L : t;
    case 3: return t;
    }
    /* To nearest, halves to even. Outside the range of 64-bit integers `t == x`. */
    f80 d = x - t;
    if (d < 0) d = -d;
    if (d > 0.5L || (d == 0.5L && ((s64)t & 1))) {
        f80 r = x < 0 ? t - 1.0L : t + 1.0L;
        return r == 0 ? __builtin_copysignl(0.0L, x) : r;
    }
    return t;
}

static inline u64 x87_toi(Ctx *c, f80 x, int bits, int trunc) {
    f80 r = trunc ? x87_trunc(x) : x87_rnd(c, x);
    f80 lim = bits == 16 ? 32768.0L : bits == 32 ? 2147483648.0L : 9223372036854775808.0L;
    if (!(r >= -lim && r < lim))
        return 1ull << (bits - 1);
    return (u64)(s64)r;
}

static inline f80 x87_scale(f80 x, f80 n) {
    if (n != n) return n;
    f80 t = x87_trunc(n);
    return scalbnl(x, t > 40000 ? 40000 : t < -40000 ? -40000 : (int)t);
}

/* `fprem` (truncated) and `fprem1` (to nearest): remainder in ST0, C2 = 0 (complete reduction),
 * C0 C3 C1 = the three low bits of the quotient. */
static inline void x87_prem(Ctx *c, int ieee) {
    f80 a = ST(0), b = ST(1);
    f80 r = ieee ? remainderl(a, b) : fmodl(a, b);
    u32 cc = 0;
    if (r == r && b != 0) {
        f80 q = (a - r) / b;
        long long qi = (q < 9e18L && q > -9e18L) ? (long long)q : 0;
        if (qi < 0) qi = -qi;
        cc = ((qi & 4) ? 0x100u : 0) | ((qi & 2) ? 0x4000u : 0) | ((qi & 1) ? 0x200u : 0);
    }
    ST(0) = r;
    c->fpsw = (c->fpsw & ~0x4700u) | cc;
}

/* `fxam`: class of ST0 in C3 C2 C0, sign in C1. */
static inline void x87_xam(Ctx *c) {
    f80 v = ST(0);
    u16 se;
    u64 mant;
    memcpy(&mant, &v, 8);
    memcpy(&se, (u8 *)&v + 8, 2);
    u32 e = se & 0x7fff, cc;
    if (e == 0x7fff)
        cc = (mant << 1) ? 0x100u : 0x500u; /* NaN, infinity */
    else if (e == 0)
        cc = mant ? 0x4400u : 0x4000u; /* denormal, zero */
    else
        cc = 0x400u; /* normal */
    c->fpsw = (c->fpsw & ~0x4700u) | cc | ((se & 0x8000) ? 0x200u : 0);
}

/* MMX: the MM registers are the mantissas of the physical x87 registers. Every write sets the top
 * of the stack to 0 and the exponent to all ones, as on the hardware. */
static inline u64 mmr(Ctx *c, int i) { u64 v; memcpy(&v, &c->fpr[i], 8); return v; }
static inline void mmw(Ctx *c, int i, u64 v) {
    u16 e = 0xffff;
    memcpy(&c->fpr[i], &v, 8);
    memcpy((u8 *)&c->fpr[i] + 8, &e, 2);
    c->fptop = 0;
}
static inline u64 m_packssdw(u64 a, u64 b) {
    Xmm r = {.q = {0, 0}};
    r.w[0] = (u16)x_sat16((s32)a); r.w[1] = (u16)x_sat16((s32)(a >> 32));
    r.w[2] = (u16)x_sat16((s32)b); r.w[3] = (u16)x_sat16((s32)(b >> 32));
    return r.q[0];
}
static inline u64 m_packsswb(u64 a, u64 b) {
    Xmm r = {.q = {0, 0}}, x = {.q = {a, b}};
    for (int i = 0; i < 8; i++) { s32 v = (s16)x.w[i]; r.b[i] = (u8)(v > 127 ? 127 : v < -128 ? -128 : v); }
    return r.q[0];
}
static inline u64 m_packuswb(u64 a, u64 b) {
    Xmm r = {.q = {0, 0}}, x = {.q = {a, b}};
    for (int i = 0; i < 8; i++) { s32 v = (s16)x.w[i]; r.b[i] = (u8)(v > 255 ? 255 : v < 0 ? 0 : v); }
    return r.q[0];
}
static inline u64 m_pshufw(u64 s, u32 imm) {
    Xmm x = {.q = {s, 0}}, r = {.q = {0, 0}};
    for (int i = 0; i < 4; i++) r.w[i] = x.w[(imm >> (2 * i)) & 3];
    return r.q[0];
}

/* 32-bit x87 environment (28 bytes), for `fnstenv`/`fldenv` and at the head of `fnsave`/`frstor`.
 * The tags are not modelled: they are written as "all empty". */
static inline void x87_env_store(Ctx *c, u32 a) {
    memset(c->mem + a, 0, 28);
    wr32(c, a, 0xffff0000u | c->fpcw);
    wr32(c, a + 4, 0xffff0000u | X87_SW());
    wr32(c, a + 8, 0xffffffffu);
}
static inline void x87_env_load(Ctx *c, u32 a) {
    u32 sw = rd32(c, a + 4);
    altair_set_fpcw(c, rd32(c, a) & 0xffff);
    c->fptop = (sw >> 11) & 7;
    c->fpsw = sw & 0x4700u;
}
static inline void x87_save(Ctx *c, u32 a) {
    x87_env_store(c, a);
    for (int i = 0; i < 8; i++) wrf80(c, a + 28 + 10 * i, ST(i));
    c->fptop = 0;
    c->fpsw = 0;
    altair_set_fpcw(c, 0x37f);
}
static inline void x87_restore(Ctx *c, u32 a) {
    x87_env_load(c, a);
    for (int i = 0; i < 8; i++) ST(i) = rdf80(c, a + 28 + 10 * i);
}

#endif
