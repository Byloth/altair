//! Kernel objects that can be waited on: events, semaphores, threads.

use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

pub enum Kind {
    Event { manual: bool },
    Semaphore { max: i64 },
    Thread,
}

/// `state` is: for an event 0/1 (signalled); for a semaphore the count; for a thread 1 when
/// it has terminated.
pub struct Object {
    pub kind: Kind,
    state: Mutex<i64>,
    cv: Condvar,
    /// Thread: exit code. Otherwise unused.
    pub exit_code: Mutex<u32>,
    /// Thread created suspended: it starts when this becomes true.
    pub gate: Mutex<bool>,
    pub gate_cv: Condvar,
}

impl Object {
    pub fn new(kind: Kind, initial: i64) -> Object {
        Object {
            kind,
            state: Mutex::new(initial),
            cv: Condvar::new(),
            exit_code: Mutex::new(259), // STILL_ACTIVE
            gate: Mutex::new(true),
            gate_cv: Condvar::new(),
        }
    }

    /// Tries to take the object without waiting.
    fn take(&self, s: &mut i64) -> bool {
        if *s <= 0 {
            return false;
        }
        match self.kind {
            Kind::Event { manual: true } | Kind::Thread => {}
            Kind::Event { manual: false } => *s = 0,
            Kind::Semaphore { .. } => *s -= 1,
        }
        true
    }

    pub fn try_wait(&self) -> bool {
        let mut s = self.state.lock().unwrap();
        self.take(&mut s)
    }

    /// Waits at most `ms` milliseconds (`None` = forever). True if the object was taken.
    pub fn wait(&self, ms: Option<u32>) -> bool {
        let deadline = ms.map(|m| Instant::now() + Duration::from_millis(m as u64));
        let mut s = self.state.lock().unwrap();
        loop {
            if self.take(&mut s) {
                return true;
            }
            match deadline {
                None => s = self.cv.wait(s).unwrap(),
                Some(d) => {
                    let now = Instant::now();
                    if now >= d {
                        return false;
                    }
                    s = self.cv.wait_timeout(s, d - now).unwrap().0;
                }
            }
        }
    }

    pub fn set(&self) {
        *self.state.lock().unwrap() = 1;
        self.cv.notify_all();
    }

    pub fn reset(&self) {
        *self.state.lock().unwrap() = 0;
    }

    /// `ReleaseSemaphore`: returns the previous count, or None if it would exceed the maximum.
    pub fn release(&self, n: i64) -> Option<i64> {
        let mut s = self.state.lock().unwrap();
        let Kind::Semaphore { max } = self.kind else { return None };
        if *s + n > max {
            return None;
        }
        let prev = *s;
        *s += n;
        self.cv.notify_all();
        Some(prev)
    }
}
