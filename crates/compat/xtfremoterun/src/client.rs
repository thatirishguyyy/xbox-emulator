use crate::chk::chk;
use crate::console::*;
use crate::server::{RUN_FLAG_REDIRECT_INPUT, RUN_FLAG_REDIRECT_MASK, RUN_FLAG_REDIRECT_OUTPUT, RUN_FLAG_WAIT, RUN_OPERATION_ID};
use crate::win::{GetTickCount64, Sleep, E_POINTER};
use crate::variant::ConsoleCopy;
use crate::REMOTE_RUN_EXTENSION_ID;
use core::ptr::null_mut;
use std::sync::Mutex;
use xtfmsg::com::{self, json, Com, Obj, IID_XBTP_CLIENT_SESSION, IID_XBTP_MESSAGE};
use xtfmsg::msg::{cvtt32, cvtt64, deserialize_body, get_json_u32, log_api, serialize_body, set_number, API_CPP};
use xtfmsg::sys::*;
use xtfmsg::xbtp;

pub const CLIENT_CPP: &str = "xbox\\devex\\xtf\\dll\\remoterun\\client.cpp";
pub const CONSOLE_ADDRESS_CPP: &[u8] = b"xbox\\devex\\xtf\\transport\\xtfmessaging\\consoleaddresshelpers.cpp\0";
pub const OPERATION_ERROR_CPP: &[u8] = b"xbox\\devex\\xtf\\transport\\xtfmessaging\\xtfoperationerror.cpp\0";

pub const XBTP_PORT_SYSTEM: u32 = 0;
pub const XBTP_PORT_TITLE: u32 = 1;
pub const XBTP_PORT_HOST: u32 = 3;
pub const XBTP_PORT_KIND_LINK: u32 = 0x0100_0000;
pub const XBTP_PORT_KIND_MASK: u32 = 0xff00_0000;
pub const LOOPBACK_ADDRESS: &str = "127.0.0.1";

pub const XBTP_MESSAGE_TYPE_MORE: i16 = 3;
pub const XBTP_MESSAGE_TYPE_END: i16 = 4;

pub const E_COULD_NOT_CONNECT: HRESULT = 0x8c11_0303u32 as i32;
pub const E_NO_IPADDRESS_IN_CONNSTR: HRESULT = 0x8c11_0407u32 as i32;
pub const E_INVALID_VM_TARGET: HRESULT = 0x8c11_0408u32 as i32;
pub const E_SEM_TIMEOUT: HRESULT = 0x8007_0079u32 as i32;
pub const E_XTFRUN_RECEIVE_TIMED_OUT: HRESULT = 0x8d16_0198u32 as i32;

pub const DEFAULT_OPEN_SESSION_TIMEOUT_MS: u64 = 30000;
pub const MAX_ADDRESS_CHARS: usize = 0x200;
pub const SESSION_KEY_CHARS: usize = 0x20;
pub const OPEN_OPTIONS_SIZE: u32 = 0x58;

pub const IID_XTF_REMOTE_RUN_CLIENT: GUID = GUID::new(0x73638f02, 0x1aea, 0x49ad, [0xb0, 0xe3, 0x2c, 0x95, 0x95, 0x02, 0x15, 0x26]);
pub const IID_XTF_REMOTE_RUN_CLIENT_BASE: GUID = GUID::new(0x9c95eba2, 0x9114, 0x4f58, [0x9f, 0x49, 0xfb, 0xff, 0xa2, 0x36, 0x3d, 0x77]);
pub const IID_XTF_REMOTE_RUN_CLIENT2: GUID = GUID::new(0x85627158, 0x492c, 0x4cde, [0x86, 0xe8, 0xaf, 0x22, 0xc3, 0xfb, 0x90, 0x7c]);
pub const IID_XTF_REMOTE_RUN_CALLBACK_EX: GUID = GUID::new(0xd74bf3b9, 0x6814, 0x4e32, [0x84, 0xb1, 0xd1, 0x06, 0xf4, 0xa3, 0x97, 0x2b]);
pub const IID_IUNKNOWN: GUID = GUID::new(0, 0, 0, [0xc0, 0, 0, 0, 0, 0, 0, 0x46]);

#[cfg(test)]
pub static TEST_LINK_PORT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

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
pub struct RemoteRunClientVtbl {
    pub query_interface: unsafe extern "system" fn(*mut RemoteRunClient, *const GUID, *mut Obj) -> HRESULT,
    pub add_ref: unsafe extern "system" fn(*mut RemoteRunClient) -> u32,
    pub release: unsafe extern "system" fn(*mut RemoteRunClient) -> u32,
    pub run: unsafe extern "system" fn(*mut RemoteRunClient, *const u16, *const u16, u32, u32, Obj) -> HRESULT,
}

#[repr(C)]
pub struct RemoteRunClient2Vtbl {
    pub query_interface: unsafe extern "system" fn(Obj, *const GUID, *mut Obj) -> HRESULT,
    pub add_ref: unsafe extern "system" fn(Obj) -> u32,
    pub release: unsafe extern "system" fn(Obj) -> u32,
    pub run_ex: unsafe extern "system" fn(Obj, *const u16, *const u16, u32, u32, Obj, *mut u32) -> HRESULT,
}

pub static CLIENT_VTBL: RemoteRunClientVtbl = RemoteRunClientVtbl { query_interface: client_qi, add_ref: client_add_ref, release: client_release, run: client_run };
pub static CLIENT2_VTBL: RemoteRunClient2Vtbl = RemoteRunClient2Vtbl { query_interface: ex_qi, add_ref: ex_add_ref, release: ex_release, run_ex: ex_run_ex };

struct RefState {
    refs: u32,
    destroying: bool,
}

#[repr(C)]
pub struct RemoteRunClient {
    pub vtbl: *const RemoteRunClientVtbl,
    pub vtbl2: *const RemoteRunClient2Vtbl,
    refs: Mutex<RefState>,
    pub address: BSTR,
    pub copy: ConsoleCopy,
    session: Mutex<Com>,
}

fn ckc(func: &str, line: u32, name: &str, hr: HRESULT, msg: &str) {
    chk(CLIENT_CPP, func, line, name, hr, msg);
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
        return hr;
    }
    *ip = a;
    S_OK
}

type OpenSession2Fn = unsafe extern "C" fn(*const u16, u32, *const OpenSessionOptions, *const GUID, *const GUID, *mut Obj) -> HRESULT;

unsafe fn xbtp_open_session2(ip: *const u16, port: u32, opts: *const OpenSessionOptions, ext: *const GUID, iid: *const GUID, out: *mut Obj) -> HRESULT {
    #[cfg(test)]
    {
        let p = TEST_LINK_PORT.load(std::sync::atomic::Ordering::SeqCst);
        if p != 0 && port & XBTP_PORT_KIND_MASK == XBTP_PORT_KIND_LINK && port & 0x00ff_ffff == XBTP_PORT_SYSTEM {
            return match xbtp::proc(b"XbtpOpenSessionEx2\0") {
                Ok(f) => {
                    let f: OpenSession2Fn = core::mem::transmute(f);
                    f(ip, XBTP_PORT_KIND_LINK | p, opts, ext, iid, out)
                }
                Err(e) => e,
            };
        }
    }
    match xbtp::proc(xbtp::OPEN_SESSION2) {
        Ok(p) => {
            let f: OpenSession2Fn = core::mem::transmute(p);
            f(ip, port, opts, ext, iid, out)
        }
        Err(e) => e,
    }
}

pub unsafe fn open_session_impl(conn: *const u16, override_port: u32, ext: *const GUID, iid: *const GUID, out: *mut Obj) -> HRESULT {
    let mut last = GetTickCount64();
    let mut ip: BSTR = null_mut();
    let mut key: BSTR = null_mut();
    let mut explicit = 0u32;
    let mut hr = split_console_address(conn, override_port, &mut ip, &mut key, &mut explicit);
    if hr < 0 {
        log_api(b"SplitConsoleAddress( connectionString, port, &consoleIpAddress, &sessionKey, &explicitPort)\0", hr, API_CPP, 0xb2);
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
        if hr != E_COULD_NOT_CONNECT {
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
        log_api(b"hr\0", hr, API_CPP, 0x105);
    }
    free_bstr(key);
    free_bstr(ip);
    hr
}

pub unsafe fn open_vm_session(conn: *const u16, port: u32, out: *mut Obj) -> HRESULT {
    let hr = open_session_impl(conn, port, &REMOTE_RUN_EXTENSION_ID, &IID_XBTP_CLIENT_SESSION, out);
    if hr < 0 {
        log_api(b"XtfOpenSessionImpl( connectionString, overridePort, true, nullptr, extensionId, iid, ppvObject, &kDefaultOpenSessionTimeoutMS)\0", hr, API_CPP, 0x11f);
        return hr;
    }
    S_OK
}

pub unsafe fn open_session_impl_19041(conn: *const u16, port: u32, ext: *const GUID, iid: *const GUID, out: *mut Obj) -> HRESULT {
    let mut ip: BSTR = null_mut();
    let mut key: BSTR = null_mut();
    let mut explicit = 0u32;
    let hr = split_console_address(conn, port, &mut ip, &mut key, &mut explicit);
    if hr < 0 {
        log_api(b"SplitConsoleAddress( connectionString, port, &consoleIpAddress, &sessionKey, &explicitPort)\0", hr, API_CPP, 0xa4);
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
    let hr = xbtp_open_session2(ip, port, &opts, ext, iid, out);
    if hr < 0 {
        log_api(b"XbtpOpenSession2( consoleIpAddress, explicitPort, &auxInfo, extensionId, iid, ppvObject)\0", hr, API_CPP, 0xc6);
    }
    free_bstr(key);
    free_bstr(ip);
    hr
}

pub unsafe fn open_vm_session_19041(conn: *const u16, port: u32, out: *mut Obj) -> HRESULT {
    let hr = open_session_impl_19041(conn, port, &REMOTE_RUN_EXTENSION_ID, &IID_XBTP_CLIENT_SESSION, out);
    if hr < 0 {
        log_api(b"XtfOpenSessionImpl( connectionString, overridePort, true, nullptr, extensionId, iid, ppvObject, &kDefaultOpenSessionTimeoutMS)\0", hr, API_CPP, 0xe0);
    }
    hr
}

type ReceiveMessageFn = unsafe extern "system" fn(Obj, u32, u32, *mut i16, *mut i32, *mut Obj) -> HRESULT;

pub unsafe fn session_receive_message(session: Obj, timeout: u32, exchange: u32, message_type: &mut i16, status: &mut i32, out: *mut Obj) -> HRESULT {
    let f: ReceiveMessageFn = core::mem::transmute(com::slot(session, 8));
    f(session, timeout, exchange, message_type, status, out)
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
            log_api(b"E_INVALIDARG\0", E_INVALIDARG, OPERATION_ERROR_CPP, 0x2b);
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
            log_api(b"errorBody->GetNumber(XbtpEndErrorReturnCodeName, &rc)\0", hr, OPERATION_ERROR_CPP, 0x39);
            return hr;
        }
        self.return_code = cvtt32(rc);
        let mut cp = 0f64;
        let n = w("Checkpoint");
        let hr = json::get_number(eb.get(), n.as_ptr(), &mut cp);
        if hr < 0 {
            log_api(b"errorBody->GetNumber(XbtpEndErrorCheckpointName, &cp)\0", hr, OPERATION_ERROR_CPP, 0x3e);
            return hr;
        }
        self.checkpoint = cvtt64(cp) as u32;
        let mut errors = Com::null();
        let n = w("Errors");
        let hr = json::get_array(eb.get(), n.as_ptr(), errors.put());
        self.errors = Some(errors);
        if hr < 0 {
            log_api(b"errorBody->GetArray(XbtpEndErrorErrorsName, &m_errors)\0", hr, OPERATION_ERROR_CPP, 0x42);
            return hr;
        }
        S_OK
    }

    pub unsafe fn initialize(&mut self, reply: Obj) -> HRESULT {
        if reply.is_null() {
            log_api(b"E_INVALIDARG\0", E_INVALIDARG, OPERATION_ERROR_CPP, 0x15);
            return E_INVALIDARG;
        }
        let mut body = Com::null();
        let hr = deserialize_body(reply, body.put());
        if hr < 0 {
            log_api(b"XtfDeserializeJsonBody(reply, &replyBody)\0", hr, OPERATION_ERROR_CPP, 0x19);
            return hr;
        }
        let hr = self.initialize_body(body.get());
        if hr < 0 {
            log_api(b"Initialize(replyBody)\0", hr, OPERATION_ERROR_CPP, 0x1a);
            return hr;
        }
        S_OK
    }
}

pub unsafe fn send_start_message(session: Obj, command: *const u16, directory: *const u16, flags: u32, period: u32, separated_stderr: bool, exchange: &mut u32) -> HRESULT {
    const F: &str = "SendStartMessage";
    let mut message = Com::null();
    let mut body = Com::null();
    let hr = xbtp::create_message(&IID_XBTP_MESSAGE, message.put());
    ck(F, 330, "XbtpCreateMessage(IID_PPV_ARGS(&message))", hr, "");
    if hr < 0 {
        return hr;
    }
    let hr = xbtp::create_json_object(body.put());
    ck(F, 331, "XbtpCreateJsonObject(&msgBody)", hr, "");
    if hr < 0 {
        return hr;
    }
    let n = w(P_FLAGS);
    let hr = set_number(body.get(), n.as_ptr(), flags as f64);
    ck(F, 333, "XtfSetJsonProperty(msgBody, FlagsProperty, dwFlags)", hr, "");
    if hr < 0 {
        return hr;
    }
    let n = w(P_PERIOD);
    let hr = set_number(body.get(), n.as_ptr(), period as f64);
    ck(F, 334, "XtfSetJsonProperty(msgBody, PeriodProperty, dwPeriod)", hr, "");
    if hr < 0 {
        return hr;
    }
    let n = w(P_COMMAND_LINE);
    let hr = json::set_string(body.get(), n.as_ptr(), command);
    ck(F, 335, "msgBody->SetString(CommandLineProperty, pszCommandLine)", hr, "");
    if hr < 0 {
        return hr;
    }
    let n = w(P_WORKING_DIRECTORY);
    let hr = json::set_string(body.get(), n.as_ptr(), directory);
    ck(F, 336, "msgBody->SetString(WorkingDirProperty, pszWorkingDirectory)", hr, "");
    if hr < 0 {
        return hr;
    }
    if separated_stderr {
        let n = w(P_RUN_ERROR_OUTPUT);
        let hr = set_number(body.get(), n.as_ptr(), 1.0);
        ck(F, 342, "XtfSetJsonProperty(msgBody, RunErrorOutputProperty, 1)", hr, "");
        if hr < 0 {
            return hr;
        }
    }
    let hr = serialize_body(body.get(), message.get());
    ck(F, 345, "XtfSerializeJsonBody(msgBody, message)", hr, "");
    if hr < 0 {
        return hr;
    }
    let hr = com::client_session::send_begin_message(session, RUN_OPERATION_ID, message.get(), exchange);
    ck(F, 349, "session->SendBeginMessage( RunOperationId, message, exchangeId)", hr, "");
    hr
}

pub unsafe fn send_result_message(session: Obj, exchange: u32, result: HRESULT) -> HRESULT {
    const F: &str = "SendResultMessage";
    let mut message = Com::null();
    let mut body = Com::null();
    let hr = xbtp::create_message(&IID_XBTP_MESSAGE, message.put());
    ck(F, 370, "XbtpCreateMessage(IID_PPV_ARGS(&message))", hr, "");
    if hr < 0 {
        return hr;
    }
    let hr = xbtp::create_json_object(body.put());
    ck(F, 371, "XbtpCreateJsonObject(&msgBody)", hr, "");
    if hr < 0 {
        return hr;
    }
    let n = w(P_MSG_ID);
    let hr = json::set_number(body.get(), n.as_ptr(), MSGID_RUN_RESULT as f64);
    ck(F, 372, "msgBody->SetNumber(MsgIdProperty, MsgId_Run_Result)", hr, "");
    if hr < 0 {
        return hr;
    }
    let n = w(P_RESULT);
    let hr = json::set_number(body.get(), n.as_ptr(), result as f64);
    ck(F, 373, "msgBody->SetNumber(ResultProperty, hrResult)", hr, "");
    if hr < 0 {
        return hr;
    }
    let hr = serialize_body(body.get(), message.get());
    ck(F, 375, "XtfSerializeJsonBody(msgBody, message)", hr, "");
    if hr < 0 {
        return hr;
    }
    let hr = client_send_more_message(session, exchange, message.get());
    ck(F, 376, "session->SendMoreMessage(exchangeId, message)", hr, "");
    hr
}

impl RemoteRunClient {
    pub unsafe fn new(address: *const u16, create_result: &mut HRESULT) -> *mut RemoteRunClient {
        let c = Box::into_raw(Box::new(RemoteRunClient { vtbl: &CLIENT_VTBL, vtbl2: &CLIENT2_VTBL, refs: Mutex::new(RefState { refs: 1, destroying: false }), address: null_mut(), copy: crate::variant::current(), session: Mutex::new(Com::null()) }));
        let mut hr = S_OK;
        if !address.is_null() && *address != 0 {
            (*c).address = SysAllocString(address);
            if (*c).address.is_null() {
                hr = E_OUTOFMEMORY;
            }
        }
        ckc("RemoteRunClient::RemoteRunClient", 34, "m_strAddress.Set(pszAddress)", hr, "");
        *create_result = hr;
        c
    }

    pub fn ex(this: *mut RemoteRunClient) -> Obj {
        unsafe { core::ptr::addr_of_mut!((*this).vtbl2) as Obj }
    }

    pub unsafe fn from_ex(ex: Obj) -> *mut RemoteRunClient {
        (ex as *mut u8).sub(core::mem::size_of::<usize>()) as *mut RemoteRunClient
    }

    pub unsafe fn run_ex(&self, command: *const u16, directory: *const u16, flags: u32, period: u32, callback: Obj, exit_code: *mut u32) -> HRESULT {
        const F: &str = "RemoteRunClient::RunEx";
        let mut flags = flags;
        let mut raw: BSTR = null_mut();
        let mut session = Com::null();
        let mut callback_ex = Com::null();
        let mut exchange = 0u32;
        let mut message_type: i16 = XBTP_MESSAGE_TYPE_END;
        let default_wd = w(crate::launch::DEFAULT_WORKING_DIRECTORY);
        let hr = 'run: {
            if command.is_null() || *command == 0 {
                ckc(F, 689, "E_INVALIDARG", E_INVALIDARG, "Command line is required");
                break 'run E_INVALIDARG;
            }
            let directory = if directory.is_null() || *directory == 0 { default_wd.as_ptr() } else { directory };
            if !exit_code.is_null() {
                *exit_code = 0;
            }
            if flags & RUN_FLAG_REDIRECT_MASK != 0 {
                flags |= RUN_FLAG_WAIT;
                if callback.is_null() {
                    ckc(F, 708, "E_INVALIDARG", E_INVALIDARG, "Callback must be supplied if either of the redirection flags are supplied");
                    break 'run E_INVALIDARG;
                }
            }
            let mut port = 0u32;
            let mut hr = split_console_address(self.address, XBTP_PORT_SYSTEM, &mut raw, null_mut(), &mut port);
            if hr < 0 {
                log_api(b"SplitConsoleAddress( connectionString, defaultPort, consoleAddress, nullptr, port)\0", hr, CONSOLE_ADDRESS_CPP, 0xd7);
            } else {
                hr = S_OK;
            }
            ckc(F, 720, "SplitConsoleAddress( m_strAddress, XBTP_PORT_SYSTEM, &rawAddress, &port)", hr, "");
            if hr < 0 {
                break 'run hr;
            }
            let loopback = from_wide(raw).eq_ignore_ascii_case(LOOPBACK_ADDRESS);
            let line = if loopback {
                port |= XBTP_PORT_KIND_LINK;
                732
            } else {
                740
            };
            let hr = open_vm_session(raw, port, session.put());
            ckc(F, line, "XtfOpenVMSession( rawAddress.Ptr(), port, XtfRemoteRunExtensionId, IID_PPV_ARGS(&spSession))", hr, "");
            if hr < 0 {
                break 'run hr;
            }
            let mut separated = false;
            if !callback.is_null() {
                separated = com::query_interface(callback, &IID_XTF_REMOTE_RUN_CALLBACK_EX, callback_ex.put()) >= 0;
            }
            let hr = send_start_message(session.get(), command, directory, flags, period, separated, &mut exchange);
            ckc(F, 753, "SendStartMessage(spSession, pszCommandLine, pszWorkingDirectory, dwFlags, dwPeriod, fSupportsSeparatedStdErr, &exchangeId)", hr, "");
            if hr < 0 {
                break 'run hr;
            }
            let mut sleep = true;
            loop {
                if sleep && period != 0 {
                    Sleep(period);
                }
                sleep = true;
                let mut reply = Com::null();
                let mut reply_body = Com::null();
                let mut status = 0i32;
                let hr = session_receive_message(session.get(), 0, exchange, &mut message_type, &mut status, reply.put());
                if hr == E_PENDING {
                    drop(reply);
                    if flags & RUN_FLAG_REDIRECT_INPUT != 0 {
                        let hr = send_console_input(callback, session.get(), exchange);
                        ckc(F, 864, "SendConsoleInput(pCallback, spSession, exchangeId)", hr, "");
                        if hr < 0 {
                            break 'run hr;
                        }
                    }
                    continue;
                }
                if hr == E_SEM_TIMEOUT {
                    ckc(F, 793, "E_XTFRUN_RECEIVE_TIMED_OUT", E_XTFRUN_RECEIVE_TIMED_OUT, "");
                    break 'run E_XTFRUN_RECEIVE_TIMED_OUT;
                }
                if hr < 0 {
                    ckc(F, 797, "hr", hr, "spSession->receive failed");
                    break 'run hr;
                }
                let hr = deserialize_body(reply.get(), reply_body.put());
                ckc(F, 802, "XtfDeserializeJsonBody(reply, &replyBody)", hr, "");
                if hr < 0 {
                    break 'run hr;
                }
                if message_type == XBTP_MESSAGE_TYPE_MORE {
                    if flags & RUN_FLAG_REDIRECT_OUTPUT == 0 {
                        ckc(F, 854, "E_UNEXPECTED", E_UNEXPECTED, "Unexpected message");
                        break 'run E_UNEXPECTED;
                    }
                    let mut msg_id = 0u32;
                    let n = w(P_MSG_ID);
                    let hr = get_json_u32(reply_body.get(), n.as_ptr(), &mut msg_id);
                    ckc(F, 807, "XtfGetJsonProperty(replyBody, MsgIdProperty, &msgId)", hr, "");
                    if hr < 0 {
                        break 'run hr;
                    }
                    if msg_id == MSGID_RUN_OUTPUT {
                        let hr = recv_console_output(callback, reply.get());
                        ckc(F, 810, "RecvConsoleOutput(pCallback, reply)", hr, "");
                        if hr < 0 {
                            break 'run hr;
                        }
                    } else if msg_id == MSGID_RUN_ERROR {
                        if callback_ex.is_null() {
                            ckc(F, 818, "E_UNEXPECTED", E_UNEXPECTED, "");
                            break 'run E_UNEXPECTED;
                        }
                        let hr = recv_console_error_output(callback_ex.get(), reply.get());
                        ckc(F, 821, "RecvConsoleErrorOutput(spCallbackEx, reply)", hr, "");
                        if hr < 0 {
                            break 'run hr;
                        }
                    }
                    sleep = false;
                    continue;
                }
                if message_type != XBTP_MESSAGE_TYPE_END {
                    ckc(F, 854, "E_UNEXPECTED", E_UNEXPECTED, "Unexpected message");
                    break 'run E_UNEXPECTED;
                }
                let mut results = OperationError::default();
                let hr = results.initialize(reply.get());
                ckc(F, 828, "opResults.Initialize(reply)", hr, "");
                if hr < 0 {
                    break 'run hr;
                }
                let reply_result = results.return_code;
                ckc(F, 829, "opResults.GetReturnCode(&replyResult)", S_OK, "");
                if reply_result < 0 {
                    ckc(F, 848, "replyResult", reply_result, "");
                    break 'run reply_result;
                }
                let mut code = 0u32;
                let n = w(P_EXIT_CODE);
                if get_json_u32(reply_body.get(), n.as_ptr(), &mut code) < 0 {
                    ckc(F, 843, "replyResult", reply_result, "");
                    break 'run reply_result;
                }
                if !exit_code.is_null() {
                    *exit_code = code;
                }
                ckc(F, 839, "dwExitCode", code as i32, "");
                break 'run code as i32;
            }
        };
        if message_type != XBTP_MESSAGE_TYPE_END {
            send_result_message(session.get(), exchange, hr);
        }
        free_bstr(raw);
        hr
    }
}

impl RemoteRunClient {
    pub unsafe fn run_19041(&self, command: *const u16, directory: *const u16, flags: u32, period: u32, callback: Obj) -> HRESULT {
        const F: &str = "RemoteRunClient::Run";
        let mut flags = flags;
        let mut raw: BSTR = null_mut();
        let mut session = self.session.lock().unwrap_or_else(|e| e.into_inner());
        let mut callback_ex = Com::null();
        let mut exchange = 0u32;
        let mut message_type: i16 = XBTP_MESSAGE_TYPE_END;
        let default_wd = w(crate::launch::DEFAULT_WORKING_DIRECTORY);
        let hr = 'run: {
            if command.is_null() || *command == 0 {
                ckc(F, 649, "E_INVALIDARG", E_INVALIDARG, "Command line is required");
                break 'run E_INVALIDARG;
            }
            let directory = if directory.is_null() || *directory == 0 { default_wd.as_ptr() } else { directory };
            if flags & RUN_FLAG_REDIRECT_MASK != 0 {
                flags |= RUN_FLAG_WAIT;
                if callback.is_null() {
                    ckc(F, 663, "E_INVALIDARG", E_INVALIDARG, "Callback must be supplied if either of the redirection flags are supplied");
                    break 'run E_INVALIDARG;
                }
            }
            let mut port = 0u32;
            let mut hr = split_console_address(self.address, XBTP_PORT_SYSTEM, &mut raw, null_mut(), &mut port);
            if hr < 0 {
                log_api(b"SplitConsoleAddress( connectionString, defaultPort, consoleAddress, nullptr, port)\0", hr, CONSOLE_ADDRESS_CPP, 0xd7);
            } else {
                hr = S_OK;
            }
            ckc(F, 675, "SplitConsoleAddress( m_strAddress, XBTP_PORT_SYSTEM, &rawAddress, &port)", hr, "");
            if hr < 0 {
                break 'run hr;
            }
            session.reset();
            let first = open_vm_session_19041(raw, port, session.put());
            if first < 0 {
                port |= XBTP_PORT_KIND_LINK;
                session.reset();
                if open_vm_session_19041(raw, port, session.put()) < 0 {
                    ckc(F, 696, "hr", first, "");
                    break 'run first;
                }
            }
            let mut separated = false;
            if !callback.is_null() {
                separated = com::query_interface(callback, &IID_XTF_REMOTE_RUN_CALLBACK_EX, callback_ex.put()) >= 0;
            }
            let hr = send_start_message(session.get(), command, directory, flags, period, separated, &mut exchange);
            ckc(F, 710, "SendStartMessage(m_session, pszCommandLine, pszWorkingDirectory, dwFlags, dwPeriod, fSupportsSeparatedStdErr, &exchangeId)", hr, "");
            if hr < 0 {
                break 'run hr;
            }
            let mut sleep = true;
            loop {
                if sleep && period != 0 {
                    Sleep(period);
                }
                sleep = true;
                let mut reply = Com::null();
                let mut reply_body = Com::null();
                let mut status = 0i32;
                let hr = session_receive_message(session.get(), 0, exchange, &mut message_type, &mut status, reply.put());
                if hr == E_PENDING {
                    drop(reply);
                    if flags & RUN_FLAG_REDIRECT_INPUT != 0 {
                        let hr = send_console_input(callback, session.get(), exchange);
                        ckc(F, 813, "SendConsoleInput(pCallback, m_session, exchangeId)", hr, "");
                        if hr < 0 {
                            break 'run hr;
                        }
                    }
                    continue;
                }
                if hr < 0 {
                    ckc(F, 750, "hr", hr, "m_session->receive failed");
                    break 'run hr;
                }
                let hr = deserialize_body(reply.get(), reply_body.put());
                ckc(F, 755, "XtfDeserializeJsonBody(reply, &replyBody)", hr, "");
                if hr < 0 {
                    break 'run hr;
                }
                if message_type == XBTP_MESSAGE_TYPE_MORE {
                    if flags & RUN_FLAG_REDIRECT_OUTPUT == 0 {
                        ckc(F, 803, "E_UNEXPECTED", E_UNEXPECTED, "Unexpected message");
                        break 'run E_UNEXPECTED;
                    }
                    let mut msg_id = 0u32;
                    let n = w(P_MSG_ID);
                    let hr = get_json_u32(reply_body.get(), n.as_ptr(), &mut msg_id);
                    ckc(F, 760, "XtfGetJsonProperty(replyBody, MsgIdProperty, &msgId)", hr, "");
                    if hr < 0 {
                        break 'run hr;
                    }
                    if msg_id == MSGID_RUN_OUTPUT {
                        let hr = recv_console_output(callback, reply.get());
                        ckc(F, 763, "RecvConsoleOutput(pCallback, reply)", hr, "");
                        if hr < 0 {
                            break 'run hr;
                        }
                    } else if msg_id == MSGID_RUN_ERROR {
                        if callback_ex.is_null() {
                            ckc(F, 771, "E_UNEXPECTED", E_UNEXPECTED, "");
                            break 'run E_UNEXPECTED;
                        }
                        let hr = recv_console_error_output(callback_ex.get(), reply.get());
                        ckc(F, 774, "RecvConsoleErrorOutput(spCallbackEx, reply)", hr, "");
                        if hr < 0 {
                            break 'run hr;
                        }
                    }
                    sleep = false;
                    continue;
                }
                if message_type != XBTP_MESSAGE_TYPE_END {
                    ckc(F, 803, "E_UNEXPECTED", E_UNEXPECTED, "Unexpected message");
                    break 'run E_UNEXPECTED;
                }
                let mut results = OperationError::default();
                let hr = results.initialize(reply.get());
                ckc(F, 781, "opResults.Initialize(reply)", hr, "");
                if hr < 0 {
                    break 'run hr;
                }
                let reply_result = results.return_code;
                ckc(F, 782, "opResults.GetReturnCode(&replyResult)", S_OK, "");
                if reply_result < 0 {
                    ckc(F, 797, "replyResult", reply_result, "");
                    break 'run reply_result;
                }
                let mut code = 0u32;
                let n = w(P_EXIT_CODE);
                if get_json_u32(reply_body.get(), n.as_ptr(), &mut code) < 0 {
                    ckc(F, 792, "replyResult", reply_result, "");
                    break 'run reply_result;
                }
                ckc(F, 788, "dwExitCode", code as i32, "");
                break 'run code as i32;
            }
        };
        if message_type != XBTP_MESSAGE_TYPE_END {
            send_result_message(session.get(), exchange, hr);
        }
        free_bstr(raw);
        hr
    }
}

impl Drop for RemoteRunClient {
    fn drop(&mut self) {
        unsafe {
            free_bstr(self.address);
            self.address = null_mut();
        }
    }
}

unsafe extern "system" fn client_qi(this: *mut RemoteRunClient, iid: *const GUID, out: *mut Obj) -> HRESULT {
    if iid.is_null() || out.is_null() {
        return E_POINTER;
    }
    let iid = &*iid;
    let p = if *iid == IID_XTF_REMOTE_RUN_CLIENT || *iid == IID_IUNKNOWN || *iid == IID_XTF_REMOTE_RUN_CLIENT_BASE {
        this as Obj
    } else if *iid == IID_XTF_REMOTE_RUN_CLIENT2 && (*this).copy.has_client2() {
        RemoteRunClient::ex(this)
    } else {
        return E_NOINTERFACE;
    };
    *out = p;
    client_add_ref(this);
    S_OK
}

unsafe extern "system" fn client_add_ref(this: *mut RemoteRunClient) -> u32 {
    let mut g = (*this).refs.lock().unwrap_or_else(|e| e.into_inner());
    g.refs += 1;
    g.refs
}

unsafe extern "system" fn client_release(this: *mut RemoteRunClient) -> u32 {
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

unsafe extern "system" fn client_run(this: *mut RemoteRunClient, command: *const u16, directory: *const u16, flags: u32, period: u32, callback: Obj) -> HRESULT {
    match (*this).copy {
        ConsoleCopy::Build19041 => (*this).run_19041(command, directory, flags, period, callback),
        ConsoleCopy::Build22000 => (*this).run_ex(command, directory, flags, period, callback, null_mut()),
    }
}

unsafe extern "system" fn ex_qi(this: Obj, iid: *const GUID, out: *mut Obj) -> HRESULT {
    client_qi(RemoteRunClient::from_ex(this), iid, out)
}

unsafe extern "system" fn ex_add_ref(this: Obj) -> u32 {
    client_add_ref(RemoteRunClient::from_ex(this))
}

unsafe extern "system" fn ex_release(this: Obj) -> u32 {
    client_release(RemoteRunClient::from_ex(this))
}

unsafe extern "system" fn ex_run_ex(this: Obj, command: *const u16, directory: *const u16, flags: u32, period: u32, callback: Obj, exit_code: *mut u32) -> HRESULT {
    (*RemoteRunClient::from_ex(this)).run_ex(command, directory, flags, period, callback, exit_code)
}

#[no_mangle]
pub unsafe extern "system" fn XtfCreateRemoteRunClient(address: *const u16, riid: *const GUID, ppv: *mut Obj) -> HRESULT {
    const F: &str = "XtfCreateRemoteRunClient";
    if ppv.is_null() {
        return E_POINTER;
    }
    *ppv = null_mut();
    let mut create_result = S_OK;
    let client = RemoteRunClient::new(address, &mut create_result);
    ckc(F, 17, "spClient.Attach(new (std::nothrow) RemoteRunClient(pszAddress, &hr))", create_result, "");
    let hr = if create_result < 0 {
        create_result
    } else {
        let hr = client_qi(client, riid, ppv);
        ckc(F, 18, "spClient->QueryInterface(riid, ppvObject)", hr, "");
        hr
    };
    client_release(client);
    hr
}
