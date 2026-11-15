//! `altair-run`: carica l'immagine del gioco, la collega alla libreria tradotta e la esegue.

mod bink;
mod com;
mod d3d9;
mod dsound;
mod fs;
mod animus_font;
mod gfx;
mod hooks;
mod overlay;
mod host;
mod input;
mod machine;
#[cfg(unix)]
mod profile;
/// Il profilatore a campionamento usa i segnali: esiste solo su Linux.
#[cfg(not(unix))]
mod profile {
    pub fn watch_current_thread() {}
    pub fn reset() {}
    pub fn report() {}
}
mod sync;
mod system;
mod user32;
mod win32;
mod window;

use altair_pe::{Image, AC1_DX9_STEAM_SHA1};
use machine::{Ctx, Mem};
use std::ffi::CStr;
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

fn sym<T: Copy>(lib: &host::Library, name: &str) -> Result<T, String> {
    let p = lib.symbol(name);
    if p.is_null() {
        return Err(format!("simbolo {name} non trovato nella libreria"));
    }
    // SAFETY: il tipo T è quello dichiarato in altair.h per quel simbolo.
    Ok(unsafe { std::mem::transmute_copy(&p) })
}

fn run() -> Result<i32, String> {
    host::fine_timers();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let exe = args.first().ok_or(USAGE)?;
    if let Some(dir) = std::path::Path::new(exe).parent() {
        animus_font::set_game_dir(dir);
    }
    let lib_path = args.iter().position(|a| a == "--lib").and_then(|i| args.get(i + 1)).ok_or(USAGE)?;
    let trace = args.iter().any(|a| a == "--trace") || std::env::var_os("ALTAIR_TRACE").is_some();

    let img = Image::load(exe).map_err(|e| e.to_string())?;
    if img.sha1() != AC1_DX9_STEAM_SHA1 {
        return Err("l'eseguibile non è AssassinsCreed_Dx9.exe della build Steam".into());
    }
    // La libreria generata da `altair image`: un `.so` su Linux, una DLL su Windows.
    let lib = &host::Library::open(lib_path)?;
    let altair_run: RunFn = sym(lib, "altair_run")?;
    let mem_create: MemCreateFn = sym(lib, "altair_mem_create")?;
    let trail: win32::TrailFn = sym(lib, "altair_trail")?;
    let call: win32::CallFn = sym(lib, "altair_call_indirect")?;
    // Agganci (hooks.rs): una libreria tradotta più vecchia può non averli.
    type SetHookFn = unsafe extern "C" fn(unsafe extern "C" fn(*mut machine::Ctx, u32) -> i32);
    match sym::<SetHookFn>(lib, "altair_set_hook") {
        Ok(set_hook) => unsafe { set_hook(hooks::dispatch) },
        Err(_) => eprintln!("[agganci] la libreria tradotta non li prevede: va rigenerata (tools/build-image.sh)"),
    }

    let base = unsafe { mem_create() };
    if base.is_null() {
        return Err("impossibile riservare lo spazio di indirizzamento guest".into());
    }
    let mem = Mem(base);
    guard_guest_memory(base);
    for s in &img.sections {
        mem.write(s.va, img.section_bytes(s));
    }

    // I filmati iniziali sono descritti da una tabella nei dati del gioco (`StartupVideoManager`,
    // voci da 36 byte da 0x18C2C08): nome, "si può saltare", dissolvenze, durata. L'originale
    // lascia saltare solo DemoIntro; qui il permesso va anche a logo, "presenta" e avvertenze,
    // così li salta il gioco stesso, con la sua dissolvenza. `ALTAIR_INTRO_SKIP=0` lo toglie.
    if img.sha1() == AC1_DX9_STEAM_SHA1 && std::env::var("ALTAIR_INTRO_SKIP").map(|v| v != "0").unwrap_or(true) {
        for entry in 1..=3 {
            mem.w8(0x18C_2C08 + 36 * entry + 4, 1);
        }
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
    hooks::set_scratch(rt.halloc(hooks::SCRATCH_SIZE, true));
    rt.setup_static_tls(&img, TEB);
    win32::install(rt, &img, altair_run, trail, call);

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
            let trapped = unsafe { altair_run(p.0, entry) };
            if trapped != 0 {
                let ctx = unsafe { &*p.0 };
                eprintln!("\nTRAP: {}", trap_text(ctx));
                win32::report_stop(ctx);
            }
            trapped
        })
        .map_err(|e| e.to_string())?;
    if window::enabled() {
        // Il thread principale passa a winit; la fine del gioco chiude il processo da qui.
        std::thread::spawn(move || {
            let code = match handle.join() {
                Ok(0) => 0,
                _ => 2,
            };
            std::process::exit(code);
        });
        window::run();
    }
    let trapped = handle.join().map_err(|_| "il thread guest è andato in panic".to_string())?;
    if trapped != 0 {
        return Ok(2);
    }
    eprintln!("l'entry point è ritornato (eax = {:#x})", ctx.eax);
    Ok(0)
}

static GUEST_BASE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Protegge i primi 64 KB dello spazio guest (in Windows non sono mai mappati: lì cadono i
/// puntatori nulli) e installa un gestore che, a un accesso guest non valido, dice dove è
/// successo invece di lasciare un core dump muto. `ALTAIR_NULL=0` toglie la protezione.
fn guard_guest_memory(base: *mut u8) {
    GUEST_BASE.store(base as usize, std::sync::atomic::Ordering::Relaxed);
    host::guard(base, !std::env::var("ALTAIR_NULL").is_ok_and(|v| v == "0"), report_fault);
}

/// Stampa ciò che si sa di un accesso non valido; dopo, il processo esce con codice 3.
fn report_fault(addr: usize) -> bool {
    let base = GUEST_BASE.load(std::sync::atomic::Ordering::Relaxed);
    let guest = base != 0 && (base..base + (1usize << 32)).contains(&addr);
    if guest {
        let guest = (addr - base) as u32;
        let thread = std::thread::current();
        eprintln!("\nACCESSO NON VALIDO alla memoria guest {guest:#010x}{} (thread {})", if guest < 0x1_0000 { ": puntatore nullo" } else { "" }, thread.name().unwrap_or("?"));
        eprintln!("  ultime funzioni guest chiamate per indirizzo, dalla più recente: {}", win32::trail_text());
    } else {
        eprintln!("\nACCESSO NON VALIDO fuori dalla memoria guest, all'indirizzo host {addr:#x}");
    }
    guest
}

pub fn trap_text(ctx: &Ctx) -> String {
    let msg = if ctx.trap_msg.is_null() { "?".into() } else { unsafe { CStr::from_ptr(ctx.trap_msg) }.to_string_lossy() };
    format!("{msg} ({:#010x})", ctx.trap_addr)
}

fn main() -> ExitCode {
    overlay::init();
    match run() {
        Ok(code) => ExitCode::from(code as u8),
        Err(e) => {
            eprintln!("errore: {e}");
            ExitCode::FAILURE
        }
    }
}
