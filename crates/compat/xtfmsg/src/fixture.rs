use crate::com::{self, Com, Obj, IID_IUNKNOWN, IID_XBTP_MESSAGE};
use crate::sys::*;
use crate::xbtp;
use core::ffi::c_void;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, Ordering};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

static LOAD: OnceLock<Result<(), String>> = OnceLock::new();

#[repr(C)]
pub struct Vt<const N: usize>([*const (); N]);
unsafe impl<const N: usize> Sync for Vt<N> {}

fn modified(p: &PathBuf) -> Option<std::time::SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

pub fn fresh_xbtp_build() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    let mut builds = Vec::new();
    let mut searched = Vec::new();
    for dir in exe.ancestors().skip(1).take(5) {
        let b = dir.join("build").join("xbtp");
        searched.push(b.clone());
        if let Ok(rd) = std::fs::read_dir(&b) {
            builds.extend(rd.flatten().map(|e| e.path().join("out").join("xbtp.dll")).filter(|p| p.is_file()));
        }
    }
    builds.into_iter().max_by_key(modified).ok_or_else(|| format!("no build\\xbtp\\*\\out\\xbtp.dll under {searched:?}; add xbtp as a dev-dependency of the crate under test"))
}

fn load_fresh_xbtp() -> Result<(), String> {
    let name = wide("xbtp.dll");
    if !unsafe { GetModuleHandleW(name.as_ptr()) }.is_null() {
        return Ok(());
    }
    let src = fresh_xbtp_build()?;
    let dir = std::env::temp_dir().join(format!("xtfmsg-xbtp-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let copy = dir.join("xbtp.dll");
    std::fs::copy(&src, &copy).map_err(|e| format!("copy {} to {}: {e}", src.display(), copy.display()))?;
    if !xbtp::preload(&copy) {
        return Err(format!("LoadLibraryW({}) copied from {} failed: {}", copy.display(), src.display(), unsafe { GetLastError() }));
    }
    if xbtp::module() == 0 {
        return Err(format!("xbtp::module() is 0 after loading {}", copy.display()));
    }
    Ok(())
}

pub fn load_xbtp() {
    if let Err(e) = LOAD.get_or_init(load_fresh_xbtp) {
        panic!("{e}");
    }
}

pub unsafe fn new_message() -> Com {
    let mut m = Com::null();
    assert_eq!(xbtp::create_message(&IID_XBTP_MESSAGE, m.put()), S_OK);
    m
}

pub unsafe fn message_with_json(text: &str) -> Com {
    let m = new_message();
    let w = wide(text);
    assert_eq!(com::message::set_json_body_w(m.get(), w.as_ptr()), S_OK);
    m
}

pub unsafe fn json_text(msg: Obj) -> Option<String> {
    let mut len = 0u32;
    let mut p: *const u8 = core::ptr::null();
    assert!(com::message::get_json_body(msg, &mut len, &mut p) >= 0);
    if p.is_null() {
        return None;
    }
    let s = core::slice::from_raw_parts(p, len as usize);
    let s = s.strip_suffix(&[0]).unwrap_or(s);
    Some(String::from_utf8_lossy(s).into_owned())
}

#[repr(C)]
pub struct EndMessage {
    vtbl: *const Vt<8>,
    refs: AtomicU32,
    pub inner: Obj,
}

unsafe extern "system" fn em_qi(o: Obj, iid: *const GUID, out: *mut Obj) -> HRESULT {
    if *iid == IID_IUNKNOWN {
        *out = o;
        em_add_ref(o);
        return S_OK;
    }
    *out = core::ptr::null_mut();
    E_NOINTERFACE
}

unsafe extern "system" fn em_add_ref(o: Obj) -> u32 {
    (*(o as *mut EndMessage)).refs.fetch_add(1, Ordering::SeqCst) + 1
}

unsafe extern "system" fn em_release(o: Obj) -> u32 {
    let e = o as *mut EndMessage;
    let n = (*e).refs.fetch_sub(1, Ordering::SeqCst) - 1;
    if n == 0 {
        com::release((*e).inner);
        drop(Box::from_raw(e));
    }
    n
}

unsafe extern "system" fn em_get_json(o: Obj, len: *mut u32, b: *mut *const u8) -> HRESULT {
    com::message::get_json_body((*(o as *mut EndMessage)).inner, len, b)
}

unsafe extern "system" fn em_get_bin(o: Obj, len: *mut u32, b: *mut *const u8) -> HRESULT {
    com::message::get_binary_body((*(o as *mut EndMessage)).inner, len, b)
}

unsafe extern "system" fn em_set_json(o: Obj, len: u32, b: *const u8) -> HRESULT {
    com::message::set_json_body_copy((*(o as *mut EndMessage)).inner, len, b)
}

unsafe extern "system" fn em_set_json_w(o: Obj, b: *const u16) -> HRESULT {
    com::message::set_json_body_w((*(o as *mut EndMessage)).inner, b)
}

unsafe extern "system" fn em_set_bin(o: Obj, len: u32, b: *const u8) -> HRESULT {
    com::message::set_binary_body_copy((*(o as *mut EndMessage)).inner, len, b)
}

static EM_VTBL: Vt<8> = Vt([em_qi as *const (), em_add_ref as *const (), em_release as *const (), em_get_json as *const (), em_get_bin as *const (), em_set_json as *const (), em_set_json_w as *const (), em_set_bin as *const ()]);

pub unsafe fn end_message() -> Com {
    let mut inner = new_message();
    let e = Box::new(EndMessage { vtbl: &EM_VTBL, refs: AtomicU32::new(1), inner: inner.detach() });
    Com(Box::into_raw(e) as Obj)
}

pub unsafe fn end_json(o: Obj) -> Option<String> {
    json_text((*(o as *mut EndMessage)).inner)
}

#[repr(C)]
pub struct Session {
    vtbl: *const Vt<7>,
    refs: AtomicU32,
    pub inbound: Mutex<VecDeque<Result<usize, HRESULT>>>,
    pub sent: Mutex<Vec<String>>,
    pub receive_flags: Mutex<Vec<u32>>,
}

unsafe extern "system" fn ss_qi(o: Obj, iid: *const GUID, out: *mut Obj) -> HRESULT {
    if *iid == IID_IUNKNOWN {
        *out = o;
        ss_add_ref(o);
        return S_OK;
    }
    *out = core::ptr::null_mut();
    E_NOINTERFACE
}

unsafe extern "system" fn ss_add_ref(o: Obj) -> u32 {
    (*(o as *mut Session)).refs.fetch_add(1, Ordering::SeqCst) + 1
}

unsafe extern "system" fn ss_release(o: Obj) -> u32 {
    let s = o as *mut Session;
    let n = (*s).refs.fetch_sub(1, Ordering::SeqCst) - 1;
    if n == 0 {
        for m in (*s).inbound.lock().unwrap().drain(..).flatten() {
            com::release(m as Obj);
        }
        drop(Box::from_raw(s));
    }
    n
}

unsafe extern "system" fn ss_send_more(o: Obj, msg: Obj) -> HRESULT {
    let t = json_text(msg).unwrap_or_default();
    (*(o as *mut Session)).sent.lock().unwrap().push(t);
    S_OK
}

unsafe extern "system" fn ss_unused(_o: Obj) -> HRESULT {
    E_NOTIMPL
}

unsafe extern "system" fn ss_receive_more(o: Obj, flags: u32, out: *mut Obj) -> HRESULT {
    let s = o as *mut Session;
    (*s).receive_flags.lock().unwrap().push(flags);
    match (*s).inbound.lock().unwrap().pop_front() {
        Some(Ok(m)) => {
            *out = m as Obj;
            S_OK
        }
        Some(Err(e)) => e,
        None => E_PENDING,
    }
}

static SS_VTBL: Vt<7> = Vt([ss_qi as *const (), ss_add_ref as *const (), ss_release as *const (), ss_send_more as *const (), ss_unused as *const (), ss_receive_more as *const (), ss_unused as *const ()]);

pub unsafe fn session() -> *mut Session {
    Box::into_raw(Box::new(Session { vtbl: &SS_VTBL, refs: AtomicU32::new(1), inbound: Mutex::new(VecDeque::new()), sent: Mutex::new(Vec::new()), receive_flags: Mutex::new(Vec::new()) }))
}

pub unsafe fn push_inbound(s: *mut Session, json: &str) {
    let mut m = message_with_json(json);
    (*s).inbound.lock().unwrap().push_back(Ok(m.detach() as usize));
}

pub unsafe fn push_inbound_error(s: *mut Session, hr: HRESULT) {
    (*s).inbound.lock().unwrap().push_back(Err(hr));
}

#[repr(C)]
pub struct ClientSession {
    vtbl: *const Vt<8>,
    refs: AtomicU32,
    pub begin_hr: HRESULT,
    pub end_hr: HRESULT,
    pub status: u32,
    pub response: Option<String>,
    pub calls: Mutex<Vec<(u32, u32, u32)>>,
}

unsafe extern "system" fn cs_add_ref(o: Obj) -> u32 {
    (*(o as *mut ClientSession)).refs.fetch_add(1, Ordering::SeqCst) + 1
}

unsafe extern "system" fn cs_release(o: Obj) -> u32 {
    let s = o as *mut ClientSession;
    let n = (*s).refs.fetch_sub(1, Ordering::SeqCst) - 1;
    if n == 0 {
        drop(Box::from_raw(s));
    }
    n
}

unsafe extern "system" fn cs_begin(o: Obj, op: u32, _msg: Obj, exchange: *mut u32) -> HRESULT {
    let s = o as *mut ClientSession;
    (*s).calls.lock().unwrap().push((op, 0, 0));
    if (*s).begin_hr < 0 {
        return (*s).begin_hr;
    }
    *exchange = 0x41;
    S_OK
}

unsafe extern "system" fn cs_end(o: Obj, timeout: u32, exchange: u32, status: *mut u32, out: *mut Obj) -> HRESULT {
    let s = o as *mut ClientSession;
    (*s).calls.lock().unwrap().push((u32::MAX, timeout, exchange));
    if (*s).end_hr < 0 {
        return (*s).end_hr;
    }
    *status = (*s).status;
    *out = match &(*s).response {
        Some(j) => message_with_json(j).detach(),
        None => new_message().detach(),
    };
    S_OK
}

static CS_VTBL: Vt<8> = Vt([ss_qi as *const (), cs_add_ref as *const (), cs_release as *const (), cs_begin as *const (), ss_unused as *const (), ss_unused as *const (), ss_unused as *const (), cs_end as *const ()]);

pub unsafe fn client_session(begin_hr: HRESULT, end_hr: HRESULT, status: u32, response: Option<&str>) -> *mut ClientSession {
    Box::into_raw(Box::new(ClientSession { vtbl: &CS_VTBL, refs: AtomicU32::new(1), begin_hr, end_hr, status, response: response.map(str::to_string), calls: Mutex::new(Vec::new()) }))
}

pub fn as_obj<T>(p: *mut T) -> Obj {
    p as *mut c_void
}
