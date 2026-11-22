use core::ffi::c_void;
use core::ptr::null_mut;
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use xtffileio::client::*;
use xtffileio::com::*;

extern "system" {
    fn LoadLibraryExW(name: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
    fn GetProcAddress(h: *mut c_void, name: *const u8) -> *mut c_void;
}

const LOAD_WITH_ALTERED_SEARCH_PATH: u32 = 8;
const CONSOLE_DLL: &str = r"D:\xbox-pc-port\games\XDefiant\extracted\EmbeddedXvd\Windows\System32\xtffileio.dll";

type Factory = unsafe extern "system" fn(*const u16, *const GUID, *mut *mut c_void) -> HRESULT;

struct Console(Factory);

unsafe impl Send for Console {}
unsafe impl Sync for Console {}

fn console() -> Option<&'static Console> {
    static C: OnceLock<Option<Console>> = OnceLock::new();
    C.get_or_init(|| {
        if !std::path::Path::new(CONSOLE_DLL).is_file() {
            return None;
        }
        unsafe {
            let h = LoadLibraryExW(wide(CONSOLE_DLL).as_ptr(), null_mut(), LOAD_WITH_ALTERED_SEARCH_PATH);
            if h.is_null() {
                return None;
            }
            let f = GetProcAddress(h, b"XtfCreateFileIOClient\0".as_ptr());
            assert!(!f.is_null(), "console xtffileio.dll has no XtfCreateFileIOClient");
            Some(Console(core::mem::transmute::<*mut c_void, Factory>(f)))
        }
    })
    .as_ref()
}

unsafe fn call(f: Factory, address: Option<&str>, iid: &GUID) -> (HRESULT, bool) {
    let w = address.map(wide);
    let p = w.as_ref().map(|v| v.as_ptr()).unwrap_or(core::ptr::null());
    let mut out = 0x55usize as *mut c_void;
    let hr = f(p, iid, &mut out);
    let null = out.is_null();
    if hr >= 0 && !null {
        release(out);
    }
    (hr, null)
}

unsafe extern "system" fn ours(address: *const u16, iid: *const GUID, ppv: *mut *mut c_void) -> HRESULT {
    create_client(address, iid, ppv)
}

#[test]
fn factory_address_errors_match_console_dll() {
    let Some(c) = console() else { return };
    let long = "b".repeat(MAX_ADDRESS_CHARS);
    let cases: Vec<Option<&str>> = vec![None, Some(""), Some("/title"), Some("/system"), Some("127.0.0.1/bogus"), Some("127.0.0.1\\nope"), Some(&long)];
    for addr in cases {
        for iid in [IID_XTF_FILEIO_CLIENT, IID_XTF_FILEIO_CLIENT2, IID_XBTP_CLIENT_SESSION] {
            let want = unsafe { call(c.0, addr, &iid) };
            let got = unsafe { call(ours, addr, &iid) };
            assert_eq!(got.0 as u32, want.0 as u32, "{addr:?}");
            assert_eq!(got.1, want.1, "{addr:?} ppv cleared");
            assert!(want.0 < 0);
        }
    }
}

#[test]
fn factory_offline_remote_addresses_match_console_dll() {
    let Some(c) = console() else { return };
    let f = c.0 as usize;
    let addrs = ["192.0.2.1", "192.0.2.1/title", "fileio-oracle.invalid"];
    let mut threads = Vec::new();
    for a in addrs {
        threads.push(std::thread::spawn(move || {
            let start = Instant::now();
            let want = unsafe { call(core::mem::transmute::<usize, Factory>(f), Some(a), &IID_XTF_FILEIO_CLIENT) };
            let mid = Instant::now();
            let got = unsafe { call(ours, Some(a), &IID_XTF_FILEIO_CLIENT) };
            (a, want, got, mid - start, mid.elapsed())
        }));
    }
    let deadline = Instant::now() + Duration::from_secs(300);
    for t in threads {
        while !t.is_finished() {
            assert!(Instant::now() < deadline, "offline factory calls did not finish within 300 s");
            std::thread::sleep(Duration::from_millis(100));
        }
        let (a, want, got, tw, tg) = t.join().unwrap();
        assert_eq!(got.0 as u32, want.0 as u32, "{a}");
        assert!(want.0 < 0, "{a}: console connected to a non-local address");
        assert_eq!(got.1, want.1, "{a}");
        let (lo, hi) = if tw < tg { (tw, tg) } else { (tg, tw) };
        assert!(hi.saturating_sub(lo) < Duration::from_secs(20), "{a}: console {tw:?} vs ours {tg:?}");
    }
}
