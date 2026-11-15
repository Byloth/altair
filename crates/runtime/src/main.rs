//! `altair-run`: carica l'immagine del gioco, la collega alla libreria tradotta e la esegue.

mod fs;
mod machine;
mod sync;
mod system;
mod user32;
mod win32;

use altair_pe::{Image, AC1_DX9_STEAM_SHA1};
use machine::{Ctx, Mem};
use std::ffi::{CStr, CString};
use std::process::ExitCode;

const USAGE: &str = "uso: altair-run <AssassinsCreed_Dx9.exe> --lib <libaltair_ac1-*.so> [--trace]

Variabili: ALTAIR_TRACE=1 registra ogni chiamata a un import.";

/// Layout dello spazio guest (l'immagine sta a 0x400000).
pub const STACK_TOP: u32 = 0x003f_0000;
pub const STACK_SIZE: u32 = 0x002f_0000;
pub const HEAP_START: u32 = 0x1000_0000;
pub const HEAP_END: u32 = 0x4000_0000;
pub const VIRTUAL_START: u32 = 0x4000_0000;
pub const VIRTUAL_END: u32 = 0x7000_0000;
pub const HOST_BASE: u32 = 0x7f00_0000;
pub const TEB: u32 = 0x7ffd_0000;
pub const PEB: u32 = 0x7ffd_f000;

type RunFn = unsafe extern "C" fn(*mut Ctx, u32) -> i32;
type MemCreateFn = unsafe extern "C" fn() -> *mut u8;

fn sym<T: Copy>(lib: *mut libc::c_void, name: &str) -> Result<T, String> {
    let c = CString::new(name).unwrap();
    // SAFETY: `lib` è un handle valido di dlopen; il tipo T è quello dichiarato in altair.h.
    let p = unsafe { libc::dlsym(lib, c.as_ptr()) };
    if p.is_null() {
        return Err(format!("simbolo {name} non trovato nella libreria"));
    }
    Ok(unsafe { std::mem::transmute_copy(&p) })
}

fn run() -> Result<i32, String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let exe = args.first().ok_or(USAGE)?;
    let lib_path = args.iter().position(|a| a == "--lib").and_then(|i| args.get(i + 1)).ok_or(USAGE)?;
    let trace = args.iter().any(|a| a == "--trace") || std::env::var_os("ALTAIR_TRACE").is_some();

    let img = Image::load(exe).map_err(|e| e.to_string())?;
    if img.sha1() != AC1_DX9_STEAM_SHA1 {
        return Err("l'eseguibile non è AssassinsCreed_Dx9.exe della build Steam".into());
    }
    let c_lib = CString::new(lib_path.as_str()).unwrap();
    // SAFETY: carica la libreria generata da `altair image`.
    let lib = unsafe { libc::dlopen(c_lib.as_ptr(), libc::RTLD_NOW) };
    if lib.is_null() {
        let err = unsafe { CStr::from_ptr(libc::dlerror()) }.to_string_lossy().into_owned();
        return Err(format!("dlopen: {err}"));
    }
    let altair_run: RunFn = sym(lib, "altair_run")?;
    let mem_create: MemCreateFn = sym(lib, "altair_mem_create")?;

    let base = unsafe { mem_create() };
    if base.is_null() {
        return Err("impossibile riservare lo spazio di indirizzamento guest".into());
    }
    let mem = Mem(base);
    for s in &img.sections {
        mem.write(s.va, img.section_bytes(s));
    }

    // TEB, PEB e TLS statica del thread principale.
    mem.w32(TEB, 0xffff_ffff); // fine della catena SEH
    mem.w32(TEB + 0x04, STACK_TOP);
    mem.w32(TEB + 0x08, STACK_TOP - STACK_SIZE);
    mem.w32(TEB + 0x18, TEB);
    mem.w32(TEB + 0x20, 0x800); // id del processo
    mem.w32(TEB + 0x24, 0x1000); // id del thread
    mem.w32(TEB + 0x30, PEB);
    mem.w32(PEB + 0x08, img.base);

    let mut rt = win32::Runtime::new(mem, &img, exe, trace);
    rt.setup_static_tls(&img, TEB);
    win32::install(rt, &img, altair_run);

    let mut ctx = Ctx::new(base);
    ctx.fs_base = TEB;
    ctx.esp = STACK_TOP - 0x100;
    ctx.host_call = Some(win32::host_call);
    mem.w32(ctx.esp, 0); // indirizzo di ritorno dell'entry point: non ci torna mai
    let entry = img.entry;

    // I frame del codice tradotto sono profondi: serve uno stack host grande.
    struct SendPtr(*mut Ctx);
    unsafe impl Send for SendPtr {}
    let p = SendPtr(&mut ctx);
    let handle = std::thread::Builder::new()
        .name("guest-main".into())
        .stack_size(1 << 30)
        .spawn(move || {
            let p = p;
            // SAFETY: il contesto vive nello stack di `run`, che aspetta la fine del thread.
            unsafe { altair_run(p.0, entry) }
        })
        .map_err(|e| e.to_string())?;
    let trapped = handle.join().map_err(|_| "il thread guest è andato in panic".to_string())?;
    if trapped != 0 {
        eprintln!("\nTRAP: {}", trap_text(&ctx));
        win32::report_stop(&ctx);
        return Ok(2);
    }
    eprintln!("l'entry point è ritornato (eax = {:#x})", ctx.eax);
    Ok(0)
}

pub fn trap_text(ctx: &Ctx) -> String {
    let msg = if ctx.trap_msg.is_null() { "?".into() } else { unsafe { CStr::from_ptr(ctx.trap_msg) }.to_string_lossy() };
    format!("{msg} ({:#010x})", ctx.trap_addr)
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code as u8),
        Err(e) => {
            eprintln!("errore: {e}");
            ExitCode::FAILURE
        }
    }
}
