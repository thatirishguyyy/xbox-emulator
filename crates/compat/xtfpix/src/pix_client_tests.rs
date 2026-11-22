use crate::client::*;
use crate::client_tests::{canon, Server, PORT_LOCK};
use crate::com::*;
use crate::pix_client::*;
use crate::server::{server_set_ticks, server_ticks, XtfCreatePixServer, BUILD_BRANCH, FRIENDLY_COMPACT_VERSION, FRIENDLY_VERSION};
use core::ffi::c_void;
use core::ptr::null_mut;
use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use xbtp::xbtp2_ext::{EndMessage, Message, MetadataVtbl, IID_XBTP_EXTENSION_METADATA};

const TITLE_TCP_PORT: u32 = 0x107d;
const HOST_TCP_PORT: u32 = 0x1069;
const HANDSHAKE_REPLY: &str = "{\"Version\":1441796429,\"Result\":0,\"CpuFrequency\":{\"L\":10000000,\"H\":1},\"SystemType\":65534}";

#[derive(Clone)]
enum Push {
    Json(String),
    Bin(Vec<u8>),
}

#[derive(Clone, Default)]
struct Script {
    reply: String,
    status: HRESULT,
    stream: bool,
    collect: bool,
    pushes: Vec<Push>,
}

static SCRIPTS: Mutex<Vec<(u32, Script)>> = Mutex::new(Vec::new());
static SEEN: Mutex<Vec<(u32, String, Vec<u8>)>> = Mutex::new(Vec::new());
static DIAG_REPLY: Mutex<String> = Mutex::new(String::new());
static RELEASE: AtomicBool = AtomicBool::new(false);
static LEVEL: AtomicI32 = AtomicI32::new(4);
static LEVEL_AT_HANDSHAKE: AtomicI32 = AtomicI32::new(-1);
static LEVEL_SETS: Mutex<Vec<u32>> = Mutex::new(Vec::new());
static QUIET_LOCK: Mutex<()> = Mutex::new(());

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn script(op: u32, s: Script) {
    let mut v = lock(&SCRIPTS);
    v.retain(|(o, _)| *o != op);
    v.push((op, s));
}

fn reply(op: u32, text: &str, status: HRESULT) {
    script(op, Script { reply: text.into(), status, ..Default::default() });
}

fn streaming(op: u32, initial: &str, pushes: Vec<Push>) {
    script(op, Script { reply: initial.into(), stream: true, pushes, ..Default::default() });
}

fn reset_scripts() {
    lock(&SCRIPTS).clear();
    lock(&SEEN).clear();
    RELEASE.store(false, Ordering::Release);
    reply(OP_VERSION_HANDSHAKE, HANDSHAKE_REPLY, S_OK);
}

fn seen_ops() -> Vec<(u32, String, Vec<u8>)> {
    lock(&SEEN).clone()
}

fn seen_without_handshake() -> Vec<(u32, String, Vec<u8>)> {
    seen_ops().into_iter().filter(|s| s.0 != OP_VERSION_HANDSHAKE).collect()
}

fn j(s: &str) -> Push {
    Push::Json(s.into())
}

unsafe fn json_of(m: *mut c_void) -> String {
    let m = m as *mut Message;
    let mut n = 0u32;
    let mut p: *mut u8 = null_mut();
    if ((*(*m).vtbl).get_json_body)(m, &mut n, &mut p) < 0 || p.is_null() {
        return String::new();
    }
    String::from_utf8_lossy(core::slice::from_raw_parts(p, n as usize)).trim_end_matches('\0').to_string()
}

unsafe fn bin_of(m: *mut c_void) -> Vec<u8> {
    let m = m as *mut Message;
    let mut n = 0u32;
    let mut p: *mut u8 = null_mut();
    if ((*(*m).vtbl).get_binary_body)(m, &mut n, &mut p) < 0 || p.is_null() {
        return Vec::new();
    }
    core::slice::from_raw_parts(p, n as usize).to_vec()
}

unsafe fn ss_send_more(s: *mut c_void, m: *mut c_void) -> HRESULT {
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT = core::mem::transmute(slot(s, 3));
    f(s, m)
}

unsafe fn ss_receive_more(s: *mut c_void, timeout: u32, out: *mut *mut c_void) -> HRESULT {
    let f: unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> HRESULT = core::mem::transmute(slot(s, 5));
    f(s, timeout, out)
}

unsafe fn push(session: *mut c_void, p: &Push) {
    let mut msg = ComPtr::null();
    assert_eq!(XbtpCreateMessage(&IID_MESSAGE, msg.out()), S_OK);
    let m = msg.0 as *mut Message;
    match p {
        Push::Json(t) => {
            let t = w(t);
            assert_eq!(((*(*m).vtbl).set_json_body_w)(m, t.as_ptr()), S_OK);
        }
        Push::Bin(b) => {
            assert_eq!(((*(*m).vtbl).set_binary_body_copy)(m, b.len() as u32, b.as_ptr()), S_OK);
        }
    }
    assert_eq!(ss_send_more(session, msg.0), S_OK);
}

unsafe fn on_op(op: u32, session: usize, received: usize, out: usize) -> HRESULT {
    if op == OP_VERSION_HANDSHAKE {
        LEVEL_AT_HANDSHAKE.store(LEVEL.load(Ordering::Acquire), Ordering::Release);
    }
    let rm = received as *mut c_void;
    lock(&SEEN).push((op, json_of(rm), bin_of(rm)));
    let s = lock(&SCRIPTS).iter().find(|(o, _)| *o == op).map(|(_, s)| s.clone()).unwrap_or_default();
    let session = session as *mut c_void;
    if s.collect {
        while !RELEASE.load(Ordering::Acquire) {
            let mut m = ComPtr::null();
            if ss_receive_more(session, 20, m.out()) >= 0 && !m.0.is_null() {
                lock(&SEEN).push((op, json_of(m.0), bin_of(m.0)));
            }
        }
        return S_OK;
    }
    if s.stream {
        push(session, &Push::Json(s.reply.clone()));
        for p in &s.pushes {
            push(session, p);
        }
        while !RELEASE.load(Ordering::Acquire) {
            std::thread::sleep(Duration::from_millis(10));
        }
        return S_OK;
    }
    if !s.reply.is_empty() {
        let m = out as *mut EndMessage;
        let t = w(&s.reply);
        assert_eq!(((*(*m).vtbl).set_json_body_w)(m, t.as_ptr()), S_OK);
    }
    s.status
}

macro_rules! op_fn {
    ($name:ident, $op:expr) => {
        unsafe extern "system" fn $name(s: usize, r: usize, o: usize) -> HRESULT {
            on_op($op, s, r, o)
        }
    };
}

op_fn!(f_track, OP_TRACK_PROCESSES);
op_fn!(f_created, OP_PROCESS_CREATED);
op_fn!(f_capture, OP_CAPTURE_NEXT_FRAME);
op_fn!(f_result, OP_CAPTURE_RESULT);
op_fn!(f_sysmon, OP_CONFIGURE_SYSMON);
op_fn!(f_get_sysmon, OP_GET_SYSMON_CONFIGURATION);
op_fn!(f_timing, OP_DO_TIMING_CAPTURE);
op_fn!(f_cont, OP_SET_CONTINUOUS_TIMING_CAPTURE);
op_fn!(f_get_cont, OP_GET_CONTINUOUS_TIMING_CAPTURE);
op_fn!(f_track_sysmon, OP_TRACK_SYSMON_CONFIGURATION);
op_fn!(f_frame, OP_FRAME_COUNTER_DATA);
op_fn!(f_ping, OP_PING);
op_fn!(f_gfx, OP_TRACK_GRAPHICS_CAPTURE_REQUESTS);
op_fn!(f_xmem, OP_XMEM_STATISTICS);
op_fn!(f_hand, OP_VERSION_HANDSHAKE);
op_fn!(f_pause, OP_SET_PAUSE_STATE);
op_fn!(f_excl, OP_CREATE_EXCLUSIVE_CONNECTION);
op_fn!(f_rel, OP_RELEASE_EXCLUSIVE_CONNECTION);
op_fn!(f_friendly, OP_GET_FRIENDLY_VERSION);

unsafe extern "system" fn f_diag(_s: usize, received: usize, out: usize) -> HRESULT {
    let rm = received as *mut c_void;
    lock(&SEEN).push((0x1000 | OP_RETRIEVE_DIAGNOSTICS, json_of(rm), bin_of(rm)));
    let r = lock(&DIAG_REPLY).clone();
    let m = out as *mut EndMessage;
    let t = w(&r);
    assert_eq!(((*(*m).vtbl).set_json_body_w)(m, t.as_ptr()), S_OK);
    S_OK
}

type OpFn = unsafe extern "system" fn(usize, usize, usize) -> HRESULT;

unsafe fn metadata(fns: &[(u32, OpFn)]) -> *mut c_void {
    let mut md: *mut c_void = null_mut();
    assert_eq!(xbtp::xbtp2_ext::XbtpCreateExtensionMetadata(&IID_XBTP_EXTENSION_METADATA, &mut md), S_OK);
    let mv = &**(md as *const *const MetadataVtbl);
    for (op, f) in fns {
        assert_eq!((mv.add_function)(md as *mut _, *op, Some(*f)), S_OK);
    }
    md
}

unsafe fn add_scripted(srv: &Server) {
    let md = metadata(&[
        (OP_TRACK_PROCESSES, f_track),
        (OP_PROCESS_CREATED, f_created),
        (OP_CAPTURE_NEXT_FRAME, f_capture),
        (OP_CAPTURE_RESULT, f_result),
        (OP_CONFIGURE_SYSMON, f_sysmon),
        (OP_GET_SYSMON_CONFIGURATION, f_get_sysmon),
        (OP_DO_TIMING_CAPTURE, f_timing),
        (OP_SET_CONTINUOUS_TIMING_CAPTURE, f_cont),
        (OP_GET_CONTINUOUS_TIMING_CAPTURE, f_get_cont),
        (OP_TRACK_SYSMON_CONFIGURATION, f_track_sysmon),
        (OP_FRAME_COUNTER_DATA, f_frame),
        (OP_PING, f_ping),
        (OP_TRACK_GRAPHICS_CAPTURE_REQUESTS, f_gfx),
        (OP_XMEM_STATISTICS, f_xmem),
        (OP_VERSION_HANDSHAKE, f_hand),
        (OP_SET_PAUSE_STATE, f_pause),
        (OP_CREATE_EXCLUSIVE_CONNECTION, f_excl),
        (OP_RELEASE_EXCLUSIVE_CONNECTION, f_rel),
        (OP_GET_FRIENDLY_VERSION, f_friendly),
    ]);
    srv.add(&PIX_EXTENSION_ID, md);
    com_release(md);
}

#[repr(C)]
struct Callback {
    vtbl: *const usize,
    refs: AtomicU32,
    calls: Mutex<Vec<String>>,
}

unsafe extern "system" fn cb_qi(_: *mut Callback, _: *const GUID, _: *mut *mut c_void) -> HRESULT {
    E_NOINTERFACE
}

unsafe extern "system" fn cb_add_ref(this: *mut Callback) -> u32 {
    (*this).refs.fetch_add(1, Ordering::AcqRel) + 1
}

unsafe extern "system" fn cb_release(this: *mut Callback) -> u32 {
    (*this).refs.fetch_sub(1, Ordering::AcqRel) - 1
}

unsafe fn wstr(p: *const u16) -> String {
    if p.is_null() {
        return "<null>".into();
    }
    String::from_utf16_lossy(core::slice::from_raw_parts(p, wlen(p)))
}

unsafe fn note(this: *mut Callback, s: String) {
    lock(&(*this).calls).push(s);
}

unsafe extern "system" fn cb_created(this: *mut Callback, path: *const u16, pid: u32) -> HRESULT {
    note(this, format!("created {} {}", wstr(path), pid));
    S_OK
}

unsafe extern "system" fn cb_exited(this: *mut Callback, path: *const u16, pid: u32) -> HRESULT {
    note(this, format!("exited {} {}", wstr(path), pid));
    S_OK
}

unsafe extern "system" fn cb_frame(this: *mut Callback, a: u32, b: u32, c: u32) -> HRESULT {
    note(this, format!("frame {a} {b} {c}"));
    S_OK
}

unsafe extern "system" fn cb_completed(this: *mut Callback, pid: u32, r: i32, path: *const u16) -> HRESULT {
    note(this, format!("completed {pid} {r} {}", wstr(path)));
    S_OK
}

unsafe extern "system" fn cb_data(this: *mut Callback, n: u32, p: *const u8) -> HRESULT {
    note(this, format!("data {:?}", core::slice::from_raw_parts(p, n as usize)));
    S_OK
}

unsafe extern "system" fn cb_dword(this: *mut Callback, v: u32) -> HRESULT {
    note(this, format!("dword {v}"));
    S_OK
}

unsafe extern "system" fn cb_timing(this: *mut Callback, r: i32, path: *const u16) -> HRESULT {
    note(this, format!("timing {r} {}", wstr(path)));
    S_OK
}

unsafe extern "system" fn cb_range(this: *mut Callback, a: u64, b: u64) -> HRESULT {
    note(this, format!("range {a:#x} {b:#x}"));
    S_OK
}

fn callback(m3: usize, m4: usize) -> &'static Callback {
    let vt: &'static [usize; 5] = Box::leak(Box::new([cb_qi as *const () as usize, cb_add_ref as *const () as usize, cb_release as *const () as usize, m3, m4]));
    Box::leak(Box::new(Callback { vtbl: vt.as_ptr(), refs: AtomicU32::new(1), calls: Mutex::new(Vec::new()) }))
}

impl Callback {
    fn ptr(&self) -> *mut c_void {
        self as *const Callback as *mut c_void
    }

    fn calls(&self) -> Vec<String> {
        lock(&self.calls).clone()
    }

    fn wait(&self, n: usize) -> Vec<String> {
        let t0 = Instant::now();
        while lock(&self.calls).len() < n && t0.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(5));
        }
        self.calls()
    }

    fn refs(&self) -> u32 {
        self.refs.load(Ordering::Acquire)
    }
}

struct Client(*mut PixClient);

impl Client {
    unsafe fn new(addr: &str) -> Client {
        let a = w(addr);
        let mut p: *mut c_void = null_mut();
        assert_eq!(XtfCreatePixClient(a.as_ptr(), &IID_PIX_CLIENT, &mut p), S_OK);
        Client(p as *mut PixClient)
    }

    unsafe fn v(&self) -> &PixClientVtbl {
        &**(self.0 as *const *const PixClientVtbl)
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        unsafe { (self.v().release)(self.0) };
    }
}

fn wait_seen(n: usize) -> Vec<(u32, String, Vec<u8>)> {
    let t0 = Instant::now();
    while seen_without_handshake().len() < n && t0.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(5));
    }
    seen_without_handshake()
}

fn finish(c: Client, srv: Server) {
    drop(c);
    RELEASE.store(true, Ordering::Release);
    drop(srv);
}

static REAL_SERVER: AtomicUsize = AtomicUsize::new(0);

unsafe fn forward(op: u32, session: usize, received: usize, reply: usize) -> HRESULT {
    let h = crate::server::handler_ptr(REAL_SERVER.load(Ordering::Acquire) as *mut c_void, op);
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void) -> HRESULT = core::mem::transmute(slot(h, 4));
    f(h, session as *mut c_void, received as *mut c_void, reply as *mut c_void)
}

macro_rules! fwd_fn {
    ($name:ident, $op:expr) => {
        unsafe extern "system" fn $name(s: usize, r: usize, o: usize) -> HRESULT {
            forward($op, s, r, o)
        }
    };
}

fwd_fn!(g_hand, OP_VERSION_HANDSHAKE);
fwd_fn!(g_track, OP_TRACK_PROCESSES);
fwd_fn!(g_created, OP_PROCESS_CREATED);
fwd_fn!(g_clear, 3);
fwd_fn!(g_sysmon, OP_CONFIGURE_SYSMON);
fwd_fn!(g_get_sysmon, OP_GET_SYSMON_CONFIGURATION);
fwd_fn!(g_track_sysmon, OP_TRACK_SYSMON_CONFIGURATION);
fwd_fn!(g_ping, OP_PING);
fwd_fn!(g_excl, OP_CREATE_EXCLUSIVE_CONNECTION);
fwd_fn!(g_rel, OP_RELEASE_EXCLUSIVE_CONNECTION);
fwd_fn!(g_friendly, OP_GET_FRIENDLY_VERSION);

unsafe fn real_pix_server(srv: &Server) -> *mut c_void {
    let mut p: *mut c_void = null_mut();
    assert_eq!(XtfCreatePixServer(&crate::com::IID_PIX_SERVER, &mut p), S_OK);
    REAL_SERVER.store(p as usize, Ordering::Release);
    let md = metadata(&[
        (OP_VERSION_HANDSHAKE, g_hand),
        (OP_TRACK_PROCESSES, g_track),
        (OP_PROCESS_CREATED, g_created),
        (3, g_clear),
        (OP_CONFIGURE_SYSMON, g_sysmon),
        (OP_GET_SYSMON_CONFIGURATION, g_get_sysmon),
        (OP_TRACK_SYSMON_CONFIGURATION, g_track_sysmon),
        (OP_PING, g_ping),
        (OP_CREATE_EXCLUSIVE_CONNECTION, g_excl),
        (OP_RELEASE_EXCLUSIVE_CONNECTION, g_rel),
        (OP_GET_FRIENDLY_VERSION, g_friendly),
    ]);
    srv.add(&PIX_EXTENSION_ID, md);
    com_release(md);
    p
}

fn parsed(s: &str) -> crate::client_tests::J {
    canon(s)
}

#[test]
fn create_pix_client_handshakes_with_ported_pix_server() {
    let _g = PORT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        let srv = Server::start(TITLE_TCP_PORT);
        let ps = real_pix_server(&srv);
        let c = Client::new("local");
        assert_eq!(client_port(c.0 as *mut c_void), XBTP_PORT_TITLE);
        let mut info = PixInfo { server_version: 1, system_type: 1, cpu_frequency: 1, reserved: [9, 9] };
        assert_eq!((c.v().get_info)(c.0, &mut info), S_OK);
        let mut freq = 0i64;
        QueryPerformanceFrequency(&mut freq);
        assert_eq!(info, PixInfo { server_version: PIX_CLIENT_VERSION, system_type: XGetSystemType() as i16 as i32, cpu_frequency: freq as u64, reserved: [0, 0] });
        for iid in [IID_PIX_CLIENT, IID_IUNKNOWN, IID_PIX_CLIENT_2, IID_PIX_CLIENT_3, IID_PIX_CLIENT_4, IID_PIX_CLIENT_5] {
            let mut q: *mut c_void = null_mut();
            assert_eq!((c.v().query_interface)(c.0, &iid, &mut q), S_OK);
            assert_eq!(q, c.0 as *mut c_void);
            assert_eq!((c.v().release)(c.0), 1);
        }
        let mut q = 0x55usize as *mut c_void;
        assert_eq!((c.v().query_interface)(c.0, &IID_PIX_SERVER, &mut q), E_NOINTERFACE);
        assert_eq!(q as usize, 0x55);
        let mut v = 0u32;
        assert_eq!((c.v().get_client_version)(c.0, &mut v), S_OK);
        assert_eq!(v, PIX_CLIENT_VERSION);
        assert_eq!((c.v().get_client_version)(c.0, null_mut()), E_POINTER);
        let (mut a, mut b, mut br, mut x) = (null_mut(), null_mut(), null_mut(), 0u32);
        assert_eq!((c.v().get_friendly_version)(c.0, &mut a, &mut b, &mut br, &mut x), S_OK);
        assert_eq!((wstr(a), wstr(b), wstr(br), x), (FRIENDLY_VERSION.to_string(), FRIENDLY_COMPACT_VERSION.to_string(), BUILD_BRANCH.to_string(), 220604));
        SysFreeString(a);
        SysFreeString(b);
        SysFreeString(br);
        assert_eq!((c.v().get_friendly_version)(c.0, null_mut(), &mut b, &mut br, &mut x), E_POINTER);
        assert_eq!((c.v().get_friendly_version)(c.0, &mut a, null_mut(), &mut br, &mut x), E_POINTER);
        let mut flags = 0xffu32;
        assert_eq!((c.v().get_sysmon_configuration)(c.0, &mut flags), S_OK);
        assert_eq!(flags, 0);
        let procs = callback(cb_created as *const () as usize, cb_exited as *const () as usize);
        assert_eq!((c.v().track_instrumented_processes)(c.0, procs.ptr()), S_OK);
        assert_eq!(procs.refs(), 2);
        let path = w("D:\\Games\\title.exe");
        assert_eq!((c.v().announce_title_process)(c.0, 4242, path.as_ptr()), S_OK);
        assert_eq!(procs.wait(1), vec!["created D:\\Games\\title.exe 4242".to_string()]);
        let addr = w("127.0.0.1");
        assert_eq!(crate::api::XtfPixClearRunningProcessList(addr.as_ptr()), S_OK);
        assert_eq!(procs.wait(2)[1], "exited D:\\Games\\title.exe 0");
        assert_eq!((c.v().track_instrumented_processes)(c.0, null_mut()), S_OK);
        assert_eq!(procs.refs(), 1);
        let cfg = callback(cb_dword as *const () as usize, 0);
        assert_eq!((c.v().track_sysmon_configuration)(c.0, cfg.ptr()), S_OK);
        assert_eq!(cfg.wait(1), vec!["dword 0".to_string()]);
        assert_eq!((c.v().track_sysmon_configuration)(c.0, null_mut()), S_OK);
        assert_eq!(cfg.refs(), 1);
        let mut p2: *mut c_void = null_mut();
        let t = w("127.0.0.1/TITLE");
        assert_eq!(XtfCreatePixClient(t.as_ptr(), &IID_PIX_SERVER, &mut p2), E_NOINTERFACE);
        assert!(p2.is_null());
        drop(c);
        drop(srv);
        com_release(ps);
    }
}

#[test]
fn create_pix_client_validates_address_before_connecting() {
    let _q = lock(&QUIET_LOCK);
    unsafe {
        let mut p = 0x77usize as *mut c_void;
        let e = w("");
        assert_eq!(XtfCreatePixClient(e.as_ptr(), &IID_PIX_CLIENT, &mut p), E_NO_IPADDRESS_IN_CONNSTR);
        assert!(p.is_null());
        let b = w("127.0.0.1/bogus");
        assert_eq!(XtfCreatePixClient(b.as_ptr(), &IID_PIX_CLIENT, &mut p), E_INVALID_VM_TARGET);
        let s = w("/system");
        assert_eq!(XtfCreatePixClient(s.as_ptr(), &IID_PIX_CLIENT, &mut p), E_NO_IPADDRESS_IN_CONNSTR);
        assert_eq!(XtfCreatePixClient(null_mut(), &IID_PIX_CLIENT, &mut p), E_INVALIDARG);
        assert_eq!(XtfCreatePixClientQuiet(b.as_ptr(), &IID_PIX_CLIENT, &mut p), E_INVALID_VM_TARGET);
        assert!(p.is_null());
    }
}

#[test]
fn handshake_reply_shapes_map_to_console_errors() {
    let _g = PORT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        let srv = Server::start(TITLE_TCP_PORT);
        add_scripted(&srv);
        reset_scripts();
        let c = Client::new("localhost");
        let mut info = PixInfo::default();
        (c.v().get_info)(c.0, &mut info);
        assert_eq!(info, PixInfo { server_version: PIX_CLIENT_VERSION, system_type: -2, cpu_frequency: 0x1_0098_9680, reserved: [0, 0] });
        let hs: Vec<_> = seen_ops().into_iter().filter(|s| s.0 == OP_VERSION_HANDSHAKE).collect();
        assert_eq!(hs.len(), 1);
        assert_eq!(hs[0].1, "{\"Version\":1441796429}");
        drop(c);
        let a = w("127.0.0.1");
        let mut p: *mut c_void = null_mut();
        reply(OP_VERSION_HANDSHAKE, "{\"Version\":7,\"Result\":-2147024891}", S_OK);
        assert_eq!(XtfCreatePixClient(a.as_ptr(), &IID_PIX_CLIENT, &mut p), E_ACCESSDENIED);
        assert!(p.is_null());
        reply(OP_VERSION_HANDSHAKE, "{\"Result\":0}", S_OK);
        assert_eq!(XtfCreatePixClient(a.as_ptr(), &IID_PIX_CLIENT, &mut p), E_JSON_NOT_FOUND);
        reply(OP_VERSION_HANDSHAKE, "{\"Version\":7}", S_OK);
        assert_eq!(XtfCreatePixClient(a.as_ptr(), &IID_PIX_CLIENT, &mut p), E_JSON_NOT_FOUND);
        reply(OP_VERSION_HANDSHAKE, "", 0x8007_0057u32 as i32);
        assert_eq!(XtfCreatePixClient(a.as_ptr(), &IID_PIX_CLIENT, &mut p), E_FAIL);
        reply(OP_VERSION_HANDSHAKE, "{\"Version\":3,\"Result\":1}", S_OK);
        assert_eq!(XtfCreatePixClient(a.as_ptr(), &IID_PIX_CLIENT, &mut p), S_OK);
        let c = Client(p as *mut PixClient);
        (c.v().get_info)(c.0, &mut info);
        assert_eq!(info, PixInfo { server_version: 3, system_type: 0, cpu_frequency: 0, reserved: [0, 0] });
        drop(c);
        RELEASE.store(true, Ordering::Release);
        drop(srv);
    }
}

#[test]
fn quiet_factory_silences_xtf_logging_for_the_call() {
    let _q = lock(&QUIET_LOCK);
    let _g = PORT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    unsafe extern "C" fn get() -> i32 {
        LEVEL.load(Ordering::Acquire)
    }
    unsafe extern "C" fn set(v: u32) -> HRESULT {
        lock(&LEVEL_SETS).push(v);
        LEVEL.store(v as i32, Ordering::Release);
        S_OK
    }
    unsafe {
        let srv = Server::start(TITLE_TCP_PORT);
        add_scripted(&srv);
        reset_scripts();
        LEVEL.store(4, Ordering::Release);
        lock(&LEVEL_SETS).clear();
        set_log_table(Some(get), Some(set));
        let a = w("local");
        let mut p: *mut c_void = null_mut();
        assert_eq!(XtfCreatePixClientQuiet(a.as_ptr(), &IID_PIX_CLIENT_3, &mut p), S_OK);
        assert!(!p.is_null());
        assert_eq!(LEVEL_AT_HANDSHAKE.load(Ordering::Acquire), 0);
        assert_eq!(*lock(&LEVEL_SETS), vec![0, 4]);
        assert_eq!(LEVEL.load(Ordering::Acquire), 4);
        let c = Client(p as *mut PixClient);
        drop(c);
        reply(OP_VERSION_HANDSHAKE, "{\"Version\":7,\"Result\":-2147467259}", S_OK);
        lock(&LEVEL_SETS).clear();
        assert_eq!(XtfCreatePixClientQuiet(a.as_ptr(), &IID_PIX_CLIENT, &mut p), E_FAIL);
        assert_eq!(*lock(&LEVEL_SETS), vec![0, 4]);
        set_log_table(None, None);
        reset_scripts();
        assert_eq!(XtfCreatePixClientQuiet(a.as_ptr(), &IID_PIX_CLIENT, &mut p), S_OK);
        drop(Client(p as *mut PixClient));
        RELEASE.store(true, Ordering::Release);
        drop(srv);
    }
}

#[test]
fn request_reply_methods_send_console_bodies() {
    let _g = PORT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        let srv = Server::start(TITLE_TCP_PORT);
        add_scripted(&srv);
        reset_scripts();
        let c = Client::new("127.0.0.1");
        reply(OP_GET_SYSMON_CONFIGURATION, "{\"SysMonFlags\":7}", S_OK);
        let mut f = 0u32;
        assert_eq!((c.v().get_sysmon_configuration)(c.0, &mut f), S_OK);
        assert_eq!(f, 7);
        reply(OP_GET_SYSMON_CONFIGURATION, "{}", S_OK);
        f = 9;
        assert_eq!((c.v().get_sysmon_configuration)(c.0, &mut f), E_JSON_NOT_FOUND);
        assert_eq!(f, 0);
        reply(OP_GET_SYSMON_CONFIGURATION, "", 0x8000_4005u32 as i32);
        assert_eq!((c.v().get_sysmon_configuration)(c.0, &mut f), E_FAIL);
        let p = w("D:\\cap\\a.wpix");
        reply(OP_CAPTURE_RESULT, "{\"Result\":0}", S_OK);
        assert_eq!((c.v().report_graphics_capture_result)(c.0, 3, -1, p.as_ptr()), S_OK);
        reply(OP_CAPTURE_RESULT, "{\"Result\":-2147024809}", S_OK);
        assert_eq!((c.v().report_graphics_capture_result)(c.0, 3, -1, p.as_ptr()), 0x8007_0057u32 as i32);
        reply(OP_CAPTURE_RESULT, "{}", S_OK);
        assert_eq!((c.v().report_graphics_capture_result)(c.0, 3, -1, p.as_ptr()), E_JSON_NOT_FOUND);
        reply(OP_CAPTURE_RESULT, "", 0x8000_4005u32 as i32);
        assert_eq!((c.v().report_graphics_capture_result)(c.0, 3, -1, p.as_ptr()), E_FAIL);
        reply(OP_SET_PAUSE_STATE, "", S_OK);
        assert_eq!((c.v().set_paused_state)(c.0, 1), S_OK);
        reply(OP_SET_PAUSE_STATE, "{\"Result\":-1}", S_OK);
        assert_eq!((c.v().set_paused_state)(c.0, 0), S_OK);
        reply(OP_SET_PAUSE_STATE, "", 0x8000_4005u32 as i32);
        assert_eq!((c.v().set_paused_state)(c.0, 1), E_FAIL);
        reply(OP_PROCESS_CREATED, "", S_OK);
        let x = w("x.exe");
        assert_eq!((c.v().announce_title_process)(c.0, 77, x.as_ptr()), S_OK);
        reply(OP_PROCESS_CREATED, "", 0x8007_0057u32 as i32);
        assert_eq!((c.v().announce_title_process)(c.0, 77, x.as_ptr()), 0x8c11_01f8u32 as i32);
        reply(OP_GET_FRIENDLY_VERSION, "{\"FriendlyVersionProperty\":\"A\",\"FriendlyCompactVersionProperty\":\"B\",\"XDKEditionProperty\":4294967297.9,\"BuildBranchProperty\":\"C\"}", S_OK);
        let (mut a, mut b, mut br, mut xdk) = (null_mut(), null_mut(), null_mut(), 0u32);
        assert_eq!((c.v().get_friendly_version)(c.0, &mut a, &mut b, &mut br, &mut xdk), S_OK);
        assert_eq!((wstr(a), wstr(b), wstr(br), xdk), ("A".into(), "B".into(), "C".into(), 1));
        SysFreeString(a);
        SysFreeString(b);
        SysFreeString(br);
        reply(OP_GET_FRIENDLY_VERSION, "{\"FriendlyVersionProperty\":\"A\",\"FriendlyCompactVersionProperty\":\"B\",\"XDKEditionProperty\":1}", S_OK);
        let (mut a, mut b, mut br, mut xdk) = (null_mut(), null_mut(), null_mut(), 5u32);
        assert_eq!((c.v().get_friendly_version)(c.0, &mut a, &mut b, &mut br, &mut xdk), E_JSON_NOT_FOUND);
        assert!(a.is_null() && b.is_null() && br.is_null());
        assert_eq!(xdk, 5);
        reply(OP_SET_CONTINUOUS_TIMING_CAPTURE, "{\"Result\":0}", S_OK);
        assert_eq!((c.v().stop_continuous_timing_capture)(c.0), S_OK);
        reply(OP_SET_CONTINUOUS_TIMING_CAPTURE, "{\"Result\":-2147467259}", S_OK);
        assert_eq!((c.v().stop_continuous_timing_capture)(c.0), E_FAIL);
        reply(OP_SET_CONTINUOUS_TIMING_CAPTURE, "{}", S_OK);
        assert_eq!((c.v().stop_continuous_timing_capture)(c.0), E_JSON_NOT_FOUND);
        let got: Vec<(u32, crate::client_tests::J)> = seen_without_handshake().into_iter().map(|(o, s, _)| (o, if s.is_empty() { canon("\"\"") } else { canon(&s) })).collect();
        let want = vec![
            (OP_GET_SYSMON_CONFIGURATION, "{}"),
            (OP_GET_SYSMON_CONFIGURATION, "{}"),
            (OP_GET_SYSMON_CONFIGURATION, "{}"),
            (OP_CAPTURE_RESULT, "{\"ProcessId\":3,\"Result\":-1,\"FilePath\":\"D:\\\\cap\\\\a.wpix\"}"),
            (OP_CAPTURE_RESULT, "{\"ProcessId\":3,\"Result\":-1,\"FilePath\":\"D:\\\\cap\\\\a.wpix\"}"),
            (OP_CAPTURE_RESULT, "{\"ProcessId\":3,\"Result\":-1,\"FilePath\":\"D:\\\\cap\\\\a.wpix\"}"),
            (OP_CAPTURE_RESULT, "{\"ProcessId\":3,\"Result\":-1,\"FilePath\":\"D:\\\\cap\\\\a.wpix\"}"),
            (OP_SET_PAUSE_STATE, "{\"PauseState\":1}"),
            (OP_SET_PAUSE_STATE, "{\"PauseState\":0}"),
            (OP_SET_PAUSE_STATE, "{\"PauseState\":1}"),
            (OP_PROCESS_CREATED, "{\"ProcessId\":77,\"ProcessPath\":\"x.exe\"}"),
            (OP_PROCESS_CREATED, "{\"ProcessId\":77,\"ProcessPath\":\"x.exe\"}"),
            (OP_GET_FRIENDLY_VERSION, "{}"),
            (OP_GET_FRIENDLY_VERSION, "{}"),
            (OP_SET_CONTINUOUS_TIMING_CAPTURE, "{\"CaptureEnabled\":0}"),
            (OP_SET_CONTINUOUS_TIMING_CAPTURE, "{\"CaptureEnabled\":0}"),
            (OP_SET_CONTINUOUS_TIMING_CAPTURE, "{\"CaptureEnabled\":0}"),
        ];
        assert_eq!(got, want.into_iter().map(|(o, s)| (o, parsed(s))).collect::<Vec<_>>());
        finish(c, srv);
    }
}

#[test]
fn streaming_methods_start_listeners_and_deliver_callbacks() {
    let _g = PORT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        let srv = Server::start(TITLE_TCP_PORT);
        add_scripted(&srv);
        reset_scripts();
        let c = Client::new("127.0.0.1/title");
        streaming(OP_CAPTURE_NEXT_FRAME, "{\"Result\":0}", vec![j("{\"ProcessId\":10,\"Result\":-5,\"FilePath\":\"D:\\\\f.wpix\"}"), j("{\"ProcessId\":11,\"Result\":0,\"FilePath\":\"late\"}")]);
        let done = callback(cb_completed as *const () as usize, 0);
        assert_eq!((c.v().capture_next_frame)(c.0, 10, 3, 2, 1, done.ptr()), S_OK);
        assert_eq!(done.wait(1), vec!["completed 10 -5 D:\\f.wpix".to_string()]);
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(done.calls().len(), 1);
        streaming(OP_TRACK_GRAPHICS_CAPTURE_REQUESTS, "{\"Result\":0}", vec![j("{\"ProcessId\":1,\"CaptureFlags\":2,\"CommandListLatencyFrameCount\":3,\"CaptureFrameCount\":4}"), j("{\"ProcessId\":1,\"CaptureFlags\":5,\"CommandListLatencyFrameCount\":6,\"CaptureFrameCount\":7}")]);
        let gfx = callback(cb_frame as *const () as usize, 0);
        assert_eq!((c.v().track_graphics_capture_requests)(c.0, gfx.ptr()), S_OK);
        assert_eq!(gfx.wait(2), vec!["frame 2 3 4".to_string(), "frame 5 6 7".to_string()]);
        streaming(OP_TRACK_GRAPHICS_CAPTURE_REQUESTS, "{\"Result\":3}", vec![]);
        assert_eq!((c.v().track_graphics_capture_requests)(c.0, null_mut()), 3);
        assert_eq!(gfx.refs(), 1);
        streaming(OP_CONFIGURE_SYSMON, "{\"Result\":0}", vec![Push::Bin(vec![1, 2, 3]), Push::Bin(vec![9])]);
        let data = callback(cb_data as *const () as usize, 0);
        assert_eq!((c.v().configure_sysmon)(c.0, 0, data.ptr()), E_INVALIDARG);
        assert_eq!((c.v().configure_sysmon)(c.0, 9, null_mut()), E_INVALIDARG);
        assert_eq!((c.v().configure_sysmon)(c.0, 9, data.ptr()), S_OK);
        assert_eq!(data.wait(2), vec!["data [1, 2, 3]".to_string(), "data [9]".to_string()]);
        streaming(OP_CONFIGURE_SYSMON, "{\"Result\":2}", vec![]);
        assert_eq!((c.v().configure_sysmon)(c.0, 0, null_mut()), 2);
        assert_eq!(data.refs(), 1);
        streaming(OP_CONFIGURE_SYSMON, "{\"Result\":-2147467259}", vec![]);
        assert_eq!((c.v().configure_sysmon)(c.0, 4, data.ptr()), E_FAIL);
        assert_eq!(data.refs(), 1);
        streaming(OP_CONFIGURE_SYSMON, "{}", vec![]);
        assert_eq!((c.v().configure_sysmon)(c.0, 4, data.ptr()), E_JSON_NOT_FOUND);
        streaming(OP_TRACK_PROCESSES, "{\"Result\":0}", vec![j("{\"ProcessId\":5,\"ProcessPath\":\"a.exe\",\"ProcessCreated\":0}"), j("{\"ProcessId\":6,\"ProcessPath\":\"b.exe\",\"ProcessCreated\":1}")]);
        let procs = callback(cb_created as *const () as usize, cb_exited as *const () as usize);
        assert_eq!((c.v().track_instrumented_processes)(c.0, procs.ptr()), S_OK);
        assert_eq!(procs.wait(2), vec!["exited a.exe 5".to_string(), "created b.exe 6".to_string()]);
        streaming(OP_TRACK_SYSMON_CONFIGURATION, "{\"Result\":0}", vec![j("{\"SysMonFlags\":3}")]);
        let cfg = callback(cb_dword as *const () as usize, 0);
        assert_eq!((c.v().track_sysmon_configuration)(c.0, cfg.ptr()), S_OK);
        assert_eq!(cfg.wait(1), vec!["dword 3".to_string()]);
        streaming(OP_SET_CONTINUOUS_TIMING_CAPTURE, "{\"Result\":0}", vec![j("{\"StartTime\":{\"L\":1,\"H\":2},\"EndTime\":{\"L\":3,\"H\":4}}")]);
        let range = callback(cb_range as *const () as usize, 0);
        assert_eq!((c.v().start_continuous_timing_capture)(c.0, range.ptr()), S_OK);
        assert_eq!(range.wait(1), vec!["range 0x200000001 0x400000003".to_string()]);
        assert_eq!((c.v().start_continuous_timing_capture_ex)(c.0, 6, range.ptr()), S_OK);
        assert_eq!(range.wait(2).len(), 2);
        reply(OP_SET_CONTINUOUS_TIMING_CAPTURE, "{\"Result\":0}", S_OK);
        assert_eq!((c.v().stop_continuous_timing_capture)(c.0), S_OK);
        assert_eq!(range.refs(), 1);
        streaming(OP_DO_TIMING_CAPTURE, "{\"Result\":0}", vec![j("{\"Result\":0,\"FilePath\":\"t1\"}"), j("{\"Result\":-1,\"FilePath\":\"t2\"}"), j("{\"Result\":0,\"FilePath\":\"t3\"}")]);
        let timing = callback(cb_timing as *const () as usize, 0);
        assert_eq!((c.v().do_timing_capture)(c.0, 0x5_0000_0006, timing.ptr()), S_OK);
        assert_eq!(timing.wait(2), vec!["timing 0 t1".to_string(), "timing -1 t2".to_string()]);
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(timing.calls().len(), 2);
        streaming(OP_GET_CONTINUOUS_TIMING_CAPTURE, "{\"Result\":0}", vec![j("{\"Result\":0,\"FilePath\":\"g1\"}"), j("{\"Result\":0,\"FilePath\":\"g2\"}")]);
        assert_eq!((c.v().get_continuous_timing_capture)(c.0, 0x10, 0x1_0000_0020, timing.ptr()), S_OK);
        assert_eq!(timing.wait(3)[2], "timing 0 g1");
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(timing.calls().len(), 3);
        assert_eq!((c.v().do_timing_capture_ex)(c.0, 2, 8, timing.ptr()), S_OK);
        let sent = wait_seen(15);
        let bodies: Vec<(u32, crate::client_tests::J)> = sent.into_iter().map(|(o, s, _)| (o, if s.is_empty() { canon("\"\"") } else { canon(&s) })).collect();
        let want = vec![
            (OP_CAPTURE_NEXT_FRAME, "{\"ProcessId\":10,\"CaptureFlags\":3,\"CommandListLatencyFrameCount\":2,\"CaptureFrameCount\":1}"),
            (OP_TRACK_GRAPHICS_CAPTURE_REQUESTS, "\"\""),
            (OP_TRACK_GRAPHICS_CAPTURE_REQUESTS, "\"\""),
            (OP_CONFIGURE_SYSMON, "{\"SysMonFlags\":9}"),
            (OP_CONFIGURE_SYSMON, "{\"SysMonFlags\":0}"),
            (OP_CONFIGURE_SYSMON, "{\"SysMonFlags\":4}"),
            (OP_CONFIGURE_SYSMON, "{\"SysMonFlags\":4}"),
            (OP_TRACK_PROCESSES, "{\"TrackingEnabled\":1}"),
            (OP_TRACK_SYSMON_CONFIGURATION, "{\"TrackingEnabled\":1}"),
            (OP_SET_CONTINUOUS_TIMING_CAPTURE, "{\"CaptureEnabled\":1}"),
            (OP_SET_CONTINUOUS_TIMING_CAPTURE, "{\"CaptureEnabled\":1,\"CaptureFlags\":6}"),
            (OP_SET_CONTINUOUS_TIMING_CAPTURE, "{\"CaptureEnabled\":0}"),
            (OP_DO_TIMING_CAPTURE, "{\"Duration\":{\"L\":6,\"H\":5}}"),
            (OP_GET_CONTINUOUS_TIMING_CAPTURE, "{\"StartTime\":{\"L\":16,\"H\":0},\"EndTime\":{\"L\":32,\"H\":1}}"),
            (OP_DO_TIMING_CAPTURE, "{\"Duration\":{\"L\":2,\"H\":0},\"CaptureFlags\":8}"),
        ];
        assert_eq!(bodies, want.into_iter().map(|(o, s)| (o, parsed(s))).collect::<Vec<_>>());
        assert_eq!((c.v().track_instrumented_processes)(c.0, null_mut()), S_OK);
        assert_eq!((c.v().track_sysmon_configuration)(c.0, null_mut()), S_OK);
        assert_eq!(procs.refs(), 1);
        assert_eq!(cfg.refs(), 1);
        finish(c, srv);
        for cb in [done, gfx, data, procs, cfg, range, timing] {
            assert_eq!(cb.refs(), 1);
        }
    }
}

#[test]
fn binary_report_streams_reuse_one_exchange() {
    let _g = PORT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        let srv = Server::start(TITLE_TCP_PORT);
        add_scripted(&srv);
        reset_scripts();
        script(OP_FRAME_COUNTER_DATA, Script { collect: true, ..Default::default() });
        script(OP_XMEM_STATISTICS, Script { collect: true, ..Default::default() });
        let c = Client::new("127.0.0.1");
        let a = [1u8, 2, 3, 4];
        let b = [5u8; 9];
        assert_eq!((c.v().report_frame_counter_data)(c.0, a.len() as u32, a.as_ptr()), S_OK);
        assert_eq!((c.v().report_frame_counter_data)(c.0, b.len() as u32, b.as_ptr()), S_OK);
        assert_eq!((c.v().report_xmem_statistics)(c.0, 2, [7u8, 8].as_ptr()), S_OK);
        assert_eq!((c.v().report_xmem_statistics)(c.0, 1, [6u8].as_ptr()), S_OK);
        assert_eq!((c.v().report_frame_counter_data)(c.0, 1, [0xffu8].as_ptr()), S_OK);
        let got = wait_seen(5);
        let mut frame: Vec<Vec<u8>> = got.iter().filter(|s| s.0 == OP_FRAME_COUNTER_DATA).map(|s| s.2.clone()).collect();
        let xmem: Vec<Vec<u8>> = got.iter().filter(|s| s.0 == OP_XMEM_STATISTICS).map(|s| s.2.clone()).collect();
        assert!(got.iter().all(|s| s.1.is_empty()));
        assert_eq!(got.iter().filter(|s| s.0 == OP_FRAME_COUNTER_DATA).count(), 3);
        frame.truncate(3);
        assert_eq!(frame, vec![a.to_vec(), b.to_vec(), vec![0xff]]);
        assert_eq!(xmem, vec![vec![7, 8], vec![6]]);
        finish(c, srv);
    }
}

#[test]
fn console_id_comes_from_host_diagnostics_extension() {
    let _g = PORT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        let title = Server::start(TITLE_TCP_PORT);
        add_scripted(&title);
        reset_scripts();
        let host = Server::start(HOST_TCP_PORT);
        let md = metadata(&[(OP_RETRIEVE_DIAGNOSTICS, f_diag)]);
        host.add(&DIAGNOSTIC_INFORMATION_EXTENSION_ID, md);
        com_release(md);
        let c = Client::new("127.0.0.1");
        assert_eq!((c.v().get_console_id)(c.0, null_mut()), E_POINTER);
        *lock(&DIAG_REPLY) = "{\"Properties\":{\"Names\":[\"OsVersion\",\"consoleID\",\"ConsoleId\"],\"Values\":[\"10.0\",\"ABC123\",\"second\"]}}".into();
        let mut id: BSTR = null_mut();
        assert_eq!((c.v().get_console_id)(c.0, &mut id), S_OK);
        assert_eq!(wstr(id), "ABC123");
        SysFreeString(id);
        *lock(&DIAG_REPLY) = "{\"Properties\":{\"Names\":[\"OsVersion\"],\"Values\":[\"10.0\"]}}".into();
        let mut id = 0x10usize as BSTR;
        assert_eq!((c.v().get_console_id)(c.0, &mut id), S_OK);
        assert_eq!(id as usize, 0x10);
        *lock(&DIAG_REPLY) = "{\"Properties\":{\"Names\":[\"ConsoleId\"]}}".into();
        assert_eq!((c.v().get_console_id)(c.0, &mut id), E_JSON_NOT_FOUND);
        *lock(&DIAG_REPLY) = "{}".into();
        assert_eq!((c.v().get_console_id)(c.0, &mut id), E_JSON_NOT_FOUND);
        *lock(&DIAG_REPLY) = "{\"Properties\":{\"Names\":[\"ConsoleId\"],\"Values\":[]}}".into();
        let hr = (c.v().get_console_id)(c.0, &mut id);
        assert!(hr < 0, "{hr:#x}");
        let diag: Vec<_> = seen_ops().into_iter().filter(|s| s.0 == 0x1000 | OP_RETRIEVE_DIAGNOSTICS).collect();
        assert_eq!(diag.len(), 5);
        assert!(diag.iter().all(|s| s.1.is_empty()));
        drop(c);
        RELEASE.store(true, Ordering::Release);
        drop(host);
        drop(title);
    }
}

#[test]
fn channel_status_uses_exclusive_connection_and_pings_on_host() {
    let _g = PORT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        let srv = Server::start(HOST_TCP_PORT);
        let ps = real_pix_server(&srv);
        let c = Client::new("127.0.0.1/host");
        assert_eq!(client_port(c.0 as *mut c_void), XBTP_PORT_HOST);
        let status = callback(cb_dword as *const () as usize, 0);
        assert_eq!((c.v().track_channel_status_with_fixed_token)(c.0, status.ptr(), 0x77, 0x1234_0000_5678, 0), S_OK);
        assert_eq!(server_ticks(ps).1, 0x1234_0000_5678);
        assert_eq!(client_token(c.0 as *mut c_void), 0x1234_0000_5678);
        assert_eq!(status.refs(), 2);
        let tick0 = server_ticks(ps).0;
        std::thread::sleep(Duration::from_millis(5600));
        let tick1 = server_ticks(ps).0;
        assert!(tick1 > tick0, "{tick0} {tick1}");
        assert!(status.calls().is_empty());
        let other = Client::new("127.0.0.1/host");
        let s2 = callback(cb_dword as *const () as usize, 0);
        assert_eq!((other.v().track_channel_status_with_fixed_token)(other.0, s2.ptr(), 1, 0x99, 0), HR_EXCLUSIVE_CONNECTION_HELD);
        assert_eq!(client_token(other.0 as *mut c_void), 0);
        assert_eq!(s2.refs(), 1);
        drop(other);
        server_set_ticks(ps, tick1, 0x4444);
        assert_eq!(status.wait(1), vec!["dword 119".to_string()]);
        server_set_ticks(ps, GetTickCount64(), 0x1234_0000_5678);
        assert_eq!((c.v().track_channel_status)(c.0, null_mut(), 0, 0), S_OK);
        let t0 = Instant::now();
        while server_ticks(ps).0 != 0 && t0.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(server_ticks(ps).0, 0);
        assert_eq!(status.refs(), 1);
        drop(c);
        drop(srv);
        com_release(ps);
        assert_eq!(status.refs(), 1);
    }
}

#[test]
fn channel_status_on_title_port_skips_exclusive_connection() {
    let _g = PORT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        let srv = Server::start(TITLE_TCP_PORT);
        add_scripted(&srv);
        reset_scripts();
        let c = Client::new("127.0.0.1");
        let status = callback(cb_dword as *const () as usize, 0);
        assert_eq!((c.v().track_channel_status)(c.0, status.ptr(), 5, 1), S_OK);
        assert_eq!(status.refs(), 2);
        assert!(seen_without_handshake().is_empty());
        assert_eq!(client_token(c.0 as *mut c_void), 0);
        assert_eq!((c.v().track_channel_status)(c.0, null_mut(), 0, 0), S_OK);
        assert_eq!(status.refs(), 1);
        assert!(seen_without_handshake().is_empty());
        finish(c, srv);
    }
}

#[test]
fn remote_pix_client_addresses_fail_offline() {
    let jobs: Vec<_> = ["10.1.2.3", "pixkit.example/title", "192.168.4.4/host"]
        .into_iter()
        .map(|a| {
            std::thread::spawn(move || unsafe {
                let s = w(a);
                let mut p = 0x5usize as *mut c_void;
                let t0 = Instant::now();
                let hr = XtfCreatePixClient(s.as_ptr(), &IID_PIX_CLIENT, &mut p);
                (hr, p as usize, t0.elapsed())
            })
        })
        .collect();
    for j in jobs {
        let (hr, p, took) = j.join().unwrap();
        assert_eq!(hr, E_COULD_NOT_CONNECT);
        assert_eq!(p, 0);
        assert!(took >= Duration::from_secs(20), "{took:?}");
    }
}

#[link(name = "kernel32", kind = "raw-dylib")]
extern "system" {
    fn LoadLibraryExW(name: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
    fn GetProcAddress(m: *mut c_void, name: *const u8) -> usize;
}

type Create = unsafe extern "system" fn(*const u16, *const GUID, *mut *mut c_void) -> HRESULT;

fn console_dll() -> Option<usize> {
    static H: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    let h = *H.get_or_init(|| {
        for g in ["XDefiant", "Fortnite"] {
            let p = format!("D:\\xbox-pc-port\\games\\{g}\\extracted\\EmbeddedXvd\\Windows\\System32\\xtfpix.dll");
            if std::path::Path::new(&p).is_file() {
                let h = unsafe { LoadLibraryExW(w(&p).as_ptr(), null_mut(), 8) };
                if !h.is_null() {
                    return h as usize;
                }
            }
        }
        0
    });
    if h == 0 {
        None
    } else {
        Some(h)
    }
}

#[test]
fn oracle_console_factory_argument_errors_match() {
    let _q = lock(&QUIET_LOCK);
    let h = console_dll().expect("console xtfpix.dll did not load with LOAD_WITH_ALTERED_SEARCH_PATH");
    unsafe {
        for name in ["XtfCreatePixClient\0", "XtfCreatePixClientQuiet\0"] {
            let p = GetProcAddress(h as *mut c_void, name.as_ptr());
            assert!(p != 0, "{name}");
            let console: Create = core::mem::transmute(p);
            let ours: Create = if name.starts_with("XtfCreatePixClientQ") { XtfCreatePixClientQuiet } else { XtfCreatePixClient };
            for addr in ["", "/system", "127.0.0.1/bogus", "box/title+x", "box/systems"] {
                let a = w(addr);
                let mut pc = 0x9usize as *mut c_void;
                let mut po = 0x9usize as *mut c_void;
                let hc = console(a.as_ptr(), &IID_PIX_CLIENT, &mut pc);
                let ho = ours(a.as_ptr(), &IID_PIX_CLIENT, &mut po);
                assert_eq!(ho, hc, "{name} {addr}");
                assert_eq!(po, pc, "{name} {addr}");
            }
        }
    }
}
