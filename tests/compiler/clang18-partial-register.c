/* Riproduzione di una miscompilazione di clang 18.1.3 (Ubuntu 24.04), trovata il 2026-11-14
 * dal test differenziale sulla funzione 0x0099BC40.
 *
 *   x = (x & 0xffffff00) | (u8)mem[x];      con x appena caricato dalla memoria
 *
 * Da -O1 in su clang scrive in x il solo byte, esteso con zeri, e perde i 24 bit alti.
 * Risultato atteso: 0x133. clang -O0 e gcc -O2: 0x133. clang -O1/-O2: 0x33.
 *
 *   clang -O2 tests/compiler/clang18-partial-register.c -o /tmp/t && /tmp/t
 *
 * Per questo il codice generato scrive i registri a 8 e 16 bit con le macro SET8L, SET8H e
 * SET16 di runtime/altair.h (scritture strette dirette) invece che con maschera e OR.
 */
#include <stdint.h>
#include <stdio.h>
#include <string.h>
typedef uint8_t u8; typedef uint32_t u32;
typedef struct { u32 eax, ecx, edx, esp; u8 *mem; } Ctx;
static inline u8 rd8(Ctx *c, u32 a) { return c->mem[a]; }
static inline u32 rd32(Ctx *c, u32 a) { u32 v; memcpy(&v, c->mem + a, 4); return v; }
__attribute__((noinline)) void f(Ctx *c) {
    u32 zf = 0;
    c->eax = (c->eax & 0xffffff00u) | (u8)(rd8(c, (u32)(c->esp + 0x10u)));
    { u8 a = (u8)c->ecx, b = (u8)c->ecx, r; r = a ^ b; zf = r == 0; c->ecx = (c->ecx & 0xffffff00u) | (u8)(r); }
    { u8 a = (u8)c->eax, b = (u8)c->eax, r; r = a & b; zf = r == 0; }
    if (!zf) goto L;
    c->ecx = rd32(c, (u32)(c->esp + 0x8u));
    c->ecx = (c->ecx & 0xffffff00u) | (u8)(rd8(c, (u32)(c->ecx)));
L:;
    c->edx = rd32(c, (u32)(c->esp + 0x4u));
}
int main(void) {
    static u8 mem[4096];
    u32 v = 0x00000100; memcpy(mem + 8, &v, 4); mem[0x100] = 0x33;
    Ctx c = {0, 0x12345678, 0, 0, mem};
    f(&c);
    printf("%#x %s\n", c.ecx, c.ecx == 0x133 ? "ok" : "MISCOMPILATO");
    return c.ecx != 0x133;
}
