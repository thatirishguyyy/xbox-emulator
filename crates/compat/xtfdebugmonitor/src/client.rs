use crate::chk::chk;
use crate::server::{OP_DETACH_PROCESS, OP_MONITOR_PROCESS};
use crate::win::{CloseHandle, CreateEventW, CreateThread, Sleep, WaitForSingleObject, E_POINTER, INVALID_HANDLE_VALUE};
use crate::profile::Profile;
use crate::DEBUG_MONITOR_EXTENSION_ID;
use core::ffi::c_void;
use core::ptr::null_mut;
use std::sync::Mutex;
use xtfmsg::com::{self, json, Com, Obj, IID_XBTP_CLIENT_SESSION, IID_XBTP_MESSAGE};
use xtfmsg::msg::{cvtt32, deserialize_body, log_api, serialize_body, API_CPP, S_E_INVALIDARG};
use xtfmsg::sys::*;
use xtfmsg::xbtp;

pub const CLIENT_CPP: &str = "xbox\\devex\\xtf\\dll\\debugmonitor\\client.cpp";
pub const CONSOLE_ADDRESS_CPP: &[u8] = b"xbox\\devex\\xtf\\transport\\xtfmessaging\\consoleaddresshelpers.cpp\0";

pub const OP_RETRIEVE_DEBUG_MESSAGES: u32 = 1;

pub const XBTP_PORT_SYSTEM: u32 = 0;
pub const XBTP_PORT_TITLE: u32 = 1;
pub const XBTP_PORT_HOST: u32 = 3;

pub const XBTP_MESSAGE_TYPE_MORE: i16 = 3;
pub const XBTP_MESSAGE_TYPE_END: i16 = 4;
pub const STATUS_OPERATION_UNDEFINED: i32 = -5;

pub const E_COULD_NOT_CONNECT: HRESULT = 0x8c11_0303u32 as i32;
pub const E_NO_IPADDRESS_IN_CONNSTR: HRESULT = 0x8c11_0407u32 as i32;
pub const E_INVALID_VM_TARGET: HRESULT = 0x8c11_0408u32 as i32;
pub const E_TRANSPORT_TIMEOUT: HRESULT = 0x8c11_0410u32 as i32;

pub const DEFAULT_OPEN_SESSION_TIMEOUT_MS: u64 = 30000;
pub const DEFAULT_RECEIVE_TIMEOUT_MS: u32 = 30000;
pub const ATTACH_RESULT_TIMEOUT_MS: u32 = 120000;
pub const PUMP_RECEIVE_TIMEOUT_MS: u32 = 200;
pub const MAX_ADDRESS_CHARS: usize = 0x200;
pub const SESSION_KEY_CHARS: usize = 0x20;
pub const OPEN_OPTIONS_SIZE: u32 = 0x58;

pub const IID_XTF_DEBUG_MONITOR_CLIENT: GUID = GUID::new(0xf459ad06, 0x8efe, 0x4ed5, [0x80, 0xea, 0xdf, 0x07, 0xbe, 0xc7, 0xbe, 0xdc]);
pub const IID_XTF_DEBUG_MONITOR_CLIENT_BASE: GUID = GUID::new(0xbfe738fb, 0x5234, 0x4494, [0xbb, 0x92, 0x43, 0xa7, 0xc9, 0x20, 0xb9, 0x83]);
pub const IID_IUNKNOWN: GUID = GUID::new(0, 0, 0, [0xc0, 0, 0, 0, 0, 0, 0, 0x46]);

#[link(name = "kernel32")]
extern "system" {
    fn GetTickCount64() -> u64;
    fn SignalObjectAndWait(signal: HANDLE, wait: HANDLE, ms: u32, alertable: BOOL) -> u32;
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct OpenSessionOptions {
    pub size: u32,
    pub session_key: [u16; SESSION_KEY_CHARS],
    pub skip_ready_check: u32,
    pub allow_unneeded_key: u32,
    pub quiet: u32,
    pub reserved: u32,
    pub timeout_ms: u32,
}

#[repr(C)]
pub struct DebugStringData {
    pub process_id: u32,
    pub debug_string: *const u16,
}

#[repr(C)]
pub struct DebugMonitorCallbackVtbl {
    pub query_interface: unsafe extern "system" fn(Obj, *const GUID, *mut Obj) -> HRESULT,
    pub add_ref: unsafe extern "system" fn(Obj) -> u32,
    pub release: unsafe extern "system" fn(Obj) -> u32,
    pub on_output_debug_string: unsafe extern "system" fn(Obj, *const DebugStringData) -> HRESULT,
}

#[repr(C)]
pub struct DebugMonitorClientVtbl {
    pub query_interface: unsafe extern "system" fn(*mut DebugMonitorClient, *const GUID, *mut Obj) -> HRESULT,
    pub add_ref: unsafe extern "system" fn(*mut DebugMonitorClient) -> u32,
    pub release: unsafe extern "system" fn(*mut DebugMonitorClient) -> u32,
    pub start: unsafe extern "system" fn(*mut DebugMonitorClient, u32, usize, Obj) -> HRESULT,
    pub stop: unsafe extern "system" fn(*mut DebugMonitorClient, u32) -> HRESULT,
}

pub static CLIENT_VTBL: DebugMonitorClientVtbl = DebugMonitorClientVtbl { query_interface: client_qi, add_ref: client_add_ref, release: client_release, start: client_start, stop: client_stop };

struct RefState {
    refs: u32,
    destroyed: bool,
}

#[repr(C)]
pub struct DebugMonitorClient {
    pub vtbl: *const DebugMonitorClientVtbl,
    refs: Mutex<RefState>,
    pub address: BSTR,
    pub port: u32,
    pub stop_event: HANDLE,
    pub pump_thread: HANDLE,
    pub client_id: f64,
    pub retrieve_session: Com,
}

#[repr(C)]
struct PumpParams {
    exchange_id: u32,
    process_id: u32,
    client: *mut DebugMonitorClient,
    callback: Obj,
    stop_event: HANDLE,
}

fn z(s: &str) -> Vec<u8> {
    let mut v = s.as_bytes().to_vec();
    v.push(0);
    v
}

fn ck(func: &str, line: u32, name: &str, hr: HRESULT, msg: &str) {
    chk(CLIENT_CPP, func, line, name, hr, msg);
}

unsafe fn log_warning(text: &str) {
    let t = z(text);
    let args = [0usize; 4];
    xbtp::log_v(xbtp::LOG_WARNING_V, t.as_ptr(), args.as_ptr());
}

pub unsafe fn address_length(conn: *const u16) -> Option<usize> {
    let n = wcslen(conn);
    let s = core::slice::from_raw_parts(conn, n);
    let len = match s.iter().position(|&c| c == b'/' as u16) {
        Some(i) => i,
        None if n < MAX_ADDRESS_CHARS => n,
        None => 0,
    };
    if len != 0 {
        Some(len)
    } else {
        None
    }
}

unsafe fn bstr_len(src: *const u16, n: usize, out: &mut BSTR) -> HRESULT {
    let mut b: BSTR = null_mut();
    if n != 0 && !src.is_null() {
        if n > 0x7fff_ffff {
            return E_OUTOFMEMORY;
        }
        b = SysAllocStringLen(src, n as u32);
        if b.is_null() {
            return E_OUTOFMEMORY;
        }
    }
    *out = b;
    S_OK
}

fn eq_nocase(s: &[u16], word: &str) -> bool {
    s.len() == word.len() && s.iter().zip(word.bytes()).all(|(&c, b)| c < 0x80 && (c as u8).to_ascii_lowercase() == b)
}

pub unsafe fn split_console_address(conn: *const u16, default_port: u32, ip: *mut BSTR, key: *mut BSTR, port: *mut u32) -> HRESULT {
    if conn.is_null() || ip.is_null() || port.is_null() {
        return E_INVALIDARG;
    }
    let len = match address_length(conn) {
        Some(l) => l,
        None => return E_NO_IPADDRESS_IN_CONNSTR,
    };
    let mut target = default_port;
    let sep = *conn.add(len);
    if sep == b'\\' as u16 || sep == b'/' as u16 {
        let tail = conn.add(len + 1);
        let t = core::slice::from_raw_parts(tail, wcslen(tail));
        target = if eq_nocase(t, "system") {
            XBTP_PORT_SYSTEM
        } else if eq_nocase(t, "title") {
            XBTP_PORT_TITLE
        } else if eq_nocase(t, "host") {
            XBTP_PORT_HOST
        } else {
            return E_INVALID_VM_TARGET;
        };
    }
    *port = target;
    if !key.is_null() {
        *key = null_mut();
        let s = core::slice::from_raw_parts(conn, wcslen(conn));
        if let Some(plus) = s.iter().rposition(|&c| c == b'+' as u16) {
            let key_len = (len as u32).wrapping_sub(plus as u32).wrapping_sub(1) as usize;
            let mut k: BSTR = null_mut();
            let hr = bstr_len(conn.add(plus + 1), key_len, &mut k);
            if hr < 0 {
                return hr;
            }
            let mut a: BSTR = null_mut();
            let hr = bstr_len(conn, plus, &mut a);
            if hr < 0 {
                if !k.is_null() {
                    SysFreeString(k);
                }
                return hr;
            }
            *key = k;
            *ip = a;
            return S_OK;
        }
    }
    let mut a: BSTR = null_mut();
    let hr = bstr_len(conn, len, &mut a);
    if hr < 0 {
        return hr;
    }
    *ip = a;
    S_OK
}

pub type OpenSession2Fn = unsafe extern "C" fn(*const u16, u32, *const OpenSessionOptions, *const GUID, *const GUID, *mut Obj) -> HRESULT;

unsafe fn xbtp_open_session2(ip: *const u16, port: u32, opts: *const OpenSessionOptions, ext: *const GUID, iid: *const GUID, out: *mut Obj) -> HRESULT {
    match xbtp::proc(xbtp::OPEN_SESSION2) {
        Ok(p) => {
            let f: OpenSession2Fn = core::mem::transmute(p);
            f(ip, port, opts, ext, iid, out)
        }
        Err(e) => e,
    }
}

unsafe fn free_bstr(b: BSTR) {
    if !b.is_null() {
        SysFreeString(b);
    }
}

pub unsafe fn open_session_impl(p: &Profile, conn: *const u16, override_port: u32, ext: *const GUID, iid: *const GUID, out: *mut Obj) -> HRESULT {
    let mut last = GetTickCount64();
    let mut ip: BSTR = null_mut();
    let mut key: BSTR = null_mut();
    let mut explicit = 0u32;
    let mut hr = split_console_address(conn, override_port, &mut ip, &mut key, &mut explicit);
    if hr < 0 {
        log_api(b"SplitConsoleAddress( connectionString, port, &consoleIpAddress, &sessionKey, &explicitPort)\0", hr, API_CPP, p.split_line);
        free_bstr(key);
        free_bstr(ip);
        return hr;
    }
    let mut opts = OpenSessionOptions { size: OPEN_OPTIONS_SIZE, session_key: [0; SESSION_KEY_CHARS], skip_ready_check: 0, allow_unneeded_key: 0, quiet: 0, reserved: 0, timeout_ms: DEFAULT_OPEN_SESSION_TIMEOUT_MS as u32 };
    if !key.is_null() {
        let n = wcslen(key);
        if n >= SESSION_KEY_CHARS {
            std::process::abort();
        }
        core::ptr::copy_nonoverlapping(key, opts.session_key.as_mut_ptr(), n);
    }
    let mut delay: u64 = 0xfa;
    let mut elapsed: u64 = 0;
    let limit = DEFAULT_OPEN_SESSION_TIMEOUT_MS;
    loop {
        hr = xbtp_open_session2(ip, override_port, &opts, ext, iid, out);
        if hr != E_COULD_NOT_CONNECT || !p.open_retry {
            break;
        }
        let mut now = GetTickCount64();
        let spent = now.wrapping_sub(last);
        let d = delay.min(0xdac);
        if spent < d {
            if elapsed + 1000 + d >= limit {
                break;
            }
            Sleep((d as u32).wrapping_sub(spent as u32));
            now = GetTickCount64();
        }
        delay += 0xfa;
        elapsed += now.wrapping_sub(last);
        last = now;
        if elapsed + 1000 >= limit {
            break;
        }
    }
    if hr < 0 {
        log_api(p.open_expr, hr, API_CPP, p.open_line);
    }
    free_bstr(key);
    free_bstr(ip);
    hr
}

pub unsafe fn open_vm_session(conn: *const u16, port: u32, out: *mut Obj) -> HRESULT {
    let hr = open_session_impl(crate::profile::current(), conn, port, &DEBUG_MONITOR_EXTENSION_ID, &IID_XBTP_CLIENT_SESSION, out);
    if hr < 0 {
        log_api(b"XtfOpenSessionImpl( connectionString, overridePort, true, nullptr, extensionId, iid, ppvObject, &kDefaultOpenSessionTimeoutMS)\0", hr, API_CPP, 0x11f);
        return hr;
    }
    S_OK
}

pub unsafe fn send_json_message_timeout(session: Obj, body: Obj, op: u32, timeout: u32, reply: *mut Obj) -> HRESULT {
    if session.is_null() {
        log_api(S_E_INVALIDARG, E_INVALIDARG, API_CPP, 0x26c);
        return E_INVALIDARG;
    }
    let mut req = Com::null();
    let mut resp = Com::null();
    let mut resp_body = Com::null();
    let fail = |hr: HRESULT, expr: &[u8], line: u32| -> HRESULT {
        log_api(expr, hr, API_CPP, line);
        if hr == E_PENDING {
            E_TRANSPORT_TIMEOUT
        } else {
            hr
        }
    };
    let hr = xbtp::create_message(&IID_XBTP_MESSAGE, req.put());
    if hr < 0 {
        return fail(hr, b"XbtpCreateMessage(IID_PPV_ARGS(&reqMsg))\0", 0x270);
    }
    if !body.is_null() {
        let hr = serialize_body(body, req.get());
        if hr < 0 {
            return fail(hr, b"XtfSerializeJsonBody(body, reqMsg)\0", 0x274);
        }
    }
    let mut exchange = 0u32;
    let hr = com::client_session::send_begin_message(session, op, req.get(), &mut exchange);
    if hr < 0 {
        return fail(hr, b"session->SendBeginMessage( operationId, reqMsg, &exchangeId)\0", 0x27c);
    }
    let mut status = 0u32;
    let hr = com::client_session::receive_end_message(session, timeout, exchange, &mut status, resp.put());
    if hr < 0 {
        return fail(hr, b"session->ReceiveEndMessage( timeoutMS, exchangeId, &status, &respMsg)\0", 0x284);
    }
    if status != 0 {
        return fail(xtfmsg::api::make_transport_status_error(status), b"MakeTransportStatusError(status)\0", 0x291);
    }
    if !reply.is_null() {
        let hr = deserialize_body(resp.get(), resp_body.put());
        if hr < 0 {
            return fail(hr, b"XtfDeserializeJsonBody(respMsg, &respBody)\0", 0x28b);
        }
        *reply = resp_body.detach();
    }
    S_OK
}

pub unsafe fn send_json_message(session: Obj, body: Obj, op: u32, reply: *mut Obj) -> HRESULT {
    let hr = send_json_message_timeout(session, body, op, DEFAULT_RECEIVE_TIMEOUT_MS, reply);
    if hr < 0 {
        log_api(b"XtfSendJsonMessage( session, body, operationId, XbtpDefaultReceiveTimeout, reply)\0", hr, API_CPP, 0x255);
        return hr;
    }
    S_OK
}

type ReceiveMessageFn = unsafe extern "system" fn(Obj, u32, u32, *mut i16, *mut i32, *mut Obj) -> HRESULT;

pub unsafe fn session_receive_message(session: Obj, timeout: u32, exchange: u32, message_type: &mut i16, status: &mut i32, out: *mut Obj) -> HRESULT {
    let f: ReceiveMessageFn = core::mem::transmute(com::slot(session, 8));
    f(session, timeout, exchange, message_type, status, out)
}

fn hr_from_wait(ret: u32) -> HRESULT {
    if (ret as i32) <= 0 {
        ret as i32
    } else {
        ((ret & 0xffff) | 0x8007_0000) as i32
    }
}

fn hr_from_close_failure() -> HRESULT {
    let e = unsafe { xtfmsg::sys::GetLastError() };
    if e == 0 {
        E_FAIL
    } else {
        hr_from_wait(e)
    }
}

fn w(s: &str) -> Vec<u16> {
    wide(s)
}

impl DebugMonitorClient {
    pub unsafe fn new(address: *const u16, create_result: &mut HRESULT) -> *mut DebugMonitorClient {
        let c = Box::into_raw(Box::new(DebugMonitorClient {
            vtbl: &CLIENT_VTBL,
            refs: Mutex::new(RefState { refs: 1, destroyed: false }),
            address: null_mut(),
            port: 0,
            stop_event: null_mut(),
            pump_thread: INVALID_HANDLE_VALUE,
            client_id: 0.0,
            retrieve_session: Com::null(),
        }));
        let mut hr = split_console_address(address, XBTP_PORT_TITLE, &mut (*c).address, null_mut(), &mut (*c).port);
        if hr < 0 {
            log_api(b"SplitConsoleAddress( connectionString, defaultPort, consoleAddress, nullptr, port)\0", hr, CONSOLE_ADDRESS_CPP, 0xd7);
        } else {
            hr = S_OK;
        }
        ck("DebugMonitorClient::DebugMonitorClient", 46, "SplitConsoleAddress( pszAddress, XBTP_PORT_TITLE, &m_address, &m_port)", hr, "");
        if hr >= 0 && (*c).port == XBTP_PORT_HOST {
            (*c).port = XBTP_PORT_TITLE;
        }
        *create_result = hr;
        c
    }

    pub unsafe fn cleanup_debug_message_pump(&mut self) -> HRESULT {
        if self.pump_thread == INVALID_HANDLE_VALUE {
            return S_OK;
        }
        let ret = SignalObjectAndWait(self.stop_event, self.pump_thread, u32::MAX, 0);
        let hr = hr_from_wait(ret);
        ck("DebugMonitorClient::CleanupDebugMessagePump", 65, "SignalObjectAndWait(m_eStopPumpThreadEvent, m_hPumpThread, INFINITE, FALSE)", hr, "");
        if ret != 0 {
            return hr;
        }
        if CloseHandle(self.pump_thread) == 0 {
            return hr_from_close_failure();
        }
        self.pump_thread = INVALID_HANDLE_VALUE;
        S_OK
    }

    pub unsafe fn start(&mut self, process_id: u32, _reserved: usize, callback: Obj) -> HRESULT {
        const F: &str = "DebugMonitorClient::Start";
        if callback.is_null() {
            return E_INVALIDARG;
        }
        let mut start = Com::null();
        let mut more = Com::null();
        let mut body = Com::null();
        let mut exchange = 0u32;
        self.client_id = 0.0;
        self.retrieve_session.reset();
        let hr = open_vm_session(self.address, self.port, self.retrieve_session.put());
        ck(F, 247, "XtfOpenVMSession( m_address, m_port, DebugMonitorExtensionId, IID_PPV_ARGS(&m_retrieveSession))", hr, "");
        if hr < 0 {
            return hr;
        }
        let hr = xbtp::create_message(&IID_XBTP_MESSAGE, start.put());
        ck(F, 249, "XbtpCreateMessage(IID_PPV_ARGS(&start))", hr, "");
        if hr < 0 {
            return hr;
        }
        let hr = xbtp::create_json_object(body.put());
        ck(F, 250, "XbtpCreateJsonObject(&body)", hr, "");
        if hr < 0 {
            return hr;
        }
        let name = w("ProcessId");
        let hr = json::set_number(body.get(), name.as_ptr(), process_id as f64);
        ck(F, 251, "body->SetNumber(ProcessIdProperty, dwProcessId)", hr, "");
        if hr < 0 {
            return hr;
        }
        let hr = serialize_body(body.get(), start.get());
        ck(F, 252, "XtfSerializeJsonBody(body, start)", hr, "");
        if hr < 0 {
            return hr;
        }
        let session = self.retrieve_session.get();
        let hr = com::client_session::send_begin_message(session, OP_MONITOR_PROCESS, start.get(), &mut exchange);
        ck(F, 257, "m_retrieveSession->SendBeginMessage( MonitorProcessOperationId, start, &exchangeId)", hr, "");
        if hr < 0 {
            return hr;
        }
        let mut message_type = 0i16;
        let mut status = 0i32;
        let hr = session_receive_message(session, ATTACH_RESULT_TIMEOUT_MS, exchange, &mut message_type, &mut status, more.put());
        if hr < 0 {
            if hr == E_PENDING {
                ck(F, 266, "hr", hr, "Timeout waiting for attach result.");
            } else {
                ck(F, 268, "hr", hr, "");
            }
            return hr;
        }
        let hr = if status != 0 {
            if status != STATUS_OPERATION_UNDEFINED {
                let hr = xtfmsg::api::make_transport_status_error(status as u32);
                ck(F, 285, "MakeTransportStatusError(status)", hr, "");
                return hr;
            }
            log_warning("Debug Monitor: Falling back to old protocol with no attach acknowledgement.");
            let hr = com::client_session::send_begin_message(session, OP_RETRIEVE_DEBUG_MESSAGES, start.get(), &mut exchange);
            ck(F, 281, "m_retrieveSession->SendBeginMessage( RetrieveDebugMessagesOperationId, start, &exchangeId)", hr, "");
            hr
        } else {
            body.reset();
            let hr = deserialize_body(more.get(), body.put());
            ck(F, 292, "XtfDeserializeJsonBody(more, &body)", hr, "");
            if hr < 0 {
                return hr;
            }
            let mut attach = 0.0f64;
            let n = w("AttachResult");
            let hr = json::get_number(body.get(), n.as_ptr(), &mut attach);
            ck(F, 293, "body->GetNumber(AttachResultProperty, &hrAttachResult)", hr, "");
            if hr < 0 {
                return hr;
            }
            let n = w("ClientIdentifier");
            json::get_number(body.get(), n.as_ptr(), &mut self.client_id);
            let hr = cvtt32(attach);
            ck(F, 295, "static_cast<HRESULT>(hrAttachResult)", hr, "Attach to remote process.");
            hr
        };
        if hr < 0 {
            return hr;
        }
        self.stop_event = CreateEventW(null_mut(), 1, 0, core::ptr::null());
        if self.stop_event == INVALID_HANDLE_VALUE {
            let hr = crate::win::last_error_hr();
            ck(F, 303, "HRESULT_FROM_WIN32(GetLastError())", hr, "Unable to create event.");
            if hr < 0 {
                return hr;
            }
        }
        let params = Box::into_raw(Box::new(PumpParams { exchange_id: exchange, process_id, client: self as *mut DebugMonitorClient, callback, stop_event: self.stop_event }));
        ck(F, 309, "hr", S_OK, "threadParams = new(nothrow) PUMP_DEBUG_MESSAGES_PARAMS");
        self.pump_thread = CreateThread(null_mut(), 0, pump_thread_worker, params as *mut c_void, 0, null_mut());
        if self.pump_thread == INVALID_HANDLE_VALUE {
            drop(Box::from_raw(params));
            let hr = crate::win::last_error_hr();
            ck(F, 320, "HRESULT_FROM_WIN32(GetLastError())", hr, "Unable to create thread.");
            return hr;
        }
        S_OK
    }

    pub unsafe fn pump_debug_messages(this: *const DebugMonitorClient, exchange: u32, process_id: u32, callback: Obj, stop_event: HANDLE) -> HRESULT {
        const F: &str = "DebugMonitorClient::PumpDebugMessages";
        let mut msg = Com::null();
        loop {
            let session = core::ptr::addr_of!((*this).retrieve_session.0).read_volatile();
            let mut message_type = 0i16;
            let mut status = 0i32;
            let hr = session_receive_message(session, PUMP_RECEIVE_TIMEOUT_MS, exchange, &mut message_type, &mut status, msg.put());
            if !stop_event.is_null() && WaitForSingleObject(stop_event, 0) == 0 {
                if CloseHandle(stop_event) == 0 {
                    return hr_from_close_failure();
                }
                return S_OK;
            }
            if hr == E_PENDING {
                continue;
            }
            ck(F, 179, "hr", hr, "");
            if hr < 0 {
                return hr;
            }
            if status != 0 {
                let hr = xtfmsg::api::make_transport_status_error(status as u32);
                ck(F, 184, "MakeTransportStatusError(status)", hr, "");
                return hr;
            }
            if message_type == XBTP_MESSAGE_TYPE_MORE {
                if callback.is_null() {
                    continue;
                }
                let mut body = Com::null();
                let hr = deserialize_body(msg.get(), body.put());
                ck(F, 195, "XtfDeserializeJsonBody(spMessage, &body)", hr, "");
                if hr < 0 {
                    return hr;
                }
                let mut text: BSTR = null_mut();
                let n = w("DebugString");
                let hr = json::get_string(body.get(), n.as_ptr(), &mut text);
                ck(F, 196, "body->GetString(DebugStringProperty, &debugString)", hr, "");
                if hr < 0 {
                    free_bstr(text);
                    return hr;
                }
                let data = DebugStringData { process_id, debug_string: text };
                let vt = &**(callback as *const *const DebugMonitorCallbackVtbl);
                let hr = (vt.on_output_debug_string)(callback, &data);
                ck(F, 200, "pCallback->OnOutputDebugString(&data)", hr, "");
                drop(body);
                free_bstr(text);
                if hr < 0 {
                    return hr;
                }
                continue;
            }
            if message_type == XBTP_MESSAGE_TYPE_END {
                return hr;
            }
            ck(F, 210, "E_UNEXPECTED", E_UNEXPECTED, "Unexpected message");
            return E_UNEXPECTED;
        }
    }

    pub unsafe fn stop(&mut self, process_id: u32) -> HRESULT {
        const F: &str = "DebugMonitorClient::Stop";
        let mut session = Com::null();
        let mut request = Com::null();
        let mut reply = Com::null();
        let hr = open_vm_session(self.address, self.port, session.put());
        ck(F, 346, "XtfOpenVMSession( m_address, m_port, DebugMonitorExtensionId, IID_PPV_ARGS(&session))", hr, "");
        if hr < 0 {
            return hr;
        }
        let hr = xbtp::create_json_object(request.put());
        ck(F, 349, "XbtpCreateJsonObject(&request)", hr, "");
        if hr < 0 {
            return hr;
        }
        let n = w("ProcessId");
        let hr = json::set_number(request.get(), n.as_ptr(), process_id as f64);
        ck(F, 350, "request->SetNumber(ProcessIdProperty, dwProcessId)", hr, "");
        if hr < 0 {
            return hr;
        }
        if self.client_id != 0.0 {
            let n = w("ClientIdentifier");
            let hr = json::set_number(request.get(), n.as_ptr(), self.client_id);
            ck(F, 355, "request->SetNumber(ClientSessionIdentifierProperty, m_clientId)", hr, "");
            if hr < 0 {
                return hr;
            }
        }
        let hr = send_json_message(session.get(), request.get(), OP_DETACH_PROCESS, reply.put());
        ck(F, 361, "XtfSendJsonMessage( session, request, DetachProcessOperationId, &reply)", hr, "");
        if hr < 0 {
            return hr;
        }
        let hr = self.cleanup_debug_message_pump();
        ck(F, 363, "CleanupDebugMessagePump()", hr, "");
        hr
    }
}

impl Drop for DebugMonitorClient {
    fn drop(&mut self) {
        unsafe {
            self.cleanup_debug_message_pump();
            self.retrieve_session.reset();
            free_bstr(self.address);
            self.address = null_mut();
        }
    }
}

unsafe extern "system" fn pump_thread_worker(p: *mut c_void) -> u32 {
    if p.is_null() {
        return 0xa0;
    }
    let params = Box::from_raw(p as *mut PumpParams);
    let hr = DebugMonitorClient::pump_debug_messages(params.client, params.exchange_id, params.process_id, params.callback, params.stop_event);
    ck("DebugMonitorClient::PumpThreadWorker", 132, "hr = params->pDbgMonClient->PumpDebugMessages( params->exchangeId, params->dwProcessId, params->pCallback, params->eStopEvent)", hr, "");
    drop(params);
    (hr < 0) as u32
}

fn guid_eq(a: &GUID, b: &GUID) -> bool {
    a.data1 == b.data1 && a.data2 == b.data2 && a.data3 == b.data3 && a.data4 == b.data4
}

unsafe extern "system" fn client_qi(this: *mut DebugMonitorClient, iid: *const GUID, out: *mut Obj) -> HRESULT {
    if iid.is_null() || out.is_null() {
        return E_POINTER;
    }
    let iid = &*iid;
    if guid_eq(iid, &IID_XTF_DEBUG_MONITOR_CLIENT) || guid_eq(iid, &IID_IUNKNOWN) || guid_eq(iid, &IID_XTF_DEBUG_MONITOR_CLIENT_BASE) {
        *out = this as Obj;
        client_add_ref(this);
        return S_OK;
    }
    E_NOINTERFACE
}

unsafe extern "system" fn client_add_ref(this: *mut DebugMonitorClient) -> u32 {
    let mut g = (*this).refs.lock().unwrap_or_else(|e| e.into_inner());
    g.refs += 1;
    g.refs
}

unsafe extern "system" fn client_release(this: *mut DebugMonitorClient) -> u32 {
    let n = {
        let mut g = (*this).refs.lock().unwrap_or_else(|e| e.into_inner());
        g.refs -= 1;
        if g.refs != 0 || g.destroyed {
            return g.refs;
        }
        g.destroyed = true;
        g.refs
    };
    drop(Box::from_raw(this));
    n
}

unsafe extern "system" fn client_start(this: *mut DebugMonitorClient, process_id: u32, reserved: usize, callback: Obj) -> HRESULT {
    (*this).start(process_id, reserved, callback)
}

unsafe extern "system" fn client_stop(this: *mut DebugMonitorClient, process_id: u32) -> HRESULT {
    (*this).stop(process_id)
}

#[no_mangle]
pub unsafe extern "system" fn XtfCreateDebugMonitorClient(address: *const u16, riid: *const GUID, ppv: *mut Obj) -> HRESULT {
    const F: &str = "XtfCreateDebugMonitorClient";
    crate::imports::bind();
    if ppv.is_null() {
        return E_POINTER;
    }
    *ppv = null_mut();
    let mut create_result = S_OK;
    let client = DebugMonitorClient::new(address, &mut create_result);
    ck(F, 21, "hr", S_OK, "spClient.Attach(new (std::nothrow) DebugMonitorClient(pszAddress, &createResult))");
    ck(F, 22, "createResult", create_result, "");
    let hr = if create_result < 0 {
        create_result
    } else {
        let hr = client_qi(client, riid, ppv);
        ck(F, 23, "spClient->QueryInterface(riid, ppvObject)", hr, "");
        hr
    };
    client_release(client);
    hr
}
