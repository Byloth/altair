//! Le funzioni di Windows importate dal gioco, implementate sull'host.
//!
//! Ogni cella della IAT punta a un trampolino in `HOST_BASE`. Quando il codice tradotto chiama
//! un indirizzo che non è una funzione tradotta, la libreria chiama `host_call`, che trova lo
//! shim con quel nome. Le API sono `stdcall`: lo shim legge gli argomenti dallo stack guest e
//! al ritorno il runtime toglie indirizzo di ritorno e argomenti.

use crate::fs;
use crate::machine::{Ctx, Heap, Mem};
use crate::sync::{Kind, Object};
use crate::{HEAP_END, HEAP_START, HOST_BASE, TEB, VIRTUAL_END, VIRTUAL_START};
use altair_pe::Image;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, RwLock};
use std::time::Instant;

const TLS_SLOTS: u32 = 0xe10; // TEB.TlsSlots
const LAST_ERROR: u32 = 0x34; // TEB.LastErrorValue
const PROCESS_HEAP: u32 = 0x0015_0000;
const GUEST_EXE: &[u8] = b"C:\\AC1\\AssassinsCreed_Dx9.exe";

pub struct Runtime {
    pub mem: Mem,
    pub trace: bool,
    /// indirizzo del trampolino -> (dll, nome)
    #[allow(dead_code)]
    imports: HashMap<u32, (String, String)>,
    heap: Heap,
    virtual_next: u32,
    /// Zone ottenute con VirtualAlloc: base -> dimensione.
    regions: std::collections::BTreeMap<u32, u32>,
    tls_next: u32,
    next_handle: u32,
    objects: HashMap<u32, Arc<Object>>,
    next_tid: u32,
    static_tls: Option<(u32, u32, u32, u32)>,
    pub fs: fs::FsState,
    /// Procedura della finestra registrata dal gioco.
    pub wndproc: u32,
    pub com_vtables: HashMap<&'static str, u32>,
    pub d3d: crate::d3d9::D3d,
    pub gfx: Option<crate::gfx::Gfx>,
    /// Oggetto DirectInput della tastiera e copione dei tasti simulati: (da, a, codice DIK).
    pub keyboard: u32,
    /// Cartella del canale di comando (`ALTAIR_CTL`).
    pub ctl_dir: Option<std::path::PathBuf>,
    pub key_script: Vec<(u64, u64, u8)>,
    /// Coda dei messaggi della finestra: (messaggio, wParam, lParam).
    pub messages: std::collections::VecDeque<(u32, u32, u32)>,
    pub window_thread: u32,
    pub text_script: Vec<(u64, char)>,
    pub text_frame: u64,
    /// Caratteri battuti dal copione, in attesa che il gioco li chieda con ToUnicode.
    pub typed: std::collections::VecDeque<(u32, char)>,
    /// Il mouse di DirectInput.
    pub mouse: u32,
    /// Ultimo stato stampato da `ALTAIR_INPUT_LOG`: tasti premuti; pulsanti e se il mouse si muove.
    pub input_seen: (Vec<u32>, ([bool; 8], bool)),
    /// Pulsanti del mouse premuti dal canale di comando: (fotogramma del rilascio, pulsante).
    pub button_script: Vec<(u64, usize)>,
    /// Tasti e pulsanti premuti dal canale di comando per un tempo reale: (rilascio, codice).
    pub key_timed: Vec<(std::time::Instant, u8)>,
    pub button_timed: Vec<(std::time::Instant, usize)>,
    start: Instant,
    cmdline: u32,
    env_a: u32,
    env_w: u32,
}

/// Una chiamata in corso: argomenti sullo stack guest sopra l'indirizzo di ritorno.
pub struct Call<'a> {
    pub ctx: &'a mut Ctx,
    pub mem: Mem,
}

impl Call<'_> {
    pub fn arg(&self, i: u32) -> u32 {
        self.mem.r32(self.ctx.esp + 4 + 4 * i)
    }
    pub fn ret_addr(&self) -> u32 {
        self.mem.r32(self.ctx.esp)
    }
    pub fn teb(&self) -> u32 {
        self.ctx.fs_base
    }
    pub fn set_last_error(&self, e: u32) {
        self.mem.w32(self.teb() + LAST_ERROR, e);
    }
}

/// Uno shim. `L` riceve il runtime già bloccato; `F` non lo tocca (percorsi caldi) oppure lo
/// blocca da sé per poco, perché può restare in attesa o richiamare codice guest.
#[derive(Clone, Copy)]
pub enum Shim {
    L(fn(&mut Runtime, &mut Call) -> u32),
    F(fn(&mut Call) -> u32),
}
use Shim::{F, L};

type RunFn = unsafe extern "C" fn(*mut Ctx, u32) -> i32;

static RT: OnceLock<Mutex<Runtime>> = OnceLock::new();
/// Per indice di trampolino: (nome, numero di argomenti, shim), o None se manca.
/// Una funzione host raggiungibile dal codice tradotto: un import o un metodo COM.
/// Le voci vivono per tutto il processo: così `host_call`, che gira centinaia di migliaia di
/// volte per fotogramma, le trova senza copiare stringhe né prendere lock per contarle.
pub struct Entry {
    pub name: String,
    shim: Option<(u32, Shim)>,
    count: std::sync::atomic::AtomicU64,
}
fn entry(name: String, shim: Option<(u32, Shim)>) -> &'static Entry {
    Box::leak(Box::new(Entry { name, shim, count: Default::default() }))
}
static TABLE: OnceLock<Vec<&'static Entry>> = OnceLock::new();

/// Quante volte è stata chiamata ogni funzione host (solo quelle chiamate almeno una volta).
pub fn call_counts() -> Vec<(String, u64)> {
    let dynamic = DYN.read().unwrap();
    TABLE.get().into_iter().flatten().chain(dynamic.iter()).map(|e| (e.name.clone(), e.count.load(Ordering::Relaxed))).filter(|(_, n)| *n > 0).collect()
}
static RUN: OnceLock<RunFn> = OnceLock::new();
pub type TrailFn = unsafe extern "C" fn(*mut u32, i32) -> i32;
static TRAIL: OnceLock<TrailFn> = OnceLock::new();
pub type CallFn = unsafe extern "C" fn(*mut Ctx, u32);
static CALL: OnceLock<CallFn> = OnceLock::new();
static TRACE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Funzioni host registrate a runtime (i metodi degli oggetti COM), dopo i trampolini degli import.
const DYN_BASE: u32 = HOST_BASE + 0x10_0000;
static DYN: RwLock<Vec<&'static Entry>> = RwLock::new(Vec::new());

/// Registra una funzione host e ne ritorna l'indirizzo guest. `shim` None = non implementata.
pub fn register_host_fn(name: String, nargs: u32, shim: Option<Shim>) -> u32 {
    let mut d = DYN.write().unwrap();
    d.push(entry(name, shim.map(|s| (nargs, s))));
    DYN_BASE + 16 * (d.len() as u32 - 1)
}

/// (nome, argomenti e shim se implementata) per un indirizzo host.
fn lookup(addr: u32) -> Option<&'static Entry> {
    if addr % 16 != 0 || addr < HOST_BASE {
        return None;
    }
    if addr >= DYN_BASE {
        return DYN.read().unwrap().get(((addr - DYN_BASE) / 16) as usize).copied();
    }
    TABLE.get()?.get(((addr - HOST_BASE) / 16) as usize).copied()
}

/// Accesso breve al runtime per gli shim di tipo `F`.
pub fn with_runtime<T>(f: impl FnOnce(&mut Runtime) -> T) -> T {
    f(&mut rt())
}

fn rt() -> MutexGuard<'static, Runtime> {
    RT.get().expect("runtime non installato").lock().unwrap_or_else(|e| e.into_inner())
}

pub fn install(rt: Runtime, img: &Image, run: RunFn, trail: TrailFn, call: CallFn) {
    let _ = TRAIL.set(trail);
    let _ = CALL.set(call);
    let table = img
        .imports
        .iter()
        .map(|imp| {
            let key = if imp.name.starts_with('#') { format!("{}!{}", imp.dll, imp.name) } else { imp.name.clone() };
            let shim = SHIMS.iter().chain(crate::system::SHIMS).chain(crate::bink::SHIMS).chain(crate::user32::SHIMS).chain(crate::input::SHIMS).chain(crate::d3d9::SHIMS).chain(EXTRA).find(|s| s.0 == key).map(|s| (s.1, s.2));
            entry(format!("{}!{}", imp.dll, imp.name), shim)
        })
        .collect();
    TRACE.store(rt.trace, Ordering::Relaxed);
    let _ = TABLE.set(table);
    let _ = RUN.set(run);
    let _ = RT.set(Mutex::new(rt));
}

impl Runtime {
    pub fn new(mem: Mem, img: &Image, exe: &str, trace: bool) -> Runtime {
        let mut imports = HashMap::new();
        for (i, imp) in img.imports.iter().enumerate() {
            let tramp = HOST_BASE + 16 * i as u32;
            mem.w32(imp.iat, tramp);
            imports.insert(tramp, (imp.dll.clone(), imp.name.clone()));
        }
        let mut rt = Runtime {
            mem,
            trace,
            imports,
            heap: Heap::new(HEAP_START, HEAP_END),
            virtual_next: VIRTUAL_START,
            regions: std::collections::BTreeMap::new(),
            tls_next: 1,
            next_handle: 0x100,
            objects: HashMap::new(),
            next_tid: 0x1004,
            static_tls: None,
            wndproc: 0,
            com_vtables: HashMap::new(),
            d3d: Default::default(),
            gfx: None,
            keyboard: 0,
            ctl_dir: std::env::var_os("ALTAIR_CTL").map(Into::into),
            key_script: Vec::new(),
            messages: Default::default(),
            window_thread: 0,
            text_script: Vec::new(),
            text_frame: 0,
            typed: Default::default(),
            mouse: 0,
            input_seen: Default::default(),
            button_script: Vec::new(),
            key_timed: Vec::new(),
            button_timed: Vec::new(),
            fs: fs::FsState::new(
                std::path::Path::new(exe).parent().map(Into::into).unwrap_or_default(),
                std::env::current_dir().unwrap_or_default().join("run").join("c"),
            ),
            start: Instant::now(),
            cmdline: 0,
            env_a: 0,
            env_w: 0,
        };
        let mut cmd = vec![b'"'];
        cmd.extend_from_slice(GUEST_EXE);
        cmd.push(b'"');
        rt.cmdline = rt.alloc_bytes(&cmd, 1);
        rt.env_a = rt.alloc_bytes(b"\0", 1);
        rt.env_w = rt.alloc_bytes(&[0, 0], 2);
        rt
    }

    /// Copia `data` nello heap guest, seguito da `zeros` byte a zero.
    fn alloc_bytes(&mut self, data: &[u8], zeros: u32) -> u32 {
        let a = self.halloc(data.len() as u32 + zeros, true);
        self.mem.write(a, data);
        a
    }

    pub fn hfree(&mut self, addr: u32) {
        self.heap.free(addr);
    }

    pub fn halloc(&mut self, size: u32, zero: bool) -> u32 {
        match self.heap.alloc(size) {
            Some((a, reused)) => {
                if zero && reused {
                    self.mem.fill(a, self.heap.capacity(a).unwrap_or(size), 0);
                }
                a
            }
            None => 0,
        }
    }

    /// TLS statica (`__declspec(thread)`): un blocco per thread, raggiunto da fs:[0x2C].
    pub fn setup_static_tls(&mut self, img: &Image, teb: u32) {
        self.static_tls = img.tls_directory();
        if let Some((_, _, index_addr, _)) = self.static_tls {
            self.mem.w32(index_addr, 0);
        }
        self.setup_static_tls_from(self.static_tls, teb);
    }

    fn setup_static_tls_from(&mut self, dir: Option<(u32, u32, u32, u32)>, teb: u32) {
        let array = self.halloc(64, true);
        self.mem.w32(teb + 0x2c, array);
        let Some((start, end, _, zero_fill)) = dir else { return };
        let block = self.halloc(end - start + zero_fill, true);
        self.mem.write(block, self.mem.bytes(start, end - start));
        self.mem.w32(array, block);
    }
}


/// Import implementati in altri moduli.
static EXTRA: &[(&str, u32, Shim)] = &[
    ("EnumDisplayMonitors", 4, F(crate::user32::enum_display_monitors)),
    ("DispatchMessageA", 1, F(crate::user32::dispatch_message_a)),
   ("dsound.dll!#11", 3, L(crate::dsound::direct_sound_create8))];

/// Chiama una funzione guest `stdcall` da uno shim e ne ritorna `eax`. Lo shim deve essere di
/// tipo `F`: il lucchetto del runtime non va tenuto mentre gira codice guest.
pub fn call_guest(c: &mut Call, addr: u32, args: &[u32]) -> u32 {
    let saved = c.ctx.esp;
    // Sotto la cima dello stack, lasciando spazio a ciò che lo shim può averci messo.
    let mut sp = saved - 0x100;
    for a in args.iter().rev() {
        sp -= 4;
        c.mem.w32(sp, *a);
    }
    sp -= 4;
    c.mem.w32(sp, 0); // indirizzo di ritorno: la funzione C tradotta torna qui da sola
    c.ctx.esp = sp;
    // SAFETY: `addr` è una funzione guest; il contesto è quello del thread corrente.
    unsafe { (CALL.get().unwrap())(c.ctx, addr) };
    c.ctx.esp = saved;
    c.ctx.eax
}

/// Contatore di tempo economico (il TSC): `host_call` lo legge due volte per chiamata.
#[inline]
fn ticks() -> u64 {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        core::arch::x86_64::_rdtsc()
    }
    #[cfg(not(target_arch = "x86_64"))]
    0
}

/// Tempo passato negli shim dal thread corrente dall'ultima lettura, in millisecondi.
/// Serve a separare, in un fotogramma, il costo del runtime da quello del codice del gioco.
pub fn take_shim_ms() -> f64 {
    static PER_MS: OnceLock<f64> = OnceLock::new();
    let per_ms = *PER_MS.get_or_init(|| {
        let (t0, c0) = (std::time::Instant::now(), ticks());
        std::thread::sleep(std::time::Duration::from_millis(20));
        (ticks().wrapping_sub(c0) as f64 / (t0.elapsed().as_secs_f64() * 1000.0)).max(1.0)
    });
    SHIM_TICKS.with(|t| t.replace(0)) as f64 / per_ms
}

thread_local! {
    static SHIM_TICKS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    /// Impostato da `ExitThread`: la trap che segue è un'uscita normale del thread.
    static EXITING: std::cell::Cell<Option<u32>> = const { std::cell::Cell::new(None) };
}

/// Chiamata dal codice tradotto per un indirizzo che non è una funzione tradotta.
pub unsafe extern "C" fn host_call(ctx: *mut Ctx, addr: u32) -> i32 {
    let ctx = unsafe { &mut *ctx };
    let mem = Mem(ctx.mem);
    let Some(found) = lookup(addr) else {
        eprintln!("\nchiamata a {addr:#010x}: non è una funzione tradotta né un import");
        return 1;
    };
    let name = &found.name;
    let Some((nargs, shim)) = found.shim else {
        let ret = mem.r32(ctx.esp);
        let args: Vec<String> = (0..6).map(|i| format!("{:#x}", mem.r32(ctx.esp + 4 + 4 * i))).collect();
        let what = if addr >= DYN_BASE { "METODO COM MANCANTE" } else { "IMPORT MANCANTE" };
        eprintln!("\n{what}: {name}  (chiamato da {ret:#010x})\n  prime parole sullo stack: {}", args.join(" "));
        return 1;
    };
    let trace = TRACE.load(Ordering::Relaxed);
    let mut call = Call { ctx, mem };
    if trace {
        let args: Vec<String> = (0..nargs).map(|i| format!("{:#x}", call.arg(i))).collect();
        eprintln!("[t{:x} {:#010x}] {name}({})", mem.r32(call.teb() + 0x24), call.ret_addr(), args.join(", "));
    }
    found.count.fetch_add(1, Ordering::Relaxed);
    let start = ticks();
    let eax = match shim {
        L(f) => f(&mut rt(), &mut call),
        F(f) => f(&mut call),
    };
    SHIM_TICKS.with(|t| t.set(t.get() + ticks().wrapping_sub(start)));
    if EXITING.with(|e| e.get()).is_some() {
        return 1;
    }
    if trace {
        eprintln!("    {name} = {eax:#x}");
    }
    ctx.eax = eax;
    ctx.esp += 4 + 4 * nargs;
    0
}

/// Stampa lo stato al momento dell'arresto: registri e una traccia dello stack guest.
/// Le ultime destinazioni di chiamate indirette del thread corrente, dalla più recente.
pub fn trail_text() -> String {
    let mut trail = [0u32; 32];
    let n = TRAIL.get().map_or(0, |f| unsafe { f(trail.as_mut_ptr(), 32) });
    trail[..n as usize]
        .iter()
        .map(|&a| match lookup(a) {
            Some(e) => e.name.rsplit('!').next().unwrap_or(&e.name).to_string(),
            None => format!("{a:#010x}"),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn report_stop(ctx: &Ctx) {
    let rt = rt();
    let mem = rt.mem;
    eprintln!(
        "  eax={:08x} ecx={:08x} edx={:08x} ebx={:08x} esp={:08x} ebp={:08x} esi={:08x} edi={:08x}",
        ctx.eax, ctx.ecx, ctx.edx, ctx.ebx, ctx.esp, ctx.ebp, ctx.esi, ctx.edi
    );
    // Indirizzi di ritorno plausibili nelle prime parole dello stack.
    let mut frames = Vec::new();
    let mut p = ctx.esp;
    let end = ctx.esp.saturating_add(0x400);
    while frames.len() < 14 && p < end {
        let v = mem.r32(p);
        if (0x0040_1000..0x0167_c000).contains(&v) {
            frames.push(format!("{v:#010x}"));
        }
        p += 4;
    }
    eprintln!("  possibili indirizzi di ritorno sullo stack: {}", frames.join(" "));
    crate::d3d9::report(&rt);
    // Va chiamata dal thread che si è fermato: la traccia è per thread.
    let mut trail = [0u32; 32];
    let n = TRAIL.get().map_or(0, |f| unsafe { f(trail.as_mut_ptr(), 32) });
    let names: Vec<String> = trail[..n as usize]
        .iter()
        .map(|&a| match lookup(a) {
            Some(e) => e.name.rsplit('!').next().unwrap_or(&e.name).to_string(),
            None => format!("{a:#010x}"),
        })
        .collect();
    eprintln!("  ultime destinazioni indirette (dalla più recente): {}", names.join(" "));
    let mut calls = call_counts();
    calls.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    let total: u64 = calls.iter().map(|(_, n)| *n).sum();
    eprintln!("  import chiamati: {} distinti, {total} chiamate; heap vivo {:.1} MB", calls.len(), rt.heap.bytes_live as f64 / 1e6);
}

fn atomic(mem: Mem, a: u32) -> &'static AtomicU32 {
    // SAFETY: le API Interlocked richiedono un indirizzo allineato a 4, come AtomicU32.
    unsafe { &*(mem.ptr(a) as *const AtomicU32) }
}

fn ctype1(ch: u32) -> u16 {
    let c = match char::from_u32(ch) {
        Some(c) => c,
        None => return 0,
    };
    let mut f = 0u16;
    if c.is_uppercase() { f |= 0x001 }
    if c.is_lowercase() { f |= 0x002 }
    if c.is_ascii_digit() { f |= 0x004 }
    if c.is_whitespace() { f |= 0x008 }
    if c.is_ascii_punctuation() { f |= 0x010 }
    if c.is_control() { f |= 0x020 }
    if c == ' ' || c == '\t' { f |= 0x040 }
    if c.is_ascii_hexdigit() { f |= 0x080 }
    if c.is_alphabetic() { f |= 0x100 }
    f
}

fn map_case(ch: u32, flags: u32) -> u32 {
    let c = char::from_u32(ch).unwrap_or('?');
    let m = if flags & 0x100 != 0 {
        c.to_lowercase().next().unwrap_or(c)
    } else if flags & 0x200 != 0 {
        c.to_uppercase().next().unwrap_or(c)
    } else {
        c
    };
    m as u32
}

// --- kernel32: processo e ambiente ---------------------------------------------------------

fn get_version(_: &mut Runtime, _: &mut Call) -> u32 {
    0x0a28_0105 // Windows XP: 5.1, build 2600
}
fn get_version_ex_a(rt: &mut Runtime, c: &mut Call) -> u32 {
    let p = c.arg(0);
    rt.mem.w32(p + 4, 5);
    rt.mem.w32(p + 8, 1);
    rt.mem.w32(p + 12, 2600);
    rt.mem.w32(p + 16, 2); // VER_PLATFORM_WIN32_NT
    rt.mem.write_cstr(p + 20, b"Service Pack 3");
    1
}
fn get_command_line_a(rt: &mut Runtime, _: &mut Call) -> u32 {
    rt.cmdline
}
fn get_environment_strings(rt: &mut Runtime, _: &mut Call) -> u32 {
    rt.env_a
}
fn get_environment_strings_w(rt: &mut Runtime, _: &mut Call) -> u32 {
    rt.env_w
}
fn ret1(_: &mut Runtime, _: &mut Call) -> u32 {
    1
}
fn ret0(_: &mut Runtime, _: &mut Call) -> u32 {
    0
}
fn get_startup_info_a(rt: &mut Runtime, c: &mut Call) -> u32 {
    rt.mem.fill(c.arg(0), 68, 0);
    rt.mem.w32(c.arg(0), 68);
    0
}
fn get_std_handle(_: &mut Runtime, c: &mut Call) -> u32 {
    // STD_INPUT_HANDLE = -10, OUTPUT = -11, ERROR = -12
    match c.arg(0) as i32 {
        -10 => 0x10,
        -11 => 0x11,
        -12 => 0x12,
        _ => 0xffff_ffff,
    }
}
fn get_file_type(_: &mut Runtime, c: &mut Call) -> u32 {
    match c.arg(0) {
        0x10..=0x12 => 2, // FILE_TYPE_CHAR
        _ => 1,           // FILE_TYPE_DISK
    }
}
fn set_handle_count(_: &mut Runtime, c: &mut Call) -> u32 {
    c.arg(0)
}
fn get_module_handle_a(rt: &mut Runtime, c: &mut Call) -> u32 {
    if c.arg(0) == 0 {
        return 0x0040_0000;
    }
    let name = rt.mem.cstr(c.arg(0)).to_ascii_lowercase();
    let name = name.trim_end_matches(".dll");
    // Un handle finto per le DLL di sistema: GetProcAddress su di esse non trova nulla.
    match name {
        "kernel32" => 0x7e00_0000,
        "user32" => 0x7e01_0000,
        _ => 0,
    }
}
fn get_module_file_name_a(rt: &mut Runtime, c: &mut Call) -> u32 {
    let (buf, size) = (c.arg(1), c.arg(2));
    let n = (GUEST_EXE.len() as u32).min(size.saturating_sub(1));
    rt.mem.write_cstr(buf, &GUEST_EXE[..n as usize]);
    n
}
fn get_proc_address(rt: &mut Runtime, c: &mut Call) -> u32 {
    if rt.trace {
        eprint!(" [{}]", rt.mem.cstr(c.arg(1)));
    }
    0
}
fn load_library_a(rt: &mut Runtime, c: &mut Call) -> u32 {
    eprintln!("LoadLibraryA(\"{}\"): non disponibile", rt.mem.cstr(c.arg(0)));
    0
}
fn exit_process(rt: &mut Runtime, c: &mut Call) -> u32 {
    eprintln!("\nExitProcess({}) chiamato da {:#010x}", c.arg(0), c.ret_addr());
    let _ = rt;
    let mut calls = call_counts();
    calls.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    eprintln!("import chiamati: {}", calls.iter().take(40).map(|(k, n)| format!("{k}×{n}")).collect::<Vec<_>>().join(" "));
    std::process::exit(c.arg(0) as i32 & 0xff);
}
fn get_current_process(_: &mut Runtime, _: &mut Call) -> u32 {
    0xffff_ffff
}
fn get_current_thread(_: &mut Runtime, _: &mut Call) -> u32 {
    0xffff_fffe
}
fn get_current_process_id(rt: &mut Runtime, c: &mut Call) -> u32 {
    rt.mem.r32(c.teb() + 0x20)
}
fn get_current_thread_id(c: &mut Call) -> u32 {
    c.mem.r32(c.teb() + 0x24)
}
fn get_last_error(c: &mut Call) -> u32 {
    c.mem.r32(c.teb() + LAST_ERROR)
}
fn set_last_error(c: &mut Call) -> u32 {
    c.set_last_error(c.arg(0));
    0
}
fn output_debug_string_a(rt: &mut Runtime, c: &mut Call) -> u32 {
    eprint!("[debug] {}", rt.mem.cstr(c.arg(0)));
    0
}
fn get_system_info(rt: &mut Runtime, c: &mut Call) -> u32 {
    let p = c.arg(0);
    rt.mem.fill(p, 36, 0);
    rt.mem.w32(p + 4, 0x1000); // dwPageSize
    rt.mem.w32(p + 8, 0x0001_0000);
    rt.mem.w32(p + 12, 0x7ffe_ffff);
    rt.mem.w32(p + 16, 1); // dwActiveProcessorMask
    rt.mem.w32(p + 20, 1); // dwNumberOfProcessors: uno, finché non ci sono i thread
    rt.mem.w32(p + 24, 586);
    rt.mem.w32(p + 28, 0x0001_0000); // dwAllocationGranularity
    rt.mem.w16(p + 32, 6);
    0
}
fn get_process_affinity_mask(rt: &mut Runtime, c: &mut Call) -> u32 {
    rt.mem.w32(c.arg(1), 1);
    rt.mem.w32(c.arg(2), 1);
    1
}
fn sleep(c: &mut Call) -> u32 {
    std::thread::sleep(std::time::Duration::from_millis(c.arg(0) as u64));
    0
}

// --- kernel32: tempo ------------------------------------------------------------------------

fn query_performance_counter(rt: &mut Runtime, c: &mut Call) -> u32 {
    // 10 MHz dall'avvio del processo: valori grandi mandano in overflow i conti del gioco.
    rt.mem.w64(c.arg(0), (rt.start.elapsed().as_nanos() / 100) as u64);
    1
}
fn query_performance_frequency(rt: &mut Runtime, c: &mut Call) -> u32 {
    rt.mem.w64(c.arg(0), 10_000_000);
    1
}
fn get_tick_count(rt: &mut Runtime, _: &mut Call) -> u32 {
    60_000 + rt.start.elapsed().as_millis() as u32
}
fn get_system_time_as_file_time(rt: &mut Runtime, c: &mut Call) -> u32 {
    let unix = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    rt.mem.w64(c.arg(0), (unix.as_nanos() / 100) as u64 + 116_444_736_000_000_000);
    0
}

// --- kernel32: memoria ----------------------------------------------------------------------

fn heap_create(_: &mut Runtime, _: &mut Call) -> u32 {
    PROCESS_HEAP
}
fn heap_alloc(rt: &mut Runtime, c: &mut Call) -> u32 {
    rt.halloc(c.arg(2), c.arg(1) & 8 != 0)
}
fn heap_free(rt: &mut Runtime, c: &mut Call) -> u32 {
    if c.arg(2) != 0 && !rt.heap.free(c.arg(2)) {
        eprintln!("HeapFree di un blocco sconosciuto {:#x} (da {:#010x})", c.arg(2), c.ret_addr());
    }
    1
}
fn heap_realloc(rt: &mut Runtime, c: &mut Call) -> u32 {
    let (flags, old, size) = (c.arg(1), c.arg(2), c.arg(3));
    let (Some(old_size), Some(cap)) = (rt.heap.size(old), rt.heap.capacity(old)) else { return 0 };
    if size <= cap {
        if flags & 8 != 0 && size > old_size {
            rt.mem.fill(old + old_size, size - old_size, 0);
        }
        rt.heap.resize_in_place(old, size);
        return old;
    }
    let new = rt.halloc(size, true);
    if new != 0 {
        rt.mem.write(new, rt.mem.bytes(old, old_size));
        rt.heap.free(old);
    }
    new
}
fn heap_size(rt: &mut Runtime, c: &mut Call) -> u32 {
    rt.heap.size(c.arg(2)).unwrap_or(0xffff_ffff)
}
fn virtual_alloc(rt: &mut Runtime, c: &mut Call) -> u32 {
    let (addr, size) = (c.arg(0), c.arg(1));
    if addr != 0 {
        return addr & !0xfff; // commit di una zona già riservata: la memoria c'è già
    }
    let size = (size + 0xffff) & !0xffff;
    if VIRTUAL_END - rt.virtual_next < size {
        return 0;
    }
    let a = rt.virtual_next;
    rt.virtual_next += size;
    rt.regions.insert(a, size);
    a
}
/// `MEM_RELEASE` (0x8000) dimentica la zona e la riazzera; gli indirizzi non vengono riusati.
fn virtual_free(rt: &mut Runtime, c: &mut Call) -> u32 {
    if c.arg(2) & 0x8000 != 0 {
        if let Some(size) = rt.regions.remove(&c.arg(0)) {
            crate::host::discard(rt.mem.ptr(c.arg(0)), size as usize);
        }
    }
    1
}
/// Il motore usa `VirtualQuery` per sapere quanto è grande una zona prima di liberarla:
/// `RegionSize` deve arrivare fino alla fine dell'allocazione.
fn virtual_query(rt: &mut Runtime, c: &mut Call) -> u32 {
    let (addr, p) = (c.arg(0) & !0xfff, c.arg(1));
    let inside = rt.regions.range(..=addr).next_back().filter(|(&b, &sz)| addr < b + sz).map(|(&b, &sz)| (b, sz));
    let (alloc_base, size, state) = match inside {
        Some((b, sz)) => (b, b + sz - addr, 0x1000), // MEM_COMMIT
        None => {
            let next = rt.regions.range(addr..).next().map_or(VIRTUAL_END, |(&b, _)| b);
            let known = (0x0010_0000..VIRTUAL_START).contains(&addr); // stack, immagine, heap
            if known { (addr & !0xffff, 0x1000, 0x1000) } else { (0, next.saturating_sub(addr).max(0x1000), 0x10000) }
        }
    };
    rt.mem.w32(p, addr);
    rt.mem.w32(p + 4, alloc_base);
    rt.mem.w32(p + 8, if state == 0x1000 { 4 } else { 0 }); // PAGE_READWRITE
    rt.mem.w32(p + 12, size);
    rt.mem.w32(p + 16, state);
    rt.mem.w32(p + 20, if state == 0x1000 { 4 } else { 0 });
    rt.mem.w32(p + 24, if state == 0x1000 { 0x20000 } else { 0 }); // MEM_PRIVATE
    28
}

// --- kernel32: TLS, sezioni critiche, Interlocked -------------------------------------------

fn tls_alloc(rt: &mut Runtime, _: &mut Call) -> u32 {
    if rt.tls_next >= 64 {
        return 0xffff_ffff;
    }
    rt.tls_next += 1;
    rt.tls_next - 1
}
fn tls_get_value(c: &mut Call) -> u32 {
    c.set_last_error(0);
    c.mem.r32(c.teb() + TLS_SLOTS + 4 * (c.arg(0) & 63))
}
fn tls_set_value(c: &mut Call) -> u32 {
    c.mem.w32(c.teb() + TLS_SLOTS + 4 * (c.arg(0) & 63), c.arg(1));
    1
}
// CRITICAL_SECTION: { DebugInfo, LockCount, RecursionCount, OwningThread, LockSemaphore, SpinCount }.
// Il lucchetto è il campo OwningThread, preso con un compare-and-swap; RecursionCount conta i
// rientri del proprietario. Niente stato sul lato host: funziona anche per le sezioni che il
// gioco inizializza a mano o copia.
fn init_critical_section(rt: &mut Runtime, c: &mut Call) -> u32 {
    rt.mem.fill(c.arg(0), 24, 0);
    rt.mem.w32(c.arg(0) + 4, 0xffff_ffff);
    1
}
fn cs_try(c: &Call) -> bool {
    let (p, me) = (c.arg(0), c.mem.r32(c.teb() + 0x24));
    let owner = atomic(c.mem, p + 12);
    if owner.load(Ordering::Acquire) == me || owner.compare_exchange(0, me, Ordering::AcqRel, Ordering::Relaxed).is_ok() {
        c.mem.w32(p + 8, c.mem.r32(p + 8) + 1);
        return true;
    }
    false
}
fn enter_critical_section(c: &mut Call) -> u32 {
    let mut spins = 0u32;
    while !cs_try(c) {
        spins += 1;
        if spins < 200 {
            std::hint::spin_loop();
        } else if spins < 400 {
            std::thread::yield_now();
        } else {
            std::thread::sleep(std::time::Duration::from_micros(200));
        }
    }
    0
}
fn try_enter_critical_section(c: &mut Call) -> u32 {
    cs_try(c) as u32
}
fn leave_critical_section(c: &mut Call) -> u32 {
    let p = c.arg(0);
    let n = c.mem.r32(p + 8).wrapping_sub(1);
    c.mem.w32(p + 8, n);
    if n == 0 {
        atomic(c.mem, p + 12).store(0, Ordering::Release);
    }
    0
}

// --- kernel32: eventi, semafori, attese, thread ---------------------------------------------

impl Runtime {
    pub fn object(&self, h: u32) -> Option<Arc<Object>> {
        self.objects.get(&h).cloned()
    }
    pub fn fresh_handle(&mut self) -> u32 {
        self.next_handle += 4;
        self.next_handle
    }
    fn new_handle(&mut self, o: Object) -> u32 {
        self.next_handle += 4;
        self.objects.insert(self.next_handle, Arc::new(o));
        self.next_handle
    }
}
fn object(h: u32) -> Option<Arc<Object>> {
    rt().objects.get(&h).cloned()
}
fn create_event_a(rt: &mut Runtime, c: &mut Call) -> u32 {
    rt.new_handle(Object::new(Kind::Event { manual: c.arg(1) != 0 }, (c.arg(2) != 0) as i64))
}
fn create_semaphore_a(rt: &mut Runtime, c: &mut Call) -> u32 {
    rt.new_handle(Object::new(Kind::Semaphore { max: c.arg(2) as i64 }, c.arg(1) as i64))
}
fn set_event(c: &mut Call) -> u32 {
    object(c.arg(0)).map(|o| o.set()).is_some() as u32
}
fn reset_event(c: &mut Call) -> u32 {
    object(c.arg(0)).map(|o| o.reset()).is_some() as u32
}
fn release_semaphore(c: &mut Call) -> u32 {
    let Some(prev) = object(c.arg(0)).and_then(|o| o.release(c.arg(1) as i64)) else { return 0 };
    if c.arg(2) != 0 {
        c.mem.w32(c.arg(2), prev as u32);
    }
    1
}
fn close_handle(rt: &mut Runtime, c: &mut Call) -> u32 {
    if !fs::close(rt, c.arg(0)) {
        rt.objects.remove(&c.arg(0));
    }
    1
}
const WAIT_TIMEOUT: u32 = 0x102;
fn timeout(ms: u32) -> Option<u32> {
    (ms != 0xffff_ffff).then_some(ms)
}
fn wait_for_single_object(c: &mut Call) -> u32 {
    match object(c.arg(0)) {
        Some(o) => {
            if o.wait(timeout(c.arg(1))) { 0 } else { WAIT_TIMEOUT }
        }
        None => {
            eprintln!("WaitForSingleObject su un handle sconosciuto {:#x} (da {:#010x})", c.arg(0), c.ret_addr());
            0xffff_ffff
        }
    }
}
fn wait_for_multiple_objects(c: &mut Call) -> u32 {
    let (n, handles, all, ms) = (c.arg(0), c.arg(1), c.arg(2) != 0, c.arg(3));
    let objs: Vec<Arc<Object>> = (0..n).filter_map(|i| object(c.mem.r32(handles + 4 * i))).collect();
    if objs.len() != n as usize || all {
        eprintln!("WaitForMultipleObjects non gestita: {n} handle, waitAll={all} (da {:#010x})", c.ret_addr());
        return 0xffff_ffff;
    }
    // Attesa di uno qualsiasi, a sondaggio: basta per le poche chiamate del gioco.
    let start = Instant::now();
    loop {
        if let Some(i) = objs.iter().position(|o| o.try_wait()) {
            return i as u32;
        }
        if timeout(ms).is_some_and(|m| start.elapsed().as_millis() >= m as u128) {
            return WAIT_TIMEOUT;
        }
        std::thread::sleep(std::time::Duration::from_micros(500));
    }
}

struct SendCtx(Box<Ctx>);
// SAFETY: il contesto è usato solo dal thread guest a cui viene consegnato.
unsafe impl Send for SendCtx {}

fn create_thread(c: &mut Call) -> u32 {
    let (stack_size, start, param, flags, id_out) = (c.arg(1), c.arg(2), c.arg(3), c.arg(4), c.arg(5));
    let mem = c.mem;
    let (handle, obj, mut ctx, tid) = {
        let mut rt = rt();
        let size = if stack_size == 0 { 0x10_0000 } else { (stack_size + 0xffff) & !0xffff };
        // Stack e TEB dalla zona di VirtualAlloc.
        let stack = rt.virtual_next;
        let teb = stack + size;
        rt.virtual_next = teb + 0x1_0000;
        let tid = rt.next_tid;
        rt.next_tid += 4;
        mem.w32(teb, 0xffff_ffff);
        mem.w32(teb + 0x04, stack + size);
        mem.w32(teb + 0x08, stack);
        mem.w32(teb + 0x18, teb);
        mem.w32(teb + 0x20, 0x800);
        mem.w32(teb + 0x24, tid);
        mem.w32(teb + 0x30, crate::PEB);
        let tls = rt.static_tls;
        rt.setup_static_tls_from(tls, teb);
        let mut ctx = Box::new(Ctx::new(mem.0));
        ctx.fs_base = teb;
        ctx.esp = stack + size - 0x40;
        ctx.host_call = Some(host_call);
        mem.w32(ctx.esp, 0);
        mem.w32(ctx.esp + 4, param);
        let o = Object::new(Kind::Thread, 0);
        *o.gate.lock().unwrap() = flags & 4 == 0; // CREATE_SUSPENDED
        let h = rt.new_handle(o);
        (h, rt.objects[&h].clone(), ctx, tid)
    };
    if id_out != 0 {
        mem.w32(id_out, tid);
    }
    ctx.eax = 0;
    let boxed = SendCtx(ctx);
    let run = *RUN.get().unwrap();
    let spawned = std::thread::Builder::new().name(format!("guest-{tid:x}")).stack_size(1 << 28).spawn(move || {
        let mut boxed = boxed;
        {
            let mut go = obj.gate.lock().unwrap();
            while !*go {
                go = obj.gate_cv.wait(go).unwrap();
            }
        }
        // SAFETY: il contesto appartiene a questo thread; `start` è una funzione guest.
        let trapped = unsafe { run(&mut *boxed.0, start) };
        let code = match EXITING.with(|e| e.get()) {
            Some(code) => code,
            None if trapped != 0 => {
                eprintln!("\nTRAP nel thread {tid:#x}: {}", crate::trap_text(&boxed.0));
                report_stop(&boxed.0);
                std::process::exit(2);
            }
            None => boxed.0.eax,
        };
        *obj.exit_code.lock().unwrap() = code;
        obj.set();
    });
    if spawned.is_err() {
        return 0;
    }
    handle
}
fn resume_thread(c: &mut Call) -> u32 {
    let Some(o) = object(c.arg(0)) else { return 0xffff_ffff };
    let mut go = o.gate.lock().unwrap();
    let was_suspended = !*go;
    *go = true;
    o.gate_cv.notify_all();
    was_suspended as u32
}
fn exit_thread(c: &mut Call) -> u32 {
    EXITING.with(|e| e.set(Some(c.arg(0))));
    0
}
fn get_exit_code_thread(c: &mut Call) -> u32 {
    let Some(o) = object(c.arg(0)) else { return 0 };
    c.mem.w32(c.arg(1), *o.exit_code.lock().unwrap());
    1
}

fn interlocked_increment(c: &mut Call) -> u32 {
    atomic(c.mem, c.arg(0)).fetch_add(1, Ordering::SeqCst).wrapping_add(1)
}
fn interlocked_decrement(c: &mut Call) -> u32 {
    atomic(c.mem, c.arg(0)).fetch_sub(1, Ordering::SeqCst).wrapping_sub(1)
}
fn interlocked_exchange(c: &mut Call) -> u32 {
    atomic(c.mem, c.arg(0)).swap(c.arg(1), Ordering::SeqCst)
}
fn interlocked_exchange_add(c: &mut Call) -> u32 {
    atomic(c.mem, c.arg(0)).fetch_add(c.arg(1), Ordering::SeqCst)
}

// --- kernel32: locale e stringhe ------------------------------------------------------------

fn get_acp(_: &mut Runtime, _: &mut Call) -> u32 {
    1252
}
fn get_oemcp(_: &mut Runtime, _: &mut Call) -> u32 {
    437
}
fn get_cp_info(rt: &mut Runtime, c: &mut Call) -> u32 {
    let p = c.arg(1);
    rt.mem.fill(p, 20, 0);
    rt.mem.w32(p, 1); // MaxCharSize
    rt.mem.w8(p + 4, b'?');
    1
}
fn multi_byte_to_wide_char(rt: &mut Runtime, c: &mut Call) -> u32 {
    let (src, mut n, dst, cap) = (c.arg(2), c.arg(3), c.arg(4), c.arg(5));
    if n == 0xffff_ffff {
        n = rt.mem.cstr_bytes(src).len() as u32 + 1;
    }
    if cap == 0 {
        return n;
    }
    let n = n.min(cap);
    for i in 0..n {
        rt.mem.w16(dst + 2 * i, rt.mem.r8(src + i) as u16);
    }
    n
}
fn wide_char_to_multi_byte(rt: &mut Runtime, c: &mut Call) -> u32 {
    let (src, mut n, dst, cap) = (c.arg(2), c.arg(3), c.arg(4), c.arg(5));
    if n == 0xffff_ffff {
        n = 1;
        while rt.mem.r16(src + 2 * (n - 1)) != 0 {
            n += 1;
        }
    }
    if cap == 0 {
        return n;
    }
    let n = n.min(cap);
    for i in 0..n {
        let ch = rt.mem.r16(src + 2 * i);
        rt.mem.w8(dst + i, if ch < 256 { ch as u8 } else { b'?' });
    }
    n
}
fn get_string_type_w(rt: &mut Runtime, c: &mut Call) -> u32 {
    let (kind, src, n, out) = (c.arg(0), c.arg(1), c.arg(2), c.arg(3));
    for i in 0..n {
        let f = if kind == 1 { ctype1(rt.mem.r16(src + 2 * i) as u32) } else { 0 };
        rt.mem.w16(out + 2 * i, f);
    }
    1
}
fn get_string_type_a(rt: &mut Runtime, c: &mut Call) -> u32 {
    let (kind, src, n, out) = (c.arg(1), c.arg(2), c.arg(3), c.arg(4));
    for i in 0..n {
        let f = if kind == 1 { ctype1(rt.mem.r8(src + i) as u32) } else { 0 };
        rt.mem.w16(out + 2 * i, f);
    }
    1
}
fn lc_map_string_w(rt: &mut Runtime, c: &mut Call) -> u32 {
    let (flags, src, mut n, dst, cap) = (c.arg(1), c.arg(2), c.arg(3), c.arg(4), c.arg(5));
    if n == 0xffff_ffff {
        n = rt.mem.wstr(src).chars().count() as u32 + 1;
    }
    if cap == 0 {
        return n;
    }
    let n = n.min(cap);
    for i in 0..n {
        rt.mem.w16(dst + 2 * i, map_case(rt.mem.r16(src + 2 * i) as u32, flags) as u16);
    }
    n
}
fn lc_map_string_a(rt: &mut Runtime, c: &mut Call) -> u32 {
    let (flags, src, mut n, dst, cap) = (c.arg(1), c.arg(2), c.arg(3), c.arg(4), c.arg(5));
    if n == 0xffff_ffff {
        n = rt.mem.cstr_bytes(src).len() as u32 + 1;
    }
    if cap == 0 {
        return n;
    }
    let n = n.min(cap);
    for i in 0..n {
        let m = map_case(rt.mem.r8(src + i) as u32, flags);
        rt.mem.w8(dst + i, if m < 256 { m as u8 } else { rt.mem.r8(src + i) });
    }
    n
}

/// Senza un vero SEH gestiamo solo l'eccezione con cui MSVC comunica al debugger il nome di un
/// thread (0x406D1388): il gioco la racchiude in un `__try` che la ignora, quindi ritornare
/// equivale. Ogni altra eccezione ferma tutto con un messaggio.
fn raise_exception(rt: &mut Runtime, c: &mut Call) -> u32 {
    let code = c.arg(0);
    if code == 0x406d_1388 {
        let info = c.arg(3);
        eprintln!("[thread] {:#x} si chiama \"{}\"", rt.mem.r32(info + 8), rt.mem.cstr(rt.mem.r32(info + 4)));
        return 0;
    }
    let what = if code == 0xe06d_7363 { " (throw C++)" } else { "" };
    eprintln!("\nRaiseException({code:#x}){what} da {:#010x}: le eccezioni non sono ancora gestite", c.ret_addr());
    std::process::exit(3);
}

/// (nome dell'import, numero di argomenti, implementazione)
static SHIMS: &[(&str, u32, Shim)] = &[
    ("GetVersion", 0, L(get_version)),
    ("GetVersionExA", 1, L(get_version_ex_a)),
    ("GetCommandLineA", 0, L(get_command_line_a)),
    ("GetEnvironmentStrings", 0, L(get_environment_strings)),
    ("GetEnvironmentStringsW", 0, L(get_environment_strings_w)),
    ("FreeEnvironmentStringsA", 1, L(ret1)),
    ("FreeEnvironmentStringsW", 1, L(ret1)),
    ("GetStartupInfoA", 1, L(get_startup_info_a)),
    ("GetStdHandle", 1, L(get_std_handle)),
    ("GetFileType", 1, L(get_file_type)),
    ("SetHandleCount", 1, L(set_handle_count)),
    ("GetModuleHandleA", 1, L(get_module_handle_a)),
    ("GetModuleFileNameA", 3, L(get_module_file_name_a)),
    ("GetProcAddress", 2, L(get_proc_address)),
    ("LoadLibraryA", 1, L(load_library_a)),
    ("FreeLibrary", 1, L(ret1)),
    ("ExitProcess", 1, L(exit_process)),
    ("GetCurrentProcess", 0, L(get_current_process)),
    ("GetCurrentThread", 0, L(get_current_thread)),
    ("GetCurrentProcessId", 0, L(get_current_process_id)),
    ("GetCurrentThreadId", 0, F(get_current_thread_id)),
    ("GetLastError", 0, F(get_last_error)),
    ("SetLastError", 1, F(set_last_error)),
    ("OutputDebugStringA", 1, L(output_debug_string_a)),
    ("GetSystemInfo", 1, L(get_system_info)),
    ("GetProcessAffinityMask", 3, L(get_process_affinity_mask)),
    ("Sleep", 1, F(sleep)),
    ("SetUnhandledExceptionFilter", 1, L(ret0)),
    ("IsDebuggerPresent", 0, L(ret0)),
    ("SetErrorMode", 1, L(ret0)),
    ("SetConsoleCtrlHandler", 2, L(ret1)),
    ("QueryPerformanceCounter", 1, L(query_performance_counter)),
    ("QueryPerformanceFrequency", 1, L(query_performance_frequency)),
    ("GetTickCount", 0, L(get_tick_count)),
    ("GetSystemTimeAsFileTime", 1, L(get_system_time_as_file_time)),
    ("HeapCreate", 3, L(heap_create)),
    ("GetProcessHeap", 0, L(heap_create)),
    ("HeapDestroy", 1, L(ret1)),
    ("HeapAlloc", 3, L(heap_alloc)),
    ("HeapFree", 3, L(heap_free)),
    ("HeapReAlloc", 4, L(heap_realloc)),
    ("HeapSize", 3, L(heap_size)),
    ("VirtualAlloc", 4, L(virtual_alloc)),
    ("VirtualFree", 3, L(virtual_free)),
    ("VirtualQuery", 3, L(virtual_query)),
    ("TlsAlloc", 0, L(tls_alloc)),
    ("TlsGetValue", 1, F(tls_get_value)),
    ("TlsSetValue", 2, F(tls_set_value)),
    ("TlsFree", 1, L(ret1)),
    ("InitializeCriticalSection", 1, L(init_critical_section)),
    ("InitializeCriticalSectionAndSpinCount", 2, L(init_critical_section)),
    ("DeleteCriticalSection", 1, L(ret0)),
    ("EnterCriticalSection", 1, F(enter_critical_section)),
    ("TryEnterCriticalSection", 1, F(try_enter_critical_section)),
    ("LeaveCriticalSection", 1, F(leave_critical_section)),
    ("InterlockedIncrement", 1, F(interlocked_increment)),
    ("InterlockedDecrement", 1, F(interlocked_decrement)),
    ("InterlockedExchange", 2, F(interlocked_exchange)),
    ("InterlockedExchangeAdd", 2, F(interlocked_exchange_add)),
    ("GetACP", 0, L(get_acp)),
    ("GetOEMCP", 0, L(get_oemcp)),
    ("GetCPInfo", 2, L(get_cp_info)),
    ("IsValidCodePage", 1, L(ret1)),
    ("MultiByteToWideChar", 6, L(multi_byte_to_wide_char)),
    ("WideCharToMultiByte", 8, L(wide_char_to_multi_byte)),
    ("GetStringTypeW", 4, L(get_string_type_w)),
    ("GetStringTypeA", 5, L(get_string_type_a)),
    ("LCMapStringW", 6, L(lc_map_string_w)),
    ("LCMapStringA", 6, L(lc_map_string_a)),
    ("CreateEventA", 4, L(create_event_a)),
    ("CreateSemaphoreA", 4, L(create_semaphore_a)),
    ("SetEvent", 1, F(set_event)),
    ("ResetEvent", 1, F(reset_event)),
    ("ReleaseSemaphore", 3, F(release_semaphore)),
    ("CloseHandle", 1, L(close_handle)),
    ("WaitForSingleObject", 2, F(wait_for_single_object)),
    ("WaitForMultipleObjects", 4, F(wait_for_multiple_objects)),
    ("CreateThread", 6, F(create_thread)),
    ("ResumeThread", 1, F(resume_thread)),
    ("ExitThread", 1, F(exit_thread)),
    ("GetExitCodeThread", 2, F(get_exit_code_thread)),
    ("SetThreadPriority", 2, L(ret1)),
    ("SetThreadPriorityBoost", 2, L(ret1)),
    ("SetThreadAffinityMask", 2, L(ret1)),
    ("SwitchToThread", 0, L(ret0)),
    ("CreateFileA", 7, L(fs::create_file_a)),
    ("ReadFile", 5, L(fs::read_file)),
    ("WriteFile", 5, L(fs::write_file)),
    ("SetFilePointer", 4, L(fs::set_file_pointer)),
    ("GetFileSize", 2, L(fs::get_file_size)),
    ("GetFileSizeEx", 2, L(fs::get_file_size_ex)),
    ("FlushFileBuffers", 1, L(fs::flush_file_buffers)),
    ("SetEndOfFile", 1, L(fs::set_end_of_file)),
    ("GetFileAttributesA", 1, L(fs::get_file_attributes_a)),
    ("CreateDirectoryA", 2, L(fs::create_directory_a)),
    ("SHCreateDirectoryExA", 3, L(fs::sh_create_directory_ex_a)),
    ("DeleteFileA", 1, L(fs::delete_file_a)),
    ("GetCurrentDirectoryA", 2, L(fs::get_current_directory_a)),
    ("SetCurrentDirectoryA", 1, L(fs::set_current_directory_a)),
    ("GetFullPathNameA", 4, L(fs::get_full_path_name_a)),
    ("SHGetSpecialFolderPathA", 4, L(fs::sh_get_special_folder_path_a)),
    ("FindFirstFileA", 2, L(fs::find_first_file_a)),
    ("FindNextFileA", 2, L(fs::find_next_file_a)),
    ("FindClose", 1, L(fs::find_close)),
    ("GetFileTime", 4, L(fs::get_file_time)),
    ("GetDriveTypeA", 1, L(fs::get_drive_type_a)),
    ("GetOverlappedResult", 4, L(fs::get_overlapped_result)),
    ("GetLogicalDriveStringsA", 2, L(fs::get_logical_drive_strings_a)),
    ("RaiseException", 4, L(raise_exception)),
];

#[allow(dead_code)]
const _TEB_CHECK: u32 = TEB;
