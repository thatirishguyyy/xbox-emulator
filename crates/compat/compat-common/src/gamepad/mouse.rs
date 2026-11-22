use core::ffi::c_void;
use core::sync::atomic::{AtomicI64, AtomicU32, AtomicU8, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Instant;

#[link(name = "user32")]
extern "system" {
    fn GetCursorPos(p: *mut [i32; 2]) -> i32;
    fn ScreenToClient(h: *mut c_void, p: *mut [i32; 2]) -> i32;
    fn GetClientRect(h: *mut c_void, r: *mut [i32; 4]) -> i32;
    fn GetForegroundWindow() -> *mut c_void;
    fn IsWindow(h: *mut c_void) -> i32;
    fn IsWindowVisible(h: *mut c_void) -> i32;
    fn GetWindow(h: *mut c_void, cmd: u32) -> *mut c_void;
    fn GetWindowThreadProcessId(h: *mut c_void, pid: *mut u32) -> u32;
    fn EnumWindows(cb: unsafe extern "system" fn(*mut c_void, isize) -> i32, l: isize) -> i32;
    fn SetWindowLongPtrW(h: *mut c_void, idx: i32, v: isize) -> isize;
    fn CallWindowProcW(prev: isize, h: *mut c_void, msg: u32, w: usize, l: isize) -> isize;
    fn SetCursor(c: *mut c_void) -> *mut c_void;
    fn LoadCursorW(inst: *mut c_void, name: *const u16) -> *mut c_void;
    fn GetClassLongPtrW(h: *mut c_void, idx: i32) -> usize;
}

extern "system" {
    fn GetModuleHandleW(n: *const u16) -> *const u8;
    fn VirtualProtect(addr: *mut c_void, size: usize, new_protect: u32, old_protect: *mut u32) -> i32;
    fn GetCurrentProcessId() -> u32;
}

pub const KIND_MOUSE: u32 = 0x20;
pub const UNITY_GXDK_REFERENCE_WIDTH: f64 = 1920.0;
pub const PRESENT_VAR: &str = "TOMBOX_PRESENT_TARGET";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub client_w: f64,
    pub client_h: f64,
    pub screen_w: f64,
    pub screen_h: f64,
}

pub fn unity_center(w: f64, h: f64) -> (f64, f64) {
    ((w * 0.5).round(), (h * 0.5).round())
}

pub fn game_point(cx: f64, cy: f64, v: &View) -> (f64, f64) {
    let gx = (cx * v.screen_w / v.client_w).clamp(0.0, v.screen_w - 1.0);
    let gy = (cy * v.screen_h / v.client_h).clamp(0.0, v.screen_h - 1.0);
    (gx, gy)
}

pub fn unity_gxdk_position(cx: f64, cy: f64, v: &View) -> (i64, i64) {
    let (w, h) = (v.screen_w, v.screen_h);
    let k = w / UNITY_GXDK_REFERENCE_WIDTH;
    let (gx, gy) = game_point(cx, cy, v);
    let (mx, my) = unity_center(w, h);
    let uy = (h - 1.0) - gy;
    let ax = ((gx - mx) / k).round();
    let ay = ((my - uy) / k).round();
    let ax = ax.clamp((-mx / k).ceil(), ((w - 1.0 - mx) / k).floor());
    let ay = ay.clamp(((my - (h - 1.0)) / k).ceil(), (my / k).floor());
    (ax as i64, ay as i64)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Raw,
    UnityGxdk,
}

pub fn parse_mode(v: &str) -> Option<Mode> {
    match v.trim().to_ascii_lowercase().as_str() {
        "raw" | "off" | "0" => Some(Mode::Raw),
        "cursor" | "unity" | "on" | "1" => Some(Mode::UnityGxdk),
        _ => None,
    }
}

pub fn parse_present_target(v: &str) -> Option<(usize, f64, f64)> {
    let mut it = v.trim().split(',');
    let h = usize::from_str_radix(it.next()?.trim().trim_start_matches("0x"), 16).ok()?;
    let w: f64 = it.next()?.trim().parse().ok()?;
    let hh: f64 = it.next()?.trim().parse().ok()?;
    if w < 1.0 || hh < 1.0 {
        return None;
    }
    Some((h, w, hh))
}

pub fn parse_pointer_feed(v: &str) -> Option<(f64, f64)> {
    let mut it = v.split_whitespace();
    let fx: f64 = it.next()?.parse().ok()?;
    let fy: f64 = it.next()?.parse().ok()?;
    if !(0.0..=1.0).contains(&fx) || !(0.0..=1.0).contains(&fy) {
        return None;
    }
    Some((fx, fy))
}

pub fn image_has(base: &[u8], needle: &[u8]) -> bool {
    let Some((&first, rest)) = needle.split_first() else { return false };
    let mut i = 0usize;
    while i + needle.len() <= base.len() {
        match base[i..base.len() - rest.len()].iter().position(|&b| b == first) {
            Some(p) => {
                let at = i + p;
                if &base[at + 1..at + needle.len()] == rest {
                    return true;
                }
                i = at + 1;
            }
            None => return false,
        }
    }
    false
}

pub fn image_has_any(sections: &[&[u8]], needle: &[u8]) -> bool {
    sections.iter().any(|s| image_has(s, needle))
}

static MODE: AtomicU8 = AtomicU8::new(0);

unsafe fn data_sections() -> Vec<&'static [u8]> {
    let base = GetModuleHandleW(core::ptr::null());
    let mut out = Vec::new();
    if base.is_null() || *base != b'M' || *base.add(1) != b'Z' {
        return out;
    }
    let pe = u32::from_le_bytes([*base.add(0x3c), *base.add(0x3d), *base.add(0x3e), *base.add(0x3f)]) as usize;
    let size = u32::from_le_bytes([*base.add(pe + 0x50), *base.add(pe + 0x51), *base.add(pe + 0x52), *base.add(pe + 0x53)]) as usize;
    let nsec = u16::from_le_bytes([*base.add(pe + 6), *base.add(pe + 7)]) as usize;
    let opt = u16::from_le_bytes([*base.add(pe + 0x14), *base.add(pe + 0x15)]) as usize;
    let sec = pe + 0x18 + opt;
    for i in 0..nsec {
        let s = sec + i * 40;
        let ch = u32::from_le_bytes([*base.add(s + 36), *base.add(s + 37), *base.add(s + 38), *base.add(s + 39)]);
        let va = u32::from_le_bytes([*base.add(s + 12), *base.add(s + 13), *base.add(s + 14), *base.add(s + 15)]) as usize;
        let vs = u32::from_le_bytes([*base.add(s + 8), *base.add(s + 9), *base.add(s + 10), *base.add(s + 11)]) as usize;
        if ch & 0x2000_0000 == 0 && ch & 0x4000_0000 != 0 && ch & 0x8000_0000 == 0 && va + vs <= size {
            out.push(core::slice::from_raw_parts(base.add(va), vs));
        }
    }
    out
}

pub fn mode() -> Mode {
    match MODE.load(Ordering::Acquire) {
        1 => return Mode::Raw,
        2 => return Mode::UnityGxdk,
        _ => {}
    }
    let (m, why) = match std::env::var("TOMBOX_MOUSE").ok().as_deref().and_then(parse_mode) {
        Some(m) => (m, "TOMBOX_MOUSE".to_string()),
        None => {
            let secs = unsafe { data_sections() };
            let unity = image_has_any(&secs, b"GXDKMouse\0") && image_has_any(&secs, b"GXDKInputManagerWorker\0");
            if unity {
                (Mode::UnityGxdk, "title EXE contains the Unity GXDK input backend (GXDKMouse), which builds its cursor from GameInput mouse deltas scaled by screen width / 1920".to_string())
            } else {
                (Mode::Raw, "title is not a Unity GXDK player".to_string())
            }
        }
    };
    MODE.store(if m == Mode::Raw { 1 } else { 2 }, Ordering::Release);
    super::log("mouse", &format!("mode {} ({why})", if m == Mode::Raw { "raw (host GameInput mouse counts passed through)" } else { "cursor (positions rebuilt from the Windows pointer so the title cursor sits under it; OS pointer hidden over the focused client)" }));
    m
}

struct Target {
    hwnd: usize,
    screen: Option<(f64, f64)>,
    checked: Option<Instant>,
    feed: Option<(f64, f64)>,
    feed_checked: Option<Instant>,
}

static TARGET: Mutex<Target> = Mutex::new(Target { hwnd: 0, screen: None, checked: None, feed: None, feed_checked: None });
static LAST_X: AtomicI64 = AtomicI64::new(0);
static LAST_Y: AtomicI64 = AtomicI64::new(0);
static LOGS: AtomicU32 = AtomicU32::new(0);

unsafe fn largest_own_window() -> usize {
    unsafe extern "system" fn cb(h: *mut c_void, l: isize) -> i32 {
        let best = &mut *(l as *mut (usize, i64));
        let mut pid = 0u32;
        GetWindowThreadProcessId(h, &mut pid);
        if pid != GetCurrentProcessId() || IsWindowVisible(h) == 0 || !GetWindow(h, 4).is_null() {
            return 1;
        }
        let mut r = [0i32; 4];
        GetClientRect(h, &mut r);
        let area = r[2] as i64 * r[3] as i64;
        if area > best.1 {
            *best = (h as usize, area);
        }
        1
    }
    let mut best: (usize, i64) = (0, 0);
    EnumWindows(cb, &mut best as *mut _ as isize);
    best.0
}

fn refresh(t: &mut Target) {
    let now = Instant::now();
    if t.checked.map(|c| now.duration_since(c).as_millis() < 500).unwrap_or(false) && t.hwnd != 0 {
        return;
    }
    t.checked = Some(now);
    let present = std::env::var(PRESENT_VAR).ok().as_deref().and_then(parse_present_target);
    let (hwnd, screen) = match present {
        Some((h, w, hh)) if unsafe { IsWindow(h as *mut c_void) } != 0 => (h, Some((w, hh))),
        _ => (unsafe { largest_own_window() }, None),
    };
    if hwnd != t.hwnd || screen != t.screen {
        super::log("mouse", &format!("pointer target hwnd={hwnd:#x} title screen {}", match screen {
            Some((w, h)) => format!("{w}x{h} (PresentX back buffer)"),
            None => "unknown yet (no PresentX target): client size used".to_string(),
        }));
        if hwnd != 0 {
            unsafe { hide_over_client(hwnd) };
        }
    }
    t.hwnd = hwnd;
    t.screen = screen;
    if t.feed_checked.map(|c| now.duration_since(c).as_millis() < 100).unwrap_or(false) {
        return;
    }
    t.feed_checked = Some(now);
    let feed = if std::env::var("TOMBOX_POINTER_FEED").map(|v| v == "1").unwrap_or(false) {
        std::fs::read_to_string(super::channel_dir().join("pointer.txt")).ok().as_deref().and_then(parse_pointer_feed)
    } else {
        None
    };
    if feed != t.feed {
        super::log("mouse", &match feed {
            Some((fx, fy)) => format!("test pointer feed active: client fraction ({fx:.4}, {fy:.4}); the Windows pointer is not read"),
            None => "test pointer feed off: the Windows pointer is read".to_string(),
        });
    }
    t.feed = feed;
}

fn current_position() -> Option<(i64, i64)> {
    let mut t = TARGET.lock().unwrap_or_else(|e| e.into_inner());
    refresh(&mut t);
    if t.hwnd == 0 {
        return None;
    }
    let h = t.hwnd as *mut c_void;
    let mut r = [0i32; 4];
    if unsafe { GetClientRect(h, &mut r) } == 0 || r[2] <= 0 || r[3] <= 0 {
        return None;
    }
    let (cw, ch) = (r[2] as f64, r[3] as f64);
    let (cx, cy) = match t.feed {
        Some((fx, fy)) => (fx * cw, fy * ch),
        None => {
            if unsafe { GetForegroundWindow() } as usize != t.hwnd {
                return None;
            }
            let mut p = [0i32; 2];
            if unsafe { GetCursorPos(&mut p) } == 0 || unsafe { ScreenToClient(h, &mut p) } == 0 {
                return None;
            }
            (p[0] as f64, p[1] as f64)
        }
    };
    let (sw, sh) = t.screen.unwrap_or((cw, ch));
    let v = View { client_w: cw, client_h: ch, screen_w: sw, screen_h: sh };
    let out = unity_gxdk_position(cx, cy, &v);
    let n = LOGS.load(Ordering::Relaxed);
    if n < 12 && (LAST_X.load(Ordering::Relaxed), LAST_Y.load(Ordering::Relaxed)) != out {
        LOGS.fetch_add(1, Ordering::Relaxed);
        let (gx, gy) = game_point(cx, cy, &v);
        super::log("mouse", &format!("pointer client ({cx:.1}, {cy:.1}) of {cw}x{ch} -> title screen ({gx:.1}, {gy:.1}) of {sw}x{sh} -> GameInput position ({}, {})", out.0, out.1));
    }
    Some(out)
}

pub fn rewrite(state: &mut [u8; 40]) {
    let (x, y) = match current_position() {
        Some(p) => {
            LAST_X.store(p.0, Ordering::Relaxed);
            LAST_Y.store(p.1, Ordering::Relaxed);
            p
        }
        None => (LAST_X.load(Ordering::Relaxed), LAST_Y.load(Ordering::Relaxed)),
    };
    state[8..16].copy_from_slice(&x.to_le_bytes());
    state[16..24].copy_from_slice(&y.to_le_bytes());
}

static ORIG: Mutex<Vec<(usize, usize)>> = Mutex::new(Vec::new());

fn orig_for(vt: usize) -> usize {
    ORIG.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|(v, _)| *v == vt).map(|(_, f)| *f).unwrap_or(0)
}

unsafe extern "system" fn get_mouse_state(this: *mut c_void, out: *mut [u8; 40]) -> bool {
    let f = orig_for(*(this as *const usize));
    if f == 0 {
        return false;
    }
    let real: unsafe extern "system" fn(*mut c_void, *mut [u8; 40]) -> bool = core::mem::transmute(f);
    let ok = real(this, out);
    if ok && !out.is_null() {
        let mut s = core::ptr::read_unaligned(out);
        rewrite(&mut s);
        core::ptr::write_unaligned(out, s);
    }
    ok
}

pub unsafe fn patch_reading(rd: *mut c_void, kind: u32) {
    if rd.is_null() || kind & KIND_MOUSE == 0 || mode() != Mode::UnityGxdk {
        return;
    }
    let vt = *(rd as *const usize);
    let mut list = ORIG.lock().unwrap_or_else(|e| e.into_inner());
    if list.iter().any(|(v, _)| *v == vt) {
        return;
    }
    let slot = (vt as *mut usize).add(16);
    let cur = *slot;
    if cur == get_mouse_state as *const () as usize {
        return;
    }
    let mut old = 0u32;
    if VirtualProtect(slot as *mut c_void, 8, 0x04, &mut old) == 0 {
        super::log("mouse", &format!("VirtualProtect on host reading vtable {vt:#x} failed; mouse positions left as raw counts"));
        return;
    }
    list.push((vt, cur));
    core::ptr::write_volatile(slot, get_mouse_state as *const () as usize);
    let mut tmp = 0u32;
    VirtualProtect(slot as *mut c_void, 8, old, &mut tmp);
    drop(list);
    super::log("mouse", &format!("host reading vtable {vt:#x} GetMouseState ({}) routed: positions follow the Windows pointer", super::host::module_of(cur)));
}

static SUBCLASSED: Mutex<Vec<(usize, isize)>> = Mutex::new(Vec::new());
static HIDE_LOGS: AtomicUsize = AtomicUsize::new(0);
static CLASS_CURSOR_NULL: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

fn prev_proc(h: usize) -> isize {
    SUBCLASSED.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|(w, _)| *w == h).map(|(_, p)| *p).unwrap_or(0)
}

pub fn hides(msg: u32, lparam: isize, focused: bool, feed: bool) -> bool {
    msg == 0x0020 && (lparam & 0xffff) as u16 == 1 && (focused || feed)
}

unsafe extern "system" fn wndproc(h: *mut c_void, msg: u32, w: usize, l: isize) -> isize {
    let prev = prev_proc(h as usize);
    if msg == 0x0020 {
        let feed = TARGET.lock().unwrap_or_else(|e| e.into_inner()).feed.is_some();
        let focused = GetForegroundWindow() == h;
        if hides(msg, l, focused, feed) {
            SetCursor(core::ptr::null_mut());
            if HIDE_LOGS.fetch_add(1, Ordering::Relaxed) == 0 {
                super::log("mouse", "Windows pointer hidden over the focused title client (the title draws its own cursor)");
            }
            return 1;
        }
        if (l & 0xffff) as u16 == 1 && CLASS_CURSOR_NULL.load(Ordering::Relaxed) {
            SetCursor(LoadCursorW(core::ptr::null_mut(), 32512 as *const u16));
            return 1;
        }
    }
    if prev == 0 {
        return 0;
    }
    CallWindowProcW(prev, h, msg, w, l)
}

unsafe fn hide_over_client(h: usize) {
    let mut list = SUBCLASSED.lock().unwrap_or_else(|e| e.into_inner());
    if list.iter().any(|(w, _)| *w == h) {
        return;
    }
    let prev = SetWindowLongPtrW(h as *mut c_void, -4, wndproc as *const () as isize);
    if prev == 0 {
        drop(list);
        super::log("mouse", &format!("could not subclass hwnd={h:#x}; Windows pointer stays visible"));
        return;
    }
    list.push((h, prev));
    drop(list);
    let cls = GetClassLongPtrW(h as *mut c_void, -12);
    CLASS_CURSOR_NULL.store(cls == 0, Ordering::Relaxed);
    super::log("mouse", &format!("hwnd={h:#x} subclassed for WM_SETCURSOR; class cursor {cls:#x}{}", if cls == 0 { " (none: arrow set while the window is unfocused)" } else { "" }));
}

#[cfg(test)]
mod tests {
    use super::*;

    struct UnityMouse {
        last: (f64, f64),
        pos: (f64, f64),
        w: f64,
        h: f64,
    }

    impl UnityMouse {
        fn new(w: f64, h: f64) -> Self {
            let mut m = UnityMouse { last: (0.0, 0.0), pos: (0.0, 0.0), w, h };
            m.reset();
            m
        }
        fn reset(&mut self) {
            let (cx, cy) = unity_center(self.w, self.h);
            self.pos = (cx, cy);
            self.last = (0.0, 0.0);
        }
        fn step(&mut self, p: (i64, i64)) {
            let (dx, dy) = (p.0 as f64 - self.last.0, p.1 as f64 - self.last.1);
            self.last = (p.0 as f64, p.1 as f64);
            let k = self.w / UNITY_GXDK_REFERENCE_WIDTH;
            self.pos.0 = (self.pos.0 + dx * k).clamp(0.0, self.w - 1.0);
            self.pos.1 = (self.pos.1 - dy * k).clamp(0.0, self.h - 1.0);
        }
        fn top_down(&self) -> (f64, f64) {
            (self.pos.0, (self.h - 1.0) - self.pos.1)
        }
    }

    fn view(cw: f64, ch: f64) -> View {
        View { client_w: cw, client_h: ch, screen_w: 1920.0, screen_h: 1080.0 }
    }

    #[test]
    fn unfocused_first_reading_is_mapped_not_raw() {
        let v = view(1920.0, 1080.0);
        let mut m = UnityMouse::new(1920.0, 1080.0);
        m.step((0, 0));
        let (cx, cy) = unity_center(1920.0, 1080.0);
        assert_eq!(m.top_down(), (cx, 1079.0 - cy));
        m.step(unity_gxdk_position(123.0, 456.0, &v));
        assert_eq!(m.top_down(), (123.0, 456.0));
    }

    #[test]
    fn same_size_client_maps_one_to_one() {
        let v = view(1920.0, 1080.0);
        let mut m = UnityMouse::new(1920.0, 1080.0);
        for &(x, y) in &[(0.0, 0.0), (960.0, 540.0), (1919.0, 1079.0), (100.0, 900.0), (1500.0, 3.0)] {
            m.step(unity_gxdk_position(x, y, &v));
            assert_eq!(m.top_down(), (x, y));
        }
    }

    #[test]
    fn resized_window_scales_to_title_screen() {
        let v = view(1280.0, 720.0);
        let mut m = UnityMouse::new(1920.0, 1080.0);
        m.step(unity_gxdk_position(640.0, 360.0, &v));
        assert_eq!(m.top_down(), (960.0, 540.0));
        m.step(unity_gxdk_position(320.0, 180.0, &v));
        assert_eq!(m.top_down(), (480.0, 270.0));
        let v2 = view(1000.0, 1000.0);
        m.step(unity_gxdk_position(500.0, 250.0, &v2));
        assert_eq!(m.top_down(), (960.0, 270.0));
    }

    #[test]
    fn dpi_scaling_cancels_out() {
        for scale in [1.0, 1.25, 1.5] {
            let v = view(1536.0 * scale, 864.0 * scale);
            let mut m = UnityMouse::new(1920.0, 1080.0);
            m.step(unity_gxdk_position(768.0 * scale, 216.0 * scale, &v));
            let (x, y) = m.top_down();
            assert!((x - 960.0).abs() <= 0.5 && (y - 270.0).abs() <= 0.5, "scale {scale}: {x},{y}");
        }
    }

    #[test]
    fn pointer_outside_client_clamps_without_drift() {
        let v = view(1920.0, 1080.0);
        let mut m = UnityMouse::new(1920.0, 1080.0);
        m.step(unity_gxdk_position(50.0, 50.0, &v));
        m.step(unity_gxdk_position(-400.0, -300.0, &v));
        assert_eq!(m.top_down(), (0.0, 0.0));
        m.step(unity_gxdk_position(5000.0, 4000.0, &v));
        assert_eq!(m.top_down(), (1919.0, 1079.0));
        m.step(unity_gxdk_position(50.0, 50.0, &v));
        assert_eq!(m.top_down(), (50.0, 50.0));
    }

    #[test]
    fn title_reset_mid_stream_stays_under_pointer() {
        let v = view(1920.0, 1080.0);
        let mut m = UnityMouse::new(1920.0, 1080.0);
        m.step(unity_gxdk_position(300.0, 200.0, &v));
        m.reset();
        m.step(unity_gxdk_position(310.0, 205.0, &v));
        assert_eq!(m.top_down(), (310.0, 205.0));
        m.step(unity_gxdk_position(310.0, 205.0, &v));
        assert_eq!(m.top_down(), (310.0, 205.0));
    }

    #[test]
    fn non_1080p_title_screen_uses_width_scale() {
        for (w, h) in [(3840.0, 2160.0), (1280.0, 720.0), (2560.0, 1440.0)] {
            let v = View { client_w: 1920.0, client_h: 1080.0, screen_w: w, screen_h: h };
            let mut m = UnityMouse::new(w, h);
            let k = w / 1920.0;
            for &(x, y) in &[(0.0, 0.0), (1919.0, 1079.0), (777.0, 333.0)] {
                m.step(unity_gxdk_position(x, y, &v));
                let (gx, gy) = game_point(x, y, &v);
                let (tx, ty) = m.top_down();
                assert!((tx - gx).abs() <= k * 0.5 + 1e-6 && (ty - gy).abs() <= k * 0.5 + 1e-6, "{w}x{h} ({x},{y}) -> ({tx},{ty}) want ({gx},{gy})");
                assert!(tx >= 0.0 && tx <= w - 1.0 && ty >= 0.0 && ty <= h - 1.0);
            }
        }
    }

    #[test]
    fn parses_modes_targets_and_feed() {
        assert_eq!(parse_mode("cursor"), Some(Mode::UnityGxdk));
        assert_eq!(parse_mode(" RAW "), Some(Mode::Raw));
        assert_eq!(parse_mode("x"), None);
        assert_eq!(parse_present_target("0x240632,1920,1080"), Some((0x240632, 1920.0, 1080.0)));
        assert_eq!(parse_present_target("240632,0,1080"), None);
        assert_eq!(parse_pointer_feed("0.5 0.25\n"), Some((0.5, 0.25)));
        assert_eq!(parse_pointer_feed("1.5 0"), None);
        assert!(image_has(b"..GXDKMouse\0..", b"GXDKMouse\0"));
        assert!(!image_has(b"..GXDKMous", b"GXDKMouse\0"));
        assert!(image_has(b"GGXDKMouse\0", b"GXDKMouse\0"));
        assert!(image_has_any(&[b"abc", b"xGXDKMouse\0"], b"GXDKMouse\0"));
    }

    #[test]
    fn pointer_hidden_only_over_focused_client() {
        assert!(hides(0x20, 1, true, false));
        assert!(!hides(0x20, 1, false, false));
        assert!(!hides(0x20, 2, true, false));
        assert!(!hides(0x200, 1, true, false));
        assert!(hides(0x20, 0x0200_0001, false, true));
    }
}
