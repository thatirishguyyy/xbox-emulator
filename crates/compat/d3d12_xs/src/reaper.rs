use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub(crate) struct Reaper {
    tx: Mutex<Sender<(usize, u64)>>,
    pending: Arc<AtomicUsize>,
    freed: Arc<AtomicU64>,
    worst_us: Arc<AtomicU64>,
}

impl Reaper {
    pub(crate) fn new<D, R>(done: D, release: R) -> Self
    where
        D: Fn() -> u64 + Send + 'static,
        R: Fn(usize) + Send + 'static,
    {
        let (tx, rx) = channel::<(usize, u64)>();
        let pending = Arc::new(AtomicUsize::new(0));
        let freed = Arc::new(AtomicU64::new(0));
        let worst_us = Arc::new(AtomicU64::new(0));
        let (p, f, w) = (pending.clone(), freed.clone(), worst_us.clone());
        std::thread::Builder::new()
            .name("tombox-reaper".into())
            .spawn(move || {
                for (res, after) in rx {
                    while done() < after {
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    let t = std::time::Instant::now();
                    release(res);
                    w.fetch_max(t.elapsed().as_micros() as u64, Ordering::Relaxed);
                    f.fetch_add(1, Ordering::Relaxed);
                    p.fetch_sub(1, Ordering::Release);
                }
            })
            .expect("spawn reaper thread");
        Reaper { tx: Mutex::new(tx), pending, freed, worst_us }
    }

    pub(crate) fn push(&self, res: usize, after_fence: u64) -> bool {
        self.pending.fetch_add(1, Ordering::AcqRel);
        if self.tx.lock().unwrap().send((res, after_fence)).is_err() {
            self.pending.fetch_sub(1, Ordering::AcqRel);
            return false;
        }
        true
    }

    pub(crate) fn pending(&self) -> usize {
        self.pending.load(Ordering::Acquire)
    }

    pub(crate) fn freed(&self) -> u64 {
        self.freed.load(Ordering::Relaxed)
    }

    pub(crate) fn worst_us(&self) -> u64 {
        self.worst_us.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn wait_until(mut f: impl FnMut() -> bool) {
        let t = Instant::now();
        while !f() {
            assert!(t.elapsed() < Duration::from_secs(5), "timed out");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn releases_off_the_caller_thread_in_order() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let l = log.clone();
        let caller = std::thread::current().id();
        let r = Reaper::new(|| u64::MAX, move |res| {
            assert_ne!(std::thread::current().id(), caller);
            std::thread::sleep(Duration::from_millis(20));
            l.lock().unwrap().push(res);
        });
        let t = Instant::now();
        for k in 1..=5 {
            assert!(r.push(k, 0));
        }
        assert!(t.elapsed() < Duration::from_millis(15), "push blocked on the release");
        wait_until(|| r.pending() == 0);
        assert_eq!(*log.lock().unwrap(), vec![1, 2, 3, 4, 5]);
        assert_eq!(r.freed(), 5);
        assert!(r.worst_us() >= 20_000);
    }

    #[test]
    fn waits_for_the_fence_before_releasing() {
        let fence = Arc::new(AtomicU64::new(3));
        let log = Arc::new(Mutex::new(Vec::new()));
        let (fd, l) = (fence.clone(), log.clone());
        let r = Reaper::new(move || fd.load(Ordering::Acquire), move |res| l.lock().unwrap().push(res));
        r.push(10, 2);
        r.push(11, 5);
        wait_until(|| log.lock().unwrap().len() == 1);
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(*log.lock().unwrap(), vec![10]);
        assert_eq!(r.pending(), 1);
        fence.store(5, Ordering::Release);
        wait_until(|| r.pending() == 0);
        assert_eq!(*log.lock().unwrap(), vec![10, 11]);
    }
}
