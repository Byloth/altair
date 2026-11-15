/* Runtime minimo per il codice generato: registro delle funzioni, trap, memoria guest. */
#include "altair.h"

#ifdef _WIN32
/* Windows (clang in versione MinGW: è l'unica che dà un `long double` a 80 bit).
   Il salto fuori dal codice guest usa le funzioni intrinseche del compilatore: `longjmp` del
   sistema srotolerebbe lo stack con il meccanismo delle eccezioni, attraversando anche i frame
   del runtime in Rust. */
#include <windows.h>
typedef void *AltairJmp[5];
#define ALTAIR_SETJMP(b) __builtin_setjmp(b)
#define ALTAIR_LONGJMP(b) __builtin_longjmp(b, 1)
#else
#include <setjmp.h>
#include <sys/mman.h>
#include <time.h>
typedef jmp_buf AltairJmp;
#define ALTAIR_SETJMP(b) setjmp(b)
#define ALTAIR_LONGJMP(b) longjmp(b, 1)
#endif

#define GUEST_SPACE (1ull << 32)

static AltairFn find(u32 addr) {
    u32 lo = 0, hi = altair_function_count;
    while (lo < hi) {
        u32 mid = lo + (hi - lo) / 2;
        if (altair_functions[mid].addr < addr)
            lo = mid + 1;
        else
            hi = mid;
    }
    return lo < altair_function_count && altair_functions[lo].addr == addr ? altair_functions[lo].fn : 0;
}

/* Le ultime destinazioni di chiamate e salti indiretti di questo thread: quando qualcosa va
 * storto dicono da dove si è arrivati, anche attraverso le tail call che non lasciano un
 * indirizzo di ritorno sullo stack. */
#define TRAIL 32
static __thread u32 trail[TRAIL];
static __thread u32 trail_pos;

int altair_trail(u32 *out, int max) {
    int n = 0;
    for (u32 i = 0; i < TRAIL && n < max; i++) {
        u32 v = trail[(trail_pos - 1 - i) % TRAIL];
        if (v)
            out[n++] = v;
    }
    return n;
}

static AltairHook hook_fn;

void altair_set_hook(AltairHook fn) { hook_fn = fn; }

int altair_hook(Ctx *c, u32 addr) { return hook_fn ? hook_fn(c, addr) : 0; }

void altair_call_indirect(Ctx *c, u32 addr) {
    trail[trail_pos++ % TRAIL] = addr;
    AltairFn fn = find(addr);
    if (fn)
        fn(c);
    else if (!c->host_call || c->host_call(c, addr))
        altair_trap(c, "funzione non tradotta o import non gestito", addr);
}

_Noreturn void altair_trap(Ctx *c, const char *msg, u32 addr) {
    c->trap_msg = msg;
    c->trap_addr = addr;
    ALTAIR_LONGJMP(*(AltairJmp *)c->trap_jmp);
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

#if defined(__x86_64__) || defined(__i386__)
static u16 host_fpcw(void) { u16 v; __asm__ volatile("fnstcw %0" : "=m"(v)); return v; }
static void set_host_fpcw(u16 v) { __asm__ volatile("fldcw %0" : : "m"(v)); }
#else
static u16 host_fpcw(void) { return 0x37f; }
static void set_host_fpcw(u16 v) { (void)v; }
#endif

void altair_set_fpcw(Ctx *c, u32 v) {
    c->fpcw = v & 0xffff;
    set_host_fpcw((u16)(v | 0x3f)); /* le eccezioni restano mascherate sull'host */
}

void altair_set_mxcsr(Ctx *c, u32 v) {
    c->mxcsr = v;
    set_host_mxcsr(v | 0x1f80); /* le eccezioni restano mascherate sull'host */
}

int altair_run(Ctx *c, u32 addr) {
    AltairJmp jb;
    void *outer = c->trap_jmp;
    volatile u32 saved = host_mxcsr();
    volatile u16 saved_cw = host_fpcw();
    c->trap_jmp = &jb;
    c->trap_msg = 0;
    if (!outer) {
        set_host_mxcsr(c->mxcsr | 0x1f80);
        set_host_fpcw((u16)(c->fpcw | 0x3f));
    }
    int trapped = ALTAIR_SETJMP(jb);
    if (!trapped)
        altair_call_indirect(c, addr);
    c->trap_jmp = outer;
    if (!outer) {
        set_host_mxcsr(saved);
        set_host_fpcw(saved_cw);
    }
    return trapped;
}

#ifdef _WIN32
/* Su Linux le pagine si materializzano quando vengono toccate. Su Windows la memoria va
   impegnata subito e tutta: una zona solo riservata darebbe errore alle funzioni di sistema
   che ci scrivono dentro (una ReadFile in un buffer del gioco). Sono 4 GB di "commit", che non
   occupano RAM finché non vengono usati ma devono stare nel limite di RAM più file di paging. */
u8 *altair_mem_create(void) {
    return VirtualAlloc(0, GUEST_SPACE, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE);
}

void altair_mem_destroy(u8 *mem) {
    if (mem)
        VirtualFree(mem, 0, MEM_RELEASE);
}

u64 altair_rdtsc(Ctx *c) {
    (void)c;
    LARGE_INTEGER now, freq;
    QueryPerformanceCounter(&now);
    QueryPerformanceFrequency(&freq);
    return (u64)((long double)now.QuadPart * 1000000000.0L / (long double)freq.QuadPart);
}
#else
u8 *altair_mem_create(void) {
    void *p = mmap(0, GUEST_SPACE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS | MAP_NORESERVE, -1, 0);
    return p == MAP_FAILED ? 0 : p;
}

void altair_mem_destroy(u8 *mem) {
    if (mem)
        munmap(mem, GUEST_SPACE);
}

u64 altair_rdtsc(Ctx *c) {
    (void)c;
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (u64)ts.tv_sec * 1000000000ull + (u64)ts.tv_nsec;
}
#endif

void altair_cpuid(Ctx *c) {
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
