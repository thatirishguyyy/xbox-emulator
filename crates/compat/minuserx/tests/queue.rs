use core::ffi::c_void;
use core::ptr::null_mut;
use std::sync::Mutex;

use minuserx::queue::{self, Msg, WM_MOUSEMOVE, WM_QUIT};
use minuserx::*;

#[link(name = "user32")]
extern "system" {
    fn CreateWindowExW(ex: u32, cls: *const u16, name: *const u16, style: u32, x: i32, y: i32, w: i32, h: i32, parent: *mut c_void, menu: *mut c_void, inst: *mut c_void, p: *mut c_void) -> *mut c_void;
    fn DestroyWindow(h: *mut c_void) -> i32;
    fn GetWindowTextW(h: *mut c_void, s: *mut u16, n: i32) -> i32;
    fn DefWindowProcW(h: *mut c_void, m: u32, w: usize, l: isize) -> isize;
    fn RegisterClassW(c: *const WndClassW) -> u16;
    fn PostMessageW(h: *mut c_void, m: u32, w: usize, l: isize) -> i32;
    fn PostQuitMessage(code: i32);
    fn GetMessageW(m: *mut Msg, h: *mut c_void, lo: u32, hi: u32) -> i32;
    fn DispatchMessageW(m: *const Msg) -> isize;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentThreadId() -> u32;
    fn SetLastError(e: u32);
    fn GetLastError() -> u32;
    fn Sleep(ms: u32);
}

#[repr(C)]
struct WndClassW {
    style: u32,
    proc_: Option<unsafe extern "system" fn(*mut c_void, u32, usize, isize) -> isize>,
    cls_extra: i32,
    wnd_extra: i32,
    instance: *mut c_void,
    icon: *mut c_void,
    cursor: *mut c_void,
    background: *mut c_void,
    menu_name: *const u16,
    class_name: *const u16,
}

static SERIAL: Mutex<()> = Mutex::new(());
static SEEN: Mutex<Vec<(u32, usize, isize)>> = Mutex::new(Vec::new());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

unsafe extern "system" fn record_proc(h: *mut c_void, m: u32, w: usize, l: isize) -> isize {
    if m == 0x000C || m >= 0x8000 && m < 0xC000 {
        SEEN.lock().unwrap_or_else(|e| e.into_inner()).push((m, w, l));
        if m == 0x8001 {
            return 77;
        }
    }
    DefWindowProcW(h, m, w, l)
}

struct Win(*mut c_void);

impl Win {
    fn new() -> Win {
        let name: Vec<u16> = "mux_queue_test\0".encode_utf16().collect();
        let mut wc: WndClassW = unsafe { core::mem::zeroed() };
        wc.proc_ = Some(record_proc);
        wc.class_name = name.as_ptr();
        unsafe { RegisterClassW(&wc) };
        let w = unsafe { CreateWindowExW(0, name.as_ptr(), name.as_ptr(), 0, 0, 0, 1, 1, null_mut(), null_mut(), null_mut(), null_mut()) };
        assert!(!w.is_null());
        let mut m = Msg::default();
        while unsafe { mux_PeekMessageW(&mut m, null_mut(), 0, 0, 1) } != 0 {}
        SEEN.lock().unwrap_or_else(|e| e.into_inner()).clear();
        Win(w)
    }
}

impl Drop for Win {
    fn drop(&mut self) {
        unsafe { DestroyWindow(self.0) };
    }
}

fn take(hwnd: *mut c_void, lo: u32, hi: u32) -> Vec<Msg> {
    let mut v = Vec::new();
    loop {
        let mut m = Msg::default();
        if unsafe { mux_PeekMessageW(&mut m, hwnd, lo, hi, 1) } == 0 {
            break;
        }
        v.push(m);
    }
    v
}

fn ids(v: &[Msg]) -> Vec<(u32, usize, isize)> {
    v.iter().filter(|m| m.message == WM_MOUSEMOVE || m.message == 0x000C || (m.message >= 0x8000 && m.message < 0xC000)).map(|m| (m.message, m.wparam, m.lparam)).collect()
}

fn deliver(hwnd: *mut c_void, msg: u32, w: usize, l: isize) -> i32 {
    unsafe {
        let cb = new_callback(false);
        let i = callback_iface(cb);
        let v: &CallbackVtbl = &**(i as *const *const CallbackVtbl);
        let hr = (v.deliver)(i, hwnd, msg, w, l);
        release(i);
        hr
    }
}

#[test]
fn deliver_coalesces_mouse_moves_and_keeps_the_latest_values() {
    let _g = serial();
    let w = Win::new();
    let h = w.0 as usize;
    let hrs: Vec<i32> = std::thread::spawn(move || {
        [(1, 10), (2, 20), (3, 30)]
            .iter()
            .map(|&(a, b)| {
                let r = deliver(h as *mut c_void, WM_MOUSEMOVE, a, b);
                unsafe { Sleep(20) };
                r
            })
            .collect()
    })
    .join()
    .unwrap();
    assert_eq!(hrs, vec![S_OK; 3]);
    let q = queue::queued(unsafe { GetCurrentThreadId() });
    let last = *q.iter().rev().find(|m| m.message == WM_MOUSEMOVE).unwrap();
    let mut peek = Msg::default();
    assert_eq!(unsafe { mux_PeekMessageW(&mut peek, w.0, WM_MOUSEMOVE, WM_MOUSEMOVE, 0) }, 1);
    assert_eq!((peek.hwnd, peek.wparam, peek.lparam, peek.time), (h, 3, 30, last.time));
    let got = take(w.0, WM_MOUSEMOVE, WM_MOUSEMOVE);
    assert_eq!(ids(&got), vec![(WM_MOUSEMOVE, 3, 30)]);
}

#[test]
fn title_posts_take_part_in_the_tail_check() {
    let _g = serial();
    let w = Win::new();
    unsafe {
        assert_eq!(deliver(w.0, WM_MOUSEMOVE, 1, 10), S_OK);
        assert_eq!(mux_PostMessageW(w.0, 0x8100, 5, 6), 1);
        assert_eq!(deliver(w.0, WM_MOUSEMOVE, 2, 20), S_OK);
        assert_eq!(mux_PostMessageA(w.0, WM_MOUSEMOVE, 4, 40), 1);
        assert_eq!(mux_PostThreadMessageW(GetCurrentThreadId(), 0x8101, 7, 8), 1);
        assert_eq!(mux_PostMessageW(w.0, WM_MOUSEMOVE, 9, 90), 1);
    }
    let got = take(null_mut(), 0, 0);
    assert_eq!(ids(&got), vec![(WM_MOUSEMOVE, 1, 10), (0x8100, 5, 6), (WM_MOUSEMOVE, 4, 40), (0x8101, 7, 8), (WM_MOUSEMOVE, 9, 90)]);
}

#[test]
fn foreign_user32_posts_keep_their_place_in_the_queue() {
    let _g = serial();
    let w = Win::new();
    unsafe {
        mux_PostMessageW(w.0, 0x8200, 1, 0);
        PostMessageW(w.0, 0x8201, 2, 0);
        mux_PostMessageW(w.0, 0x8202, 3, 0);
        PostMessageW(w.0, 0x8203, 4, 0);
    }
    assert_eq!(ids(&take(w.0, 0, 0)), vec![(0x8200, 1, 0), (0x8201, 2, 0), (0x8202, 3, 0), (0x8203, 4, 0)]);
}

#[test]
fn sync_only_messages_queue_and_dispatch_to_the_window_procedure() {
    let _g = serial();
    let w = Win::new();
    let text: Vec<u16> = "console\0".encode_utf16().collect();
    unsafe {
        SetLastError(5);
        assert_eq!(mux_PostMessageW(w.0, 0x000C, 0, text.as_ptr() as isize), 1);
        assert_eq!(GetLastError(), 0);
        let mut m = Msg::default();
        assert_eq!(mux_GetMessageW(&mut m, w.0, 0x000C, 0x000C), 1);
        assert_eq!((m.hwnd, m.message, m.lparam), (w.0 as usize, 0x000C, text.as_ptr() as isize));
        assert_eq!(mux_DispatchMessageW(&m), 1);
        assert_eq!(GetLastError(), 0);
    }
    assert_eq!(*SEEN.lock().unwrap(), vec![(0x000C, 0, text.as_ptr() as isize)]);
    let mut buf = [0u16; 16];
    let n = unsafe { GetWindowTextW(w.0, buf.as_mut_ptr(), 16) } as usize;
    assert_eq!(String::from_utf16_lossy(&buf[..n]), "console");
}

#[test]
fn ordinary_dispatch_is_unchanged() {
    let _g = serial();
    let w = Win::new();
    unsafe {
        PostMessageW(w.0, 0x8001, 3, 4);
        let mut m = Msg::default();
        assert_eq!(mux_GetMessageW(&mut m, null_mut(), 0, 0), 1);
        assert_eq!(m.message, 0x8001);
        assert_eq!(mux_DispatchMessageA(&m), 77);
        let thread_msg = Msg { message: 0x8002, ..Msg::default() };
        assert_eq!(mux_DispatchMessageW(&thread_msg), 0);
        assert_eq!(mux_DispatchMessageW(core::ptr::null()), 0);
        assert_eq!(GetLastError(), 87);
    }
    assert_eq!(*SEEN.lock().unwrap(), vec![(0x8001, 3, 4)]);
}

#[test]
fn range_and_window_filters_match_the_real_message_id() {
    let _g = serial();
    let w = Win::new();
    let text: Vec<u16> = "x\0".encode_utf16().collect();
    unsafe {
        mux_PostMessageW(w.0, 0x000C, 0, text.as_ptr() as isize);
        mux_PostThreadMessageW(GetCurrentThreadId(), 0x8300, 1, 2);
        mux_PostMessageW(w.0, 0x8301, 3, 4);
        let mut m = Msg { message: 99, ..Msg::default() };
        assert_eq!(mux_PeekMessageW(&mut m, w.0, 0x8000, 0x8000, 1), 0);
        assert_eq!(m, Msg::default());
        assert_eq!(mux_PeekMessageW(&mut m, w.0, 0x0C, 0x0C, 0), 1);
        assert_eq!(m.message, 0x0C);
        assert_eq!(mux_PeekMessageW(&mut m, usize::MAX as *mut c_void, 0, 0, 1), 1);
        assert_eq!((m.hwnd, m.message), (0, 0x8300));
        assert_eq!(mux_PeekMessageW(&mut m, w.0, 0x8301, 0x0C, 1), 1);
        assert_eq!(m.message, 0x0C);
        assert_eq!(mux_PeekMessageW(&mut m, w.0, 0x8301, 0x0C, 1), 1);
        assert_eq!(m.message, 0x8301);
        assert_eq!(mux_PeekMessageW(&mut m, w.0, 0x0B, 0x0D, 1), 0);
        assert_eq!(mux_PeekMessageW(&mut m, w.0, 0, 0, 1), 0);
    }
}

#[test]
fn more_than_ten_thousand_posts_are_held_and_delivered_in_order() {
    let _g = serial();
    let w = Win::new();
    let tid = unsafe { GetCurrentThreadId() };
    let n = 10_050usize;
    for i in 0..n {
        assert_eq!(unsafe { mux_PostMessageW(w.0, 0x8400, i, 0) }, 1, "post {i}");
    }
    assert!(queue::overflow_len(tid) > 0, "user32 quota never reached");
    let got: Vec<usize> = take(w.0, 0x8400, 0x8400).iter().map(|m| m.wparam).collect();
    assert_eq!(got, (0..n).collect::<Vec<_>>());
    assert_eq!(queue::overflow_len(tid), 0);
}

#[test]
fn threads_pumping_user32_directly_still_receive_posts() {
    let _g = serial();
    let (tx, rx) = std::sync::mpsc::channel();
    let t = std::thread::spawn(move || unsafe {
        let name: Vec<u16> = "mux_queue_test\0".encode_utf16().collect();
        let mut wc: WndClassW = core::mem::zeroed();
        wc.proc_ = Some(record_proc);
        wc.class_name = name.as_ptr();
        RegisterClassW(&wc);
        let w = CreateWindowExW(0, name.as_ptr(), name.as_ptr(), 0, 0, 0, 1, 1, null_mut(), null_mut(), null_mut(), null_mut());
        tx.send((w as usize, GetCurrentThreadId())).unwrap();
        let mut m = Msg::default();
        while GetMessageW(&mut m, null_mut(), 0, 0) > 0 {
            DispatchMessageW(&m);
            if m.message == 0x8601 {
                break;
            }
        }
        DestroyWindow(w);
    });
    let (w, tid) = rx.recv().unwrap();
    SEEN.lock().unwrap().clear();
    assert!(!queue::is_consumer(tid));
    unsafe {
        assert_eq!(mux_PostMessageW(w as *mut c_void, 0x8600, 1, 2), 1);
        assert_eq!(mux_PostMessageW(w as *mut c_void, 0x8601, 3, 4), 1);
    }
    t.join().unwrap();
    assert_eq!(*SEEN.lock().unwrap(), vec![(0x8600, 1, 2), (0x8601, 3, 4)]);
}

#[test]
fn queue_state_is_dropped_when_the_consumer_thread_exits() {
    let _g = serial();
    let tid = std::thread::spawn(|| unsafe {
        let mut m = Msg::default();
        mux_PeekMessageW(&mut m, null_mut(), 0, 0, 0);
        mux_PostThreadMessageW(GetCurrentThreadId(), 0x8700, 0, 0);
        let tid = GetCurrentThreadId();
        assert!(queue::is_consumer(tid));
        assert_eq!(queue::queued(tid).len(), 1);
        tid
    })
    .join()
    .unwrap();
    assert!(!queue::is_consumer(tid));
    assert!(queue::queued(tid).is_empty());
}

#[test]
fn get_message_waits_returns_zero_for_quit_and_minus_one_for_bad_windows() {
    let _g = serial();
    let w = Win::new();
    let h = w.0 as usize;
    let t = std::thread::spawn(move || {
        unsafe { Sleep(50) };
        unsafe { mux_PostMessageW(h as *mut c_void, 0x8500, 1, 2) }
    });
    let mut m = Msg::default();
    assert_eq!(unsafe { mux_GetMessageW(&mut m, w.0, 0x8500, 0x8500) }, 1);
    assert_eq!((m.message, m.wparam, m.lparam), (0x8500, 1, 2));
    assert_eq!(t.join().unwrap(), 1);
    unsafe { PostQuitMessage(3) };
    assert_eq!(unsafe { mux_GetMessageW(&mut m, null_mut(), 0, 0) }, 0);
    assert_eq!((m.message, m.wparam), (WM_QUIT, 3));
    let dead = Win::new();
    let d = dead.0;
    drop(dead);
    assert_eq!(unsafe { mux_GetMessageW(&mut m, d, 0, 0) }, -1);
    assert_eq!(unsafe { GetLastError() }, 1400);
    unsafe { SetLastError(0) };
    assert_eq!(unsafe { mux_PostMessageW(d, 0x8501, 0, 0) }, 0);
    assert_eq!(unsafe { GetLastError() }, 87);
}

#[test]
fn host_kbm_message_set_matches_compat_common() {
    for m in 0u32..0x400 {
        assert_eq!(minuserx::queue::host_kbm_message(m), compat_common::host_input::kbm_message(m), "{m:#x}");
    }
}
