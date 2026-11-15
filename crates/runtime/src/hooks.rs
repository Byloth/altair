//! Hooks: functions of the translated game that call the runtime on entry.
//!
//! The list is in `runtime/hooks.txt`: the translator puts at the top of those functions
//! `if (altair_hook(c, address)) return;` and the call arrives here. Two uses:
//! - **observing**: every hook counts its calls (`hooks` command of the control channel) and,
//!   with `ALTAIR_HOOK_LOG=1`, writes the first ones of each to the log with time and arguments;
//! - **replacing**: an entry of `REPLACEMENTS` can do the work in place of the function and
//!   return `true`, after leaving registers and stack as after the original `ret`.
//!
//! Hooks run on the game's threads, without the runtime lock: no shared state
//! that is not atomic.

use crate::machine::Ctx;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

struct Hook {
    addr: u32,
    name: &'static str,
    calls: AtomicU64,
    /// Calls already reported by the last `hooks` command.
    seen: AtomicU64,
}

/// Who steps in on a function, by address. Returns `true` only if it has replaced it
/// entirely; one that merely tweaks the arguments returns `false` and the function carries on.
const REPLACEMENTS: &[(u32, fn(&mut Ctx) -> bool)] = &[(0x8a_1920, animus_version)];

/// Guest memory of our own for the replaced texts: `SLOTS` slots of `SLOT` bytes, used in rotation
/// (the hooked function copies the text right away, the slot is needed only for that instant).
static SCRATCH: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static NEXT: AtomicU64 = AtomicU64::new(0);
const SLOT: u32 = 512;
const SLOTS: u32 = 16;
pub const SCRATCH_SIZE: u32 = SLOT * SLOTS;

pub fn set_scratch(addr: u32) {
    SCRATCH.store(addr, Ordering::Relaxed);
}

/// The Animus version shown by the game. The interface texts all go through
/// `0x8A1920` (output string, zero-terminated UTF-16 text) as soon as they are read from the data:
/// if the text is "… Animus … 1.28", in any language, the function receives a copy of it with
/// our version. The Animus font only covers Latin-1: no Greek letters.
/// `ALTAIR_ANIMUS_VERSION` changes it; empty, it leaves the original.
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

/// The function registered in the translated library (`altair_set_hook`).
pub unsafe extern "C" fn dispatch(c: *mut Ctx, addr: u32) -> i32 {
    let c = unsafe { &mut *c };
    let t = table();
    if let Ok(i) = t.binary_search_by_key(&addr, |h| h.addr) {
        let n = t[i].calls.fetch_add(1, Ordering::Relaxed) + 1;
        if n <= log_limit() {
            let mem = crate::machine::Mem(c.mem);
            let arg = |k: u32| mem.r32(c.esp.wrapping_add(4 + 4 * k));
            eprintln!(
                "[hook] t={:.2} s {} ({addr:#x}) no. {n}: ecx {:#x}, arguments {:#x} {:#x} {:#x}, called from {:#x}, thread {}",
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

/// Report for the `hooks` command: total calls and calls since the last report.
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
