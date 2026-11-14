/* SSE/SSE2 per il codice generato, in C portabile.
 *
 * Ogni operazione lavora corsia per corsia con l'aritmetica IEEE del C: sugli host x86-64 il
 * compilatore la traduce nelle stesse istruzioni SSE dell'originale, quindi i risultati
 * coincidono bit per bit (arrotondamento compreso, perché durante l'esecuzione guest l'MXCSR
 * dell'host è quello del guest). Va compilato senza -ffast-math e con -ffp-contract=off.
 *
 * Differenze note rispetto all'hardware, scelte per coincidere con l'emulatore di riferimento:
 * `rsqrt*` e `rcp*` sono esatti (1/sqrt(x), 1/x), mentre le CPU reali danno 12 bit.
 * Su host non x86 andranno riviste la propagazione dei NaN e le conversioni.
 */
#ifndef ALTAIR_SSE_H
#define ALTAIR_SSE_H

#include <math.h>

static inline float rdf32(Ctx *c, u32 a) { float v; memcpy(&v, c->mem + a, 4); return v; }
static inline double rdf64(Ctx *c, u32 a) { double v; memcpy(&v, c->mem + a, 8); return v; }
static inline Xmm xld(Ctx *c, u32 a) { Xmm v; memcpy(&v, c->mem + a, 16); return v; }
static inline void xst(Ctx *c, u32 a, Xmm v) { memcpy(c->mem + a, &v, 16); }
/* Carica 64 o 32 bit e azzera il resto, come `movq`/`movd`/`movss` da memoria. */
static inline Xmm xq(u64 v) { Xmm r = {.q = {v, 0}}; return r; }
static inline Xmm xd(u32 v) { Xmm r = {.d = {v, 0, 0, 0}}; return r; }

/* Confronti: `comiss`/`ucomiss`/`comisd`/`ucomisd` scrivono zf, pf, cf e azzerano gli altri. */
#define X_COMI(a, b) \
    { int un = (a) != (a) || (b) != (b); zf = un || (a) == (b); pf = un; cf = un || (a) < (b); of = sf = af = 0; }

/* Predicati di `cmpps`/`cmpss`/`cmppd`/`cmpsd` (imm8 da 0 a 7). */
#define X_CMP(x, y, imm, out) \
    switch ((imm) & 7) { \
    case 0: out = (x) == (y); break; \
    case 1: out = (x) < (y); break; \
    case 2: out = (x) <= (y); break; \
    case 3: out = (x) != (x) || (y) != (y); break; \
    case 4: out = !((x) == (y)); break; \
    case 5: out = !((x) < (y)); break; \
    case 6: out = !((x) <= (y)); break; \
    default: out = !((x) != (x) || (y) != (y)); break; \
    }

/* Conversioni verso intero: fuori intervallo o NaN danno 0x80000000, come su x86. */
static inline u32 x_f2i(double x) { return (x >= -2147483648.0 && x < 2147483648.0) ? (u32)(s32)x : 0x80000000u; }

#define X_PS2(name, expr) \
    static inline Xmm x_##name(Xmm a, Xmm b) { \
        Xmm r; \
        for (int i = 0; i < 4; i++) { float x = a.f[i], y = b.f[i]; r.f[i] = (expr); } \
        return r; \
    }
#define X_PD2(name, expr) \
    static inline Xmm x_##name(Xmm a, Xmm b) { \
        Xmm r; \
        for (int i = 0; i < 2; i++) { double x = a.db[i], y = b.db[i]; r.db[i] = (expr); } \
        return r; \
    }
#define X_PS1(name, expr) \
    static inline Xmm x_##name(Xmm a) { \
        Xmm r; \
        for (int i = 0; i < 4; i++) { float x = a.f[i]; r.f[i] = (expr); } \
        return r; \
    }
#define X_INT2(name, lane, n, expr) \
    static inline Xmm x_##name(Xmm a, Xmm b) { \
        Xmm r; \
        for (int i = 0; i < n; i++) r.lane[i] = (expr); \
        return r; \
    }

X_PS2(addps, x + y)
X_PS2(subps, x - y)
X_PS2(mulps, x * y)
X_PS2(divps, x / y)
/* min/max di x86: se uno è NaN o sono uguali, vince il secondo operando. */
X_PS2(minps, x < y ? x : y)
X_PS2(maxps, x > y ? x : y)
X_PD2(addpd, x + y)
X_PD2(subpd, x - y)
X_PD2(mulpd, x * y)
X_PD2(divpd, x / y)
X_PD2(minpd, x < y ? x : y)
X_PD2(maxpd, x > y ? x : y)
X_PS1(sqrtps, sqrtf(x))
X_PS1(rsqrtps, 1.0f / sqrtf(x))
X_PS1(rcpps, 1.0f / x)

X_INT2(and, q, 2, a.q[i] & b.q[i])
X_INT2(andn, q, 2, ~a.q[i] & b.q[i])
X_INT2(or, q, 2, a.q[i] | b.q[i])
X_INT2(xor, q, 2, a.q[i] ^ b.q[i])
X_INT2(paddb, b, 16, (u8)(a.b[i] + b.b[i]))
X_INT2(paddw, w, 8, (u16)(a.w[i] + b.w[i]))
X_INT2(paddd, d, 4, a.d[i] + b.d[i])
X_INT2(paddq, q, 2, a.q[i] + b.q[i])
X_INT2(psubb, b, 16, (u8)(a.b[i] - b.b[i]))
X_INT2(psubw, w, 8, (u16)(a.w[i] - b.w[i]))
X_INT2(psubd, d, 4, a.d[i] - b.d[i])
X_INT2(psubq, q, 2, a.q[i] - b.q[i])
X_INT2(pcmpeqb, b, 16, a.b[i] == b.b[i] ? 0xff : 0)
X_INT2(pcmpeqw, w, 8, a.w[i] == b.w[i] ? 0xffff : 0)
X_INT2(pcmpeqd, d, 4, a.d[i] == b.d[i] ? 0xffffffffu : 0)
X_INT2(pcmpgtw, w, 8, (s16)a.w[i] > (s16)b.w[i] ? 0xffff : 0)
X_INT2(pcmpgtd, d, 4, (s32)a.d[i] > (s32)b.d[i] ? 0xffffffffu : 0)
X_INT2(pmullw, w, 8, (u16)((s16)a.w[i] * (s16)b.w[i]))
X_INT2(pmulhw, w, 8, (u16)(((s32)(s16)a.w[i] * (s32)(s16)b.w[i]) >> 16))
X_INT2(pmuludq, q, 2, (u64)a.d[2 * i] * b.d[2 * i])

static inline Xmm x_cmpps(Xmm a, Xmm b, u32 imm) {
    Xmm r;
    for (int i = 0; i < 4; i++) { int t; X_CMP(a.f[i], b.f[i], imm, t) r.d[i] = t ? 0xffffffffu : 0; }
    return r;
}
static inline Xmm x_cmppd(Xmm a, Xmm b, u32 imm) {
    Xmm r;
    for (int i = 0; i < 2; i++) { int t; X_CMP(a.db[i], b.db[i], imm, t) r.q[i] = t ? ~0ull : 0; }
    return r;
}
static inline u32 x_cmpss(float a, float b, u32 imm) { int t; X_CMP(a, b, imm, t) return t ? 0xffffffffu : 0; }
static inline u64 x_cmpsd(double a, double b, u32 imm) { int t; X_CMP(a, b, imm, t) return t ? ~0ull : 0; }

static inline Xmm x_shufps(Xmm a, Xmm b, u32 imm) {
    Xmm r = {.d = {a.d[imm & 3], a.d[(imm >> 2) & 3], b.d[(imm >> 4) & 3], b.d[(imm >> 6) & 3]}};
    return r;
}
static inline Xmm x_shufpd(Xmm a, Xmm b, u32 imm) { Xmm r = {.q = {a.q[imm & 1], b.q[(imm >> 1) & 1]}}; return r; }
static inline Xmm x_pshufd(Xmm s, u32 imm) {
    Xmm r = {.d = {s.d[imm & 3], s.d[(imm >> 2) & 3], s.d[(imm >> 4) & 3], s.d[(imm >> 6) & 3]}};
    return r;
}
static inline Xmm x_unpcklps(Xmm a, Xmm b) { Xmm r = {.d = {a.d[0], b.d[0], a.d[1], b.d[1]}}; return r; }
static inline Xmm x_unpckhps(Xmm a, Xmm b) { Xmm r = {.d = {a.d[2], b.d[2], a.d[3], b.d[3]}}; return r; }
static inline Xmm x_unpcklpd(Xmm a, Xmm b) { Xmm r = {.q = {a.q[0], b.q[0]}}; return r; }
static inline Xmm x_unpckhpd(Xmm a, Xmm b) { Xmm r = {.q = {a.q[1], b.q[1]}}; return r; }
static inline Xmm x_punpcklbw(Xmm a, Xmm b) { Xmm r; for (int i = 0; i < 8; i++) { r.b[2 * i] = a.b[i]; r.b[2 * i + 1] = b.b[i]; } return r; }
static inline Xmm x_punpcklwd(Xmm a, Xmm b) { Xmm r; for (int i = 0; i < 4; i++) { r.w[2 * i] = a.w[i]; r.w[2 * i + 1] = b.w[i]; } return r; }
static inline Xmm x_movhlps(Xmm a, Xmm b) { a.q[0] = b.q[1]; return a; }
static inline Xmm x_movlhps(Xmm a, Xmm b) { a.q[1] = b.q[0]; return a; }

static inline Xmm x_cvtps2pd(Xmm s) { Xmm r; r.db[0] = (double)s.f[0]; r.db[1] = (double)s.f[1]; return r; }
static inline Xmm x_cvtpd2ps(Xmm s) { Xmm r = {.q = {0, 0}}; r.f[0] = (float)s.db[0]; r.f[1] = (float)s.db[1]; return r; }
static inline Xmm x_cvtdq2ps(Xmm s) { Xmm r; for (int i = 0; i < 4; i++) r.f[i] = (float)(s32)s.d[i]; return r; }
static inline Xmm x_cvttps2dq(Xmm s) { Xmm r; for (int i = 0; i < 4; i++) r.d[i] = x_f2i(s.f[i]); return r; }
static inline Xmm x_cvtps2dq(Xmm s) { Xmm r; for (int i = 0; i < 4; i++) r.d[i] = x_f2i(rintf(s.f[i])); return r; }

static inline Xmm x_pslldq(Xmm a, u32 n) { Xmm r = {.q = {0, 0}}; for (u32 i = n; i < 16; i++) r.b[i] = a.b[i - n]; return r; }
static inline Xmm x_psrldq(Xmm a, u32 n) { Xmm r = {.q = {0, 0}}; for (u32 i = n; i < 16; i++) r.b[i - n] = a.b[i]; return r; }
static inline Xmm x_psllq(Xmm a, u64 n) { for (int i = 0; i < 2; i++) a.q[i] = n > 63 ? 0 : a.q[i] << n; return a; }
static inline Xmm x_psrlq(Xmm a, u64 n) { for (int i = 0; i < 2; i++) a.q[i] = n > 63 ? 0 : a.q[i] >> n; return a; }
static inline Xmm x_pslld(Xmm a, u64 n) { for (int i = 0; i < 4; i++) a.d[i] = n > 31 ? 0 : a.d[i] << n; return a; }
static inline Xmm x_psrld(Xmm a, u64 n) { for (int i = 0; i < 4; i++) a.d[i] = n > 31 ? 0 : a.d[i] >> n; return a; }
static inline Xmm x_psrad(Xmm a, u64 n) { for (int i = 0; i < 4; i++) a.d[i] = (u32)((s32)a.d[i] >> (n > 31 ? 31 : n)); return a; }
static inline Xmm x_psllw(Xmm a, u64 n) { for (int i = 0; i < 8; i++) a.w[i] = n > 15 ? 0 : (u16)(a.w[i] << n); return a; }
static inline Xmm x_psrlw(Xmm a, u64 n) { for (int i = 0; i < 8; i++) a.w[i] = n > 15 ? 0 : (u16)(a.w[i] >> n); return a; }
static inline Xmm x_psraw(Xmm a, u64 n) { for (int i = 0; i < 8; i++) a.w[i] = (u16)((s16)a.w[i] >> (n > 15 ? 15 : n)); return a; }

static inline u32 x_movmskps(Xmm a) { return (a.d[0] >> 31) | ((a.d[1] >> 31) << 1) | ((a.d[2] >> 31) << 2) | ((a.d[3] >> 31) << 3); }
static inline u32 x_movmskpd(Xmm a) { return (u32)(a.q[0] >> 63) | ((u32)(a.q[1] >> 63) << 1); }
static inline u32 x_pmovmskb(Xmm a) { u32 m = 0; for (int i = 0; i < 16; i++) m |= (u32)(a.b[i] >> 7) << i; return m; }

#endif
