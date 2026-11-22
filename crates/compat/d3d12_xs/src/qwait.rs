use core::ffi::c_void;
use core::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pending {
    pub fence: usize,
    pub value: u64,
    pub since: i64,
}

#[derive(Default, Debug, PartialEq)]
pub struct Tally {
    pub waits: u64,
    pub blocked: u64,
    pub resolved: u64,
    pub resolved_ticks: i64,
    pub max_ticks: i64,
}

#[derive(Default)]
pub struct Book {
    pending: Vec<Pending>,
    tally: Tally,
}

const CAP: usize = 4096;

impl Book {
    pub fn note(&mut self, fence: usize, value: u64, completed: u64, now: i64) {
        self.tally.waits += 1;
        if completed >= value {
            return;
        }
        self.tally.blocked += 1;
        if self.pending.len() < CAP {
            self.pending.push(Pending { fence, value, since: now });
        }
    }

    pub fn poll(&mut self, now: i64, completed: impl Fn(usize) -> u64) {
        let t = &mut self.tally;
        self.pending.retain(|p| {
            if completed(p.fence) < p.value {
                return true;
            }
            let d = now - p.since;
            t.resolved += 1;
            t.resolved_ticks += d;
            t.max_ticks = t.max_ticks.max(d);
            false
        });
    }

    pub fn take(&mut self) -> (Tally, usize) {
        (core::mem::take(&mut self.tally), self.pending.len())
    }
}

static BOOK: Mutex<Option<Book>> = Mutex::new(None);
pub static INTERNAL: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
pub const STEP_ADDR: usize = 0;
pub const STEP_COND: usize = 1;
pub const STEP_POLL: usize = 2;
pub const STEP_PROBE: usize = 3;

fn with<R>(f: impl FnOnce(&mut Book) -> R) -> R {
    f(BOOK.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(Book::default))
}

pub fn step(kind: usize) {
    INTERNAL[kind].fetch_add(1, Ordering::Relaxed);
}

unsafe fn completed(fence: usize) -> u64 {
    let vt = unsafe { *(fence as *const *const usize) };
    let f: unsafe extern "system" fn(*mut c_void) -> u64 = unsafe { core::mem::transmute(*vt.add(8)) };
    unsafe { f(fence as *mut c_void) }
}

pub fn poll() {
    let now = crate::frame::qpc();
    with(|b| b.poll(now, |f| unsafe { completed(f) }));
}

pub fn report(frames: u64) -> String {
    poll();
    let (t, open) = with(|b| b.take());
    let f = crate::frame::qpf() as f64;
    let ms = |x: i64| x as f64 * 1000.0 / f;
    let n: Vec<u64> = INTERNAL.iter().map(|a| a.swap(0, Ordering::Relaxed)).collect();
    let fr = frames.max(1) as f64;
    format!(
        " qwait=title {:.1}/f blocked {:.1}/f resolved-after {:.2}ms avg {:.1}ms max open {open} | internal addr {:.1}/f cond {:.1}/f poll {:.1}/f probe {:.1}/f",
        t.waits as f64 / fr, t.blocked as f64 / fr,
        if t.resolved > 0 { ms(t.resolved_ticks) / t.resolved as f64 } else { 0.0 }, ms(t.max_ticks),
        n[STEP_ADDR] as f64 / fr, n[STEP_COND] as f64 / fr, n[STEP_POLL] as f64 / fr, n[STEP_PROBE] as f64 / fr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn satisfied_waits_are_counted_but_not_tracked() {
        let mut b = Book::default();
        b.note(1, 5, 5, 100);
        b.note(1, 6, 5, 100);
        let (t, open) = b.take();
        assert_eq!((t.waits, t.blocked, open), (2, 1, 1));
    }

    #[test]
    fn blocked_waits_resolve_with_their_latency() {
        let mut b = Book::default();
        b.note(1, 10, 3, 100);
        b.note(2, 4, 0, 150);
        b.poll(200, |f| if f == 1 { 10 } else { 3 });
        b.poll(400, |_| 10);
        let (t, open) = b.take();
        assert_eq!(open, 0);
        assert_eq!(t.resolved, 2);
        assert_eq!(t.resolved_ticks, 100 + 250);
        assert_eq!(t.max_ticks, 250);
        let (t, _) = b.take();
        assert_eq!(t, Tally::default());
    }
}
