use crate::com::*;
use crate::find::{get_file_info_properties, MsgFindFilesInfo};
use crate::msg::*;
use core::ffi::c_void;
use core::ptr::null_mut;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

pub const IID_IUNKNOWN: GUID = GUID::new(0, 0, 0, [0xc0, 0, 0, 0, 0, 0, 0, 0x46]);
pub const IID_XTF_FILEIO_CLIENT: GUID = GUID::new(0x87e7b741, 0xcb00, 0x4600, [0x9d, 0x14, 0x17, 0x3e, 0x3a, 0xa0, 0xf8, 0xfe]);
pub const IID_XTF_FILEIO_CLIENT_BASE: GUID = GUID::new(0x6b6b8463, 0xa3fb, 0x4bbf, [0x95, 0xbe, 0x0f, 0x6f, 0x1a, 0x40, 0x43, 0x3c]);
pub const IID_XTF_FILEIO_CLIENT2: GUID = GUID::new(0xb26df5eb, 0xee9c, 0x401e, [0xbc, 0x7a, 0xec, 0xb1, 0xdd, 0x23, 0xf7, 0x6e]);
pub const IID_XTF_FILEIO_CLIENT3: GUID = GUID::new(0xd649465c, 0x86a7, 0x49e1, [0x88, 0x3c, 0xef, 0xd1, 0x93, 0x8c, 0xbe, 0xde]);
pub const IID_XBTP_CLIENT_SESSION: GUID = GUID::new(0xcf84f553, 0xcd9b, 0x4608, [0x96, 0x40, 0x28, 0x71, 0x4e, 0x5e, 0xd4, 0x40]);

pub const E_NOINTERFACE: HRESULT = 0x8000_4002u32 as i32;
pub const E_POINTER: HRESULT = 0x8000_4003u32 as i32;
pub const E_COULD_NOT_CONNECT: HRESULT = 0x8c11_0303u32 as i32;
pub const E_NO_IPADDRESS_IN_CONNSTR: HRESULT = 0x8c11_0407u32 as i32;
pub const E_INVALID_VM_TARGET: HRESULT = 0x8c11_0408u32 as i32;
pub const E_APP_OPERATIONS_REMOTE: HRESULT = 0x8c11_01fbu32 as i32;
pub const TRANSPORT_STATUS_BASE: u32 = 0x8c11_0100;
pub const UNEXPECTED_MESSAGE_BASE: u32 = 0x8c11_0200;

pub const XBTP_PORT_SYSTEM: u32 = 0;
pub const XBTP_PORT_TITLE: u32 = 1;
pub const XBTP_PORT_HOST: u32 = 3;
pub const XBTP_PORT_KIND_LINK: u32 = 0x0100_0000;
pub const XBTP_PORT_KIND_MASK: u32 = 0xff00_0000;
pub const XBTP_MESSAGE_TYPE_MORE: i16 = 3;
pub const XBTP_MESSAGE_TYPE_END: i16 = 4;

pub const DEFAULT_OPEN_SESSION_TIMEOUT_MS: u64 = 30000;
pub const DIRECT_PROTOCOL_QUERY_TIMEOUT_MS: u32 = 30000;
pub const MAX_ADDRESS_CHARS: usize = 0x200;
pub const SESSION_KEY_CHARS: usize = 0x20;
pub const OPEN_OPTIONS_SIZE: u32 = 0x58;
pub const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;

pub const OP_GET_FILE_SIZE: u32 = 1;
pub const OP_GET_FILE_ATTRIBUTES: u32 = 2;
pub const OP_SET_FILE_ATTRIBUTES: u32 = 3;
pub const OP_MODIFY_FILE_ATTRIBUTES: u32 = 4;
pub const OP_DELETE_FILE: u32 = 5;
pub const OP_CREATE_DIRECTORY: u32 = 6;
pub const OP_REMOVE_DIRECTORY: u32 = 7;
pub const OP_FIND_FILES: u32 = 9;
pub const OP_GET_FILE_INFO: u32 = 0xc;
pub const OP_APPLICATION_OPERATIONS_START: u32 = 0xd;
pub const OP_APPLICATION_OPERATIONS_END: u32 = 0xe;
pub const OP_STORAGE_DEVICE_FREE_SPACE: u32 = 0x10;
pub const OP_TRANSFER_FILES_SUPPORTED: u32 = 0x19;

#[link(name = "kernel32")]
extern "system" {
    fn GetTickCount64() -> u64;
    fn Sleep(ms: u32);
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct XtfFileInfo {
    pub file_name: *const u16,
    pub attributes: u32,
    pub creation: u64,
    pub access: u64,
    pub write: u64,
    pub size: u64,
    pub reserved: [u64; 3],
}

impl Default for XtfFileInfo {
    fn default() -> Self {
        XtfFileInfo { file_name: null_mut(), attributes: 0, creation: 0, access: 0, write: 0, size: 0, reserved: [0; 3] }
    }
}

#[repr(C)]
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
pub struct ClientSessionVtbl {
    pub base: UnknownVtbl,
    pub send_begin_message: unsafe extern "system" fn(*mut c_void, u32, *mut c_void, *mut i32) -> HRESULT,
    pub send_more_message: unsafe extern "system" fn(*mut c_void, i32, *mut c_void) -> HRESULT,
    pub send_more_message_async: usize,
    pub receive_more_message: unsafe extern "system" fn(*mut c_void, u32, i32, *mut *mut c_void) -> HRESULT,
    pub receive_end_message: unsafe extern "system" fn(*mut c_void, u32, i32, *mut i32, *mut *mut c_void) -> HRESULT,
    pub receive_message: unsafe extern "system" fn(*mut c_void, u32, i32, *mut i16, *mut i32, *mut *mut c_void) -> HRESULT,
}

#[repr(C)]
pub struct FindCallbackVtbl {
    pub base: UnknownVtbl,
    pub on_file: unsafe extern "system" fn(*mut c_void, *const u16, *const u16, *const XtfFileInfo) -> HRESULT,
}

#[cfg(test)]
thread_local! {
    pub static TEST_PORT: core::cell::Cell<u32> = const { core::cell::Cell::new(0) };
}

fn test_port() -> u32 {
    #[cfg(test)]
    {
        TEST_PORT.with(|p| p.get())
    }
    #[cfg(not(test))]
    {
        0
    }
}

unsafe fn wcslen(p: *const u16) -> usize {
    let mut n = 0;
    while *p.add(n) != 0 {
        n += 1;
    }
    n
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

pub unsafe fn split_console_address(conn: *const u16, default_port: u32, ip: &mut Bstr, key: Option<&mut Bstr>, port: &mut u32) -> HRESULT {
    if conn.is_null() {
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
    if let Some(key) = key {
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
                SysFreeString(k);
                return hr;
            }
            *key.out() = k;
            *ip.out() = a;
            return S_OK;
        }
    }
    let mut a: BSTR = null_mut();
    let hr = bstr_len(conn, len, &mut a);
    if hr < 0 {
        return hr;
    }
    *ip.out() = a;
    S_OK
}

pub unsafe fn get_final_address(address: *const u16, out: &mut Bstr) -> HRESULT {
    if !address.is_null() && address_length(address).is_some() {
        let b = SysAllocString(address);
        if b.is_null() {
            return E_OUTOFMEMORY;
        }
        *out.out() = b;
        return S_OK;
    }
    E_NO_IPADDRESS_IN_CONNSTR
}

unsafe fn xbtp_open(ip: *const u16, port: u32, opts: &OpenSessionOptions, out: *mut *mut c_void, redirect: u32) -> HRESULT {
    let o = opts as *const OpenSessionOptions as *const c_void;
    #[cfg(test)]
    {
        if redirect != 0 {
            return XbtpOpenSessionEx2(ip, (port & XBTP_PORT_KIND_MASK) | redirect, o, &FILEIO_EXTENSION_ID, &IID_XBTP_CLIENT_SESSION, out);
        }
    }
    let _ = redirect;
    XbtpOpenSession2(ip, port, o, &FILEIO_EXTENSION_ID, &IID_XBTP_CLIENT_SESSION, out)
}

pub unsafe fn open_vm_session(address: *const u16, port: u32, out: &mut Com, redirect: u32) -> HRESULT {
    let mut last = GetTickCount64();
    let mut ip = Bstr::null();
    let mut key = Bstr::null();
    let mut explicit = 0u32;
    let hr = split_console_address(address, port, &mut ip, Some(&mut key), &mut explicit);
    if hr < 0 {
        return hr;
    }
    let mut opts = OpenSessionOptions { size: OPEN_OPTIONS_SIZE, session_key: [0; SESSION_KEY_CHARS], skip_ready_check: 0, allow_unneeded_key: 0, quiet: 0, reserved: 0, timeout_ms: DEFAULT_OPEN_SESSION_TIMEOUT_MS as u32 };
    if !key.0.is_null() {
        let n = wcslen(key.0);
        if n >= SESSION_KEY_CHARS {
            std::process::abort();
        }
        core::ptr::copy_nonoverlapping(key.0, opts.session_key.as_mut_ptr(), n);
    }
    let mut delay: u64 = 0xfa;
    let mut elapsed: u64 = 0;
    let limit = DEFAULT_OPEN_SESSION_TIMEOUT_MS;
    let mut hr;
    loop {
        hr = xbtp_open(ip.0, port, &opts, out.out(), redirect);
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
    hr
}

unsafe fn sv(s: *mut c_void) -> &'static ClientSessionVtbl {
    vt::<ClientSessionVtbl>(s)
}

pub unsafe fn add_ref(p: *mut c_void) -> Com {
    if !p.is_null() {
        (vt::<UnknownVtbl>(p).add_ref)(p);
    }
    Com(p)
}

pub fn transport_status(status: i32) -> HRESULT {
    (TRANSPORT_STATUS_BASE | (status as u32 & 0xff)) as i32
}

pub unsafe fn receive_end_message<R: Props>(session: *mut c_void, timeout: u32, exchange: i32, resp: &mut R) -> HRESULT {
    let mut status = 0i32;
    let mut msg = Com::null();
    let hr = (sv(session).receive_end_message)(session, timeout, exchange, &mut status, msg.out());
    if hr == E_PENDING {
        return hr;
    }
    if hr < 0 {
        return hr;
    }
    if status != 0 {
        return transport_status(status);
    }
    deserialize(resp, msg.0)
}

pub unsafe fn send_command<Q: Props, R: Props>(session: *mut c_void, op: u32, req: &Q, resp: &mut R) -> HRESULT {
    if session.is_null() {
        return E_POINTER;
    }
    let mut msg = Com::null();
    let hr = XbtpCreateMessage(&IID_XBTP_MESSAGE_IMPL, msg.out());
    if hr < 0 {
        return hr;
    }
    let hr = serialize_impl(req, msg.0);
    if hr < 0 {
        return hr;
    }
    let mut exchange = 0i32;
    let hr = (sv(session).send_begin_message)(session, op, msg.0, &mut exchange);
    if hr < 0 {
        return hr;
    }
    receive_end_message(session, XTF_RECEIVE_RECOMMENDED_TIMEOUT_MS, exchange, resp)
}

unsafe fn command_result<Q: Props, R: Props>(session: *mut c_void, op: u32, req: &Q, resp: &mut R, result: impl Fn(&R) -> i32) -> HRESULT {
    let hr = send_command(session, op, req, resp);
    if hr < 0 {
        return hr;
    }
    result(resp)
}

fn name_msg(name: *const u16) -> Result<MsgFileName, HRESULT> {
    if name.is_null() {
        return Ok(MsgFileName::default());
    }
    let b = unsafe { SysAllocString(name) };
    if b.is_null() {
        return Err(E_OUTOFMEMORY);
    }
    Ok(MsgFileName { file_name: Bstr(b) })
}

#[derive(Default)]
pub struct MsgGetFileInfoResponse {
    pub base: MsgResponse,
    pub info: crate::file::FileInfo,
}

impl Props for MsgGetFileInfoResponse {
    unsafe fn deserialize_properties(&mut self, props: *mut c_void) -> HRESULT {
        let hr = self.base.deserialize_properties(props);
        if hr < 0 {
            return hr;
        }
        if self.base.result < 0 {
            return hr;
        }
        get_file_info_properties(props, &mut self.info)
    }
    unsafe fn serialize_properties(&self, props: *mut c_void) -> HRESULT {
        let hr = self.base.serialize_properties(props);
        if hr < 0 {
            return hr;
        }
        crate::find::set_file_info_properties(props, &self.info)
    }
}

pub struct NoProps;

impl Props for NoProps {
    unsafe fn deserialize_properties(&mut self, _props: *mut c_void) -> HRESULT {
        S_OK
    }
    unsafe fn serialize_properties(&self, _props: *mut c_void) -> HRESULT {
        S_OK
    }
}

pub struct RemoteFile {
    pub session: Com,
}

pub struct FoundEntry {
    pub name: Bstr,
    pub info: XtfFileInfo,
}

impl FoundEntry {
    pub fn info_ptr(&self) -> XtfFileInfo {
        let mut i = self.info;
        i.file_name = self.name.0;
        i
    }
}

pub struct FindResult {
    pub root: Bstr,
    pub pattern: Bstr,
}

fn alloc_or_empty(b: &Bstr) -> Result<Bstr, HRESULT> {
    let empty = [0u16];
    let src = if b.0.is_null() { empty.as_ptr() } else { b.0 as *const u16 };
    let r = unsafe { SysAllocString(src) };
    if r.is_null() {
        Err(E_OUTOFMEMORY)
    } else {
        Ok(Bstr(r))
    }
}

impl RemoteFile {
    pub unsafe fn get_file_size(&self, name: *const u16, size: *mut u64) -> HRESULT {
        let req = match name_msg(name) {
            Ok(r) => r,
            Err(e) => return e,
        };
        let mut resp = MsgGetFileSizeResponse::default();
        let hr = command_result(self.session.0, OP_GET_FILE_SIZE, &req, &mut resp, |r| r.base.result);
        if hr >= 0 {
            if size.is_null() {
                return E_POINTER;
            }
            *size = resp.size;
        }
        hr
    }

    pub unsafe fn get_file_attributes(&self, name: *const u16, attrs: *mut u32) -> HRESULT {
        let req = match name_msg(name) {
            Ok(r) => r,
            Err(e) => return e,
        };
        let mut resp = MsgGetFileAttributesResponse::default();
        let hr = command_result(self.session.0, OP_GET_FILE_ATTRIBUTES, &req, &mut resp, |r| r.base.result);
        if hr >= 0 {
            if attrs.is_null() {
                return E_POINTER;
            }
            *attrs = resp.attributes;
        }
        hr
    }

    pub unsafe fn set_file_attributes(&self, name: *const u16, attrs: u32) -> HRESULT {
        let base = match name_msg(name) {
            Ok(r) => r,
            Err(e) => return e,
        };
        let req = MsgSetFileAttributes { base, attributes: attrs };
        let mut resp = MsgResponse::default();
        command_result(self.session.0, OP_SET_FILE_ATTRIBUTES, &req, &mut resp, |r| r.result)
    }

    pub unsafe fn modify_file_attributes(&self, name: *const u16, add: u32, remove: u32) -> HRESULT {
        let base = match name_msg(name) {
            Ok(r) => r,
            Err(e) => return e,
        };
        let req = MsgModifyFileAttributes { base, add, remove };
        let mut resp = MsgResponse::default();
        command_result(self.session.0, OP_MODIFY_FILE_ATTRIBUTES, &req, &mut resp, |r| r.result)
    }

    pub unsafe fn delete_files(&self, name: *const u16, flags: u32) -> HRESULT {
        let base = match name_msg(name) {
            Ok(r) => r,
            Err(e) => return e,
        };
        let req = MsgFlags { base, flags };
        let mut resp = MsgResponse::default();
        command_result(self.session.0, OP_DELETE_FILE, &req, &mut resp, |r| r.result)
    }

    pub unsafe fn create_directory(&self, name: *const u16) -> HRESULT {
        let req = match name_msg(name) {
            Ok(r) => r,
            Err(e) => return e,
        };
        let mut resp = MsgResponse::default();
        command_result(self.session.0, OP_CREATE_DIRECTORY, &req, &mut resp, |r| r.result)
    }

    pub unsafe fn remove_directory(&self, name: *const u16, flags: u32) -> HRESULT {
        let base = match name_msg(name) {
            Ok(r) => r,
            Err(e) => return e,
        };
        let req = MsgFlags { base, flags };
        let mut resp = MsgResponse::default();
        command_result(self.session.0, OP_REMOVE_DIRECTORY, &req, &mut resp, |r| r.result)
    }

    pub unsafe fn get_file_info(&self, name: *const u16, out: *mut XtfFileInfo) -> HRESULT {
        let req = match name_msg(name) {
            Ok(r) => r,
            Err(e) => return e,
        };
        let mut resp = MsgGetFileInfoResponse::default();
        let hr = command_result(self.session.0, OP_GET_FILE_INFO, &req, &mut resp, |r| r.base.result);
        if hr >= 0 {
            if out.is_null() {
                return E_POINTER;
            }
            *out = XtfFileInfo { file_name: null_mut(), attributes: resp.info.attributes, creation: resp.info.creation, access: resp.info.access, write: resp.info.write, size: resp.info.size, reserved: [0; 3] };
        }
        hr
    }

    pub unsafe fn find_files(&self, pattern: *const u16, include: u32, exclude: u32, recursion: u32, flags: u32, on_file: &mut dyn FnMut(&Bstr, &Bstr, &FoundEntry) -> HRESULT, out: Option<&mut FindResult>) -> HRESULT {
        let session = self.session.0;
        if session.is_null() {
            return E_POINTER;
        }
        let base = match name_msg(pattern) {
            Ok(r) => r,
            Err(e) => return e,
        };
        let req = MsgFindFiles { base: MsgFlags { base, flags }, include, exclude, recursion };
        let mut msg = Com::null();
        let hr = XbtpCreateMessage(&IID_XBTP_MESSAGE_IMPL, msg.out());
        if hr < 0 {
            return hr;
        }
        let hr = serialize_impl(&req, msg.0);
        if hr < 0 {
            return hr;
        }
        let mut exchange = 0i32;
        let hr = (sv(session).send_begin_message)(session, OP_FIND_FILES, msg.0, &mut exchange);
        if hr < 0 {
            return hr;
        }
        let mut out = out;
        let mut first = true;
        let mut root = Bstr::null();
        let mut pat = Bstr::null();
        loop {
            let mut ty = 0i16;
            let mut status = 0i32;
            let mut reply = Com::null();
            let hr = (sv(session).receive_message)(session, XTF_RECEIVE_RECOMMENDED_TIMEOUT_MS, exchange, &mut ty, &mut status, reply.out());
            if hr < 0 {
                return hr;
            }
            if status != 0 {
                return transport_status(status);
            }
            let mut props = Com::null();
            let hr = deserialize_json_body(reply.0, &mut props);
            if hr < 0 {
                return hr;
            }
            if ty == XBTP_MESSAGE_TYPE_END {
                let mut r = MsgResponse::default();
                let hr = r.deserialize_properties(props.0);
                if hr < 0 {
                    return hr;
                }
                return r.result;
            }
            if ty != XBTP_MESSAGE_TYPE_MORE {
                return (UNEXPECTED_MESSAGE_BASE | (ty as u16 as u32 & 0xff)) as i32;
            }
            if first {
                let mut info = MsgFindFilesInfo::default();
                let hr = get_string(props.0, "RootDirectory", &mut info.root_directory);
                if hr < 0 {
                    return hr;
                }
                root = core::mem::take(&mut info.root_directory);
                let hr = get_string(props.0, "SearchPattern", &mut info.search_pattern);
                if hr < 0 {
                    return hr;
                }
                pat = core::mem::take(&mut info.search_pattern);
                if let Some(o) = out.as_deref_mut() {
                    match alloc_or_empty(&root) {
                        Ok(b) => o.root = b,
                        Err(e) => return e,
                    }
                    match alloc_or_empty(&pat) {
                        Ok(b) => o.pattern = b,
                        Err(e) => return e,
                    }
                }
                first = false;
                continue;
            }
            let mut name = Bstr::null();
            let mut fi = crate::file::FileInfo::default();
            let hr = get_string(props.0, "FileName", &mut name);
            let hr = if hr < 0 { hr } else { get_file_info_properties(props.0, &mut fi) };
            if hr < 0 {
                return hr;
            }
            let entry = FoundEntry { name, info: XtfFileInfo { file_name: null_mut(), attributes: fi.attributes, creation: fi.creation, access: fi.access, write: fi.write, size: fi.size, reserved: [0; 3] } };
            let hr = on_file(&root, &pat, &entry);
            if hr < 0 {
                return hr;
            }
        }
    }
}

pub fn wcsicmp(a: &[u16], b: &[u16]) -> i32 {
    let low = |c: u16| if (b'A' as u16..=b'Z' as u16).contains(&c) { c + 32 } else { c };
    for i in 0.. {
        let x = low(a.get(i).copied().unwrap_or(0)) as i32;
        let y = low(b.get(i).copied().unwrap_or(0)) as i32;
        if x != y || x == 0 {
            return x - y;
        }
    }
    0
}

pub unsafe fn bstr_slice<'a>(b: &Bstr) -> &'a [u16] {
    if b.0.is_null() {
        &[]
    } else {
        core::slice::from_raw_parts(b.0, wcslen(b.0))
    }
}

#[derive(Default)]
pub struct FileList {
    pub entries: Vec<FoundEntry>,
    pub total_size: u64,
    pub total_rounded: u64,
    pub files: u32,
    pub dirs: u32,
}

impl FileList {
    pub fn add(&mut self, e: FoundEntry) {
        if e.info.attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
            self.dirs += 1;
        } else {
            self.files += 1;
            self.total_size += e.info.size;
            self.total_rounded += (e.info.size + 0xfff) & !0xfff;
        }
        self.entries.push(e);
    }

    pub fn sort(&mut self) {
        self.entries.sort_by(|a, b| {
            let da = (a.info.attributes & FILE_ATTRIBUTE_DIRECTORY != 0) as i32;
            let db = (b.info.attributes & FILE_ATTRIBUTE_DIRECTORY != 0) as i32;
            if da != db {
                return (db - da).cmp(&0);
            }
            let (x, y) = unsafe { (bstr_slice(&a.name), bstr_slice(&b.name)) };
            let c = wcsicmp(x, y);
            if c != 0 {
                return c.cmp(&0);
            }
            if x < y {
                core::cmp::Ordering::Less
            } else {
                core::cmp::Ordering::Greater
            }
        });
    }
}

fn copy_entry(e: &FoundEntry) -> Result<FoundEntry, HRESULT> {
    let name = if e.name.0.is_null() {
        Bstr::null()
    } else {
        let b = unsafe { SysAllocString(e.name.0) };
        if b.is_null() {
            return Err(E_OUTOFMEMORY);
        }
        Bstr(b)
    };
    Ok(FoundEntry { name, info: e.info })
}

unsafe fn invoke_callback(callback: *mut c_void, root: &Bstr, pattern: &Bstr, e: &FoundEntry) -> HRESULT {
    let info = e.info_ptr();
    (vt::<FindCallbackVtbl>(callback).on_file)(callback, root.0, pattern.0, &info)
}

pub unsafe fn is_application_path(path: *const u16) -> bool {
    if path.is_null() || *path != b'{' as u16 {
        return false;
    }
    let s = core::slice::from_raw_parts(path.add(1), wcslen(path.add(1)));
    let pat = [b'}' as u16, b':' as u16, b'\\' as u16];
    s.windows(3).any(|w| w == pat)
}

struct State {
    address: Bstr,
    port: u32,
    session_a: Com,
    session_b: Com,
    direct: Option<u32>,
}

#[repr(C)]
pub struct FileIOClient {
    pub vtbl: *const ClientVtbl,
    pub vtbl2: *const Client2Vtbl,
    pub vtbl3: *const Client3Vtbl,
    refs: AtomicU32,
    state: Mutex<State>,
    remote: Mutex<RemoteFile>,
    redirect: u32,
}

type P = *mut c_void;
type W = *const u16;

#[repr(C)]
pub struct ClientVtbl {
    pub query_interface: unsafe extern "system" fn(P, *const GUID, *mut P) -> HRESULT,
    pub add_ref: unsafe extern "system" fn(P) -> u32,
    pub release: unsafe extern "system" fn(P) -> u32,
    pub find_files: unsafe extern "system" fn(P, W, u32, u32, u32, u32, P) -> HRESULT,
    pub copy_files: unsafe extern "system" fn(P, W, u32, u32, u32, W, u32, P) -> HRESULT,
    pub delete_files: unsafe extern "system" fn(P, W, u32, u32, u32, u32, P) -> HRESULT,
    pub get_file_size: unsafe extern "system" fn(P, W, *mut u64) -> HRESULT,
    pub get_file_attributes: unsafe extern "system" fn(P, W, *mut u32) -> HRESULT,
    pub set_file_attributes: unsafe extern "system" fn(P, W, u32, u32, u32, u32, u32, P) -> HRESULT,
    pub create_directory: unsafe extern "system" fn(P, W) -> HRESULT,
    pub remove_directory: unsafe extern "system" fn(P, W, u32, P) -> HRESULT,
    pub get_file_info: unsafe extern "system" fn(P, W, *mut XtfFileInfo) -> HRESULT,
    pub open_session: unsafe extern "system" fn(P) -> HRESULT,
    pub start_application_operations: unsafe extern "system" fn(P, W) -> HRESULT,
    pub end_application_operations: unsafe extern "system" fn(P, W) -> HRESULT,
}

#[repr(C)]
pub struct Client2Vtbl {
    pub query_interface: unsafe extern "system" fn(P, *const GUID, *mut P) -> HRESULT,
    pub add_ref: unsafe extern "system" fn(P) -> u32,
    pub release: unsafe extern "system" fn(P) -> u32,
    pub copy_files: unsafe extern "system" fn(P, W, *const W, u32, W, u32, P) -> HRESULT,
}

#[repr(C)]
pub struct Client3Vtbl {
    pub query_interface: unsafe extern "system" fn(P, *const GUID, *mut P) -> HRESULT,
    pub add_ref: unsafe extern "system" fn(P) -> u32,
    pub release: unsafe extern "system" fn(P) -> u32,
    pub local_application_operations_start: unsafe extern "system" fn(P, W) -> HRESULT,
    pub local_application_operations_end: unsafe extern "system" fn(P, W) -> HRESULT,
    pub get_storage_device_free_space: unsafe extern "system" fn(P, W, *mut u64, *mut u64) -> HRESULT,
    pub copy_files_direct_list: unsafe extern "system" fn(P, P, u32, W, W, u32, P, P) -> HRESULT,
    pub copy_files_perf: unsafe extern "system" fn(P, W, u32, u32, u32, W, u32, P, P) -> HRESULT,
    pub calculate_incremental_list: usize,
}

pub static CLIENT_VTBL: ClientVtbl = ClientVtbl {
    query_interface: c_qi,
    add_ref: c_add_ref,
    release: c_release,
    find_files: c_find_files,
    copy_files: crate::client_copy::c_copy_files,
    delete_files: c_delete_files,
    get_file_size: c_get_file_size,
    get_file_attributes: c_get_file_attributes,
    set_file_attributes: c_set_file_attributes,
    create_directory: c_create_directory,
    remove_directory: c_remove_directory,
    get_file_info: c_get_file_info,
    open_session: c_open_session,
    start_application_operations: c_start_application_operations,
    end_application_operations: c_end_application_operations,
};

pub static CLIENT2_VTBL: Client2Vtbl = Client2Vtbl {
    query_interface: i2_qi,
    add_ref: i2_add_ref,
    release: i2_release,
    copy_files: crate::client_copy::i2_copy_files,
};

pub static CLIENT3_VTBL: Client3Vtbl = Client3Vtbl {
    query_interface: i3_qi,
    add_ref: i3_add_ref,
    release: i3_release,
    local_application_operations_start: i3_local_start,
    local_application_operations_end: i3_local_end,
    get_storage_device_free_space: i3_free_space,
    copy_files_direct_list: crate::client_copy::i3_copy_files_direct_list,
    copy_files_perf: crate::client_copy::i3_copy_files_perf,
    calculate_incremental_list: crate::client_copy::I3_CALCULATE_INCREMENTAL_LIST,
};

impl FileIOClient {
    pub unsafe fn new(address: *const u16, hr_out: &mut HRESULT) -> *mut FileIOClient {
        let c = Box::into_raw(Box::new(FileIOClient {
            vtbl: &CLIENT_VTBL,
            vtbl2: &CLIENT2_VTBL,
            vtbl3: &CLIENT3_VTBL,
            refs: AtomicU32::new(1),
            state: Mutex::new(State { address: Bstr::null(), port: 0, session_a: Com::null(), session_b: Com::null(), direct: None }),
            remote: Mutex::new(RemoteFile { session: Com::null() }),
            redirect: test_port(),
        }));
        *hr_out = (*c).construct(address);
        c
    }

    unsafe fn construct(&self, address: *const u16) -> HRESULT {
        let mut fin = Bstr::null();
        let hr = get_final_address(address, &mut fin);
        if hr < 0 {
            return hr;
        }
        let mut ip = Bstr::null();
        let mut port = 0u32;
        let hr = split_console_address(fin.0, 0, &mut ip, None, &mut port);
        if hr < 0 {
            return hr;
        }
        {
            let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
            st.address = ip;
            st.port = port;
        }
        let hr = self.open_session();
        if hr < 0 {
            return hr;
        }
        let b = {
            let st = self.state.lock().unwrap_or_else(|e| e.into_inner());
            add_ref(st.session_b.0)
        };
        self.remote.lock().unwrap_or_else(|e| e.into_inner()).session = b;
        S_OK
    }

    pub fn as_unknown(this: *mut FileIOClient) -> P {
        this as P
    }

    unsafe fn from2(p: P) -> &'static FileIOClient {
        &*((p as *mut u8).sub(8) as *const FileIOClient)
    }

    unsafe fn from3(p: P) -> &'static FileIOClient {
        &*((p as *mut u8).sub(0x10) as *const FileIOClient)
    }

    pub fn port(&self) -> u32 {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).port
    }

    pub fn connection(&self) -> (Bstr, u32, u32) {
        let st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let addr = Bstr::from_str(&st.address.to_string()).unwrap_or_else(|_| Bstr::null());
        (addr, st.port, self.redirect)
    }

    pub fn session_a(&self) -> Com {
        let st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        unsafe { add_ref(st.session_a.0) }
    }

    pub fn remote_file(&self) -> RemoteFile {
        let r = self.remote.lock().unwrap_or_else(|e| e.into_inner());
        RemoteFile { session: unsafe { add_ref(r.session.0) } }
    }

    pub unsafe fn open_session(&self) -> HRESULT {
        let (addr, port) = {
            let st = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if st.address.0.is_null() {
                return E_UNEXPECTED;
            }
            let b = SysAllocString(st.address.0);
            if b.is_null() {
                return E_OUTOFMEMORY;
            }
            (Bstr(b), st.port)
        };
        let mut link_ok = false;
        let mut a = Com::null();
        let mut hr = open_vm_session(addr.0, port | XBTP_PORT_KIND_LINK, &mut a, self.redirect);
        if hr >= 0 {
            link_ok = true;
        } else {
            a = Com::null();
            hr = open_vm_session(addr.0, port, &mut a, self.redirect);
        }
        {
            let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
            st.session_a = Com::null();
            if hr >= 0 {
                st.session_a = a;
            }
            st.session_b = Com::null();
        }
        if hr < 0 {
            return hr;
        }
        let mut b = Com::null();
        let mut hr = open_vm_session(addr.0, port, &mut b, self.redirect);
        let mut new_port = port;
        if hr < 0 && link_ok && port == XBTP_PORT_TITLE {
            b = Com::null();
            hr = open_vm_session(addr.0, port | XBTP_PORT_KIND_LINK, &mut b, self.redirect);
            if hr >= 0 {
                new_port = port | XBTP_PORT_KIND_LINK;
            }
        }
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if hr >= 0 {
            st.session_b = b;
            st.port = new_port;
        }
        hr
    }

    pub unsafe fn query_interface(this: *mut FileIOClient, iid: *const GUID, out: *mut P) -> HRESULT {
        if out.is_null() {
            return E_POINTER;
        }
        if iid.is_null() {
            return E_INVALIDARG;
        }
        let iid = *iid;
        let p: P = if iid == IID_XTF_FILEIO_CLIENT || iid == IID_IUNKNOWN || iid == IID_XTF_FILEIO_CLIENT_BASE {
            this as P
        } else if iid == IID_XTF_FILEIO_CLIENT2 {
            (this as *mut u8).add(8) as P
        } else if iid == IID_XTF_FILEIO_CLIENT3 {
            (this as *mut u8).add(0x10) as P
        } else {
            return E_NOINTERFACE;
        };
        (*this).refs.fetch_add(1, Ordering::SeqCst);
        *out = p;
        S_OK
    }

    pub unsafe fn release(this: *mut FileIOClient) -> u32 {
        let n = (*this).refs.fetch_sub(1, Ordering::SeqCst) - 1;
        if n == 0 {
            drop(Box::from_raw(this));
        }
        n
    }

    pub unsafe fn start_application_operations(&self, path: W) -> HRESULT {
        let hr = i3_local_start(&self.vtbl3 as *const _ as P, path);
        if hr != E_APP_OPERATIONS_REMOTE {
            return hr;
        }
        let req = match name_msg(path) {
            Ok(r) => r,
            Err(e) => return e,
        };
        let a = self.session_a();
        let mut resp = MsgResponse::default();
        command_result(a.0, OP_APPLICATION_OPERATIONS_START, &req, &mut resp, |r| r.result)
    }

    pub unsafe fn end_application_operations(&self, path: W) -> HRESULT {
        let hr = i3_local_end(&self.vtbl3 as *const _ as P, path);
        if hr != E_APP_OPERATIONS_REMOTE {
            return hr;
        }
        let req = match name_msg(path) {
            Ok(r) => r,
            Err(e) => return e,
        };
        let a = self.session_a();
        let mut resp = MsgResponse::default();
        command_result(a.0, OP_APPLICATION_OPERATIONS_END, &req, &mut resp, |r| r.result)
    }

    pub unsafe fn with_application_operations(&self, path: W, f: impl FnOnce() -> HRESULT) -> HRESULT {
        let app = is_application_path(path);
        if app {
            let hr = self.start_application_operations(path);
            if hr < 0 {
                return hr;
            }
        }
        let hr = f();
        if app {
            self.end_application_operations(path);
        }
        hr
    }

    pub unsafe fn find_files(&self, pattern: W, include: u32, exclude: u32, recursion: u32, flags: u32, callback: P) -> HRESULT {
        let rf = self.remote_file();
        let mut cb = |root: &Bstr, pat: &Bstr, e: &FoundEntry| -> HRESULT {
            if callback.is_null() {
                return E_POINTER;
            }
            invoke_callback(callback, root, pat, e)
        };
        rf.find_files(pattern, include, exclude, recursion, flags, &mut cb, None)
    }

    pub unsafe fn list_files(&self, pattern: W, include: u32, exclude: u32, recursion: u32, flags: u32, list: &mut FileList, out: Option<&mut FindResult>) -> HRESULT {
        let rf = self.remote_file();
        let mut cb = |_: &Bstr, _: &Bstr, e: &FoundEntry| -> HRESULT {
            match copy_entry(e) {
                Ok(c) => {
                    list.add(c);
                    S_OK
                }
                Err(err) => err,
            }
        };
        rf.find_files(pattern, include, exclude, recursion, flags, &mut cb, out)
    }

    pub unsafe fn delete_files(&self, pattern: W, include: u32, exclude: u32, recursion: u32, flags: u32, callback: P) -> HRESULT {
        self.with_application_operations(pattern, || {
            let mut list = FileList::default();
            let mut fr = FindResult { root: Bstr::null(), pattern: Bstr::null() };
            let hr = self.list_files(pattern, include, exclude | FILE_ATTRIBUTE_DIRECTORY, recursion, 0, &mut list, Some(&mut fr));
            if hr < 0 {
                return hr;
            }
            list.sort();
            let rf = self.remote_file();
            for e in &list.entries {
                if !callback.is_null() {
                    let hr = invoke_callback(callback, &fr.root, &fr.pattern, e);
                    if hr < 0 {
                        return hr;
                    }
                }
                let hr = rf.delete_files(e.name.0, flags);
                if hr < 0 {
                    return hr;
                }
            }
            hr
        })
    }

    pub unsafe fn set_file_attributes(&self, pattern: W, include: u32, exclude: u32, recursion: u32, add: u32, remove: u32, callback: P) -> HRESULT {
        self.with_application_operations(pattern, || {
            let mut list = FileList::default();
            let mut fr = FindResult { root: Bstr::null(), pattern: Bstr::null() };
            let hr = self.list_files(pattern, include, exclude, recursion, 0, &mut list, Some(&mut fr));
            if hr < 0 {
                return hr;
            }
            list.sort();
            let rf = self.remote_file();
            for e in &list.entries {
                if !callback.is_null() {
                    let hr = invoke_callback(callback, &fr.root, &fr.pattern, e);
                    if hr < 0 {
                        return hr;
                    }
                }
                let hr = rf.modify_file_attributes(e.name.0, add, remove);
                if hr < 0 {
                    return hr;
                }
            }
            hr
        })
    }

    pub unsafe fn create_directory(&self, path: W) -> HRESULT {
        self.with_application_operations(path, || self.remote_file().create_directory(path))
    }

    pub unsafe fn remove_directory(&self, dir: W, flags: u32, callback: P) -> HRESULT {
        self.with_application_operations(dir, || {
            if flags & 1 != 0 {
                let mut pattern: Vec<u16> = if dir.is_null() { Vec::new() } else { core::slice::from_raw_parts(dir, wcslen(dir)).to_vec() };
                pattern.extend_from_slice(&[b'\\' as u16, b'*' as u16, 0]);
                let p = SysAllocString(pattern.as_ptr());
                if p.is_null() {
                    return E_OUTOFMEMORY;
                }
                let p = Bstr(p);
                let hr = self.delete_files(p.0, 0, FILE_ATTRIBUTE_DIRECTORY, 0, flags & 0x8000_0000, callback);
                if hr < 0 {
                    return hr;
                }
                let mut dirs = FileList::default();
                let hr = self.list_files(p.0, FILE_ATTRIBUTE_DIRECTORY, 0, 0, 0, &mut dirs, None);
                if hr < 0 {
                    return hr;
                }
                dirs.sort();
                for e in &dirs.entries {
                    let hr = self.remove_directory(e.name.0, flags, callback);
                    if hr < 0 {
                        return hr;
                    }
                }
            }
            self.remote_file().remove_directory(dir, flags)
        })
    }

    pub unsafe fn get_storage_device_free_space(&self, device: W, total: *mut u64, free: *mut u64) -> HRESULT {
        if device.is_null() {
            return E_INVALIDARG;
        }
        if total.is_null() || free.is_null() {
            return E_POINTER;
        }
        *total = 0;
        *free = 0;
        let b = SysAllocString(device);
        if b.is_null() {
            return E_OUTOFMEMORY;
        }
        let req = MsgStorageDevice { storage_device: Bstr(b) };
        let a = self.session_a();
        let mut resp = MsgStorageDeviceFreeSpaceResponse::default();
        let hr = command_result(a.0, OP_STORAGE_DEVICE_FREE_SPACE, &req, &mut resp, |r| r.base.result);
        if hr >= 0 {
            *total = resp.total;
            *free = resp.free;
        }
        hr
    }

    pub unsafe fn direct_protocol_level(&self) -> u32 {
        {
            let st = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(l) = st.direct {
                return l;
            }
        }
        let level = self.query_direct_protocol_level();
        self.state.lock().unwrap_or_else(|e| e.into_inner()).direct = Some(level);
        level
    }

    unsafe fn query_direct_protocol_level(&self) -> u32 {
        let a = self.session_a();
        if a.0.is_null() {
            return 0;
        }
        let mut msg = Com::null();
        if XbtpCreateMessage(&IID_XBTP_MESSAGE_IMPL, msg.out()) < 0 {
            return 0;
        }
        if serialize_impl(&NoProps, msg.0) < 0 {
            return 0;
        }
        let mut exchange = 0i32;
        if (sv(a.0).send_begin_message)(a.0, OP_TRANSFER_FILES_SUPPORTED, msg.0, &mut exchange) < 0 {
            return 0;
        }
        let mut status = 0i32;
        let mut reply = Com::null();
        if (sv(a.0).receive_end_message)(a.0, DIRECT_PROTOCOL_QUERY_TIMEOUT_MS, exchange, &mut status, reply.out()) < 0 || status != 0 {
            return 0;
        }
        let mut props = Com::null();
        if deserialize_json_body(reply.0, &mut props) < 0 {
            return 0;
        }
        let mut result = 0i32;
        if get_i32(props.0, "Result", &mut result) < 0 || result < 0 {
            return 0;
        }
        let mut level = 0u32;
        if get_u32(props.0, "ProtocolLevelSupported", &mut level) < 0 {
            return 1;
        }
        level
    }
}

impl Drop for FileIOClient {
    fn drop(&mut self) {}
}

pub unsafe fn create_client(address: W, iid: *const GUID, ppv: *mut P) -> HRESULT {
    if ppv.is_null() {
        return E_POINTER;
    }
    *ppv = null_mut();
    let mut hr = S_OK;
    let c = FileIOClient::new(address, &mut hr);
    if hr < 0 {
        FileIOClient::release(c);
        return hr;
    }
    let hr = FileIOClient::query_interface(c, iid, ppv);
    FileIOClient::release(c);
    hr
}

unsafe fn me(p: P) -> &'static FileIOClient {
    &*(p as *const FileIOClient)
}

unsafe extern "system" fn c_qi(this: P, iid: *const GUID, out: *mut P) -> HRESULT {
    FileIOClient::query_interface(this as *mut FileIOClient, iid, out)
}

unsafe extern "system" fn c_add_ref(this: P) -> u32 {
    me(this).refs.fetch_add(1, Ordering::SeqCst) + 1
}

unsafe extern "system" fn c_release(this: P) -> u32 {
    FileIOClient::release(this as *mut FileIOClient)
}

unsafe extern "system" fn c_find_files(this: P, pattern: W, include: u32, exclude: u32, recursion: u32, flags: u32, callback: P) -> HRESULT {
    me(this).find_files(pattern, include, exclude, recursion, flags, callback)
}

unsafe extern "system" fn c_delete_files(this: P, pattern: W, include: u32, exclude: u32, recursion: u32, flags: u32, callback: P) -> HRESULT {
    me(this).delete_files(pattern, include, exclude, recursion, flags, callback)
}

unsafe extern "system" fn c_get_file_size(this: P, name: W, size: *mut u64) -> HRESULT {
    me(this).remote_file().get_file_size(name, size)
}

unsafe extern "system" fn c_get_file_attributes(this: P, name: W, attrs: *mut u32) -> HRESULT {
    me(this).remote_file().get_file_attributes(name, attrs)
}

unsafe extern "system" fn c_set_file_attributes(this: P, pattern: W, include: u32, exclude: u32, recursion: u32, add: u32, remove: u32, callback: P) -> HRESULT {
    me(this).set_file_attributes(pattern, include, exclude, recursion, add, remove, callback)
}

unsafe extern "system" fn c_create_directory(this: P, path: W) -> HRESULT {
    me(this).create_directory(path)
}

unsafe extern "system" fn c_remove_directory(this: P, dir: W, flags: u32, callback: P) -> HRESULT {
    me(this).remove_directory(dir, flags, callback)
}

unsafe extern "system" fn c_get_file_info(this: P, name: W, out: *mut XtfFileInfo) -> HRESULT {
    me(this).remote_file().get_file_info(name, out)
}

unsafe extern "system" fn c_open_session(this: P) -> HRESULT {
    me(this).open_session()
}

unsafe extern "system" fn c_start_application_operations(this: P, path: W) -> HRESULT {
    me(this).start_application_operations(path)
}

unsafe extern "system" fn c_end_application_operations(this: P, path: W) -> HRESULT {
    me(this).end_application_operations(path)
}

unsafe extern "system" fn i2_qi(this: P, iid: *const GUID, out: *mut P) -> HRESULT {
    FileIOClient::query_interface(FileIOClient::from2(this) as *const _ as *mut FileIOClient, iid, out)
}

unsafe extern "system" fn i2_add_ref(this: P) -> u32 {
    FileIOClient::from2(this).refs.fetch_add(1, Ordering::SeqCst) + 1
}

unsafe extern "system" fn i2_release(this: P) -> u32 {
    FileIOClient::release(FileIOClient::from2(this) as *const _ as *mut FileIOClient)
}

unsafe extern "system" fn i3_qi(this: P, iid: *const GUID, out: *mut P) -> HRESULT {
    FileIOClient::query_interface(FileIOClient::from3(this) as *const _ as *mut FileIOClient, iid, out)
}

unsafe extern "system" fn i3_add_ref(this: P) -> u32 {
    FileIOClient::from3(this).refs.fetch_add(1, Ordering::SeqCst) + 1
}

unsafe extern "system" fn i3_release(this: P) -> u32 {
    FileIOClient::release(FileIOClient::from3(this) as *const _ as *mut FileIOClient)
}

unsafe extern "system" fn i3_local_start(_this: P, _path: W) -> HRESULT {
    S_OK
}

unsafe extern "system" fn i3_local_end(_this: P, _path: W) -> HRESULT {
    S_OK
}

unsafe extern "system" fn i3_free_space(this: P, device: W, total: *mut u64, free: *mut u64) -> HRESULT {
    FileIOClient::from3(this).get_storage_device_free_space(device, total, free)
}

pub unsafe fn client_from_i3(p: P) -> &'static FileIOClient {
    FileIOClient::from3(p)
}

pub unsafe fn client_from_i2(p: P) -> &'static FileIOClient {
    FileIOClient::from2(p)
}

pub unsafe fn client_from_main(p: P) -> &'static FileIOClient {
    me(p)
}
