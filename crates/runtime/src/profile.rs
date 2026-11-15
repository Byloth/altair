//! Profilatore a campionamento, per quando non c'è `perf` (`ALTAIR_PROFILE=1`).
//!
//! Un thread manda mille volte al secondo un segnale al thread che disegna; il gestore
//! annota dove stava eseguendo (il registro RIP). Alla fine i campioni si raggruppano per
//! libreria e per simbolo esportato più vicino (`dladdr`). Non ricostruisce le pile di
//! chiamata: dice *dove* si spende il tempo, non *per conto di chi*.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

const CAPACITY: usize = 1 << 18;
static SAMPLES: [AtomicU64; CAPACITY] = [const { AtomicU64::new(0) }; CAPACITY];
/// Per ogni campione, il primo indirizzo di ritorno sullo stack che cade nel nostro eseguibile:
/// chi, nel runtime (wgpu compreso), ha chiamato la libreria in cui si trovava il thread.
static CALLERS: [AtomicU64; CAPACITY] = [const { AtomicU64::new(0) }; CAPACITY];
static NEXT: AtomicUsize = AtomicUsize::new(0);
/// Intervallo del codice dell'eseguibile, e fine dello stack del thread osservato.
static TEXT: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];
static STACK_END: AtomicU64 = AtomicU64::new(0);
static STARTED: std::sync::Once = std::sync::Once::new();

pub fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("ALTAIR_PROFILE").is_ok_and(|v| v == "1"))
}

extern "C" fn on_sample(_: libc::c_int, _: *mut libc::siginfo_t, context: *mut libc::c_void) {
    // Solo operazioni atomiche: il gestore interrompe codice qualunque.
    #[cfg(target_arch = "x86_64")]
    unsafe {
        let regs = &(*(context as *mut libc::ucontext_t)).uc_mcontext.gregs;
        let (rip, rsp) = (regs[libc::REG_RIP as usize] as u64, regs[libc::REG_RSP as usize] as u64);
        let slot = NEXT.fetch_add(1, Ordering::Relaxed) % CAPACITY;
        SAMPLES[slot].store(rip, Ordering::Relaxed);
        // Scansione prudente dello stack: le librerie del driver non hanno puntatori di frame.
        let (lo, hi, end) = (TEXT[0].load(Ordering::Relaxed), TEXT[1].load(Ordering::Relaxed), STACK_END.load(Ordering::Relaxed));
        let mut caller = 0;
        if (lo..hi).contains(&rip) {
            caller = rip;
        } else if rsp < end {
            let mut p = rsp & !7;
            let stop = end.min(p + 64 * 1024);
            while p < stop {
                let v = *(p as *const u64);
                if (lo..hi).contains(&v) {
                    caller = v;
                    break;
                }
                p += 8;
            }
        }
        CALLERS[slot].store(caller, Ordering::Relaxed);
    }
    #[cfg(not(target_arch = "x86_64"))]
    let _ = context;
}

/// Da chiamare dal thread da osservare (quello che presenta i fotogrammi): la prima volta
/// avvia il campionamento.
pub fn watch_current_thread() {
    if !enabled() {
        return;
    }
    STARTED.call_once(|| unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = on_sample as *const () as usize;
        action.sa_flags = libc::SA_SIGINFO | libc::SA_RESTART;
        libc::sigaction(libc::SIGUSR2, &action, std::ptr::null_mut());
        let target = libc::pthread_self() as usize;
        // Dove sta il codice dell'eseguibile (prima mappatura eseguibile di /proc/self/maps che
        // porta il suo nome) e dove finisce lo stack di questo thread.
        let exe = std::env::current_exe().ok().and_then(|p| p.to_str().map(str::to_string)).unwrap_or_default();
        for line in std::fs::read_to_string("/proc/self/maps").unwrap_or_default().lines() {
            let mut f = line.split_whitespace();
            let (Some(range), Some(perms)) = (f.next(), f.next()) else { continue };
            if perms.contains('x') && line.ends_with(&exe) {
                if let Some((a, b)) = range.split_once('-') {
                    TEXT[0].store(u64::from_str_radix(a, 16).unwrap_or(0), Ordering::Relaxed);
                    TEXT[1].store(u64::from_str_radix(b, 16).unwrap_or(0), Ordering::Relaxed);
                    break;
                }
            }
        }
        let mut attr: libc::pthread_attr_t = std::mem::zeroed();
        if libc::pthread_getattr_np(libc::pthread_self(), &mut attr) == 0 {
            let (mut base, mut size) = (std::ptr::null_mut(), 0usize);
            libc::pthread_attr_getstack(&attr, &mut base, &mut size);
            STACK_END.store(base as u64 + size as u64, Ordering::Relaxed);
            libc::pthread_attr_destroy(&mut attr);
        }
        std::thread::Builder::new()
            .name("profilo".into())
            .spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_micros(1000));
                libc::pthread_kill(target as libc::pthread_t, libc::SIGUSR2);
            })
            .expect("thread del profilatore");
        eprintln!("[profilo] campionamento del thread di rendering a 1 kHz");
    });
}

/// Azzera i campioni (per misurare una scena sola).
pub fn reset() {
    NEXT.store(0, Ordering::Relaxed);
}

/// Stampa dove sono caduti i campioni: per libreria, poi i simboli più frequenti.
pub fn report() {
    if !enabled() {
        return;
    }
    let n = NEXT.load(Ordering::Relaxed).min(CAPACITY);
    if n == 0 {
        return;
    }
    let mut by_lib: HashMap<String, u64> = HashMap::new();
    let mut by_sym: HashMap<(String, String), u64> = HashMap::new();
    for s in &SAMPLES[..n] {
        let addr = s.load(Ordering::Relaxed);
        let mut info: libc::Dl_info = unsafe { std::mem::zeroed() };
        let found = unsafe { libc::dladdr(addr as *const libc::c_void, &mut info) } != 0;
        let text = |p: *const libc::c_char| if p.is_null() { None } else { Some(unsafe { std::ffi::CStr::from_ptr(p) }.to_string_lossy().into_owned()) };
        let lib = if found { text(info.dli_fname) } else { None }.map(|f| f.rsplit('/').next().unwrap_or("?").to_string()).unwrap_or_else(|| "(fuori da ogni libreria: codice generato o kernel)".into());
        let sym = if found { text(info.dli_sname) } else { None }.unwrap_or_else(|| format!("+{:#x}", addr.wrapping_sub(info.dli_fbase as u64) & !0xfff));
        *by_lib.entry(lib.clone()).or_default() += 1;
        *by_sym.entry((lib, sym)).or_default() += 1;
    }
    let pct = |c: u64| 100.0 * c as f64 / n as f64;
    let mut libs: Vec<_> = by_lib.into_iter().collect();
    libs.sort_by_key(|(_, c)| std::cmp::Reverse(*c));
    eprintln!("\n[profilo] {n} campioni del thread di rendering, per libreria:");
    for (lib, c) in libs.iter().take(12) {
        eprintln!("[profilo]   {:5.1}%  {lib}", pct(*c));
    }
    // Chi, nel nostro eseguibile, ha portato il thread fuori: indirizzi relativi all'inizio
    // del file, da dare a `addr2line -f -C -i -e target/release/altair-run`.
    let base = TEXT[0].load(Ordering::Relaxed);
    let mut file_offset = 0u64;
    for line in std::fs::read_to_string("/proc/self/maps").unwrap_or_default().lines() {
        if line.starts_with(&format!("{base:x}-")) {
            file_offset = line.split_whitespace().nth(2).and_then(|o| u64::from_str_radix(o, 16).ok()).unwrap_or(0);
        }
    }
    let mut by_caller: HashMap<(u64, bool), u64> = HashMap::new();
    for i in 0..n {
        let (rip, caller) = (SAMPLES[i].load(Ordering::Relaxed), CALLERS[i].load(Ordering::Relaxed));
        if caller != 0 {
            *by_caller.entry((caller - base + file_offset, caller == rip)).or_default() += 1;
        }
    }
    let mut callers: Vec<_> = by_caller.into_iter().collect();
    callers.sort_by_key(|(_, c)| std::cmp::Reverse(*c));
    eprintln!("[profilo] punti del nostro eseguibile da cui si esce verso le librerie (o in cui si sta, se segnato \"dentro\"):");
    for ((offset, inside), c) in callers.iter().take(30) {
        eprintln!("[profilo]   {:5.1}%  {offset:#x}{}", pct(*c), if *inside { "  dentro" } else { "" });
    }
    let mut syms: Vec<_> = by_sym.into_iter().collect();
    syms.sort_by_key(|(_, c)| std::cmp::Reverse(*c));
    eprintln!("[profilo] simboli (o pagine, dove non ci sono simboli) più frequenti:");
    for ((lib, sym), c) in syms.iter().take(12) {
        eprintln!("[profilo]   {:5.1}%  {lib}  {}", pct(*c), &sym[..sym.len().min(110)]);
    }
}
