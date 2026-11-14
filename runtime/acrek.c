/* Runtime minimo per il codice generato: registro delle funzioni, trap, memoria guest. */
#include "acrek.h"

#include <setjmp.h>
#include <sys/mman.h>

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

int acrek_run(Ctx *c, u32 addr) {
    jmp_buf jb;
    void *outer = c->trap_jmp;
    c->trap_jmp = &jb;
    c->trap_msg = 0;
    int trapped = setjmp(jb);
    if (!trapped)
        acrek_call_indirect(c, addr);
    c->trap_jmp = outer;
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
