use crate::client::*;
use crate::console::*;
use crate::launch::*;
use crate::server::*;
use crate::tests::{child_cmd, guard, Build, Jobs, TEST_LINK_PORT};
use crate::*;
use core::ptr::null_mut;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use xtfmsg::com::{self, Obj, IID_XBTP_EXTENSION_METADATA};
use xtfmsg::sys::*;

#[repr(C)]
struct DispatcherVtbl {
    query_interface: usize,
    add_ref: usize,
    release: unsafe extern "system" fn(Obj) -> u32,
    shutdown: unsafe extern "system" fn(Obj) -> HRESULT,
    add_extension: unsafe extern "system" fn(Obj, *const GUID, Obj) -> HRESULT,
    register_extension: usize,
    register_redirect: usize,
    set_session_key: unsafe extern "system" fn(Obj, *const u16) -> HRESULT,
}

const IID_XBTP_DISPATCHER: GUID = GUID::new(0xe6358d7e, 0x3b86, 0x425b, [0xb3, 0xd7, 0x3d, 0x21, 0x82, 0xb9, 0x76, 0x6a]);

type StartDispatcherFn = unsafe extern "C" fn(u32, i32, *const GUID, *mut Obj) -> HRESULT;

struct Dispatcher(Obj);

impl Dispatcher {
    unsafe fn start() -> Dispatcher {
        let f: StartDispatcherFn = core::mem::transmute(xtfmsg::xbtp::proc(b"XbtpStartDispatcherEx\0").unwrap());
        let mut out: Obj = null_mut();
        let hr = f(TEST_LINK_PORT, 0, &IID_XBTP_DISPATCHER, &mut out);
        assert_eq!(hr, S_OK, "XbtpStartDispatcherEx on port {TEST_LINK_PORT} failed: 0x{:08X}", hr as u32);
        Dispatcher(out)
    }

    unsafe fn vt(&self) -> &DispatcherVtbl {
        &**(self.0 as *const *const DispatcherVtbl)
    }

    unsafe fn with_remote_run() -> Dispatcher {
        let d = Dispatcher::start();
        let mut md: Obj = null_mut();
        assert_eq!(XtfOrdinal_4(&REMOTE_RUN_EXTENSION_ID, &IID_XBTP_EXTENSION_METADATA, &mut md), S_OK);
        assert_eq!((d.vt().add_extension)(d.0, &REMOTE_RUN_EXTENSION_ID, md), S_OK);
        com::release(md);
        d
    }

    unsafe fn with_remote_run_keyed(key: &str) -> Dispatcher {
        let d = Dispatcher::with_remote_run();
        let k = wide(key);
        assert_eq!((d.vt().set_session_key)(d.0, k.as_ptr()), S_OK);
        d
    }

    unsafe fn with_handler(f: xtfmsg::com::metadata::OperationFn) -> Dispatcher {
        let d = Dispatcher::start();
        let mut md: Obj = null_mut();
        assert_eq!(xtfmsg::xbtp::create_extension_metadata(&IID_XBTP_EXTENSION_METADATA, &mut md), S_OK);
        assert_eq!(xtfmsg::com::metadata::add_function(md, RUN_OPERATION_ID, Some(f)), S_OK);
        assert_eq!((d.vt().add_extension)(d.0, &REMOTE_RUN_EXTENSION_ID, md), S_OK);
        com::release(md);
        d
    }
}

impl Drop for Dispatcher {
    fn drop(&mut self) {
        unsafe {
            (self.vt().shutdown)(self.0);
            (self.vt().release)(self.0);
        }
    }
}

#[repr(C)]
struct Callback {
    vtbl: *const RemoteRunCallbackExVtbl,
    refs: AtomicU32,
    ex: bool,
    inputs: Mutex<Vec<String>>,
    output: Mutex<String>,
    error: Mutex<String>,
    input_calls: AtomicU32,
}

unsafe extern "system" fn cb_qi(o: Obj, iid: *const GUID, out: *mut Obj) -> HRESULT {
    let c = &*(o as *mut Callback);
    if *iid == IID_XTF_REMOTE_RUN_CALLBACK_EX && c.ex {
        *out = o;
        cb_add_ref(o);
        return S_OK;
    }
    *out = null_mut();
    E_NOINTERFACE
}

unsafe extern "system" fn cb_add_ref(o: Obj) -> u32 {
    (*(o as *mut Callback)).refs.fetch_add(1, Ordering::SeqCst) + 1
}

unsafe extern "system" fn cb_release(o: Obj) -> u32 {
    (*(o as *mut Callback)).refs.fetch_sub(1, Ordering::SeqCst) - 1
}

unsafe extern "system" fn cb_input(o: Obj, out: *mut BSTR) -> HRESULT {
    let c = &*(o as *mut Callback);
    c.input_calls.fetch_add(1, Ordering::SeqCst);
    let mut q = c.inputs.lock().unwrap();
    *out = if q.is_empty() {
        null_mut()
    } else {
        let s = q.remove(0);
        let w = wide(&s);
        SysAllocString(w.as_ptr())
    };
    S_OK
}

unsafe extern "system" fn cb_output(o: Obj, text: *const u16) -> HRESULT {
    (*(o as *mut Callback)).output.lock().unwrap().push_str(&from_wide(text));
    S_OK
}

unsafe extern "system" fn cb_error(o: Obj, text: *const u16) -> HRESULT {
    (*(o as *mut Callback)).error.lock().unwrap().push_str(&from_wide(text));
    S_OK
}

static CB_VTBL: RemoteRunCallbackExVtbl = RemoteRunCallbackExVtbl {
    base: RemoteRunCallbackVtbl { query_interface: cb_qi, add_ref: cb_add_ref, release: cb_release, input: cb_input, output: cb_output },
    error: cb_error,
};

impl Callback {
    fn new(ex: bool, inputs: &[&str]) -> Box<Callback> {
        Box::new(Callback {
            vtbl: &CB_VTBL,
            refs: AtomicU32::new(1),
            ex,
            inputs: Mutex::new(inputs.iter().map(|s| s.to_string()).collect()),
            output: Mutex::new(String::new()),
            error: Mutex::new(String::new()),
            input_calls: AtomicU32::new(0),
        })
    }
    fn obj(&self) -> Obj {
        self as *const Callback as Obj
    }
}

struct Client(Obj);

impl Client {
    unsafe fn create(address: Option<&str>) -> (HRESULT, Client) {
        let a = address.map(wide);
        let mut out: Obj = null_mut();
        let hr = XtfCreateRemoteRunClient(a.as_ref().map_or(core::ptr::null(), |v| v.as_ptr()), &IID_XTF_REMOTE_RUN_CLIENT2, &mut out);
        (hr, Client(out))
    }
    unsafe fn run_ex(&self, cmd: Option<&str>, wd: Option<&str>, flags: u32, period: u32, cb: Obj, exit: Option<&mut u32>) -> HRESULT {
        let c = cmd.map(wide);
        let d = wd.map(wide);
        let vt = &**(self.0 as *const *const RemoteRunClient2Vtbl);
        (vt.run_ex)(self.0, c.as_ref().map_or(core::ptr::null(), |v| v.as_ptr()), d.as_ref().map_or(core::ptr::null(), |v| v.as_ptr()), flags, period, cb, exit.map_or(null_mut(), |e| e as *mut u32))
    }
    unsafe fn run(&self, cmd: &str, flags: u32, period: u32, cb: Obj) -> HRESULT {
        let mut base: Obj = null_mut();
        assert_eq!(com::query_interface(self.0, &IID_XTF_REMOTE_RUN_CLIENT, &mut base), S_OK);
        let vt = &**(base as *const *const RemoteRunClientVtbl);
        let c = wide(cmd);
        let hr = (vt.run)(base as *mut RemoteRunClient, c.as_ptr(), core::ptr::null(), flags, period, cb);
        com::release(base);
        hr
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { com::release(self.0) };
        }
    }
}

fn on_worker<T: Send + 'static>(secs: u64, f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(Duration::from_secs(secs)).expect("client call did not return in time")
}

#[test]
fn factory_and_query_interface_follow_console_rules() {
    let _g = guard();
    unsafe {
        let (hr, c) = Client::create(None);
        assert_eq!(hr, S_OK);
        assert!(!c.0.is_null());
        let this = RemoteRunClient::from_ex(c.0);
        assert!((*this).address.is_null());
        for iid in [IID_XTF_REMOTE_RUN_CLIENT, IID_IUNKNOWN, IID_XTF_REMOTE_RUN_CLIENT_BASE] {
            let mut o: Obj = null_mut();
            assert_eq!(com::query_interface(c.0, &iid, &mut o), S_OK);
            assert_eq!(o, this as Obj);
            com::release(o);
        }
        let mut o: Obj = null_mut();
        assert_eq!(com::query_interface(this as Obj, &IID_XTF_REMOTE_RUN_CLIENT2, &mut o), S_OK);
        assert_eq!(o, c.0);
        com::release(o);
        let mut o: Obj = 0x1234 as Obj;
        assert_eq!(com::query_interface(c.0, &IID_XTF_REMOTE_RUN_CALLBACK_EX, &mut o), E_NOINTERFACE);
        assert_eq!(o, 0x1234 as Obj);
        let a = wide("");
        let mut out: Obj = 0x55 as Obj;
        assert_eq!(XtfCreateRemoteRunClient(a.as_ptr(), &IID_XTF_REMOTE_RUN_CALLBACK_EX, &mut out), E_NOINTERFACE);
        assert!(out.is_null());
        let (hr, c2) = Client::create(Some("192.168.0.5+key/title"));
        assert_eq!(hr, S_OK);
        assert_eq!(from_wide((*RemoteRunClient::from_ex(c2.0)).address), "192.168.0.5+key/title");
    }
}

#[test]
fn split_console_address_matches_console_rules() {
    let _g = guard();
    unsafe {
        let cases: [(&str, HRESULT, &str, Option<&str>, u32); 8] = [
            ("1.2.3.4", S_OK, "1.2.3.4", None, 0),
            ("1.2.3.4/title", S_OK, "1.2.3.4", None, 1),
            ("1.2.3.4/HOST", S_OK, "1.2.3.4", None, 3),
            ("1.2.3.4/System", S_OK, "1.2.3.4", None, 0),
            ("1.2.3.4+abc/title", S_OK, "1.2.3.4", Some("abc"), 1),
            ("1.2.3.4/bogus", E_INVALID_VM_TARGET, "", None, 0),
            ("/title", E_NO_IPADDRESS_IN_CONNSTR, "", None, 0),
            ("", E_NO_IPADDRESS_IN_CONNSTR, "", None, 0),
        ];
        for (conn, hr, ip, key, port) in cases {
            let c = wide(conn);
            let mut a: BSTR = null_mut();
            let mut k: BSTR = null_mut();
            let mut p = 0xffu32;
            assert_eq!(split_console_address(c.as_ptr(), 0, &mut a, &mut k, &mut p), hr, "{conn}");
            if hr == S_OK {
                assert_eq!(from_wide(a), ip);
                assert_eq!(if k.is_null() { None } else { Some(from_wide(k)) }.as_deref(), key);
                assert_eq!(p, port);
            }
            free_bstr(a);
            free_bstr(k);
        }
        let mut a: BSTR = null_mut();
        let mut p = 0u32;
        assert_eq!(split_console_address(core::ptr::null(), 0, &mut a, null_mut(), &mut p), E_INVALIDARG);
        let c = wide("1.2.3.4+abc/title");
        assert_eq!(split_console_address(c.as_ptr(), 0, &mut a, null_mut(), &mut p), S_OK);
        assert_eq!(from_wide(a), "1.2.3.4+abc");
        free_bstr(a);
    }
}

#[test]
fn run_ex_argument_and_address_errors_match_console() {
    let _g = guard();
    unsafe {
        let (_, c) = Client::create(Some("1.2.3.4/bogus"));
        let cb = Callback::new(true, &[]);
        let mut code = 77u32;
        assert_eq!(c.run_ex(None, None, 0, 0, cb.obj(), Some(&mut code)), E_INVALIDARG);
        assert_eq!(code, 77);
        assert_eq!(c.run_ex(Some(""), None, 0, 0, cb.obj(), Some(&mut code)), E_INVALIDARG);
        assert_eq!(c.run_ex(Some("x.exe"), None, RUN_FLAG_REDIRECT_OUTPUT, 0, null_mut(), Some(&mut code)), E_INVALIDARG);
        assert_eq!(code, 0);
        code = 9;
        assert_eq!(c.run_ex(Some("x.exe"), None, RUN_FLAG_REDIRECT_INPUT, 0, null_mut(), Some(&mut code)), E_INVALIDARG);
        assert_eq!(code, 0);
        assert_eq!(c.run_ex(Some("x.exe"), None, 0, 0, null_mut(), None), E_INVALID_VM_TARGET);
        let (_, c) = Client::create(Some("/title"));
        assert_eq!(c.run_ex(Some("x.exe"), None, 0, 0, null_mut(), None), E_NO_IPADDRESS_IN_CONNSTR);
        let (_, c) = Client::create(None);
        assert_eq!(c.run_ex(Some("x.exe"), None, 0, 0, null_mut(), None), E_INVALIDARG);
        assert_eq!(c.run("x.exe", 0, 0, null_mut()), E_INVALIDARG);
        assert_eq!(cb.input_calls.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn remote_console_address_fails_could_not_connect_without_socket() {
    let _g = guard();
    let r = on_worker(60, || unsafe {
        let (hr, c) = Client::create(Some("10.255.0.7"));
        assert_eq!(hr, S_OK);
        let t = Instant::now();
        let mut code = 5u32;
        let hr = c.run_ex(Some("x.exe"), None, 0, 0, null_mut(), Some(&mut code));
        (hr, t.elapsed(), code)
    });
    assert_eq!(r.0, E_COULD_NOT_CONNECT);
    assert!(r.1 >= Duration::from_secs(20) && r.1 < Duration::from_secs(40), "console open retry loop took {:?}", r.1);
    assert_eq!(r.2, 0);
}

#[test]
fn client_end_to_end_streams_separated_output_and_exit_code() {
    let _g = guard();
    let _d = unsafe { Dispatcher::with_remote_run() };
    let (hr, code, out, err) = on_worker(60, || unsafe {
        let (_, c) = Client::create(Some("127.0.0.1"));
        let cb = Callback::new(true, &[]);
        let mut code = 0u32;
        let hr = c.run_ex(Some(&child_cmd("child_streams_main")), None, RUN_FLAG_REDIRECT_OUTPUT, 10, cb.obj(), Some(&mut code));
        let out = cb.output.lock().unwrap().clone();
        let err = cb.error.lock().unwrap().clone();
        (hr, code, out, err)
    });
    assert_eq!(hr, 3);
    assert_eq!(code, 3);
    assert!(out.contains("child out") && !out.contains("child err"), "{out:?}");
    assert!(err.contains("child err"), "{err:?}");
}

#[test]
fn client_end_to_end_merges_stderr_for_plain_callback_and_feeds_input() {
    let _g = guard();
    let _d = unsafe { Dispatcher::with_remote_run() };
    let (hr, out) = on_worker(60, || unsafe {
        let (_, c) = Client::create(Some("127.0.0.1/system"));
        let cb = Callback::new(false, &[]);
        let hr = c.run(&child_cmd("child_streams_main"), RUN_FLAG_REDIRECT_OUTPUT, 10, cb.obj());
        let out = cb.output.lock().unwrap().clone();
        (hr, out)
    });
    assert_eq!(hr, 3);
    assert!(out.contains("child out") && out.contains("child err"), "{out:?}");
    let (hr, code, out, calls) = on_worker(60, || unsafe {
        let (_, c) = Client::create(Some("127.0.0.1"));
        let cb = Callback::new(true, &["", "pong\n"]);
        let mut code = 9u32;
        let hr = c.run_ex(Some(&child_cmd("child_echo_main")), Some("%SystemRoot%"), RUN_FLAG_REDIRECT_INPUT | RUN_FLAG_REDIRECT_OUTPUT, 10, cb.obj(), Some(&mut code));
        let out = cb.output.lock().unwrap().clone();
        (hr, code, out, cb.input_calls.load(Ordering::SeqCst))
    });
    assert_eq!(hr, 0);
    assert_eq!(code, 0);
    assert!(out.contains("got:pong"), "{out:?}");
    assert!(calls >= 2);
}

#[test]
fn client_end_to_end_reports_server_launch_failure() {
    let _g = guard();
    let _d = unsafe { Dispatcher::with_remote_run() };
    let none = crate::tests::NoTitleJobs::new();
    let (hr, code) = on_worker(60, || unsafe {
        let (_, c) = Client::create(Some("127.0.0.1"));
        let mut code = 4u32;
        let hr = c.run_ex(Some(&child_cmd("child_marker_main")), None, RUN_FLAG_WAIT, 0, null_mut(), Some(&mut code));
        (hr, code)
    });
    assert_eq!(hr as u32, 0xD000_0034);
    assert_eq!(code, 0);
    drop(none);
    let hr = on_worker(60, || unsafe {
        let (_, c) = Client::create(Some("127.0.0.1"));
        c.run_ex(Some("C:\\absent\\tool.exe"), None, RUN_FLAG_WAIT, 0, null_mut(), None)
    });
    assert_eq!(hr, 0x8007_0002u32 as i32);
    let hr = on_worker(60, || unsafe {
        let (_, c) = Client::create(Some("127.0.0.1"));
        c.run_ex(Some(&child_cmd("child_marker_main")), None, RUN_FLAG_WAIT, 0, null_mut(), None)
    });
    assert_eq!(hr, 5);
}

static SCRIPT: Mutex<Vec<String>> = Mutex::new(Vec::new());
static SEEN: Mutex<Vec<String>> = Mutex::new(Vec::new());

unsafe extern "system" fn scripted_handler(session: usize, received: usize, reply: usize) -> HRESULT {
    SEEN.lock().unwrap().push(xtfmsg::fixture::json_text(received as Obj).unwrap_or_default());
    let script = SCRIPT.lock().unwrap().clone();
    for step in script {
        if let Some(j) = step.strip_prefix("more:") {
            let m = xtfmsg::fixture::message_with_json(j);
            assert_eq!(com::session::send_more_message(session as Obj, m.get()), S_OK);
        } else if step == "wait" {
            let deadline = Instant::now() + Duration::from_secs(20);
            loop {
                let mut m: Obj = null_mut();
                let hr = com::session::receive_more_message(session as Obj, 0, &mut m);
                if hr >= 0 {
                    SEEN.lock().unwrap().push(xtfmsg::fixture::json_text(m).unwrap_or_default());
                    com::release(m);
                    break;
                }
                if Instant::now() > deadline {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        } else if let Some(j) = step.strip_prefix("end:") {
            let w = wide(j);
            assert_eq!(com::end_message::set_json_body_w(reply as Obj, w.as_ptr()), S_OK);
        }
    }
    S_OK
}

unsafe fn scripted(script: &[&str], flags: u32, ex: bool) -> (HRESULT, u32, Vec<String>, String, String) {
    *SCRIPT.lock().unwrap() = script.iter().map(|s| s.to_string()).collect();
    SEEN.lock().unwrap().clear();
    let d = Dispatcher::with_handler(scripted_handler);
    let (hr, code, out, err) = on_worker(60, move || {
        let (_, c) = Client::create(Some("127.0.0.1"));
        let cb = Callback::new(ex, &[]);
        let mut code = 8u32;
        let hr = c.run_ex(Some("tool.exe a"), Some("D:\\work"), flags, 0, cb.obj(), Some(&mut code));
        let out = cb.output.lock().unwrap().clone();
        let err = cb.error.lock().unwrap().clone();
        (hr, code, out, err)
    });
    drop(d);
    let seen = SEEN.lock().unwrap().clone();
    (hr, code, seen, out, err)
}

fn fields(json: &str) -> Vec<String> {
    let mut v: Vec<String> = json.trim_matches(|c| c == '{' || c == '}').split(',').map(str::to_string).collect();
    v.sort();
    v
}

#[test]
fn client_protocol_against_scripted_server() {
    let _g = guard();
    unsafe {
        let (hr, code, seen, _, _) = scripted(&["end:{\"ExitCode\":7}"], 0, true);
        assert_eq!((hr, code), (7, 7));
        assert_eq!(seen.len(), 1);
        assert_eq!(fields(&seen[0]), fields("{\"CommandLine\":\"tool.exe a\",\"Run_Error_Output\":1,\"WorkingDirectory\":\"D:\\\\work\",\"dwFlags\":0,\"dwPeriod\":0}"));
        let (hr, code, seen, _, _) = scripted(&["end:{\"ExitCode\":2}"], 0, false);
        assert_eq!((hr, code), (2, 2));
        assert_eq!(fields(&seen[0]), fields("{\"CommandLine\":\"tool.exe a\",\"WorkingDirectory\":\"D:\\\\work\",\"dwFlags\":0,\"dwPeriod\":0}"));
        let (hr, code, seen, _, _) = scripted(&["end:{}"], RUN_FLAG_REDIRECT_OUTPUT, true);
        assert_eq!((hr, code), (0, 0));
        assert_eq!(seen.len(), 1);
        assert_eq!(fields(&seen[0]), fields("{\"CommandLine\":\"tool.exe a\",\"Run_Error_Output\":1,\"WorkingDirectory\":\"D:\\\\work\",\"dwFlags\":1073741825,\"dwPeriod\":0}"));
        let (hr, code, _, _, _) = scripted(&["end:{\"XtfOperationError\":{\"ReturnCode\":-2147024891,\"Checkpoint\":2,\"Errors\":[\"e\"]},\"ExitCode\":3}"], 0, true);
        assert_eq!((hr, code), (0x8007_0005u32 as i32, 0));
        let (hr, _, _, _, _) = scripted(&["end:{\"XtfOperationError\":{\"Checkpoint\":2,\"Errors\":[]}}"], 0, true);
        assert_eq!(hr, E_JSON_NOT_FOUND);
        let (hr, _, seen, _, _) = scripted(&["more:{\"MsgId\":1,\"Run_Output\":\"x\"}", "wait", "end:{}"], 0, true);
        assert_eq!(hr, E_UNEXPECTED);
        assert_eq!(fields(seen.last().unwrap()), fields("{\"MsgId\":2,\"Result\":-2147418113}"));
        let (hr, _, seen, out, _) = scripted(&["more:{\"MsgId\":1,\"Run_Output\":\"ab\"}", "more:{\"MsgId\":9}", "more:{\"MsgId\":3,\"Run_Error_Output\":\"zz\"}", "wait", "end:{}"], RUN_FLAG_REDIRECT_OUTPUT, false);
        assert_eq!(hr, E_UNEXPECTED);
        assert_eq!(out, "ab");
        assert_eq!(fields(seen.last().unwrap()), fields("{\"MsgId\":2,\"Result\":-2147418113}"));
        let (hr, _, _, out, err) = scripted(&["more:{\"MsgId\":1,\"Run_Output\":\"ab\"}", "more:{\"MsgId\":3,\"Run_Error_Output\":\"zz\"}", "end:{\"ExitCode\":0}"], RUN_FLAG_REDIRECT_OUTPUT, true);
        assert_eq!((hr, out.as_str(), err.as_str()), (0, "ab", "zz"));
        let (hr, _, seen, _, _) = scripted(&["more:{\"Run_Output\":\"ab\"}", "wait", "end:{}"], RUN_FLAG_REDIRECT_OUTPUT, true);
        assert_eq!(hr, E_JSON_NOT_FOUND);
        assert_eq!(fields(seen.last().unwrap()), fields("{\"MsgId\":2,\"Result\":-2089484279}"));
    }
}

#[test]
fn operation_error_initialize_follows_console_helper() {
    let _g = guard();
    unsafe {
        let mut e = OperationError::default();
        assert_eq!(e.initialize(null_mut()), E_INVALIDARG);
        let m = xtfmsg::fixture::message_with_json("{\"ExitCode\":1}");
        assert_eq!(e.initialize(m.get()), S_OK);
        assert_eq!((e.return_code, e.checkpoint), (0, 0));
        let m = xtfmsg::fixture::message_with_json("{\"XtfOperationError\":{\"ReturnCode\":-5,\"Checkpoint\":4294967297,\"Errors\":[\"a\"]}}");
        assert_eq!(e.initialize(m.get()), S_OK);
        assert_eq!((e.return_code, e.checkpoint), (-5, 1));
        let m = xtfmsg::fixture::message_with_json("{\"XtfOperationError\":{\"ReturnCode\":-5,\"Checkpoint\":1}}");
        assert_eq!(OperationError::default().initialize(m.get()), E_JSON_NOT_FOUND);
        let m = xtfmsg::fixture::message_with_json("not json");
        assert!(OperationError::default().initialize(m.get()) < 0);
    }
}

#[test]
fn launcher_job_lookup_follows_console_names() {
    let _g = guard();
    unsafe {
        let (st, h) = nt_open_job_object(NT_TOOLS_JOB);
        assert_eq!((st, h.is_null()), (STATUS_OBJECT_NAME_NOT_FOUND, true));
        assert!(open_job_object_w(HOST_TOOLS_JOB).is_null());
        assert_eq!(xtfmsg::sys::GetLastError(), 2);
        let j = Jobs::new(&[NT_TOOLS_JOB, HOST_TOOLS_JOB]);
        let (st, h) = nt_open_job_object("\\??\\jobobjectxboxtools");
        assert_eq!(st, 0);
        assert!(!h.is_null() && h != j.jobs[0].get());
        crate::win::CloseHandle(h);
        assert!(open_job_object_w("global\\XtfHostControlledToolsJobObject").is_null());
        let h = open_job_object_w(HOST_TOOLS_JOB);
        assert!(!h.is_null());
        crate::win::CloseHandle(h);
    }
}


impl Client {
    unsafe fn create_base(address: Option<&str>) -> (HRESULT, Client) {
        let a = address.map(wide);
        let mut out: Obj = null_mut();
        let hr = XtfCreateRemoteRunClient(a.as_ref().map_or(core::ptr::null(), |v| v.as_ptr()), &IID_XTF_REMOTE_RUN_CLIENT, &mut out);
        (hr, Client(out))
    }
    unsafe fn run_wd(&self, cmd: Option<&str>, wd: Option<&str>, flags: u32, period: u32, cb: Obj) -> HRESULT {
        let c = cmd.map(wide);
        let d = wd.map(wide);
        let vt = &**(self.0 as *const *const RemoteRunClientVtbl);
        (vt.run)(self.0 as *mut RemoteRunClient, c.as_ref().map_or(core::ptr::null(), |v| v.as_ptr()), d.as_ref().map_or(core::ptr::null(), |v| v.as_ptr()), flags, period, cb)
    }
}

#[test]
fn client_19041_copy_has_no_client2_and_checks_arguments_like_console() {
    let _g = guard();
    let _b = Build::set("19041");
    unsafe {
        let mut out: Obj = 0x55usize as Obj;
        let a = wide("127.0.0.1");
        assert_eq!(XtfCreateRemoteRunClient(a.as_ptr(), &IID_XTF_REMOTE_RUN_CLIENT2, &mut out), E_NOINTERFACE);
        assert!(out.is_null());
        let (hr, c) = Client::create_base(Some("1.2.3.4/bogus"));
        assert_eq!(hr, S_OK);
        assert_eq!((*(c.0 as *mut RemoteRunClient)).copy, crate::variant::ConsoleCopy::Build19041);
        let mut p: Obj = 0x66usize as Obj;
        assert_eq!(com::query_interface(c.0, &IID_XTF_REMOTE_RUN_CLIENT2, &mut p), E_NOINTERFACE);
        assert_eq!(p, 0x66usize as Obj);
        for iid in [IID_IUNKNOWN, IID_XTF_REMOTE_RUN_CLIENT_BASE, IID_XTF_REMOTE_RUN_CLIENT] {
            assert_eq!(com::query_interface(c.0, &iid, &mut p), S_OK);
            assert_eq!(p, c.0);
            com::release(p);
        }
        let cb = Callback::new(true, &[]);
        assert_eq!(c.run_wd(None, None, 0, 0, cb.obj()), E_INVALIDARG);
        assert_eq!(c.run_wd(Some(""), None, 0, 0, cb.obj()), E_INVALIDARG);
        assert_eq!(c.run_wd(Some("x.exe"), None, RUN_FLAG_REDIRECT_OUTPUT, 0, null_mut()), E_INVALIDARG);
        assert_eq!(c.run_wd(Some("x.exe"), None, RUN_FLAG_REDIRECT_INPUT, 0, null_mut()), E_INVALIDARG);
        assert_eq!(c.run_wd(Some("x.exe"), None, 0, 0, null_mut()), E_INVALID_VM_TARGET);
        let (_, c) = Client::create_base(Some("/title"));
        assert_eq!(c.run_wd(Some("x.exe"), None, 0, 0, null_mut()), E_NO_IPADDRESS_IN_CONNSTR);
        let (_, c) = Client::create_base(None);
        assert_eq!(c.run_wd(Some("x.exe"), None, 0, 0, null_mut()), E_INVALIDARG);
    }
}

#[test]
fn client_19041_remote_address_fails_could_not_connect_without_socket() {
    let _g = guard();
    let _b = Build::set("19041");
    let r = on_worker(90, || unsafe {
        let (hr, c) = Client::create_base(Some("10.255.0.7"));
        assert_eq!(hr, S_OK);
        let t = Instant::now();
        let hr = c.run_wd(Some("x.exe"), None, 0, 0, null_mut());
        (hr, t.elapsed())
    });
    assert_eq!(r.0, E_COULD_NOT_CONNECT, "{:?}", r.1);
}

#[test]
fn client_19041_end_to_end_falls_back_to_link_and_returns_exit_code() {
    let _g = guard();
    let _b = Build::set("19041");
    let _d = unsafe { Dispatcher::with_remote_run() };
    let (hr, out, err) = on_worker(60, || unsafe {
        let (_, c) = Client::create_base(Some("127.0.0.1"));
        let cb = Callback::new(true, &[]);
        let hr = c.run_wd(Some(&child_cmd("child_streams_main")), None, RUN_FLAG_REDIRECT_OUTPUT, 10, cb.obj());
        let out = cb.output.lock().unwrap().clone();
        let err = cb.error.lock().unwrap().clone();
        (hr, out, err)
    });
    assert_eq!(hr, 3);
    assert!(out.contains("child out") && !out.contains("child err"), "{out:?}");
    assert!(err.contains("child err"), "{err:?}");
    let (hr, out, calls) = on_worker(60, || unsafe {
        let (_, c) = Client::create_base(Some("127.0.0.1/system"));
        let cb = Callback::new(false, &["", "pong\n"]);
        let hr = c.run_wd(Some(&child_cmd("child_echo_main")), Some("%SystemRoot%"), RUN_FLAG_REDIRECT_INPUT | RUN_FLAG_REDIRECT_OUTPUT, 10, cb.obj());
        let out = cb.output.lock().unwrap().clone();
        (hr, out, cb.input_calls.load(Ordering::SeqCst))
    });
    assert_eq!(hr, 0);
    assert!(out.contains("got:pong"), "{out:?}");
    assert!(calls >= 2);
    let hr = on_worker(60, || unsafe {
        let (_, c) = Client::create_base(Some("127.0.0.1"));
        c.run_wd(Some("G:\\unlisted.exe"), None, RUN_FLAG_WAIT, 0, null_mut())
    });
    assert_eq!(hr, 0x8007_0002u32 as i32);
}

unsafe fn scripted_19041(script: &[&str], flags: u32, ex: bool) -> (HRESULT, Vec<String>, String, String) {
    *SCRIPT.lock().unwrap() = script.iter().map(|s| s.to_string()).collect();
    SEEN.lock().unwrap().clear();
    let d = Dispatcher::with_handler(scripted_handler);
    let (hr, out, err) = on_worker(60, move || {
        let (_, c) = Client::create_base(Some("127.0.0.1"));
        let cb = Callback::new(ex, &[]);
        let hr = c.run_wd(Some("tool.exe a"), Some("D:\\work"), flags, 0, cb.obj());
        let out = cb.output.lock().unwrap().clone();
        let err = cb.error.lock().unwrap().clone();
        (hr, out, err)
    });
    drop(d);
    let seen = SEEN.lock().unwrap().clone();
    (hr, seen, out, err)
}

#[test]
fn client_19041_protocol_against_scripted_server() {
    let _g = guard();
    let _b = Build::set("19041");
    unsafe {
        let (hr, seen, _, _) = scripted_19041(&["end:{\"ExitCode\":7}"], 0, true);
        assert_eq!(hr, 7);
        assert_eq!(fields(&seen[0]), fields("{\"CommandLine\":\"tool.exe a\",\"Run_Error_Output\":1,\"WorkingDirectory\":\"D:\\\\work\",\"dwFlags\":0,\"dwPeriod\":0}"));
        let (hr, _, _, _) = scripted_19041(&["end:{}"], 0, false);
        assert_eq!(hr, 0);
        let (hr, _, _, _) = scripted_19041(&["end:{\"XtfOperationError\":{\"ReturnCode\":-2147024891,\"Checkpoint\":2,\"Errors\":[\"e\"]},\"ExitCode\":3}"], 0, true);
        assert_eq!(hr, 0x8007_0005u32 as i32);
        let (hr, seen, _, _) = scripted_19041(&["more:{\"MsgId\":1,\"Run_Output\":\"x\"}", "wait", "end:{}"], 0, true);
        assert_eq!(hr, E_UNEXPECTED);
        assert_eq!(fields(seen.last().unwrap()), fields("{\"MsgId\":2,\"Result\":-2147418113}"));
        let (hr, seen, out, _) = scripted_19041(&["more:{\"MsgId\":1,\"Run_Output\":\"ab\"}", "more:{\"MsgId\":3,\"Run_Error_Output\":\"zz\"}", "wait", "end:{}"], RUN_FLAG_REDIRECT_OUTPUT, false);
        assert_eq!(hr, E_UNEXPECTED);
        assert_eq!(out, "ab");
        assert_eq!(fields(seen.last().unwrap()), fields("{\"MsgId\":2,\"Result\":-2147418113}"));
        let (hr, _, out, err) = scripted_19041(&["more:{\"MsgId\":1,\"Run_Output\":\"ab\"}", "more:{\"MsgId\":3,\"Run_Error_Output\":\"zz\"}", "end:{\"ExitCode\":0}"], RUN_FLAG_REDIRECT_OUTPUT, true);
        assert_eq!((hr, out.as_str(), err.as_str()), (0, "ab", "zz"));
    }
}

#[test]
fn keyed_dispatcher_refuses_unauthenticated_remote_run_sessions_like_console() {
    let _g = guard();
    let _d = unsafe { Dispatcher::with_remote_run_keyed("k3yXtf") };
    let dir = crate::tests::root().join("console").join("fs").join("C").join("authmarkers");
    std::fs::create_dir_all(&dir).unwrap();
    let marker = dir.join("xtfremoterun_marker.txt");
    let _ = std::fs::remove_file(&marker);
    for addr in ["127.0.0.1", "127.0.0.1/system"] {
        let (hr, code) = on_worker(90, move || unsafe {
            let (_, c) = Client::create(Some(addr));
            let mut code = 7u32;
            let hr = c.run_ex(Some(&child_cmd("child_marker_main")), Some("C:\\authmarkers"), RUN_FLAG_WAIT, 0, null_mut(), Some(&mut code));
            (hr, code)
        });
        assert_eq!(hr as u32, 0x8c11_040b, "{addr}");
        assert_eq!(code, 0, "{addr}");
        assert!(!marker.exists(), "{addr} launched the child without the session key");
    }
    let link = XBTP_PORT_SYSTEM | XBTP_PORT_KIND_LINK;
    let r = on_worker(60, move || unsafe {
        let mut out: Vec<HRESULT> = Vec::new();
        for addr in ["127.0.0.1", "127.0.0.1+wrong", "127.0.0.1+k3yXtf"] {
            let a = wide(addr);
            let mut s: Obj = null_mut();
            out.push(open_vm_session(a.as_ptr(), link, &mut s));
            if !s.is_null() {
                com::release(s);
            }
        }
        out
    });
    assert_eq!(r.iter().map(|h| *h as u32).collect::<Vec<_>>(), vec![0x8c11_040b, 0x8c11_040b, 0]);
}
