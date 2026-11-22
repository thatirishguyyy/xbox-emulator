use crate::com::*;
use crate::dispatcher::{self, Dispatcher};
use crate::etw;
use crate::imports::{self, chk};
use crate::shared::*;
use crate::sys::*;
use crate::{EventSourceMap, TreeNode};
use core::ffi::c_void;
use core::ptr::null_mut;
use core::sync::atomic::{AtomicI32, AtomicU32, Ordering};

pub const EVENTSERVER_EXTENSION_ID: GUID = GUID::new(0x749fdefa, 0xcf4a, 0x4204, [0xbc, 0x04, 0x90, 0x36, 0xa6, 0x1c, 0x4d, 0xf6]);
pub const IID_XBTP_EXTENSION_METADATA_IMPL: GUID = GUID::new(0xe2b6c063, 0x7d82, 0x414a, [0xbe, 0x86, 0x67, 0xed, 0x1b, 0x44, 0x77, 0x97]);
pub const IID_XBTP_OPERATION_SESSION_PROPERTIES: GUID = GUID::new(0xf6e48feb, 0x4135, 0x4a7d, [0x9d, 0x3f, 0x77, 0x64, 0xef, 0xf2, 0x0c, 0x80]);
pub const REGISTER_FOR_EVENTS_ID: u32 = 1;
pub const QUERY_PROTOCOL_LEVEL_ID: u32 = 2;
pub const CURRENT_PROTOCOL_LEVEL: f64 = 2.0;
pub const DEFAULT_PROTOCOL_LEVEL: f64 = 1.0;
pub const DEFAULT_TRACE_LEVEL: f64 = 5.0;
pub const MAX_SESSIONS: i32 = 0x32;

pub static ACTIVE_SESSIONS: AtomicI32 = AtomicI32::new(0);
pub static LOGGER_COUNTER: AtomicU32 = AtomicU32::new(0);

const F_REG: &[u8] = b"XtfEventServer::RegisterForEventsHandler\0";
const F_QPL: &[u8] = b"XtfEventServer::QueryProtocolLevel\0";

pub unsafe fn extension_entry_point(extension: *const GUID, iid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    let f = b"XbtpServerEntryPoint\0";
    if *extension != EVENTSERVER_EXTENSION_ID {
        return chk(E_INVALIDARG, b"E_INVALIDARG\0", f, 16);
    }
    if *iid != IID_XBTP_EXTENSION_METADATA_IMPL {
        return chk(E_INVALIDARG, b"E_INVALIDARG\0", f, 20);
    }
    let i = match imports::get() {
        Ok(i) => i,
        Err(e) => return e,
    };
    let mut md = Ptr::null();
    let hr = chk((i.create_extension_metadata)(&IID_XBTP_EXTENSION_METADATA_IMPL, md.out()), b"XbtpCreateExtensionMetadata(IID_PPV_ARGS(&metadata))\0", f, 24);
    if hr < 0 {
        return hr;
    }
    let hr = chk(md_add_function(md.0, REGISTER_FOR_EVENTS_ID, register_for_events_handler), b"metadata->AddFunction(RegisterForEventsId, RegisterForEventsHandler)\0", f, 26);
    if hr < 0 {
        return hr;
    }
    let hr = chk(md_add_function(md.0, QUERY_PROTOCOL_LEVEL_ID, query_protocol_level), b"metadata->AddFunction(QueryProtocolLevelId, QueryProtocolLevel)\0", f, 27);
    if hr < 0 {
        return hr;
    }
    *out = md.detach();
    S_OK
}

pub unsafe extern "system" fn query_protocol_level(_session: usize, _received: usize, reply: usize) -> HRESULT {
    let i = match imports::get() {
        Ok(i) => i,
        Err(e) => return e,
    };
    let mut body = Ptr::null();
    let mut hr = chk((i.create_json_object)(body.out()), b"XbtpCreateJsonObject(&responseBody)\0", F_QPL, 19);
    if hr >= 0 {
        hr = if body.0.is_null() { E_INVALIDARG } else { obj_set_number(body.0, wz("ProtocolLevel").as_ptr(), CURRENT_PROTOCOL_LEVEL) };
        chk(hr, b"XtfSetJsonProperty(responseBody, ProtocolLevel, c_CurrentProtocolLevel)\0", F_QPL, 20);
    }
    if body.0.is_null() {
        return hr;
    }
    obj_set_number(body.0, wz("Result").as_ptr(), hr as f64);
    let reply = reply as Obj;
    if reply.is_null() {
        return E_INVALIDARG;
    }
    let mut w = Bstr::null();
    let r = obj_stringify(body.0, w.out());
    if r < 0 {
        return r;
    }
    reply_set_json_body_w(reply, w.0)
}

pub unsafe fn deserialize_json_body(message: Obj, body: *mut Obj) -> HRESULT {
    if message.is_null() || body.is_null() {
        return E_INVALIDARG;
    }
    let i = match imports::get() {
        Ok(i) => i,
        Err(e) => return e,
    };
    let mut len = 0u32;
    let mut unparsed: *mut u8 = null_mut();
    let hr = msg_get_json_body(message, &mut len, &mut unparsed);
    if hr < 0 {
        return hr;
    }
    let hr = if unparsed.is_null() { (i.create_json_object)(body) } else { (i.get_json_object_from_utf8)(unparsed, body) };
    if hr < 0 {
        hr
    } else {
        S_OK
    }
}

pub unsafe fn parse_provider_list(list: BSTR, out: &mut Vec<GUID>) -> HRESULT {
    let mut p = list;
    while !p.is_null() {
        let mut next: *mut u16 = null_mut();
        let mut q = p;
        while *q != 0 {
            if *q == b',' as u16 {
                *q = 0;
                next = q.add(1);
                break;
            }
            q = q.add(1);
        }
        let mut g = GUID::default();
        let hr = chk(IIDFromString(p, &mut g), b"IIDFromString(guidName, &guid)\0", F_REG, 74);
        if hr < 0 {
            return hr;
        }
        out.push(g);
        p = next;
    }
    S_OK
}

unsafe fn tree_next(head: *mut TreeNode, n: *mut TreeNode) -> *mut TreeNode {
    if (*(*n).right).isnil == 0 {
        let mut m = (*n).right;
        while (*(*m).left).isnil == 0 {
            m = (*m).left;
        }
        return m;
    }
    let mut c = n;
    let mut p = (*n).parent;
    while p != head && (*p).isnil == 0 && c == (*p).right {
        c = p;
        p = (*p).parent;
    }
    p
}

pub unsafe fn registered_sources(map: *mut EventSourceMap, v: &mut Vec<SharedPtr>) -> HRESULT {
    let head = (*map).head;
    if head.is_null() {
        return S_OK;
    }
    let mut n = (*head).left;
    while n != head && (*n).isnil == 0 {
        let impl_ptr = *((n as *mut u8).add(0x78) as *const *mut c_void);
        if impl_ptr.is_null() {
            return E_UNEXPECTED;
        }
        let call: unsafe extern "C" fn(*mut c_void) -> *mut c_void = core::mem::transmute(*(*(impl_ptr as *const *const usize)).add(2));
        let obj = call(impl_ptr);
        v.push(wrap_object(obj));
        n = tree_next(head, n);
    }
    S_OK
}

struct Sources(Vec<SharedPtr>);

impl Drop for Sources {
    fn drop(&mut self) {
        for s in self.0.drain(..) {
            unsafe { decref(s.ctrl) };
        }
    }
}

unsafe fn source_initialize(s: SharedPtr, d: &Strong) -> HRESULT {
    let f: unsafe extern "C" fn(*mut c_void, *mut SharedPtr) -> HRESULT = core::mem::transmute(*(*(s.ptr as *const *const usize)).add(1));
    let mut arg = d.copy();
    f(s.ptr, &mut arg)
}

unsafe fn source_shutdown(s: SharedPtr) -> HRESULT {
    let f: unsafe extern "C" fn(*mut c_void) -> HRESULT = core::mem::transmute(*(*(s.ptr as *const *const usize)).add(2));
    f(s.ptr)
}

struct SessionGuard;

impl Drop for SessionGuard {
    fn drop(&mut self) {
        ACTIVE_SESSIONS.fetch_sub(1, Ordering::AcqRel);
    }
}

pub unsafe extern "system" fn register_for_events_handler(session: usize, received: usize, _reply: usize) -> HRESULT {
    let session = session as Obj;
    let received = received as Obj;
    let d = Strong(dispatcher::make_dispatcher());
    let mut sources = Sources(Vec::new());
    let mut list = Bstr::null();
    let mut req = Ptr::null();
    let n = ACTIVE_SESSIONS.fetch_add(1, Ordering::AcqRel) + 1;
    let _g = SessionGuard;
    if n > MAX_SESSIONS {
        let mut props = Ptr::null();
        let hr = chk(query_interface(session, &IID_XBTP_OPERATION_SESSION_PROPERTIES, props.out()), b"session->QueryInterface(__uuidof(IXbtpOperationSessionProperties), (PVOID*)&sessionQuery)\0", F_REG, 43);
        if hr < 0 {
            return hr;
        }
        let mut address = Bstr::null();
        let hr = chk(props_get_property(props.0, wz("address").as_ptr(), address.out()), b"sessionQuery->GetProperty(L\"address\", &callerAddress)\0", F_REG, 44);
        if hr < 0 {
            return hr;
        }
        imports::log_info(b"XtfEventServer: EventServer_NoMoreSessions 0x%X %ws\n\0", &[E_NO_MORE_SESSIONS as u32 as u64, address.0 as u64]);
        return chk(E_NO_MORE_SESSIONS, b"EventServer_NoMoreSessions\0", F_REG, 51);
    }
    let disp = d.0.ptr as *mut Dispatcher;
    let hr = chk(dispatcher::initialize(disp, session), b"dispatcher->Initialize(session)\0", F_REG, 54);
    if hr < 0 {
        return hr;
    }
    let hr = chk(deserialize_json_body(received, req.out()), b"XtfDeserializeJsonBody(received, &reqBody)\0", F_REG, 58);
    if hr < 0 {
        return hr;
    }
    if obj_get_string(req.0, wz("ExclusiveETWProvider").as_ptr(), list.out()) >= 0 {
        let mut providers = Vec::new();
        let hr = parse_provider_list(list.0, &mut providers);
        if hr < 0 {
            return hr;
        }
        let mut trace_level = DEFAULT_TRACE_LEVEL;
        if obj_get_number(req.0, wz("TraceLevel").as_ptr(), &mut trace_level) < 0 {
            trace_level = DEFAULT_TRACE_LEVEL;
        }
        let mut protocol = DEFAULT_PROTOCOL_LEVEL;
        if obj_get_number(req.0, wz("ProtocolLevel").as_ptr(), &mut protocol) < 0 {
            protocol = DEFAULT_PROTOCOL_LEVEL;
        } else if protocol > CURRENT_PROTOCOL_LEVEL {
            return chk(E_PROTOCOL_VERSION_NOT_SUPPORTED, b"E_PROTOCOL_VERSION_NOT_SUPPORTED\0", F_REG, 104);
        }
        let k = LOGGER_COUNTER.fetch_add(1, Ordering::AcqRel) + 1;
        let src = etw::new_source(etw::logger_name(k), cvtt_i32(trace_level) as u8, cvtt_i64(protocol) as u32, providers);
        sources.0.push(wrap_object(src as *mut c_void));
    } else {
        let hr = registered_sources(crate::GetEventSourceMap(), &mut sources.0);
        if hr < 0 {
            return hr;
        }
    }
    for s in sources.0.iter() {
        let hr = chk(source_initialize(*s, &d), b"eventSource->Initialize(dispatcher)\0", F_REG, 138);
        if hr < 0 {
            return hr;
        }
    }
    let hr = dispatcher::handle_events(disp);
    for s in sources.0.iter() {
        source_shutdown(*s);
    }
    hr
}
