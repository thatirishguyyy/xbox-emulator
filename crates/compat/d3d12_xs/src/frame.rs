use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use compat_common::record;

extern "system" {
    fn CreateSemaphoreW(sa: *mut c_void, initial: i32, max: i32, name: *const u16) -> isize;
    fn CreateEventW(sa: *mut c_void, manual: i32, initial: i32, name: *const u16) -> isize;
    fn ReleaseSemaphore(h: isize, n: i32, prev: *mut i32) -> i32;
    fn SetEvent(h: isize) -> i32;
    fn CloseHandle(h: isize) -> i32;
    fn DuplicateHandle(sp: isize, sh: isize, tp: isize, th: *mut isize, access: u32, inherit: i32, options: u32) -> i32;
    fn GetCurrentProcess() -> isize;
    fn GetCurrentThread() -> isize;
    fn WaitForMultipleObjects(n: u32, h: *const isize, all: i32, ms: u32) -> u32;
    fn QueryPerformanceCounter(v: *mut i64) -> i32;
    fn QueryPerformanceFrequency(v: *mut i64) -> i32;
    fn CreateWaitableTimerExW(sa: *mut c_void, name: *const u16, flags: u32, access: u32) -> isize;
    fn SetWaitableTimer(h: isize, due: *const i64, period: i32, cb: *const c_void, arg: *const c_void, resume: i32) -> i32;
    fn SetThreadPriority(h: isize, p: i32) -> i32;
    fn IsBadReadPtr(p: *const c_void, n: usize) -> i32;
}

#[link(name = "dwmapi")]
extern "system" {
    fn DwmGetCompositionTimingInfo(hwnd: isize, info: *mut u8) -> i32;
}

pub const S_OK: i32 = 0;
pub const S_FALSE: i32 = 1;
pub const HRESULT_FROM_NT_SUCCESS: i32 = 0x1000_0000;
pub const E_FAIL: i32 = 0x8000_4005u32 as i32;
pub const E_INVALIDARG: i32 = 0x8007_0057u32 as i32;
const WAIT_TIMEOUT: u32 = 0x102;
const TYPES: usize = 3;
const ORIGIN_SEMAPHORE_MAX: i32 = 4;
const FLAG_WAIT_ALL: u32 = 1;
const FLAG_PROCESS_TOKEN_ON_TIMEOUT: u32 = 2;
const FLAG_UNSCHEDULE: u32 = 1;
const TOKEN_MASK: u64 = 0xffff_ffff_ffff;
const KNOWN_TOKENS: usize = 64;

pub fn interval_valid(len_us: u32, high_refresh: bool) -> bool {
    let within = |base: u32, span: u32| len_us >= base && len_us - base <= span;
    if within(8333, 9) || within(25000, 25) {
        return high_refresh;
    }
    within(16667, 16) || within(33333, 34) || within(50000, 49) || within(66667, 67)
}

pub fn vblanks_per_interval(len_us: u32) -> (u32, bool) {
    let within = |base: u32, span: u32| len_us >= base && len_us - base <= span;
    if within(16667, 16) { (1, false) }
    else if within(33333, 34) { (2, false) }
    else if within(50000, 49) { (3, false) }
    else if within(66667, 67) { (4, false) }
    else if within(8333, 9) { (1, true) }
    else if within(25000, 25) { (3, true) }
    else { (1, false) }
}

pub fn next_deadline(base: i64, interval: i64, offset: i64, after: i64) -> (i64, i64) {
    if interval <= 0 {
        return (i64::MAX, 0);
    }
    let first = base + offset;
    if after < first {
        return (first, 0);
    }
    let k = (after - first) / interval + 1;
    (first + k * interval, k)
}

struct TypeState {
    internal: isize,
    offset_us: u32,
    dups: Vec<isize>,
    scheduled: bool,
}

struct Pacing {
    interval_us: u32,
    period: u32,
    flags: u32,
    origin_offset: i64,
    active: bool,
    types: [TypeState; TYPES],
    generation: u64,
    base_qpc: i64,
    interval_qpc: i64,
    host_locked: bool,
    host_refresh_mhz: u32,
}

unsafe impl Send for Pacing {}

static STATE: Mutex<Option<Pacing>> = Mutex::new(None);
static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);
static ISSUED: Mutex<[u64; KNOWN_TOKENS]> = Mutex::new([0; KNOWN_TOKENS]);
static ISSUED_POS: AtomicU64 = AtomicU64::new(0);
static FLIPPED: Mutex<Vec<(u64, i64, i64)>> = Mutex::new(Vec::new());
static LAST_ORIGIN_QPC: AtomicI64 = AtomicI64::new(0);
static WAKE: OnceLock<isize> = OnceLock::new();
static TIMER_STARTED: AtomicBool = AtomicBool::new(false);
static PRESENTED: AtomicBool = AtomicBool::new(false);
const FLIPPED_KEEP: usize = 64;

pub fn qpc() -> i64 {
    let mut v = 0i64;
    unsafe { QueryPerformanceCounter(&mut v) };
    v
}

pub fn qpf() -> i64 {
    static F: OnceLock<i64> = OnceLock::new();
    *F.get_or_init(|| {
        let mut v = 0i64;
        unsafe { QueryPerformanceFrequency(&mut v) };
        v.max(1)
    })
}

fn us_to_ticks(us: i64) -> i64 {
    us * qpf() / 1_000_000
}

struct HostVblank {
    refresh_mhz: u32,
    vblank_qpc: i64,
    period_qpc: i64,
}

fn host_vblank() -> Option<HostVblank> {
    let mut info = [0u8; 292];
    info[0..4].copy_from_slice(&292u32.to_le_bytes());
    if unsafe { DwmGetCompositionTimingInfo(0, info.as_mut_ptr()) } < 0 {
        return None;
    }
    let u32at = |o: usize| u32::from_le_bytes(info[o..o + 4].try_into().unwrap());
    let u64at = |o: usize| u64::from_le_bytes(info[o..o + 8].try_into().unwrap());
    let (num, den) = (u32at(4) as u64, u32at(8) as u64);
    let refresh_mhz = if den == 0 { 0 } else { (num * 1000 / den) as u32 };
    let period = u64at(12) as i64;
    let vblank = u64at(28) as i64;
    if period <= 0 || vblank <= 0 {
        return None;
    }
    Some(HostVblank { refresh_mhz, vblank_qpc: vblank, period_qpc: period })
}

pub fn host_lock(interval_us: u32, host_period_qpc: i64, freq: i64) -> Option<i64> {
    if host_period_qpc <= 0 || freq <= 0 {
        return None;
    }
    let interval_qpc = interval_us as i64 * freq / 1_000_000;
    let n = (interval_qpc + host_period_qpc / 2) / host_period_qpc;
    if n < 1 || (n * host_period_qpc - interval_qpc).abs() * 100 > interval_qpc {
        return None;
    }
    Some(n * host_period_qpc)
}

fn new_pacing() -> Pacing {
    let mk = |i: usize| -> TypeState {
        let h = unsafe {
            if i == 0 {
                CreateSemaphoreW(core::ptr::null_mut(), 0, ORIGIN_SEMAPHORE_MAX, core::ptr::null())
            } else {
                CreateEventW(core::ptr::null_mut(), 0, 0, core::ptr::null())
            }
        };
        TypeState { internal: h, offset_us: 0, dups: Vec::new(), scheduled: false }
    };
    let hv = host_vblank();
    Pacing {
        interval_us: 0,
        period: 0,
        flags: 0,
        origin_offset: -1,
        active: false,
        types: [mk(0), mk(1), mk(2)],
        generation: 0,
        base_qpc: 0,
        interval_qpc: 0,
        host_locked: false,
        host_refresh_mhz: hv.map(|v| v.refresh_mhz).unwrap_or(0),
    }
}

fn with<R>(f: impl FnOnce(&mut Pacing) -> R) -> R {
    let mut g = STATE.lock().unwrap();
    f(g.get_or_insert_with(new_pacing))
}

fn wake_event() -> isize {
    *WAKE.get_or_init(|| unsafe { CreateEventW(core::ptr::null_mut(), 0, 0, core::ptr::null()) })
}

fn kick_timer() {
    unsafe { SetEvent(wake_event()) };
}

unsafe fn signal(h: isize) {
    if h == 0 {
        return;
    }
    if SetEvent(h) == 0 {
        ReleaseSemaphore(h, 1, core::ptr::null_mut());
    }
}

fn high_refresh() -> bool {
    with(|p| p.host_refresh_mhz >= 119_000)
}

struct Plan {
    generation: u64,
    interval: i64,
    base: i64,
    events: Vec<(usize, i64, isize, Vec<isize>)>,
}

fn plan() -> Option<Plan> {
    with(|p| {
        if !p.active || p.interval_us == 0 {
            return None;
        }
        let events: Vec<(usize, i64, isize, Vec<isize>)> = p.types.iter().enumerate()
            .filter(|(_, t)| t.scheduled)
            .map(|(i, t)| (i, us_to_ticks(t.offset_us as i64), t.internal, t.dups.clone()))
            .collect();
        if events.is_empty() {
            return None;
        }
        let interval = if p.interval_qpc > 0 { p.interval_qpc } else { us_to_ticks(p.interval_us as i64) };
        let (interval, events) = crate::frame_fast::retime(interval, events);
        Some(Plan { generation: p.generation, interval, base: p.base_qpc, events })
    })
}

fn resync_to_host() {
    let Some(v) = host_vblank() else { return };
    crate::frame_fast::host_period(v.period_qpc, v.refresh_mhz);
    with(|p| {
        if !p.host_locked || !p.active {
            return;
        }
        if let Some(iv) = host_lock(p.interval_us, v.period_qpc, qpf()) {
            p.interval_qpc = iv;
            p.base_qpc = v.vblank_qpc;
            p.host_refresh_mhz = v.refresh_mhz;
        }
    });
}

fn sleep_until(timer: isize, wake: isize, deadline: i64) -> bool {
    const SPIN_US: i64 = 600;
    loop {
        let now = qpc();
        let left = deadline - now;
        if left <= 0 {
            return true;
        }
        let left_us = left * 1_000_000 / qpf();
        if left_us > SPIN_US {
            let due = -((left_us - SPIN_US) * 10);
            unsafe { SetWaitableTimer(timer, &due, 0, core::ptr::null(), core::ptr::null(), 0) };
            let hs = [timer, wake];
            let r = unsafe { WaitForMultipleObjects(2, hs.as_ptr(), 0, u32::MAX) };
            if r == 1 {
                return false;
            }
        } else {
            core::hint::spin_loop();
        }
    }
}

fn timer_thread() {
    unsafe { SetThreadPriority(GetCurrentThread(), 15) };
    let timer = unsafe { CreateWaitableTimerExW(core::ptr::null_mut(), core::ptr::null(), 0x2, 0x1f0003) };
    let timer = if timer != 0 { timer } else { unsafe { CreateWaitableTimerExW(core::ptr::null_mut(), core::ptr::null(), 0, 0x1f0003) } };
    let wake = wake_event();
    let mut last: [i64; TYPES] = [i64::MIN / 2; TYPES];
    let mut origins = 0u64;
    loop {
        let Some(pl) = plan() else {
            unsafe { WaitForMultipleObjects(1, &wake, 0, u32::MAX) };
            continue;
        };
        let uncapped = crate::frame_fast::uncapped();
        if uncapped && PRESENTED.swap(false, Ordering::AcqRel) {
            let mut order: Vec<usize> = (0..pl.events.len()).collect();
            order.sort_by_key(|&i| (pl.events[i].0 != 0, pl.events[i].1));
            for i in order {
                let (ty, _, internal, dups) = &pl.events[i];
                if !with(|p| p.generation == pl.generation && p.types[*ty].scheduled) {
                    continue;
                }
                let t = qpc();
                last[*ty] = t;
                fire(*ty, *internal, dups, t, t, &mut origins);
            }
            continue;
        }
        let now = qpc();
        let mut best: Option<(i64, usize)> = None;
        for (i, (ty, off, _, _)) in pl.events.iter().enumerate() {
            let (mut d, _) = next_deadline(pl.base, pl.interval, *off, now - 1);
            if uncapped && last[*ty] > i64::MIN / 2 {
                d = (last[*ty] + pl.interval).max(now);
            } else if d - last[*ty] < pl.interval / 2 {
                d += pl.interval;
            }
            if best.map(|b| d < b.0).unwrap_or(true) {
                best = Some((d, i));
            }
        }
        let Some((deadline, idx)) = best else { continue };
        if !sleep_until(timer, wake, deadline) {
            continue;
        }
        let (ty, _, internal, dups) = &pl.events[idx];
        let still = with(|p| p.generation == pl.generation && p.types[*ty].scheduled);
        if !still {
            continue;
        }
        last[*ty] = if uncapped { qpc() } else { deadline };
        fire(*ty, *internal, dups, qpc(), deadline, &mut origins);
    }
}

fn fire(ty: usize, internal: isize, dups: &[isize], t: i64, deadline: i64, origins: &mut u64) {
    if ty == 0 {
        *origins += 1;
        if *origins % 120 == 0 {
            resync_to_host();
        }
        LAST_ORIGIN_QPC.store(t, Ordering::Relaxed);
    }
    unsafe {
        if ty == 0 {
            ReleaseSemaphore(internal, 1, core::ptr::null_mut());
        } else {
            SetEvent(internal);
        }
        for &h in dups {
            signal(h);
        }
    }
    crate::perf::on_frame_event(ty, t, t - deadline);
}

pub fn on_present() {
    if TIMER_STARTED.load(Ordering::Relaxed) && crate::frame_fast::uncapped() {
        PRESENTED.store(true, Ordering::Release);
        kick_timer();
    }
}

fn ensure_timer() {
    if !TIMER_STARTED.swap(true, Ordering::SeqCst) {
        let _ = std::thread::Builder::new().name("tombox-frame-clock".into()).spawn(timer_thread);
    }
}

fn start_clock(p: &mut Pacing) {
    let hv = host_vblank();
    if let Some(v) = hv.as_ref() {
        crate::frame_fast::host_period(v.period_qpc, v.refresh_mhz);
    }
    p.host_refresh_mhz = hv.as_ref().map(|v| v.refresh_mhz).unwrap_or(p.host_refresh_mhz);
    let lock = hv.as_ref().and_then(|v| host_lock(p.interval_us, v.period_qpc, qpf()));
    let aligned = lock.is_some();
    match (&hv, lock) {
        (Some(v), Some(iv)) => {
            p.base_qpc = v.vblank_qpc;
            p.interval_qpc = iv;
        }
        _ => {
            p.base_qpc = qpc();
            p.interval_qpc = us_to_ticks(p.interval_us as i64);
        }
    }
    p.host_locked = aligned;
    p.active = true;
    let (vb, half) = vblanks_per_interval(p.interval_us);
    record("d3d12_xs", "frame.clock", &format!(
        "frame clock started: interval={}us periodicity={} flags={:#x} vblanks/interval={vb}{} host refresh={}.{:03}Hz phase={}",
        p.interval_us, p.period, p.flags, if half { " (120Hz base)" } else { "" },
        p.host_refresh_mhz / 1000, p.host_refresh_mhz % 1000,
        if aligned { "locked to host vblank" } else { "free-running (host refresh not a multiple of the interval)" }));
}

pub unsafe extern "system" fn set_frame_interval_x(_this: *mut c_void, _output: *mut c_void, len_us: u32, periodicity: u32, flags: u32) -> i32 {
    if !interval_valid(len_us, high_refresh()) && len_us != 0 {
        record("d3d12_xs", "SetFrameIntervalX", &format!("length={len_us}us periodicity={periodicity} flags={flags:#x} -> E_INVALIDARG (not a supported interval)"));
        return E_INVALIDARG;
    }
    let hr = with(|p| {
        let changed = p.interval_us != len_us;
        if changed {
            p.types[0].scheduled = false;
        }
        p.interval_us = len_us;
        if changed && p.active && len_us != 0 {
            let lock = host_vblank().and_then(|v| host_lock(len_us, v.period_qpc, qpf()));
            p.host_locked = lock.is_some();
            p.interval_qpc = lock.unwrap_or_else(|| us_to_ticks(len_us as i64));
        }
        if len_us != 0 {
            p.flags = flags;
            p.period = periodicity;
            if p.origin_offset != -1 {
                p.origin_offset %= len_us as i64;
            }
        } else {
            p.period = 0;
            p.flags = 0;
            p.origin_offset = -1;
            p.active = false;
            for t in p.types.iter_mut() {
                t.scheduled = false;
            }
        }
        p.generation += 1;
        S_FALSE
    });
    kick_timer();
    record("d3d12_xs", "SetFrameIntervalX", &format!("length={len_us}us periodicity={periodicity} flags={flags:#x} -> {hr:#x}"));
    hr
}

unsafe fn read_ancillary(anc: *const u8) -> Result<Vec<isize>, i32> {
    if anc.is_null() {
        return Ok(Vec::new());
    }
    if IsBadReadPtr(anc as *const c_void, 16) != 0 {
        return Err(E_INVALIDARG);
    }
    let count = core::ptr::read_unaligned(anc as *const u32) as usize;
    if count == 0 {
        return Ok(Vec::new());
    }
    let list = core::ptr::read_unaligned(anc.add(8) as *const usize);
    if count > 63 || list < 0x10000 || IsBadReadPtr(list as *const c_void, count * 8) != 0 {
        return Err(E_INVALIDARG);
    }
    Ok((0..count).map(|i| core::ptr::read_unaligned((list as *const isize).add(i))).collect())
}

unsafe fn write_signaled_index(anc: *const u8, idx: i32) {
    if anc.is_null() || IsBadReadPtr(anc as *const c_void, 0x18) != 0 {
        return;
    }
    let out = core::ptr::read_unaligned(anc.add(0x10) as *const usize);
    if out >= 0x10000 && IsBadReadPtr(out as *const c_void, 4) == 0 {
        core::ptr::write_unaligned(out as *mut i32, idx);
    }
}

pub unsafe extern "system" fn schedule_frame_event_x(_this: *mut c_void, ty: u32, offset_us: u32, anc: *const u8, flags: u32) -> i32 {
    let handles = read_ancillary(anc);
    record("d3d12_xs", "ScheduleFrameEventX", &format!(
        "type={ty} offset={offset_us}us ancillary={} flags={flags:#x}",
        match &handles { Ok(v) => format!("{v:x?}"), Err(_) => "unreadable".into() }));
    let unschedule = flags & FLAG_UNSCHEDULE != 0;
    let pre = with(|p| {
        if p.interval_us == 0 || ty as usize >= TYPES {
            return Err(E_INVALIDARG);
        }
        if !p.active && ty != 0 {
            return Err(E_INVALIDARG);
        }
        if unschedule && (ty == 0 || !p.types[0].scheduled) {
            return Err(E_INVALIDARG);
        }
        Ok(())
    });
    if let Err(hr) = pre {
        record("d3d12_xs", "ScheduleFrameEventX", &format!("type={ty} -> {hr:#010x}"));
        return hr;
    }
    let handles = match handles {
        Ok(v) => v,
        Err(hr) => return hr,
    };
    let mut dups = Vec::with_capacity(handles.len());
    if !unschedule {
        for &h in &handles {
            let mut d: isize = 0;
            if DuplicateHandle(GetCurrentProcess(), h, GetCurrentProcess(), &mut d, 0, 0, 2) == 0 {
                for x in dups {
                    CloseHandle(x);
                }
                record("d3d12_xs", "ScheduleFrameEventX", &format!("type={ty} DuplicateHandle({h:#x}) failed -> E_FAIL"));
                return E_FAIL;
            }
            dups.push(d);
        }
    }
    let hr = with(|p| {
        let mut hr = S_FALSE;
        if !p.active {
            start_clock(p);
            hr = HRESULT_FROM_NT_SUCCESS;
        }
        let interval = p.interval_us.max(1);
        let t = &mut p.types[ty as usize];
        for h in t.dups.drain(..) {
            CloseHandle(h);
        }
        if unschedule {
            t.scheduled = false;
        } else {
            t.offset_us = offset_us % interval;
            t.dups = dups;
            t.scheduled = true;
            if ty == 0 {
                p.origin_offset = (offset_us % interval) as i64;
            }
        }
        p.generation += 1;
        hr
    });
    ensure_timer();
    kick_timer();
    hr
}

fn remember_token(t: u64) {
    let i = ISSUED_POS.fetch_add(1, Ordering::Relaxed) as usize % KNOWN_TOKENS;
    ISSUED.lock().unwrap()[i] = t;
}

fn known_token(t: u64) -> bool {
    ISSUED.lock().unwrap().iter().any(|&x| x == t)
}

pub unsafe extern "system" fn wait_frame_event_x(_this: *mut c_void, ty: u32, timeout_ms: u32, anc: *const u8, flags: u32, token: *mut u64) -> i32 {
    static CALLS: AtomicU64 = AtomicU64::new(0);
    if CALLS.fetch_add(1, Ordering::Relaxed) < 3 {
        record("d3d12_xs", "WaitFrameEventX", &format!(
            "type={ty} timeout={timeout_ms}ms ancillary={anc:p} flags={flags:#x} token_out={token:p}"));
    }
    let internal = with(|p| {
        if (ty as usize) < TYPES && p.types[ty as usize].scheduled { Some(p.types[ty as usize].internal) } else { None }
    });
    let Some(internal) = internal else { return E_INVALIDARG };
    let token_in = if token.is_null() { 0 } else { *token };
    if ty == 0 && token_in != 0 && (!known_token(token_in) || flags & FLAG_PROCESS_TOKEN_ON_TIMEOUT != 0) {
        return E_INVALIDARG;
    }
    let extra = match read_ancillary(anc) {
        Ok(v) => v,
        Err(hr) => return hr,
    };
    let mut hs = Vec::with_capacity(1 + extra.len());
    hs.push(internal);
    hs.extend_from_slice(&extra);
    let t0 = qpc();
    let r = WaitForMultipleObjects(hs.len() as u32, hs.as_ptr(), (flags & FLAG_WAIT_ALL) as i32, timeout_ms);
    let t1 = qpc();
    if ty == 0 {
        crate::perf::on_origin_wait(t1 - t0, t1);
    }
    let (hr, idx) = if r == WAIT_TIMEOUT {
        (S_FALSE, -1i32)
    } else if (r as usize) < hs.len() {
        (S_OK, if r == 0 { -2 } else { r as i32 - 1 })
    } else {
        (E_FAIL, -1)
    };
    write_signaled_index(anc, idx);
    if hr == E_FAIL {
        return hr;
    }
    let process_token = idx == -2 || (idx == -1 && flags & FLAG_PROCESS_TOKEN_ON_TIMEOUT != 0);
    if process_token && ty == 0 && token_in == 0 {
        let t = NEXT_TOKEN.fetch_add(1, Ordering::Relaxed) & TOKEN_MASK;
        let t = if t == 0 { NEXT_TOKEN.fetch_add(1, Ordering::Relaxed) & TOKEN_MASK } else { t };
        remember_token(t);
        if !token.is_null() {
            *token = t;
        }
    }
    hr
}

pub fn mark_flipped(token: u64) {
    if token == 0 {
        return;
    }
    let now = qpc();
    let origin = LAST_ORIGIN_QPC.load(Ordering::Relaxed);
    let mut f = FLIPPED.lock().unwrap();
    f.push((token, origin, now));
    if f.len() > FLIPPED_KEEP {
        let drop = f.len() - FLIPPED_KEEP;
        f.drain(..drop);
    }
}

pub unsafe extern "system" fn frame_statistics_x(_this: *mut c_void, token: u64, ty: u32, count: *mut u32, stats: *mut u8) -> i32 {
    static CALLS: AtomicU64 = AtomicU64::new(0);
    let n = CALLS.fetch_add(1, Ordering::Relaxed);
    let flipped = FLIPPED.lock().unwrap().iter().rev().find(|(t, _, _)| *t == token).map(|(_, o, q)| (*o, *q));
    if n < 4 {
        record("d3d12_xs", "FrameStatistics", &format!(
            "token={token:#x} type={ty:#x} count_in={} -> {}",
            if count.is_null() { 0 } else { *count },
            if flipped.is_some() { "flipped" } else { "not flipped yet" }));
    }
    if count.is_null() || stats.is_null() {
        return E_INVALIDARG;
    }
    let Some((origin, flip)) = flipped else {
        *count = 0;
        return E_FAIL;
    };
    if *count < 1 {
        return E_INVALIDARG;
    }
    *count = 1;
    *(stats as *mut u32) = ty;
    *(stats.add(0x10) as *mut i64) = if origin != 0 && origin <= flip { origin } else { flip };
    *(stats.add(0x18) as *mut i64) = flip;
    S_OK
}

pub fn interval_us() -> u32 {
    with(|p| p.interval_us)
}

pub fn host_refresh_mhz() -> u32 {
    with(|p| p.host_refresh_mhz)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interval_windows_match_console_driver() {
        assert!(interval_valid(16667, false));
        assert!(interval_valid(16683, false));
        assert!(!interval_valid(16684, false));
        assert!(!interval_valid(16666, false));
        assert!(interval_valid(33333, false));
        assert!(interval_valid(50049, false));
        assert!(interval_valid(66734, false));
        assert!(!interval_valid(8333, false));
        assert!(interval_valid(8333, true));
        assert!(!interval_valid(25000, false));
        assert!(interval_valid(25025, true));
        assert!(!interval_valid(20000, true));
        assert_eq!(vblanks_per_interval(16667), (1, false));
        assert_eq!(vblanks_per_interval(33333), (2, false));
        assert_eq!(vblanks_per_interval(25000), (3, true));
    }

    #[test]
    fn deadlines_do_not_drift() {
        let interval = 16_667_000i64;
        let base = 1_000_000i64;
        let off = 6_666_000i64;
        let (d0, k0) = next_deadline(base, interval, off, 0);
        assert_eq!((d0, k0), (base + off, 0));
        let (d, k) = next_deadline(base, interval, off, base + off + 1000 * interval + 5);
        assert_eq!(k, 1001);
        assert_eq!(d, base + off + 1001 * interval);
        let (d2, k2) = next_deadline(base, interval, off, d - 1);
        assert_eq!((d2, k2), (d, k));
    }

    #[test]
    fn missed_ticks_are_skipped_not_burst() {
        let interval = 100i64;
        let (d, k) = next_deadline(0, interval, 0, 1050);
        assert_eq!((d, k), (1100, 11));
    }

    fn reset() {
        *STATE.lock().unwrap() = None;
    }

    #[test]
    fn origin_wait_semantics() {
        let _g = TEST_LOCK.lock().unwrap();
        reset();
        unsafe {
            let this = core::ptr::null_mut();
            let mut tok = 0u64;
            assert_eq!(wait_frame_event_x(this, 0, 0, core::ptr::null(), 0, &mut tok), E_INVALIDARG);
            assert_eq!(schedule_frame_event_x(this, 0, 0, core::ptr::null(), 0), E_INVALIDARG);
            assert_eq!(set_frame_interval_x(this, core::ptr::null_mut(), 12345, 3, 0), E_INVALIDARG);
            assert_eq!(set_frame_interval_x(this, core::ptr::null_mut(), 16667, 3, 0), S_FALSE);
            assert_eq!(schedule_frame_event_x(this, 2, 0, core::ptr::null(), 0), E_INVALIDARG);
            assert!(schedule_frame_event_x(this, 0, 6666, core::ptr::null(), 0) >= 0);
            assert_eq!(schedule_frame_event_x(this, 0, 0, core::ptr::null(), FLAG_UNSCHEDULE), E_INVALIDARG);
            let hr = wait_frame_event_x(this, 0, 0, core::ptr::null(), 0, &mut tok);
            assert!(hr == S_FALSE || hr == S_OK);
            if hr == S_FALSE {
                assert_eq!(tok, 0);
            }
            tok = 0;
            let hr = wait_frame_event_x(this, 0, 0, core::ptr::null(), FLAG_PROCESS_TOKEN_ON_TIMEOUT, &mut tok);
            assert!(hr == S_FALSE || hr == S_OK);
            assert_ne!(tok, 0);
            let first = tok;
            let mut tok2 = 0u64;
            let t0 = qpc();
            assert_eq!(wait_frame_event_x(this, 0, 1000, core::ptr::null(), 0, &mut tok2), S_OK);
            let waited_ms = (qpc() - t0) * 1000 / qpf();
            assert!(waited_ms <= 40, "waited {waited_ms} ms");
            assert_ne!(tok2, 0);
            assert_ne!(tok2, first);
            std::thread::sleep(std::time::Duration::from_millis(60));
            let t1 = qpc();
            let mut tok3 = 0u64;
            assert_eq!(wait_frame_event_x(this, 0, 1000, core::ptr::null(), 0, &mut tok3), S_OK);
            let latched_ms = (qpc() - t1) * 1000 / qpf();
            assert!(latched_ms <= 2, "latched origin should return immediately, took {latched_ms} ms");
            let mut bogus = 0xdead_beefu64;
            assert_eq!(wait_frame_event_x(this, 0, 0, core::ptr::null(), 0, &mut bogus), E_INVALIDARG);
        }
        reset();
    }

    #[test]
    fn ancillary_events_are_duplicated_and_unscheduled() {
        let _g = TEST_LOCK.lock().unwrap();
        reset();
        unsafe {
            let this = core::ptr::null_mut();
            let ev = CreateEventW(core::ptr::null_mut(), 0, 0, core::ptr::null());
            let list = [ev];
            let mut anc = [0u8; 24];
            anc[0..4].copy_from_slice(&1u32.to_le_bytes());
            anc[8..16].copy_from_slice(&(list.as_ptr() as usize).to_le_bytes());
            assert_eq!(set_frame_interval_x(this, core::ptr::null_mut(), 16667, 3, 0), S_FALSE);
            assert!(schedule_frame_event_x(this, 0, 0, core::ptr::null(), 0) >= 0);
            assert!(schedule_frame_event_x(this, 2, 4000, anc.as_ptr(), 0) >= 0);
            let r = WaitForMultipleObjects(1, &ev, 0, 200);
            assert_eq!(r, 0, "ancillary event was not signaled by the frame clock");
            let mut tok = 0u64;
            assert_eq!(wait_frame_event_x(this, 2, 200, core::ptr::null(), 0, &mut tok), S_OK);
            assert_eq!(tok, 0);
            assert!(schedule_frame_event_x(this, 2, 0, core::ptr::null(), FLAG_UNSCHEDULE) >= 0);
            assert_eq!(wait_frame_event_x(this, 2, 0, core::ptr::null(), 0, &mut tok), E_INVALIDARG);
            CloseHandle(ev);
        }
        reset();
    }

    static TEST_LOCK: Mutex<()> = Mutex::new(());
}
