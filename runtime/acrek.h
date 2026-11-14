/* Supporto per il codice C generato da `acrek translate`.
 *
 * Il codice generato non contiene altro che chiamate a queste funzioni e macro: tutto ciò
 * che dipende dalla piattaforma sta nel runtime, non nel codice tradotto.
 */
#ifndef ACREK_H
#define ACREK_H

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
};

typedef void (*AcrekFn)(Ctx *c);
typedef struct {
    u32 addr;
    AcrekFn fn;
} AcrekEntry;

/* Definiti dal codice generato: funzioni tradotte, ordinate per indirizzo. */
extern const AcrekEntry acrek_functions[];
extern const u32 acrek_function_count;

void acrek_call_indirect(Ctx *c, u32 addr);
_Noreturn void acrek_trap(Ctx *c, const char *msg, u32 addr);
/* Esegue la funzione guest `addr`. Ritorna 0, oppure 1 se è scattata una trap
 * (motivo in c->trap_msg, indirizzo in c->trap_addr). */
int acrek_run(Ctx *c, u32 addr);
/* Contatore di tempo a 1 GHz per `rdtsc`. */
u64 acrek_rdtsc(Ctx *c);
/* `cpuid`: una CPU fissa di classe SSE3, uguale su ogni host. */
void acrek_cpuid(Ctx *c);
u8 *acrek_mem_create(void);
void acrek_mem_destroy(u8 *mem);

static inline u8 rd8(Ctx *c, u32 a) { return c->mem[a]; }
static inline u16 rd16(Ctx *c, u32 a) { u16 v; memcpy(&v, c->mem + a, 2); return v; }
static inline u32 rd32(Ctx *c, u32 a) { u32 v; memcpy(&v, c->mem + a, 4); return v; }
static inline void wr8(Ctx *c, u32 a, u8 v) { c->mem[a] = v; }
static inline void wr16(Ctx *c, u32 a, u16 v) { memcpy(c->mem + a, &v, 2); }
static inline void wr32(Ctx *c, u32 a, u32 v) { memcpy(c->mem + a, &v, 4); }

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

#endif
