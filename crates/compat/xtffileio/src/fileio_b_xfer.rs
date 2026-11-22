use super::*;
use crate::file::{self, FileInfo, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_READONLY, INVALID_FILE_ATTRIBUTES};
use crate::find::{append_path, find_files, FindCallback, FoundFile};
use crate::paths::{full_path_name, host_path, path_type, PATH_APPLICATION, PATH_CONSOLE};
use std::alloc::{alloc_zeroed, dealloc, Layout};

pub const DIRECT_PROTOCOL_BLOCK_SIZE: u64 = 0x80000;
pub const LEADER_SIZE: usize = 0x30;
pub const MIN_LEADER_BODY: u32 = 0x38;
pub const XTFCOPYFILE_SHAREWRITE: u32 = 0x4000_0000;
pub const XTFCOPYFILE_DEFER_LOCKED: u32 = 0x8000_0000;
const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const FILE_SHARE_READ: u32 = 1;
const FILE_SHARE_WRITE: u32 = 2;
const CREATE_NEW: u32 = 1;
const CREATE_ALWAYS: u32 = 2;
const OPEN_EXISTING: u32 = 3;
const FILE_FLAG_NO_BUFFERING: u32 = 0x2000_0000;
const INVALID_HANDLE: isize = -1;
const ERROR_ACCESS_DENIED: u32 = 5;
const ERROR_SHARING_VIOLATION: u32 = 0x20;
const ERROR_FILE_EXISTS: u32 = 80;
const ERROR_HANDLE_EOF: u32 = 38;
const OPEN_ATTEMPTS: u32 = 5;
const BUFFER_ALIGN: usize = 4096;

#[repr(C)]
#[derive(Default)]
struct Overlapped {
    internal: usize,
    internal_high: usize,
    offset: u32,
    offset_high: u32,
    event: isize,
}

#[allow(clashing_extern_declarations)]
#[link(name = "kernel32")]
extern "system" {
    fn CreateFileW(name: *const u16, access: u32, share: u32, sa: *mut c_void, disp: u32, flags: u32, tmpl: isize) -> isize;
    fn ReadFile(h: isize, buf: *mut u8, n: u32, read: *mut u32, ov: *mut Overlapped) -> i32;
    fn WriteFile(h: isize, buf: *const u8, n: u32, written: *mut u32, ov: *mut Overlapped) -> i32;
    fn GetFileSizeEx(h: isize, size: *mut i64) -> i32;
    fn CloseHandle(h: isize) -> i32;
    fn SetFileTime(h: isize, c: *const u64, a: *const u64, m: *const u64) -> i32;
    fn SetFilePointerEx(h: isize, dist: i64, newpos: *mut i64, method: u32) -> i32;
    fn SetEndOfFile(h: isize) -> i32;
    fn DeleteFileW(name: *const u16) -> i32;
    fn Sleep(ms: u32);
    fn GetLastError() -> u32;
    fn GetTickCount64() -> u64;
}

fn last_hr() -> HRESULT {
    hr_from_win32(unsafe { GetLastError() })
}

fn last_hr_or_fail() -> HRESULT {
    hr_from_last_error_or_fail(unsafe { GetLastError() })
}

pub fn host_wide(console: &str) -> Result<Vec<u16>, HRESULT> {
    host_path(console).map(|h| h.wide).map_err(hr_from_win32)
}

pub fn open_with_retry(console: &str, access: u32, share: u32, disp: u32, flags: u32) -> Result<isize, HRESULT> {
    let w = host_wide(console)?;
    let mut attempt = 0;
    loop {
        attempt += 1;
        let h = unsafe { CreateFileW(w.as_ptr(), access, share, null_mut(), disp, flags, 0) };
        if h != INVALID_HANDLE {
            return Ok(h);
        }
        let e = unsafe { GetLastError() };
        let hr = hr_from_last_error_or_fail(e);
        if attempt >= OPEN_ATTEMPTS {
            return Err(hr);
        }
        if e == ERROR_ACCESS_DENIED || e == ERROR_SHARING_VIOLATION {
            unsafe { Sleep(100) };
            continue;
        }
        if (hr as u32) >> 16 == 0x8007 {
            return Err(hr);
        }
    }
}

pub struct TransferList {
    pub hr: HRESULT,
    pub root: String,
    pub files: Vec<FoundFile>,
}

struct Collect<'a> {
    root: &'a mut Option<String>,
    files: &'a mut Vec<FoundFile>,
}

impl FindCallback for Collect<'_> {
    fn on_found_file(&mut self, root: &str, _pattern: Option<&str>, found: &FoundFile) -> HRESULT {
        if self.root.is_none() {
            *self.root = Some(root.to_string());
        }
        self.files.push(FoundFile { file_name: found.file_name.clone(), info: FileInfo { ..found.info } });
        S_OK
    }
}

pub fn find_for_transfer(search: &str, include: u32, exclude: u32, recursion: u32) -> TransferList {
    let mut root = None;
    let mut files = Vec::new();
    let hr = find_files(search, include, exclude, recursion, &mut Collect { root: &mut root, files: &mut files });
    TransferList { hr, root: root.unwrap_or_default(), files }
}

pub use crate::find::relative_file_name;

pub fn leader_bytes(block: u64, size: u64, info: &FileInfo, name: &[u16]) -> Vec<u8> {
    let mut v = Vec::with_capacity(LEADER_SIZE + name.len() * 2 + 2);
    v.extend_from_slice(&block.to_le_bytes());
    v.extend_from_slice(&size.to_le_bytes());
    v.extend_from_slice(&info.attributes.to_le_bytes());
    v.extend_from_slice(&0u32.to_le_bytes());
    v.extend_from_slice(&info.creation.to_le_bytes());
    v.extend_from_slice(&info.access.to_le_bytes());
    v.extend_from_slice(&info.write.to_le_bytes());
    for u in name {
        v.extend_from_slice(&u.to_le_bytes());
    }
    v.extend_from_slice(&[0, 0]);
    v
}

unsafe fn json_head(msg: *mut c_void, dest: &[u16], size: u64, info: &FileInfo, flags: u32, token: u64) -> HRESULT {
    let body = match new_object() {
        Ok(b) => b,
        Err(e) => return e,
    };
    let o = body.0;
    let steps: [&dyn Fn() -> HRESULT; 9] = [
        &|| set_wstr(o, "FileName", dest),
        &|| set_u64(o, "FileBlockSize", DIRECT_PROTOCOL_BLOCK_SIZE),
        &|| set_u64(o, "FileSize", size),
        &|| set_u32(o, "FileAttributes", info.attributes),
        &|| set_u64(o, "CreationTime", info.creation),
        &|| set_u64(o, "LastAccessTime", info.access),
        &|| set_u64(o, "LastWriteTime", info.write),
        &|| set_u64(o, "LastErrorToken", token),
        &|| set_u64(o, "Flags", flags as u64),
    ];
    for s in steps {
        let hr = s();
        if hr < 0 {
            return hr;
        }
    }
    serialize_object(o, msg)
}

unsafe fn json_no_json_preamble(msg: *mut c_void, flags: u32, token: u64) -> HRESULT {
    let body = match new_object() {
        Ok(b) => b,
        Err(e) => return e,
    };
    let hr = set_u64(body.0, "LastErrorToken", token);
    if hr < 0 {
        return hr;
    }
    let hr = set_u64(body.0, "Flags", flags as u64);
    if hr < 0 {
        return hr;
    }
    serialize_object(body.0, msg)
}

unsafe fn send_terminator(session: &mut dyn Channel, timer: &mut Timer) {
    timer.mark("ReadCpu");
    if let Ok(m) = new_message() {
        if let Ok(b) = new_object() {
            set_wstr(b.0, "FileName", &[]);
            serialize_object(b.0, m.0);
        }
        session.send(m.0);
    }
    timer.mark("WriteNetwork");
    timer.mark("ReadCpu");
}

pub struct Outgoing<'a> {
    pub flags: u32,
    pub no_json: bool,
    pub token: u64,
    pub root: &'a str,
    pub pattern: &'a str,
}

#[allow(clippy::too_many_arguments)]
unsafe fn send_one_file(session: &mut dyn Channel, f: &FoundFile, dest: &[u16], out: &Outgoing, timer: &mut Timer, first_global: &mut bool, buf: &mut [u8], notify: &mut Option<&mut dyn CopyNotify>) -> HRESULT {
    timer.mark("ReadCpu");
    let flags = out.flags;
    let share = FILE_SHARE_READ | if flags & XTFCOPYFILE_SHAREWRITE != 0 { FILE_SHARE_WRITE } else { 0 };
    let h = match open_with_retry(&f.file_name, GENERIC_READ, share, OPEN_EXISTING, 0) {
        Ok(h) => h,
        Err(e) => return e,
    };
    let dest_s = String::from_utf16_lossy(dest);
    let mut size = 0u64;
    let mut sent = 0u64;
    let mut abort = false;
    let hr = 'file: {
        let mut sz = 0i64;
        if GetFileSizeEx(h, &mut sz) == 0 {
            break 'file last_hr_or_fail();
        }
        size = sz as u64;
        timer.mark("ReadFileMeta");
        if let Some(n) = notify.as_mut() {
            let hr = n.start(out.root, out.pattern, &f.file_name, &f.info, &dest_s);
            if hr < 0 {
                abort = true;
                break 'file hr;
            }
            let hr = n.progress(&f.file_name, &dest_s, size, 0);
            if hr < 0 {
                abort = true;
                break 'file hr;
            }
        }
        let mut first = true;
        while sent < size || (size == 0 && first) {
            let msg = match new_message() {
                Ok(m) => m,
                Err(e) => break 'file e,
            };
            let mut payload: Vec<u8> = Vec::new();
            if out.no_json {
                if *first_global {
                    *first_global = false;
                    let hr = json_no_json_preamble(msg.0, flags, out.token);
                    if hr < 0 {
                        break 'file hr;
                    }
                }
                if first {
                    payload = leader_bytes(DIRECT_PROTOCOL_BLOCK_SIZE, size, &f.info, dest);
                }
            } else if first {
                let hr = json_head(msg.0, dest, size, &f.info, flags, out.token);
                if hr < 0 {
                    break 'file hr;
                }
            }
            first = false;
            if size > 0 {
                timer.mark("ReadCpu");
                let mut n = 0u32;
                if ReadFile(h, buf.as_mut_ptr(), buf.len() as u32, &mut n, null_mut()) == 0 {
                    break 'file last_hr_or_fail();
                }
                timer.mark("ReadFile");
                if n == 0 {
                    break 'file hr_from_win32(ERROR_HANDLE_EOF);
                }
                sent += n as u64;
                payload.extend_from_slice(&buf[..n as usize]);
            }
            if !payload.is_empty() {
                let hr = set_binary(msg.0, &payload);
                if hr < 0 {
                    break 'file hr;
                }
            }
            timer.mark("ReadCpu");
            let hr = session.send(msg.0);
            if hr < 0 {
                break 'file hr;
            }
            timer.mark("WriteNetwork");
            if let Some(n) = notify.as_mut() {
                let hr = n.progress(&f.file_name, &dest_s, size, sent);
                if hr < 0 {
                    abort = true;
                    break 'file hr;
                }
            }
        }
        S_OK
    };
    CloseHandle(h);
    if abort && sent < size {
        timer.mark("ReadCpu");
        if let Ok(m) = new_message() {
            session.send(m.0);
        }
        timer.mark("WriteNetwork");
    }
    hr
}

#[allow(clippy::too_many_arguments)]
pub unsafe fn files_to_messages(session: &mut dyn Channel, root: &str, files: &[FoundFile], dst: &[u16], dst_is_directory: bool, out: &Outgoing, timer: &mut Timer, count: &mut u32, mut notify: Option<&mut dyn CopyNotify>) -> HRESULT {
    *count = 0;
    let mut buf = vec![0u8; DIRECT_PROTOCOL_BLOCK_SIZE as usize];
    let mut first_global = true;
    let dst_s = String::from_utf16_lossy(dst);
    let mut hr = S_OK;
    for f in files {
        if f.info.attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
            continue;
        }
        let dest: Vec<u16> = if dst_is_directory {
            match relative_file_name(root, &f.file_name) {
                Ok(rel) => append_path(&dst_s, Some(&rel)).encode_utf16().collect(),
                Err(e) => {
                    hr = e;
                    break;
                }
            }
        } else {
            dst.to_vec()
        };
        hr = send_one_file(session, f, &dest, out, timer, &mut first_global, &mut buf, &mut notify);
        if hr < 0 {
            break;
        }
        *count += 1;
        if let Some(n) = notify.as_mut() {
            let r = n.end(out.root, out.pattern, &f.file_name, &f.info, &String::from_utf16_lossy(&dest), hr);
            if r < 0 {
                hr = r;
                break;
            }
        }
    }
    send_terminator(session, timer);
    hr
}

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct FileHead {
    pub block: u64,
    pub size: u64,
    pub attributes: u32,
    pub creation: u64,
    pub access: u64,
    pub write: u64,
}

pub struct AlignedBuf {
    ptr: *mut u8,
    len: usize,
}

impl AlignedBuf {
    pub fn new(len: usize) -> Result<Self, HRESULT> {
        if len == 0 {
            return Ok(AlignedBuf { ptr: null_mut(), len: 0 });
        }
        let layout = Layout::from_size_align(len, BUFFER_ALIGN).map_err(|_| E_OUTOFMEMORY)?;
        let ptr = unsafe { alloc_zeroed(layout) };
        if ptr.is_null() {
            return Err(E_OUTOFMEMORY);
        }
        Ok(AlignedBuf { ptr, len })
    }

    pub fn set(&mut self, data: &[u8]) -> HRESULT {
        if data.len() > self.len {
            let mut n = match AlignedBuf::new(data.len()) {
                Ok(b) => b,
                Err(e) => return e,
            };
            core::mem::swap(self, &mut n);
        }
        if !data.is_empty() {
            unsafe { core::ptr::copy_nonoverlapping(data.as_ptr(), self.ptr, data.len()) };
        }
        S_OK
    }
}

impl Drop for AlignedBuf {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            unsafe { dealloc(self.ptr, Layout::from_size_align_unchecked(self.len, BUFFER_ALIGN)) };
        }
    }
}

pub fn delete_files(console: &str) -> HRESULT {
    let a = file::get_attributes_raw(console);
    let hr = if a == INVALID_FILE_ATTRIBUTES {
        last_hr()
    } else {
        if a & FILE_ATTRIBUTE_READONLY != 0 {
            let hr = file::set_file_attributes(console, a & !FILE_ATTRIBUTE_READONLY);
            if hr < 0 {
                return hr;
            }
        }
        match host_wide(console) {
            Ok(w) => {
                if unsafe { DeleteFileW(w.as_ptr()) } != 0 {
                    S_OK
                } else {
                    let hr = last_hr();
                    file::set_file_attributes(console, a);
                    hr
                }
            }
            Err(e) => e,
        }
    };
    if hr == HR_FILE_NOT_FOUND || hr == HR_PATH_NOT_FOUND {
        S_OK
    } else {
        hr
    }
}

pub struct Writer {
    pub handle: isize,
    pub local: String,
    pub final_name: Option<String>,
    pub no_buffering: bool,
    pub head: FileHead,
}

fn temp_file_in(dir: &str) -> Result<String, HRESULT> {
    let mut seed = unsafe { GetTickCount64() } as u32;
    loop {
        let name = append_path(dir, Some(&format!("XTF{:04X}.tmp", seed & 0xFFFF)));
        match open_with_retry(&name, GENERIC_READ | GENERIC_WRITE, 0, CREATE_NEW, 0) {
            Ok(h) => {
                unsafe { CloseHandle(h) };
                return Ok(name);
            }
            Err(e) if e == hr_from_win32(ERROR_FILE_EXISTS) => seed = seed.wrapping_add(1),
            Err(e) => return Err(e),
        }
    }
}

impl Writer {
    pub fn open(path: Option<&str>, flags: u32, head: &FileHead, no_buffering: bool) -> Result<Writer, HRESULT> {
        let path = path.ok_or(E_INVALIDARG)?;
        let units: Vec<u16> = path.encode_utf16().collect();
        let (ty, rel) = path_type(&units)?;
        if ty == PATH_APPLICATION {
            return Err(HR_FILE_NOT_FOUND);
        }
        let rel = if ty == PATH_CONSOLE { String::from_utf16_lossy(&units[rel..]) } else { path.to_string() };
        let mut local = full_path_name(&rel)?;
        let dir = file::get_parent_directory(&local)?;
        let hr = file::create_directory(&dir);
        if hr < 0 {
            return Err(hr);
        }
        let mut final_name = None;
        let hr = delete_files(&local);
        if hr < 0 {
            if hr == E_ACCESSDENIED && flags & XTFCOPYFILE_DEFER_LOCKED != 0 {
                final_name = Some(local.clone());
                local = temp_file_in(&dir)?;
            } else {
                return Err(hr);
            }
        }
        let attrs = if no_buffering { FILE_FLAG_NO_BUFFERING } else { 0 };
        let handle = open_with_retry(&local, GENERIC_WRITE, FILE_SHARE_READ, CREATE_ALWAYS, attrs)?;
        Ok(Writer { handle, local, final_name, no_buffering, head: *head })
    }

    pub fn write_at(&mut self, data: &[u8], offset: u64) -> Result<u32, HRESULT> {
        let mut ov = Overlapped { offset: offset as u32, offset_high: (offset >> 32) as u32, ..Default::default() };
        let mut n = 0u32;
        if unsafe { WriteFile(self.handle, data.as_ptr(), data.len() as u32, &mut n, &mut ov) } == 0 {
            return Err(last_hr_or_fail());
        }
        Ok(n)
    }

    fn truncate(&self, size: u64) -> HRESULT {
        unsafe {
            if SetFilePointerEx(self.handle, size as i64, null_mut(), 0) == 0 || SetEndOfFile(self.handle) == 0 {
                return last_hr_or_fail();
            }
        }
        S_OK
    }

    pub fn close(&mut self, hr: HRESULT) -> HRESULT {
        if self.handle == INVALID_HANDLE {
            return S_OK;
        }
        if hr >= 0 {
            if self.no_buffering {
                unsafe { CloseHandle(self.handle) };
                self.handle = INVALID_HANDLE;
                self.handle = match open_with_retry(&self.local, GENERIC_WRITE, FILE_SHARE_READ, OPEN_EXISTING, 0) {
                    Ok(h) => h,
                    Err(e) => return e,
                };
                self.no_buffering = false;
                let r = self.truncate(self.head.size);
                if r < 0 {
                    return r;
                }
            }
            let h = &self.head;
            if unsafe { SetFileTime(self.handle, &h.creation, &h.access, &h.write) } == 0 {
                return last_hr_or_fail();
            }
            let r = file::set_file_attributes(&self.local, h.attributes);
            if r < 0 {
                return r;
            }
        }
        if hr < 0 {
            self.truncate(0);
        }
        unsafe { CloseHandle(self.handle) };
        self.handle = INVALID_HANDLE;
        let mut out = hr;
        if hr == HR_CANCELLED || hr == HR_CONNRESET {
            delete_files(&self.local);
            self.final_name = None;
        }
        if let Some(fin) = self.final_name.take() {
            let r = file::move_file_deferred(&self.local, Some(&fin));
            if r < 0 {
                out = r;
            }
        }
        out
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        if self.handle != INVALID_HANDLE {
            self.close(HR_CANCELLED);
        }
    }
}

pub fn write_small_file(path: Option<&str>, flags: u32, head: &FileHead, data: &[u8]) -> HRESULT {
    let mut w = match Writer::open(path, flags, head, false) {
        Ok(w) => w,
        Err(e) => return e,
    };
    let mut written = 0u32;
    let mut hr_write = S_OK;
    if head.size != 0 {
        let mut buf = data.to_vec();
        if (buf.len() as u64) < head.size {
            buf.resize(head.size as usize, 0);
        }
        match w.write_at(&buf[..head.size as usize], 0) {
            Ok(n) => written = n,
            Err(e) => hr_write = e,
        }
    }
    let hr_close = w.close(hr_write);
    if hr_write < 0 {
        hr_write
    } else if hr_close < 0 {
        hr_close
    } else if written as u64 != head.size {
        E_UNEXPECTED
    } else {
        S_OK
    }
}

enum Head {
    End,
    File { name: Vec<u16>, head: FileHead, data_off: usize },
}

unsafe fn parse_json_head(msg: *mut c_void) -> Result<Head, HRESULT> {
    let mut body = Com::null();
    let hr = deserialize_json_body(msg, &mut body);
    if hr < 0 {
        return Err(hr);
    }
    let mut name = Bstr::null();
    let hr = get_bstr(body.0, "FileName", &mut name);
    if hr < 0 {
        return Err(hr);
    }
    if name.is_empty() {
        return Ok(Head::End);
    }
    let units = bstr_units(&name);
    let chk = |hr: HRESULT| if hr < 0 { Err(hr) } else { Ok(()) };
    let mut h = FileHead::default();
    let (ty, _) = path_type(&units)?;
    let path_ok = ty != PATH_APPLICATION;
    let full = if path_ok { Some(full_path_name(&String::from_utf16_lossy(&units))?) } else { None };
    chk(get_u64(body.0, "FileBlockSize", &mut h.block))?;
    chk(get_u64(body.0, "FileSize", &mut h.size))?;
    chk(get_u32(body.0, "FileAttributes", &mut h.attributes))?;
    chk(get_u64(body.0, "CreationTime", &mut h.creation))?;
    chk(get_u64(body.0, "LastAccessTime", &mut h.access))?;
    chk(get_u64(body.0, "LastWriteTime", &mut h.write))?;
    Ok(Head::File { name: full.map(|f| f.encode_utf16().collect()).unwrap_or_default(), head: h, data_off: 0 })
}

fn rd64(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}

unsafe fn parse_leader(msg: *mut c_void) -> Result<Head, HRESULT> {
    let (p, n) = binary_body(msg)?;
    if n == 0 {
        return Ok(Head::End);
    }
    if p.is_null() {
        return Err(E_INVALIDARG);
    }
    if n < MIN_LEADER_BODY {
        return Err(HR_BAD_LENGTH);
    }
    let b = binary_slice(p, n);
    let h = FileHead {
        block: rd64(b, 0),
        size: rd64(b, 8),
        attributes: u32::from_le_bytes(b[0x10..0x14].try_into().unwrap()),
        creation: rd64(b, 0x18),
        access: rd64(b, 0x20),
        write: rd64(b, 0x28),
    };
    let mut name = Vec::new();
    let mut o = LEADER_SIZE;
    while o + 1 < b.len() {
        let u = u16::from_le_bytes([b[o], b[o + 1]]);
        o += 2;
        if u == 0 {
            break;
        }
        name.push(u);
    }
    if name.is_empty() {
        return Ok(Head::End);
    }
    let (ty, _) = path_type(&name)?;
    let full = if ty != PATH_APPLICATION { full_path_name(&String::from_utf16_lossy(&name))?.encode_utf16().collect() } else { Vec::new() };
    Ok(Head::File { name: full, head: h, data_off: o.min(b.len()) })
}

pub unsafe fn messages_to_files(session: &mut dyn Channel, request: *mut c_void, flags: u32, timer: &mut Timer, count: &mut u32, no_json: bool, mut notify: Option<&mut dyn CopyNotify>) -> HRESULT {
    *count = 0;
    let mut pending = S_OK;
    let mut dst: Option<String> = None;
    let mut first = true;
    let mut bufs: [Option<AlignedBuf>; 3] = [None, None, None];
    loop {
        let more;
        let head_msg = if first && !request.is_null() {
            first = false;
            request
        } else {
            first = false;
            timer.mark("WriteCpu");
            more = match session.receive_more(DEFAULT_RECEIVE_TIMEOUT) {
                Ok(m) => m,
                Err(e) => return e,
            };
            timer.mark("ReadNetwork");
            more.0
        };
        let parsed = if no_json { parse_leader(head_msg) } else { parse_json_head(head_msg) };
        let (name, head, data_off) = match parsed {
            Err(e) => return e,
            Ok(Head::End) => return pending,
            Ok(Head::File { name, head, data_off }) => (name, head, data_off),
        };
        if !name.is_empty() {
            dst = Some(String::from_utf16_lossy(&name));
        }
        if pending < 0 {
            return pending;
        }
        let dst_s = dst.clone().unwrap_or_default();
        let info = FileInfo { attributes: head.attributes, creation: head.creation, access: head.access, write: head.write, size: head.size };
        if let Some(n) = notify.as_mut() {
            let hr = n.start(&dst_s, &dst_s, &dst_s, &info, &dst_s);
            if hr < 0 {
                return hr;
            }
            let hr = n.progress(&dst_s, &dst_s, head.size, 0);
            if hr < 0 {
                return hr;
            }
        }
        if head.size <= head.block {
            timer.mark("WriteCpu");
            timer.mark("WriteFileMeta");
            let (p, n) = match binary_body(head_msg) {
                Ok(v) => v,
                Err(e) => return e,
            };
            timer.mark("ReadNetwork");
            let all = binary_slice(p, n);
            let data = &all[data_off.min(all.len())..];
            *count += 1;
            timer.mark("WriteFileMeta");
            pending = write_small_file(dst.as_deref(), flags, &head, data);
            if let Some(n) = notify.as_mut() {
                if pending >= 0 {
                    let hr = n.progress(&dst_s, &dst_s, head.size, head.size);
                    if hr < 0 {
                        return hr;
                    }
                }
                let hr = n.end(&dst_s, &dst_s, &dst_s, &info, &dst_s, pending);
                if hr < 0 {
                    return hr;
                }
            }
            continue;
        }
        for b in bufs.iter_mut() {
            *b = match AlignedBuf::new(head.block as usize) {
                Ok(v) => Some(v),
                Err(e) => return e,
            };
        }
        timer.mark("WriteCpu");
        let mut w = match Writer::open(dst.as_deref(), flags, &head, true) {
            Ok(w) => w,
            Err(e) => return e,
        };
        timer.mark("WriteFileMeta");
        let mut offset = 0u64;
        let mut k = 0usize;
        while offset < head.size {
            timer.mark("WriteCpu");
            let chunk_msg;
            let (p, n, skip) = if k == 0 {
                match binary_body(head_msg) {
                    Ok((p, n)) => (p, n, data_off),
                    Err(e) => return e,
                }
            } else {
                chunk_msg = match session.receive_more(DEFAULT_RECEIVE_TIMEOUT) {
                    Ok(m) => m,
                    Err(e) => return e,
                };
                match binary_body(chunk_msg.0) {
                    Ok((p, n)) => (p, n, 0),
                    Err(e) => return e,
                }
            };
            let all = binary_slice(p, n);
            let data = &all[skip.min(all.len())..];
            timer.mark("ReadNetwork");
            if data.is_empty() {
                return HR_CANCELLED;
            }
            timer.mark("WriteCpu");
            let buf = bufs[k % 3].as_mut().unwrap();
            let hr = buf.set(data);
            if hr < 0 {
                return hr;
            }
            let len = head.block as usize;
            let slice = core::slice::from_raw_parts(buf.ptr, len.min(buf.len));
            if let Err(e) = w.write_at(slice, offset) {
                return e;
            }
            offset += head.block;
            k += 1;
            timer.mark("WriteFile");
            if let Some(n) = notify.as_mut() {
                let hr = n.progress(&dst_s, &dst_s, head.size, offset.min(head.size));
                if hr < 0 {
                    w.close(hr);
                    return hr;
                }
            }
        }
        timer.mark("WriteCpu");
        let hr = w.close(S_OK);
        if hr < 0 {
            return hr;
        }
        timer.mark("WriteFileMeta");
        *count += 1;
        if let Some(n) = notify.as_mut() {
            let r = n.end(&dst_s, &dst_s, &dst_s, &info, &dst_s, hr);
            if r < 0 {
                return r;
            }
        }
    }
}
