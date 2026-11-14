/* x87 per il codice generato.
 *
 * I registri sono `long double`, che sugli host x86 è il formato esteso a 80 bit dell'x87: il
 * compilatore traduce le operazioni nelle stesse istruzioni dell'originale. Precisione e
 * arrotondamento seguono il guest perché, durante l'esecuzione guest, la parola di controllo
 * x87 dell'host è quella del guest (altair_set_fpcw). Va compilato senza -ffast-math.
 *
 * Non modellati: i tag dei registri (uno stack vuoto si legge come il valore rimasto nel
 * registro) e i flag di eccezione nella parola di stato (sempre zero). Le funzioni
 * trascendenti passano dalla libm in long double: vicine all'hardware, non identiche.
 * Su host senza formato a 80 bit (ARM, wasm) servirà un'implementazione software.
 */
#ifndef ALTAIR_X87_H
#define ALTAIR_X87_H

_Static_assert(sizeof(long double) == 16 && __LDBL_MANT_DIG__ == 64, "serve un host con long double a 80 bit");

#define ST(i) (c->fpr[(c->fptop + (i)) & 7])
static inline void fpush(Ctx *c, f80 v) { c->fptop = (c->fptop - 1) & 7; c->fpr[c->fptop] = v; }
static inline void fpop(Ctx *c) { c->fptop = (c->fptop + 1) & 7; }

static inline f80 rdf80(Ctx *c, u32 a) { f80 v = 0; memcpy(&v, c->mem + a, 10); return v; }
static inline void wrf80(Ctx *c, u32 a, f80 v) { memcpy(c->mem + a, &v, 10); }
static inline void wrf32(Ctx *c, u32 a, float v) { memcpy(c->mem + a, &v, 4); }
static inline void wrf64(Ctx *c, u32 a, double v) { memcpy(c->mem + a, &v, 8); }

/* Parola di stato come la legge `fnstsw`: codici di condizione più la cima dello stack. */
#define X87_SW() ((c->fpsw & ~0x3800u) | (c->fptop << 11))
/* `fcom` e simili: C3 C2 C0 = 111 non ordinati, 001 minore, 100 uguale, 000 maggiore. */
#define X87_COM(a, b) \
    c->fpsw = (c->fpsw & ~0x4500u) | (((a) != (a) || (b) != (b)) ? 0x4500u : (a) < (b) ? 0x100u : (a) == (b) ? 0x4000u : 0u);

/* Conversione a intero come `fistp` (arrotondamento della parola di controllo) o `fisttp`
 * (troncamento). Fuori intervallo o NaN: l'"intero indefinito", il minimo negativo. */
static inline u64 x87_toi(f80 x, int bits, int trunc) {
    f80 r = trunc ? truncl(x) : rintl(x);
    f80 lim = bits == 16 ? 32768.0L : bits == 32 ? 2147483648.0L : 9223372036854775808.0L;
    if (!(r >= -lim && r < lim))
        return 1ull << (bits - 1);
    return (u64)(s64)r;
}

static inline f80 x87_scale(f80 x, f80 n) {
    if (n != n) return n;
    f80 t = truncl(n);
    return scalbnl(x, t > 40000 ? 40000 : t < -40000 ? -40000 : (int)t);
}

/* `fprem` (troncato) e `fprem1` (al più vicino): resto in ST0, C2 = 0 (riduzione completa),
 * C0 C3 C1 = i tre bit bassi del quoziente. */
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

/* `fxam`: classe di ST0 in C3 C2 C0, segno in C1. */
static inline void x87_xam(Ctx *c) {
    f80 v = ST(0);
    u16 se;
    u64 mant;
    memcpy(&mant, &v, 8);
    memcpy(&se, (u8 *)&v + 8, 2);
    u32 e = se & 0x7fff, cc;
    if (e == 0x7fff)
        cc = (mant << 1) ? 0x100u : 0x500u; /* NaN, infinito */
    else if (e == 0)
        cc = mant ? 0x4400u : 0x4000u; /* denormale, zero */
    else
        cc = 0x400u; /* normale */
    c->fpsw = (c->fpsw & ~0x4700u) | cc | ((se & 0x8000) ? 0x200u : 0);
}

/* MMX: i registri MM sono le mantisse dei registri x87 fisici. Ogni scrittura porta la cima
 * dello stack a 0 e l'esponente a tutti uno, come sull'hardware. */
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

/* Ambiente x87 a 32 bit (28 byte), per `fnstenv`/`fldenv` e in testa a `fnsave`/`frstor`.
 * I tag non sono modellati: si scrivono come "tutti vuoti". */
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
