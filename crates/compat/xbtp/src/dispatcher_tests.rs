use crate::api::*;
use crate::conn::tests::{init, NET_LOCK};
use crate::conn::{TcpConnection, TcpConnection_InitializeServer_sp, XbtpConnection, E_COULD_NOT_CONNECT};
use crate::dispatcher::*;
use crate::dispatcher_com::*;
use crate::handshake::*;
use crate::msvc::{cpp_delete, cpp_new, SharedPtr};
use crate::registry::{XbtpExtensionRegistry, XbtpExtensionRegistry_Initialize, XbtpExtensionRegistry_dtor};
use crate::session_impl::{SessionImplVtbl, IID_XBTP_CLIENT_SESSION, IID_XBTP_SESSION};
use crate::sys::*;
use crate::xbtp2_ext::{com_release, new_message, Message, MetadataVtbl, XbtpCreateExtensionMetadata, IID_XBTP_EXTENSION_METADATA};
use core::ffi::c_void;
use core::ptr::null_mut;

const EXT: GUID = GUID::new(0x7e57d15a, 0x1111, 0x2222, [1, 2, 3, 4, 5, 6, 7, 8]);
const OTHER_EXT: GUID = GUID::new(0x7e57d15b, 0x1111, 0x2222, [1, 2, 3, 4, 5, 6, 7, 9]);
const ECHO_OP: u32 = 7;
const STREAM_OP: u32 = 8;
const FAIL_OP: u32 = 9;

unsafe fn json_text(m: *mut c_void) -> String {
    let m = m as *mut Message;
    let mut n = 0u32;
    let mut p: *mut u8 = null_mut();
    assert_eq!(((*(*m).vtbl).get_json_body)(m, &mut n, &mut p), S_OK);
    if p.is_null() {
        return String::new();
    }
    let s = core::slice::from_raw_parts(p, n as usize);
    String::from_utf8_lossy(s).trim_end_matches('\0').to_string()
}

unsafe fn set_reply(m: *mut c_void, text: &str) {
    let w = wide(text);
    assert_eq!(crate::xbtp2_ext::reply_set_json_w(m, w.as_ptr()), S_OK);
}

unsafe fn set_json(m: *mut c_void, text: &str) {
    let m = m as *mut Message;
    let w = wide(text);
    assert_eq!(((*(*m).vtbl).set_json_body_w)(m, w.as_ptr()), S_OK);
}

unsafe extern "system" fn echo(_session: usize, received: usize, reply: usize) -> HRESULT {
    let t = json_text(received as *mut c_void);
    set_reply(reply as *mut c_void, &t.replace("ping", "pong"));
    S_OK
}

unsafe extern "system" fn stream(session: usize, _received: usize, reply: usize) -> HRESULT {
    let s = session as *mut ServerSession;
    let more = new_message() as *mut c_void;
    set_json(more, "{\"part\":1}");
    let hr = ((*(*s).vtbl).send_more_message)(s, more);
    com_release(more);
    if hr < 0 {
        return hr;
    }
    let mut ev: HANDLE = null_mut();
    let hr = ((*(*s).vtbl).get_message_received_event)(s, &mut ev);
    if hr < 0 || ev.is_null() {
        return E_FAIL;
    }
    let mut got: *mut c_void = null_mut();
    let hr = ((*(*s).vtbl).receive_more_message)(s, 10_000, &mut got);
    if hr < 0 {
        return hr;
    }
    let t = json_text(got);
    com_release(got);
    set_reply(reply as *mut c_void, &format!("{{\"client\":{t}}}"));
    S_OK
}

unsafe extern "system" fn fail(_session: usize, _received: usize, _reply: usize) -> HRESULT {
    E_FAIL
}

unsafe fn drop_registry(p: *mut c_void) {
    XbtpExtensionRegistry_dtor(p as *mut XbtpExtensionRegistry);
    cpp_delete(p);
}

unsafe fn drop_dispatcher(p: *mut c_void) {
    XbtpConnectionDispatcher_dtor(p as *mut XbtpConnectionDispatcher);
    cpp_delete(p);
}

struct Server {
    disp: SharedPtr<XbtpConnectionDispatcher>,
    com: *mut DispatcherCom,
    run: Option<std::thread::JoinHandle<HRESULT>>,
}

struct Raw(*mut XbtpConnectionDispatcher);
unsafe impl Send for Raw {}

unsafe fn start_server(tcp_port: u32, pipe: Option<&str>, key: Option<&str>) -> Server {
    assert!(crate::xbtp2_sys::XbtpInitialize() >= 0);
    let r = cpp_new(8) as *mut XbtpExtensionRegistry;
    (*r).m_impl = null_mut();
    assert_eq!(XbtpExtensionRegistry_Initialize(r), S_OK);
    let mut reg = SharedPtr::new_with(r, drop_registry).unwrap();
    let mut tcp = SharedPtr::<TcpConnection>::default();
    assert_eq!(TcpConnection_InitializeServer_sp(tcp_port, &mut tcp), S_OK);
    let mut t: SharedPtr<XbtpConnection> = tcp.cast();
    let mut link = SharedPtr::<XbtpConnection>::default();
    let d = cpp_new(8) as *mut XbtpConnectionDispatcher;
    (*d).m_impl = core::ptr::null();
    assert_eq!(XbtpConnectionDispatcher_Initialize(d, &mut reg, &mut t, &mut link), S_OK);
    if let Some(p) = pipe {
        let w = wide(p);
        assert_eq!(XbtpConnectionDispatcher_AddPipeEndpoint(d, w.as_ptr()), S_OK);
    }
    let disp = SharedPtr::new_with(d, drop_dispatcher).unwrap();
    let mut out: *mut c_void = null_mut();
    let mut arg = disp.clone_ref();
    assert_eq!(XbtpWrapDispatcher(&mut arg, &IID_XBTP_DISPATCHER, &mut out), S_OK);
    let com = out as *mut DispatcherCom;
    let mut md: *mut c_void = null_mut();
    assert_eq!(XbtpCreateExtensionMetadata(&IID_XBTP_EXTENSION_METADATA, &mut md), S_OK);
    let mv = &**(md as *const *const MetadataVtbl);
    assert_eq!((mv.add_function)(md as *mut _, ECHO_OP, Some(echo)), S_OK);
    assert_eq!((mv.add_function)(md as *mut _, STREAM_OP, Some(stream)), S_OK);
    assert_eq!((mv.add_function)(md as *mut _, FAIL_OP, Some(fail)), S_OK);
    assert_eq!(((*(*com).vtbl).add_extension)(com, &EXT, md), S_OK);
    assert_eq!(((*(*com).vtbl).add_extension)(com, &EXT, md), 0x8007_1392u32 as i32);
    assert_eq!(((*(*com).vtbl).add_extension)(com, &OTHER_EXT, null_mut()), E_INVALIDARG);
    com_release(md);
    if let Some(k) = key {
        let w = wide(k);
        assert_eq!(((*(*com).vtbl).set_session_key)(com, w.as_ptr()), S_OK);
    }
    let raw = Raw(d);
    let run = std::thread::spawn(move || {
        let raw = raw;
        unsafe { XbtpConnectionDispatcher_Run(raw.0) }
    });
    Server { disp, com, run: Some(run) }
}

unsafe fn stop_server(mut s: Server) {
    assert_eq!(((*(*s.com).vtbl).shutdown)(s.com), S_OK);
    assert_eq!(s.run.take().unwrap().join().unwrap(), S_OK);
    ((*(*s.com).vtbl).release)(s.com);
    s.disp.release();
}

unsafe fn vt(s: *mut c_void) -> &'static SessionImplVtbl {
    &**(s as *const *const SessionImplVtbl)
}

unsafe fn call(s: *mut c_void, op: u32, body: &str) -> (i32, String) {
    let v = vt(s);
    let m = new_message() as *mut c_void;
    set_json(m, body);
    let mut ex = 0i32;
    assert_eq!((v.send_begin_message)(s as *mut _, op, m, &mut ex), S_OK);
    com_release(m);
    let mut status = 0i32;
    let mut reply: *mut c_void = null_mut();
    assert_eq!((v.receive_end_message)(s as *mut _, 10_000, ex, &mut status, &mut reply), S_OK);
    let t = json_text(reply);
    com_release(reply);
    (status, t)
}

#[test]
fn pipe_session_round_trips_through_dispatcher() {
    let _g = NET_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    init();
    unsafe {
        let pipe = format!(r"\\.\pipe\xbtp_dispatch_test_{}", GetCurrentProcessId());
        let srv = start_server(47901, Some(&pipe), None);
        let path = wide(&pipe);
        let mut s: *mut c_void = null_mut();
        assert_eq!(XbtpOpenPipeSession(path.as_ptr(), &EXT, &IID_XBTP_CLIENT_SESSION, &mut s), S_OK);
        let (st, t) = call(s, ECHO_OP, "{\"v\":\"ping\"}");
        assert_eq!(st, 0);
        assert_eq!(t, "{\"v\":\"pong\"}");
        let v = vt(s);
        let m = new_message() as *mut c_void;
        set_json(m, "{}");
        let mut ex = 0i32;
        assert_eq!((v.send_begin_message)(s as *mut _, STREAM_OP, m, &mut ex), S_OK);
        let mut part: *mut c_void = null_mut();
        assert_eq!((v.receive_more_message)(s as *mut _, 10_000, ex, &mut part), S_OK);
        assert_eq!(json_text(part), "{\"part\":1}");
        com_release(part);
        set_json(m, "{\"answer\":42}");
        assert_eq!((v.send_more_message)(s as *mut _, ex, m), S_OK);
        com_release(m);
        let mut status = 1i32;
        let mut reply: *mut c_void = null_mut();
        assert_eq!((v.receive_end_message)(s as *mut _, 10_000, ex, &mut status, &mut reply), S_OK);
        assert_eq!(status, 0);
        assert_eq!(json_text(reply), "{\"client\":{\"answer\":42}}");
        com_release(reply);
        let (st, _) = call(s, 99, "{}");
        assert_eq!(st, STATUS_OPERATION_UNDEFINED);
        let (st, _) = call(s, FAIL_OP, "{}");
        assert_eq!(st, STATUS_HANDLER_FAILED);
        let (st, t) = call(s, ECHO_OP, "{\"again\":\"ping\"}");
        assert_eq!((st, t.as_str()), (0, "{\"again\":\"pong\"}"));
        let (st, t) = call(s, TITLE_STATUS_OPERATION, "{}");
        assert_eq!((st, t.as_str()), (0, "{\"status\":\"unknown\"}"));
        com_release(s);
        let mut s2 = SharedPtr::default();
        assert_eq!(XbtpOpenPipeSessionShared(path.as_ptr(), &OTHER_EXT, &mut s2), crate::session::status_error(STATUS_EXTENSION_UNKNOWN));
        assert!(s2.ptr.is_null());
        assert_eq!(XbtpOpenPipeSessionShared(core::ptr::null(), &EXT, &mut s2), E_INVALIDARG);
        let empty = [0u16];
        assert_eq!(XbtpOpenPipeSession(empty.as_ptr(), &EXT, &IID_XBTP_SESSION, &mut s), E_INVALIDARG);
        stop_server(srv);
    }
}

unsafe fn open_ex2(port: u32, key: &str, allow: bool) -> (HRESULT, *mut c_void) {
    let mut o = XBTP_OPEN_SESSION_OPTIONS::default();
    let k: Vec<u16> = key.encode_utf16().collect();
    o.session_key[..k.len()].copy_from_slice(&k);
    o.allow_unneeded_key = allow as u32;
    o.quiet = 1;
    let host = wide("127.0.0.1");
    let mut s: *mut c_void = null_mut();
    let hr = XbtpOpenSessionEx2(host.as_ptr(), port, &o, &EXT, &IID_XBTP_CLIENT_SESSION, &mut s);
    (hr, s)
}

#[test]
fn ex2_handshake_with_session_key_over_loopback() {
    let _g = NET_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    init();
    unsafe {
        let srv = start_server(47903, None, Some("s3cret"));
        let (hr, s) = open_ex2(47903, "s3cret", false);
        assert_eq!(hr, S_OK);
        let (st, t) = call(s, ECHO_OP, "{\"k\":\"ping\"}");
        assert_eq!((st, t.as_str()), (0, "{\"k\":\"pong\"}"));
        com_release(s);
        let (hr, s) = open_ex2(47903, "wrong", false);
        assert_eq!(hr, E_SESSION_KEY_MISMATCH);
        com_release(s);
        stop_server(srv);
        let srv = start_server(47905, None, None);
        let (hr, s) = open_ex2(47905, "", false);
        assert_eq!(hr, S_OK);
        com_release(s);
        let (hr, s) = open_ex2(47905, "unneeded", false);
        assert_eq!(hr, E_SESSION_KEY_REQUIRED);
        com_release(s);
        let (hr, s) = open_ex2(47905, "unneeded", true);
        assert_eq!(hr, S_OK);
        com_release(s);
        let host = wide("127.0.0.1+");
        let mut s: *mut c_void = null_mut();
        assert_eq!(XbtpOpenSession(host.as_ptr(), 5, &EXT, &IID_XBTP_SESSION, &mut s), E_INVALIDARG);
        let mut s: *mut c_void = null_mut();
        assert_eq!(XbtpOpenSessionEx(host.as_ptr(), 47905, &EXT, &IID_XBTP_SESSION, null_mut()), E_INVALIDARG);
        let host = wide("127.0.0.1");
        assert_eq!(XbtpOpenSessionEx(host.as_ptr(), 47905, &EXT, &IID_XBTP_SESSION, &mut s), S_OK);
        let (st, _) = call(s, ECHO_OP, "{}");
        assert_eq!(st, 0);
        com_release(s);
        let mut bad = XBTP_OPEN_SESSION_OPTIONS::default();
        bad.size = 0x50;
        assert_eq!(XbtpOpenSessionEx2(host.as_ptr(), 47905, &bad, &EXT, &IID_XBTP_SESSION, &mut s), E_INVALIDARG);
        stop_server(srv);
    }
}

#[test]
fn open_session_parses_key_and_maps_port() {
    assert_eq!(get_server_port(0), Ok(0x1073));
    assert_eq!(get_server_port(1), Ok(0x107d));
    assert_eq!(get_server_port(3), Ok(0x1069));
    assert_eq!(get_server_port(0x0100_0001), Ok(0x0100_107d));
    assert_eq!(get_server_port(2), Err(E_INVALIDARG));
    assert_eq!(get_server_port(0x1073), Err(E_INVALIDARG));
    assert_eq!(core::mem::size_of::<XBTP_OPEN_SESSION_OPTIONS>(), 0x58);
    assert_eq!(core::mem::offset_of!(XBTP_OPEN_SESSION_OPTIONS, skip_ready_check), 0x44);
    assert_eq!(core::mem::offset_of!(XBTP_OPEN_SESSION_OPTIONS, quiet), 0x4c);
    assert_eq!(core::mem::offset_of!(XBTP_OPEN_SESSION_OPTIONS, timeout_ms), 0x54);
}

#[test]
fn remote_console_addresses_fail_like_no_route() {
    let _g = NET_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    init();
    unsafe {
        let mut s: *mut c_void = null_mut();
        for host in ["192.168.1.20", "10.0.0.7", "8.8.8.8"] {
            let w = wide(host);
            assert_eq!(XbtpOpenSessionEx(w.as_ptr(), 4211, &EXT, &IID_XBTP_SESSION, &mut s), E_COULD_NOT_CONNECT);
            assert!(s.is_null());
        }
        let w = wide("my-devkit");
        let hr = XbtpOpenSessionEx(w.as_ptr(), 4211, &EXT, &IID_XBTP_SESSION, &mut s);
        assert_eq!(hr, E_COULD_NOT_CONNECT);
        let w = wide("192.168.1.20+key");
        assert_eq!(XbtpOpenSession(w.as_ptr(), 0, &EXT, &IID_XBTP_SESSION, &mut s), E_COULD_NOT_CONNECT);
        let mut raw: *mut crate::session::XbtpSession = null_mut();
        assert_eq!(XbtpOpenSessionCpp(&EXT, w.as_ptr(), 0x0200_0000, &mut raw, true), E_INVALIDARG);
        assert_eq!(XbtpOpenSessionInternal(&EXT, w.as_ptr(), 0, null_mut(), true, 10), E_INVALIDARG);
        let w = wide("192.168.1.20");
        let hr = XbtpOpenSessionCpp(&EXT, w.as_ptr(), 0x0100_1073, &mut raw, true);
        assert!(raw.is_null());
    }
}

#[test]
fn wrap_dispatcher_rejects_null_and_reports_interfaces() {
    unsafe {
        let mut out: *mut c_void = 1 as *mut c_void;
        let mut empty = SharedPtr::<XbtpConnectionDispatcher>::default();
        assert_eq!(XbtpWrapDispatcher(&mut empty, &IID_XBTP_DISPATCHER, &mut out), E_INVALIDARG);
        let d = cpp_new(8) as *mut XbtpConnectionDispatcher;
        (*d).m_impl = core::ptr::null();
        let mut sp = SharedPtr::new_with(d, drop_dispatcher).unwrap();
        let mut a = sp.clone_ref();
        assert_eq!(XbtpWrapDispatcher(&mut a, &IID_XBTP_DISPATCHER, null_mut()), E_INVALIDARG);
        let mut a = sp.clone_ref();
        assert_eq!(XbtpWrapDispatcher(&mut a, &IID_XBTP_SESSION, &mut out), E_NOINTERFACE);
        let mut a = sp.clone_ref();
        assert_eq!(XbtpWrapDispatcher(&mut a, &IID_XBTP_DISPATCHER_ALT, &mut out), S_OK);
        let c = out as *mut DispatcherCom;
        assert_eq!(((*(*c).vtbl).add_extension)(c, &EXT, null_mut()), E_INVALIDARG);
        let long = wide("0123456789abcdef0123456789abcdefX");
        assert_eq!(((*(*c).vtbl).set_session_key)(c, long.as_ptr()), S_OK);
        assert_eq!(((*(*c).vtbl).set_session_key)(c, null_mut()), S_OK);
        ((*(*c).vtbl).release)(c);
        let mut reg = SharedPtr::<XbtpExtensionRegistry>::default();
        let mut t = SharedPtr::<XbtpConnection>::default();
        let mut l = SharedPtr::<XbtpConnection>::default();
        assert_eq!(XbtpConnectionDispatcher_Initialize(d, &mut reg, &mut t, &mut l), E_INVALIDARG);
        assert_eq!(XbtpConnectionDispatcher_Run(d), E_INVALIDARG);
        let p = wide(r"\\.\pipe\x");
        assert_eq!(XbtpConnectionDispatcher_AddPipeEndpoint(d, p.as_ptr()), E_INVALIDARG);
        XbtpConnectionDispatcher_Shutdown(d);
        sp.release();
    }
}

#[test]
fn server_session_vtable_matches_xtf_contract() {
    assert_eq!(core::mem::offset_of!(ServerSessionVtbl, send_more_message), 0x18);
    assert_eq!(core::mem::offset_of!(ServerSessionVtbl, send_more_message_async), 0x20);
    assert_eq!(core::mem::offset_of!(ServerSessionVtbl, receive_more_message), 0x28);
    assert_eq!(core::mem::offset_of!(ServerSessionVtbl, get_message_received_event), 0x30);
    assert_eq!(core::mem::offset_of!(SessionImplVtbl, send_begin_message), 0x18);
    assert_eq!(core::mem::offset_of!(SessionImplVtbl, receive_end_message), 0x38);
    assert_eq!(core::mem::offset_of!(SessionImplVtbl, get_property), 0x50);
    assert_eq!(core::mem::offset_of!(DispatcherComVtbl, add_extension), 0x20);
    assert_eq!(core::mem::offset_of!(DispatcherComVtbl, set_session_key), 0x38);
}

#[test]
fn reply_is_operation_end_message_with_console_slots() {
    unsafe {
        let inner = new_message();
        let e = crate::xbtp2_ext::new_end_message(inner) as *mut c_void;
        let slots = *(e as *const *const usize);
        let set_w: unsafe extern "system" fn(*mut c_void, *const u16) -> HRESULT = core::mem::transmute(*slots.add(6));
        let w = wide("{\"a\":1}");
        assert_eq!(set_w(e, w.as_ptr()), S_OK);
        assert_eq!(json_text(inner as *mut c_void), "{\"a\":1}");
        let set_bin: unsafe extern "system" fn(*mut c_void, u32, *const u8) -> HRESULT = core::mem::transmute(*slots.add(7));
        let data = [1u8, 2, 3];
        assert_eq!(set_bin(e, 3, data.as_ptr()), S_OK);
        let get_bin: unsafe extern "system" fn(*mut c_void, *mut u32, *mut *mut u8) -> HRESULT = core::mem::transmute(*slots.add(4));
        let mut n = 0u32;
        let mut p: *mut u8 = null_mut();
        assert_eq!(get_bin(e, &mut n, &mut p), S_OK);
        assert_eq!(core::slice::from_raw_parts(p, n as usize), &data);
        let mut q: *mut c_void = null_mut();
        assert_eq!(crate::xbtp2_ext::com_qi(e, &crate::xbtp2_ext::IID_XBTP_OPERATION_END_MESSAGE, &mut q), S_OK);
        com_release(q);
        assert_eq!(crate::xbtp2_ext::com_qi(e, &crate::xbtp2_ext::IID_XBTP_MESSAGE, &mut q), E_NOINTERFACE);
        com_release(e);
        com_release(inner as *mut c_void);
    }
}

#[test]
fn register_redirect_sends_clients_to_the_redirect_location() {
    let _g = NET_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    init();
    unsafe {
        let srv = start_server(47960, None, None);
        let loopback = wide("127.0.0.1");
        assert_eq!(((*(*srv.com).vtbl).register_redirect)(srv.com, 47961, loopback.as_ptr(), 47999), S_OK);
        assert_eq!(((*(*srv.com).vtbl).register_redirect)(srv.com, 47961, loopback.as_ptr(), 47960), S_OK);
        let (hr, s) = open_ex2(47961, "", false);
        assert_eq!(hr, S_OK);
        let (st, t) = call(s, ECHO_OP, "{\"r\":\"ping\"}");
        assert_eq!((st, t.as_str()), (0, "{\"r\":\"pong\"}"));
        com_release(s);
        assert_eq!(((*(*srv.com).vtbl).register_redirect)(srv.com, 47962, null_mut(), 0), S_OK);
        let (hr, s) = open_ex2(47962, "", false);
        assert_eq!(hr, crate::conn::E_SOCKET_CLOSED);
        assert!(s.is_null());
        let remote = wide("192.0.2.1");
        assert_eq!(((*(*srv.com).vtbl).register_redirect)(srv.com, 47963, remote.as_ptr(), 47960), S_OK);
        let (hr, s) = open_ex2(47963, "", false);
        assert_eq!(hr, crate::conn::E_COULD_NOT_CONNECT);
        assert!(s.is_null());
        assert_eq!(((*(*srv.com).vtbl).register_redirect)(srv.com, 47960, loopback.as_ptr(), 1), S_OK);
        stop_server(srv);
    }
}
