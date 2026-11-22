use core::ffi::c_void;
use core::ptr::null_mut;
use std::path::PathBuf;
use std::sync::OnceLock;

use minuserx::queue::Msg;
use minuserx::{DLL_FILE, EXPORTS_TXT};

#[link(name = "kernel32")]
extern "system" {
    fn LoadLibraryW(p: *const u16) -> *mut c_void;
    fn GetProcAddress(h: *mut c_void, n: *const u8) -> *mut c_void;
    fn GetLastError() -> u32;
    fn SetLastError(e: u32);
    fn GetCurrentThreadId() -> u32;
}

#[link(name = "user32")]
extern "system" {
    fn CreateWindowExW(ex: u32, cls: *const u16, name: *const u16, style: u32, x: i32, y: i32, w: i32, h: i32, parent: *mut c_void, menu: *mut c_void, inst: *mut c_void, p: *mut c_void) -> *mut c_void;
    fn DestroyWindow(h: *mut c_void) -> i32;
}

fn built() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    let mut c: Vec<PathBuf> = exe.ancestors().map(|d| d.join(DLL_FILE)).collect();
    if let Some(b) = exe.ancestors().find(|d| d.file_name().map_or(false, |n| n == "build")) {
        for k in std::fs::read_dir(b).into_iter().flatten().flatten() {
            for h in std::fs::read_dir(k.path()).into_iter().flatten().flatten() {
                c.push(h.path().join("out").join(DLL_FILE));
            }
        }
    }
    c.into_iter()
        .filter_map(|p| std::fs::metadata(&p).and_then(|m| m.modified()).ok().map(|t| (t, p)))
        .max()
        .map(|(_, p)| p)
        .unwrap_or_else(|| panic!("{DLL_FILE} not built near {}", exe.display()))
}

fn dll() -> usize {
    static H: OnceLock<usize> = OnceLock::new();
    *H.get_or_init(|| {
        let p: Vec<u16> = built().display().to_string().encode_utf16().chain(Some(0)).collect();
        let h = unsafe { LoadLibraryW(p.as_ptr()) } as usize;
        assert_ne!(h, 0);
        h
    })
}

fn export(name: &str) -> usize {
    let p = unsafe { GetProcAddress(dll() as *mut c_void, format!("{name}\0").as_ptr()) } as usize;
    assert_ne!(p, 0, "{name} not exported");
    p
}

fn notimpl(name: &str) {
    let f: unsafe extern "system" fn(usize, usize, usize, usize) -> usize = unsafe { core::mem::transmute(export(name)) };
    unsafe { SetLastError(0) };
    assert_eq!(unsafe { f(1, 2, 3, 4) }, 0, "{name}");
    assert_eq!(unsafe { GetLastError() }, 0x8000_4001, "{name}");
}

macro_rules! notimpl_by_name {
    ($($t:ident: $n:ident),*) => {
        $(
            #[test]
            fn $t() {
                notimpl(stringify!($n));
            }
        )*
    };
}

notimpl_by_name!(
    clip_cursor: ClipCursor,
    get_clip_cursor: GetClipCursor,
    get_cursor: GetCursor,
    get_message_extra_info: GetMessageExtraInfo,
    get_message_pos: GetMessagePos,
    get_message_time: GetMessageTime,
    in_send_message: InSendMessage,
    in_send_message_ex: InSendMessageEx,
    register_window_message_a: RegisterWindowMessageA,
    register_window_message_w: RegisterWindowMessageW,
    send_message_callback_a: SendMessageCallbackA,
    send_message_callback_w: SendMessageCallbackW,
    send_message_timeout_a: SendMessageTimeoutA,
    send_message_timeout_w: SendMessageTimeoutW,
    send_notify_message_a: SendNotifyMessageA,
    send_notify_message_w: SendNotifyMessageW,
    set_cursor_pos: SetCursorPos,
    set_message_extra_info: SetMessageExtraInfo,
    wait_message: WaitMessage
);

#[test]
fn ordinal_2534_by_ordinal() {
    let p = unsafe { GetProcAddress(dll() as *mut c_void, 2534usize as *const u8) } as usize;
    let f: unsafe extern "system" fn() -> usize = unsafe { core::mem::transmute(p) };
    unsafe { SetLastError(0) };
    assert_eq!(unsafe { f() }, 0);
    assert_eq!(unsafe { GetLastError() }, 0x8000_4001);
}

type GetHooks = unsafe extern "system" fn(*mut *mut c_void) -> i32;
type SetHooks = unsafe extern "system" fn(*mut c_void) -> i32;

#[test]
fn min_user_xp_get_hooks_and_set_hooks() {
    let get: GetHooks = unsafe { core::mem::transmute(export("MinUserXP_GetHooks")) };
    let set: SetHooks = unsafe { core::mem::transmute(export("MinUserXP_SetHooks")) };
    assert_eq!(unsafe { set(null_mut()) }, 0);
    let mut out = 1usize as *mut c_void;
    assert_eq!(unsafe { get(&mut out) }, 0);
    assert!(out.is_null());
}

type Post = unsafe extern "system" fn(*mut c_void, u32, usize, isize) -> i32;
type PostThread = unsafe extern "system" fn(u32, u32, usize, isize) -> i32;
type Peek = unsafe extern "system" fn(*mut Msg, *mut c_void, u32, u32, u32) -> i32;
type Get = unsafe extern "system" fn(*mut Msg, *mut c_void, u32, u32) -> i32;
type Dispatch = unsafe extern "system" fn(*const Msg) -> isize;

fn message_rows(a: bool) {
    let n = if a { ["PostMessageA", "PostThreadMessageA", "PeekMessageA", "GetMessageA", "DispatchMessageA"] } else { ["PostMessageW", "PostThreadMessageW", "PeekMessageW", "GetMessageW", "DispatchMessageW"] };
    let post: Post = unsafe { core::mem::transmute(export(n[0])) };
    let post_thread: PostThread = unsafe { core::mem::transmute(export(n[1])) };
    let peek: Peek = unsafe { core::mem::transmute(export(n[2])) };
    let get: Get = unsafe { core::mem::transmute(export(n[3])) };
    let dispatch: Dispatch = unsafe { core::mem::transmute(export(n[4])) };
    let cls: Vec<u16> = "STATIC\0".encode_utf16().collect();
    unsafe {
        let w = CreateWindowExW(0, cls.as_ptr(), cls.as_ptr(), 0, 0, 0, 1, 1, null_mut(), null_mut(), null_mut(), null_mut());
        assert!(!w.is_null());
        let mut m = Msg::default();
        while peek(&mut m, null_mut(), 0, 0, 1) != 0 {}
        let text: Vec<u16> = "byname\0".encode_utf16().collect();
        assert_eq!(post(w, 0x000C, 0, text.as_ptr() as isize), 1);
        assert_eq!(post_thread(GetCurrentThreadId(), 0x8800, 1, 2), 1);
        assert_eq!(peek(&mut m, usize::MAX as *mut c_void, 0, 0, 0), 1);
        assert_eq!((m.hwnd, m.message, m.wparam, m.lparam), (0, 0x8800, 1, 2));
        assert_eq!(get(&mut m, w, 0x000C, 0x000C), 1);
        assert_eq!(m.message, 0x000C);
        assert_eq!(dispatch(&m), 1);
        assert_eq!(get(&mut m, usize::MAX as *mut c_void, 0x8800, 0x8800), 1);
        assert_eq!(m.message, 0x8800);
        assert_eq!(peek(&mut m, null_mut(), 0x8800, 0x8800, 1), 0);
        DestroyWindow(w);
    }
}

#[test]
fn post_peek_get_dispatch_message_w() {
    message_rows(false);
}

#[test]
fn post_peek_get_dispatch_message_a() {
    message_rows(true);
}

#[test]
fn every_implemented_row_is_named_here() {
    let src = include_str!("by_name.rs");
    for l in EXPORTS_TXT.lines() {
        let f: Vec<&str> = l.split_whitespace().collect();
        if f.len() >= 3 && f[2] == "implemented" && !f[1].starts_with('#') {
            let n = f[1];
            let named = src.contains(&format!(": {n},")) || src.contains(&format!(": {n}\n")) || src.contains(&format!("\"{n}\""));
            assert!(named, "{n} has no by-name test");
        }
    }
}

#[repr(C)]
struct WndClassExA {
    size: u32,
    style: u32,
    proc_: usize,
    cls_extra: i32,
    wnd_extra: i32,
    inst: *mut c_void,
    icon: *mut c_void,
    cursor: *mut c_void,
    bg: *mut c_void,
    menu: *const u8,
    name: *const u8,
    icon_sm: *mut c_void,
}

#[link(name = "user32")]
extern "system" {
    fn GetModuleHandleW(n: *const u16) -> *mut c_void;
    fn RegisterClassExA(c: *const WndClassExA) -> u16;
    fn UnregisterClassA(n: *const u8, inst: *mut c_void) -> i32;
    fn CreateWindowExA(ex: u32, cls: *const u8, name: *const u8, style: u32, x: i32, y: i32, w: i32, h: i32, parent: *mut c_void, menu: *mut c_void, inst: *mut c_void, p: *mut c_void) -> *mut c_void;
    fn DefWindowProcA(h: *mut c_void, m: u32, w: usize, l: isize) -> isize;
    fn IsWindowUnicode(h: *mut c_void) -> i32;
}

const GWLP_WNDPROC: i32 = -4;
const GWLP_USERDATA: i32 = -21;

unsafe extern "system" fn proc_a(h: *mut c_void, m: u32, w: usize, l: isize) -> isize {
    DefWindowProcA(h, m, w, l)
}

unsafe extern "system" fn proc_b(h: *mut c_void, m: u32, w: usize, l: isize) -> isize {
    DefWindowProcA(h, m, w, l)
}

type GetLong = unsafe extern "system" fn(*mut c_void, i32) -> isize;
type SetLong = unsafe extern "system" fn(*mut c_void, i32, isize) -> isize;

#[test]
fn ansi_window_wndproc_reads_back_registered_proc() {
    let get_a: GetLong = unsafe { core::mem::transmute(export("GetWindowLongPtrA")) };
    let get_w: GetLong = unsafe { core::mem::transmute(export("GetWindowLongPtrW")) };
    let set_a: SetLong = unsafe { core::mem::transmute(export("SetWindowLongPtrA")) };
    let set_w: SetLong = unsafe { core::mem::transmute(export("SetWindowLongPtrW")) };
    unsafe {
        let inst = GetModuleHandleW(null_mut());
        let name = b"TomboxMuxAnsiLong\0";
        let c = WndClassExA { size: core::mem::size_of::<WndClassExA>() as u32, style: 0, proc_: proc_a as usize, cls_extra: 0, wnd_extra: 0, inst, icon: null_mut(), cursor: null_mut(), bg: null_mut(), menu: core::ptr::null(), name: name.as_ptr(), icon_sm: null_mut() };
        assert_ne!(RegisterClassExA(&c), 0);
        let h = CreateWindowExA(0, name.as_ptr(), b"t\0".as_ptr(), 0, 0, 0, 10, 10, null_mut(), null_mut(), inst, null_mut());
        assert!(!h.is_null());
        assert_eq!(IsWindowUnicode(h), 0);
        assert_eq!(get_a(h, GWLP_WNDPROC) as usize, proc_a as usize);
        assert_eq!(get_w(h, GWLP_WNDPROC) as usize, proc_a as usize);
        assert_eq!(set_a(h, GWLP_WNDPROC, proc_b as isize) as usize, proc_a as usize);
        assert_eq!(get_a(h, GWLP_WNDPROC) as usize, proc_b as usize);
        assert_eq!(set_w(h, GWLP_WNDPROC, proc_a as isize) as usize, proc_b as usize);
        assert_eq!(IsWindowUnicode(h), 0);
        assert_eq!(get_a(h, GWLP_WNDPROC) as usize, proc_a as usize);
        assert_eq!(set_a(h, GWLP_USERDATA, 0x1234), 0);
        assert_eq!(get_w(h, GWLP_USERDATA), 0x1234);
        for bad in [-6, -8, -12, -16, -20] {
            SetLastError(0x77);
            assert_eq!(get_w(h, bad), 0, "get index {bad}");
            assert_eq!(GetLastError(), 87, "get index {bad}");
            SetLastError(0x77);
            assert_eq!(get_a(h, bad), 0);
            assert_eq!(GetLastError(), 87);
            SetLastError(0x77);
            assert_eq!(set_w(h, bad, 5), 0, "set index {bad}");
            assert_eq!(GetLastError(), 87, "set index {bad}");
            SetLastError(0x77);
            assert_eq!(set_a(h, bad, 5), 0);
            assert_eq!(GetLastError(), 87);
        }
        SetLastError(0x77);
        assert_eq!(get_a(h, GWLP_USERDATA), 0x1234);
        assert_eq!(GetLastError(), 0);
        SetLastError(0x77);
        assert_eq!(set_a(h, GWLP_USERDATA, 0), 0x1234);
        assert_eq!(GetLastError(), 0);
        DestroyWindow(h);
        UnregisterClassA(name.as_ptr(), inst);
    }
}

#[link(name = "user32")]
extern "system" {
    fn GetWindowTextW(h: *mut c_void, buf: *mut u16, n: i32) -> i32;
    fn IsWindowVisible(h: *mut c_void) -> i32;
    fn DefWindowProcW(h: *mut c_void, m: u32, w: usize, l: isize) -> isize;
}

unsafe extern "system" fn proc_w(h: *mut c_void, m: u32, w: usize, l: isize) -> isize {
    DefWindowProcW(h, m, w, l)
}

#[test]
fn utf8_ansi_windows_are_unicode_by_name() {
    type RegA = unsafe extern "system" fn(*const WndClassExA) -> u16;
    type CreateA = unsafe extern "system" fn(u32, *const u8, *const u8, u32, i32, i32, i32, i32, *mut c_void, *mut c_void, *mut c_void, *mut c_void) -> *mut c_void;
    type CreateW = unsafe extern "system" fn(u32, *const u16, *const u16, u32, i32, i32, i32, i32, *mut c_void, *mut c_void, *mut c_void, *mut c_void) -> *mut c_void;
    type Show = unsafe extern "system" fn(*mut c_void, i32) -> i32;
    let reg: RegA = unsafe { core::mem::transmute(export("RegisterClassExA")) };
    let create_a: CreateA = unsafe { core::mem::transmute(export("CreateWindowExA")) };
    let create_w: CreateW = unsafe { core::mem::transmute(export("CreateWindowExW")) };
    let show: Show = unsafe { core::mem::transmute(export("ShowWindow")) };
    unsafe {
        let inst = GetModuleHandleW(null_mut());
        let name = b"TomboxMuxUtf8\0";
        let c = WndClassExA { size: core::mem::size_of::<WndClassExA>() as u32, style: 0, proc_: proc_w as usize, cls_extra: 0, wnd_extra: 0, inst, icon: null_mut(), cursor: null_mut(), bg: null_mut(), menu: core::ptr::null(), name: name.as_ptr(), icon_sm: null_mut() };
        assert_ne!(reg(&c), 0);
        let h = create_a(0, name.as_ptr(), "Trackmania \u{e9}\0".as_ptr(), 0, 0, 0, 10, 10, null_mut(), null_mut(), inst, null_mut());
        assert!(!h.is_null());
        assert_ne!(IsWindowUnicode(h), 0);
        let mut buf = [0u16; 64];
        let n = GetWindowTextW(h, buf.as_mut_ptr(), 64) as usize;
        assert!(String::from_utf16_lossy(&buf[..n]).ends_with("Trackmania \u{e9}"));
        show(h, 5);
        assert_ne!(IsWindowVisible(h), 0);
        DestroyWindow(h);
        let wname: Vec<u16> = "TomboxMuxUtf8\0".encode_utf16().collect();
        let t: Vec<u16> = "w\0".encode_utf16().collect();
        let h = create_w(0, wname.as_ptr(), t.as_ptr(), 0, 0, 0, 10, 10, null_mut(), null_mut(), inst, null_mut());
        assert!(!h.is_null());
        DestroyWindow(h);
        UnregisterClassA(name.as_ptr(), inst);
    }
}
