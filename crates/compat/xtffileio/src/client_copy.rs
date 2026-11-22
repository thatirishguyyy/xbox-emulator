use crate::client::*;
use crate::com::*;
use crate::file::{self, FileInfo, INVALID_FILE_ATTRIBUTES};
use crate::find::{append_path, find_files, relative_file_name, FindCallback, FoundFile};
use crate::msg::*;
use crate::paths::{full_path_name, path_type, PATH_APPLICATION, PATH_CONSOLE};
use crate::transfer::{optimal_chunks, FileReader, FileWriter, TransferRequest};
use core::ffi::c_void;
use core::ptr::null_mut;

pub const E_NOTIMPL: HRESULT = 0x8000_4001u32 as i32;
pub const E_UNEXPECTED: HRESULT = 0x8000_FFFFu32 as i32;
pub const E_PENDING: HRESULT = 0x8000_000Au32 as i32;
pub const HR_FILE_NOT_FOUND: HRESULT = 0x8007_0002u32 as i32;
pub const HR_PATH_NOT_FOUND: HRESULT = 0x8007_0003u32 as i32;
pub const HR_ACCESS_DENIED: HRESULT = 0x8007_0005u32 as i32;
pub const HR_SHARING_VIOLATION: HRESULT = 0x8007_0020u32 as i32;
pub const HR_IO_DEVICE: HRESULT = 0x8007_045Du32 as i32;
pub const HR_DIRECT_ACCESS_DENIED: HRESULT = 0x8d16_0181u32 as i32;
pub const HR_DIRECT_SHARING_VIOLATION: HRESULT = 0x8d16_0182u32 as i32;
pub const OP_SEND_FILE: u32 = 0xa;
pub const OP_RECV_FILE: u32 = 0xb;
pub const OP_TRANSFER_TO_CONSOLE: u32 = 0x1b;
pub const OP_TRANSFER_FROM_CONSOLE: u32 = 0x1c;
pub const OP_LAST_DIRECT_PROTOCOL_ERROR: u32 = 0x1d;
pub const OP_TRANSFER_TO_CONSOLE_NO_JSON: u32 = 0x1e;
pub const OP_TRANSFER_FROM_CONSOLE_NO_JSON: u32 = 0x1f;
pub const LEGACY_RECEIVE_TIMEOUT_MS: u32 = 120000;
pub const DIRECT_RECEIVE_TIMEOUT_MS: u32 = 30000;
pub const PERF_SIZE: usize = 0xc8;
pub const PERF_READ_PENDING: usize = 8;
pub const PERF_WRITE_PENDING: usize = 0x68;
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;

type P = *mut c_void;
type W = *const u16;

#[repr(C)]
pub struct CopyCallbackVtbl {
    pub base: UnknownVtbl,
    pub on_start_file_copy: unsafe extern "system" fn(P, W, W, *const XtfFileInfo, W) -> HRESULT,
    pub on_file_copy_progress: unsafe extern "system" fn(P, W, W, u64, u64) -> HRESULT,
    pub on_end_file_copy: unsafe extern "system" fn(P, W, W, *const XtfFileInfo, W, HRESULT) -> HRESULT,
}

unsafe fn units(p: W) -> Vec<u16> {
    if p.is_null() {
        return Vec::new();
    }
    let mut n = 0;
    while *p.add(n) != 0 {
        n += 1;
    }
    core::slice::from_raw_parts(p, n).to_vec()
}

fn xtf_info(name: &[u16], info: &FileInfo) -> XtfFileInfo {
    XtfFileInfo { file_name: name.as_ptr(), attributes: info.attributes, creation: info.creation, access: info.access, write: info.write, size: info.size, reserved: [0; 3] }
}

pub struct Notify {
    pub callback: P,
}

impl Notify {
    pub unsafe fn start(&mut self, root: &str, pattern: &str, name: &str, info: &FileInfo, dest: &str) -> HRESULT {
        if self.callback.is_null() {
            return S_OK;
        }
        let (r, p, n, d) = (wide(root), wide(pattern), wide(name), wide(dest));
        let x = xtf_info(&n, info);
        (vt::<CopyCallbackVtbl>(self.callback).on_start_file_copy)(self.callback, r.as_ptr(), p.as_ptr(), &x, d.as_ptr())
    }

    pub unsafe fn progress(&mut self, src: &str, dst: &str, size: u64, done: u64) -> HRESULT {
        if self.callback.is_null() {
            return S_OK;
        }
        let (s, d) = (wide(src), wide(dst));
        (vt::<CopyCallbackVtbl>(self.callback).on_file_copy_progress)(self.callback, s.as_ptr(), d.as_ptr(), size, done)
    }

    pub unsafe fn end(&mut self, root: &str, pattern: &str, name: &str, info: &FileInfo, dest: &str, hr: HRESULT) -> HRESULT {
        if self.callback.is_null() {
            return S_OK;
        }
        let (r, p, n, d) = (wide(root), wide(pattern), wide(name), wide(dest));
        let x = xtf_info(&n, info);
        (vt::<CopyCallbackVtbl>(self.callback).on_end_file_copy)(self.callback, r.as_ptr(), p.as_ptr(), &x, d.as_ptr(), hr)
    }
}

#[cfg(fileio_b)]
impl crate::fileio_b::CopyNotify for Notify {
    unsafe fn start(&mut self, root: &str, pattern: &str, name: &str, info: &FileInfo, dest: &str) -> HRESULT {
        Notify::start(self, root, pattern, name, info, dest)
    }
    unsafe fn progress(&mut self, src: &str, dst: &str, size: u64, done: u64) -> HRESULT {
        Notify::progress(self, src, dst, size, done)
    }
    unsafe fn end(&mut self, root: &str, pattern: &str, name: &str, info: &FileInfo, dest: &str, hr: HRESULT) -> HRESULT {
        Notify::end(self, root, pattern, name, info, dest, hr)
    }
}

pub struct Entry {
    pub name: String,
    pub info: FileInfo,
}

pub struct Found {
    pub root: String,
    pub search: String,
    pub entries: Vec<Entry>,
}

struct Collect<'a> {
    root: &'a mut Option<String>,
    search: &'a mut Option<String>,
    out: &'a mut Vec<Entry>,
}

impl FindCallback for Collect<'_> {
    fn on_found_file(&mut self, root: &str, pattern: Option<&str>, found: &FoundFile) -> HRESULT {
        if self.root.is_none() {
            *self.root = Some(root.to_string());
            *self.search = pattern.map(|p| p.to_string());
        }
        self.out.push(Entry { name: found.file_name.clone(), info: FileInfo { ..found.info } });
        S_OK
    }
}

fn local_find(pattern: &str, include: u32, exclude: u32, recursion: u32) -> Result<Found, HRESULT> {
    let mut root = None;
    let mut search = None;
    let mut entries = Vec::new();
    let hr = find_files(pattern, include, exclude, recursion, &mut Collect { root: &mut root, search: &mut search, out: &mut entries });
    if hr < 0 {
        return Err(hr);
    }
    Ok(Found { root: root.unwrap_or_default(), search: search.unwrap_or_default(), entries })
}

unsafe fn remote_find(c: &FileIOClient, pattern: &str, include: u32, exclude: u32, recursion: u32) -> Result<Found, HRESULT> {
    let w = wide(pattern);
    let mut list = FileList::default();
    let mut fr = FindResult { root: Bstr::null(), pattern: Bstr::null() };
    let hr = c.list_files(w.as_ptr(), include, exclude, recursion, 0, &mut list, Some(&mut fr));
    if hr < 0 {
        return Err(hr);
    }
    let entries = list
        .entries
        .iter()
        .map(|e| Entry {
            name: String::from_utf16_lossy(bstr_slice(&e.name)),
            info: FileInfo { attributes: e.info.attributes, creation: e.info.creation, access: e.info.access, write: e.info.write, size: e.info.size },
        })
        .collect();
    Ok(Found { root: fr.root.to_string(), search: fr.pattern.to_string(), entries })
}

unsafe fn attributes_of(c: &FileIOClient, remote: bool, path: &str) -> (HRESULT, u32) {
    let (hr, a) = if remote {
        let w = wide(path);
        let mut a = INVALID_FILE_ATTRIBUTES;
        let hr = c.remote_file().get_file_attributes(w.as_ptr(), &mut a);
        (hr, if hr < 0 { INVALID_FILE_ATTRIBUTES } else { a })
    } else {
        match full_path_name(path).and_then(|f| file::get_file_attributes(&f)) {
            Ok(a) => (S_OK, a),
            Err(e) => (e, INVALID_FILE_ATTRIBUTES),
        }
    };
    if hr == HR_FILE_NOT_FOUND || hr == HR_PATH_NOT_FOUND {
        return (S_OK, INVALID_FILE_ATTRIBUTES);
    }
    (hr, a)
}

struct Side {
    kind: u32,
    remote: bool,
    path: String,
}

fn side_of(p: &[u16]) -> Result<Side, HRESULT> {
    let (kind, rel) = path_type(p)?;
    let path = if kind == PATH_CONSOLE { String::from_utf16_lossy(&p[rel..]) } else { String::from_utf16_lossy(p) };
    Ok(Side { kind, remote: kind != 0, path })
}

fn has_wildcard(s: &str) -> bool {
    s.contains('*') || s.contains('?')
}

unsafe fn message() -> Result<Com, HRESULT> {
    let mut m = Com::null();
    let hr = XbtpCreateMessage(&IID_XBTP_MESSAGE_IMPL, m.out());
    if hr < 0 {
        Err(hr)
    } else {
        Ok(m)
    }
}

unsafe fn set_props_json(props: *mut c_void, msg: *mut c_void) -> HRESULT {
    let mut w = Bstr::null();
    let hr = (vt::<JsonObjectVtbl>(props).stringify)(props, w.out());
    if hr < 0 {
        return hr;
    }
    (vt::<MessageImplVtbl>(msg).set_json_body_w)(msg, w.0)
}

unsafe fn csv(s: P) -> &'static ClientSessionVtbl {
    vt::<ClientSessionVtbl>(s)
}

unsafe fn send_cancel(session: P, exchange: i32, offset: u64) {
    let m = match message() {
        Ok(m) => m,
        Err(_) => return,
    };
    let mut props = Com::null();
    if XbtpCreateJsonObject(props.out()) < 0 {
        return;
    }
    set_u64(props.0, "FileOffset", offset);
    set_u32(props.0, "CancelTransfer", 1);
    if set_props_json(props.0, m.0) < 0 {
        return;
    }
    (csv(session).send_more_message)(session, exchange, m.0);
}

pub unsafe fn legacy_send_file(session: P, src: &str, dst: &str, flags: u32, max_size: u64, info: &FileInfo, notify: &mut Notify) -> HRESULT {
    if session.is_null() {
        return E_POINTER;
    }
    let su: Vec<u16> = src.encode_utf16().collect();
    match path_type(&su) {
        Ok((PATH_APPLICATION, _)) => return HR_FILE_NOT_FOUND,
        Ok(_) => {}
        Err(e) => return e,
    }
    let req = TransferRequest {
        src: match Bstr::from_str(src) {
            Ok(b) => b,
            Err(e) => return e,
        },
        dst: match Bstr::from_str(dst) {
            Ok(b) => b,
            Err(e) => return e,
        },
        flags,
        info: FileInfo { ..*info },
    };
    let mut r = FileReader::new(req);
    let mut hr = r.start();
    let mut exchange = 0i32;
    let mut begun = false;
    if hr >= 0 {
        r.chunk = optimal_chunks(max_size).1;
        hr = notify.progress(src, dst, r.req.info.size, 0);
    }
    if hr >= 0 {
        let size = r.req.info.size;
        let mut buf = vec![0u8; r.chunk as usize];
        let mut first = true;
        hr = loop {
            let n = (size - r.offset).min(r.chunk as u64) as u32;
            let at = r.offset;
            let done = match r.stream.read_at(&mut buf[..n as usize], at) {
                Ok(d) => d,
                Err(e) => break e,
            };
            if done != n {
                break HR_IO_DEVICE;
            }
            let m = match message() {
                Ok(m) => m,
                Err(e) => break e,
            };
            let h = (vt::<MessageImplVtbl>(m.0).set_binary_body_copy)(m.0, n, if n == 0 { core::ptr::null() } else { buf.as_ptr() });
            if h < 0 {
                break h;
            }
            let h = r.chunk_json(first, at, m.0);
            if h < 0 {
                break h;
            }
            let h = if first { (csv(session).send_begin_message)(session, OP_SEND_FILE, m.0, &mut exchange) } else { (csv(session).send_more_message)(session, exchange, m.0) };
            if h < 0 {
                break h;
            }
            begun = true;
            first = false;
            r.offset += n as u64;
            if r.offset > size {
                break E_UNEXPECTED;
            }
            let h = notify.progress(src, dst, size, r.offset);
            if h < 0 {
                if r.offset < size {
                    send_cancel(session, exchange, r.offset);
                }
                break h;
            }
            if r.offset >= size {
                break S_OK;
            }
        };
    }
    if begun {
        let mut resp = MsgResponse::default();
        let e = receive_end_message(session, XTF_RECEIVE_RECOMMENDED_TIMEOUT_MS, exchange, &mut resp);
        if hr >= 0 {
            hr = if e < 0 { e } else { resp.result };
        }
    }
    if r.stream.is_open() {
        let c = r.stream.close_with_retry(false);
        if hr >= 0 && c < 0 {
            hr = c;
        }
    }
    hr
}

pub unsafe fn legacy_receive_file(session: P, src: &str, dst: &str, flags: u32, notify: &mut Notify) -> HRESULT {
    if session.is_null() {
        return E_POINTER;
    }
    let du: Vec<u16> = dst.encode_utf16().collect();
    match path_type(&du) {
        Ok((PATH_APPLICATION, _)) => return HR_FILE_NOT_FOUND,
        Ok(_) => {}
        Err(e) => return e,
    }
    let req = TransferRequest {
        src: match Bstr::from_str(src) {
            Ok(b) => b,
            Err(e) => return e,
        },
        dst: match Bstr::from_str(dst) {
            Ok(b) => b,
            Err(e) => return e,
        },
        flags,
        info: FileInfo::default(),
    };
    let mut w = FileWriter::new(req);
    let mut exchange = 0i32;
    let mut hr = w.start();
    let mut begun = false;
    if hr >= 0 {
        hr = 'send: {
            let m = match message() {
                Ok(m) => m,
                Err(e) => break 'send e,
            };
            let mut props = Com::null();
            let h = XbtpCreateJsonObject(props.out());
            if h < 0 {
                break 'send h;
            }
            let h = set_string(props.0, "SrcFileName", &w.req.src);
            if h < 0 {
                break 'send h;
            }
            let h = set_string(props.0, "DstFileName", &w.req.dst);
            if h < 0 {
                break 'send h;
            }
            let h = set_u32(props.0, "Flags", flags);
            if h < 0 {
                break 'send h;
            }
            let h = set_props_json(props.0, m.0);
            if h < 0 {
                break 'send h;
            }
            let h = (csv(session).send_begin_message)(session, OP_RECV_FILE, m.0, &mut exchange);
            if h >= 0 {
                begun = true;
            }
            h
        };
    }
    if hr >= 0 {
        hr = notify.progress(src, dst, 0, 0);
    }
    let mut ended = false;
    if hr >= 0 {
        let mut first = true;
        hr = loop {
            let mut ty = 0i16;
            let mut status = 0i32;
            let mut m = Com::null();
            let h = (csv(session).receive_message)(session, LEGACY_RECEIVE_TIMEOUT_MS, exchange, &mut ty, &mut status, m.out());
            if h == E_PENDING {
                continue;
            }
            if h < 0 {
                break h;
            }
            if status != 0 {
                break transport_status(status);
            }
            if ty == XBTP_MESSAGE_TYPE_END {
                ended = true;
                let mut resp = MsgResponse::default();
                let d = deserialize(&mut resp, m.0);
                if d < 0 {
                    break d;
                }
                if resp.result < 0 {
                    break resp.result;
                }
                if first || w.offset < w.req.info.size {
                    break E_UNEXPECTED;
                }
                break S_OK;
            }
            if ty != XBTP_MESSAGE_TYPE_MORE {
                break (UNEXPECTED_MESSAGE_BASE | (ty as u16 as u32 & 0xff)) as i32;
            }
            if first {
                let mut props = Com::null();
                let d = deserialize_json_body(m.0, &mut props);
                if d < 0 {
                    break d;
                }
                let d = crate::find::get_file_info_properties(props.0, &mut w.req.info);
                if d < 0 {
                    break d;
                }
            }
            first = false;
            let (off, n) = match w.begin_write(m.0) {
                Ok(v) => v,
                Err(e) => break e,
            };
            if n == u32::MAX {
                break HR_IO_DEVICE;
            }
            let size = w.req.info.size;
            if off + n as u64 > size {
                break E_UNEXPECTED;
            }
            let p = notify.progress(src, dst, size, off + n as u64);
            if p < 0 {
                break p;
            }
            if w.offset >= size {
                break S_OK;
            }
        };
    }
    if begun && !ended {
        let mut resp = MsgResponse::default();
        let e = receive_end_message(session, LEGACY_RECEIVE_TIMEOUT_MS, exchange, &mut resp);
        if hr >= 0 {
            hr = if e < 0 { e } else { resp.result };
        }
    }
    let f = w.finish(hr);
    if hr >= 0 && f < 0 {
        f
    } else {
        hr
    }
}

#[allow(clippy::too_many_arguments)]
unsafe fn copy_legacy(c: &FileIOClient, found: &Found, dst: &str, dst_remote: bool, dst_directory: bool, flags: u32, notify: &mut Notify) -> HRESULT {
    let max_size = found.entries.iter().map(|e| e.info.size).max().unwrap_or(0);
    let session = c.session_a();
    for e in &found.entries {
        if e.info.attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
            continue;
        }
        let dest = if dst_directory {
            match relative_file_name(&found.root, &e.name) {
                Ok(rel) => append_path(dst, Some(&rel)),
                Err(err) => return err,
            }
        } else {
            dst.to_string()
        };
        let hr = notify.start(&found.root, &found.search, &e.name, &e.info, &dest);
        if hr < 0 {
            return hr;
        }
        let hr = if dst_remote { legacy_send_file(session.0, &e.name, &dest, flags, max_size, &e.info, notify) } else { legacy_receive_file(session.0, &e.name, &dest, flags, notify) };
        if hr < 0 {
            return hr;
        }
        let hr = notify.end(&found.root, &found.search, &e.name, &e.info, &dest, hr);
        if hr < 0 {
            return hr;
        }
    }
    S_OK
}

#[cfg(fileio_b)]
mod direct {
    use super::*;
    use crate::fileio_b::xfer::{files_to_messages, messages_to_files, Outgoing};
    use crate::fileio_b::{set_number, Channel, Timer};

    pub struct ClientChannel {
        pub session: P,
        pub op: u32,
        pub exchange: i32,
        pub begun: bool,
    }

    impl Channel for ClientChannel {
        unsafe fn send(&mut self, msg: *mut c_void) -> HRESULT {
            if self.session.is_null() {
                return crate::fileio_b::HR_NO_SESSION;
            }
            if !self.begun {
                let hr = (csv(self.session).send_begin_message)(self.session, self.op, msg, &mut self.exchange);
                if hr >= 0 {
                    self.begun = true;
                }
                return hr;
            }
            (csv(self.session).send_more_message)(self.session, self.exchange, msg)
        }
        unsafe fn receive_more(&mut self, timeout: u32) -> Result<Com, HRESULT> {
            if self.session.is_null() {
                return Err(crate::fileio_b::HR_NO_SESSION);
            }
            let mut m = Com::null();
            let hr = (csv(self.session).receive_more_message)(self.session, timeout, self.exchange, m.out());
            if hr < 0 {
                Err(hr)
            } else {
                Ok(m)
            }
        }
    }

    unsafe fn receive_end(ch: &ClientChannel, props: &mut Com) -> HRESULT {
        let mut status = 0i32;
        let mut m = Com::null();
        let hr = (csv(ch.session).receive_end_message)(ch.session, DIRECT_RECEIVE_TIMEOUT_MS, ch.exchange, &mut status, m.out());
        if hr < 0 {
            return hr;
        }
        if status != 0 {
            return transport_status(status);
        }
        let hr = deserialize_json_body(m.0, props);
        if hr < 0 {
            return hr;
        }
        let mut result = 0i32;
        let hr = get_i32(props.0, "Result", &mut result);
        if hr < 0 {
            return hr;
        }
        result
    }

    const PERF_SLOTS: [(&str, usize); 12] = [
        ("ReadCpu", 0x18),
        ("WriteCpu", 0x78),
        ("ReadFileScan", 0x10),
        ("WriteFileScan", 0x70),
        ("ReadNetwork", 0x88),
        ("WriteNetwork", 0x28),
        ("ReadFileMeta", 0x38),
        ("WriteFileMeta", 0x98),
        ("ReadFile", 0x48),
        ("WriteFile", 0xa8),
        ("ReadDiskWait", 0x58),
        ("WriteDiskWait", 0xb8),
    ];

    unsafe fn extract_json_body(timer: &mut Timer, props: *mut c_void) {
        if props.is_null() {
            return;
        }
        for (name, _) in PERF_SLOTS {
            let mut v = 0f64;
            if crate::fileio_b::get_number(props, name, &mut v) >= 0 {
                timer.mark_plus(name, v);
            }
        }
    }

    unsafe fn put(perf: *mut u8, off: usize, v: f64) {
        core::ptr::write_unaligned(perf.add(off) as *mut f64, v);
    }

    unsafe fn get(perf: *mut u8, off: usize) -> f64 {
        core::ptr::read_unaligned(perf.add(off) as *const f64)
    }

    unsafe fn deserialize_perf(timer: &Timer, perf: *mut u8) {
        for (name, off) in PERF_SLOTS {
            if let Some(v) = timer.get(name) {
                put(perf, off, v);
            }
        }
        for base in [0x18usize, 0x78] {
            let total: f64 = (0..5).map(|i| get(perf, base + i * 0x10)).sum::<f64>() / 100.0;
            if total > 0.0 {
                for i in 0..5 {
                    let v = get(perf, base + i * 0x10) / total;
                    put(perf, base + 8 + i * 0x10, v);
                }
            }
        }
    }

    pub unsafe fn reset_perf(perf: P) {
        if perf.is_null() {
            return;
        }
        let p = perf as *mut u8;
        core::ptr::write_bytes(p, 0, PERF_SIZE);
        *p.add(PERF_READ_PENDING) = 1;
        *p.add(PERF_WRITE_PENDING) = 1;
    }

    unsafe fn last_error_token(c: &FileIOClient, token: u64) -> HRESULT {
        let (addr, port, redirect) = c.connection();
        let mut s = Com::null();
        let hr = open_vm_session(addr.0, port, &mut s, redirect);
        if hr < 0 {
            return hr;
        }
        let m = match message() {
            Ok(m) => m,
            Err(e) => return e,
        };
        let mut props = Com::null();
        let hr = XbtpCreateJsonObject(props.out());
        if hr < 0 {
            return hr;
        }
        let hr = set_u64(props.0, "LastErrorToken", token);
        if hr < 0 {
            return hr;
        }
        let hr = set_props_json(props.0, m.0);
        if hr < 0 {
            return hr;
        }
        let mut exchange = 0i32;
        let hr = (csv(s.0).send_begin_message)(s.0, OP_LAST_DIRECT_PROTOCOL_ERROR, m.0, &mut exchange);
        if hr < 0 {
            return hr;
        }
        let mut resp = MsgResponse::default();
        let hr = receive_end_message(s.0, DIRECT_RECEIVE_TIMEOUT_MS, exchange, &mut resp);
        if hr < 0 {
            return hr;
        }
        resp.result
    }

    #[allow(clippy::too_many_arguments)]
    pub unsafe fn copy_direct(c: &FileIOClient, level: u32, found: &Found, pattern: &str, include: u32, exclude: u32, recursion: u32, dst: &str, dst_remote: bool, dst_directory: bool, flags: u32, perf: P, notify: &mut Notify) -> HRESULT {
        reset_perf(perf);
        let session = c.session_a();
        let mut timer = Timer::new();
        let mut count = 0u32;
        let no_json = level == 2;
        let mut remote = Com::null();
        if dst_remote {
            let mut ch = ClientChannel { session: session.0, op: if no_json { OP_TRANSFER_TO_CONSOLE_NO_JSON } else { OP_TRANSFER_TO_CONSOLE }, exchange: 0, begun: false };
            let token = &ch as *const ClientChannel as u64;
            let files: Vec<FoundFile> = found.entries.iter().map(|e| FoundFile { file_name: e.name.clone(), info: FileInfo { ..e.info } }).collect();
            let out = Outgoing { flags, no_json, token, root: &found.root, pattern: &found.search };
            let dst_u: Vec<u16> = dst.encode_utf16().collect();
            let mut hr = files_to_messages(&mut ch, &found.root, &files, &dst_u, dst_directory, &out, &mut timer, &mut count, Some(notify));
            if ch.begun {
                let r = receive_end(&ch, &mut remote);
                if hr >= 0 {
                    hr = r;
                }
            }
            if hr < 0 {
                let q = last_error_token(c, token);
                return if q < 0 {
                    q
                } else if hr == HR_ACCESS_DENIED {
                    HR_DIRECT_ACCESS_DENIED
                } else if hr == HR_SHARING_VIOLATION {
                    HR_DIRECT_SHARING_VIOLATION
                } else {
                    hr
                };
            }
            if !perf.is_null() {
                *(perf as *mut u8).add(PERF_WRITE_PENDING) = 0;
                extract_json_body(&mut timer, remote.0);
                deserialize_perf(&timer, perf as *mut u8);
            }
            return hr;
        }
        let mut ch = ClientChannel { session: session.0, op: if no_json { OP_TRANSFER_FROM_CONSOLE_NO_JSON } else { OP_TRANSFER_FROM_CONSOLE }, exchange: 0, begun: false };
        let hr = 'req: {
            let m = match message() {
                Ok(m) => m,
                Err(e) => break 'req e,
            };
            let mut props = Com::null();
            let h = XbtpCreateJsonObject(props.out());
            if h < 0 {
                break 'req h;
            }
            let steps: [&dyn Fn() -> HRESULT; 7] = [
                &|| set_string(props.0, "SearchPattern", &Bstr::from_str(pattern).unwrap_or_else(|_| Bstr::null())),
                &|| set_u32(props.0, "IncludeFileAttributes", include),
                &|| set_u32(props.0, "ExcludeFileAttributes", exclude),
                &|| set_u32(props.0, "RecursionLevels", recursion),
                &|| set_u32(props.0, "Flags", flags),
                &|| set_string(props.0, "DstFileName", &Bstr::from_str(dst).unwrap_or_else(|_| Bstr::null())),
                &|| set_number(props.0, "IsDestinationDirectory", if dst_directory { 1.0 } else { 0.0 }),
            ];
            for s in steps {
                let h = s();
                if h < 0 {
                    break 'req h;
                }
            }
            let h = set_props_json(props.0, m.0);
            if h < 0 {
                break 'req h;
            }
            ch.send(m.0)
        };
        if hr < 0 {
            return hr;
        }
        let mut hr = messages_to_files(&mut ch, null_mut(), flags, &mut timer, &mut count, no_json, Some(notify));
        let r = receive_end(&ch, &mut remote);
        if r < 0 && (hr >= 0 || !remote.0.is_null()) {
            hr = r;
        }
        if hr >= 0 && !perf.is_null() {
            *(perf as *mut u8).add(PERF_READ_PENDING) = 0;
            extract_json_body(&mut timer, remote.0);
            deserialize_perf(&timer, perf as *mut u8);
        }
        hr
    }
}

#[allow(clippy::too_many_arguments)]
pub unsafe fn copy_files_internal(c: &FileIOClient, pattern: W, include: u32, exclude: u32, recursion: u32, dst: W, flags: u32, perf: P, callback: P) -> HRESULT {
    if pattern.is_null() || dst.is_null() {
        return E_INVALIDARG;
    }
    let pu = units(pattern);
    let du = units(dst);
    let src = match side_of(&pu) {
        Ok(s) => s,
        Err(e) => return e,
    };
    let dsts = match side_of(&du) {
        Ok(s) => s,
        Err(e) => return e,
    };
    if src.remote == dsts.remote {
        return E_INVALIDARG;
    }
    let app: Option<W> = if src.kind == PATH_APPLICATION {
        Some(pattern)
    } else if dsts.kind == PATH_APPLICATION {
        Some(dst)
    } else {
        None
    };
    if let Some(a) = app {
        let hr = c.start_application_operations(a);
        if hr < 0 {
            return hr;
        }
    }
    let hr = copy_files_body(c, &src, &dsts, include, exclude, recursion, flags, perf, callback);
    if let Some(a) = app {
        c.end_application_operations(a);
    }
    hr
}

#[allow(clippy::too_many_arguments)]
unsafe fn copy_files_body(c: &FileIOClient, src: &Side, dsts: &Side, include: u32, exclude: u32, recursion: u32, flags: u32, perf: P, callback: P) -> HRESULT {
    let level = direct_level(c);
    let skip_find = src.remote && level != 0;
    let found = if skip_find {
        Found { root: String::new(), search: String::new(), entries: Vec::new() }
    } else {
        let r = if src.remote { remote_find(c, &src.path, include & !FILE_ATTRIBUTE_DIRECTORY, exclude | FILE_ATTRIBUTE_DIRECTORY, recursion) } else { local_find(&src.path, include & !FILE_ATTRIBUTE_DIRECTORY, exclude | FILE_ATTRIBUTE_DIRECTORY, recursion) };
        match r {
            Ok(f) => f,
            Err(e) => return e,
        }
    };
    if !skip_find && found.entries.is_empty() {
        return HR_FILE_NOT_FOUND;
    }
    let count = found.entries.len();
    let dst_directory = if dsts.path.ends_with('\\') {
        true
    } else {
        let (hr, attrs) = attributes_of(c, dsts.remote, &dsts.path);
        if hr < 0 {
            return hr;
        }
        let multi = if src.remote { has_wildcard(&src.path) } else { count != 1 };
        if multi {
            attrs == INVALID_FILE_ATTRIBUTES || attrs & FILE_ATTRIBUTE_DIRECTORY != 0
        } else if attrs != INVALID_FILE_ATTRIBUTES {
            attrs & FILE_ATTRIBUTE_DIRECTORY != 0
        } else {
            let (shr, sattrs) = attributes_of(c, src.remote, &src.path);
            if shr < 0 {
                return shr;
            }
            sattrs != INVALID_FILE_ATTRIBUTES && sattrs & FILE_ATTRIBUTE_DIRECTORY != 0
        }
    };
    if !dst_directory && count > 1 {
        return E_INVALIDARG;
    }
    let mut notify = Notify { callback };
    if level != 0 {
        #[cfg(fileio_b)]
        {
            let pat = if found.search.is_empty() { src.path.clone() } else { found.search.clone() };
            return direct::copy_direct(c, level, &found, &pat, include, exclude, recursion, &dsts.path, dsts.remote, dst_directory, flags, perf, &mut notify);
        }
    }
    let _ = perf;
    copy_legacy(c, &found, &dsts.path, dsts.remote, dst_directory, flags, &mut notify)
}

unsafe fn direct_level(c: &FileIOClient) -> u32 {
    #[cfg(fileio_b)]
    {
        let l = c.direct_protocol_level();
        if l >= 2 {
            return 2;
        }
        l
    }
    #[cfg(not(fileio_b))]
    {
        let _ = c;
        0
    }
}

pub unsafe extern "system" fn c_copy_files(this: P, pattern: W, include: u32, exclude: u32, recursion: u32, dst: W, flags: u32, callback: P) -> HRESULT {
    copy_files_internal(client_from_main(this), pattern, include, exclude, recursion, dst, flags, null_mut(), callback)
}

pub unsafe extern "system" fn i2_copy_files(this: P, root: W, files: *const W, count: u32, dst: W, flags: u32, callback: P) -> HRESULT {
    let c = client_from_i2(this);
    if root.is_null() || dst.is_null() || (files.is_null() && count != 0) {
        return E_INVALIDARG;
    }
    let ru = units(root);
    let du = units(dst);
    let rk = match path_type(&ru) {
        Ok((k, _)) => k,
        Err(e) => return e,
    };
    let dk = match path_type(&du) {
        Ok((k, _)) => k,
        Err(e) => return e,
    };
    let app: Option<W> = if rk == PATH_APPLICATION {
        Some(root)
    } else if dk == PATH_APPLICATION {
        Some(dst)
    } else {
        None
    };
    if let Some(a) = app {
        let hr = c.start_application_operations(a);
        if hr < 0 {
            return hr;
        }
    }
    let rs = String::from_utf16_lossy(&ru);
    let mut hr = S_OK;
    for i in 0..count as usize {
        let f = *files.add(i);
        if f.is_null() {
            hr = E_INVALIDARG;
            break;
        }
        let part = String::from_utf16_lossy(&units(f));
        let pattern = wide(&append_path(&rs, Some(&part)));
        hr = (CLIENT_VTBL.copy_files)(FileIOClient::as_unknown(c as *const FileIOClient as *mut FileIOClient), pattern.as_ptr(), 0, 0, 0, dst, flags, callback);
        if hr < 0 {
            break;
        }
    }
    if let Some(a) = app {
        c.end_application_operations(a);
    }
    hr
}

pub unsafe extern "system" fn i3_copy_files_direct_list(_this: P, list: P, reserved: u32, _pattern: W, _dst: W, _flags: u32, _perf: P, _callback: P) -> HRESULT {
    if list.is_null() || reserved != 0 {
        return E_INVALIDARG;
    }
    E_NOTIMPL
}

pub unsafe extern "system" fn i3_copy_files_perf(this: P, pattern: W, include: u32, exclude: u32, recursion: u32, dst: W, flags: u32, perf: P, callback: P) -> HRESULT {
    copy_files_internal(client_from_i3(this), pattern, include, exclude, recursion, dst, flags, perf, callback)
}

pub const I3_CALCULATE_INCREMENTAL_LIST: usize = 0;
