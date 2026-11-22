use crate::adapter::receive_text;
use crate::base::{log_error_args, log_info_args};
use crate::conn::{channel_delete, channel_rearm, channel_receive, channel_send, connection_accept, connection_event, NpConnection, NpConnection_InitializeServer_sp, XbtpChannel, XbtpConnection};
use crate::encoding::*;
use crate::handshake::{server_handshake, title_status, HANDSHAKE_OPERATION, TITLE_STATUS_OPERATION};
use crate::link::WSABUF;
use crate::msvc::SharedPtr;
use crate::registry::{lookup, registry_impl, XbtpExtensionRegistry};
use crate::session::*;
use crate::session_impl::{new_session_impl, SessionImpl, SendCompletion, SESSION_IMPL_VTBL};
use crate::sys::*;
use crate::xbtp2_ext::{com_release, message_set_received, new_message, HandlerVtbl};
use crate::xbtp2_json::IID_IUNKNOWN;
use crate::xbtp2_sys::TLS_EVENT_INDEX;
use core::ffi::c_void;
use core::ptr::null_mut;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

const INFINITE: u32 = 0xFFFF_FFFF;
pub const E_PROTOCOL_MISMATCH: HRESULT = 0x8c11_01ffu32 as i32;
pub const E_CONNECT_REFUSED: HRESULT = 0x8c11_01f9u32 as i32;
pub const E_SESSION_REFUSED: HRESULT = 0x8376_0002u32 as i32;
pub const E_EXTENSION_NOT_FOUND: HRESULT = 0x8c11_01fdu32 as i32;
pub const E_AGENT_PROHIBITED: HRESULT = 0x8c11_01feu32 as i32;
pub const E_SERVER_SESSION_DETACHED: HRESULT = 0x8c11_0409u32 as i32;
pub const SERVER_AGENT: &[u8] = b"XbtpServerAgent/220604/GameCore\0";
pub const SERVER_RECEIVE_TIMEOUT_MS: u32 = 60_000;
pub const MAX_AGENT_LENGTH: u32 = 0x80;
pub const STATUS_OK: i32 = 0;
pub const STATUS_BAD_REQUEST: i32 = -1;
pub const STATUS_INVALID_AGENT: i32 = -2;
pub const STATUS_EXTENSION_UNKNOWN: i32 = -3;
pub const STATUS_SESSION_UNKNOWN: i32 = -4;
pub const STATUS_OPERATION_UNDEFINED: i32 = -5;
pub const STATUS_PROTOCOL: i32 = -7;
pub const STATUS_HANDLER_FAILED: i32 = -8;
pub const ROUTER_SIGNATURE_A: u16 = 0xAFA7;
pub const ROUTER_SIGNATURE_B: u16 = 0xAFA8;
pub const IID_XBTP_SERVER_SESSION: GUID = GUID::new(0x90ee5033, 0x9e66, 0x4e1a, [0x8c, 0x46, 0x9c, 0xee, 0xff, 0xf2, 0xb2, 0xb5]);
pub const IID_XBTP_SERVER_SESSION_ALT: GUID = GUID::new(0xfcc4c938, 0x5dab, 0x4b9a, [0x8d, 0x27, 0xfd, 0x14, 0x3b, 0x3d, 0xcd, 0xef]);
pub const IID_XBTP_SESSION_PROPERTIES: GUID = GUID::new(0xf6e48feb, 0x4135, 0x4a7d, [0x9d, 0x3f, 0x77, 0x64, 0xef, 0xf2, 0x0c, 0x80]);
pub const IID_XBTP_DISPATCHER: GUID = GUID::new(0xe6358d7e, 0x3b86, 0x425b, [0xb3, 0xd7, 0x3d, 0x21, 0x82, 0xb9, 0x76, 0x6a]);
pub const IID_XBTP_DISPATCHER_ALT: GUID = GUID::new(0x335b4b4b, 0xc976, 0x4356, [0xa5, 0x6e, 0xc5, 0x01, 0x61, 0xfc, 0x77, 0x40]);
pub const MAX_SESSION_KEY: usize = 0x20;
pub const E_PIPE_ALREADY_ADDED: HRESULT = 0x8007_1392u32 as i32;
pub const E_REDIRECT_NOT_SPECIFIED: HRESULT = 0x8c11_0307u32 as i32;
pub const STATUS_REDIRECTED: i32 = -6;

pub struct RedirectData {
    pub incoming_port: u32,
    pub listener: Mutex<SharedPtr<XbtpConnection>>,
    pub location: Mutex<Option<Vec<u8>>>,
    pub redirect_port: AtomicU32,
}

unsafe impl Send for RedirectData {}
unsafe impl Sync for RedirectData {}

impl RedirectData {
    pub unsafe fn update_location(&self, address: *const u16, port: u32) -> HRESULT {
        let mut loc = self.location.lock().unwrap_or_else(|e| e.into_inner());
        *loc = None;
        self.redirect_port.store(port, Ordering::SeqCst);
        if address.is_null() {
            return S_OK;
        }
        let mut len = 0u32;
        let mut text: *mut u8 = null_mut();
        let hr = crate::conn::tcp_redirect_location(port, address, &mut len, &mut text);
        if hr >= 0 && !text.is_null() {
            *loc = Some(core::slice::from_raw_parts(text, len as usize).to_vec());
        }
        if !text.is_null() {
            free(text as *mut c_void);
        }
        hr
    }

    pub unsafe fn open_connection(&self) -> HRESULT {
        let mut conn = SharedPtr::<crate::conn::TcpConnection>::default();
        let hr = crate::conn::TcpConnection_InitializeServer_sp(self.incoming_port, &mut conn);
        if hr < 0 {
            conn.release();
            return hr;
        }
        let mut l = self.listener.lock().unwrap_or_else(|e| e.into_inner());
        l.release();
        *l = conn.cast();
        hr
    }

    pub fn format_location(&self) -> Result<Vec<u8>, HRESULT> {
        match self.location.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            Some(v) => Ok(v.clone()),
            None => Err(E_REDIRECT_NOT_SPECIFIED),
        }
    }
}

pub unsafe fn add_redirect(d: &DispatcherImpl, incoming_port: u32, address: *const u16, redirect_port: u32) -> HRESULT {
    let mut list = d.redirects.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(r) = list.iter().find(|r| r.incoming_port == incoming_port) {
        let hr = r.update_location(address, redirect_port);
        if hr >= 0 {
            log_info_args(b"Redirect added %d -> %ws : %d\0", &[incoming_port as usize, address as usize, redirect_port as usize]);
        }
        return hr;
    }
    let r = Arc::new(RedirectData { incoming_port, listener: Mutex::new(SharedPtr::default()), location: Mutex::new(None), redirect_port: AtomicU32::new(0) });
    let hr = r.update_location(address, redirect_port);
    if hr < 0 {
        return hr;
    }
    let hr = r.open_connection();
    if hr < 0 {
        log_error_args(b"Failed 0x%X to open redirect connection\0", &[hr as u32 as usize]);
        return hr;
    }
    list.push(r);
    drop(list);
    SetEvent(d.wake);
    log_info_args(b"Redirect added %d -> %ws : %d\0", &[incoming_port as usize, address as usize, redirect_port as usize]);
    hr
}

const ST_DONE: u32 = 0;
const ST_CONNECT: u32 = 1;
const ST_SESSION: u32 = 2;
const ST_ACTIVE: u32 = 3;
const ST_CLOSED: u32 = 4;

pub struct DispatcherImpl {
    pub registry: SharedPtr<XbtpExtensionRegistry>,
    pub tcp: SharedPtr<XbtpConnection>,
    pub link: SharedPtr<XbtpConnection>,
    pub pipes: Mutex<Vec<SharedPtr<XbtpConnection>>>,
    pub pipe_paths: Mutex<Vec<Vec<u16>>>,
    pub redirects: Mutex<Vec<Arc<RedirectData>>>,
    pub shutdown: HANDLE,
    pub stopped: HANDLE,
    pub wake: HANDLE,
    pub session_key: Mutex<Option<Vec<u16>>>,
    pub sessions: Mutex<Vec<std::thread::JoinHandle<()>>>,
    pub running: AtomicBool,
    pub active: AtomicU32,
    pub thread: Mutex<Option<std::thread::JoinHandle<HRESULT>>>,
}

unsafe impl Send for DispatcherImpl {}
unsafe impl Sync for DispatcherImpl {}

impl Drop for DispatcherImpl {
    fn drop(&mut self) {
        unsafe {
            for h in [self.shutdown, self.stopped, self.wake] {
                if !h.is_null() {
                    CloseHandle(h);
                }
            }
            self.registry.release();
            self.tcp.release();
            self.link.release();
            for mut p in core::mem::take(&mut *self.pipes.lock().unwrap()) {
                p.release();
            }
        }
    }
}

#[repr(C)]
pub struct XbtpConnectionDispatcher {
    pub m_impl: *const DispatcherImpl,
}

pub unsafe fn dispatcher_impl(this: *mut XbtpConnectionDispatcher) -> Option<Arc<DispatcherImpl>> {
    let p = (*this).m_impl;
    if p.is_null() {
        return None;
    }
    Arc::increment_strong_count(p);
    Some(Arc::from_raw(p))
}

#[export_name = "??1XbtpConnectionDispatcher@@QEAA@XZ"]
pub unsafe extern "C" fn XbtpConnectionDispatcher_dtor(this: *mut XbtpConnectionDispatcher) {
    XbtpConnectionDispatcher_Shutdown(this);
    let p = core::mem::replace(&mut (*this).m_impl, core::ptr::null());
    if !p.is_null() {
        drop(Arc::from_raw(p));
    }
}

#[export_name = "?Initialize@XbtpConnectionDispatcher@@QEAAJV?$shared_ptr@VXbtpExtensionRegistry@@@std@@V?$shared_ptr@VXbtpConnection@@@3@1@Z"]
pub unsafe extern "C" fn XbtpConnectionDispatcher_Initialize(this: *mut XbtpConnectionDispatcher, registry: *mut SharedPtr<XbtpExtensionRegistry>, tcp: *mut SharedPtr<XbtpConnection>, link: *mut SharedPtr<XbtpConnection>) -> HRESULT {
    let mut reg = SharedPtr::take(registry);
    let mut t = SharedPtr::take(tcp);
    let mut l = SharedPtr::take(link);
    if reg.ptr.is_null() || t.ptr.is_null() {
        reg.release();
        t.release();
        l.release();
        return E_INVALIDARG;
    }
    let shutdown = CreateEventW(null_mut(), 1, 0, core::ptr::null());
    let stopped = CreateEventW(null_mut(), 1, 1, core::ptr::null());
    let wake = CreateEventW(null_mut(), 0, 0, core::ptr::null());
    let imp = DispatcherImpl {
        registry: reg,
        tcp: t,
        link: l,
        pipes: Mutex::new(Vec::new()),
        pipe_paths: Mutex::new(Vec::new()),
        redirects: Mutex::new(Vec::new()),
        shutdown,
        stopped,
        wake,
        session_key: Mutex::new(None),
        sessions: Mutex::new(Vec::new()),
        running: AtomicBool::new(false),
        active: AtomicU32::new(0),
        thread: Mutex::new(None),
    };
    if shutdown.is_null() || stopped.is_null() || wake.is_null() {
        drop(imp);
        return E_OUTOFMEMORY;
    }
    let old = core::mem::replace(&mut (*this).m_impl, Arc::into_raw(Arc::new(imp)));
    if !old.is_null() {
        drop(Arc::from_raw(old));
    }
    S_OK
}

#[export_name = "?AddPipeEndpoint@XbtpConnectionDispatcher@@QEAAJPEBG@Z"]
pub unsafe extern "C" fn XbtpConnectionDispatcher_AddPipeEndpoint(this: *mut XbtpConnectionDispatcher, path: *const u16) -> HRESULT {
    let d = match dispatcher_impl(this) {
        Some(d) => d,
        None => return E_INVALIDARG,
    };
    let mut np = SharedPtr::<NpConnection>::default();
    let hr = NpConnection_InitializeServer_sp(path, &mut np);
    if hr < 0 {
        return hr;
    }
    let key: Vec<u16> = if path.is_null() { Vec::new() } else { core::slice::from_raw_parts(path, wcslen(path)).to_vec() };
    {
        let mut paths = d.pipe_paths.lock().unwrap();
        if paths.iter().any(|p| *p == key) {
            np.release();
            return E_PIPE_ALREADY_ADDED;
        }
        paths.push(key);
    }
    d.pipes.lock().unwrap().push(np.cast());
    log_info_args(b"Pipe added %ws\0", &[path as usize]);
    SetEvent(d.wake);
    S_OK
}

#[export_name = "?Shutdown@XbtpConnectionDispatcher@@QEAAXXZ"]
pub unsafe extern "C" fn XbtpConnectionDispatcher_Shutdown(this: *mut XbtpConnectionDispatcher) {
    let d = match dispatcher_impl(this) {
        Some(d) => d,
        None => return,
    };
    log_info_args(b"Waiting for shutdown to complete\0", &[]);
    SetEvent(d.shutdown);
    WaitForSingleObject(d.stopped, INFINITE);
}

pub unsafe fn set_session_key(d: &DispatcherImpl, key: *const u16) -> HRESULT {
    let mut k = d.session_key.lock().unwrap();
    *k = if key.is_null() || *key == 0 { None } else { Some(core::slice::from_raw_parts(key, wcslen(key)).to_vec()) };
    S_OK
}

unsafe fn connection_list(d: &DispatcherImpl) -> Vec<(SharedPtr<XbtpConnection>, Option<Arc<RedirectData>>)> {
    let mut v = Vec::new();
    if !d.tcp.ptr.is_null() {
        v.push((d.tcp.clone_ref(), None));
    }
    if !d.link.ptr.is_null() {
        v.push((d.link.clone_ref(), None));
    }
    for r in d.redirects.lock().unwrap_or_else(|e| e.into_inner()).iter() {
        let l = r.listener.lock().unwrap_or_else(|e| e.into_inner());
        if !l.ptr.is_null() {
            v.push((l.clone_ref(), Some(r.clone())));
        }
    }
    for p in d.pipes.lock().unwrap().iter() {
        v.push((p.clone_ref(), None));
    }
    v
}

#[export_name = "?Run@XbtpConnectionDispatcher@@QEAAJXZ"]
pub unsafe extern "C" fn XbtpConnectionDispatcher_Run(this: *mut XbtpConnectionDispatcher) -> HRESULT {
    let d = match dispatcher_impl(this) {
        Some(d) => d,
        None => return E_INVALIDARG,
    };
    if d.running.swap(true, Ordering::SeqCst) {
        return E_UNEXPECTED;
    }
    ResetEvent(d.stopped);
    run_claimed(&d)
}

unsafe fn run_claimed(d: &Arc<DispatcherImpl>) -> HRESULT {
    let hr = run_loop(d);
    let joins = core::mem::take(&mut *d.sessions.lock().unwrap());
    for j in joins {
        let _ = j.join();
    }
    d.running.store(false, Ordering::SeqCst);
    SetEvent(d.stopped);
    hr
}

pub unsafe fn start_running(this: *mut XbtpConnectionDispatcher) -> HRESULT {
    let d = match dispatcher_impl(this) {
        Some(d) => d,
        None => return E_INVALIDARG,
    };
    let mut t = d.thread.lock().unwrap();
    if let Some(h) = t.as_ref() {
        if !h.is_finished() {
            return E_UNEXPECTED;
        }
    }
    if let Some(h) = t.take() {
        let _ = h.join();
    }
    if d.running.swap(true, Ordering::SeqCst) {
        return E_UNEXPECTED;
    }
    ResetEvent(d.stopped);
    let dd = d.clone();
    match std::thread::Builder::new().spawn(move || unsafe { run_claimed(&dd) }) {
        Ok(h) => {
            *t = Some(h);
            S_OK
        }
        Err(_) => {
            d.running.store(false, Ordering::SeqCst);
            SetEvent(d.stopped);
            E_OUTOFMEMORY
        }
    }
}

unsafe fn run_loop(d: &Arc<DispatcherImpl>) -> HRESULT {
    loop {
        let conns = connection_list(d);
        let mut handles = vec![d.shutdown, d.wake];
        let mut owners = Vec::new();
        for (c, r) in conns.iter() {
            let mut ev: HANDLE = null_mut();
            if connection_event(c.ptr, &mut ev) >= 0 && !ev.is_null() {
                handles.push(ev);
                owners.push((c.ptr, r.clone()));
            }
        }
        let w = WaitForMultipleObjects(handles.len() as u32, handles.as_ptr(), 0, INFINITE);
        if w == 0 {
            return S_OK;
        }
        if w == 1 {
            continue;
        }
        let i = w as usize;
        if i < 2 || i >= handles.len() {
            let hr = hr_last_error();
            log_error_args(b"Failed 0x%X waiting for connections\0", &[hr as u32 as usize]);
            return hr;
        }
        let mut ch: *mut XbtpChannel = null_mut();
        let (owner, redirect) = owners[i - 2].clone();
        let hr = connection_accept(owner, &mut ch);
        if hr == E_PENDING {
            continue;
        }
        if hr < 0 {
            log_error_args(b"Failed 0x%X to accept connection\0", &[hr as u32 as usize]);
            continue;
        }
        if ch.is_null() {
            continue;
        }
        begin_session(d, ch, redirect);
    }
}

unsafe fn drop_channel(p: *mut c_void) {
    channel_delete(p as *mut XbtpChannel);
}

struct SendPtr(*mut XbtpChannel);
unsafe impl Send for SendPtr {}

unsafe fn begin_session(d: &Arc<DispatcherImpl>, ch: *mut XbtpChannel, redirect: Option<Arc<RedirectData>>) {
    let key = d.session_key.lock().unwrap().clone();
    let dd = d.clone();
    let p = SendPtr(ch);
    let j = std::thread::spawn(move || {
        let p = p;
        let channel = match SharedPtr::new_with(p.0, drop_channel) {
            Some(c) => c,
            None => {
                channel_delete(p.0);
                return;
            }
        };
        let mut a = ServerAdapter { dispatcher: dd, channel, session_id: GUID::default(), extension: GUID::default(), key, redirect };
        a.run();
    });
    let mut s = d.sessions.lock().unwrap();
    s.retain(|h| !h.is_finished());
    s.push(j);
}

pub struct ServerAdapter {
    pub dispatcher: Arc<DispatcherImpl>,
    pub channel: SharedPtr<XbtpChannel>,
    pub session_id: GUID,
    pub extension: GUID,
    pub key: Option<Vec<u16>>,
    pub redirect: Option<Arc<RedirectData>>,
}

unsafe fn send_bytes(ch: *mut XbtpChannel, mut v: Vec<u8>) -> HRESULT {
    channel_send(ch, &[WSABUF { len: v.len() as u32, buf: v.as_mut_ptr() }], None, null_mut())
}

impl ServerAdapter {
    unsafe fn shutting_down(&self) -> bool {
        WaitForSingleObject(self.dispatcher.shutdown, 0) == 0
    }

    pub unsafe fn run(&mut self) {
        let n = self.dispatcher.active.fetch_add(1, Ordering::SeqCst) + 1;
        log_info_args(b"ServerAdapter : Session %d started\0", &[n as usize]);
        let mut state = ST_CONNECT;
        while state != ST_CLOSED && !self.shutting_down() {
            state = match state {
                ST_CONNECT => {
                    if self.negotiate_connect() < 0 {
                        ST_DONE
                    } else {
                        ST_SESSION
                    }
                }
                ST_SESSION => {
                    if self.negotiate_session() < 0 {
                        ST_DONE
                    } else {
                        ST_ACTIVE
                    }
                }
                ST_ACTIVE => self.session_loop(),
                _ => break,
            };
            if state == ST_DONE {
                break;
            }
        }
        self.dispatcher.active.fetch_sub(1, Ordering::SeqCst);
        log_info_args(b"ServerAdapter : Session %d terminated\0", &[n as usize]);
    }

    unsafe fn negotiate_connect(&mut self) -> HRESULT {
        let ch = self.channel.ptr;
        let mut fixed = [0u8; 8];
        let hr = channel_receive(ch, SERVER_RECEIVE_TIMEOUT_MS, 8, fixed.as_mut_ptr());
        if hr < 0 {
            log_error_args(b"ServerAdapter : failed 0x%X to receive connect message\0", &[hr as u32 as usize]);
            return hr;
        }
        let mut req = XBTP_CONNECT_REQUEST::default();
        let hr = DeserializeFixedConnectRequest(8, fixed.as_mut_ptr(), &mut req);
        if hr < 0 {
            log_error_args(b"ServerAdapter : error (0x%X) incomplete connect request\0", &[hr as u32 as usize]);
            return hr;
        }
        if req.signature == ROUTER_SIGNATURE_A || req.signature == ROUTER_SIGNATURE_B {
            return E_PROTOCOL_MISMATCH;
        }
        (*ch).started = 1;
        channel_rearm(ch);
        let mut agent: *mut u8 = null_mut();
        if req.agent.length > MAX_AGENT_LENGTH {
            log_error_args(b"ServerAdapter : agent string too large %d\0", &[req.agent.length as usize]);
        } else {
            let hr = receive_text(ch, SERVER_RECEIVE_TIMEOUT_MS, req.agent.length, &mut agent);
            if hr < 0 {
                log_error_args(b"ServerAdapter : failed 0x%X to receive agent\0", &[hr as u32 as usize]);
                return hr;
            }
        }
        let status = if req.signature != XBTP_PROTOCOL_V1_SIGNATURE as u16 {
            log_error_args(b"ServerAdapter : incorrect connect request signature 0x%X\0", &[req.signature as i16 as i32 as usize]);
            STATUS_BAD_REQUEST
        } else if req.protocol_version != 1 {
            log_error_args(b"ServerAdapter : incorrect connect request version 0x%X\0", &[req.protocol_version as i16 as i32 as usize]);
            STATUS_BAD_REQUEST
        } else if agent.is_null() {
            log_error_args(b"ServerAdapter : agent is invalid\0", &[]);
            STATUS_INVALID_AGENT
        } else {
            log_info_args(b"ServerAdapter : accepted connect request from agent %hs\0", &[agent as usize]);
            STATUS_OK
        };
        if !agent.is_null() {
            free(agent as *mut c_void);
        }
        let hr = match build_connect_response(status, SERVER_AGENT) {
            Ok(v) => send_bytes(ch, v),
            Err(hr) => hr,
        };
        if hr < 0 {
            return hr;
        }
        if status != STATUS_OK {
            return E_CONNECT_REFUSED;
        }
        S_OK
    }

    unsafe fn negotiate_session(&mut self) -> HRESULT {
        let ch = self.channel.ptr;
        let mut fixed = [0u8; FIXED_SESSION_REQUEST_BYTE_SIZE as usize];
        let hr = channel_receive(ch, SERVER_RECEIVE_TIMEOUT_MS, FIXED_SESSION_REQUEST_BYTE_SIZE, fixed.as_mut_ptr());
        if hr < 0 {
            log_error_args(b"ServerAdapter : failed 0x%X to receive session request\0", &[hr as u32 as usize]);
            return hr;
        }
        let mut req = XBTP_CREATE_SESSION_REQUEST_MESSAGE::default();
        let hr = DeserializeCreateSessionRequest(FIXED_SESSION_REQUEST_BYTE_SIZE, fixed.as_mut_ptr(), &mut req);
        if hr < 0 {
            return hr;
        }
        let mut location: Vec<u8> = Vec::new();
        let status = if let Some(r) = self.redirect.as_ref() {
            match r.format_location() {
                Ok(v) => {
                    location = v;
                    STATUS_REDIRECTED
                }
                Err(hr) => return hr,
            }
        } else if registry_impl(self.dispatcher.registry.ptr).is_valid(&req.extension_id) {
            self.session_id = req.session_id;
            self.extension = req.extension_id;
            log_info_args(b"Session negotiated\0", &[]);
            STATUS_OK
        } else {
            STATUS_EXTENSION_UNKNOWN
        };
        let hr = match build_create_session_response(XBTP_PROTOCOL_V1_SIGNATURE as u16, &req.session_id, status, &location) {
            Ok(v) => send_bytes(ch, v),
            Err(hr) => hr,
        };
        if hr < 0 {
            return hr;
        }
        if status != STATUS_OK {
            return E_SESSION_REFUSED;
        }
        S_OK
    }

    unsafe fn send_disconnect(&self, status: i32) -> HRESULT {
        match build_disconnect(XBTP_PROTOCOL_V1_SIGNATURE as u16, status) {
            Ok(v) => send_bytes(self.channel.ptr, v),
            Err(hr) => hr,
        }
    }

    unsafe fn session_loop(&mut self) -> u32 {
        let ch = self.channel.ptr;
        let hs = [self.dispatcher.shutdown, (*ch).recv_event];
        let w = WaitForMultipleObjects(2, hs.as_ptr(), 0, INFINITE);
        if w == 0 {
            self.send_disconnect(STATUS_OK);
            return ST_CLOSED;
        }
        if w != 1 {
            self.send_disconnect(STATUS_PROTOCOL);
            return ST_DONE;
        }
        let mut fixed = [0u8; 8];
        let hr = channel_receive(ch, 0, 8, fixed.as_mut_ptr());
        if hr == E_PENDING {
            return ST_ACTIVE;
        }
        if hr < 0 {
            return ST_DONE;
        }
        let header = match parse_header(&fixed) {
            Some(h) => h,
            None => {
                log_error_args(b"ServerAdapter : failed 0x%X to read header\0", &[E_INVALIDARG as u32 as usize]);
                self.send_disconnect(STATUS_PROTOCOL);
                return ST_DONE;
            }
        };
        if header.signature != XBTP_PROTOCOL_V1_SIGNATURE as u16 {
            log_error_args(b"ServerAdapter : signature %d is invalid - ending session\0", &[header.signature as i16 as i32 as usize]);
            self.send_disconnect(STATUS_PROTOCOL);
            return ST_DONE;
        }
        let t = header.message_type as i16;
        if t == -1 {
            return ST_CLOSED;
        }
        if t == XBTP_MESSAGE_TYPE_ONEWAY || t == XBTP_MESSAGE_TYPE_BEGIN || t == -4 {
            let mut state = ST_ACTIVE;
            if self.handle_message(&header, &mut state) < 0 {
                return ST_CLOSED;
            }
            return state;
        }
        log_error_args(b"ServerAdapter : message type %d unexpected - ending session\0", &[t as i32 as usize]);
        self.send_disconnect(STATUS_PROTOCOL);
        ST_DONE
    }

    unsafe fn handle_message(&mut self, header: &XBTP_MESSAGE_HEADER, state: &mut u32) -> HRESULT {
        let ch = self.channel.ptr;
        if header.message_type as i16 == -4 {
            let n = FIXED_CLOSE_SESSION_MESSAGE_BYTE_SIZE - FIXED_MESSAGE_HEADER_BYTE_SIZE;
            let mut rest = [0u8; 0x10];
            let hr = channel_receive(ch, SERVER_RECEIVE_TIMEOUT_MS, n, rest.as_mut_ptr());
            if hr < 0 {
                return hr;
            }
            let mut h = *header;
            let mut m = XBTP_CLOSE_SESSION_MESSAGE::default();
            let hr = DeserializeCloseSessionMessage(&mut h, n, rest.as_mut_ptr(), &mut m);
            if hr < 0 {
                return hr;
            }
            if m.session_id == self.session_id {
                *state = ST_CLOSED;
                return S_OK;
            }
            self.send_disconnect(STATUS_SESSION_UNKNOWN);
            return E_SESSION_NOT_FOUND;
        }
        let mut msg = XBTP_BEGIN_MESSAGE::default();
        let hr = receive_fixed_extension_message(&self.session_id, header, ch, SERVER_RECEIVE_TIMEOUT_MS, &mut msg);
        if hr == E_SESSION_NOT_FOUND {
            self.send_disconnect(STATUS_SESSION_UNKNOWN);
            *state = ST_CLOSED;
            return hr;
        }
        if hr < 0 {
            log_error_args(b"ServerAdapter : Failed 0x%X to receive (message type %d)\0", &[hr as u32 as usize, header.message_type as i16 as i32 as usize]);
            return hr;
        }
        let mut tls = TlsEventData { extension_id: self.extension, session_id: self.session_id, operation_id: msg.operation_id };
        TlsSetValue(TLS_EVENT_INDEX.load(Ordering::SeqCst), &mut tls as *mut TlsEventData as *mut c_void);
        let hr = self.handle_extension_message(&mut msg, state);
        TlsSetValue(TLS_EVENT_INDEX.load(Ordering::SeqCst), null_mut());
        hr
    }

    unsafe fn handle_extension_message(&mut self, msg: &mut XBTP_BEGIN_MESSAGE, state: &mut u32) -> HRESULT {
        let session = new_session();
        if session.is_null() {
            free_message_bodies(msg);
            return E_OUTOFMEMORY;
        }
        let mut no_adapter = SharedPtr::default();
        let mut ch = self.channel.clone_ref();
        let hr = XbtpSession_Initialize(session, &self.session_id, &mut no_adapter, &mut ch);
        if hr < 0 {
            session_delete(session);
            free_message_bodies(msg);
            return hr;
        }
        let (found, mut def, reason) = lookup(self.dispatcher.registry.ptr, &self.extension);
        if !found {
            let g = guid_args(&self.extension);
            log_error_args(b"Extension {%08X-%04X-%04X-%02X%02X-%02X%02X%02X%02X%02X%02X} not found\0", &g);
            let status = match reason {
                1 | 3 => STATUS_EXTENSION_UNKNOWN,
                2 => STATUS_HANDLER_FAILED,
                _ => STATUS_PROTOCOL,
            };
            self.send_disconnect(status);
            def.release();
            session_delete(session);
            free_message_bodies(msg);
            return E_EXTENSION_NOT_FOUND;
        }
        let network = ((*(*self.channel.ptr).vtbl).is_network)(self.channel.ptr);
        let status = if (network && self.key.is_some()) || msg.operation_id > 0x7fff_fff0 {
            self.route_metadata_message(session, msg, network)
        } else {
            let router = (*def.ptr).router.ptr;
            let handler = if router.is_null() { null_mut() } else { (*router).handler(msg.operation_id) };
            handle_routed_message(session, msg, handler)
        };
        def.release();
        let hr = if status != 0 && status != STATUS_OPERATION_UNDEFINED && status != STATUS_HANDLER_FAILED {
            let g = guid_args(&self.extension);
            let mut args = g.to_vec();
            args.push(msg.operation_id as usize);
            args.push(status as u32 as usize);
            log_error_args(b"Extension {%08X-%04X-%04X-%02X%02X-%02X%02X%02X%02X%02X%02X} op %d failed %d\0", &args);
            self.send_disconnect(status);
            *state = ST_CLOSED;
            status_error(status)
        } else {
            S_OK
        };
        session_delete(session);
        hr
    }

    unsafe fn route_metadata_message(&mut self, session: *mut XbtpSession, msg: &mut XBTP_BEGIN_MESSAGE, network: bool) -> i32 {
        if network && self.key.is_some() {
            if msg.operation_id != HANDSHAKE_OPERATION {
                free_message_bodies(msg);
                return STATUS_INVALID_AGENT;
            }
            let key = self.key.clone();
            let status = handle_with(session, msg, |received, reply| server_handshake(key.as_deref(), received, reply));
            if status < 0 {
                return STATUS_INVALID_AGENT;
            }
            self.key = None;
            return S_OK;
        }
        let status = match msg.operation_id {
            HANDSHAKE_OPERATION => {
                log_info_args(b"ServerAdapter : SessionKey supplied but none needed on console\0", &[]);
                handle_with(session, msg, |received, reply| server_handshake(None, received, reply))
            }
            TITLE_STATUS_OPERATION => handle_with(session, msg, |_, reply| title_status(reply)),
            _ => {
                free_message_bodies(msg);
                0
            }
        };
        if status < 0 {
            STATUS_INVALID_AGENT
        } else {
            S_OK
        }
    }
}

#[repr(C)]
pub struct TlsEventData {
    pub extension_id: GUID,
    pub session_id: GUID,
    pub operation_id: u32,
}

unsafe fn free_message_bodies(msg: &mut XBTP_BEGIN_MESSAGE) {
    if !msg.body.json_body.is_null() {
        free(msg.body.json_body as *mut c_void);
        msg.body.json_body = null_mut();
    }
    if !msg.body.binary_body.is_null() {
        free(msg.body.binary_body as *mut c_void);
        msg.body.binary_body = null_mut();
    }
}

unsafe fn drop_body(p: *mut c_void) {
    let b = p as *mut XBTP_MESSAGE_BODY;
    if !(*b).json_body.is_null() {
        free((*b).json_body as *mut c_void);
    }
    if !(*b).binary_body.is_null() {
        free((*b).binary_body as *mut c_void);
    }
    free(p);
}

unsafe fn take_body(msg: &mut XBTP_BEGIN_MESSAGE) -> Option<SharedPtr<XBTP_MESSAGE_BODY>> {
    let b = malloc(core::mem::size_of::<XBTP_MESSAGE_BODY>()) as *mut XBTP_MESSAGE_BODY;
    if b.is_null() {
        free_message_bodies(msg);
        return None;
    }
    b.write(msg.body);
    msg.body.json_body = null_mut();
    msg.body.binary_body = null_mut();
    let sp = SharedPtr::new_with(b, drop_body);
    if sp.is_none() {
        drop_body(b as *mut c_void);
    }
    sp
}

unsafe fn no_drop(_p: *mut c_void) {}

pub unsafe fn handle_routed_message(session: *mut XbtpSession, msg: &mut XBTP_BEGIN_MESSAGE, handler: *mut c_void) -> i32 {
    if handler.is_null() {
        let g = guid_args(&msg.session_id);
        let mut args = vec![msg.operation_id as usize];
        args.extend_from_slice(&g);
        log_error_args(b"No handler for op %d in session {%08X-%04X-%04X-%02X%02X-%02X%02X%02X%02X%02X%02X}\0", &args);
        free_message_bodies(msg);
        return send_reply(session, msg.exchange_id, STATUS_OPERATION_UNDEFINED, core::ptr::null_mut());
    }
    handle_with(session, msg, WithServer(|s: *mut c_void, received: *mut c_void, reply: *mut c_void| {
        let vt = &**(handler as *const *const HandlerVtbl);
        (vt.invoke)(handler, s as usize, received as usize, reply as usize)
    }))
}

unsafe fn send_reply(session: *mut XbtpSession, exchange: i32, status: i32, reply: *mut c_void) -> i32 {
    let mut body = XBTP_MESSAGE_BODY::default();
    if !reply.is_null() {
        let hr = crate::session_impl::format_message_body(reply, &mut body);
        if hr < 0 {
            return STATUS_PROTOCOL;
        }
    }
    let hr = XbtpSession_SendEndMessage(session, exchange, status, &body);
    if hr < 0 {
        return STATUS_PROTOCOL;
    }
    status
}

pub unsafe fn handle_with<F>(session: *mut XbtpSession, msg: &mut XBTP_BEGIN_MESSAGE, f: F) -> i32
where
    F: HandlerCall,
{
    let body = match take_body(msg) {
        Some(b) => b,
        None => return STATUS_PROTOCOL,
    };
    let received = new_message() as *mut c_void;
    if message_set_received(received as *mut _, body) < 0 {
        com_release(received);
        return STATUS_PROTOCOL;
    }
    let inner = new_message();
    let reply = crate::xbtp2_ext::new_end_message(inner) as *mut c_void;
    let server = new_server_session(session, msg.exchange_id);
    {
        let g = guid_args(&msg.session_id);
        let mut args = g.to_vec();
        args.push(msg.operation_id as usize);
        log_info_args(b"Starting {%08X-%04X-%04X-%02X%02X-%02X%02X%02X%02X%02X%02X} %X\0", &args);
    }
    let hr = f.call(server as *mut c_void, received, reply);
    {
        let g = guid_args(&msg.session_id);
        let mut args = g.to_vec();
        args.push(msg.operation_id as usize);
        args.push(hr as u32 as usize);
        log_info_args(b"Finished {%08X-%04X-%04X-%02X%02X-%02X%02X%02X%02X%02X%02X} %X 0x%X\0", &args);
    }
    server_session_detach(server);
    ss_release(server as *mut c_void);
    let status = if hr < 0 {
        log_error_args(b"Op %d failed 0x%X\0", &[msg.operation_id as usize, hr as u32 as usize]);
        STATUS_HANDLER_FAILED
    } else {
        STATUS_OK
    };
    com_release(received);
    com_release(reply);
    let r = send_reply(session, msg.exchange_id, status, inner as *mut c_void);
    com_release(inner as *mut c_void);
    r
}

pub trait HandlerCall {
    unsafe fn call(self, server: *mut c_void, received: *mut c_void, reply: *mut c_void) -> HRESULT;
}

impl<F: FnOnce(*mut c_void, *mut c_void) -> HRESULT> HandlerCall for F {
    unsafe fn call(self, _server: *mut c_void, received: *mut c_void, reply: *mut c_void) -> HRESULT {
        self(received, reply)
    }
}

pub struct WithServer<F>(pub F);

impl<F: FnOnce(*mut c_void, *mut c_void, *mut c_void) -> HRESULT> HandlerCall for WithServer<F> {
    unsafe fn call(self, server: *mut c_void, received: *mut c_void, reply: *mut c_void) -> HRESULT {
        (self.0)(server, received, reply)
    }
}

#[repr(C)]
pub struct ServerSessionVtbl {
    pub query_interface: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub send_more_message: unsafe extern "system" fn(*mut ServerSession, *mut c_void) -> HRESULT,
    pub send_more_message_async: unsafe extern "system" fn(*mut ServerSession, *mut c_void, Option<SendCompletion>, *mut c_void) -> HRESULT,
    pub receive_more_message: unsafe extern "system" fn(*mut ServerSession, u32, *mut *mut c_void) -> HRESULT,
    pub get_message_received_event: unsafe extern "system" fn(*mut ServerSession, *mut HANDLE) -> HRESULT,
}

#[repr(C)]
pub struct SessionPropertiesVtbl {
    pub query_interface: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub get_property: unsafe extern "system" fn(*mut c_void, *const u16, *mut BSTR) -> HRESULT,
}

#[repr(C)]
pub struct ServerSession {
    pub vtbl: *const ServerSessionVtbl,
    pub props: *const SessionPropertiesVtbl,
    pub refs: AtomicU32,
    pub exchange: i32,
    pub inner: Mutex<*mut SessionImpl>,
}

pub static SERVER_SESSION_VTBL: ServerSessionVtbl = ServerSessionVtbl {
    query_interface: ss_qi,
    add_ref: ss_add_ref,
    release: ss_release,
    send_more_message: ss_send_more_message,
    send_more_message_async: ss_send_more_message_async,
    receive_more_message: ss_receive_more_message,
    get_message_received_event: ss_get_message_received_event,
};

pub static SESSION_PROPERTIES_VTBL: SessionPropertiesVtbl = SessionPropertiesVtbl {
    query_interface: sp_qi,
    add_ref: sp_add_ref,
    release: sp_release,
    get_property: sp_get_property,
};

unsafe fn new_server_session(session: *mut XbtpSession, exchange: i32) -> *mut ServerSession {
    let sp = SharedPtr::new_with(session, no_drop).unwrap_or_default();
    let inner = new_session_impl(sp);
    Box::into_raw(Box::new(ServerSession { vtbl: &SERVER_SESSION_VTBL, props: &SESSION_PROPERTIES_VTBL, refs: AtomicU32::new(1), exchange, inner: Mutex::new(inner) }))
}

unsafe fn server_session_detach(s: *mut ServerSession) {
    let p = core::mem::replace(&mut *(*s).inner.lock().unwrap_or_else(|e| e.into_inner()), null_mut());
    if !p.is_null() {
        ((*(*p).vtbl).release)(p);
    }
}

unsafe extern "system" fn ss_qi(this: *mut c_void, iid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    let s = this as *mut ServerSession;
    if *iid == IID_XBTP_SERVER_SESSION || *iid == IID_IUNKNOWN || *iid == IID_XBTP_SERVER_SESSION_ALT {
        *out = this;
    } else if *iid == IID_XBTP_SESSION_PROPERTIES {
        *out = &mut (*s).props as *mut _ as *mut c_void;
    } else {
        *out = null_mut();
        return E_NOINTERFACE;
    }
    ss_add_ref(this);
    S_OK
}

unsafe extern "system" fn ss_add_ref(this: *mut c_void) -> u32 {
    (*(this as *mut ServerSession)).refs.fetch_add(1, Ordering::AcqRel) + 1
}

unsafe extern "system" fn ss_release(this: *mut c_void) -> u32 {
    let s = this as *mut ServerSession;
    let n = (*s).refs.fetch_sub(1, Ordering::AcqRel) - 1;
    if n == 0 {
        server_session_detach(s);
        drop(Box::from_raw(s));
    }
    n
}

unsafe fn props_base(this: *mut c_void) -> *mut c_void {
    (this as *mut u8).sub(core::mem::offset_of!(ServerSession, props)) as *mut c_void
}

unsafe extern "system" fn sp_qi(this: *mut c_void, iid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    ss_qi(props_base(this), iid, out)
}

unsafe extern "system" fn sp_add_ref(this: *mut c_void) -> u32 {
    ss_add_ref(props_base(this))
}

unsafe extern "system" fn sp_release(this: *mut c_void) -> u32 {
    ss_release(props_base(this))
}

unsafe fn with_inner(s: *mut ServerSession, f: impl FnOnce(*mut SessionImpl) -> HRESULT) -> HRESULT {
    let g = (*s).inner.lock().unwrap_or_else(|e| e.into_inner());
    if g.is_null() {
        return E_SERVER_SESSION_DETACHED;
    }
    f(*g)
}

unsafe extern "system" fn ss_send_more_message(s: *mut ServerSession, msg: *mut c_void) -> HRESULT {
    let ex = (*s).exchange;
    with_inner(s, |i| (SESSION_IMPL_VTBL.send_more_message)(i, ex, msg))
}

unsafe extern "system" fn ss_send_more_message_async(s: *mut ServerSession, msg: *mut c_void, cb: Option<SendCompletion>, ctx: *mut c_void) -> HRESULT {
    let ex = (*s).exchange;
    with_inner(s, |i| (SESSION_IMPL_VTBL.send_more_message_async)(i, ex, msg, cb, ctx))
}

unsafe extern "system" fn ss_receive_more_message(s: *mut ServerSession, timeout: u32, out: *mut *mut c_void) -> HRESULT {
    let ex = (*s).exchange;
    with_inner(s, |i| (SESSION_IMPL_VTBL.receive_more_message)(i, timeout, ex, out))
}

unsafe extern "system" fn ss_get_message_received_event(s: *mut ServerSession, out: *mut HANDLE) -> HRESULT {
    with_inner(s, |i| (SESSION_IMPL_VTBL.get_message_received_event)(i, out))
}

unsafe extern "system" fn sp_get_property(this: *mut c_void, name: *const u16, value: *mut BSTR) -> HRESULT {
    let s = props_base(this) as *mut ServerSession;
    with_inner(s, |i| (SESSION_IMPL_VTBL.get_property)(i, name, value))
}
