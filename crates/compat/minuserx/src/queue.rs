use core::ffi::c_void;
use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, OnceLock};

use crate::{hr_win32, E_INVALIDARG, S_OK};

pub const WM_QUIT: u32 = 0x12;
pub const WM_MOUSEMOVE: u32 = 0x200;
pub const PM_REMOVE: u32 = 1;
const QS_POSTMESSAGE: u32 = 0x8;
const QS_ALLINPUT: u32 = 0x4ff;
const PM_QS_POSTMESSAGE: u32 = QS_POSTMESSAGE << 16;
const INFINITE: u32 = 0xffff_ffff;
const ERROR_NOT_ENOUGH_QUOTA: u32 = 1816;
const ERROR_MESSAGE_SYNC_ONLY: u32 = 1159;
const ERROR_INVALID_WINDOW_HANDLE: u32 = 1400;
const ERROR_INVALID_PARAMETER: u32 = 87;
const GWLP_WNDPROC: i32 = -4;

#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentThreadId() -> u32;
    fn GetLastError() -> u32;
    fn SetLastError(e: u32);
    fn GetTickCount() -> u32;
}

#[link(name = "user32")]
extern "system" {
    fn IsWindow(h: *mut c_void) -> i32;
    fn IsChild(parent: *mut c_void, h: *mut c_void) -> i32;
    fn IsGUIThread(convert: i32) -> i32;
    fn PostThreadMessageW(tid: u32, msg: u32, w: usize, l: isize) -> i32;
    fn PostMessageW(h: *mut c_void, msg: u32, w: usize, l: isize) -> i32;
    fn PeekMessageW(m: *mut Msg, h: *mut c_void, lo: u32, hi: u32, flags: u32) -> i32;
    fn DispatchMessageW(m: *const Msg) -> isize;
    fn GetWindowThreadProcessId(h: *mut c_void, pid: *mut u32) -> u32;
    fn GetWindowLongPtrW(h: *mut c_void, i: i32) -> isize;
    fn CallWindowProcW(p: isize, h: *mut c_void, m: u32, w: usize, l: isize) -> isize;
    fn MsgWaitForMultipleObjectsEx(n: u32, h: *const *mut c_void, ms: u32, mask: u32, flags: u32) -> u32;
    fn RegisterWindowMessageW(s: *const u16) -> u32;
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Msg {
    pub hwnd: usize,
    pub message: u32,
    pub wparam: usize,
    pub lparam: isize,
    pub time: u32,
    pub pt: [i32; 2],
    pub private: u32,
}

#[derive(Default)]
struct ThreadQueue {
    consumer: bool,
    ready: VecDeque<Msg>,
    pending: VecDeque<Msg>,
    overflow: VecDeque<Msg>,
}

impl ThreadQueue {
    fn tail(&mut self) -> Option<&mut Msg> {
        if !self.overflow.is_empty() {
            return self.overflow.back_mut();
        }
        if !self.pending.is_empty() {
            return self.pending.back_mut();
        }
        self.ready.back_mut()
    }
}

fn queues() -> &'static Mutex<HashMap<u32, ThreadQueue>> {
    static Q: OnceLock<Mutex<HashMap<u32, ThreadQueue>>> = OnceLock::new();
    Q.get_or_init(|| Mutex::new(HashMap::new()))
}

fn lock() -> std::sync::MutexGuard<'static, HashMap<u32, ThreadQueue>> {
    queues().lock().unwrap_or_else(|e| e.into_inner())
}

pub fn token() -> u32 {
    static C: OnceLock<u32> = OnceLock::new();
    *C.get_or_init(|| {
        let n: Vec<u16> = "TomBox.minuserx.QueuedMessage\0".encode_utf16().collect();
        unsafe { RegisterWindowMessageW(n.as_ptr()) }
    })
}

pub fn last_error_from(hr: i32) {
    let e = if hr >= 0 {
        0
    } else if (hr as u32) & 0x1fff_0000 == 0x0007_0000 {
        hr as u32 & 0xffff
    } else {
        hr as u32
    };
    unsafe { SetLastError(e) };
}

struct ConsumerGuard(u32);

impl Drop for ConsumerGuard {
    fn drop(&mut self) {
        lock().remove(&self.0);
    }
}

thread_local! {
    static CONSUMER: std::cell::RefCell<Option<ConsumerGuard>> = const { std::cell::RefCell::new(None) };
}

fn mark_consumer(tid: u32) {
    let fresh = CONSUMER.with(|c| {
        let mut c = c.borrow_mut();
        if c.is_none() {
            *c = Some(ConsumerGuard(tid));
            true
        } else {
            false
        }
    });
    if fresh {
        lock().entry(tid).or_default().consumer = true;
    }
}

pub fn is_consumer(tid: u32) -> bool {
    lock().get(&tid).is_some_and(|q| q.consumer)
}

unsafe fn direct(tid: u32, rec: &Msg) -> Result<(), u32> {
    let ok = if rec.hwnd == 0 { PostThreadMessageW(tid, rec.message, rec.wparam, rec.lparam) } else { PostMessageW(rec.hwnd as *mut c_void, rec.message, rec.wparam, rec.lparam) };
    if ok != 0 {
        Ok(())
    } else {
        Err(GetLastError())
    }
}

unsafe fn enqueue(tid: u32, rec: Msg) -> i32 {
    let tok = token();
    let mut qs = lock();
    let q = qs.entry(tid).or_default();
    if !q.consumer && q.ready.is_empty() && q.pending.is_empty() && q.overflow.is_empty() {
        match direct(tid, &rec) {
            Ok(()) => return S_OK,
            Err(ERROR_MESSAGE_SYNC_ONLY) | Err(ERROR_NOT_ENOUGH_QUOTA) => {}
            Err(e) => {
                let hr = hr_win32(e);
                return if hr < 0 { hr } else { E_INVALIDARG };
            }
        }
    }
    if rec.message == WM_MOUSEMOVE {
        if let Some(t) = q.tail() {
            if t.message == WM_MOUSEMOVE {
                *t = rec;
                return S_OK;
            }
        }
    }
    if !q.overflow.is_empty() {
        q.overflow.push_back(rec);
        return S_OK;
    }
    if PostThreadMessageW(tid, tok, 0, 0) != 0 {
        q.pending.push_back(rec);
        return S_OK;
    }
    match GetLastError() {
        ERROR_NOT_ENOUGH_QUOTA => {
            q.overflow.push_back(rec);
            S_OK
        }
        e => {
            let hr = hr_win32(e);
            if hr < 0 {
                hr
            } else {
                E_INVALIDARG
            }
        }
    }
}

fn record(hwnd: usize, msg: u32, wparam: usize, lparam: isize) -> Msg {
    Msg { hwnd, message: msg, wparam, lparam, time: unsafe { GetTickCount() }, pt: [0; 2], private: 0 }
}

pub unsafe fn post(hwnd: *mut c_void, msg: u32, wparam: usize, lparam: isize) -> i32 {
    if !hwnd.is_null() && IsWindow(hwnd) == 0 {
        return E_INVALIDARG;
    }
    let tid = if hwnd.is_null() {
        IsGUIThread(1);
        GetCurrentThreadId()
    } else {
        GetWindowThreadProcessId(hwnd, core::ptr::null_mut())
    };
    enqueue(tid, record(hwnd as usize, msg, wparam, lparam))
}

pub unsafe fn post_thread(tid: u32, msg: u32, wparam: usize, lparam: isize) -> i32 {
    if tid == GetCurrentThreadId() {
        IsGUIThread(1);
    }
    enqueue(tid, record(0, msg, wparam, lparam))
}

unsafe fn drain(tid: u32) {
    let tok = token();
    let before = lock().get(&tid).map_or(0, |q| q.pending.len());
    let mut seen = 0usize;
    loop {
        let mut m = Msg::default();
        if PeekMessageW(&mut m, core::ptr::null_mut(), 0, 0, PM_REMOVE | PM_QS_POSTMESSAGE) == 0 {
            break;
        }
        let mut qs = lock();
        let q = qs.entry(tid).or_default();
        if m.message == tok {
            seen += 1;
            if let Some(r) = q.pending.pop_front() {
                q.ready.push_back(r);
            }
        } else {
            q.ready.push_back(m);
        }
    }
    let mut qs = lock();
    if let Some(q) = qs.get_mut(&tid) {
        for _ in 0..before.saturating_sub(seen) {
            match q.pending.pop_front() {
                Some(r) => q.ready.push_back(r),
                None => break,
            }
        }
        if q.pending.is_empty() {
            let o: Vec<Msg> = q.overflow.drain(..).collect();
            q.ready.extend(o);
        }
    }
}

unsafe fn matches(m: &Msg, hwnd: usize, lo: u32, hi: u32) -> bool {
    let range = if lo == 0 && hi == 0 {
        true
    } else if lo <= hi {
        m.message >= lo && m.message <= hi
    } else {
        m.message >= lo || m.message <= hi
    };
    if !range {
        return false;
    }
    if m.message == WM_QUIT && m.hwnd == 0 {
        return true;
    }
    match hwnd {
        0 => true,
        usize::MAX => m.hwnd == 0,
        h => m.hwnd == h || (m.hwnd != 0 && IsChild(h as *mut c_void, m.hwnd as *mut c_void) != 0),
    }
}

pub unsafe fn peek(out: *mut Msg, hwnd: *mut c_void, lo: u32, hi: u32, flags: u32) -> Result<bool, u32> {
    if out.is_null() {
        return Err(ERROR_INVALID_PARAMETER);
    }
    *out = Msg::default();
    let h = hwnd as usize;
    if h != 0 && h != usize::MAX && IsWindow(hwnd) == 0 {
        return Err(ERROR_INVALID_WINDOW_HANDLE);
    }
    let qs_mask = flags >> 16;
    let tid = GetCurrentThreadId();
    mark_consumer(tid);
    let tok = token();
    loop {
        if qs_mask == 0 || qs_mask & QS_POSTMESSAGE != 0 {
            drain(tid);
            let mut qs = lock();
            if let Some(q) = qs.get_mut(&tid) {
                if let Some(i) = q.ready.iter().position(|m| matches(m, h, lo, hi)) {
                    *out = q.ready[i];
                    if flags & PM_REMOVE != 0 {
                        q.ready.remove(i);
                    }
                    return Ok(true);
                }
            }
        }
        let mut m = Msg::default();
        if PeekMessageW(&mut m, hwnd, lo, hi, flags) == 0 {
            return Ok(false);
        }
        if m.message == tok && m.hwnd == 0 {
            if flags & PM_REMOVE == 0 {
                let mut t = Msg::default();
                PeekMessageW(&mut t, core::ptr::null_mut(), tok, tok, PM_REMOVE | PM_QS_POSTMESSAGE);
            }
            let mut qs = lock();
            let q = qs.entry(tid).or_default();
            if let Some(r) = q.pending.pop_front() {
                q.ready.push_back(r);
            }
            continue;
        }
        if crate::test_run() && !taken_over() && matches!(m.message, 0x0100 | 0x0104 | 0x0201 | 0x0204 | 0x0207 | 0x020B) {
            take_over(m.message);
        }
        if host_kbm_message(m.message) && host_kbm_hidden() {
            if flags & PM_REMOVE == 0 {
                let mut t = Msg::default();
                PeekMessageW(&mut t, m.hwnd as *mut c_void, m.message, m.message, PM_REMOVE);
            }
            continue;
        }
        *out = m;
        return Ok(true);
    }
}

pub fn host_kbm_message(msg: u32) -> bool {
    msg == 0x00FF || (0x0100..=0x0109).contains(&msg) || (0x0200..=0x020E).contains(&msg) || (0x02A0..=0x02A3).contains(&msg)
}

pub fn host_kbm_hidden() -> bool {
    static H: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *H.get_or_init(|| matches!(std::env::var("TOMBOX_HOST_KBM").ok().as_deref().map(str::trim), Some("0") | Some("off") | Some("false"))) && !taken_over()
}

pub unsafe fn get(out: *mut Msg, hwnd: *mut c_void, lo: u32, hi: u32) -> Result<bool, u32> {
    loop {
        if peek(out, hwnd, lo, hi, PM_REMOVE)? {
            return Ok((*out).message != WM_QUIT);
        }
        MsgWaitForMultipleObjectsEx(0, core::ptr::null(), INFINITE, QS_ALLINPUT, 0);
    }
}

pub unsafe fn dispatch(m: *const Msg) -> Result<isize, u32> {
    if m.is_null() {
        return Err(ERROR_INVALID_PARAMETER);
    }
    SetLastError(0);
    let r = DispatchMessageW(m);
    if r == 0 && GetLastError() == ERROR_MESSAGE_SYNC_ONLY && (*m).hwnd != 0 {
        let h = (*m).hwnd as *mut c_void;
        let p = GetWindowLongPtrW(h, GWLP_WNDPROC);
        if p != 0 {
            SetLastError(0);
            return Ok(CallWindowProcW(p, h, (*m).message, (*m).wparam, (*m).lparam));
        }
    }
    Ok(r)
}

pub fn queued(tid: u32) -> Vec<Msg> {
    let qs = lock();
    qs.get(&tid).map_or_else(Vec::new, |q| q.ready.iter().chain(q.pending.iter()).chain(q.overflow.iter()).copied().collect())
}

pub fn overflow_len(tid: u32) -> usize {
    lock().get(&tid).map_or(0, |q| q.overflow.len())
}

static TAKEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn taken_over() -> bool {
    if TAKEN.load(std::sync::atomic::Ordering::Acquire) {
        return true;
    }
    if !crate::test_run() {
        return false;
    }
    if std::env::var("TOMBOX_TAKEN").map(|v| !v.trim().is_empty()).unwrap_or(false) {
        if !TAKEN.swap(true, std::sync::atomic::Ordering::AcqRel) {
            crate::restore_test_windows();
        }
        return true;
    }
    false
}

pub fn takeover_record(pid: u32, title: &str, at: &str) -> String {
    format!("script_pid=0 game_pid={pid} title={title} started={at} takeover=1")
}

fn take_over(msg: u32) {
    if TAKEN.swap(true, std::sync::atomic::Ordering::AcqRel) {
        return;
    }
    let reason = format!("window message {msg:#x} on the test window");
    if !std::env::var("TOMBOX_TAKEN").map(|v| !v.trim().is_empty()).unwrap_or(false) {
        std::env::set_var("TOMBOX_TAKEN", &reason);
        let pid = std::process::id();
        let title = std::env::current_exe().ok().and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned())).unwrap_or_default();
        let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0).to_string();
        for (var, body) in [("TOMBOX_USER_PLAYING_FILE", takeover_record(pid, &title, &at)), ("TOMBOX_TAKEOVER_FILE", format!("pid={pid} reason={reason} at={at}"))] {
            if let Ok(path) = std::env::var(var) {
                if !path.trim().is_empty() {
                    let _ = std::fs::write(path.trim(), body);
                }
            }
        }
    }
    crate::restore_test_windows();
}
