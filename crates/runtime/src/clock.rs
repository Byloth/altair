//! The clock the game reads, with a debug fast-forward.
//!
//! `QueryPerformanceCounter` and `GetTickCount` do not return real time but this virtual time,
//! which normally advances with it. With a speed factor above one the game believes more time
//! has passed between two frames and plays everything faster: a way to get through long
//! non-interactive scenes while testing (F9, the `speed N` control-channel command, or
//! `ALTAIR_SPEED=N` at start-up). Everything the game uses to measure time follows the factor:
//! the clocks, `Sleep` and timed waits (win32.rs) and the progress of sound buffers (dsound.rs,
//! silent meanwhile). Bink videos do not. It is a debugging
//! aid, not a feature.

use std::sync::Mutex;
use std::time::Instant;

struct State {
    /// Real instant and virtual nanoseconds at the last speed change.
    anchor: Instant,
    virtual_ns: u128,
    speed: u32,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

/// The factor F9 switches to.
pub const FAST: u32 = 8;

fn with<T>(f: impl FnOnce(&mut State) -> T) -> T {
    let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let state = guard.get_or_insert_with(|| {
        let speed = std::env::var("ALTAIR_SPEED").ok().and_then(|v| v.parse().ok()).filter(|&n| (1..=64).contains(&n)).unwrap_or(1);
        State { anchor: Instant::now(), virtual_ns: 0, speed }
    });
    f(state)
}

/// Virtual nanoseconds since the process started.
pub fn now_ns() -> u128 {
    with(|s| s.virtual_ns + s.anchor.elapsed().as_nanos() * s.speed as u128)
}

pub fn speed() -> u32 {
    with(|s| s.speed)
}

/// Changes the factor (1 = real time) without making the virtual time jump.
pub fn set_speed(speed: u32) {
    let speed = speed.clamp(1, 64);
    with(|s| {
        s.virtual_ns += s.anchor.elapsed().as_nanos() * s.speed as u128;
        s.anchor = Instant::now();
        s.speed = speed;
    });
    eprintln!("[clock] game time at {speed}x");
}

/// F9: between real time and the fast factor.
pub fn toggle_fast() {
    set_speed(if speed() == 1 { FAST } else { 1 });
}
