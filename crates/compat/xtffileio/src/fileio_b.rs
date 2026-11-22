use crate::com::*;
use crate::msg::*;
use core::ffi::c_void;
use core::ptr::null_mut;
use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Instant;

#[path = "fileio_b_xfer.rs"]
pub mod xfer;

#[path = "fileio_b_diff.rs"]
pub mod diff;

#[path = "fileio_b_stream.rs"]
pub mod stream;

#[cfg(test)]
#[path = "fileio_b_tests.rs"]
mod tests;

pub const DIRECT_PROTOCOL_CURRENT_LEVEL: u32 = 2;
pub const HR_NO_SESSION: HRESULT = 0x8007_139Fu32 as i32;
pub const HR_CANCELLED: HRESULT = 0x8007_04C7u32 as i32;
pub const HR_CONNRESET: HRESULT = 0x8007_2746u32 as i32;
pub const E_UNEXPECTED: HRESULT = 0x8000_FFFFu32 as i32;
pub const E_PENDING: HRESULT = 0x8000_000Au32 as i32;
pub const E_NOTIMPL: HRESULT = 0x8000_4001u32 as i32;
pub const HR_BAD_LENGTH: HRESULT = 0x8007_00EAu32 as i32;
pub const DEFAULT_RECEIVE_TIMEOUT: u32 = 30000;
pub const MAX_ERROR_TOKENS: usize = 0x40;
pub const IID_XBTP_MESSAGE: GUID = GUID::new(0xd24904a1, 0x8c12, 0x4856, [0x9d, 0x86, 0x4e, 0x1f, 0x3b, 0x55, 0x5d, 0x73]);

#[repr(C)]
pub struct DirectSessionVtbl {
    pub base: UnknownVtbl,
    pub send_more_message: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    pub slot4: usize,
    pub receive_more_message: unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> HRESULT,
    pub get_message_received_event: unsafe extern "system" fn(*mut c_void, *mut isize) -> HRESULT,
}

#[derive(Clone, Copy)]
pub struct Session(pub *mut c_void);

impl Session {
    pub unsafe fn send_more(&self, msg: *mut c_void) -> HRESULT {
        if self.0.is_null() {
            return HR_NO_SESSION;
        }
        (vt::<DirectSessionVtbl>(self.0).send_more_message)(self.0, msg)
    }

    pub unsafe fn receive_more(&self, timeout: u32) -> Result<Com, HRESULT> {
        if self.0.is_null() {
            return Err(HR_NO_SESSION);
        }
        let mut m = Com::null();
        let hr = (vt::<DirectSessionVtbl>(self.0).receive_more_message)(self.0, timeout, m.out());
        if hr < 0 {
            Err(hr)
        } else {
            Ok(m)
        }
    }

    pub unsafe fn received_event(&self) -> Result<isize, HRESULT> {
        let mut h = 0isize;
        let hr = (vt::<DirectSessionVtbl>(self.0).get_message_received_event)(self.0, &mut h);
        if hr < 0 {
            Err(hr)
        } else {
            Ok(h)
        }
    }
}

pub unsafe fn new_object() -> Result<Com, HRESULT> {
    let mut o = Com::null();
    let hr = XbtpCreateJsonObject(o.out());
    if hr < 0 {
        Err(hr)
    } else {
        Ok(o)
    }
}

pub unsafe fn new_message() -> Result<Com, HRESULT> {
    let mut m = Com::null();
    let hr = XbtpCreateMessage(&IID_XBTP_MESSAGE, m.out());
    if hr < 0 {
        Err(hr)
    } else {
        Ok(m)
    }
}

pub unsafe fn set_number(obj: *mut c_void, name: &str, v: f64) -> HRESULT {
    if obj.is_null() {
        return E_INVALIDARG;
    }
    let n = wide(name);
    (vt::<JsonObjectVtbl>(obj).set_number)(obj, n.as_ptr(), v)
}

pub unsafe fn get_number(obj: *mut c_void, name: &str, out: &mut f64) -> HRESULT {
    if obj.is_null() {
        return E_INVALIDARG;
    }
    let n = wide(name);
    (vt::<JsonObjectVtbl>(obj).get_number)(obj, n.as_ptr(), out)
}

pub unsafe fn set_wstr(obj: *mut c_void, name: &str, v: &[u16]) -> HRESULT {
    if obj.is_null() {
        return E_INVALIDARG;
    }
    let n = wide(name);
    let mut z = v.to_vec();
    z.push(0);
    (vt::<JsonObjectVtbl>(obj).set_string)(obj, n.as_ptr(), z.as_ptr())
}

pub unsafe fn get_bstr(obj: *mut c_void, name: &str, out: &mut Bstr) -> HRESULT {
    if obj.is_null() {
        return E_INVALIDARG;
    }
    get_string(obj, name, out)
}

pub unsafe fn bstr_units(b: &Bstr) -> Vec<u16> {
    if b.0.is_null() {
        return Vec::new();
    }
    let mut n = 0;
    while *b.0.add(n) != 0 {
        n += 1;
    }
    core::slice::from_raw_parts(b.0, n).to_vec()
}

pub unsafe fn get_low_flags(obj: *mut c_void, out: &mut u32) -> HRESULT {
    let mut v = 0u64;
    let hr = get_u64(obj, "Flags", &mut v);
    if hr >= 0 {
        *out = v as u32;
    }
    hr
}

pub unsafe fn serialize_object(obj: *mut c_void, message: *mut c_void) -> HRESULT {
    if obj.is_null() || message.is_null() {
        return E_INVALIDARG;
    }
    let mut w = Bstr::null();
    let hr = (vt::<JsonObjectVtbl>(obj).stringify)(obj, w.out());
    if hr < 0 {
        return hr;
    }
    (vt::<MessageImplVtbl>(message).set_json_body_w)(message, w.0)
}

pub unsafe fn serialize_response_object(obj: *mut c_void, response: *mut c_void) -> HRESULT {
    if obj.is_null() || response.is_null() {
        return E_INVALIDARG;
    }
    let mut w = Bstr::null();
    let hr = (vt::<JsonObjectVtbl>(obj).stringify)(obj, w.out());
    if hr < 0 {
        return hr;
    }
    (vt::<MessageVtbl>(response).set_json_body_w)(response, w.0)
}

pub unsafe fn binary_body(message: *mut c_void) -> Result<(*mut u8, u32), HRESULT> {
    if message.is_null() {
        return Err(E_INVALIDARG);
    }
    let mut n = 0u32;
    let mut p: *mut u8 = null_mut();
    let hr = (vt::<MessageImplVtbl>(message).get_binary_body)(message, &mut n, &mut p);
    if hr < 0 {
        Err(hr)
    } else {
        Ok((p, n))
    }
}

pub unsafe fn binary_slice<'a>(p: *mut u8, n: u32) -> &'a [u8] {
    if p.is_null() || n == 0 {
        &[]
    } else {
        core::slice::from_raw_parts(p, n as usize)
    }
}

pub unsafe fn set_binary(message: *mut c_void, data: &[u8]) -> HRESULT {
    let p = if data.is_empty() { core::ptr::null() } else { data.as_ptr() };
    (vt::<MessageImplVtbl>(message).set_binary_body_copy)(message, data.len() as u32, p)
}

pub struct Timer {
    last: Instant,
    pub table: Vec<(&'static str, f64)>,
}

impl Default for Timer {
    fn default() -> Self {
        Timer::new()
    }
}

impl Timer {
    pub fn new() -> Self {
        Timer { last: Instant::now(), table: Vec::new() }
    }

    pub fn mark(&mut self, name: &'static str) {
        let now = Instant::now();
        let ms = now.duration_since(self.last).as_secs_f64() * 1000.0;
        self.last = now;
        match self.table.iter_mut().find(|e| e.0.eq_ignore_ascii_case(name)) {
            Some(e) => e.1 += ms,
            None => self.table.push((name, ms)),
        }
    }

    pub fn mark_plus(&mut self, name: &'static str, extra: f64) {
        self.mark(name);
        if let Some(e) = self.table.iter_mut().find(|e| e.0.eq_ignore_ascii_case(name)) {
            e.1 += extra;
        }
    }

    pub fn get(&self, name: &str) -> Option<f64> {
        self.table.iter().find(|e| e.0.eq_ignore_ascii_case(name)).map(|e| e.1)
    }

    pub fn names(&self) -> Vec<&'static str> {
        self.table.iter().map(|e| e.0).collect()
    }

    pub unsafe fn merge(&self, obj: *mut c_void) -> HRESULT {
        if obj.is_null() {
            return E_INVALIDARG;
        }
        let mut hr = S_OK;
        for (name, total) in &self.table {
            hr = set_number(obj, name, *total);
            if hr < 0 {
                break;
            }
        }
        hr
    }
}

pub trait Channel {
    unsafe fn send(&mut self, msg: *mut c_void) -> HRESULT;
    unsafe fn receive_more(&mut self, timeout: u32) -> Result<Com, HRESULT>;
}

impl Channel for Session {
    unsafe fn send(&mut self, msg: *mut c_void) -> HRESULT {
        self.send_more(msg)
    }
    unsafe fn receive_more(&mut self, timeout: u32) -> Result<Com, HRESULT> {
        Session::receive_more(self, timeout)
    }
}

pub trait CopyNotify {
    unsafe fn start(&mut self, root: &str, pattern: &str, name: &str, info: &crate::file::FileInfo, dest: &str) -> HRESULT;
    unsafe fn progress(&mut self, src: &str, dst: &str, size: u64, done: u64) -> HRESULT;
    unsafe fn end(&mut self, root: &str, pattern: &str, name: &str, info: &crate::file::FileInfo, dest: &str, hr: HRESULT) -> HRESULT;
}

static ERROR_TOKENS: Mutex<VecDeque<(u64, HRESULT)>> = Mutex::new(VecDeque::new());

pub fn add_error_token(token: u64, hr: HRESULT) {
    if token == 0 {
        return;
    }
    let mut l = ERROR_TOKENS.lock().unwrap_or_else(|e| e.into_inner());
    if l.len() > MAX_ERROR_TOKENS {
        l.pop_back();
    }
    l.push_front((token, hr));
}

pub fn take_error_token(token: u64) -> HRESULT {
    let mut l = ERROR_TOKENS.lock().unwrap_or_else(|e| e.into_inner());
    match l.iter().position(|e| e.0 == token) {
        Some(i) => l.remove(i).map_or(S_OK, |e| e.1),
        None => S_OK,
    }
}

pub fn error_token_count() -> usize {
    ERROR_TOKENS.lock().unwrap_or_else(|e| e.into_inner()).len()
}

pub unsafe extern "system" fn transfer_files_supported(_session: usize, _request: *mut c_void, response: *mut c_void) -> HRESULT {
    let mut body = Com::null();
    let mut hr = XbtpCreateJsonObject(body.out());
    if hr >= 0 {
        hr = set_u32(body.0, "ProtocolLevelSupported", DIRECT_PROTOCOL_CURRENT_LEVEL);
    }
    set_i32(body.0, "Result", hr);
    serialize_response_object(body.0, response)
}

pub unsafe extern "system" fn get_last_direct_protocol_error(_session: usize, request: *mut c_void, response: *mut c_void) -> HRESULT {
    let mut body = Com::null();
    let mut hr = deserialize_json_body(request, &mut body);
    if hr >= 0 {
        let mut token = 0u64;
        hr = get_u64(body.0, "LastErrorToken", &mut token);
        if hr >= 0 {
            hr = take_error_token(token);
        }
    }
    serialize(&MsgResponse { result: hr }, response)
}

unsafe fn to_console(session: usize, request: *mut c_void, response: *mut c_void, no_json: bool) -> HRESULT {
    let mut timer = Timer::new();
    let mut count = 0u32;
    let mut rb = Com::null();
    let mut hr = XbtpCreateJsonObject(rb.out());
    if hr >= 0 {
        let mut body = Com::null();
        hr = deserialize_json_body(request, &mut body);
        if hr >= 0 {
            let mut token = 0u64;
            get_u64(body.0, "LastErrorToken", &mut token);
            let mut flags = 0u32;
            get_low_flags(body.0, &mut flags);
            hr = xfer::messages_to_files(&mut Session(session as *mut c_void), request, flags, &mut timer, &mut count, no_json, None);
            if hr < 0 {
                add_error_token(token, hr);
            }
            timer.merge(rb.0);
            set_u32(rb.0, "FileCount", count);
        }
    }
    set_i32(rb.0, "Result", hr);
    serialize_response_object(rb.0, response)
}

pub unsafe extern "system" fn transfer_files_to_console(session: usize, request: *mut c_void, response: *mut c_void) -> HRESULT {
    to_console(session, request, response, false)
}

pub unsafe extern "system" fn transfer_files_to_console_no_json(session: usize, request: *mut c_void, response: *mut c_void) -> HRESULT {
    to_console(session, request, response, true)
}

pub struct FromConsoleRequest {
    pub search_pattern: Bstr,
    pub include: u32,
    pub exclude: u32,
    pub recursion: u32,
    pub flags: u32,
    pub dst: Bstr,
    pub dst_is_directory: bool,
}

pub unsafe fn read_from_console_request(body: *mut c_void) -> Result<FromConsoleRequest, HRESULT> {
    let mut r = FromConsoleRequest { search_pattern: Bstr::null(), include: 0, exclude: 0, recursion: 0, flags: 0, dst: Bstr::null(), dst_is_directory: false };
    let chk = |hr: HRESULT| if hr < 0 { Err(hr) } else { Ok(()) };
    chk(get_bstr(body, "SearchPattern", &mut r.search_pattern))?;
    chk(get_u32(body, "IncludeFileAttributes", &mut r.include))?;
    chk(get_u32(body, "ExcludeFileAttributes", &mut r.exclude))?;
    chk(get_u32(body, "RecursionLevels", &mut r.recursion))?;
    chk(get_u32(body, "Flags", &mut r.flags))?;
    chk(get_bstr(body, "DstFileName", &mut r.dst))?;
    let mut d = 0f64;
    chk(get_number(body, "IsDestinationDirectory", &mut d))?;
    r.dst_is_directory = d != 0.0;
    Ok(r)
}

unsafe fn from_console(session: usize, request: *mut c_void, response: *mut c_void, no_json: bool) -> HRESULT {
    let mut timer = Timer::new();
    let mut rb = Com::null();
    let hr = 'reply: {
        let hr = XbtpCreateJsonObject(rb.out());
        if hr < 0 {
            break 'reply hr;
        }
        let mut body = Com::null();
        let hr = deserialize_json_body(request, &mut body);
        if hr < 0 {
            break 'reply hr;
        }
        let req = match read_from_console_request(body.0) {
            Ok(r) => r,
            Err(e) => break 'reply e,
        };
        let pattern = bstr_units(&req.search_pattern);
        let (ty, rel) = match crate::paths::path_type(&pattern) {
            Ok(v) => v,
            Err(e) => break 'reply e,
        };
        if ty == crate::paths::PATH_APPLICATION {
            break 'reply HR_FILE_NOT_FOUND;
        }
        let search = if ty == crate::paths::PATH_CONSOLE { &pattern[rel..] } else { &pattern[..] };
        let search = String::from_utf16_lossy(search);
        timer.mark("ReadCpu");
        let found = xfer::find_for_transfer(&search, req.include, req.exclude, req.recursion);
        timer.mark("ReadFileScan");
        let dst = bstr_units(&req.dst);
        let mut count = 0u32;
        let out = xfer::Outgoing { flags: req.flags, no_json, token: 0, root: "", pattern: "" };
        let hr_xfer = xfer::files_to_messages(&mut Session(session as *mut c_void), &found.root, &found.files, &dst, req.dst_is_directory, &out, &mut timer, &mut count, None);
        if found.hr < 0 {
            break 'reply found.hr;
        }
        if hr_xfer < 0 {
            break 'reply hr_xfer;
        }
        let hr = timer.merge(rb.0);
        if hr < 0 {
            break 'reply hr;
        }
        set_u32(rb.0, "FileCount", count)
    };
    set_i32(rb.0, "Result", hr);
    serialize_response_object(rb.0, response)
}

pub unsafe extern "system" fn transfer_files_from_console(session: usize, request: *mut c_void, response: *mut c_void) -> HRESULT {
    from_console(session, request, response, false)
}

pub unsafe extern "system" fn transfer_files_from_console_no_json(session: usize, request: *mut c_void, response: *mut c_void) -> HRESULT {
    from_console(session, request, response, true)
}

pub unsafe extern "system" fn calculate_incremental_changes(session: usize, request: *mut c_void, response: *mut c_void) -> HRESULT {
    diff::calculate_incremental_changes(Session(session as *mut c_void), request, response)
}

pub unsafe extern "system" fn stream_operation_handler(session: usize, request: *mut c_void, response: *mut c_void) -> HRESULT {
    stream::stream_operation_handler(Session(session as *mut c_void), request, response)
}
