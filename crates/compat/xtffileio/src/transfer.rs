use crate::com::*;
use crate::file::{self, FileInfo};
use crate::find::{get_file_info_properties, set_file_info_properties};
use crate::msg::*;
use crate::paths::*;
use core::ffi::c_void;
use core::ptr::null_mut;

const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const FILE_SHARE_READ: u32 = 1;
const CREATE_NEW: u32 = 1;
const CREATE_ALWAYS: u32 = 2;
const OPEN_EXISTING: u32 = 3;
const FILE_FLAG_OVERLAPPED: u32 = 0x4000_0000;
const FILE_FLAG_SEQUENTIAL_SCAN: u32 = 0x0800_0000;
const INVALID_HANDLE_VALUE: isize = -1;
pub const CHUNK_COUNT: u32 = 4;
pub const MIN_CHUNK: u32 = 0x1000;
pub const MAX_CHUNK: u32 = 0xa0000;
pub const XTF_SHARE_WRITE_FLAG: u32 = 0x4000_0000;

#[repr(C)]
#[derive(Default)]
struct Overlapped {
    internal: usize,
    internal_high: usize,
    offset: u32,
    offset_high: u32,
    event: isize,
}

#[link(name = "kernel32")]
extern "system" {
    fn CreateFileW(name: *const u16, access: u32, share: u32, sa: *mut c_void, disp: u32, flags: u32, tmpl: isize) -> isize;
    fn CloseHandle(h: isize) -> i32;
    fn ReadFile(h: isize, buf: *mut u8, n: u32, done: *mut u32, ov: *mut Overlapped) -> i32;
    fn WriteFile(h: isize, buf: *const u8, n: u32, done: *mut u32, ov: *mut Overlapped) -> i32;
    fn SetFilePointerEx(h: isize, dist: i64, newpos: *mut i64, method: u32) -> i32;
    fn SetEndOfFile(h: isize) -> i32;
    fn GetFileSizeEx(h: isize, size: *mut i64) -> i32;
    fn GetFileTime(h: isize, c: *mut u64, a: *mut u64, w: *mut u64) -> i32;
    fn SetFileTime(h: isize, c: *const u64, a: *const u64, w: *const u64) -> i32;
    fn GetTickCount64() -> u64;
    fn Sleep(ms: u32);
    fn CreateEventW(sa: *mut c_void, manual: i32, initial: i32, name: *const u16) -> isize;
    fn SetEvent(h: isize) -> i32;
    fn WaitForSingleObject(h: isize, ms: u32) -> u32;
    fn GetCurrentProcess() -> isize;
}

#[link(name = "advapi32")]
extern "system" {
    fn AllocateAndInitializeSid(auth: *const [u8; 6], count: u8, s0: u32, s1: u32, s2: u32, s3: u32, s4: u32, s5: u32, s6: u32, s7: u32, sid: *mut *mut c_void) -> i32;
    fn FreeSid(sid: *mut c_void) -> *mut c_void;
    fn SetEntriesInAclW(count: u32, list: *const ExplicitAccessW, old: *mut c_void, new: *mut *mut c_void) -> u32;
    fn SetNamedSecurityInfoW(name: *const u16, kind: u32, info: u32, owner: *mut c_void, group: *mut c_void, dacl: *mut c_void, sacl: *mut c_void) -> u32;
    fn OpenProcessToken(p: isize, access: u32, token: *mut isize) -> i32;
    fn LookupPrivilegeValueW(system: *const u16, name: *const u16, luid: *mut u64) -> i32;
    fn AdjustTokenPrivileges(token: isize, disable_all: i32, new: *const TokenPrivileges, len: u32, prev: *mut c_void, ret: *mut u32) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn LocalFree(p: *mut c_void) -> *mut c_void;
}

#[repr(C)]
struct Trustee {
    multiple: *mut c_void,
    multiple_op: u32,
    form: u32,
    kind: u32,
    name: *mut c_void,
}

#[repr(C)]
struct ExplicitAccessW {
    permissions: u32,
    mode: u32,
    inheritance: u32,
    trustee: Trustee,
}

#[repr(C, packed(4))]
struct TokenPrivileges {
    count: u32,
    luid: u64,
    attributes: u32,
}

fn last_hr() -> HRESULT {
    hr_from_win32(unsafe { file::GetLastError() })
}

pub struct Stream {
    pub handle: isize,
}

impl Stream {
    pub fn closed() -> Self {
        Stream { handle: INVALID_HANDLE_VALUE }
    }

    pub fn is_open(&self) -> bool {
        self.handle != INVALID_HANDLE_VALUE
    }

    fn raw_open(&mut self, console: &str, access: u32, share: u32, disp: u32, flags: u32) -> HRESULT {
        let h = match host_path(console) {
            Ok(p) => unsafe { CreateFileW(p.wide.as_ptr(), access, share, null_mut(), disp, flags & !FILE_FLAG_OVERLAPPED, 0) },
            Err(e) => {
                unsafe { file::SetLastError(e) };
                INVALID_HANDLE_VALUE
            }
        };
        self.handle = h;
        if h == INVALID_HANDLE_VALUE {
            last_hr()
        } else {
            S_OK
        }
    }

    pub fn open(&mut self, console: &str, access: u32, share: u32, disp: u32, flags: u32) -> HRESULT {
        let mut hr = S_OK;
        for _ in 0..5 {
            hr = self.raw_open(console, access, share, disp, flags);
            if hr >= 0 {
                break;
            }
            if (hr as u32) & 0xffff_0000 == 0x8007_0000 {
                let code = hr as u32 & 0xffff;
                if code != 5 && code != 0x20 {
                    break;
                }
                unsafe { Sleep(100) };
            }
        }
        hr
    }

    pub fn seek(&mut self, pos: i64) -> HRESULT {
        let mut np = 0i64;
        if unsafe { SetFilePointerEx(self.handle, pos, &mut np, 0) } == 0 {
            last_hr()
        } else {
            S_OK
        }
    }

    pub fn set_end_of_file(&mut self) -> HRESULT {
        if unsafe { SetEndOfFile(self.handle) } == 0 {
            last_hr()
        } else {
            S_OK
        }
    }

    pub fn close(&mut self, truncate: bool) -> HRESULT {
        let mut hr = S_OK;
        if self.is_open() {
            if truncate {
                hr = self.seek(0);
                if hr >= 0 {
                    hr = self.set_end_of_file();
                }
            }
            unsafe { CloseHandle(self.handle) };
            self.handle = INVALID_HANDLE_VALUE;
        }
        hr
    }

    pub fn close_with_retry(&mut self, truncate: bool) -> HRESULT {
        let mut hr = S_OK;
        for _ in 0..5 {
            hr = self.close(truncate);
            if hr >= 0 {
                break;
            }
            if (hr as u32) & 0xffff_0000 == 0x8007_0000 {
                let code = hr as u32 & 0xffff;
                if code != 5 && code != 0x20 {
                    break;
                }
                unsafe { Sleep(100) };
            }
        }
        hr
    }

    pub fn read_at(&mut self, buf: &mut [u8], offset: u64) -> Result<u32, HRESULT> {
        let mut ov = Overlapped { offset: offset as u32, offset_high: (offset >> 32) as u32, ..Default::default() };
        let mut done = 0u32;
        if unsafe { ReadFile(self.handle, buf.as_mut_ptr(), buf.len() as u32, &mut done, &mut ov) } == 0 {
            return Err(last_hr());
        }
        Ok(done)
    }

    pub fn write_at(&mut self, buf: *const u8, n: u32, offset: u64) -> Result<u32, HRESULT> {
        let mut ov = Overlapped { offset: offset as u32, offset_high: (offset >> 32) as u32, ..Default::default() };
        let mut done = 0u32;
        if unsafe { WriteFile(self.handle, buf, n, &mut done, &mut ov) } == 0 {
            return Err(last_hr());
        }
        Ok(done)
    }

    pub fn file_size(&mut self) -> Result<u64, HRESULT> {
        let mut s = 0i64;
        if unsafe { GetFileSizeEx(self.handle, &mut s) } == 0 {
            Err(last_hr())
        } else {
            Ok(s as u64)
        }
    }

    pub fn file_time(&mut self) -> Result<(u64, u64, u64), HRESULT> {
        let (mut c, mut a, mut w) = (0u64, 0u64, 0u64);
        if unsafe { GetFileTime(self.handle, &mut c, &mut a, &mut w) } == 0 {
            Err(last_hr())
        } else {
            Ok((c, a, w))
        }
    }

    pub fn set_file_time(&mut self, c: u64, a: u64, w: u64) -> HRESULT {
        if unsafe { SetFileTime(self.handle, &c, &a, &w) } == 0 {
            last_hr()
        } else {
            S_OK
        }
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        if self.is_open() {
            unsafe { CloseHandle(self.handle) };
            self.handle = INVALID_HANDLE_VALUE;
        }
    }
}

pub fn optimal_chunks(size: u64) -> (u32, u32) {
    let chunk = if size < MIN_CHUNK as u64 { MIN_CHUNK } else { size.min(MAX_CHUNK as u64) as u32 };
    let mut count = 1u32;
    let mut left = size;
    while left > chunk as u64 {
        left -= chunk as u64;
        count += 1;
        if count >= CHUNK_COUNT {
            break;
        }
    }
    (count, chunk)
}

pub fn get_temp_file_name(dir: &str, prefix: &str) -> Result<String, HRESULT> {
    let p: Vec<char> = prefix.chars().collect();
    if dir.is_empty() || p.len() < 3 {
        return Err(E_INVALIDARG);
    }
    let seed = (unsafe { GetTickCount64() } % 0xffff) as u16;
    let mut n = seed;
    loop {
        let name = format!("{dir}\\{}{}{}{:04X}.tmp", p[0], p[1], p[2], n);
        n = n.wrapping_add(1);
        let mut s = Stream::closed();
        let hr = s.raw_open(&name, GENERIC_READ | GENERIC_WRITE, 0, CREATE_NEW, 0);
        drop(s);
        if hr == HR_FILE_EXISTS && n != seed {
            continue;
        }
        return if hr < 0 { Err(hr) } else { Ok(name) };
    }
}

unsafe fn set_privilege(token: isize, name: &str, enable: bool) -> bool {
    let mut luid = 0u64;
    let n = wide(name);
    if LookupPrivilegeValueW(core::ptr::null(), n.as_ptr(), &mut luid) == 0 {
        return false;
    }
    let tp = TokenPrivileges { count: 1, luid, attributes: if enable { 2 } else { 0 } };
    if AdjustTokenPrivileges(token, 0, &tp, 0x10, null_mut(), null_mut()) == 0 {
        return false;
    }
    file::GetLastError() != 0x514
}

pub fn grant_full_access(console: &str) -> bool {
    let Ok(host) = host_path(console) else { return false };
    let path = wide(&host.path.to_string_lossy());
    unsafe {
        let world_auth = [0u8, 0, 0, 0, 0, 1];
        let nt_auth = [0u8, 0, 0, 0, 0, 5];
        let mut world: *mut c_void = null_mut();
        let mut admins: *mut c_void = null_mut();
        let mut acl: *mut c_void = null_mut();
        let mut token: isize = 0;
        let mut ok = false;
        'work: {
            if AllocateAndInitializeSid(&world_auth, 1, 0, 0, 0, 0, 0, 0, 0, 0, &mut world) == 0 {
                break 'work;
            }
            if AllocateAndInitializeSid(&nt_auth, 2, 0x20, 0x220, 0, 0, 0, 0, 0, 0, &mut admins) == 0 {
                break 'work;
            }
            let ea = [
                ExplicitAccessW { permissions: 0x1001_0000, mode: 2, inheritance: 0, trustee: Trustee { multiple: null_mut(), multiple_op: 0, form: 0, kind: 5, name: world } },
                ExplicitAccessW { permissions: 0x1001_0000, mode: 2, inheritance: 0, trustee: Trustee { multiple: null_mut(), multiple_op: 0, form: 0, kind: 2, name: admins } },
            ];
            if SetEntriesInAclW(2, ea.as_ptr(), null_mut(), &mut acl) != 0 {
                break 'work;
            }
            let e = SetNamedSecurityInfoW(path.as_ptr(), 1, 4, null_mut(), null_mut(), acl, null_mut());
            if e == 0 {
                ok = true;
                break 'work;
            }
            if e != 5 {
                break 'work;
            }
            if OpenProcessToken(GetCurrentProcess(), 0x20, &mut token) == 0 {
                token = 0;
                break 'work;
            }
            if !set_privilege(token, "SeTakeOwnershipPrivilege", true) {
                break 'work;
            }
            if SetNamedSecurityInfoW(path.as_ptr(), 1, 1, admins, null_mut(), null_mut(), null_mut()) != 0 {
                break 'work;
            }
            if !set_privilege(token, "SeTakeOwnershipPrivilege", false) {
                break 'work;
            }
            ok = SetNamedSecurityInfoW(path.as_ptr(), 1, 4, null_mut(), null_mut(), acl, null_mut()) == 0;
        }
        if !admins.is_null() {
            FreeSid(admins);
        }
        if !world.is_null() {
            FreeSid(world);
        }
        if !acl.is_null() {
            LocalFree(acl);
        }
        if token != 0 {
            CloseHandle(token);
        }
        ok
    }
}

unsafe fn session_receive(session: *mut c_void, out: &mut Com) -> HRESULT {
    if session.is_null() {
        return E_INVALIDARG;
    }
    (vt::<SessionVtbl>(session).receive_more_message)(session, XTF_RECEIVE_RECOMMENDED_TIMEOUT_MS, out.out())
}

unsafe fn props_of(message: *mut c_void) -> Result<Com, HRESULT> {
    let mut p = Com::null();
    let hr = deserialize_json_body(message, &mut p);
    if hr < 0 {
        Err(hr)
    } else {
        Ok(p)
    }
}

pub struct TransferRequest {
    pub src: Bstr,
    pub dst: Bstr,
    pub flags: u32,
    pub info: FileInfo,
}

pub unsafe fn parse_transfer_request(message: *mut c_void, with_info: bool) -> Result<TransferRequest, HRESULT> {
    let props = props_of(message)?;
    if with_info {
        props_of(message)?;
    }
    let mut r = TransferRequest { src: Bstr::null(), dst: Bstr::null(), flags: 0, info: FileInfo::default() };
    let hr = get_string(props.0, "SrcFileName", &mut r.src);
    if hr < 0 {
        return Err(hr);
    }
    let hr = get_string(props.0, "DstFileName", &mut r.dst);
    if hr < 0 {
        return Err(hr);
    }
    let hr = get_u32(props.0, "Flags", &mut r.flags);
    if hr < 0 {
        return Err(hr);
    }
    if with_info {
        let hr = get_file_info_properties(props.0, &mut r.info);
        if hr < 0 {
            return Err(hr);
        }
    }
    Ok(r)
}

unsafe fn local_of(name: &Bstr) -> Result<String, HRESULT> {
    let (t, rel) = crate::handlers::local_path(name)?;
    if t == PATH_APPLICATION {
        return Err(HR_FILE_NOT_FOUND);
    }
    full_path_name(&rel)
}

pub struct FileWriter {
    pub req: TransferRequest,
    pub local: String,
    pub final_name: String,
    pub deferred: bool,
    pub stream: Stream,
    pub offset: u64,
}

impl FileWriter {
    pub fn new(req: TransferRequest) -> Self {
        FileWriter { req, local: String::new(), final_name: String::new(), deferred: false, stream: Stream::closed(), offset: 0 }
    }

    pub unsafe fn start(&mut self) -> HRESULT {
        self.local = match local_of(&self.req.dst) {
            Ok(l) => l,
            Err(e) => return e,
        };
        let dir = match file::get_parent_directory(&self.local) {
            Ok(d) => d,
            Err(e) => return e,
        };
        let hr = file::create_directory(&dir);
        if hr < 0 {
            return hr;
        }
        let mut hr = file::delete_files(&self.local, 0);
        if hr == E_ACCESSDENIED && grant_full_access(&self.local) {
            hr = file::delete_files(&self.local, 0);
        }
        if (hr as u32) & 0xffff_0000 == 0x8007_0000 && matches!(hr as u32 & 0xffff, 2 | 3) {
            hr = S_OK;
        } else if hr == E_ACCESSDENIED && (self.req.flags as i32) < 0 {
            self.final_name = core::mem::take(&mut self.local);
            match get_temp_file_name(&dir, "XTF") {
                Ok(t) => {
                    self.local = t;
                    hr = S_OK;
                }
                Err(e) => return e,
            }
            self.deferred = true;
        }
        if hr < 0 {
            return hr;
        }
        self.stream.open(&self.local, GENERIC_WRITE, FILE_SHARE_READ, CREATE_ALWAYS, FILE_FLAG_OVERLAPPED)
    }

    pub unsafe fn begin_write(&mut self, message: *mut c_void) -> Result<(u64, u32), HRESULT> {
        let props = props_of(message)?;
        let mut cancel = 0u32;
        let hr = get_u32(props.0, "CancelTransfer", &mut cancel);
        if hr != E_JSON_NOT_FOUND {
            if hr < 0 {
                return Err(hr);
            }
            if cancel > 0 {
                return Err(HR_CANCELLED);
            }
        }
        let mut off = 0u64;
        let hr = get_u64(props.0, "FileOffset", &mut off);
        if hr < 0 {
            return Err(hr);
        }
        if off != self.offset {
            return Err(E_UNEXPECTED);
        }
        let mut n = 0u32;
        let mut buf: *mut u8 = null_mut();
        let hr = (vt::<MessageVtbl>(message).get_binary_body)(message, &mut n, &mut buf);
        if hr < 0 {
            return Err(hr);
        }
        let done = self.stream.write_at(buf, n, off)?;
        self.offset += n as u64;
        Ok((off, if done == n { n } else { u32::MAX }))
    }

    pub unsafe fn process(&mut self, session: *mut c_void, request: *mut c_void) -> HRESULT {
        let mut first = true;
        loop {
            let size = self.req.info.size;
            if self.offset >= size && (size != 0 || !first) {
                return S_OK;
            }
            let mut received = Com::null();
            let m = if first {
                request
            } else {
                loop {
                    let hr = session_receive(session, &mut received);
                    if hr == E_PENDING {
                        continue;
                    }
                    if hr < 0 {
                        return hr;
                    }
                    break;
                }
                received.0
            };
            let (off, n) = match self.begin_write(m) {
                Ok(v) => v,
                Err(e) => return e,
            };
            if n == u32::MAX {
                return HR_IO_DEVICE;
            }
            if off + n as u64 > size {
                return E_UNEXPECTED;
            }
            first = false;
            if size == 0 {
                return S_OK;
            }
        }
    }

    pub unsafe fn finish(&mut self, op: HRESULT) -> HRESULT {
        if !self.stream.is_open() {
            return S_OK;
        }
        if op >= 0 {
            let i = self.req.info;
            let hr = self.stream.set_file_time(i.creation, i.access, i.write);
            if hr < 0 {
                return hr;
            }
            let hr = file::set_file_attributes(&self.local, i.attributes);
            if hr < 0 {
                return hr;
            }
        }
        let hr = self.stream.close_with_retry(op < 0);
        if hr < 0 {
            return hr;
        }
        let mut hr = hr;
        if op == HR_WSAECONNRESET || op == HR_CANCELLED {
            hr = file::delete_files(&self.local, 0);
            if hr < 0 {
                return hr;
            }
            self.deferred = false;
        }
        if self.deferred {
            hr = file::move_file_deferred(&self.local, Some(&self.final_name));
        }
        hr
    }

    pub unsafe fn execute(&mut self, session: *mut c_void, request: *mut c_void) -> HRESULT {
        let mut hr = self.start();
        if hr >= 0 {
            hr = self.process(session, request);
        }
        let f = self.finish(hr);
        if hr >= 0 && f < 0 {
            f
        } else {
            hr
        }
    }
}

pub unsafe extern "system" fn send_file_handler(session: usize, request: *mut c_void, response: *mut c_void) -> HRESULT {
    let hr = match parse_transfer_request(request, true) {
        Ok(req) => FileWriter::new(req).execute(session as *mut c_void, request),
        Err(e) => e,
    };
    serialize(&MsgResponse { result: hr }, response)
}

struct SentEvent {
    event: isize,
}

unsafe extern "system" fn message_sent(_a: usize, _b: usize, _c: usize, ctx: *mut c_void) {
    let e = &*(ctx as *const SentEvent);
    if e.event != 0 {
        SetEvent(e.event);
    }
}

pub struct FileReader {
    pub req: TransferRequest,
    pub local: String,
    pub stream: Stream,
    pub offset: u64,
    pub chunk: u32,
}

impl FileReader {
    pub fn new(req: TransferRequest) -> Self {
        FileReader { req, local: String::new(), stream: Stream::closed(), offset: 0, chunk: MIN_CHUNK }
    }

    pub unsafe fn start(&mut self) -> HRESULT {
        self.local = match local_of(&self.req.src) {
            Ok(l) => l,
            Err(e) => return e,
        };
        let share = ((self.req.flags & XTF_SHARE_WRITE_FLAG) | 0x2000_0000) >> 29;
        let hr = self.stream.open(&self.local, GENERIC_READ, share, OPEN_EXISTING, FILE_FLAG_OVERLAPPED | FILE_FLAG_SEQUENTIAL_SCAN);
        if hr < 0 {
            return hr;
        }
        self.req.info.attributes = match file::get_file_attributes(&self.local) {
            Ok(a) => a,
            Err(e) => return e,
        };
        match self.stream.file_time() {
            Ok((c, a, w)) => {
                self.req.info.creation = c;
                self.req.info.access = a;
                self.req.info.write = w;
            }
            Err(e) => return e,
        }
        self.req.info.size = match self.stream.file_size() {
            Ok(s) => s,
            Err(e) => return e,
        };
        self.chunk = optimal_chunks(self.req.info.size).1;
        S_OK
    }

    pub unsafe fn chunk_json(&self, first: bool, offset: u64, msg: *mut c_void) -> HRESULT {
        let mut props = Com::null();
        let hr = XbtpCreateJsonObject(props.out());
        if hr < 0 {
            return hr;
        }
        if first {
            let hr = set_string(props.0, "SrcFileName", &self.req.src);
            if hr < 0 {
                return hr;
            }
            let hr = set_string(props.0, "DstFileName", &self.req.dst);
            if hr < 0 {
                return hr;
            }
            let hr = set_u32(props.0, "Flags", self.req.flags);
            if hr < 0 {
                return hr;
            }
            let hr = set_file_info_properties(props.0, &self.req.info);
            if hr < 0 {
                return hr;
            }
        }
        let hr = set_u64(props.0, "FileOffset", offset);
        if hr < 0 {
            return hr;
        }
        let mut w = Bstr::null();
        let hr = (vt::<JsonObjectVtbl>(props.0).stringify)(props.0, w.out());
        if hr < 0 {
            return hr;
        }
        (vt::<MessageImplVtbl>(msg).set_json_body_w)(msg, w.0)
    }

    pub unsafe fn process(&mut self, session: *mut c_void) -> HRESULT {
        if session.is_null() {
            return E_INVALIDARG;
        }
        let mut buf = vec![0u8; self.chunk as usize];
        let ev = SentEvent { event: CreateEventW(null_mut(), 0, 0, core::ptr::null()) };
        if ev.event == 0 {
            return last_hr();
        }
        let mut first = true;
        let hr = loop {
            let size = self.req.info.size;
            let n = (size - self.offset).min(self.chunk as u64) as u32;
            let at = self.offset;
            let done = match self.stream.read_at(&mut buf[..n as usize], at) {
                Ok(d) => d,
                Err(e) => break e,
            };
            self.offset += n as u64;
            if done != n {
                break HR_IO_DEVICE;
            }
            let mut msg = Com::null();
            let hr = XbtpCreateMessage(&IID_XBTP_MESSAGE_IMPL, msg.out());
            if hr < 0 {
                break hr;
            }
            let hr = (vt::<MessageImplVtbl>(msg.0).set_binary_body)(msg.0, n, buf.as_ptr());
            if hr < 0 {
                break hr;
            }
            let hr = self.chunk_json(first, at, msg.0);
            if hr < 0 {
                break hr;
            }
            let hr = (vt::<SessionVtbl>(session).send_more_message_async)(session, msg.0, Some(message_sent), &ev as *const SentEvent as *mut c_void);
            if hr < 0 {
                break hr;
            }
            WaitForSingleObject(ev.event, u32::MAX);
            first = false;
            if self.offset >= size {
                break S_OK;
            }
        };
        CloseHandle(ev.event);
        hr
    }

    pub unsafe fn execute(&mut self, session: *mut c_void) -> HRESULT {
        let mut hr = self.start();
        if hr >= 0 {
            hr = self.process(session);
        }
        let c = if self.stream.is_open() { self.stream.close_with_retry(false) } else { S_OK };
        if hr >= 0 && c < 0 {
            c
        } else {
            hr
        }
    }
}

pub unsafe extern "system" fn recv_file_handler(session: usize, request: *mut c_void, response: *mut c_void) -> HRESULT {
    let hr = match parse_transfer_request(request, false) {
        Ok(req) => FileReader::new(req).execute(session as *mut c_void),
        Err(e) => e,
    };
    serialize(&MsgResponse { result: hr }, response)
}
