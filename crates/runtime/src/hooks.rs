//! Agganci: funzioni del gioco tradotto che all'ingresso chiamano il runtime.
//!
//! L'elenco sta in `runtime/hooks.txt`: il traduttore mette in testa a quelle funzioni
//! `if (altair_hook(c, indirizzo)) return;` e qui arriva la chiamata. Due usi:
//! - **osservare**: ogni aggancio conta le chiamate (comando `hooks` del canale di comando) e,
//!   con `ALTAIR_HOOK_LOG=1`, scrive nel registro le prime di ognuno con istante e argomenti;
//! - **sostituire**: una voce di `REPLACEMENTS` può fare il lavoro al posto della funzione e
//!   ritornare `true`, dopo aver lasciato registri e stack come dopo il `ret` originale.
//!
//! Gli agganci girano sui thread del gioco, senza il lock del runtime: niente stato condiviso
//! che non sia atomico.

use crate::machine::Ctx;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

struct Hook {
    addr: u32,
    name: &'static str,
    calls: AtomicU64,
    /// Chiamate già riferite dall'ultimo comando `hooks`.
    seen: AtomicU64,
}

/// Chi interviene su una funzione, per indirizzo. Ritorna `true` solo se l'ha sostituita del
/// tutto; chi si limita a ritoccare gli argomenti ritorna `false` e la funzione prosegue.
const REPLACEMENTS: &[(u32, fn(&mut Ctx) -> bool)] = &[(0x8a_1920, animus_version)];

/// Memoria guest nostra per i testi sostituiti: `SLOTS` caselle da `SLOT` byte, usate a giro
/// (la funzione agganciata copia subito il testo, la casella serve solo per quell'istante).
static SCRATCH: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static NEXT: AtomicU64 = AtomicU64::new(0);
const SLOT: u32 = 512;
const SLOTS: u32 = 16;
pub const SCRATCH_SIZE: u32 = SLOT * SLOTS;

pub fn set_scratch(addr: u32) {
    SCRATCH.store(addr, Ordering::Relaxed);
}

/// La versione dell'Animus mostrata dal gioco. I testi dell'interfaccia passano tutti da
/// `0x8A1920` (stringa di uscita, testo UTF-16 terminato da zero) appena letti dai dati: se il
/// testo è "… Animus … 1.28", in qualunque lingua, alla funzione ne arriva una copia con la
/// versione nostra. Il carattere dell'Animus copre solo Latin-1: niente lettere greche.
/// `ALTAIR_ANIMUS_VERSION` la cambia; vuota, lascia l'originale.
fn animus_version(c: &mut Ctx) -> bool {
    const OLD: &str = "1.28";
    static NEW: OnceLock<Vec<u16>> = OnceLock::new();
    let new = NEW.get_or_init(|| std::env::var("ALTAIR_ANIMUS_VERSION").unwrap_or_else(|_| "2.0.0-dev.1".into()).encode_utf16().collect());
    let scratch = SCRATCH.load(Ordering::Relaxed);
    if new.is_empty() || scratch == 0 {
        return false;
    }
    let mem = crate::machine::Mem(c.mem);
    let src = mem.r32(c.esp.wrapping_add(8));
    if src < 0x1_0000 {
        return false;
    }
    let mut text: Vec<u16> = Vec::new();
    for i in 0..120u32 {
        let ch = mem.r16(src + 2 * i);
        if ch == 0 {
            break;
        }
        text.push(ch);
    }
    let old: Vec<u16> = OLD.encode_utf16().collect();
    let animus: Vec<u16> = "Animus".encode_utf16().collect();
    if text.len() >= 119 || !text.ends_with(&old) || !text.windows(animus.len()).any(|w| w == animus) {
        return false;
    }
    text.truncate(text.len() - old.len());
    text.extend_from_slice(new);
    text.push(0);
    if text.len() as u32 * 2 > SLOT {
        return false;
    }
    let slot = scratch + (NEXT.fetch_add(1, Ordering::Relaxed) % SLOTS as u64) as u32 * SLOT;
    for (i, ch) in text.iter().enumerate() {
        mem.w16(slot + 2 * i as u32, *ch);
    }
    mem.w32(c.esp.wrapping_add(8), slot);
    false
}

fn table() -> &'static Vec<Hook> {
    static TABLE: OnceLock<Vec<Hook>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut v: Vec<Hook> = include_str!("../../../runtime/hooks.txt")
            .lines()
            .filter_map(|l| {
                let mut words = l.split('#').next()?.split_whitespace();
                let addr = u32::from_str_radix(words.next()?.trim_start_matches("0x"), 16).ok()?;
                Some(Hook { addr, name: words.next().unwrap_or("?"), calls: AtomicU64::new(0), seen: AtomicU64::new(0) })
            })
            .collect();
        v.sort_by_key(|h| h.addr);
        v
    })
}

fn start() -> Instant {
    static START: OnceLock<Instant> = OnceLock::new();
    *START.get_or_init(Instant::now)
}

fn log_limit() -> u64 {
    static LIMIT: OnceLock<u64> = OnceLock::new();
    *LIMIT.get_or_init(|| std::env::var("ALTAIR_HOOK_LOG").ok().map(|v| v.parse().ok().filter(|&n| n > 1).unwrap_or(8)).unwrap_or(0))
}

/// La funzione registrata nella libreria tradotta (`altair_set_hook`).
pub unsafe extern "C" fn dispatch(c: *mut Ctx, addr: u32) -> i32 {
    let c = unsafe { &mut *c };
    let t = table();
    if let Ok(i) = t.binary_search_by_key(&addr, |h| h.addr) {
        let n = t[i].calls.fetch_add(1, Ordering::Relaxed) + 1;
        if n <= log_limit() {
            let mem = crate::machine::Mem(c.mem);
            let arg = |k: u32| mem.r32(c.esp.wrapping_add(4 + 4 * k));
            eprintln!(
                "[aggancio] t={:.2} s {} ({addr:#x}) n. {n}: ecx {:#x}, argomenti {:#x} {:#x} {:#x}, chiamata da {:#x}, thread {}",
                start().elapsed().as_secs_f64(),
                t[i].name,
                c.ecx,
                arg(0),
                arg(1),
                arg(2),
                mem.r32(c.esp),
                std::thread::current().name().unwrap_or("?")
            );
        }
    }
    match REPLACEMENTS.iter().find(|r| r.0 == addr) {
        Some(r) => (r.1)(c) as i32,
        None => 0,
    }
}

/// Resoconto per il comando `hooks`: chiamate totali e dall'ultimo resoconto.
pub fn report() -> String {
    start();
    table()
        .iter()
        .map(|h| {
            let n = h.calls.load(Ordering::Relaxed);
            format!("{} {n} (+{})", h.name, n - h.seen.swap(n, Ordering::Relaxed))
        })
        .collect::<Vec<_>>()
        .join("; ")
}
