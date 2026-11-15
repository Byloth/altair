//! The state of the guest machine as seen by the runtime: CPU context, memory, heap.

use std::collections::HashMap;

/// An 80-bit x87 register, opaque to the runtime (in C it is `long double`).
#[repr(C, align(16))]
#[derive(Clone, Copy)]
pub struct F80(pub [u8; 16]);

/// Must match `struct Ctx` in `runtime/altair.h` field by field.
#[repr(C)]
pub struct Ctx {
    pub eax: u32,
    pub ecx: u32,
    pub edx: u32,
    pub ebx: u32,
    pub esp: u32,
    pub ebp: u32,
    pub esi: u32,
    pub edi: u32,
    pub df: u32,
    pub fs_base: u32,
    pub mem: *mut u8,
    pub host_call: Option<unsafe extern "C" fn(*mut Ctx, u32) -> i32>,
    pub trap_jmp: *mut std::ffi::c_void,
    pub trap_msg: *const std::ffi::c_char,
    pub trap_addr: u32,
    pub mxcsr: u32,
    pub xmm: [[u64; 2]; 8],
    pub fpr: [F80; 8],
    pub fptop: u32,
    pub fpcw: u32,
    pub fpsw: u32,
    pub eflags_id: u32,
}

const _: () = assert!(std::mem::size_of::<Ctx>() == 352);

impl Ctx {
    pub fn new(mem: *mut u8) -> Ctx {
        // SAFETY: all fields are integers, pointers or Option of function pointer: zero is valid.
        let mut c: Ctx = unsafe { std::mem::zeroed() };
        c.mem = mem;
        c.mxcsr = 0x1f80;
        c.fpcw = 0x027f;
        c
    }
}

/// Access to guest memory. The 4 GB space is reserved in full, so every 32-bit address
/// is a valid host pointer.
#[derive(Clone, Copy)]
pub struct Mem(pub *mut u8);

// SAFETY: guest memory is shared between the guest threads as it was in the original process.
unsafe impl Send for Mem {}
unsafe impl Sync for Mem {}

impl Mem {
    pub fn ptr(self, a: u32) -> *mut u8 {
        // SAFETY: see the comment on the type.
        unsafe { self.0.add(a as usize) }
    }
    pub fn r8(self, a: u32) -> u8 {
        unsafe { *self.ptr(a) }
    }
    pub fn r16(self, a: u32) -> u16 {
        unsafe { (self.ptr(a) as *const u16).read_unaligned() }
    }
    pub fn r32(self, a: u32) -> u32 {
        unsafe { (self.ptr(a) as *const u32).read_unaligned() }
    }
    pub fn w8(self, a: u32, v: u8) {
        unsafe { *self.ptr(a) = v }
    }
    pub fn w16(self, a: u32, v: u16) {
        unsafe { (self.ptr(a) as *mut u16).write_unaligned(v) }
    }
    pub fn w32(self, a: u32, v: u32) {
        unsafe { (self.ptr(a) as *mut u32).write_unaligned(v) }
    }
    pub fn w64(self, a: u32, v: u64) {
        unsafe { (self.ptr(a) as *mut u64).write_unaligned(v) }
    }
    pub fn bytes<'a>(self, a: u32, len: u32) -> &'a [u8] {
        unsafe { std::slice::from_raw_parts(self.ptr(a), len as usize) }
    }
    pub fn write(self, a: u32, data: &[u8]) {
        unsafe { std::ptr::copy_nonoverlapping(data.as_ptr(), self.ptr(a), data.len()) }
    }
    pub fn fill(self, a: u32, len: u32, v: u8) {
        unsafe { std::ptr::write_bytes(self.ptr(a), v, len as usize) }
    }
    /// Zero-terminated string, at most 64 KB.
    pub fn cstr_bytes<'a>(self, a: u32) -> &'a [u8] {
        let mut n = 0;
        while n < 0x10000 && self.r8(a + n) != 0 {
            n += 1;
        }
        self.bytes(a, n)
    }
    pub fn cstr(self, a: u32) -> String {
        if a == 0 {
            return String::new();
        }
        self.cstr_bytes(a).iter().map(|&b| b as char).collect()
    }
    pub fn wstr(self, a: u32) -> String {
        let mut s = String::new();
        let mut p = a;
        while a != 0 && s.len() < 0x8000 {
            let ch = self.r16(p);
            if ch == 0 {
                break;
            }
            s.push(char::from_u32(ch as u32).unwrap_or('?'));
            p += 2;
        }
        s
    }
    pub fn write_cstr(self, a: u32, s: &[u8]) {
        self.write(a, s);
        self.w8(a + s.len() as u32, 0);
    }
}

/// Guest heap allocator: blocks aligned to 16, free lists per size.
/// The metadata lives on the host side, so an overflow in the game does not corrupt it.
pub struct Heap {
    next: u32,
    end: u32,
    free: HashMap<u32, Vec<u32>>,
    /// address -> (requested size, block size)
    live: HashMap<u32, (u32, u32)>,
    pub bytes_live: u64,
}

impl Heap {
    pub fn new(start: u32, end: u32) -> Heap {
        Heap { next: start, end, free: HashMap::new(), live: HashMap::new(), bytes_live: 0 }
    }

    fn class(size: u32) -> u32 {
        let s = size.max(1);
        if s <= 4096 {
            (s + 15) & !15
        } else {
            s.next_power_of_two().min((s + 0xffff) & !0xffff)
        }
    }

    /// Returns (address, the block is recycled and so may contain old data).
    pub fn alloc(&mut self, size: u32) -> Option<(u32, bool)> {
        let class = Heap::class(size);
        let (addr, reused) = match self.free.get_mut(&class).and_then(Vec::pop) {
            Some(a) => (a, true),
            None => {
                if self.end - self.next < class {
                    return None;
                }
                let a = self.next;
                self.next += class;
                (a, false)
            }
        };
        self.live.insert(addr, (size, class));
        self.bytes_live += class as u64;
        Some((addr, reused))
    }

    pub fn free(&mut self, addr: u32) -> bool {
        match self.live.remove(&addr) {
            Some((_, class)) => {
                self.free.entry(class).or_default().push(addr);
                self.bytes_live -= class as u64;
                true
            }
            None => false,
        }
    }

    pub fn size(&self, addr: u32) -> Option<u32> {
        self.live.get(&addr).map(|b| b.0)
    }

    pub fn capacity(&self, addr: u32) -> Option<u32> {
        self.live.get(&addr).map(|b| b.1)
    }

    pub fn resize_in_place(&mut self, addr: u32, size: u32) {
        if let Some(b) = self.live.get_mut(&addr) {
            b.0 = size;
        }
    }
}
