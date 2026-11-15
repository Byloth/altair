/* Minimal runtime for the generated code: function registry, traps, guest memory. */
#include "altair.h"

#ifdef _WIN32
/* Windows (the MinGW flavour of clang: it is the only one that gives an 80-bit `long double`).
   The jump out of guest code uses the compiler's intrinsic functions: the system's `longjmp`
   would unwind the stack with the exception mechanism, also crossing the frames
   of the Rust runtime. */
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

/* The last targets of this thread's indirect calls and jumps: when something goes
 * wrong they say how one got there, even through tail calls that leave no
 * return address on the stack. */
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
        altair_trap(c, "untranslated function or unhandled import", addr);
}

_Noreturn void altair_trap(Ctx *c, const char *msg, u32 addr) {
    c->trap_msg = msg;
    c->trap_addr = addr;
    ALTAIR_LONGJMP(*(AltairJmp *)c->trap_jmp);
}

/* Host MXCSR: during guest execution the guest's one is in effect, so rounding,
 * flush-to-zero and denormals behave as in the original. */
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
    set_host_fpcw((u16)(v | 0x3f)); /* exceptions stay masked on the host */
}

void altair_set_mxcsr(Ctx *c, u32 v) {
    c->mxcsr = v;
    set_host_mxcsr(v | 0x1f80); /* exceptions stay masked on the host */
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
/* On Linux pages materialize when they are touched. On Windows the memory must be
   committed immediately and in full: a region that is only reserved would make the system
   functions that write into it fail (a ReadFile into a game buffer). That is 4 GB of "commit",
   which take up no RAM until they are used but must fit within the limit of RAM plus page file. */
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
        c->eax = 1; /* highest leaf */
        c->ebx = 0x756e6547; /* "GenuineIntel" */
        c->edx = 0x49656e69;
        c->ecx = 0x6c65746e;
    } else if (leaf == 1) {
        c->eax = 0x000006f6; /* family 6: Core 2 */
        /* FPU, TSC, CX8, CMOV, MMX, FXSR, SSE, SSE2 */
        c->edx = (1u << 0) | (1u << 4) | (1u << 8) | (1u << 15) | (1u << 23) | (1u << 24) | (1u << 25) | (1u << 26);
        c->ecx = 1u << 0; /* SSE3 */
    }
}
