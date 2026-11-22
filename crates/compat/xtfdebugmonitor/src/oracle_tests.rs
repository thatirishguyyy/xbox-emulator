use crate::build_tests::{FORTNITE_DLL, XDEFIANT_DLL};
use crate::client::*;
use crate::profile::*;
use crate::server::*;
use crate::tests::canon;
use crate::DEBUG_MONITOR_EXTENSION_ID;
use core::ffi::c_void;
use core::ptr::null_mut;
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use xtfmsg::com::{Com, IID_XBTP_EXTENSION_METADATA};
use xtfmsg::fixture::{end_json, end_message, message_with_json};
use xtfmsg::sys::*;

extern "system" {
    fn LoadLibraryExW(name: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
    fn GetProcAddress(h: *mut c_void, name: *const u8) -> *mut c_void;
}

const LOAD_WITH_ALTERED_SEARCH_PATH: u32 = 8;
const DLL_ENV: &str = "XTFDEBUGMONITOR_ORACLE_DLL";
const CHILD_TEST: &str = "oracle_tests::oracle_child_main";

type Factory = unsafe extern "system" fn(*const u16, *const GUID, *mut *mut c_void) -> HRESULT;
type Entry = unsafe extern "system" fn(*const GUID, *const GUID, *mut *mut c_void) -> HRESULT;
type Start = unsafe extern "system" fn(*mut c_void, u32, usize, *mut c_void) -> HRESULT;
type Qi = unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT;
type Count = unsafe extern "system" fn(*mut c_void, *mut u32) -> HRESULT;
type Handler = unsafe extern "system" fn(usize, usize, usize) -> HRESULT;
type Inner = unsafe fn(&'static Profile, usize, usize, usize) -> HRESULT;

struct Copy {
    path: &'static str,
    size: u64,
    profile: &'static Profile,
    handlers: [(u32, usize); 4],
}

const COPIES: [Copy; 2] = [
    Copy { path: XDEFIANT_DLL, size: 147456, profile: &P22000, handlers: [(5, 0x8950), (3, 0x9730), (2, 0x9a50), (4, 0x9d30)] },
    Copy { path: FORTNITE_DLL, size: 105472, profile: &P19041, handlers: [(5, 0x7770), (3, 0x8560), (2, 0x8830), (4, 0x8b00)] },
];

unsafe fn slot<T: core::marker::Copy>(o: *mut c_void, i: usize) -> T {
    let p = *(*(o as *const *const usize)).add(i);
    core::mem::transmute_copy(&p)
}

unsafe fn create(f: Factory, address: Option<&str>, iid: &GUID) -> (HRESULT, *mut c_void) {
    let a = address.map(wide);
    let mut out = 0x5555usize as *mut c_void;
    let hr = f(a.as_ref().map(|v| v.as_ptr()).unwrap_or(core::ptr::null()), iid, &mut out);
    (hr, out)
}

const BAD: GUID = GUID::new(1, 2, 3, [4; 8]);

unsafe fn invoke(f: &dyn Fn(usize, usize, usize) -> HRESULT, session: usize, request: Option<&str>) -> (HRESULT, String) {
    let req = match request {
        Some(j) => message_with_json(j),
        None => Com::null(),
    };
    let reply = end_message();
    let hr = f(session, req.get() as usize, reply.get() as usize);
    (hr, end_json(reply.get()).unwrap_or_default())
}

fn inner_for(op: u32) -> Inner {
    match op {
        5 => query_monitor,
        3 => monitor_process,
        2 => detach_process,
        _ => continue_process,
    }
}

unsafe fn compare(c: &Copy, h: *mut c_void) {
    let path = c.path;
    let factory: Factory = core::mem::transmute(GetProcAddress(h, b"XtfCreateDebugMonitorClient\0".as_ptr()));
    let entry: Entry = core::mem::transmute(GetProcAddress(h, 4usize as *const u8));
    for (addr, iid) in [(Some(""), IID_XTF_DEBUG_MONITOR_CLIENT), (Some("127.0.0.1/bogus"), IID_XTF_DEBUG_MONITOR_CLIENT), (None, IID_XTF_DEBUG_MONITOR_CLIENT), (Some(&"a".repeat(0x200) as &str), IID_XTF_DEBUG_MONITOR_CLIENT), (Some("127.0.0.1"), BAD)] {
        let (want, wout) = create(factory, addr, &iid);
        let (got, gout) = create(XtfCreateDebugMonitorClient, addr, &iid);
        assert_eq!(got as u32, want as u32, "{path}: {addr:?}");
        assert_eq!(wout, null_mut(), "{addr:?}");
        assert_eq!(gout, null_mut(), "{addr:?}");
    }
    for addr in ["127.0.0.1", "127.0.0.1/system", "127.0.0.1/HOST", "10.0.0.1+key/title", &"a".repeat(0x1ff)] {
        let (want, console) = create(factory, Some(addr), &IID_XTF_DEBUG_MONITOR_CLIENT);
        let (got, ours) = create(XtfCreateDebugMonitorClient, Some(addr), &IID_XTF_DEBUG_MONITOR_CLIENT);
        assert_eq!((got, want), (S_OK, S_OK), "{path}: {addr}");
        for iid in [IID_IUNKNOWN, IID_XTF_DEBUG_MONITOR_CLIENT_BASE, IID_XTF_DEBUG_MONITOR_CLIENT, BAD] {
            let mut a = 0x77usize as *mut c_void;
            let mut b = 0x77usize as *mut c_void;
            let hw = slot::<Qi>(console, 0)(console, &iid, &mut a);
            let hg = slot::<Qi>(ours, 0)(ours, &iid, &mut b);
            assert_eq!(hg, hw, "{path}: {addr} QI");
            assert_eq!(a == console, b == ours, "{addr} QI identity");
            assert_eq!(a == 0x77usize as *mut c_void, b == 0x77usize as *mut c_void, "{addr} QI out untouched");
        }
        assert_eq!(slot::<Start>(ours, 3)(ours, 7, 0, null_mut()), slot::<Start>(console, 3)(console, 7, 0, null_mut()), "{path}: {addr} Start null callback");
        let r = slot::<unsafe extern "system" fn(*mut c_void) -> u32>(ours, 2);
        while r(ours) > 0 {}
    }
    let mut out = 0x1234usize as *mut c_void;
    assert_eq!(entry(&BAD, &IID_XBTP_EXTENSION_METADATA, &mut out), E_UNEXPECTED, "{path}");
    assert_eq!(crate::XtfOrdinal_4(&BAD, &IID_XBTP_EXTENSION_METADATA, &mut out), E_UNEXPECTED);
    assert_eq!(entry(&DEBUG_MONITOR_EXTENSION_ID, &BAD, &mut out), E_UNEXPECTED);
    assert_eq!(crate::XtfOrdinal_4(&DEBUG_MONITOR_EXTENSION_ID, &BAD, &mut out), E_UNEXPECTED);
    assert_eq!(out, 0x1234usize as *mut c_void);
    let mut md: *mut c_void = null_mut();
    assert_eq!(entry(&DEBUG_MONITOR_EXTENSION_ID, &IID_XBTP_EXTENSION_METADATA, &mut md), S_OK);
    assert!(!md.is_null());
    let mut n = 0u32;
    assert_eq!(slot::<Count>(md, 3)(md, &mut n), S_OK);
    assert_eq!(n as usize, crate::HANDLERS.len());
    assert_eq!(crate::HANDLERS.iter().map(|h| h.0).collect::<Vec<_>>(), [5, 3, 2, 4]);
    let requests: [(u32, usize, Option<&str>); 9] = [
        (5, 0, None),
        (3, 1, None),
        (3, 1, Some("{}")),
        (3, 0, Some("{\"ProcessId\":4294967075}")),
        (2, 0, None),
        (2, 0, Some("{\"ForceAll\":1}")),
        (4, 0, None),
        (4, 0, Some("{\"ExceptionContinue\":65538}")),
        (4, 0, Some("{\"ProcessId\":4294967075}")),
    ];
    let mut compared = 0;
    for (op, session, request) in requests {
        let rva = c.handlers.iter().find(|h| h.0 == op).unwrap().1;
        let console: Handler = core::mem::transmute((h as usize + rva) as *const ());
        let (hw, tw) = invoke(&|s, r, p| console(s, r, p), session, request);
        let inner = inner_for(op);
        let (hg, tg) = invoke(&|s, r, p| inner(c.profile, s, r, p), session, request);
        assert_eq!(hg as u32, hw as u32, "{path}: op {op} {request:?}");
        assert_eq!(canon(&tg), canon(&tw), "{path}: op {op} {request:?}");
        if hw < 0 {
            compared += 1;
        }
    }
    assert_eq!(compared, 8, "{path}: every error request must fail on the console copy");
}

#[test]
#[ignore]
fn oracle_child_main() {
    let Ok(path) = std::env::var(DLL_ENV) else { return };
    let c = COPIES.iter().find(|c| c.path == path).expect("known console copy");
    let size = std::fs::metadata(c.path).unwrap().len();
    assert_eq!(size, c.size, "{path}: handler RVAs belong to a different build of this DLL");
    let h = unsafe { LoadLibraryExW(wide(c.path).as_ptr(), null_mut(), LOAD_WITH_ALTERED_SEARCH_PATH) };
    if h.is_null() {
        println!("ORACLE-NOLOAD {} {}", c.path, unsafe { GetLastError() });
        return;
    }
    assert!(!unsafe { GetProcAddress(h, b"XtfCreateDebugMonitorClient\0".as_ptr()) }.is_null());
    let _g = UseProfile::on(c.profile);
    unsafe { compare(c, h) };
    println!("ORACLE-OK {}", c.path);
}

fn run_child(c: &Copy) {
    assert!(std::path::Path::new(c.path).is_file(), "{} is missing", c.path);
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([CHILD_TEST, "--exact", "--ignored", "--nocapture", "--test-threads=1", "-q"])
        .env(DLL_ENV, c.path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    let status = loop {
        if let Some(s) = child.try_wait().unwrap() {
            break s;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("oracle child for {} timed out", c.path);
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let mut out = String::new();
    let mut err = String::new();
    child.stdout.take().unwrap().read_to_string(&mut out).unwrap();
    child.stderr.take().unwrap().read_to_string(&mut err).unwrap();
    assert!(status.success(), "oracle child for {} failed: {status}\n{out}\n{err}", c.path);
    if let Some(l) = out.lines().find(|l| l.starts_with("ORACLE-NOLOAD")) {
        panic!("console DLL did not load, nothing was compared: {l}");
    }
    assert!(out.lines().any(|l| l == format!("ORACLE-OK {}", c.path)), "oracle child for {} printed no result:\n{out}\n{err}", c.path);
}

#[test]
fn xdefiant_22000_copy_matches_this_port() {
    run_child(&COPIES[0]);
}

#[test]
fn fortnite_19041_copy_matches_this_port() {
    run_child(&COPIES[1]);
}
