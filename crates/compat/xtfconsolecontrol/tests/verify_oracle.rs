use core::ffi::c_void;
use core::ptr::null_mut;
use std::sync::OnceLock;
use xtfconsolecontrol::client::*;
use xtfmsg::com::{release, IID_IUNKNOWN};
use xtfmsg::sys::*;

extern "system" {
    fn LoadLibraryExW(name: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
}

const LOAD_WITH_ALTERED_SEARCH_PATH: u32 = 8;
const CONSOLE_DLL: &str = r"D:\xbox-pc-port\games\Fortnite\extracted\EmbeddedXvd\Windows\System32\xtfconsolecontrol.dll";

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
            assert!(!h.is_null(), "console xtfconsolecontrol.dll failed to load: {}", GetLastError());
            let f = GetProcAddress(h, b"XtfCreateConsoleControlClient\0".as_ptr());
            assert!(!f.is_null(), "console xtfconsolecontrol.dll has no XtfCreateConsoleControlClient");
            Some(Console(core::mem::transmute::<*mut c_void, Factory>(f)))
        }
    })
    .as_ref()
}

unsafe fn create(f: Factory, address: Option<&str>, iid: &GUID) -> (HRESULT, *mut c_void) {
    let w = address.map(wide);
    let p = w.as_ref().map(|v| v.as_ptr()).unwrap_or(core::ptr::null());
    let mut out = 0x55usize as *mut c_void;
    let hr = f(p, iid, &mut out);
    (hr, out)
}

unsafe fn vt<'a>(o: *mut c_void) -> &'a ConsoleControlClientVtbl {
    &**(o as *const *const ConsoleControlClientVtbl)
}

fn factories() -> Option<[Factory; 2]> {
    console().map(|c| [c.0, XtfCreateConsoleControlClient as Factory])
}

#[test]
fn factory_and_query_interface_match_console_dll() {
    let Some(fs) = factories() else { return };
    let other = GUID::new(1, 2, 3, [4; 8]);
    let mut seen = vec![];
    for f in fs {
        unsafe {
            let mut row = vec![];
            let (hr, out) = create(f, None, &IID_IUNKNOWN);
            row.push((hr as u32, out.is_null()));
            let (hr, out) = create(f, Some("127.0.0.1"), &other);
            row.push((hr as u32, out.is_null()));
            for iid in [IID_XTF_CONSOLE_CONTROL_CLIENT, IID_IUNKNOWN, IID_XTF_CONSOLE_CONTROL_CLIENT_BASE] {
                for addr in ["127.0.0.1", ""] {
                    let (hr, out) = create(f, Some(addr), &iid);
                    row.push((hr as u32, out.is_null()));
                    if hr >= 0 {
                        let mut q = 0x77usize as *mut c_void;
                        let qhr = (vt(out).query_interface)(out as *mut ConsoleControlClient, &other, &mut q);
                        row.push((qhr as u32, q == 0x77usize as *mut c_void));
                        row.push(((vt(out).add_ref)(out as *mut ConsoleControlClient), false));
                        release(out);
                        row.push((release(out), false));
                    }
                }
            }
            seen.push(row);
        }
    }
    assert_eq!(seen[1], seen[0]);
    assert_eq!(seen[0][0], (E_INVALIDARG as u32, true));
}

#[test]
fn calls_that_fail_before_any_session_opens_match_console_dll() {
    let Some(fs) = factories() else { return };
    let mut seen = vec![];
    for f in fs {
        unsafe {
            let mut row: Vec<u32> = vec![];
            for addr in ["", "/title", "10.9.8.7/era", "10.9.8.7\\x/bogus"] {
                let (hr, c) = create(f, Some(addr), &IID_XTF_CONSOLE_CONTROL_CLIENT);
                assert_eq!(hr, S_OK);
                let cc = c as *mut ConsoleControlClient;
                let v = vt(c);
                let key = wide("hostname");
                let mut b: BSTR = null_mut();
                row.push((v.get_config_value)(cc, key.as_ptr(), &mut b) as u32);
                row.push(b.is_null() as u32);
                let val = wide("x");
                row.push((v.set_config_value)(cc, key.as_ptr(), val.as_ptr()) as u32);
                row.push((v.get_running_processes)(cc, 0x10usize as *mut c_void) as u32);
                row.push((v.shutdown_console)(cc, 2) as u32);
                let mut p = 3i32;
                row.push((v.updates_pending)(cc, &mut p) as u32);
                row.push(p as u32);
                let mut st = SystemTime { year: 5, ..Default::default() };
                row.push((v.get_system_time)(cc, &mut st) as u32);
                row.push((st == SystemTime::default()) as u32);
                row.push((v.get_config_settings)(cc, null_mut()) as u32);
                let img = wide("a");
                let mut path: BSTR = null_mut();
                row.push((v.generate_stack_dump)(cc, 4, 0, img.as_ptr(), &mut path) as u32);
                for key in ["AccessKey", "SESSIONKEY"] {
                    let k = wide(key);
                    for bad in ["not valid!", &"a".repeat(32), "-"] {
                        let b = wide(bad);
                        row.push((v.set_config_value)(cc, k.as_ptr(), b.as_ptr()) as u32);
                    }
                }
                row.push((v.updates_pending)(cc, null_mut()) as u32);
                row.push((v.get_system_time)(cc, null_mut()) as u32);
                let mut a = 0i32;
                row.push((v.query_connected_standby_ex)(cc, &mut a, null_mut(), &mut a) as u32);
                release(c);
            }
            seen.push(row);
        }
    }
    assert_eq!(seen[1], seen[0]);
    assert!(seen[0].contains(&(E_INVALID_VM_TARGET as u32)));
    assert!(seen[0].contains(&(E_NO_IPADDRESS_IN_CONNSTR as u32)));
}
