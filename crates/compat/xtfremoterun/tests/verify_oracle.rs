use core::ffi::c_void;
use core::ptr::null_mut;
use std::sync::{Mutex, MutexGuard, OnceLock};
use xtfmsg::com::IID_XBTP_EXTENSION_METADATA;
use xtfmsg::sys::*;
use xtfremoterun::client::*;
use xtfremoterun::REMOTE_RUN_EXTENSION_ID;

extern "system" {
    fn LoadLibraryExW(name: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
    fn GetProcAddress(h: *mut c_void, name: *const u8) -> *mut c_void;
}

const LOAD_WITH_ALTERED_SEARCH_PATH: u32 = 8;
const BUILD_VAR: &str = "TOMBOX_CONSOLE_OSBUILD";
const CONSOLE_DLLS: [(&str, &str); 2] = [
    (r"D:\xbox-pc-port\games\XDefiant\extracted\EmbeddedXvd\Windows\System32\xtfremoterun.dll", "22000"),
    (r"D:\xbox-pc-port\games\Fortnite\extracted\EmbeddedXvd\Windows\System32\xtfremoterun.dll", "19041"),
];

type Factory = unsafe extern "system" fn(*const u16, *const GUID, *mut *mut c_void) -> HRESULT;
type Entry = unsafe extern "system" fn(*const GUID, *const GUID, *mut *mut c_void) -> HRESULT;
type Qi = unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT;
type Release = unsafe extern "system" fn(*mut c_void) -> u32;
type Count = unsafe extern "system" fn(*mut c_void, *mut u32) -> HRESULT;
type RunEx = unsafe extern "system" fn(*mut c_void, *const u16, *const u16, u32, u32, *mut c_void, *mut u32) -> HRESULT;
type Run = unsafe extern "system" fn(*mut c_void, *const u16, *const u16, u32, u32, *mut c_void) -> HRESULT;

struct Console {
    path: &'static str,
    build: &'static str,
    factory: Factory,
    entry: Entry,
}

unsafe impl Send for Console {}
unsafe impl Sync for Console {}

static LOCK: Mutex<()> = Mutex::new(());

fn guard() -> MutexGuard<'static, ()> {
    xtfmsg::fixture::load_xbtp();
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn consoles() -> &'static Vec<Console> {
    static C: OnceLock<Vec<Console>> = OnceLock::new();
    C.get_or_init(|| {
        let mut v = Vec::new();
        for (p, build) in CONSOLE_DLLS {
            if !std::path::Path::new(p).is_file() {
                eprintln!("{p} missing; that copy is skipped");
                continue;
            }
            let h = unsafe { LoadLibraryExW(wide(p).as_ptr(), null_mut(), LOAD_WITH_ALTERED_SEARCH_PATH) };
            if h.is_null() {
                eprintln!("{p} does not load on this PC; that copy is skipped");
                continue;
            }
            unsafe {
                let f = GetProcAddress(h, b"XtfCreateRemoteRunClient\0".as_ptr());
                let e = GetProcAddress(h, 4usize as *const u8);
                assert!(!f.is_null() && !e.is_null(), "{p}: exports");
                v.push(Console { path: p, build, factory: core::mem::transmute::<*mut c_void, Factory>(f), entry: core::mem::transmute::<*mut c_void, Entry>(e) });
            }
        }
        v
    })
}

struct Build;

impl Build {
    fn set(b: &str) -> Build {
        std::env::set_var(BUILD_VAR, b);
        Build
    }
}

impl Drop for Build {
    fn drop(&mut self) {
        std::env::remove_var(BUILD_VAR);
    }
}

unsafe fn slot<T: Copy>(o: *mut c_void, i: usize) -> T {
    let p = *(*(o as *const *const usize)).add(i);
    core::mem::transmute_copy(&p)
}

unsafe fn create(f: Factory, address: Option<&str>, iid: &GUID) -> (HRESULT, *mut c_void) {
    let a = address.map(wide);
    let mut out = 0x5555usize as *mut c_void;
    let hr = f(a.as_ref().map(|v| v.as_ptr()).unwrap_or(core::ptr::null()), iid, &mut out);
    (hr, out)
}

unsafe fn ours(address: Option<&str>, iid: &GUID) -> (HRESULT, *mut c_void) {
    let a = address.map(wide);
    let mut out = 0x5555usize as *mut c_void;
    let hr = XtfCreateRemoteRunClient(a.as_ref().map(|v| v.as_ptr()).unwrap_or(core::ptr::null()), iid, &mut out as *mut *mut c_void as *mut _);
    (hr, out)
}

unsafe fn release_all(o: *mut c_void) {
    let r = slot::<Release>(o, 2);
    while r(o) > 0 {}
}

const BAD: GUID = GUID::new(1, 2, 3, [4; 8]);

fn require_both() -> &'static Vec<Console> {
    let c = consoles();
    if c.len() < CONSOLE_DLLS.len() {
        eprintln!("only {} of {} console copies load here", c.len(), CONSOLE_DLLS.len());
    }
    c
}

#[test]
fn factory_and_query_interface_match_console_dll() {
    let _g = guard();
    for c in require_both() {
        let _b = Build::set(c.build);
        unsafe {
            for (addr, iid) in [(None, IID_XTF_REMOTE_RUN_CLIENT), (Some(""), IID_XTF_REMOTE_RUN_CLIENT2), (Some("1.2.3.4"), BAD)] {
                let (want, wout) = create(c.factory, addr, &iid);
                let (got, gout) = ours(addr, &iid);
                assert_eq!(got as u32, want as u32, "{}: {addr:?} {iid:?}", c.path);
                assert_eq!(wout.is_null(), gout.is_null(), "{}: {addr:?} out", c.path);
                if want >= 0 {
                    release_all(wout);
                    release_all(gout);
                }
            }
            for addr in [None, Some("127.0.0.1"), Some("1.2.3.4/title")] {
                let (want, console) = create(c.factory, addr, &IID_XTF_REMOTE_RUN_CLIENT);
                let (got, mine) = ours(addr, &IID_XTF_REMOTE_RUN_CLIENT);
                assert_eq!((got, want), (S_OK, S_OK), "{}: {addr:?}", c.path);
                for iid in [IID_IUNKNOWN, IID_XTF_REMOTE_RUN_CLIENT_BASE, IID_XTF_REMOTE_RUN_CLIENT, IID_XTF_REMOTE_RUN_CLIENT2, BAD] {
                    let mut a = 0x77usize as *mut c_void;
                    let mut b = 0x77usize as *mut c_void;
                    let hw = slot::<Qi>(console, 0)(console, &iid, &mut a);
                    let hg = slot::<Qi>(mine, 0)(mine, &iid, &mut b);
                    assert_eq!(hg, hw, "{}: {addr:?} QI {iid:?}", c.path);
                    assert_eq!(a == 0x77usize as *mut c_void, b == 0x77usize as *mut c_void, "{}: {addr:?} QI out untouched", c.path);
                    if hw >= 0 {
                        assert_eq!(a == console, b == mine, "{}: {addr:?} QI identity {iid:?}", c.path);
                        slot::<Release>(a, 2)(a);
                        slot::<Release>(b, 2)(b);
                    }
                }
                release_all(console);
                release_all(mine);
            }
        }
    }
}

unsafe fn run_both(c: &Console, addr: Option<&str>, cmd: Option<&str>, flags: u32, ex: bool) -> ((HRESULT, u32), (HRESULT, u32)) {
    let mut res = Vec::new();
    let iid = if ex { IID_XTF_REMOTE_RUN_CLIENT2 } else { IID_XTF_REMOTE_RUN_CLIENT };
    for console in [true, false] {
        let (hr, o) = if console { create(c.factory, addr, &iid) } else { ours(addr, &iid) };
        assert_eq!(hr, S_OK, "{}", c.path);
        let cmd_w = cmd.map(wide);
        let cmd_p = cmd_w.as_ref().map_or(core::ptr::null(), |v| v.as_ptr());
        let mut code = 0x4242u32;
        let hr = if ex { slot::<RunEx>(o, 3)(o, cmd_p, core::ptr::null(), flags, 0, null_mut(), &mut code) } else { slot::<Run>(o, 3)(o, cmd_p, core::ptr::null(), flags, 0, null_mut()) };
        release_all(o);
        res.push((hr, code));
    }
    (res[0], res[1])
}

#[test]
fn run_offline_paths_match_console_dll() {
    let _g = guard();
    let cases: [(Option<&str>, Option<&str>, u32); 7] = [
        (Some("127.0.0.1"), None, 0),
        (Some("127.0.0.1"), Some(""), 0),
        (Some("127.0.0.1"), Some("x.exe"), 0x4000_0000),
        (Some("127.0.0.1"), Some("x.exe"), 0x8000_0000),
        (None, Some("x.exe"), 0),
        (Some("1.2.3.4/bogus"), Some("x.exe"), 0),
        (Some("/title"), Some("x.exe"), 0),
    ];
    for c in require_both() {
        let _b = Build::set(c.build);
        unsafe {
            for ex in [false, true] {
                if ex && c.build == "19041" {
                    continue;
                }
                for (addr, cmd, flags) in cases {
                    let (want, got) = run_both(c, addr, cmd, flags, ex);
                    assert_eq!(got.0 as u32, want.0 as u32, "{}: {addr:?} {cmd:?} {flags:#x} ex={ex}", c.path);
                    assert_eq!(got.1, want.1, "{}: {addr:?} {cmd:?} {flags:#x} ex={ex} exit code", c.path);
                }
            }
        }
    }
}

#[test]
fn entry_point_matches_console_dll() {
    let _g = guard();
    for c in require_both() {
        let _b = Build::set(c.build);
        unsafe {
            let mut out = 0x1234usize as *mut c_void;
            assert_eq!((c.entry)(&BAD, &IID_XBTP_EXTENSION_METADATA, &mut out), E_UNEXPECTED, "{}", c.path);
            assert_eq!(xtfremoterun::XtfOrdinal_4(&BAD, &IID_XBTP_EXTENSION_METADATA, &mut out as *mut *mut c_void as *mut _), E_UNEXPECTED);
            assert_eq!((c.entry)(&REMOTE_RUN_EXTENSION_ID, &BAD, &mut out), E_UNEXPECTED);
            assert_eq!(xtfremoterun::XtfOrdinal_4(&REMOTE_RUN_EXTENSION_ID, &BAD, &mut out as *mut *mut c_void as *mut _), E_UNEXPECTED);
            assert_eq!(out, 0x1234usize as *mut c_void);
            let mut want: *mut c_void = null_mut();
            let mut got: *mut c_void = null_mut();
            assert_eq!((c.entry)(&REMOTE_RUN_EXTENSION_ID, &IID_XBTP_EXTENSION_METADATA, &mut want), S_OK);
            assert_eq!(xtfremoterun::XtfOrdinal_4(&REMOTE_RUN_EXTENSION_ID, &IID_XBTP_EXTENSION_METADATA, &mut got as *mut *mut c_void as *mut _), S_OK);
            assert!(!want.is_null() && !got.is_null());
            let (mut n, mut m) = (0u32, 0u32);
            assert_eq!(slot::<Count>(want, 3)(want, &mut n), S_OK);
            assert_eq!(slot::<Count>(got, 3)(got, &mut m), S_OK);
            assert_eq!((m, n), (1, 1), "{}", c.path);
            assert_eq!(xtfremoterun::HANDLERS.iter().map(|h| h.0).collect::<Vec<_>>(), [1]);
        }
    }
}

#[test]
fn both_console_copies_are_compared() {
    let _g = guard();
    let c = consoles();
    assert_eq!(c.iter().map(|c| c.build).collect::<Vec<_>>(), ["22000", "19041"], "the oracle must load both the 22000 and the 19041 console copy");
}
