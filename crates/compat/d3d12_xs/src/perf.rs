use core::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::Mutex;
use compat_common::record;
use crate::frame::{qpc, qpf};

pub const UPLOAD: usize = 0;
pub const RTCAP: usize = 1;
pub const COPY: usize = 2;
pub const SHADER: usize = 3;
pub const PSO: usize = 4;
pub const PWAIT: usize = 5;
pub const PFLIP: usize = 6;
pub const OVERLAY: usize = 7;
const SLOTS: usize = 8;
const NAMES: [&str; SLOTS] = ["upload", "rtcap", "present_copy", "shader_xlat", "pso_create", "present_wait", "present_flip", "overlay"];
const RING: usize = 1024;
const WINDOW_SECONDS: i64 = 5;

static TICKS: [AtomicU64; SLOTS] = [const { AtomicU64::new(0) }; SLOTS];
static MAXT: [AtomicU64; SLOTS] = [const { AtomicU64::new(0) }; SLOTS];
static COUNT: [AtomicU64; SLOTS] = [const { AtomicU64::new(0) }; SLOTS];
static ORIGIN_WAIT: AtomicU64 = AtomicU64::new(0);
static ORIGIN_WAITS: AtomicU64 = AtomicU64::new(0);
static ORIGIN_LAST: AtomicI64 = AtomicI64::new(0);
static ORIGIN_LATE_MAX: AtomicI64 = AtomicI64::new(0);
static ORIGIN_TICKS: AtomicU64 = AtomicU64::new(0);

struct Window {
    start: i64,
    last_present: i64,
    present_int: Vec<f32>,
    frame_loop: Vec<f32>,
    frames: u64,
}

static WIN: Mutex<Window> = Mutex::new(Window { start: 0, last_present: 0, present_int: Vec::new(), frame_loop: Vec::new(), frames: 0 });
static WAS: AtomicBool = AtomicBool::new(true);
static LIVE: crate::live_sw::Sw = crate::live_sw::Sw::new("TOMBOX_PERF", "perf", true);

pub fn enabled() -> bool {
    static DEFAULT: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let d = *DEFAULT.get_or_init(|| compat_common::cfg("perf").as_deref() != Some("0"));
    let on = LIVE.on_or(d);
    if WAS.swap(on, Ordering::Relaxed) != on && on {
        let mut w = WIN.lock().unwrap();
        w.start = 0;
        w.present_int.clear();
        w.frame_loop.clear();
        w.frames = 0;
    }
    on
}

pub fn add(slot: usize, ticks: i64) {
    let t = ticks.max(0) as u64;
    TICKS[slot].fetch_add(t, Ordering::Relaxed);
    COUNT[slot].fetch_add(1, Ordering::Relaxed);
    MAXT[slot].fetch_max(t, Ordering::Relaxed);
}

pub fn timed<R>(slot: usize, f: impl FnOnce() -> R) -> R {
    if !enabled() {
        return f();
    }
    let t0 = qpc();
    let r = f();
    add(slot, qpc() - t0);
    r
}

pub fn on_origin_wait(ticks: i64, now: i64) {
    ORIGIN_WAIT.fetch_add(ticks.max(0) as u64, Ordering::Relaxed);
    ORIGIN_WAITS.fetch_add(1, Ordering::Relaxed);
    let prev = ORIGIN_LAST.swap(now, Ordering::Relaxed);
    if prev != 0 && enabled() {
        let ms = (now - prev) as f64 * 1000.0 / qpf() as f64;
        let mut w = WIN.lock().unwrap();
        if w.frame_loop.len() < RING {
            w.frame_loop.push(ms as f32);
        }
    }
}

pub fn on_frame_event(ty: usize, _now: i64, late: i64) {
    if ty == 0 {
        ORIGIN_TICKS.fetch_add(1, Ordering::Relaxed);
        ORIGIN_LATE_MAX.fetch_max(late, Ordering::Relaxed);
    }
}

pub fn percentile(v: &mut [f32], p: f64) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
    let idx = ((v.len() as f64 - 1.0) * p).round() as usize;
    v[idx.min(v.len() - 1)]
}

fn ms(ticks: u64) -> f64 {
    ticks as f64 * 1000.0 / qpf() as f64
}

pub fn on_present() {
    if !enabled() {
        return;
    }
    let now = qpc();
    let f = qpf();
    let mut w = WIN.lock().unwrap();
    if w.start == 0 {
        w.start = now;
        w.last_present = now;
        return;
    }
    let dt = (now - w.last_present) as f64 * 1000.0 / f as f64;
    w.last_present = now;
    w.frames += 1;
    if w.present_int.len() < RING {
        w.present_int.push(dt as f32);
    }
    if now - w.start < WINDOW_SECONDS * f {
        return;
    }
    let secs = (now - w.start) as f64 / f as f64;
    let frames = w.frames.max(1);
    let fps = w.frames as f64 / secs;
    let mut pi = core::mem::take(&mut w.present_int);
    let mut fl = core::mem::take(&mut w.frame_loop);
    w.start = now;
    w.frames = 0;
    drop(w);
    let mean_pi = if pi.is_empty() { 0.0 } else { pi.iter().map(|&x| x as f64).sum::<f64>() / pi.len() as f64 };
    let p50 = percentile(&mut pi, 0.5);
    let p99 = percentile(&mut pi, 0.99);
    let maxpi = pi.last().copied().unwrap_or(0.0);
    let fl50 = percentile(&mut fl, 0.5);
    let fl99 = percentile(&mut fl, 0.99);
    let waits = ORIGIN_WAITS.swap(0, Ordering::Relaxed).max(1);
    let wait_ms = ms(ORIGIN_WAIT.swap(0, Ordering::Relaxed)) / waits as f64;
    let origin_ticks = ORIGIN_TICKS.swap(0, Ordering::Relaxed);
    let late_us = ORIGIN_LATE_MAX.swap(0, Ordering::Relaxed) as f64 * 1e6 / f as f64;
    let mut layers = String::new();
    for s in 0..SLOTS {
        let t = TICKS[s].swap(0, Ordering::Relaxed);
        let n = COUNT[s].swap(0, Ordering::Relaxed);
        let mx = MAXT[s].swap(0, Ordering::Relaxed);
        if n == 0 {
            continue;
        }
        if s == SHADER || s == PSO {
            layers.push_str(&format!(" {}={}x/{:.1}ms(max {:.1})", NAMES[s], n, ms(t), ms(mx)));
        } else {
            layers.push_str(&format!(" {}={:.2}ms/f(max {:.1})", NAMES[s], ms(t) / frames as f64, ms(mx)));
        }
    }
    layers.push_str(&crate::qwait::report(frames));
    record("d3d12_xs", "[perf]", &format!(
        "fps={fps:.1} ft_p50={fl50:.2}ms ft_p99={fl99:.2}ms present_int={mean_pi:.2}ms (p50 {p50:.2} p99 {p99:.2} max {maxpi:.1}) origin_wait={wait_ms:.2}ms/f origin_ticks={origin_ticks} clock_late_max={late_us:.0}us interval={}us host={}.{:03}Hz{layers}",
        crate::frame::interval_us(), crate::frame::host_refresh_mhz() / 1000, crate::frame::host_refresh_mhz() % 1000));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles() {
        let mut v: Vec<f32> = (1..=100).map(|x| x as f32).collect();
        assert_eq!(percentile(&mut v, 0.5), 51.0);
        assert_eq!(percentile(&mut v, 0.99), 99.0);
        assert_eq!(percentile(&mut [], 0.5), 0.0);
    }
}
