use crate::client::*;
use crate::config::sorted_list;
use crate::privilege;
use crate::process::{host_of_console, ERROR_INVALID_PARAMETER};
use crate::system::are_updates_pending;
use crate::tests::{dump_dir, guard};
use crate::*;
use core::ptr::null_mut;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use xtfmsg::com::{self, Obj, IID_IUNKNOWN, IID_XBTP_EXTENSION_METADATA};
use xtfmsg::sys::*;

const SYSTEM_TEST_PORT: u32 = 47931;
const TITLE_TEST_PORT: u32 = 47932;
const HOST_TEST_PORT: u32 = 47933;

#[repr(C)]
struct DispatcherVtbl {
    query_interface: usize,
    add_ref: usize,
    release: unsafe extern "system" fn(Obj) -> u32,
    shutdown: unsafe extern "system" fn(Obj) -> HRESULT,
    add_extension: unsafe extern "system" fn(Obj, *const GUID, Obj) -> HRESULT,
}

const IID_XBTP_DISPATCHER: GUID = GUID::new(0xe6358d7e, 0x3b86, 0x425b, [0xb3, 0xd7, 0x3d, 0x21, 0x82, 0xb9, 0x76, 0x6a]);

type StartDispatcherFn = unsafe extern "C" fn(u32, i32, *const GUID, *mut Obj) -> HRESULT;

struct Dispatcher(Obj);

impl Dispatcher {
    unsafe fn console(port: u32) -> Dispatcher {
        let f: StartDispatcherFn = core::mem::transmute(xtfmsg::xbtp::proc(b"XbtpStartDispatcherEx\0").unwrap());
        let mut out: Obj = null_mut();
        let hr = f(port, 0, &IID_XBTP_DISPATCHER, &mut out);
        assert_eq!(hr, S_OK, "XbtpStartDispatcherEx on port {port} failed: 0x{:08X}", hr as u32);
        let d = Dispatcher(out);
        let mut md: Obj = null_mut();
        assert_eq!(XtfOrdinal_4(&CONSOLE_EXTENSION_ID, &IID_XBTP_EXTENSION_METADATA, &mut md), S_OK);
        assert_eq!((d.vt().add_extension)(d.0, &CONSOLE_EXTENSION_ID, md), S_OK);
        com::release(md);
        d
    }

    unsafe fn bare(port: u32) -> Dispatcher {
        let f: StartDispatcherFn = core::mem::transmute(xtfmsg::xbtp::proc(b"XbtpStartDispatcherEx\0").unwrap());
        let mut out: Obj = null_mut();
        let hr = f(port, 0, &IID_XBTP_DISPATCHER, &mut out);
        assert_eq!(hr, S_OK, "XbtpStartDispatcherEx on port {port} failed: 0x{:08X}", hr as u32);
        Dispatcher(out)
    }

    unsafe fn add(&self, ext: &GUID, op: u32, f: xtfmsg::com::metadata::OperationFn) {
        let mut md: Obj = null_mut();
        assert_eq!(xtfmsg::xbtp::create_extension_metadata(&IID_XBTP_EXTENSION_METADATA, &mut md), S_OK);
        assert_eq!(xtfmsg::com::metadata::add_function(md, op, Some(f)), S_OK);
        assert_eq!((self.vt().add_extension)(self.0, ext, md), S_OK);
        com::release(md);
    }

    unsafe fn vt(&self) -> &DispatcherVtbl {
        &**(self.0 as *const *const DispatcherVtbl)
    }
}

static DARK_HITS: [AtomicU32; 3] = [AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0)];
static DARK_BAD: AtomicU32 = AtomicU32::new(0);
static REBOOT_V1_HITS: AtomicU32 = AtomicU32::new(0);

unsafe extern "system" fn go_dark_handler<const I: usize>(_session: usize, request: usize, _reply: usize) -> HRESULT {
    let mut body: Obj = null_mut();
    let mut v = 0u32;
    let name = wide("SysIsDark");
    if xtfmsg::msg::deserialize_body(request as Obj, &mut body) < 0 || xtfmsg::msg::get_json_u32(body, name.as_ptr(), &mut v) < 0 || v != 1 {
        DARK_BAD.fetch_add(1, Ordering::SeqCst);
    }
    if !body.is_null() {
        com::release(body);
    }
    DARK_HITS[I].fetch_add(1, Ordering::SeqCst);
    S_OK
}

unsafe extern "system" fn reboot_v1_handler(_session: usize, _request: usize, _reply: usize) -> HRESULT {
    REBOOT_V1_HITS.fetch_add(1, Ordering::SeqCst);
    S_OK
}

fn dark_hits() -> [u32; 3] {
    [0, 1, 2].map(|i| DARK_HITS[i].swap(0, Ordering::SeqCst))
}

impl Drop for Dispatcher {
    fn drop(&mut self) {
        unsafe {
            (self.vt().shutdown)(self.0);
            (self.vt().release)(self.0);
        }
    }
}

struct Ports;

impl Ports {
    fn map() -> Ports {
        *TEST_PORTS.lock().unwrap() = Some([SYSTEM_TEST_PORT, TITLE_TEST_PORT, 0, HOST_TEST_PORT]);
        Ports
    }
}

impl Drop for Ports {
    fn drop(&mut self) {
        *TEST_PORTS.lock().unwrap() = None;
    }
}

struct Loopback {
    _system: Dispatcher,
    _title: Dispatcher,
    _ports: Ports,
}

unsafe fn loopback() -> Loopback {
    let ports = Ports::map();
    Loopback { _system: Dispatcher::console(SYSTEM_TEST_PORT), _title: Dispatcher::console(TITLE_TEST_PORT), _ports: ports }
}

struct Client(*mut ConsoleControlClient);

impl Client {
    unsafe fn new(address: &str) -> Client {
        let a = wide(address);
        let mut out: Obj = null_mut();
        assert_eq!(XtfCreateConsoleControlClient(a.as_ptr(), &IID_XTF_CONSOLE_CONTROL_CLIENT, &mut out), S_OK);
        Client(out as *mut ConsoleControlClient)
    }

    unsafe fn vt(&self) -> &ConsoleControlClientVtbl {
        &*(*self.0).vtbl
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        unsafe {
            (self.vt().release)(self.0);
        }
    }
}

unsafe fn take(b: BSTR) -> Option<String> {
    if b.is_null() {
        return None;
    }
    let s = from_wide(b);
    SysFreeString(b);
    Some(s)
}

#[repr(C)]
struct Collector {
    vtbl: *const CollectorVtbl,
    calls: AtomicU32,
    stop_after: u32,
    reply: HRESULT,
    seen: Mutex<Vec<(String, String)>>,
}

unsafe extern "system" fn col_qi(_o: Obj, _iid: *const GUID, _out: *mut Obj) -> HRESULT {
    E_NOINTERFACE
}

unsafe extern "system" fn col_ref(_o: Obj) -> u32 {
    1
}

unsafe fn wstr(p: *const u16) -> String {
    if p.is_null() {
        "<null>".to_string()
    } else {
        from_wide(p)
    }
}

unsafe extern "system" fn col_setting(o: Obj, s: *const ConfigSetting) -> HRESULT {
    let c = &*(o as *const Collector);
    let n = c.calls.fetch_add(1, Ordering::SeqCst) + 1;
    c.seen.lock().unwrap().push((wstr((*s).name), wstr((*s).value)));
    if n >= c.stop_after {
        c.reply
    } else {
        S_OK
    }
}

unsafe extern "system" fn col_process(o: Obj, p: *const ProcessInfo) -> HRESULT {
    let c = &*(o as *const Collector);
    let n = c.calls.fetch_add(1, Ordering::SeqCst) + 1;
    c.seen.lock().unwrap().push(((*p).process_id.to_string(), wstr((*p).name)));
    if n >= c.stop_after {
        c.reply
    } else {
        S_OK
    }
}

static SETTING_VTBL: CollectorVtbl = CollectorVtbl { qi: col_qi, add_ref: col_ref, release: col_ref, on_item: col_setting as *const () };
static PROCESS_VTBL: CollectorVtbl = CollectorVtbl { qi: col_qi, add_ref: col_ref, release: col_ref, on_item: col_process as *const () };

#[repr(C)]
struct CollectorVtbl {
    qi: unsafe extern "system" fn(Obj, *const GUID, *mut Obj) -> HRESULT,
    add_ref: unsafe extern "system" fn(Obj) -> u32,
    release: unsafe extern "system" fn(Obj) -> u32,
    on_item: *const (),
}

unsafe impl Sync for CollectorVtbl {}

fn collector(vtbl: &'static CollectorVtbl, stop_after: u32, reply: HRESULT) -> Box<Collector> {
    Box::new(Collector { vtbl, calls: AtomicU32::new(0), stop_after, reply, seen: Mutex::new(vec![]) })
}

#[test]
fn factory_checks_arguments_and_answers_three_interfaces() {
    let _g = guard();
    unsafe {
        let a = wide("127.0.0.1");
        assert_eq!(XtfCreateConsoleControlClient(a.as_ptr(), &IID_IUNKNOWN, null_mut()), E_POINTER);
        let mut out = 0x1234usize as Obj;
        assert_eq!(XtfCreateConsoleControlClient(core::ptr::null(), &IID_IUNKNOWN, &mut out), E_INVALIDARG);
        assert!(out.is_null());
        let other = GUID::new(1, 2, 3, [4; 8]);
        let mut out = 0x1234usize as Obj;
        assert_eq!(XtfCreateConsoleControlClient(a.as_ptr(), &other, &mut out), E_NOINTERFACE);
        assert!(out.is_null());
        for iid in [IID_XTF_CONSOLE_CONTROL_CLIENT, IID_IUNKNOWN, IID_XTF_CONSOLE_CONTROL_CLIENT_BASE] {
            let mut out: Obj = null_mut();
            assert_eq!(XtfCreateConsoleControlClient(a.as_ptr(), &iid, &mut out), S_OK);
            let c = out as *mut ConsoleControlClient;
            assert_eq!(from_wide((*c).address), "127.0.0.1");
            let mut again: Obj = null_mut();
            assert_eq!(com::query_interface(out, &IID_XTF_CONSOLE_CONTROL_CLIENT_BASE, &mut again), S_OK);
            assert_eq!(again, out);
            let mut none = 0x55usize as Obj;
            assert_eq!(com::query_interface(out, &other, &mut none), E_NOINTERFACE);
            assert_eq!(none, 0x55usize as Obj);
            assert_eq!(com::add_ref(out), 3);
            assert_eq!(com::release(out), 2);
            assert_eq!(com::release(again), 1);
            assert_eq!(com::release(out), 0);
        }
        let empty = wide("");
        let mut out: Obj = null_mut();
        assert_eq!(XtfCreateConsoleControlClient(empty.as_ptr(), &IID_IUNKNOWN, &mut out), S_OK);
        assert!((*(out as *mut ConsoleControlClient)).address.is_null());
        let mut v: BSTR = null_mut();
        assert_eq!(com::release(out), 0);
        let c = Client::new("");
        let k = wide("hostname");
        assert_eq!((c.vt().get_config_value)(c.0, k.as_ptr(), &mut v), E_INVALIDARG);
        assert_eq!(CLIENT_SLOTS, 14);
    }
}

#[test]
fn split_console_address_parses_targets_and_session_keys() {
    let _g = guard();
    unsafe {
        let run = |s: &str, def: u32, with_key: bool| -> (HRESULT, Option<String>, Option<String>, u32) {
            let w = wide(s);
            let mut ip: BSTR = null_mut();
            let mut key: BSTR = null_mut();
            let mut port = 0xffu32;
            let hr = split_console_address(w.as_ptr(), def, &mut ip, if with_key { &mut key } else { null_mut() }, &mut port);
            (hr, take(ip), take(key), port)
        };
        assert_eq!(run("10.0.0.5", 1, true), (S_OK, Some("10.0.0.5".into()), None, 1));
        assert_eq!(run("10.0.0.5/title", 0, true), (S_OK, Some("10.0.0.5".into()), None, 1));
        assert_eq!(run("10.0.0.5\\HOST", 0, false), (S_OK, Some("10.0.0.5\\HOST".into()), None, 0));
        assert_eq!(run("10.0.0.5/HOST", 0, false), (S_OK, Some("10.0.0.5".into()), None, 3));
        assert_eq!(run("10.0.0.5/System", 1, false).3, 0);
        assert_eq!(run("10.0.0.5/era", 0, true).0, E_INVALID_VM_TARGET);
        assert_eq!(run("/title", 0, true).0, E_NO_IPADDRESS_IN_CONNSTR);
        assert_eq!(run("10.0.0.5+abc12/title", 0, true), (S_OK, Some("10.0.0.5".into()), Some("abc12".into()), 1));
        assert_eq!(run("10.0.0.5+abc12", 0, false), (S_OK, Some("10.0.0.5+abc12".into()), None, 0));
        let long = "a".repeat(MAX_ADDRESS_CHARS);
        assert_eq!(run(&long, 0, true).0, E_NO_IPADDRESS_IN_CONNSTR);
    }
}

#[test]
fn access_key_values_are_checked_before_any_session_opens() {
    let _g = guard();
    unsafe {
        let _p = Ports::map();
        let c = Client::new("127.0.0.1");
        for key in ["AccessKey", "sessionkey"] {
            let k = wide(key);
            let bad = wide("not valid!");
            assert_eq!((c.vt().set_config_value)(c.0, k.as_ptr(), bad.as_ptr()), E_INVALIDARG);
            let long = wide(&"a".repeat(MAX_ACCESS_KEY_CHARS + 1));
            assert_eq!((c.vt().set_config_value)(c.0, k.as_ptr(), long.as_ptr()), E_INVALIDARG);
        }
        assert_eq!(verify_session_key(core::ptr::null()), S_OK);
        let ok = wide(&"Z9".repeat(15));
        assert_eq!(verify_session_key(ok.as_ptr()), S_OK);
        let k = wide("AccessKey");
        let good = wide("abc123");
        assert!((c.vt().set_config_value)(c.0, k.as_ptr(), good.as_ptr()) < 0);
    }
}

#[test]
fn config_calls_round_trip_through_loopback_console_server() {
    let _g = guard();
    unsafe {
        let _l = loopback();
        let c = Client::new("127.0.0.1");
        let k = wide("hostname");
        let mut v: BSTR = 0x1usize as BSTR;
        assert_eq!((c.vt().get_config_value)(c.0, k.as_ptr(), &mut v), S_OK);
        assert!(!v.is_null());
        assert_eq!(take(v).as_deref(), Some(""));
        let val = wide("devkit");
        assert_eq!((c.vt().set_config_value)(c.0, k.as_ptr(), val.as_ptr()), S_OK);
        let ak = wide("AccessKey");
        let good = wide("abc123");
        assert_eq!((c.vt().set_config_value)(c.0, ak.as_ptr(), good.as_ptr()), S_OK);
        let empty = wide("");
        let mut v: BSTR = null_mut();
        assert_eq!((c.vt().get_config_value)(c.0, empty.as_ptr(), &mut v), S_OK);
        take(v);
        let mut list: *mut ConfigSetting = null_mut();
        let mut n = 0u32;
        assert_eq!((c.vt().get_all_settings)(c.0, &mut list, &mut n), S_OK);
        let want: Vec<&str> = sorted_list().iter().map(|s| s.name).collect();
        assert_eq!(n as usize, want.len());
        for (i, name) in want.iter().enumerate() {
            let e = &*list.add(i);
            assert_eq!(take(e.name).as_deref(), Some(*name));
            assert_eq!(take(e.value).unwrap_or_default(), "");
        }
        CoTaskMemFreeTest(list as *mut core::ffi::c_void);
        let all = collector(&SETTING_VTBL, u32::MAX, S_OK);
        assert_eq!((c.vt().get_config_settings)(c.0, &*all as *const Collector as Obj), S_OK);
        let seen: Vec<String> = all.seen.lock().unwrap().iter().map(|p| p.0.clone()).collect();
        assert_eq!(seen, want);
        let stop = collector(&SETTING_VTBL, 3, 1);
        assert_eq!((c.vt().get_config_settings)(c.0, &*stop as *const Collector as Obj), 1);
        assert_eq!(stop.calls.load(Ordering::SeqCst), 3);
        assert_eq!((c.vt().get_config_settings)(c.0, null_mut()), E_INVALIDARG);
    }
}

#[link(name = "ole32")]
extern "system" {
    #[link_name = "CoTaskMemFree"]
    fn CoTaskMemFreeTest(p: *mut core::ffi::c_void);
}

#[test]
fn status_calls_read_console_state_over_loopback() {
    let _g = guard();
    unsafe {
        let _l = loopback();
        let c = Client::new("127.0.0.1");
        let mut pending = 7i32;
        assert_eq!((c.vt().updates_pending)(c.0, &mut pending), S_OK);
        assert_eq!(pending, are_updates_pending() as i32);
        assert_eq!((c.vt().updates_pending)(c.0, null_mut()), E_INVALIDARG);
        let mut st = SystemTime { year: 1, ..Default::default() };
        assert_eq!((c.vt().get_system_time)(c.0, &mut st), S_OK);
        let (y, m, d, _, _) = crate::system::system_time_fields();
        assert!((st.year, st.month, st.day) == (y, m, d) || st.day != d, "{st:?}");
        assert!(st.year >= 2024 && (1..=12).contains(&st.month), "{st:?}");
        assert_eq!((c.vt().get_system_time)(c.0, null_mut()), E_INVALIDARG);
        let (mut a, mut b, mut x) = (5i32, 5i32, 5i32);
        assert_eq!((c.vt().query_connected_standby_ex)(c.0, &mut a, &mut b, &mut x), S_OK);
        assert_eq!((a, b, x), (0, 0, 0));
        let (mut a, mut b) = (5i32, 5i32);
        assert_eq!((c.vt().query_connected_standby)(c.0, &mut a, &mut b), S_OK);
        assert_eq!((a, b), (0, 0));
        assert_eq!((c.vt().query_connected_standby_ex)(c.0, &mut a, null_mut(), &mut b), E_POINTER);
        let all = collector(&PROCESS_VTBL, u32::MAX, S_OK);
        assert_eq!((c.vt().get_running_processes)(c.0, &*all as *const Collector as Obj), S_OK);
        let seen = all.seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].0, std::process::id().to_string());
        let exe = std::env::current_exe().unwrap();
        match crate::process::console_path_of_host(&exe) {
            Some(p) => assert_eq!(seen[0].1, p),
            None => assert_eq!(seen[0].1, "<unknown>"),
        }
        let neg = collector(&PROCESS_VTBL, 1, E_FAIL);
        assert_eq!((c.vt().get_running_processes)(c.0, &*neg as *const Collector as Obj), E_FAIL);
        let pos = collector(&PROCESS_VTBL, 1, 1);
        assert_eq!((c.vt().get_running_processes)(c.0, &*pos as *const Collector as Obj), 1);
    }
}

#[test]
fn generate_stack_dump_goes_to_the_title_server_and_returns_its_path() {
    let _g = guard();
    unsafe {
        let _l = loopback();
        let c = Client::new("127.0.0.1");
        let image = wide("xtfccclient");
        let mut path: BSTR = null_mut();
        assert_eq!((c.vt().generate_stack_dump)(c.0, std::process::id(), 1, image.as_ptr(), &mut path), S_OK);
        let path = take(path).unwrap();
        assert!(path.starts_with("D:\\xtfccclient_") && path.ends_with("-mini.dmp"), "{path}");
        let host = host_of_console(&path).unwrap();
        assert!(host.starts_with(dump_dir()));
        assert_eq!(&std::fs::read(&host).unwrap()[..4], b"MDMP");
        std::fs::remove_file(&host).unwrap();
        let other = wide("xtfccother");
        let mut path: BSTR = null_mut();
        assert_eq!((c.vt().generate_stack_dump)(c.0, std::process::id().wrapping_add(4), 0, other.as_ptr(), &mut path), hr_from_win32(ERROR_INVALID_PARAMETER));
        assert!(path.is_null());
        for e in std::fs::read_dir(dump_dir()).unwrap().filter_map(|e| e.ok()) {
            if e.file_name().to_string_lossy().starts_with("xtfccother_") {
                assert_eq!(std::fs::metadata(e.path()).unwrap().len(), 0);
                std::fs::remove_file(e.path()).unwrap();
            }
        }
        let sys_only = Client::new("127.0.0.1/system");
        let mut path: BSTR = null_mut();
        assert_eq!((sys_only.vt().generate_stack_dump)(sys_only.0, std::process::id(), 1, image.as_ptr(), &mut path), S_OK);
        let p = take(path).unwrap();
        std::fs::remove_file(host_of_console(&p).unwrap()).unwrap();
    }
}

#[test]
fn shutdown_reaches_the_server_handler_without_touching_the_pc() {
    let _g = guard();
    unsafe {
        let _l = loopback();
        for flags in [0u32, 1, 2, 3] {
            assert_eq!(privilege::set_privilege(privilege::SE_SHUTDOWN_NAME, false, None), S_OK);
            assert!(!privilege::is_enabled(privilege::SE_SHUTDOWN_NAME));
            let c = Client::new("127.0.0.1");
            assert_eq!((c.vt().shutdown_console)(c.0, flags), S_OK);
            let t = Instant::now();
            while !privilege::is_enabled(privilege::SE_SHUTDOWN_NAME) && t.elapsed() < Duration::from_secs(10) {
                std::thread::sleep(Duration::from_millis(20));
            }
            assert!(privilege::is_enabled(privilege::SE_SHUTDOWN_NAME), "flags {flags}");
        }
        assert_eq!(privilege::set_privilege(privilege::SE_SHUTDOWN_NAME, false, None), S_OK);
        let c = Client::new("127.0.0.1/era");
        assert_eq!((c.vt().shutdown_console)(c.0, 0), E_INVALID_VM_TARGET);
    }
}

#[test]
fn shutdown_without_a_system_server_falls_back_to_the_host_provision_session() {
    let _g = guard();
    unsafe {
        let _p = Ports::map();
        let c = Client::new("127.0.0.1");
        let hr = (c.vt().shutdown_console)(c.0, 0);
        assert!(hr < 0, "0x{:08X}", hr as u32);
        let h = Client::new("127.0.0.1/host");
        let hr2 = (h.vt().shutdown_console)(h.0, 0);
        assert_eq!(hr2, hr);
        assert!(!privilege::is_enabled(privilege::SE_SHUTDOWN_NAME));
    }
}

#[test]
fn calls_without_a_listening_server_return_the_open_failure() {
    let _g = guard();
    unsafe {
        let _p = Ports::map();
        let c = Client::new("127.0.0.1");
        let k = wide("hostname");
        let mut v: BSTR = null_mut();
        let hr = (c.vt().get_config_value)(c.0, k.as_ptr(), &mut v);
        assert!(hr < 0, "0x{:08X}", hr as u32);
        assert!(v.is_null());
        let mut pending = 9i32;
        assert_eq!((c.vt().updates_pending)(c.0, &mut pending), hr);
        assert_eq!(pending, 0);
        let mut st = SystemTime { year: 9, ..Default::default() };
        assert_eq!((c.vt().get_system_time)(c.0, &mut st), hr);
        assert_eq!(st, SystemTime::default());
    }
}

#[test]
fn remote_consoles_fail_offline_like_the_console() {
    let _g = guard();
    unsafe {
        let c = Client::new("10.1.2.3");
        let k = wide("hostname");
        let mut v: BSTR = null_mut();
        assert_eq!((c.vt().get_config_value)(c.0, k.as_ptr(), &mut v), E_COULD_NOT_CONNECT);
        let t = Client::new("10.1.2.3/title");
        let image = wide("x");
        let mut path: BSTR = null_mut();
        assert_eq!((t.vt().generate_stack_dump)(t.0, 4, 0, image.as_ptr(), &mut path), E_COULD_NOT_CONNECT);
        let n = Client::new("devkit.invalid");
        let mut v: BSTR = null_mut();
        assert_eq!((n.vt().get_config_value)(n.0, k.as_ptr(), &mut v), E_COULD_NOT_CONNECT);
    }
}

#[test]
fn shutdown_sends_go_dark_to_host_system_and_title_unless_power_mode_aware() {
    let _g = guard();
    unsafe {
        let _p = Ports::map();
        let system = Dispatcher::console(SYSTEM_TEST_PORT);
        let title = Dispatcher::console(TITLE_TEST_PORT);
        let host = Dispatcher::console(HOST_TEST_PORT);
        system.add(&DIAGNOSTIC_INFORMATION_EXTENSION_ID, OP_SET_SYSTEM_GO_DARK, go_dark_handler::<0>);
        title.add(&DIAGNOSTIC_INFORMATION_EXTENSION_ID, OP_SET_SYSTEM_GO_DARK, go_dark_handler::<1>);
        host.add(&DIAGNOSTIC_INFORMATION_EXTENSION_ID, OP_SET_SYSTEM_GO_DARK, go_dark_handler::<2>);
        dark_hits();
        DARK_BAD.store(0, Ordering::SeqCst);
        for (address, flags, want) in [("127.0.0.1", 0u32, [1u32, 1, 1]), ("127.0.0.1", 1, [1, 1, 1]), ("127.0.0.1", 2, [0, 0, 0]), ("127.0.0.1", 3, [0, 0, 0]), ("127.0.0.1/title", 0, [1, 1, 1]), ("127.0.0.1/host", 0, [0, 0, 0])] {
            assert_eq!(privilege::set_privilege(privilege::SE_SHUTDOWN_NAME, false, None), S_OK);
            let c = Client::new(address);
            assert_eq!((c.vt().shutdown_console)(c.0, flags), S_OK, "{address} {flags}");
            let t = Instant::now();
            while !privilege::is_enabled(privilege::SE_SHUTDOWN_NAME) && t.elapsed() < Duration::from_secs(10) {
                std::thread::sleep(Duration::from_millis(20));
            }
            assert!(privilege::is_enabled(privilege::SE_SHUTDOWN_NAME), "{address} {flags}");
            assert_eq!(dark_hits(), want, "{address} {flags}");
        }
        assert_eq!(DARK_BAD.load(Ordering::SeqCst), 0);
        assert_eq!(privilege::set_privilege(privilege::SE_SHUTDOWN_NAME, false, None), S_OK);
        drop((system, title, host));
    }
}

#[test]
fn shutdown_falls_back_to_reboot_v1_on_the_host_provision_extension() {
    let _g = guard();
    unsafe {
        let _p = Ports::map();
        let host = Dispatcher::bare(HOST_TEST_PORT);
        host.add(&XTF_PROVISION_EXTENSION_ID, OP_REBOOT_V1, reboot_v1_handler);
        REBOOT_V1_HITS.store(0, Ordering::SeqCst);
        assert_eq!(privilege::set_privilege(privilege::SE_SHUTDOWN_NAME, false, None), S_OK);
        let c = Client::new("127.0.0.1");
        assert_eq!((c.vt().shutdown_console)(c.0, 0), S_OK);
        assert_eq!(REBOOT_V1_HITS.load(Ordering::SeqCst), 1);
        let t = Client::new("127.0.0.1/title");
        assert_eq!((t.vt().shutdown_console)(t.0, 2), S_OK);
        assert_eq!(REBOOT_V1_HITS.load(Ordering::SeqCst), 2);
        let h = Client::new("127.0.0.1/host");
        assert!((h.vt().shutdown_console)(h.0, 0) < 0);
        assert_eq!(REBOOT_V1_HITS.load(Ordering::SeqCst), 2);
        assert!(!privilege::is_enabled(privilege::SE_SHUTDOWN_NAME));
    }
}