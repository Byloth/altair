//! COM objects: a vtable in guest memory whose slots are trampolines to host functions.
//!
//! An object is `[vtable pointer][reference count][data…]`. Every interface
//! declares all its methods, in vtable order, with the number of arguments (`this`
//! included): those without an implementation stop the game, saying which method is needed.

use crate::win32::{register_host_fn, Call, Runtime, Shim};

pub type Method = (&'static str, u32, Option<Shim>);

pub const E_NOINTERFACE: u32 = 0x8000_4002;

/// Creates an object of interface `iface` with `extra` bytes of data after the header.
pub fn create(rt: &mut Runtime, iface: &'static str, methods: &[Method], extra: u32) -> u32 {
    let vtable = match rt.com_vtables.get(iface) {
        Some(&v) => v,
        None => {
            let v = rt.halloc(4 * methods.len() as u32, true);
            for (i, (name, nargs, shim)) in methods.iter().enumerate() {
                rt.mem.w32(v + 4 * i as u32, register_host_fn(format!("{iface}::{name}"), *nargs, *shim));
            }
            rt.com_vtables.insert(iface, v);
            v
        }
    };
    let obj = rt.halloc(8 + extra, true);
    rt.mem.w32(obj, vtable);
    rt.mem.w32(obj + 4, 1);
    obj
}

pub fn guid(rt: &Runtime, p: u32) -> String {
    let b = rt.mem.bytes(p, 16);
    format!(
        "{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{}",
        rt.mem.r32(p),
        rt.mem.r16(p + 4),
        rt.mem.r16(p + 6),
        b[8],
        b[9],
        b[10..].iter().map(|x| format!("{x:02X}")).collect::<String>()
    )
}

pub fn add_ref(rt: &mut Runtime, c: &mut Call) -> u32 {
    let n = rt.mem.r32(c.arg(0) + 4) + 1;
    rt.mem.w32(c.arg(0) + 4, n);
    n
}

/// Objects are never freed: the count only serves to answer the caller.
pub fn release(rt: &mut Runtime, c: &mut Call) -> u32 {
    let n = rt.mem.r32(c.arg(0) + 4).saturating_sub(1);
    rt.mem.w32(c.arg(0) + 4, n);
    n
}

/// `QueryInterface` of an object that offers no other interfaces.
pub fn no_interface(rt: &mut Runtime, c: &mut Call) -> u32 {
    eprintln!("[com] QueryInterface {} not offered (from {:#010x})", guid(rt, c.arg(1)), c.ret_addr());
    rt.mem.w32(c.arg(2), 0);
    E_NOINTERFACE
}

pub fn ok(_: &mut Runtime, _: &mut Call) -> u32 {
    0
}
