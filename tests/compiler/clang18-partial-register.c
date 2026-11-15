/* Reproduction of a miscompilation by clang 18.1.3 (Ubuntu 24.04), found on 2026-11-14
 * by the differential test on function 0x0099BC40.
 *
 *   x = (x & 0xffffff00) | (u8)mem[x];      with x just loaded from memory
 *
 * From -O1 upwards clang writes only the byte into x, zero-extended, and loses the upper 24 bits.
 * Expected result: 0x133. clang -O0 and gcc -O2: 0x133. clang -O1/-O2: 0x33.
 *
 *   clang -O2 tests/compiler/clang18-partial-register.c -o /tmp/t && /tmp/t
 *
 * This is why the generated code writes the 8- and 16-bit registers with the SET8L, SET8H and
 * SET16 macros of runtime/altair.h (direct narrow writes) instead of with mask and OR.
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
    printf("%#x %s\n", c.ecx, c.ecx == 0x133 ? "ok" : "MISCOMPILED");
    return c.ecx != 0x133;
}
