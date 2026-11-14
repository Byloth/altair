/* Runtime minimo per il codice generato: registro delle funzioni, trap, memoria guest. */
#include "acrek.h"

#include <setjmp.h>
#include <sys/mman.h>
#include <time.h>

#define GUEST_SPACE (1ull << 32)

static AcrekFn find(u32 addr) {
    u32 lo = 0, hi = acrek_function_count;
    while (lo < hi) {
        u32 mid = lo + (hi - lo) / 2;
        if (acrek_functions[mid].addr < addr)
            lo = mid + 1;
        else
            hi = mid;
    }
    return lo < acrek_function_count && acrek_functions[lo].addr == addr ? acrek_functions[lo].fn : 0;
}

void acrek_call_indirect(Ctx *c, u32 addr) {
    AcrekFn fn = find(addr);
    if (fn)
        fn(c);
    else if (!c->host_call || c->host_call(c, addr))
        acrek_trap(c, "funzione non tradotta o import non gestito", addr);
}

_Noreturn void acrek_trap(Ctx *c, const char *msg, u32 addr) {
    c->trap_msg = msg;
    c->trap_addr = addr;
    longjmp(*(jmp_buf *)c->trap_jmp, 1);
}

/* MXCSR dell'host: durante l'esecuzione guest vale quello del guest, così arrotondamento,
 * flush-to-zero e denormali si comportano come nell'originale. */
#if defined(__x86_64__) || defined(__i386__)
static u32 host_mxcsr(void) { return __builtin_ia32_stmxcsr(); }
static void set_host_mxcsr(u32 v) { __builtin_ia32_ldmxcsr(v); }
#else
static u32 host_mxcsr(void) { return 0x1f80; }
static void set_host_mxcsr(u32 v) { (void)v; }
#endif

void acrek_set_mxcsr(Ctx *c, u32 v) {
    c->mxcsr = v;
    set_host_mxcsr(v | 0x1f80); /* le eccezioni restano mascherate sull'host */
}

int acrek_run(Ctx *c, u32 addr) {
    jmp_buf jb;
    void *outer = c->trap_jmp;
    volatile u32 saved = host_mxcsr();
    c->trap_jmp = &jb;
    c->trap_msg = 0;
    if (!outer)
        set_host_mxcsr(c->mxcsr | 0x1f80);
    int trapped = setjmp(jb);
    if (!trapped)
        acrek_call_indirect(c, addr);
    c->trap_jmp = outer;
    if (!outer)
        set_host_mxcsr(saved);
    return trapped;
}

u8 *acrek_mem_create(void) {
    void *p = mmap(0, GUEST_SPACE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS | MAP_NORESERVE, -1, 0);
    return p == MAP_FAILED ? 0 : p;
}

void acrek_mem_destroy(u8 *mem) {
    if (mem)
        munmap(mem, GUEST_SPACE);
}

u64 acrek_rdtsc(Ctx *c) {
    (void)c;
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (u64)ts.tv_sec * 1000000000ull + (u64)ts.tv_nsec;
}

void acrek_cpuid(Ctx *c) {
    u32 leaf = c->eax;
    c->eax = c->ebx = c->ecx = c->edx = 0;
    if (leaf == 0) {
        c->eax = 1; /* foglia massima */
        c->ebx = 0x756e6547; /* "GenuineIntel" */
        c->edx = 0x49656e69;
        c->ecx = 0x6c65746e;
    } else if (leaf == 1) {
        c->eax = 0x000006f6; /* famiglia 6: Core 2 */
        /* FPU, TSC, CX8, CMOV, MMX, FXSR, SSE, SSE2 */
        c->edx = (1u << 0) | (1u << 4) | (1u << 8) | (1u << 15) | (1u << 23) | (1u << 24) | (1u << 25) | (1u << 26);
        c->ecx = 1u << 0; /* SSE3 */
    }
}
