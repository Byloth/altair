/* Supporto per il codice C generato da `altair translate`.
 *
 * Il codice generato non contiene altro che chiamate a queste funzioni e macro: tutto ciò
 * che dipende dalla piattaforma sta nel runtime, non nel codice tradotto.
 */
#ifndef ALTAIR_H
#define ALTAIR_H

#include <stdint.h>
#include <string.h>

typedef uint8_t u8;
typedef uint16_t u16;
typedef uint32_t u32;
typedef uint64_t u64;
typedef int8_t s8;
typedef int16_t s16;
typedef int32_t s32;
typedef int64_t s64;

typedef struct Ctx Ctx;

/* Un registro XMM, visto per corsie. */
typedef union {
    u8 b[16];
    u16 w[8];
    u32 d[4];
    u64 q[2];
    float f[4];
    double db[2];
} Xmm;

/* Stato di un thread guest. L'ordine dei campi è replicato in tools/difftest.py. */
struct Ctx {
    u32 eax, ecx, edx, ebx, esp, ebp, esi, edi;
    u32 df;        /* direction flag: 0 avanti, 1 indietro */
    u32 fs_base;   /* indirizzo guest del TEB */
    u8 *mem;       /* base dello spazio di indirizzamento guest (4 GB riservati) */
    /* Chiamata a un indirizzo che non è una funzione tradotta: import e funzioni host.
     * All'ingresso l'indirizzo di ritorno è in cima allo stack guest, come dopo una `call`;
     * la funzione host lo toglie insieme agli argomenti. Ritorna 0, oppure un valore diverso
     * da zero se l'indirizzo non è gestito (il runtime fa scattare una trap). */
    int (*host_call)(Ctx *c, u32 addr);
    void *trap_jmp;
    const char *trap_msg;
    u32 trap_addr;
    u32 mxcsr;     /* parola di controllo SSE del guest; all'avvio 0x1f80 */
    Xmm xmm[8];
};

typedef void (*AltairFn)(Ctx *c);
typedef struct {
    u32 addr;
    AltairFn fn;
} AltairEntry;

/* Definiti dal codice generato: funzioni tradotte, ordinate per indirizzo. */
extern const AltairEntry altair_functions[];
extern const u32 altair_function_count;

void altair_call_indirect(Ctx *c, u32 addr);
_Noreturn void altair_trap(Ctx *c, const char *msg, u32 addr);
/* Esegue la funzione guest `addr`. Ritorna 0, oppure 1 se è scattata una trap
 * (motivo in c->trap_msg, indirizzo in c->trap_addr). */
int altair_run(Ctx *c, u32 addr);
/* Contatore di tempo a 1 GHz per `rdtsc`. */
u64 altair_rdtsc(Ctx *c);
/* `cpuid`: una CPU fissa di classe SSE3, uguale su ogni host. */
void altair_cpuid(Ctx *c);
/* `ldmxcsr`: aggiorna la parola di controllo del guest e quella dell'host. */
void altair_set_mxcsr(Ctx *c, u32 v);
u8 *altair_mem_create(void);
void altair_mem_destroy(u8 *mem);

static inline u8 rd8(Ctx *c, u32 a) { return c->mem[a]; }
static inline u16 rd16(Ctx *c, u32 a) { u16 v; memcpy(&v, c->mem + a, 2); return v; }
static inline u32 rd32(Ctx *c, u32 a) { u32 v; memcpy(&v, c->mem + a, 4); return v; }
static inline void wr8(Ctx *c, u32 a, u8 v) { c->mem[a] = v; }
static inline void wr16(Ctx *c, u32 a, u16 v) { memcpy(c->mem + a, &v, 2); }
static inline void wr32(Ctx *c, u32 a, u32 v) { memcpy(c->mem + a, &v, 4); }
static inline u64 rd64(Ctx *c, u32 a) { u64 v; memcpy(&v, c->mem + a, 8); return v; }
static inline void wr64(Ctx *c, u32 a, u64 v) { memcpy(c->mem + a, &v, 8); }

/* Scrittura dei registri a 8 e 16 bit: scritture strette dirette nel campo a 32 bit.
 * Non si usa `reg = (reg & maschera) | v` perché clang 18 lo miscompila (vedi
 * tests/compiler/clang18-partial-register.c). Richiede un host little-endian. */
_Static_assert(__BYTE_ORDER__ == __ORDER_LITTLE_ENDIAN__, "serve un host little-endian");
#define SET8L(reg, v) do { u8 t_ = (u8)(v); *(u8 *)&(reg) = t_; } while (0)
#define SET8H(reg, v) do { u8 t_ = (u8)(v); *((u8 *)&(reg) + 1) = t_; } while (0)
#define SET16(reg, v) do { u16 t_ = (u16)(v); memcpy(&(reg), &t_, 2); } while (0)

/* Puntatore host a un indirizzo guest, per le operazioni atomiche. */
#define MP(T, a) ((T *)(c->mem + (u32)(a)))

/* Registro EFLAGS ricostruito dalle variabili locali (bit 1 sempre a 1). */
#define EFLAGS() (cf | 2u | (pf << 2) | (af << 4) | (zf << 6) | (sf << 7) | (c->df << 10) | (of << 11))
#define SET_EFLAGS(v) \
    cf = (v) & 1; pf = ((v) >> 2) & 1; af = ((v) >> 4) & 1; zf = ((v) >> 6) & 1; sf = ((v) >> 7) & 1; \
    c->df = ((v) >> 10) & 1; of = ((v) >> 11) & 1;

/* Flag: variabili locali `cf zf sf of pf af` della funzione generata, sempre 0 o 1. */
#define F_ZSP(r, bits) \
    zf = ((r) == 0); sf = ((r) >> ((bits) - 1)) & 1; pf = !__builtin_parity((u8)(r));
#define F_ADD(a, b, r, bits) \
    cf = (r) < (a); of = ((((a) ^ (r)) & ((b) ^ (r))) >> ((bits) - 1)) & 1; \
    af = (((a) ^ (b) ^ (r)) >> 4) & 1; F_ZSP(r, bits)
#define F_SUB(a, b, r, bits) \
    cf = (a) < (b); of = ((((a) ^ (b)) & ((a) ^ (r))) >> ((bits) - 1)) & 1; \
    af = (((a) ^ (b) ^ (r)) >> 4) & 1; F_ZSP(r, bits)
#define F_LOGIC(r, bits) cf = 0; of = 0; af = 0; F_ZSP(r, bits)

#include "altair_sse.h"

#endif
