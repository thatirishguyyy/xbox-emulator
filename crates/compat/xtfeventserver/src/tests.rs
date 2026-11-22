use crate::com::*;
use crate::dispatcher::{self, Dispatcher};
use crate::etw::{self, EtwSource};
use crate::imports::Imports;
use crate::server::*;
use crate::shared::*;
use crate::sys::*;
use crate::{GetEventSourceMap, TreeNode, XtfOrdinal_4};
use core::ffi::c_void;
use core::mem::transmute;
use core::ptr::null_mut;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Mutex, RwLock};
use std::time::{Duration, Instant};

pub(crate) fn test_imports() -> Result<Imports, HRESULT> {
    unsafe {
        Ok(Imports {
            create_extension_metadata: transmute(xbtp::xbtp2_ext::XbtpCreateExtensionMetadata as *const () as usize),
            get_json_object_from_utf8: transmute(xbtp::xbtp2_json::XbtpGetJsonObjectFromUtf8 as *const () as usize),
            create_json_object: transmute(xbtp::xbtp2_json::XbtpCreateJsonObject as *const () as usize),
            log_info_v: transmute(xbtp::base::LogInfoV as *const () as usize),
            create_message: transmute(xbtp::xbtp2_ext::XbtpCreateMessage as *const () as usize),
        })
    }
}

#[repr(C)]
struct EventDescriptor {
    id: u16,
    version: u8,
    channel: u8,
    level: u8,
    opcode: u8,
    task: u16,
    keyword: u64,
}

#[repr(C)]
struct DataDescriptor {
    ptr: u64,
    size: u32,
    reserved: u32,
}

#[link(name = "advapi32")]
extern "system" {
    fn EventRegister(id: *const GUID, cb: *const c_void, ctx: *const c_void, h: *mut u64) -> u32;
    fn EventUnregister(h: u64) -> u32;
    fn EventWrite(h: u64, d: *const EventDescriptor, n: u32, data: *const DataDescriptor) -> u32;
}

static REGISTER_LOCK: RwLock<()> = RwLock::new(());

const TEST_PROVIDER: GUID = GUID::new(0x5d2e6f1a, 0x7c3b, 0x4e9d, [0x9a, 0x41, 0x2f, 0x6b, 0x0c, 0x8e, 0x31, 0x77]);
const TEST_PROVIDER_TEXT: &str = "{5D2E6F1A-7C3B-4E9D-9A41-2F6B0C8E3177}";

struct Sent {
    json: String,
    binary: Vec<u8>,
}

#[repr(C)]
struct SessionVtbl {
    qi: unsafe extern "system" fn(Obj, *const GUID, *mut Obj) -> HRESULT,
    add_ref: unsafe extern "system" fn(Obj) -> u32,
    release: unsafe extern "system" fn(Obj) -> u32,
    send_more: unsafe extern "system" fn(Obj, Obj) -> HRESULT,
}

#[repr(C)]
struct PropsVtbl {
    qi: unsafe extern "system" fn(Obj, *const GUID, *mut Obj) -> HRESULT,
    add_ref: unsafe extern "system" fn(Obj) -> u32,
    release: unsafe extern "system" fn(Obj) -> u32,
    get_property: unsafe extern "system" fn(Obj, *const u16, *mut BSTR) -> HRESULT,
}

#[repr(C)]
struct Session {
    vtbl: *const SessionVtbl,
    props: Props,
    refs: AtomicU32,
    fail_from: u32,
    fail_hr: HRESULT,
    sent: Mutex<Vec<Sent>>,
    address_asked: AtomicBool,
}

#[repr(C)]
struct Props {
    vtbl: *const PropsVtbl,
    owner: *mut Session,
}

unsafe extern "system" fn ses_qi(this: Obj, iid: *const GUID, out: *mut Obj) -> HRESULT {
    let s = this as *mut Session;
    if *iid == IID_XBTP_OPERATION_SESSION_PROPERTIES {
        ses_add_ref(this);
        *out = &mut (*s).props as *mut Props as Obj;
        return S_OK;
    }
    *out = null_mut();
    0x8000_4002u32 as i32
}

unsafe extern "system" fn ses_add_ref(this: Obj) -> u32 {
    (*(this as *mut Session)).refs.fetch_add(1, Ordering::AcqRel) + 1
}

unsafe extern "system" fn ses_release(this: Obj) -> u32 {
    (*(this as *mut Session)).refs.fetch_sub(1, Ordering::AcqRel) - 1
}

unsafe extern "system" fn ses_send_more(this: Obj, m: Obj) -> HRESULT {
    let s = &*(this as *mut Session);
    let mut n = 0u32;
    let mut p: *mut u8 = null_mut();
    msg_get_json_body(m, &mut n, &mut p);
    let json = if p.is_null() { String::new() } else { String::from_utf8(core::slice::from_raw_parts(p, n as usize).iter().copied().take_while(|c| *c != 0).collect()).unwrap() };
    let f: unsafe extern "system" fn(Obj, *mut u32, *mut *mut u8) -> HRESULT = transmute(slot(m, 4));
    let mut bn = 0u32;
    let mut bp: *mut u8 = null_mut();
    f(m, &mut bn, &mut bp);
    let binary = if bp.is_null() { Vec::new() } else { core::slice::from_raw_parts(bp, bn as usize).to_vec() };
    let mut v = s.sent.lock().unwrap();
    v.push(Sent { json, binary });
    if v.len() as u32 >= s.fail_from {
        s.fail_hr
    } else {
        S_OK
    }
}

unsafe extern "system" fn pr_qi(this: Obj, iid: *const GUID, out: *mut Obj) -> HRESULT {
    ses_qi((*(this as *mut Props)).owner as Obj, iid, out)
}

unsafe extern "system" fn pr_add_ref(this: Obj) -> u32 {
    ses_add_ref((*(this as *mut Props)).owner as Obj)
}

unsafe extern "system" fn pr_release(this: Obj) -> u32 {
    ses_release((*(this as *mut Props)).owner as Obj)
}

unsafe extern "system" fn pr_get_property(this: Obj, name: *const u16, out: *mut BSTR) -> HRESULT {
    let s = &*(*(this as *mut Props)).owner;
    s.address_asked.store(true, Ordering::Release);
    assert_eq!(String::from_utf16(core::slice::from_raw_parts(name, wlen(name))).unwrap(), "address");
    *out = SysAllocString(wz("127.0.0.1").as_ptr());
    S_OK
}

static SESSION_VTBL: SessionVtbl = SessionVtbl { qi: ses_qi, add_ref: ses_add_ref, release: ses_release, send_more: ses_send_more };
static PROPS_VTBL: PropsVtbl = PropsVtbl { qi: pr_qi, add_ref: pr_add_ref, release: pr_release, get_property: pr_get_property };

fn new_session(fail_from: u32, fail_hr: HRESULT) -> Box<Session> {
    let mut b = Box::new(Session {
        vtbl: &SESSION_VTBL,
        props: Props { vtbl: &PROPS_VTBL, owner: null_mut() },
        refs: AtomicU32::new(1),
        fail_from,
        fail_hr,
        sent: Mutex::new(Vec::new()),
        address_asked: AtomicBool::new(false),
    });
    let p: *mut Session = &mut *b;
    b.props.owner = p;
    b
}

unsafe fn new_message(json: Option<&str>) -> Ptr {
    let mut m = Ptr::null();
    assert_eq!(xbtp::xbtp2_ext::XbtpCreateMessage(&xbtp::xbtp2_ext::IID_XBTP_MESSAGE, m.out()), S_OK);
    if let Some(j) = json {
        let mut b = j.as_bytes().to_vec();
        b.push(0);
        let f: unsafe extern "system" fn(Obj, u32, *const u8) -> HRESULT = transmute(slot(m.0, 6));
        assert_eq!(f(m.0, b.len() as u32, b.as_ptr()), S_OK);
    }
    m
}

unsafe fn parse(text: &str) -> Ptr {
    let mut b = text.as_bytes().to_vec();
    b.push(0);
    let mut o = Ptr::null();
    assert_eq!(xbtp::xbtp2_json::XbtpGetJsonObjectFromUtf8(b.as_ptr(), o.out()), S_OK, "{}", text);
    o
}

unsafe fn num(o: &Ptr, k: &str) -> Option<f64> {
    let mut d = 0f64;
    if obj_get_number(o.0, wz(k).as_ptr(), &mut d) == S_OK {
        Some(d)
    } else {
        None
    }
}

unsafe fn text(o: &Ptr, k: &str) -> Option<String> {
    let mut s = Bstr::null();
    if obj_get_string(o.0, wz(k).as_ptr(), s.out()) == S_OK && !s.0.is_null() {
        Some(String::from_utf16(core::slice::from_raw_parts(s.0, SysStringLen(s.0) as usize)).unwrap())
    } else {
        None
    }
}

unsafe fn call_register(session: &Session, json: &str) -> HRESULT {
    let req = new_message(Some(json));
    let rep = new_message(None);
    invoke(REGISTER_FOR_EVENTS_ID, session as *const Session as usize, req.0 as usize, rep.0 as usize)
}

unsafe fn handlers() -> (Obj, Vec<(u32, Obj)>) {
    let mut md: Obj = null_mut();
    assert_eq!(XtfOrdinal_4(&EVENTSERVER_EXTENSION_ID, &IID_XBTP_EXTENSION_METADATA_IMPL, &mut md), S_OK);
    let count: unsafe extern "system" fn(Obj, *mut u32) -> HRESULT = transmute(slot(md, 3));
    let get: unsafe extern "system" fn(Obj, u32, *mut Obj) -> HRESULT = transmute(slot(md, 4));
    let mut n = 0u32;
    assert_eq!(count(md, &mut n), S_OK);
    let mut hs = vec![null_mut(); n as usize];
    assert_eq!(get(md, n * 8, hs.as_mut_ptr()), S_OK);
    let mut out = Vec::new();
    for h in hs {
        let id_of: unsafe extern "system" fn(Obj, *mut u32) -> HRESULT = transmute(slot(h, 3));
        let mut id = 0u32;
        assert_eq!(id_of(h, &mut id), S_OK);
        out.push((id, h));
    }
    (md, out)
}

unsafe fn invoke(id: u32, a: usize, b: usize, c: usize) -> HRESULT {
    let (md, hs) = handlers();
    let h = hs.iter().find(|(i, _)| *i == id).expect("handler registered").1;
    add_ref(h);
    release(md);
    let f: unsafe extern "system" fn(Obj, usize, usize, usize) -> HRESULT = transmute(slot(h, 4));
    let hr = f(h, a, b, c);
    release(h);
    hr
}

#[test]
fn entry_point_checks_ids_and_registers_two_handlers() {
    unsafe {
        let mut out: Obj = null_mut();
        let other = GUID::new(1, 2, 3, [0; 8]);
        assert_eq!(XtfOrdinal_4(&other, &IID_XBTP_EXTENSION_METADATA_IMPL, &mut out), E_INVALIDARG);
        assert_eq!(XtfOrdinal_4(&EVENTSERVER_EXTENSION_ID, &other, &mut out), E_INVALIDARG);
        assert!(out.is_null());
        let (md, hs) = handlers();
        let mut ids: Vec<u32> = hs.iter().map(|(i, _)| *i).collect();
        ids.sort();
        assert_eq!(ids, vec![REGISTER_FOR_EVENTS_ID, QUERY_PROTOCOL_LEVEL_ID]);
        release(md);
    }
}

#[test]
fn query_protocol_level_replies_level_2_and_result() {
    unsafe {
        let req = new_message(None);
        let rep = xbtp_reply();
        assert_eq!(invoke(QUERY_PROTOCOL_LEVEL_ID, 0, req.0 as usize, rep.0 as usize), S_OK);
        let mut n = 0u32;
        let mut p: *mut u8 = null_mut();
        assert_eq!(msg_get_json_body(rep.0, &mut n, &mut p), S_OK);
        let t = String::from_utf8(core::slice::from_raw_parts(p, n as usize).iter().copied().take_while(|c| *c != 0).collect()).unwrap();
        let o = parse(&t);
        assert_eq!(num(&o, "ProtocolLevel"), Some(2.0));
        assert_eq!(num(&o, "Result"), Some(0.0));
    }
}

#[repr(C)]
struct ReplyVtbl {
    qi: unsafe extern "system" fn(Obj, *const GUID, *mut Obj) -> HRESULT,
    add_ref: unsafe extern "system" fn(Obj) -> u32,
    release: unsafe extern "system" fn(Obj) -> u32,
    get_json_body: unsafe extern "system" fn(Obj, *mut u32, *mut *mut u8) -> HRESULT,
    get_binary_body: unsafe extern "system" fn(Obj, *mut u32, *mut *mut u8) -> HRESULT,
    set_json_body: unsafe extern "system" fn(Obj, u32, *const u8) -> HRESULT,
    set_json_body_w: unsafe extern "system" fn(Obj, *const u16) -> HRESULT,
}

#[repr(C)]
struct Reply {
    vtbl: *const ReplyVtbl,
    refs: AtomicU32,
    inner: Obj,
}

unsafe extern "system" fn rp_qi(this: Obj, _iid: *const GUID, out: *mut Obj) -> HRESULT {
    *out = this;
    rp_add_ref(this);
    S_OK
}

unsafe extern "system" fn rp_add_ref(this: Obj) -> u32 {
    (*(this as *mut Reply)).refs.fetch_add(1, Ordering::AcqRel) + 1
}

unsafe extern "system" fn rp_release(this: Obj) -> u32 {
    let n = (*(this as *mut Reply)).refs.fetch_sub(1, Ordering::AcqRel) - 1;
    if n == 0 {
        let b = Box::from_raw(this as *mut Reply);
        release(b.inner);
    }
    n
}

unsafe extern "system" fn rp_get_json_body(this: Obj, n: *mut u32, b: *mut *mut u8) -> HRESULT {
    msg_get_json_body((*(this as *mut Reply)).inner, n, b)
}

unsafe extern "system" fn rp_get_binary_body(this: Obj, n: *mut u32, b: *mut *mut u8) -> HRESULT {
    let i = (*(this as *mut Reply)).inner;
    let f: unsafe extern "system" fn(Obj, *mut u32, *mut *mut u8) -> HRESULT = transmute(slot(i, 4));
    f(i, n, b)
}

unsafe extern "system" fn rp_set_json_body(this: Obj, n: u32, b: *const u8) -> HRESULT {
    let i = (*(this as *mut Reply)).inner;
    let f: unsafe extern "system" fn(Obj, u32, *const u8) -> HRESULT = transmute(slot(i, 5));
    f(i, n, b)
}

unsafe extern "system" fn rp_set_json_body_w(this: Obj, w: *const u16) -> HRESULT {
    msg_set_json_body_w((*(this as *mut Reply)).inner, w)
}

static REPLY_VTBL: ReplyVtbl = ReplyVtbl { qi: rp_qi, add_ref: rp_add_ref, release: rp_release, get_json_body: rp_get_json_body, get_binary_body: rp_get_binary_body, set_json_body: rp_set_json_body, set_json_body_w: rp_set_json_body_w };

unsafe fn xbtp_reply() -> Ptr {
    let i = new_message(None).detach();
    Ptr(Box::into_raw(Box::new(Reply { vtbl: &REPLY_VTBL, refs: AtomicU32::new(1), inner: i })) as Obj)
}

#[test]
fn register_rejects_protocol_above_current_before_tracing() {
    let _g = REGISTER_LOCK.read().unwrap_or_else(|e| e.into_inner());
    unsafe {
        let s = new_session(1, E_FAIL);
        let before = LOGGER_COUNTER.load(Ordering::Acquire);
        let hr = call_register(&s, &format!("{{\"ExclusiveETWProvider\":\"{}\",\"ProtocolLevel\":2.5}}", TEST_PROVIDER_TEXT));
        assert_eq!(hr, E_PROTOCOL_VERSION_NOT_SUPPORTED);
        assert!(s.sent.lock().unwrap().is_empty());
        assert_eq!(LOGGER_COUNTER.load(Ordering::Acquire), before);
        assert_eq!(s.refs.load(Ordering::Acquire), 1);
    }
}

#[test]
fn register_rejects_bad_provider_guid_with_iidfromstring_error() {
    let _g = REGISTER_LOCK.read().unwrap_or_else(|e| e.into_inner());
    unsafe {
        let s = new_session(1, E_FAIL);
        let mut g = GUID::default();
        let expect = IIDFromString(wz("not-a-guid").as_ptr(), &mut g);
        assert!(expect < 0);
        let hr = call_register(&s, &format!("{{\"ExclusiveETWProvider\":\"{},not-a-guid\"}}", TEST_PROVIDER_TEXT));
        assert_eq!(hr, expect);
        let hr = call_register(&s, &format!("{{\"ExclusiveETWProvider\":\"{},\"}}", TEST_PROVIDER_TEXT));
        assert!(hr < 0);
        assert!(s.sent.lock().unwrap().is_empty());
    }
}

#[test]
fn provider_list_splits_on_commas() {
    unsafe {
        let b = SysAllocString(wz(&format!("{},{{2CB15D1D-5FC1-11D2-ABE1-00A0C911F518}}", TEST_PROVIDER_TEXT)).as_ptr());
        let mut v = Vec::new();
        assert_eq!(parse_provider_list(b, &mut v), S_OK);
        SysFreeString(b);
        assert_eq!(v, vec![TEST_PROVIDER, etw::SPECIAL_PROVIDER]);
    }
}

#[test]
fn register_enforces_fifty_session_limit_with_caller_address() {
    let _g = REGISTER_LOCK.write().unwrap_or_else(|e| e.into_inner());
    unsafe {
        let s = new_session(1, E_FAIL);
        let saved = ACTIVE_SESSIONS.swap(MAX_SESSIONS, Ordering::AcqRel);
        let hr = call_register(&s, "{}");
        let after = ACTIVE_SESSIONS.swap(saved, Ordering::AcqRel);
        assert_eq!(hr, E_NO_MORE_SESSIONS);
        assert_eq!(after, MAX_SESSIONS);
        assert!(s.address_asked.load(Ordering::Acquire));
        assert!(s.sent.lock().unwrap().is_empty());
        assert_eq!(s.refs.load(Ordering::Acquire), 1);
    }
}

#[repr(C)]
struct FakeSourceVtbl {
    dtor: unsafe extern "C" fn(*mut FakeSource, u32) -> *mut FakeSource,
    initialize: unsafe extern "C" fn(*mut FakeSource, *mut SharedPtr) -> HRESULT,
    shutdown: unsafe extern "C" fn(*mut FakeSource) -> HRESULT,
}

#[repr(C)]
struct FakeSource {
    vtbl: *const FakeSourceVtbl,
}

static FAKE_INIT: AtomicU32 = AtomicU32::new(0);
static FAKE_SHUTDOWN: AtomicU32 = AtomicU32::new(0);
static FAKE_DTOR: AtomicU32 = AtomicU32::new(0);

unsafe extern "C" fn fake_dtor(this: *mut FakeSource, flags: u32) -> *mut FakeSource {
    FAKE_DTOR.fetch_add(1, Ordering::AcqRel);
    if flags & 1 != 0 {
        drop(Box::from_raw(this));
    }
    this
}

unsafe extern "C" fn fake_initialize(_this: *mut FakeSource, d: *mut SharedPtr) -> HRESULT {
    FAKE_INIT.fetch_add(1, Ordering::AcqRel);
    let held = Strong(*d);
    let disp = held.0.ptr as *mut Dispatcher;
    let bytes = [1u8, 2, 3, 4, 5];
    ((*(*disp).vtbl).queue_binary_event)(disp, wz("FakeEvent").as_ptr(), bytes.len() as u32, bytes.as_ptr())
}

unsafe extern "C" fn fake_shutdown(_this: *mut FakeSource) -> HRESULT {
    FAKE_SHUTDOWN.fetch_add(1, Ordering::AcqRel);
    S_OK
}

static FAKE_VTBL: FakeSourceVtbl = FakeSourceVtbl { dtor: fake_dtor, initialize: fake_initialize, shutdown: fake_shutdown };

#[repr(C)]
struct FuncImplVtbl {
    a: usize,
    b: usize,
    do_call: unsafe extern "C" fn(*mut FuncImpl) -> *mut c_void,
}

#[repr(C)]
struct FuncImpl {
    vtbl: *const FuncImplVtbl,
}

unsafe extern "C" fn factory(_f: *mut FuncImpl) -> *mut c_void {
    Box::into_raw(Box::new(FakeSource { vtbl: &FAKE_VTBL })) as *mut c_void
}

static FUNC_VTBL: FuncImplVtbl = FuncImplVtbl { a: 0, b: 0, do_call: factory };

#[test]
fn register_without_exclusive_provider_runs_registered_event_sources() {
    let _g = REGISTER_LOCK.write().unwrap_or_else(|e| e.into_inner());
    unsafe {
        let map = GetEventSourceMap();
        let head = (*map).head;
        let node = Box::into_raw(Box::new([0u8; 0x80])) as *mut TreeNode;
        let func = Box::into_raw(Box::new(FuncImpl { vtbl: &FUNC_VTBL }));
        (*node).left = head;
        (*node).right = head;
        (*node).parent = head;
        (*node).color = 1;
        (*node).isnil = 0;
        *((node as *mut u8).add(0x78) as *mut *mut FuncImpl) = func;
        (*head).parent = node;
        (*head).left = node;
        (*head).right = node;
        (*map).size = 1;
        let s = new_session(1, 0x8007_2746u32 as i32);
        let t = Instant::now();
        let hr = call_register(&s, "{\"TraceLevel\":3}");
        (*head).parent = head;
        (*head).left = head;
        (*head).right = head;
        (*map).size = 0;
        drop(Box::from_raw(node as *mut [u8; 0x80]));
        drop(Box::from_raw(func));
        assert_eq!(hr, 0x8007_2746u32 as i32);
        assert!(t.elapsed() < Duration::from_secs(10));
        assert_eq!(FAKE_INIT.load(Ordering::Acquire), 1);
        assert_eq!(FAKE_SHUTDOWN.load(Ordering::Acquire), 1);
        assert_eq!(FAKE_DTOR.load(Ordering::Acquire), 1);
        let sent = s.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        let o = parse(&sent[0].json);
        assert_eq!(text(&o, "EventType").as_deref(), Some("FakeEvent"));
        let mut d = Ptr::null();
        let get_obj: unsafe extern "system" fn(Obj, *const u16, *mut Obj) -> HRESULT = transmute(slot(o.0, 8));
        assert_eq!(get_obj(o.0, wz("EventData").as_ptr(), d.out()), S_OK);
        assert_eq!(sent[0].binary, vec![1, 2, 3, 4, 5]);
        assert_eq!(s.refs.load(Ordering::Acquire), 1);
        assert_eq!(ACTIVE_SESSIONS.load(Ordering::Acquire), 0);
    }
}

struct StopSessions;

impl Drop for StopSessions {
    fn drop(&mut self) {
        let names: Vec<Vec<u16>> = etw::TRACE_RESULTS.lock().unwrap_or_else(|e| e.into_inner()).iter().map(|(n, _)| n.clone()).collect();
        for n in names {
            let mut props = vec![0u8; etw::PROPS_SIZE];
            props[0..4].copy_from_slice(&(etw::PROPS_SIZE as u32).to_le_bytes());
            props[0x74..0x78].copy_from_slice(&0x78u32.to_le_bytes());
            unsafe { ControlTraceW(0, n.as_ptr(), props.as_mut_ptr(), etw::EVENT_TRACE_CONTROL_STOP) };
        }
    }
}

#[test]
fn register_exclusive_provider_streams_etw_events_or_console_access_error() {
    let _g = REGISTER_LOCK.read().unwrap_or_else(|e| e.into_inner());
    let _stop = StopSessions;
    unsafe {
        let mut reg = 0u64;
        assert_eq!(EventRegister(&TEST_PROVIDER, core::ptr::null(), core::ptr::null(), &mut reg), 0);
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let st = stop.clone();
        let writer = std::thread::spawn(move || {
            let desc = EventDescriptor { id: 0x1234, version: 1, channel: 0, level: 4, opcode: 0, task: 7, keyword: 0 };
            let payload = 0x1122_3344_5566_7788u64;
            while !st.load(Ordering::Acquire) {
                let dd = DataDescriptor { ptr: &payload as *const u64 as u64, size: 8, reserved: 0 };
                EventWrite(reg, &desc, 1, &dd);
                std::thread::sleep(Duration::from_millis(50));
            }
        });
        let s = new_session(1, 0x8007_2746u32 as i32);
        let before = LOGGER_COUNTER.load(Ordering::Acquire);
        let t = Instant::now();
        let hr = call_register(&s, &format!("{{\"ExclusiveETWProvider\":\"{}\",\"TraceLevel\":4,\"ProtocolLevel\":2}}", TEST_PROVIDER_TEXT));
        let elapsed = t.elapsed();
        stop.store(true, Ordering::Release);
        writer.join().unwrap();
        EventUnregister(reg);
        assert_eq!(hr, 0x8007_2746u32 as i32);
        assert!(LOGGER_COUNTER.load(Ordering::Acquire) > before);
        let results = etw::TRACE_RESULTS.lock().unwrap().clone();
        let trace_hr = results.iter().rev().find(|(n, _)| String::from_utf16_lossy(n).starts_with(etw::LOGGER_NAME_FORMAT)).map(|(_, h)| *h);
        let sent = s.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        let o = parse(&sent[0].json);
        if trace_hr == Some(hr_from_win32(5)) {
            assert_eq!(text(&o, "EventType").as_deref(), Some("KeepAliveEvent"));
            assert!(elapsed >= Duration::from_millis(29_500), "{:?}", elapsed);
            assert!(sent[0].binary.is_empty());
        } else {
            assert_eq!(text(&o, "EventType").as_deref(), Some("ETWEvent"), "trace result {:x?} json {}", trace_hr, sent[0].json);
            let mut d = Ptr::null();
            let get_obj: unsafe extern "system" fn(Obj, *const u16, *mut Obj) -> HRESULT = transmute(slot(o.0, 8));
            assert_eq!(get_obj(o.0, wz("EventData").as_ptr(), d.out()), S_OK);
            assert_eq!(text(&d, "ExclusiveETWProvider").as_deref(), Some(TEST_PROVIDER_TEXT));
            let b = &sent[0].binary;
            assert!(b.len() >= 0x78);
            assert_eq!(&b[0x18..0x28], core::slice::from_raw_parts(&TEST_PROVIDER as *const GUID as *const u8, 16));
            assert_eq!(u16::from_le_bytes([b[0x28], b[0x29]]), 0x1234);
            assert_eq!(b[0x2c], 4);
            assert_eq!(u16::from_le_bytes([b[0x2e], b[0x2f]]), 7);
            assert_eq!(u16::from_le_bytes([b[0x56], b[0x57]]), 8);
            assert_eq!(u64::from_le_bytes(b[0x60..0x68].try_into().unwrap()), 0x70);
            assert_eq!(u64::from_le_bytes(b[0x70..0x78].try_into().unwrap()), 0x1122_3344_5566_7788);
        }
    }
}

unsafe fn live_dispatcher(session: &Session) -> Strong {
    let d = Strong(dispatcher::make_dispatcher());
    assert_eq!(dispatcher::initialize(d.0.ptr as *mut Dispatcher, session as *const Session as Obj), S_OK);
    d
}

fn synthetic_record(user: &[u8], ext: &mut Vec<[u8; 16]>, ext_data: &[Vec<u8>]) -> Vec<u8> {
    let mut r = vec![0u8; 0x70];
    r[4..6].copy_from_slice(&0x0240u16.to_le_bytes());
    r[6..8].copy_from_slice(&0x0001u16.to_le_bytes());
    r[0x18..0x28].copy_from_slice(unsafe { core::slice::from_raw_parts(&TEST_PROVIDER as *const GUID as *const u8, 16) });
    r[0x28..0x2a].copy_from_slice(&0x0042u16.to_le_bytes());
    r[0x2a] = 3;
    r[0x2b] = 0x10;
    r[0x2c] = 4;
    r[0x2d] = 1;
    r[0x2e..0x30].copy_from_slice(&0x0099u16.to_le_bytes());
    r[0x54..0x56].copy_from_slice(&(ext.len() as u16).to_le_bytes());
    r[0x56..0x58].copy_from_slice(&(user.len() as u16).to_le_bytes());
    for (k, e) in ext.iter_mut().enumerate() {
        e[6..8].copy_from_slice(&(ext_data[k].len() as u16).to_le_bytes());
        e[8..16].copy_from_slice(&(ext_data[k].as_ptr() as u64).to_le_bytes());
    }
    r[0x58..0x60].copy_from_slice(&(if ext.is_empty() { 0 } else { ext.as_ptr() as u64 }).to_le_bytes());
    r[0x60..0x68].copy_from_slice(&(user.as_ptr() as u64).to_le_bytes());
    r
}

#[test]
fn protocol1_event_queues_descriptor_fields_tl_schema_and_record() {
    unsafe {
        let s = new_session(1, E_FAIL);
        let d = live_dispatcher(&s);
        let src = etw::new_source(etw::logger_name(9999), 4, 1, Vec::new());
        incweak(d.0.ctrl);
        (*src).dispatcher = d.0;
        (*src).running.store(1, Ordering::Release);
        let user = [9u8, 8, 7];
        let tl = vec![b"\x10\x00\x00MyEvent\x00".to_vec()];
        let mut ext = vec![[0u8; 16]];
        ext[0][2..4].copy_from_slice(&etw::EXT_TYPE_EVENT_SCHEMA_TL.to_le_bytes());
        let rec = synthetic_record(&user, &mut ext, &tl);
        let mut full = rec.clone();
        full[0x68..0x70].copy_from_slice(&(src as u64).to_le_bytes());
        etw::event_record_callback(full.as_mut_ptr());
        (*src).running.store(0, Ordering::Release);
        let disp = &*(d.0.ptr as *mut Dispatcher);
        let q = core::mem::take(&mut *disp.queue.lock().unwrap());
        assert_eq!(q.len(), 1);
        let ev = &q[0];
        assert_eq!(String::from_utf16_lossy(&ev.name[..ev.name.len() - 1]), "ETWEvent");
        assert_eq!(ev.binary.len(), 0x73);
        assert_eq!(&ev.binary[..0x68], &full[..0x68]);
        assert_eq!(&ev.binary[0x70..], &user);
        let j = Ptr(ev.data.0);
        add_ref(j.0);
        assert_eq!(num(&j, "EventFlags"), Some(0x240 as f64));
        assert_eq!(num(&j, "EventProperty"), Some(1.0));
        assert_eq!(num(&j, "DescriptorId"), Some(0x42 as f64));
        assert_eq!(num(&j, "DescriptorVersion"), Some(3.0));
        assert_eq!(num(&j, "DescriptorChannel"), Some(16.0));
        assert_eq!(num(&j, "EventDescriptorLevel"), Some(4.0));
        assert_eq!(num(&j, "DescriptorOpcode"), Some(1.0));
        assert_eq!(num(&j, "DescriptorTask"), Some(0x99 as f64));
        let mut w = Bstr::null();
        assert_eq!(obj_stringify(j.0, w.out()), S_OK);
        let all = String::from_utf16_lossy(core::slice::from_raw_parts(w.0, SysStringLen(w.0) as usize));
        assert_eq!(text(&j, "Schema_TL").as_deref(), Some("MyEvent"), "{}", all);
        drop(q);
        drop(j);
        drop(Box::from_raw(src as *mut EtwSource));
    }
}

#[test]
fn protocol2_block_relocates_user_data_and_extended_items_to_offsets() {
    unsafe {
        let user = [1u8, 2, 3, 4];
        let data = vec![vec![0xaau8; 5], vec![0xbbu8; 2]];
        let mut ext = vec![[0u8; 16], [0u8; 16]];
        ext[0][2] = 1;
        ext[1][2] = 5;
        let rec = synthetic_record(&user, &mut ext, &data);
        let b = etw::protocol2_block(rec.as_ptr());
        assert_eq!(b.len(), 0x70 + 2 * 16 + 4 + 5 + 2);
        assert_eq!(&b[..0x58], &rec[..0x58]);
        assert_eq!(u64::from_le_bytes(b[0x60..0x68].try_into().unwrap()), 0x70);
        assert_eq!(u64::from_le_bytes(b[0x68..0x70].try_into().unwrap()), 0);
        assert_eq!(&b[0x70..0x74], &user);
        assert_eq!(u64::from_le_bytes(b[0x58..0x60].try_into().unwrap()), 0x74);
        assert_eq!(b[0x74 + 2], 1);
        assert_eq!(u64::from_le_bytes(b[0x74 + 8..0x74 + 16].try_into().unwrap()), 0x94);
        assert_eq!(u64::from_le_bytes(b[0x84 + 8..0x84 + 16].try_into().unwrap()), 0x99);
        assert_eq!(&b[0x94..0x99], &[0xaa; 5]);
        assert_eq!(&b[0x99..0x9b], &[0xbb; 2]);
    }
}

#[test]
fn dispatcher_rejects_bad_events_and_sends_queued_binary_events() {
    unsafe {
        let s = new_session(2, E_FAIL);
        let d = live_dispatcher(&s);
        let disp = d.0.ptr as *mut Dispatcher;
        let v = &*(*disp).vtbl;
        assert_eq!((v.queue_json_with_binary_event)(disp, core::ptr::null(), null_mut(), 0, core::ptr::null()), E_INVALIDARG);
        assert_eq!((v.queue_json_with_binary_event)(disp, wz("").as_ptr(), null_mut(), 0, core::ptr::null()), E_INVALIDARG);
        assert_eq!((v.queue_json_with_binary_event)(disp, wz("A").as_ptr(), null_mut(), 3, core::ptr::null()), E_INVALIDARG);
        assert_eq!((v.queue_binary_event)(disp, wz("First").as_ptr(), 2, [7u8, 8].as_ptr()), S_OK);
        let mut j = Ptr::null();
        assert_eq!(xbtp::xbtp2_json::XbtpCreateJsonObject(j.out()), S_OK);
        assert_eq!(obj_set_number(j.0, wz("k").as_ptr(), 5.0), S_OK);
        assert_eq!((v.queue_json_event)(disp, wz("Second").as_ptr(), j.0), S_OK);
        assert_eq!(dispatcher::handle_events(disp), E_FAIL);
        let sent = s.sent.lock().unwrap();
        assert_eq!(sent.len(), 2);
        assert_eq!(text(&parse(&sent[0].json), "EventType").as_deref(), Some("First"));
        assert_eq!(sent[0].binary, vec![7, 8]);
        assert_eq!(text(&parse(&sent[1].json), "EventType").as_deref(), Some("Second"));
        assert!(sent[1].binary.is_empty());
        let fresh = Strong(dispatcher::make_dispatcher());
        let fd = fresh.0.ptr as *mut Dispatcher;
        assert_eq!(((*(*fd).vtbl).queue_json_event)(fd, wz("X").as_ptr(), null_mut()), E_UNEXPECTED);
        assert_eq!(dispatcher::handle_events(fd), E_UNEXPECTED);
        assert_eq!(dispatcher::initialize(fd, null_mut()), E_INVALIDARG);
    }
}

#[test]
fn realtime_properties_follow_console_layout() {
    unsafe {
        let name = etw::logger_name(3);
        let mut p = vec![0u8; etw::PROPS_SIZE];
        etw::fill_properties(&mut p, etw::EVENT_TRACE_REAL_TIME_MODE, &name, 0x200, 0x1e, 0x32, 0);
        etw::realtime_properties_layout(&mut p);
        let g = |o: usize| u32::from_le_bytes(p[o..o + 4].try_into().unwrap());
        assert_eq!(g(0), 0x338);
        assert_eq!(g(0x28), 3);
        assert_eq!(g(0x2c), 0x20000);
        assert_eq!((g(0x30), g(0x34), g(0x38)), (0x200, 0x1e, 0x32));
        assert_eq!(g(0x40), 0x0200_0110);
        assert_eq!(g(0x44), 200);
        assert_eq!(g(0x48), 0x80ff_02c0);
        assert_eq!(g(0x70), 0);
        assert_eq!(g(0x74), 0x78);
        let n: Vec<u16> = p[0x78..0x78 + 2 * (name.len() - 1)].chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        assert_eq!(String::from_utf16(&n).unwrap(), "XtfEventServerETWLogger_3");
        assert_eq!((g(0x2c0), g(0x2c4), g(0x308), g(0x32c)), (0x3001e, 0x30011, 0x10009, 0x20003));
        let long = wz("XtfEventServerETWLogger_123456789");
        let mut h = 0u64;
        assert_eq!(etw::start_etw_trace_generic(&long, &[], &mut h), E_INVALIDARG);
        assert_eq!(String::from_utf16(&etw::trace_file_path(&name)[..]).unwrap(), "d:\\xbperf\\XtfEventServerETWLogger_3_u.etl\0");
    }
}
