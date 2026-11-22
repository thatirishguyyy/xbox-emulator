use crate::chk::chk;
use crate::{CONSOLE_EXTENSION_ID, OP_GENERATE_STACK_DUMP, OP_GET_ALL_SETTINGS, OP_GET_CONFIG_VALUE, OP_GET_RUNNING_PROCESSES, OP_GET_SYSTEM_TIME, OP_IN_CONNECTED_STANDBY, OP_SET_CONFIG_VALUE, OP_SHUTDOWN, OP_UPDATES_PENDING};
use core::ffi::c_void;
use core::ptr::null_mut;
use std::sync::Mutex;
use xtfmsg::com::{self, array, json, Com, Obj, IID_IUNKNOWN, IID_XBTP_CLIENT_SESSION, IID_XBTP_MESSAGE};
use xtfmsg::msg::{cvtt32, cvtt64, get_json_bool, get_json_u32, log_api, set_json_bool, set_json_u32};
use xtfmsg::sys::*;
use xtfmsg::xbtp;

pub const CLIENT_CPP: &str = "xbox\\devex\\xtf\\dll\\console\\client.cpp";
pub const SHUTDOWN_CPP: &str = "xbox\\devex\\xtf\\dll\\console\\shutdown.cpp";
pub const SYSTEMTIME_CPP: &str = "xbox\\devex\\xtf\\dll\\console\\systemtime.cpp";
pub const UPDATESPENDING_CPP: &str = "xbox\\devex\\xtf\\dll\\console\\updatespending.cpp";
pub const RUNNINGPROCESSES_CPP: &str = "xbox\\devex\\xtf\\dll\\console\\getrunningprocesses.cpp";
pub const CONNECTEDSTANDBY_CPP: &str = "xbox\\devex\\xtf\\dll\\console\\inconnectedstandby.cpp";
pub const MESSAGEHELPERS_CPP: &str = "xbox\\devex\\xtf\\dll\\console\\messagehelpers.cpp";
pub const SESSIONKEY_CPP: &str = "xbox\\devex\\xtf\\dll\\console\\sessionkey.cpp";
pub const JSON_HELPERS_H: &str = "xbox\\devex\\xtf\\transport\\XtfMessaging\\XtfJsonHelpers.h";
pub const API_CPP: &[u8] = b"xbox\\devex\\xtf\\transport\\xtfmessaging\\api.cpp\0";
pub const CONSOLE_ADDRESS_CPP: &[u8] = b"xbox\\devex\\xtf\\transport\\xtfmessaging\\consoleaddresshelpers.cpp\0";
pub const OPERATION_ERROR_CPP: &[u8] = b"xbox\\devex\\xtf\\transport\\xtfmessaging\\xtfoperationerror.cpp\0";

pub const IID_XTF_CONSOLE_CONTROL_CLIENT: GUID = GUID::new(0x9e32cbaf, 0xbe25, 0x4a64, [0x8d, 0x62, 0xb5, 0x69, 0x16, 0x4c, 0x96, 0xad]);
pub const IID_XTF_CONSOLE_CONTROL_CLIENT_BASE: GUID = GUID::new(0xb0332def, 0x2c16, 0x43be, [0x80, 0xc4, 0x91, 0x24, 0x90, 0x6f, 0x35, 0x21]);
pub const XTF_PROVISION_EXTENSION_ID: GUID = GUID::new(0x7b80396d, 0x0d0f, 0x43cf, [0x84, 0x57, 0xe5, 0x2e, 0xce, 0x88, 0x19, 0xb0]);
pub const DIAGNOSTIC_INFORMATION_EXTENSION_ID: GUID = GUID::new(0xfd21ac23, 0x3a3a, 0x45ce, [0xa7, 0x85, 0xc9, 0x73, 0x37, 0x2d, 0xd8, 0x81]);

pub const XBTP_PORT_SYSTEM: u32 = 0;
pub const XBTP_PORT_TITLE: u32 = 1;
pub const XBTP_PORT_HOST: u32 = 3;
pub const XBTP_PORT_KIND_LINK: u32 = 0x0100_0000;

pub const E_POINTER: HRESULT = 0x8000_4003u32 as i32;
pub const E_COULD_NOT_CONNECT: HRESULT = 0x8c11_0303u32 as i32;
pub const E_NO_IPADDRESS_IN_CONNSTR: HRESULT = 0x8c11_0407u32 as i32;
pub const E_INVALID_VM_TARGET: HRESULT = 0x8c11_0408u32 as i32;
pub const E_SESSION_OPEN_FINAL: HRESULT = 0x8c11_040du32 as i32;
pub const E_SYSTEM_NOT_READY: HRESULT = 0x8c11_040eu32 as i32;
pub const E_TRANSPORT_TIMEOUT: HRESULT = 0x8c11_0410u32 as i32;
pub const E_WSAECONNRESET: HRESULT = 0x8007_2746u32 as i32;

pub const OP_SET_SYSTEM_GO_DARK: u32 = 0x10;
pub const OP_REBOOT_V1: u32 = 2;
pub const DEFAULT_OPEN_SESSION_TIMEOUT_MS: u32 = 30000;
pub const DEFAULT_RECEIVE_TIMEOUT_MS: u32 = 30000;
pub const STACK_DUMP_REPLY_TIMEOUT_MS: u32 = 300000;
pub const REBOOT_REPLY_TIMEOUT_MS: u32 = 150;
pub const SHUTDOWN_NOT_READY_WINDOW_MS: u64 = 300000;
pub const SHUTDOWN_NOT_READY_SLEEP_MS: u32 = 5000;
pub const STANDBY_OPEN_TIMEOUT_MS: u32 = 250;
pub const STANDBY_OPEN_ATTEMPTS: u32 = 25;
pub const OPEN_ATTEMPT_CAP_MS: u32 = 500;
pub const OPEN_RETRY_SPACING_MS: u64 = 250;
pub const MAX_ADDRESS_CHARS: usize = 0x200;
pub const SESSION_KEY_CHARS: usize = 0x20;
pub const MAX_ACCESS_KEY_CHARS: usize = 0x1f;
pub const OPEN_OPTIONS_SIZE: u32 = 0x58;

pub const ACCESS_KEY_MESSAGE: &[u8] = b"The access key '%ws' isn't valid.\r\nIt can only consist of letters and numbers and be %d characters long\0";

#[link(name = "kernel32")]
extern "system" {
    fn GetTickCount64() -> u64;
    fn Sleep(ms: u32);
    fn FileTimeToSystemTime(ft: *const u64, st: *mut SystemTime) -> i32;
    fn OutputDebugStringA(s: *const u8);
}

#[link(name = "ole32")]
extern "system" {
    fn CoTaskMemAlloc(n: usize) -> *mut c_void;
    fn CoTaskMemFree(p: *mut c_void);
}

extern "C" {
    fn iswalnum(c: u16) -> i32;
}

#[cfg(test)]
pub static TEST_PORTS: Mutex<Option<[u32; 4]>> = Mutex::new(None);

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct SystemTime {
    pub year: u16,
    pub month: u16,
    pub day_of_week: u16,
    pub day: u16,
    pub hour: u16,
    pub minute: u16,
    pub second: u16,
    pub milliseconds: u16,
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
pub struct ConfigSetting {
    pub name: BSTR,
    pub value: BSTR,
}

#[repr(C)]
pub struct ProcessInfo {
    pub process_id: u32,
    pub name: BSTR,
}

fn ck(file: &str, func: &str, line: u32, name: &str, hr: HRESULT) {
    chk(file, func, line, name, hr, "");
}

fn ckm(file: &str, func: &str, line: u32, name: &str, hr: HRESULT, msg: &str) {
    chk(file, func, line, name, hr, msg);
}

fn la(expr: &str, hr: HRESULT, file: &[u8], line: u32) {
    let mut e = expr.as_bytes().to_vec();
    e.push(0);
    log_api(&e, hr, file, line);
}

fn w(s: &str) -> Vec<u16> {
    wide(s)
}

unsafe fn free_bstr(b: BSTR) {
    if !b.is_null() {
        SysFreeString(b);
    }
}

fn now_ms() -> u64 {
    use std::sync::OnceLock;
    use std::time::Instant;
    static BASE: OnceLock<Instant> = OnceLock::new();
    let b = *BASE.get_or_init(Instant::now);
    b.elapsed().as_millis() as u64 + 1
}

unsafe fn log_error(fmt: &[u8], args: &[usize]) {
    xbtp::log_v(b"?LogErrorV@@YAXPEBDPEAD@Z\0", fmt.as_ptr(), args.as_ptr());
}

type XtfLogMessageFn = unsafe extern "C" fn(u32, *mut c_void, *const u8, *const u8, i32, *const u8, *const usize, u32) -> i32;

#[repr(C)]
struct LogContext {
    a: u64,
    b: u64,
    c: i32,
    d: u32,
}

pub unsafe fn customer_string(fmt: &[u8], args: &[usize]) {
    static CTX: Mutex<LogContext> = Mutex::new(LogContext { a: 0, b: 0, c: -1, d: 0 });
    let name = wide("XtfBaseServices.dll");
    let mut m = GetModuleHandleW(name.as_ptr());
    if m.is_null() {
        m = LoadLibraryW(name.as_ptr());
    }
    let p = if m.is_null() { null_mut() } else { GetProcAddress(m, b"XtfLogMessage\0".as_ptr()) };
    if !p.is_null() {
        let f: XtfLogMessageFn = core::mem::transmute(p);
        let mut c = CTX.lock().unwrap_or_else(|e| e.into_inner());
        f(0, &mut *c as *mut LogContext as *mut c_void, core::ptr::null(), core::ptr::null(), 0, fmt.as_ptr(), args.as_ptr(), 3);
        return;
    }
    OutputDebugStringA(b"Can't output customer string:\0".as_ptr());
    OutputDebugStringA(fmt.as_ptr());
    OutputDebugStringA(b"\r\n\0".as_ptr());
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
        la("E_INVALIDARG", E_INVALIDARG, CONSOLE_ADDRESS_CPP, 0xe9);
        return E_INVALIDARG;
    }
    let len = match address_length(conn) {
        Some(l) => l,
        None => {
            la("E_NO_IPADDRESS_IN_CONNSTR", E_NO_IPADDRESS_IN_CONNSTR, CONSOLE_ADDRESS_CPP, 0xf4);
            return E_NO_IPADDRESS_IN_CONNSTR;
        }
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
            la("E_INVALID_VM_TARGET", E_INVALID_VM_TARGET, CONSOLE_ADDRESS_CPP, 0x10d);
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
                la("sessionKeyScratch.Set(keySeparator +1, sessionKeyLength)", hr, CONSOLE_ADDRESS_CPP, 0x129);
                return hr;
            }
            let mut a: BSTR = null_mut();
            let hr = bstr_len(conn, plus, &mut a);
            if hr < 0 {
                la("ipAddressScratch.Set(connectionString, ipAddressLength)", hr, CONSOLE_ADDRESS_CPP, 0x12a);
                free_bstr(k);
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
        la("scratch.Set(connectionString, addressLength)", hr, CONSOLE_ADDRESS_CPP, 0x133);
        return hr;
    }
    *ip = a;
    S_OK
}

type OpenSession2Fn = unsafe extern "C" fn(*const u16, u32, *const OpenSessionOptions, *const GUID, *const GUID, *mut Obj) -> HRESULT;

unsafe fn xbtp_open_session2(ip: *const u16, port: u32, opts: *const OpenSessionOptions, ext: *const GUID, out: *mut Obj) -> HRESULT {
    #[cfg(test)]
    {
        let map = *TEST_PORTS.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(m) = map {
            let i = port as usize;
            if i < m.len() && i != 2 {
                return match xbtp::proc(b"XbtpOpenSessionEx2\0") {
                    Ok(f) => {
                        let f: OpenSession2Fn = core::mem::transmute(f);
                        f(ip, XBTP_PORT_KIND_LINK | m[i], opts, ext, &IID_XBTP_CLIENT_SESSION, out)
                    }
                    Err(e) => e,
                };
            }
        }
    }
    match xbtp::proc(xbtp::OPEN_SESSION2) {
        Ok(p) => {
            let f: OpenSession2Fn = core::mem::transmute(p);
            f(ip, port, opts, ext, &IID_XBTP_CLIENT_SESSION, out)
        }
        Err(e) => e,
    }
}

pub unsafe fn open_session_impl(conn: *const u16, port: u32, ext: *const GUID, out: *mut Obj, timeout: Option<u32>) -> HRESULT {
    let mut opts = OpenSessionOptions { size: 0, session_key: [0; SESSION_KEY_CHARS], skip_ready_check: 0, allow_unneeded_key: 0, quiet: 0, reserved: 0, timeout_ms: timeout.unwrap_or(DEFAULT_OPEN_SESSION_TIMEOUT_MS) };
    let mut ip: BSTR = null_mut();
    let mut key: BSTR = null_mut();
    let mut explicit = 0u32;
    let mut hr = split_console_address(conn, port, &mut ip, &mut key, &mut explicit);
    if hr < 0 {
        la("SplitConsoleAddress( connectionString, port, &consoleIpAddress, &sessionKey, &explicitPort)", hr, API_CPP, 0xa4);
    } else {
        opts.size = OPEN_OPTIONS_SIZE;
        if !key.is_null() {
            let n = wcslen(key);
            if n >= SESSION_KEY_CHARS {
                std::process::abort();
            }
            core::ptr::copy_nonoverlapping(key, opts.session_key.as_mut_ptr(), n);
        }
        hr = xbtp_open_session2(ip, port, &opts, ext, out);
        if hr < 0 {
            la("XbtpOpenSession2( consoleIpAddress, explicitPort, &auxInfo, extensionId, iid, ppvObject)", hr, API_CPP, 0xc6);
        } else {
            hr = S_OK;
        }
    }
    free_bstr(key);
    free_bstr(ip);
    hr
}

pub unsafe fn open_vm_session(conn: *const u16, port: u32, ext: *const GUID, out: &mut Com) -> HRESULT {
    out.reset();
    let hr = open_session_impl(conn, port, ext, out.put(), None);
    if hr < 0 {
        la("XtfOpenSessionImpl( connectionString, overridePort, true, nullptr, extensionId, iid, ppvObject, &kDefaultOpenSessionTimeoutMS)", hr, API_CPP, 0xe0);
        return hr;
    }
    S_OK
}

pub unsafe fn open_vm_session_timeout(conn: *const u16, timeout: Option<u32>, out: *mut Obj) -> HRESULT {
    let mut last = now_ms();
    let limit = timeout.unwrap_or(DEFAULT_OPEN_SESSION_TIMEOUT_MS);
    if out.is_null() {
        return E_POINTER;
    }
    *out = null_mut();
    let mut session = Com::null();
    let attempt = limit.min(OPEN_ATTEMPT_CAP_MS);
    let mut elapsed: u64 = 0;
    let mut hr;
    loop {
        session.reset();
        hr = open_session_impl(conn, XBTP_PORT_SYSTEM, &CONSOLE_EXTENSION_ID, session.put(), Some(attempt));
        if timeout.is_none() || hr == S_OK || hr == E_SESSION_OPEN_FINAL {
            break;
        }
        let mut now = now_ms();
        let spent = now.wrapping_sub(last);
        if spent < OPEN_RETRY_SPACING_MS {
            Sleep((OPEN_RETRY_SPACING_MS - spent) as u32);
            now = now_ms();
        }
        if limit != u32::MAX {
            elapsed += now.wrapping_sub(last);
        }
        last = now;
        if elapsed > limit as u64 {
            break;
        }
    }
    if hr >= 0 {
        *out = session.detach();
    }
    hr
}

pub unsafe fn serialize_body(body: Obj, message: Obj) -> HRESULT {
    if body.is_null() || message.is_null() {
        la("E_INVALIDARG", E_INVALIDARG, API_CPP, 0x1f4);
        return E_INVALIDARG;
    }
    let mut text: BSTR = null_mut();
    let mut hr = json::stringify(body, &mut text);
    if hr < 0 {
        la("body->Stringify(&wbody)", hr, API_CPP, 0x1f7);
    } else {
        hr = com::message::set_json_body_w(message, text);
        if hr < 0 {
            la("message->SetJsonBodyW(wbody)", hr, API_CPP, 0x1f8);
        } else {
            hr = S_OK;
        }
    }
    free_bstr(text);
    hr
}

pub unsafe fn setup_json_body(unparsed: *const u8, out: *mut Obj) -> HRESULT {
    if unparsed.is_null() {
        let hr = xbtp::create_json_object(out);
        if hr < 0 {
            la("XbtpCreateJsonObject(body)", hr, API_CPP, 0x73);
            return hr;
        }
        return S_OK;
    }
    let hr = xbtp::get_json_object_from_utf8(unparsed, out);
    if hr < 0 {
        la("XbtpGetJsonObjectFromUtf8(unparsedBody, body)", hr, API_CPP, 0x77);
        return hr;
    }
    S_OK
}

pub unsafe fn deserialize_body(message: Obj, out: *mut Obj) -> HRESULT {
    if message.is_null() || out.is_null() {
        la("E_INVALIDARG", E_INVALIDARG, API_CPP, 0x1ae);
        return E_INVALIDARG;
    }
    let mut len = 0u32;
    let mut body: *const u8 = core::ptr::null();
    let hr = com::message::get_json_body(message, &mut len, &mut body);
    if hr < 0 {
        la("message->GetJsonBody(&length, &unparsedBody)", hr, API_CPP, 0x1b3);
        return hr;
    }
    let hr = setup_json_body(body, out);
    if hr < 0 {
        la("SetupJsonBody(unparsedBody, body)", hr, API_CPP, 0x1b4);
        return hr;
    }
    S_OK
}

pub unsafe fn send_json_message_timeout(session: Obj, body: Obj, op: u32, timeout: u32, reply: Option<&mut Com>) -> HRESULT {
    if session.is_null() {
        la("E_INVALIDARG", E_INVALIDARG, API_CPP, 0x225);
        return E_INVALIDARG;
    }
    let mut req = Com::null();
    let mut resp = Com::null();
    let fail = |hr: HRESULT, expr: &str, line: u32| -> HRESULT {
        la(expr, hr, API_CPP, line);
        if hr == E_PENDING {
            E_TRANSPORT_TIMEOUT
        } else {
            hr
        }
    };
    let hr = xbtp::create_message(&IID_XBTP_MESSAGE, req.put());
    if hr < 0 {
        return fail(hr, "XbtpCreateMessage(IID_PPV_ARGS(&reqMsg))", 0x229);
    }
    if !body.is_null() {
        let hr = serialize_body(body, req.get());
        if hr < 0 {
            return fail(hr, "XtfSerializeJsonBody(body, reqMsg)", 0x22d);
        }
    }
    let mut exchange = 0u32;
    let hr = com::client_session::send_begin_message(session, op, req.get(), &mut exchange);
    if hr < 0 {
        return fail(hr, "session->SendBeginMessage( operationId, reqMsg, &exchangeId)", 0x235);
    }
    let mut status = 0u32;
    let hr = com::client_session::receive_end_message(session, timeout, exchange, &mut status, resp.put());
    if hr < 0 {
        return fail(hr, "session->ReceiveEndMessage( timeoutMS, exchangeId, &status, &respMsg)", 0x23d);
    }
    if status != 0 {
        return fail(((status & 0xff) | 0x8c11_0100) as i32, "MakeTransportStatusError(status)", 0x24a);
    }
    if let Some(r) = reply {
        let mut b = Com::null();
        let hr = deserialize_body(resp.get(), b.put());
        if hr < 0 {
            return fail(hr, "XtfDeserializeJsonBody(respMsg, &respBody)", 0x244);
        }
        *r = b;
    }
    S_OK
}

pub unsafe fn send_json_message(session: Obj, body: Obj, op: u32, reply: Option<&mut Com>) -> HRESULT {
    let hr = send_json_message_timeout(session, body, op, DEFAULT_RECEIVE_TIMEOUT_MS, reply);
    if hr < 0 {
        la("XtfSendJsonMessage( session, body, operationId, XbtpDefaultReceiveTimeout, reply)", hr, API_CPP, 0x20e);
        return hr;
    }
    S_OK
}

#[derive(Default)]
pub struct OperationError {
    pub checkpoint: u32,
    pub return_code: i32,
    pub errors: Option<Com>,
}

impl OperationError {
    pub unsafe fn initialize_body(&mut self, body: Obj) -> HRESULT {
        if body.is_null() {
            la("E_INVALIDARG", E_INVALIDARG, OPERATION_ERROR_CPP, 0x2b);
            return E_INVALIDARG;
        }
        let mut eb = Com::null();
        let n = w("XtfOperationError");
        if json::get_object(body, n.as_ptr(), eb.put()) < 0 {
            xbtp::log_info(b"No error body found in reply message\0".as_ptr(), &[]);
            return S_OK;
        }
        let mut rc = 0f64;
        let n = w("ReturnCode");
        let hr = json::get_number(eb.get(), n.as_ptr(), &mut rc);
        if hr < 0 {
            la("errorBody->GetNumber(XbtpEndErrorReturnCodeName, &rc)", hr, OPERATION_ERROR_CPP, 0x39);
            return hr;
        }
        self.return_code = cvtt32(rc);
        let mut cp = 0f64;
        let n = w("Checkpoint");
        let hr = json::get_number(eb.get(), n.as_ptr(), &mut cp);
        if hr < 0 {
            la("errorBody->GetNumber(XbtpEndErrorCheckpointName, &cp)", hr, OPERATION_ERROR_CPP, 0x3e);
            return hr;
        }
        self.checkpoint = cvtt64(cp) as u32;
        let mut errors = Com::null();
        let n = w("Errors");
        let hr = json::get_array(eb.get(), n.as_ptr(), errors.put());
        self.errors = Some(errors);
        if hr < 0 {
            la("errorBody->GetArray(XbtpEndErrorErrorsName, &m_errors)", hr, OPERATION_ERROR_CPP, 0x42);
            return hr;
        }
        S_OK
    }
}

pub unsafe fn check_task_errors(response: Obj) -> HRESULT {
    const F: &str = "XtfConsoleControl::CheckTaskErrors";
    if response.is_null() {
        ck(MESSAGEHELPERS_CPP, F, 19, "E_INVALIDARG", E_INVALIDARG);
        return E_INVALIDARG;
    }
    let mut e = OperationError::default();
    let hr = e.initialize_body(response);
    ck(MESSAGEHELPERS_CPP, F, 22, "errorCheck.Initialize(response)", hr);
    if hr < 0 {
        return hr;
    }
    let result = e.return_code;
    ck(MESSAGEHELPERS_CPP, F, 23, "errorCheck.GetReturnCode(&result)", S_OK);
    ck(MESSAGEHELPERS_CPP, F, 24, "result", result);
    result
}

pub unsafe fn get_json_u64_property(body: Obj, name: &str, out: *mut u64) -> HRESULT {
    const F: &str = "XtfGetJsonProperty64";
    if body.is_null() || out.is_null() {
        ck(JSON_HELPERS_H, F, 44, "E_INVALIDARG", E_INVALIDARG);
        return E_INVALIDARG;
    }
    let mut num = Com::null();
    let n = w(name);
    let hr = json::get_object(body, n.as_ptr(), num.put());
    if hr == E_JSON_NOT_FOUND {
        return hr;
    }
    ck(JSON_HELPERS_H, F, 52, "hr", hr);
    if hr < 0 {
        return hr;
    }
    let mut d = 0f64;
    let l = w("L");
    let hr = json::get_number(num.get(), l.as_ptr(), &mut d);
    ck(JSON_HELPERS_H, F, 56, "num->GetNumber(L\"L\", &dval)", hr);
    if hr < 0 {
        return hr;
    }
    let low = cvtt64(d) as u32;
    let mut d = 0f64;
    let h = w("H");
    let hr = json::get_number(num.get(), h.as_ptr(), &mut d);
    ck(JSON_HELPERS_H, F, 59, "num->GetNumber(L\"H\", &dval)", hr);
    if hr < 0 {
        return hr;
    }
    let high = cvtt64(d) as u32;
    *out = ((high as u64) << 32) | low as u64;
    hr
}

pub unsafe fn verify_session_key(key: *const u16) -> HRESULT {
    const F: &str = "ConsoleConfig::VerifySessionKey";
    if key.is_null() {
        return S_OK;
    }
    let n = wcslen(key);
    if n > MAX_ACCESS_KEY_CHARS {
        log_error(b"Session key is too long (%d)\0", &[n]);
        ck(SESSIONKEY_CPP, F, 31, "E_INVALIDARG", E_INVALIDARG);
        return E_INVALIDARG;
    }
    for i in 0..n {
        if iswalnum(*key.add(i)) == 0 {
            log_error(b"Session key[%d] is invalid\0", &[i]);
            ck(SESSIONKEY_CPP, F, 40, "E_INVALIDARG", E_INVALIDARG);
            return E_INVALIDARG;
        }
    }
    S_OK
}

unsafe fn wcsicmp_eq(a: *const u16, b: &str) -> bool {
    if a.is_null() || *a == 0 || b.is_empty() {
        return false;
    }
    let s = core::slice::from_raw_parts(a, wcslen(a));
    let t: Vec<u16> = b.encode_utf16().collect();
    let low = |c: u16| if (b'A' as u16..=b'Z' as u16).contains(&c) { c + 32 } else { c };
    s.len() == t.len() && s.iter().zip(t.iter()).all(|(&x, &y)| low(x) == low(y))
}

unsafe fn set_string(body: Obj, name: &str, v: *const u16) -> HRESULT {
    let n = w(name);
    json::set_string(body, n.as_ptr(), v)
}

unsafe fn get_string(body: Obj, name: &str, out: *mut BSTR) -> HRESULT {
    let n = w(name);
    json::get_string(body, n.as_ptr(), out)
}

pub unsafe fn send_reboot_command(session: Obj, flags: u32, resp_body: Option<&mut Com>) -> HRESULT {
    const F: &str = "SendRebootCommand";
    let Some(resp_body) = resp_body.filter(|_| !session.is_null()) else {
        ck(SHUTDOWN_CPP, F, 166, "E_INVALIDARG", E_INVALIDARG);
        return E_INVALIDARG;
    };
    let reboot = flags & 1 != 0;
    let power_mode_aware = (flags >> 1) & 1 != 0;
    let mut req = Com::null();
    let hr = xbtp::create_json_object(req.put());
    ck(SHUTDOWN_CPP, F, 172, "XbtpCreateJsonObject(&reqBody)", hr);
    if hr < 0 {
        return hr;
    }
    let n = w("Reboot");
    let hr = set_json_bool(req.get(), n.as_ptr(), reboot);
    ck(SHUTDOWN_CPP, F, 176, "XtfSetJsonProperty<bool>( reqBody, RebootBoolProperty, bReboot)", hr);
    if hr < 0 {
        return hr;
    }
    let n = w("PowerModeAware");
    let hr = set_json_bool(req.get(), n.as_ptr(), power_mode_aware);
    ck(SHUTDOWN_CPP, F, 180, "XtfSetJsonProperty<bool>( reqBody, PowerModeAwareBoolProperty, bPowerModeAware)", hr);
    if hr < 0 {
        return hr;
    }
    let mut msg = Com::null();
    let hr = xbtp::create_message(&IID_XBTP_MESSAGE, msg.put());
    ck(SHUTDOWN_CPP, F, 183, "XbtpCreateMessage(IID_PPV_ARGS(&reqMsg))", hr);
    if hr < 0 {
        return hr;
    }
    let hr = serialize_body(req.get(), msg.get());
    ck(SHUTDOWN_CPP, F, 184, "XtfSerializeJsonBody(reqBody, reqMsg)", hr);
    if hr < 0 {
        return hr;
    }
    let mut exchange = 0u32;
    let hr = com::client_session::send_begin_message(session, OP_SHUTDOWN, msg.get(), &mut exchange);
    ck(SHUTDOWN_CPP, F, 186, "session->SendBeginMessage(ConsoleShutdownOperationId, reqMsg, &exchangeId)", hr);
    if hr < 0 {
        return hr;
    }
    let mut status = 0u32;
    let mut resp = Com::null();
    let mut hr = com::client_session::receive_end_message(session, REBOOT_REPLY_TIMEOUT_MS, exchange, &mut status, resp.put());
    if hr < 0 {
        hr = S_OK;
    }
    if status != 0 || resp.is_null() {
        return hr;
    }
    let hr = deserialize_body(resp.get(), resp_body.put());
    ck(SHUTDOWN_CPP, F, 202, "XtfDeserializeJsonBody(respMsg, respBody)", hr);
    hr
}

pub unsafe fn send_system_go_dark_to(address: *const u16, port: u32) -> HRESULT {
    const F: &str = "SendSystemGoDarkTo";
    let mut session = Com::null();
    let hr = open_vm_session(address, port, &DIAGNOSTIC_INFORMATION_EXTENSION_ID, &mut session);
    ck(SHUTDOWN_CPP, F, 229, "XtfOpenVMSession(inAddress, inPort, DiagnosticInformationExtensionId, IID_PPV_ARGS(&session))", hr);
    if hr < 0 {
        return hr;
    }
    let mut req = Com::null();
    let hr = xbtp::create_json_object(req.put());
    ck(SHUTDOWN_CPP, F, 230, "XbtpCreateJsonObject(&reqBody)", hr);
    if hr < 0 {
        return hr;
    }
    let n = w("SysIsDark");
    let hr = set_json_u32(req.get(), n.as_ptr(), 1);
    ck(SHUTDOWN_CPP, F, 233, "XtfSetJsonProperty<DWORD>(reqBody, SystemGoDarkProperty, 1U)", hr);
    if hr < 0 {
        return hr;
    }
    let mut resp = Com::null();
    let hr = send_json_message(session.get(), req.get(), OP_SET_SYSTEM_GO_DARK, Some(&mut resp));
    ck(SHUTDOWN_CPP, F, 236, "XtfSendJsonMessage(session, reqBody, SetSystemGoDarkOperationId, &respBody)", hr);
    hr
}

struct RefState {
    refs: u32,
    destroying: bool,
}

#[repr(C)]
pub struct ConsoleControlClientVtbl {
    pub query_interface: unsafe extern "system" fn(*mut ConsoleControlClient, *const GUID, *mut Obj) -> HRESULT,
    pub add_ref: unsafe extern "system" fn(*mut ConsoleControlClient) -> u32,
    pub release: unsafe extern "system" fn(*mut ConsoleControlClient) -> u32,
    pub shutdown_console: unsafe extern "system" fn(*mut ConsoleControlClient, u32) -> HRESULT,
    pub updates_pending: unsafe extern "system" fn(*mut ConsoleControlClient, *mut i32) -> HRESULT,
    pub get_running_processes: unsafe extern "system" fn(*mut ConsoleControlClient, Obj) -> HRESULT,
    pub get_system_time: unsafe extern "system" fn(*mut ConsoleControlClient, *mut SystemTime) -> HRESULT,
    pub get_config_value: unsafe extern "system" fn(*mut ConsoleControlClient, *const u16, *mut BSTR) -> HRESULT,
    pub set_config_value: unsafe extern "system" fn(*mut ConsoleControlClient, *const u16, *const u16) -> HRESULT,
    pub get_all_settings: unsafe extern "system" fn(*mut ConsoleControlClient, *mut *mut ConfigSetting, *mut u32) -> HRESULT,
    pub get_config_settings: unsafe extern "system" fn(*mut ConsoleControlClient, Obj) -> HRESULT,
    pub generate_stack_dump: unsafe extern "system" fn(*mut ConsoleControlClient, u32, u32, *const u16, *mut BSTR) -> HRESULT,
    pub query_connected_standby: unsafe extern "system" fn(*mut ConsoleControlClient, *mut i32, *mut i32) -> HRESULT,
    pub query_connected_standby_ex: unsafe extern "system" fn(*mut ConsoleControlClient, *mut i32, *mut i32, *mut i32) -> HRESULT,
}

pub static CLIENT_VTBL: ConsoleControlClientVtbl = ConsoleControlClientVtbl {
    query_interface: client_qi,
    add_ref: client_add_ref,
    release: client_release,
    shutdown_console: client_shutdown_console,
    updates_pending: client_updates_pending,
    get_running_processes: client_get_running_processes,
    get_system_time: client_get_system_time,
    get_config_value: client_get_config_value,
    set_config_value: client_set_config_value,
    get_all_settings: client_get_all_settings,
    get_config_settings: client_get_config_settings,
    generate_stack_dump: client_generate_stack_dump,
    query_connected_standby: client_query_connected_standby,
    query_connected_standby_ex: client_query_connected_standby_ex,
};

pub const CLIENT_SLOTS: usize = core::mem::size_of::<ConsoleControlClientVtbl>() / core::mem::size_of::<usize>();

#[repr(C)]
pub struct ConsoleControlClient {
    pub vtbl: *const ConsoleControlClientVtbl,
    refs: Mutex<RefState>,
    pub address: BSTR,
}

type CallbackFn<T> = unsafe extern "system" fn(Obj, *const T) -> HRESULT;

unsafe fn callback_slot3<T>(cb: Obj, arg: *const T) -> HRESULT {
    let f: CallbackFn<T> = core::mem::transmute(com::slot(cb, 3));
    f(cb, arg)
}

impl ConsoleControlClient {
    pub unsafe fn new(address: *const u16, create_result: &mut HRESULT) -> *mut ConsoleControlClient {
        let c = Box::into_raw(Box::new(ConsoleControlClient { vtbl: &CLIENT_VTBL, refs: Mutex::new(RefState { refs: 1, destroying: false }), address: null_mut() }));
        let mut hr = S_OK;
        if !address.is_null() && *address != 0 {
            (*c).address = SysAllocString(address);
            if (*c).address.is_null() {
                hr = E_OUTOFMEMORY;
            }
        }
        ck(CLIENT_CPP, "ConsoleControlClient::ConsoleControlClient", 39, "m_strAddress.Set(pszAddress)", hr);
        *create_result = hr;
        c
    }

    pub unsafe fn get_config_settings(&self, callback: Obj) -> HRESULT {
        const F: &str = "ConsoleControlClient::GetConfigSettings";
        if callback.is_null() {
            ck(CLIENT_CPP, F, 108, "E_INVALIDARG", E_INVALIDARG);
            return E_INVALIDARG;
        }
        let mut session = Com::null();
        let hr = open_vm_session(self.address, XBTP_PORT_SYSTEM, &CONSOLE_EXTENSION_ID, &mut session);
        ck(CLIENT_CPP, F, 115, "XtfOpenVMSession( m_strAddress, XBTP_PORT_SYSTEM, ConsoleExtensionId, IID_PPV_ARGS(&session))", hr);
        if hr < 0 {
            return hr;
        }
        let mut req = Com::null();
        let hr = xbtp::create_json_object(req.put());
        ck(CLIENT_CPP, F, 117, "XbtpCreateJsonObject(&reqBody)", hr);
        if hr < 0 {
            return hr;
        }
        let mut resp = Com::null();
        let hr = send_json_message(session.get(), req.get(), OP_GET_ALL_SETTINGS, Some(&mut resp));
        ck(CLIENT_CPP, F, 118, "XtfSendJsonMessage(session, reqBody, ConsoleGetAllConfigSettingsOperationId, &respBody)", hr);
        if hr < 0 {
            return hr;
        }
        let hr = check_task_errors(resp.get());
        ck(CLIENT_CPP, F, 119, "CheckTaskErrors(respBody)", hr);
        if hr < 0 {
            return hr;
        }
        let mut settings_body = Com::null();
        let n = w("SettingsArrayObject");
        let hr = json::get_object(resp.get(), n.as_ptr(), settings_body.put());
        ck(CLIENT_CPP, F, 120, "respBody->GetObject(AllSettingsArrayObjectProperty, &settingsBody)", hr);
        if hr < 0 {
            return hr;
        }
        let mut settings = Com::null();
        let n = w("AllSettings");
        let hr = json::get_array(settings_body.get(), n.as_ptr(), settings.put());
        ck(CLIENT_CPP, F, 122, "settingsBody->GetArray(AllSettingsArrayProperty, &settingsArray)", hr);
        if hr < 0 {
            return hr;
        }
        let mut count = 0u32;
        let mut hr = array::get_count(settings.get(), &mut count);
        ck(CLIENT_CPP, F, 123, "settingsArray->GetSize(&settingsCount)", hr);
        if hr < 0 {
            return hr;
        }
        let mut name: BSTR = null_mut();
        let mut value: BSTR = null_mut();
        let mut setting = Com::null();
        for i in 0..count {
            array::get_object(settings.get(), i, setting.put());
            free_bstr(name);
            name = null_mut();
            if !setting.is_null() {
                get_string(setting.get(), "SettingName", &mut name);
            }
            free_bstr(value);
            value = null_mut();
            if !setting.is_null() {
                get_string(setting.get(), "SettingValue", &mut value);
            }
            let pair = ConfigSetting { name, value };
            hr = callback_slot3(callback, &pair);
            if hr != S_OK {
                break;
            }
        }
        free_bstr(value);
        free_bstr(name);
        hr
    }

    pub unsafe fn get_all_settings(&self, out: *mut *mut ConfigSetting, out_count: *mut u32) -> HRESULT {
        const F: &str = "ConsoleControlClient::GetAllSettings";
        let mut session = Com::null();
        let hr = open_vm_session(self.address, XBTP_PORT_SYSTEM, &CONSOLE_EXTENSION_ID, &mut session);
        ck(CLIENT_CPP, F, 167, "XtfOpenVMSession( m_strAddress, XBTP_PORT_SYSTEM, ConsoleExtensionId, IID_PPV_ARGS(&session))", hr);
        if hr < 0 {
            return hr;
        }
        let mut req = Com::null();
        let hr = xbtp::create_json_object(req.put());
        ck(CLIENT_CPP, F, 169, "XbtpCreateJsonObject(&reqBody)", hr);
        if hr < 0 {
            return hr;
        }
        let mut resp = Com::null();
        let hr = send_json_message(session.get(), req.get(), OP_GET_ALL_SETTINGS, Some(&mut resp));
        ck(CLIENT_CPP, F, 170, "XtfSendJsonMessage(session, reqBody, ConsoleGetAllConfigSettingsOperationId, &respBody)", hr);
        if hr < 0 {
            return hr;
        }
        let hr = check_task_errors(resp.get());
        ck(CLIENT_CPP, F, 171, "CheckTaskErrors(respBody)", hr);
        if hr < 0 {
            return hr;
        }
        let mut settings_body = Com::null();
        let n = w("SettingsArrayObject");
        let hr = json::get_object(resp.get(), n.as_ptr(), settings_body.put());
        ck(CLIENT_CPP, F, 172, "respBody->GetObject(AllSettingsArrayObjectProperty, &settingsBody)", hr);
        if hr < 0 {
            return hr;
        }
        let mut settings = Com::null();
        let n = w("AllSettings");
        let hr = json::get_array(settings_body.get(), n.as_ptr(), settings.put());
        ck(CLIENT_CPP, F, 174, "settingsBody->GetArray(AllSettingsArrayProperty, &settingsArray)", hr);
        if hr < 0 {
            return hr;
        }
        let mut count = 0u32;
        let hr = array::get_count(settings.get(), &mut count);
        ck(CLIENT_CPP, F, 175, "settingsArray->GetSize(&settingsCount)", hr);
        if hr < 0 {
            return hr;
        }
        let list = CoTaskMemAlloc(count as usize * core::mem::size_of::<ConfigSetting>()) as *mut ConfigSetting;
        if list.is_null() && count != 0 {
            return E_OUTOFMEMORY;
        }
        let discard = |filled: u32| {
            for j in 0..filled as usize {
                free_bstr((*list.add(j)).name);
                free_bstr((*list.add(j)).value);
            }
            CoTaskMemFree(list as *mut c_void);
        };
        let mut hr = S_OK;
        for i in 0..count {
            let mut setting = Com::null();
            hr = array::get_object(settings.get(), i, setting.put());
            ck(CLIENT_CPP, F, 184, "settingsArray->GetObjectAt(i, &setting)", hr);
            if hr < 0 {
                discard(i);
                return hr;
            }
            let mut key: BSTR = null_mut();
            hr = get_string(setting.get(), "SettingName", &mut key);
            ck(CLIENT_CPP, F, 186, "setting->GetString(SettingNameProperty, &strKey)", hr);
            if hr < 0 {
                free_bstr(key);
                discard(i);
                return hr;
            }
            let mut value: BSTR = null_mut();
            hr = get_string(setting.get(), "SettingValue", &mut value);
            ck(CLIENT_CPP, F, 188, "setting->GetString(SettingValueProperty, &strValue)", hr);
            if hr < 0 {
                free_bstr(value);
                free_bstr(key);
                discard(i);
                return hr;
            }
            list.add(i as usize).write(ConfigSetting { name: key, value });
        }
        if out.is_null() || out_count.is_null() {
            discard(count);
            return E_POINTER;
        }
        *out = list;
        *out_count = count;
        hr
    }

    pub unsafe fn get_config_value(&self, key: *const u16, value: *mut BSTR) -> HRESULT {
        const F: &str = "ConsoleControlClient::GetConfigValue";
        let mut session = Com::null();
        let hr = open_vm_session(self.address, XBTP_PORT_SYSTEM, &CONSOLE_EXTENSION_ID, &mut session);
        ck(CLIENT_CPP, F, 220, "XtfOpenVMSession( m_strAddress, XBTP_PORT_SYSTEM, ConsoleExtensionId, IID_PPV_ARGS(&session))", hr);
        if hr < 0 {
            return hr;
        }
        let mut req = Com::null();
        let hr = xbtp::create_json_object(req.put());
        ck(CLIENT_CPP, F, 222, "XbtpCreateJsonObject(&reqBody)", hr);
        if hr < 0 {
            return hr;
        }
        let hr = set_string(req.get(), "ConfigKey", key);
        ck(CLIENT_CPP, F, 223, "reqBody->SetString(ConfigKeyPropertyName, pszKey)", hr);
        if hr < 0 {
            return hr;
        }
        let mut resp = Com::null();
        let hr = send_json_message(session.get(), req.get(), OP_GET_CONFIG_VALUE, Some(&mut resp));
        ck(CLIENT_CPP, F, 224, "XtfSendJsonMessage(session, reqBody, ConsoleGetConfigValueOperationId, &respBody)", hr);
        if hr < 0 {
            return hr;
        }
        let hr = check_task_errors(resp.get());
        ck(CLIENT_CPP, F, 225, "CheckTaskErrors(respBody)", hr);
        if hr < 0 {
            return hr;
        }
        if value.is_null() {
            return E_POINTER;
        }
        let hr = get_string(resp.get(), "ConfigValue", value);
        ck(CLIENT_CPP, F, 226, "respBody->GetString(ConfigValuePropertyName, pbstrValue)", hr);
        hr
    }

    pub unsafe fn set_config_value(&self, key: *const u16, value: *const u16) -> HRESULT {
        const F: &str = "ConsoleControlClient::SetConfigValue";
        if wcsicmp_eq(key, "AccessKey") || wcsicmp_eq(key, "SessionKey") {
            let hr = verify_session_key(value);
            if hr < 0 {
                customer_string(ACCESS_KEY_MESSAGE, &[value as usize, MAX_ACCESS_KEY_CHARS]);
                ck(CLIENT_CPP, F, 257, "hr", hr);
                return hr;
            }
        }
        let mut session = Com::null();
        let hr = open_vm_session(self.address, XBTP_PORT_SYSTEM, &CONSOLE_EXTENSION_ID, &mut session);
        ck(CLIENT_CPP, F, 265, "XtfOpenVMSession( m_strAddress, XBTP_PORT_SYSTEM, ConsoleExtensionId, IID_PPV_ARGS(&session))", hr);
        if hr < 0 {
            return hr;
        }
        let mut req = Com::null();
        let hr = xbtp::create_json_object(req.put());
        ck(CLIENT_CPP, F, 267, "XbtpCreateJsonObject(&reqBody)", hr);
        if hr < 0 {
            return hr;
        }
        let hr = set_string(req.get(), "ConfigKey", key);
        ck(CLIENT_CPP, F, 268, "reqBody->SetString( ConfigKeyPropertyName, pszKey)", hr);
        if hr < 0 {
            return hr;
        }
        let hr = set_string(req.get(), "ConfigValue", value);
        ck(CLIENT_CPP, F, 269, "reqBody->SetString( ConfigValuePropertyName, pszValue)", hr);
        if hr < 0 {
            return hr;
        }
        let mut resp = Com::null();
        let hr = send_json_message(session.get(), req.get(), OP_SET_CONFIG_VALUE, Some(&mut resp));
        if hr == E_WSAECONNRESET && wcsicmp_eq(key, "ipaddress") {
            return S_OK;
        }
        ck(CLIENT_CPP, F, 279, "hr", hr);
        if hr < 0 {
            return hr;
        }
        let hr = check_task_errors(resp.get());
        ck(CLIENT_CPP, F, 280, "CheckTaskErrors(respBody)", hr);
        hr
    }

    pub unsafe fn generate_stack_dump(&self, process_id: u32, dump_type: u32, image_name: *const u16, file_path: *mut BSTR) -> HRESULT {
        const F: &str = "ConsoleControlClient::GenerateStackDump";
        let mut session = Com::null();
        let hr = open_vm_session(self.address, XBTP_PORT_TITLE, &CONSOLE_EXTENSION_ID, &mut session);
        ck(CLIENT_CPP, F, 318, "XtfOpenVMSession( m_strAddress, XBTP_PORT_TITLE, ConsoleExtensionId, IID_PPV_ARGS(&session))", hr);
        if hr < 0 {
            return hr;
        }
        let mut req = Com::null();
        let hr = xbtp::create_json_object(req.put());
        ck(CLIENT_CPP, F, 320, "XbtpCreateJsonObject(&reqBody)", hr);
        if hr < 0 {
            return hr;
        }
        let pid = uint32_to_bstr(process_id);
        let hr = set_string(req.get(), "ProcessId", pid);
        free_bstr(pid);
        ck(CLIENT_CPP, F, 322, "reqBody->SetString(ProcessIdERAProperty, processIdString)", hr);
        if hr < 0 {
            return hr;
        }
        let ty = uint32_to_bstr(dump_type);
        let hr = set_string(req.get(), "DumpType", ty);
        free_bstr(ty);
        ck(CLIENT_CPP, F, 323, "reqBody->SetString(DumpTypeProperty, ConvertUINT32ToBSTR(DumpType))", hr);
        if hr < 0 {
            return hr;
        }
        let hr = set_string(req.get(), "ImageName", image_name);
        ck(CLIENT_CPP, F, 324, "reqBody->SetString(ExecutableImageNameProperty, ImageName)", hr);
        if hr < 0 {
            return hr;
        }
        let mut resp = Com::null();
        let hr = send_json_message_timeout(session.get(), req.get(), OP_GENERATE_STACK_DUMP, STACK_DUMP_REPLY_TIMEOUT_MS, Some(&mut resp));
        ck(CLIENT_CPP, F, 329, "XtfSendJsonMessage(session, reqBody, GenerateStackDumpOperationId, c_ReplyTimeoutMS, &respBody)", hr);
        if hr < 0 {
            return hr;
        }
        let hr = check_task_errors(resp.get());
        ck(CLIENT_CPP, F, 331, "CheckTaskErrors(respBody)", hr);
        if hr < 0 {
            return hr;
        }
        if file_path.is_null() {
            return E_POINTER;
        }
        let hr = get_string(resp.get(), "FilePath", file_path);
        ck(CLIENT_CPP, F, 332, "respBody->GetString(DumpFilePathProperty, FilePath)", hr);
        hr
    }

    pub unsafe fn updates_pending(&self, pending: *mut i32) -> HRESULT {
        const F: &str = "ConsoleControlClient::UpdatesPending";
        if pending.is_null() {
            return E_INVALIDARG;
        }
        let mut is_pending = false;
        let hr = 'run: {
            let mut session = Com::null();
            let hr = open_vm_session(self.address, XBTP_PORT_SYSTEM, &CONSOLE_EXTENSION_ID, &mut session);
            ck(UPDATESPENDING_CPP, F, 74, "XtfOpenVMSession( m_strAddress, XBTP_PORT_SYSTEM, ConsoleExtensionId, IID_PPV_ARGS(&session))", hr);
            if hr < 0 {
                break 'run hr;
            }
            let mut req = Com::null();
            let hr = xbtp::create_json_object(req.put());
            ck(UPDATESPENDING_CPP, F, 76, "XbtpCreateJsonObject(&reqBody)", hr);
            if hr < 0 {
                break 'run hr;
            }
            let mut resp = Com::null();
            let hr = send_json_message(session.get(), req.get(), OP_UPDATES_PENDING, Some(&mut resp));
            ck(UPDATESPENDING_CPP, F, 77, "XtfSendJsonMessage(session, reqBody, ConsoleUpdatesPendingOperationId, &respBody)", hr);
            if hr < 0 {
                break 'run hr;
            }
            let hr = check_task_errors(resp.get());
            ck(UPDATESPENDING_CPP, F, 78, "CheckTaskErrors(respBody)", hr);
            if hr < 0 {
                break 'run hr;
            }
            let n = w("UpdatesPending");
            let hr = get_json_bool(resp.get(), n.as_ptr(), &mut is_pending);
            ck(UPDATESPENDING_CPP, F, 79, "XtfGetJsonProperty(respBody, UpdatesPendingProperty, &isPending)", hr);
            hr
        };
        *pending = is_pending as i32;
        hr
    }

    pub unsafe fn get_system_time(&self, out: *mut SystemTime) -> HRESULT {
        const F: &str = "ConsoleControlClient::GetSystemTime";
        if out.is_null() {
            return E_INVALIDARG;
        }
        let mut st = SystemTime::default();
        let hr = 'run: {
            let mut session = Com::null();
            let hr = open_vm_session(self.address, XBTP_PORT_SYSTEM, &CONSOLE_EXTENSION_ID, &mut session);
            ck(SYSTEMTIME_CPP, F, 26, "XtfOpenVMSession( m_strAddress, XBTP_PORT_SYSTEM, ConsoleExtensionId, IID_PPV_ARGS(&session))", hr);
            if hr < 0 {
                break 'run hr;
            }
            let mut req = Com::null();
            let hr = xbtp::create_json_object(req.put());
            ck(SYSTEMTIME_CPP, F, 28, "XbtpCreateJsonObject(&reqBody)", hr);
            if hr < 0 {
                break 'run hr;
            }
            let mut resp = Com::null();
            let hr = send_json_message(session.get(), req.get(), OP_GET_SYSTEM_TIME, Some(&mut resp));
            ck(SYSTEMTIME_CPP, F, 29, "XtfSendJsonMessage(session, reqBody, ConsoleGetSystemTimeOperationId, &respBody)", hr);
            if hr < 0 {
                break 'run hr;
            }
            let hr = check_task_errors(resp.get());
            ck(SYSTEMTIME_CPP, F, 30, "CheckTaskErrors(respBody)", hr);
            if hr < 0 {
                break 'run hr;
            }
            let mut ft = 0u64;
            let hr = get_json_u64_property(resp.get(), "ConsoleTime", &mut ft);
            ck(SYSTEMTIME_CPP, F, 31, "XtfGetJsonProperty<UINT64>(respBody, SystemTimeProperty, (UINT64 *)fTime)", hr);
            if hr < 0 {
                break 'run hr;
            }
            FileTimeToSystemTime(&ft, &mut st);
            hr
        };
        *out = st;
        hr
    }

    pub unsafe fn get_running_processes(&self, callback: Obj) -> HRESULT {
        const F: &str = "ConsoleControlClient::GetRunningProcesses";
        let mut address: BSTR = null_mut();
        let mut port = 0u32;
        let mut hr = split_console_address(self.address, XBTP_PORT_SYSTEM, &mut address, null_mut(), &mut port);
        if hr < 0 {
            la("SplitConsoleAddress( connectionString, defaultPort, consoleAddress, nullptr, port)", hr, CONSOLE_ADDRESS_CPP, 0xd7);
        } else {
            hr = S_OK;
        }
        ck(RUNNINGPROCESSES_CPP, F, 370, "SplitConsoleAddress( m_strAddress, XBTP_PORT_SYSTEM, &address, &port)", hr);
        let hr = 'run: {
            if hr < 0 {
                break 'run hr;
            }
            let mut session = Com::null();
            let hr = open_vm_session(address, port, &CONSOLE_EXTENSION_ID, &mut session);
            ck(RUNNINGPROCESSES_CPP, F, 376, "XtfOpenVMSession( address, port, ConsoleExtensionId, IID_PPV_ARGS(&session))", hr);
            if hr < 0 {
                break 'run hr;
            }
            let mut req = Com::null();
            let hr = xbtp::create_json_object(req.put());
            ck(RUNNINGPROCESSES_CPP, F, 378, "XbtpCreateJsonObject(&reqBody)", hr);
            if hr < 0 {
                break 'run hr;
            }
            let mut resp = Com::null();
            let hr = send_json_message(session.get(), req.get(), OP_GET_RUNNING_PROCESSES, Some(&mut resp));
            ck(RUNNINGPROCESSES_CPP, F, 379, "XtfSendJsonMessage(session, reqBody, ConsoleGetRunningProcessesOperationId, &respBody)", hr);
            if hr < 0 {
                break 'run hr;
            }
            let checked = check_task_errors(resp.get());
            ck(RUNNINGPROCESSES_CPP, F, 380, "CheckTaskErrors(respBody)", checked);
            if checked < 0 {
                break 'run checked;
            }
            let mut processes = Com::null();
            let n = w("Processes");
            if json::get_array(resp.get(), n.as_ptr(), processes.put()) < 0 {
                break 'run checked;
            }
            let mut count = 0u32;
            let mut hr = array::get_count(processes.get(), &mut count);
            ck(RUNNINGPROCESSES_CPP, F, 385, "processes->GetSize(&cProcesses)", hr);
            if hr < 0 {
                break 'run hr;
            }
            for i in 0..count {
                let mut process = Com::null();
                hr = array::get_object(processes.get(), i, process.put());
                ck(RUNNINGPROCESSES_CPP, F, 390, "processes->GetObjectAt(i, &process)", hr);
                if hr < 0 {
                    break 'run hr;
                }
                let mut name: BSTR = null_mut();
                let got = get_string(process.get(), "ProcessName", &mut name);
                if got < 0 || name.is_null() || *name == 0 {
                    let unknown = w("<unknown>");
                    let b = SysAllocString(unknown.as_ptr());
                    if !b.is_null() {
                        free_bstr(name);
                        name = b;
                    }
                }
                let mut pid = 0u32;
                let n = w("ProcessId");
                if get_json_u32(process.get(), n.as_ptr(), &mut pid) < 0 {
                    pid = 0;
                }
                let info = ProcessInfo { process_id: pid, name };
                hr = if callback.is_null() { E_POINTER } else { callback_slot3(callback, &info) };
                ck(RUNNINGPROCESSES_CPP, F, 410, "pCallback->OnFoundProcess(&xpi)", hr);
                free_bstr(name);
                if hr < 0 {
                    break 'run hr;
                }
            }
            hr
        };
        free_bstr(address);
        hr
    }

    pub unsafe fn shutdown_console(&self, flags: u32) -> HRESULT {
        const F: &str = "ConsoleControlClient::ShutdownConsole";
        let mut address: BSTR = null_mut();
        let mut port = 0u32;
        let mut hr = split_console_address(self.address, XBTP_PORT_SYSTEM, &mut address, null_mut(), &mut port);
        if hr < 0 {
            la("SplitConsoleAddress( connectionString, defaultPort, consoleAddress, nullptr, port)", hr, CONSOLE_ADDRESS_CPP, 0xd7);
        } else {
            hr = S_OK;
        }
        ck(SHUTDOWN_CPP, F, 265, "SplitConsoleAddress( m_strAddress, XBTP_PORT_SYSTEM, &consoleAddress, &port)", hr);
        let result = 'run: {
            if hr < 0 {
                break 'run hr;
            }
            if port == XBTP_PORT_TITLE {
                port = XBTP_PORT_SYSTEM;
            }
            let deadline = GetTickCount64() + SHUTDOWN_NOT_READY_WINDOW_MS;
            let mut session = Com::null();
            let mut hr = hr;
            loop {
                if hr == E_SYSTEM_NOT_READY {
                    Sleep(SHUTDOWN_NOT_READY_SLEEP_MS);
                }
                hr = open_vm_session(address, port, &CONSOLE_EXTENSION_ID, &mut session);
                if hr != E_SYSTEM_NOT_READY || GetTickCount64() >= deadline {
                    break;
                }
            }
            let mut resp = Com::null();
            if hr >= 0 {
                hr = send_reboot_command(session.get(), flags, Some(&mut resp));
            }
            if hr >= 0 {
                if port != XBTP_PORT_HOST && flags & 2 == 0 {
                    send_system_go_dark_to(address, XBTP_PORT_HOST);
                    send_system_go_dark_to(address, XBTP_PORT_SYSTEM);
                    send_system_go_dark_to(address, XBTP_PORT_TITLE);
                }
                ck(SHUTDOWN_CPP, F, 325, "hr", hr);
            } else if port == XBTP_PORT_HOST {
                ck(SHUTDOWN_CPP, F, 325, "hr", hr);
                break 'run hr;
            } else {
                hr = open_vm_session(address, XBTP_PORT_HOST, &XTF_PROVISION_EXTENSION_ID, &mut session);
                ck(SHUTDOWN_CPP, F, 319, "XtfOpenVMSession( consoleAddress, XBTP_PORT_HOST, XtfProvisionExtensionId, IID_PPV_ARGS(&session))", hr);
                if hr < 0 {
                    break 'run hr;
                }
                hr = send_json_message(session.get(), null_mut(), OP_REBOOT_V1, None);
                ck(SHUTDOWN_CPP, F, 321, "XtfSendJsonMessage(session, nullptr, RebootV1OperationId, nullptr)", hr);
            }
            if hr < 0 || resp.is_null() {
                break 'run hr;
            }
            let mut error = OperationError::default();
            let hr = error.initialize_body(resp.get());
            ck(SHUTDOWN_CPP, F, 331, "errorCheck.Initialize(respBody)", hr);
            if hr < 0 {
                break 'run hr;
            }
            ck(SHUTDOWN_CPP, F, 332, "errorCheck.GetReturnCode(&hr)", S_OK);
            ck(SHUTDOWN_CPP, F, 333, "hr", S_OK);
            S_OK
        };
        free_bstr(address);
        result
    }

    pub unsafe fn query_connected_standby_ex(&self, in_standby: *mut i32, allows_instant_on: *mut i32, allow_xbox_app: *mut i32) -> HRESULT {
        const F: &str = "ConsoleControlClient::QueryConnectedStandbyEx";
        if in_standby.is_null() || allows_instant_on.is_null() || allow_xbox_app.is_null() {
            return E_POINTER;
        }
        let mut is_in_cs = false;
        let mut instant_on = false;
        let mut xbox_app = false;
        let hr = 'run: {
            let mut session = Com::null();
            let mut hr = S_OK;
            for _ in 0..STANDBY_OPEN_ATTEMPTS {
                session.reset();
                hr = open_vm_session_timeout(self.address, Some(STANDBY_OPEN_TIMEOUT_MS), session.put());
                if hr >= 0 {
                    break;
                }
            }
            ck(CONNECTEDSTANDBY_CPP, F, 59, "hr", hr);
            if hr < 0 {
                break 'run hr;
            }
            let mut req = Com::null();
            let hr = xbtp::create_json_object(req.put());
            ck(CONNECTEDSTANDBY_CPP, F, 61, "XbtpCreateJsonObject(&reqBody)", hr);
            if hr < 0 {
                break 'run hr;
            }
            let mut resp = Com::null();
            let hr = send_json_message(session.get(), req.get(), OP_IN_CONNECTED_STANDBY, Some(&mut resp));
            ck(CONNECTEDSTANDBY_CPP, F, 62, "XtfSendJsonMessage(session, reqBody, ConsoleInConnectedStandbyOperationId, &respBody)", hr);
            if hr < 0 {
                break 'run hr;
            }
            let hr = check_task_errors(resp.get());
            ck(CONNECTEDSTANDBY_CPP, F, 63, "CheckTaskErrors(respBody)", hr);
            if hr < 0 {
                break 'run hr;
            }
            let n = w("InConnectedStandby");
            let hr = get_json_bool(resp.get(), n.as_ptr(), &mut is_in_cs);
            ck(CONNECTEDSTANDBY_CPP, F, 64, "XtfGetJsonProperty(respBody, InConnectedStandbyProperty, &isInCS)", hr);
            if hr < 0 {
                break 'run hr;
            }
            let n = w("AllowsInstantOn");
            let hr = get_json_bool(resp.get(), n.as_ptr(), &mut instant_on);
            ck(CONNECTEDSTANDBY_CPP, F, 65, "XtfGetJsonProperty(respBody, AllowsInstantOnProperty, &allowsInstantOn)", hr);
            if hr < 0 {
                break 'run hr;
            }
            let n = w("AllowXboxAppConnections");
            let r = get_json_bool(resp.get(), n.as_ptr(), &mut xbox_app);
            let hr = if r == E_JSON_NOT_FOUND { S_OK } else { r };
            ck(CONNECTEDSTANDBY_CPP, F, 69, "XtfTryGetJsonProperty(respBody, AllowsXboxAppConnectionsProperty, nullptr, &allowXboxAppConnections)", hr);
            hr
        };
        *in_standby = is_in_cs as i32;
        *allows_instant_on = instant_on as i32;
        *allow_xbox_app = xbox_app as i32;
        hr
    }
}

pub unsafe fn uint32_to_bstr(v: u32) -> BSTR {
    let t = w(&(v as i32).to_string());
    SysAllocString(t.as_ptr())
}

impl Drop for ConsoleControlClient {
    fn drop(&mut self) {
        unsafe {
            free_bstr(self.address);
            self.address = null_mut();
        }
    }
}

unsafe extern "system" fn client_qi(this: *mut ConsoleControlClient, iid: *const GUID, out: *mut Obj) -> HRESULT {
    if iid.is_null() || out.is_null() {
        return E_POINTER;
    }
    let iid = &*iid;
    if *iid == IID_XTF_CONSOLE_CONTROL_CLIENT || *iid == IID_IUNKNOWN || *iid == IID_XTF_CONSOLE_CONTROL_CLIENT_BASE {
        *out = this as Obj;
        client_add_ref(this);
        return S_OK;
    }
    E_NOINTERFACE
}

unsafe extern "system" fn client_add_ref(this: *mut ConsoleControlClient) -> u32 {
    let mut g = (*this).refs.lock().unwrap_or_else(|e| e.into_inner());
    g.refs += 1;
    g.refs
}

unsafe extern "system" fn client_release(this: *mut ConsoleControlClient) -> u32 {
    let n = {
        let mut g = (*this).refs.lock().unwrap_or_else(|e| e.into_inner());
        g.refs -= 1;
        if g.refs != 0 || g.destroying {
            return g.refs;
        }
        g.destroying = true;
        g.refs
    };
    drop(Box::from_raw(this));
    n
}

unsafe extern "system" fn client_shutdown_console(this: *mut ConsoleControlClient, flags: u32) -> HRESULT {
    (*this).shutdown_console(flags)
}

unsafe extern "system" fn client_updates_pending(this: *mut ConsoleControlClient, pending: *mut i32) -> HRESULT {
    (*this).updates_pending(pending)
}

unsafe extern "system" fn client_get_running_processes(this: *mut ConsoleControlClient, callback: Obj) -> HRESULT {
    (*this).get_running_processes(callback)
}

unsafe extern "system" fn client_get_system_time(this: *mut ConsoleControlClient, out: *mut SystemTime) -> HRESULT {
    (*this).get_system_time(out)
}

unsafe extern "system" fn client_get_config_value(this: *mut ConsoleControlClient, key: *const u16, value: *mut BSTR) -> HRESULT {
    (*this).get_config_value(key, value)
}

unsafe extern "system" fn client_set_config_value(this: *mut ConsoleControlClient, key: *const u16, value: *const u16) -> HRESULT {
    (*this).set_config_value(key, value)
}

unsafe extern "system" fn client_get_all_settings(this: *mut ConsoleControlClient, out: *mut *mut ConfigSetting, count: *mut u32) -> HRESULT {
    (*this).get_all_settings(out, count)
}

unsafe extern "system" fn client_get_config_settings(this: *mut ConsoleControlClient, callback: Obj) -> HRESULT {
    (*this).get_config_settings(callback)
}

unsafe extern "system" fn client_generate_stack_dump(this: *mut ConsoleControlClient, pid: u32, dump_type: u32, image: *const u16, path: *mut BSTR) -> HRESULT {
    (*this).generate_stack_dump(pid, dump_type, image, path)
}

unsafe extern "system" fn client_query_connected_standby(this: *mut ConsoleControlClient, in_standby: *mut i32, allows_instant_on: *mut i32) -> HRESULT {
    let mut dummy = 0i32;
    ((*(*this).vtbl).query_connected_standby_ex)(this, in_standby, allows_instant_on, &mut dummy)
}

unsafe extern "system" fn client_query_connected_standby_ex(this: *mut ConsoleControlClient, in_standby: *mut i32, allows_instant_on: *mut i32, allow_xbox_app: *mut i32) -> HRESULT {
    (*this).query_connected_standby_ex(in_standby, allows_instant_on, allow_xbox_app)
}

#[no_mangle]
pub unsafe extern "system" fn XtfCreateConsoleControlClient(address: *const u16, riid: *const GUID, ppv: *mut Obj) -> HRESULT {
    const F: &str = "XtfCreateConsoleControlClient";
    if ppv.is_null() {
        return E_POINTER;
    }
    *ppv = null_mut();
    if address.is_null() {
        ck(CLIENT_CPP, F, 19, "E_INVALIDARG", E_INVALIDARG);
        return E_INVALIDARG;
    }
    let mut create_result = S_OK;
    let client = ConsoleControlClient::new(address, &mut create_result);
    ckm(CLIENT_CPP, F, 22, "hr", create_result, "spClient.Attach(new (std::nothrow) ConsoleControlClient(pszAddress, &hr))");
    let hr = if create_result < 0 {
        create_result
    } else {
        let hr = client_qi(client, riid, ppv);
        ck(CLIENT_CPP, F, 23, "spClient->QueryInterface(riid, ppvObject)", hr);
        hr
    };
    client_release(client);
    hr
}
