use crate::com::*;
use crate::file::{self, FileInfo, Win32FindDataW, FILE_ATTRIBUTE_DIRECTORY};
use crate::handlers::{local_path, HasFileName};
use crate::msg::*;
use crate::paths::*;
use core::ffi::c_void;

pub const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
pub const IO_REPARSE_TAG_SYMLINK: u32 = 0xA000_000C;
const FIND_FIRST_EX_LARGE_FETCH: u32 = 2;
const INVALID_HANDLE_VALUE: isize = -1;

#[repr(C)]
#[derive(Default)]
struct ByHandleFileInformation {
    attributes: u32,
    creation: [u32; 2],
    access: [u32; 2],
    write: [u32; 2],
    volume_serial: u32,
    size_high: u32,
    size_low: u32,
    links: u32,
    index_high: u32,
    index_low: u32,
}

#[link(name = "kernel32")]
extern "system" {
    fn FindFirstFileExW(name: *const u16, level: u32, data: *mut Win32FindDataW, op: u32, filter: *mut c_void, flags: u32) -> isize;
    fn FindNextFileW(h: isize, data: *mut Win32FindDataW) -> i32;
    fn FindClose(h: isize) -> i32;
    fn CreateFileW(name: *const u16, access: u32, share: u32, sa: *mut c_void, disp: u32, flags: u32, tmpl: isize) -> isize;
    fn GetFileInformationByHandle(h: isize, info: *mut ByHandleFileInformation) -> i32;
    fn CloseHandle(h: isize) -> i32;
    fn Sleep(ms: u32);
}

#[derive(Default)]
pub struct MsgFindFilesInfo {
    pub root_directory: Bstr,
    pub search_pattern: Bstr,
}

impl Props for MsgFindFilesInfo {
    unsafe fn deserialize_properties(&mut self, props: *mut c_void) -> HRESULT {
        let hr = get_string(props, "RootDirectory", &mut self.root_directory);
        if hr < 0 {
            return hr;
        }
        get_string(props, "SearchPattern", &mut self.search_pattern)
    }
    unsafe fn serialize_properties(&self, props: *mut c_void) -> HRESULT {
        let hr = set_string(props, "RootDirectory", &self.root_directory);
        if hr < 0 {
            return hr;
        }
        set_string(props, "SearchPattern", &self.search_pattern)
    }
}

pub unsafe fn get_file_info_properties(props: *mut c_void, info: &mut FileInfo) -> HRESULT {
    let hr = get_u32(props, "FileAttributes", &mut info.attributes);
    if hr < 0 {
        return hr;
    }
    let hr = get_u64(props, "CreationTime", &mut info.creation);
    if hr < 0 {
        return hr;
    }
    let hr = get_u64(props, "LastAccessTime", &mut info.access);
    if hr < 0 {
        return hr;
    }
    let hr = get_u64(props, "LastWriteTime", &mut info.write);
    if hr < 0 {
        return hr;
    }
    get_u64(props, "FileSize", &mut info.size)
}

pub unsafe fn set_file_info_properties(props: *mut c_void, info: &FileInfo) -> HRESULT {
    let hr = set_u32(props, "FileAttributes", info.attributes);
    if hr < 0 {
        return hr;
    }
    let hr = set_u64(props, "CreationTime", info.creation);
    if hr < 0 {
        return hr;
    }
    let hr = set_u64(props, "LastAccessTime", info.access);
    if hr < 0 {
        return hr;
    }
    let hr = set_u64(props, "LastWriteTime", info.write);
    if hr < 0 {
        return hr;
    }
    set_u64(props, "FileSize", info.size)
}

#[derive(Default)]
pub struct MsgFindFilesEntry {
    pub file_name: Bstr,
    pub info: FileInfo,
}

impl Props for MsgFindFilesEntry {
    unsafe fn deserialize_properties(&mut self, props: *mut c_void) -> HRESULT {
        let hr = get_string(props, "FileName", &mut self.file_name);
        if hr < 0 {
            return hr;
        }
        get_file_info_properties(props, &mut self.info)
    }
    unsafe fn serialize_properties(&self, props: *mut c_void) -> HRESULT {
        let hr = set_string(props, "FileName", &self.file_name);
        if hr < 0 {
            return hr;
        }
        set_file_info_properties(props, &self.info)
    }
}

pub fn create_path(path: &str, path_type: u32) -> Result<String, HRESULT> {
    if path.is_empty() {
        return Err(E_INVALIDARG);
    }
    for bad in ["..\\", "\\..", "\\\\"] {
        if path.contains(bad) {
            return Err(E_INVALIDARG);
        }
    }
    let full = full_path_name(path)?;
    let mut ch: Vec<char> = full.chars().collect();
    if let Some(c) = ch.first_mut() {
        *c = c.to_ascii_uppercase();
    }
    let full: String = ch.into_iter().collect();
    match path_type {
        PATH_NORMAL => Ok(full),
        PATH_CONSOLE => Ok(format!("X{full}")),
        _ => Err(E_INVALIDARG),
    }
}

pub fn append_path(cur: &str, part: Option<&str>) -> String {
    let mut out = String::from(cur);
    if let Some(p) = part {
        let p = p.trim_start_matches('\\');
        if !p.is_empty() {
            if !out.is_empty() && !out.ends_with('\\') {
                out.push('\\');
            }
            out.push_str(p);
        }
    }
    out
}

pub struct FoundFile {
    pub file_name: String,
    pub info: FileInfo,
}

pub trait FindCallback {
    fn on_found_file(&mut self, root: &str, pattern: Option<&str>, found: &FoundFile) -> HRESULT;
}

struct Enumerator {
    handle: isize,
}

pub const ENUM_MORE: HRESULT = 0;
pub const ENUM_DONE: HRESULT = 1;

impl Enumerator {
    fn new() -> Self {
        Enumerator { handle: INVALID_HANDLE_VALUE }
    }

    fn reset(&mut self) {
        if self.handle != INVALID_HANDLE_VALUE {
            unsafe { FindClose(self.handle) };
            self.handle = INVALID_HANDLE_VALUE;
        }
    }

    fn next(&mut self, search: &str, data: &mut Win32FindDataW) -> HRESULT {
        let mut attempt = 0;
        loop {
            attempt += 1;
            if self.handle == INVALID_HANDLE_VALUE {
                let h = match crate::paths::host_search_path(search) {
                    Ok(p) => unsafe { FindFirstFileExW(p.wide.as_ptr(), 1, data, 0, core::ptr::null_mut(), FIND_FIRST_EX_LARGE_FETCH) },
                    Err(e) => {
                        unsafe { file::SetLastError(e) };
                        INVALID_HANDLE_VALUE
                    }
                };
                self.handle = h;
                if h == INVALID_HANDLE_VALUE {
                    let e = unsafe { file::GetLastError() };
                    match e {
                        2 | 5 => return ENUM_DONE,
                        6 | 0x40 if attempt <= 3 => {
                            unsafe { Sleep(100) };
                            continue;
                        }
                        _ => return hr_from_win32(e),
                    }
                }
            } else if unsafe { FindNextFileW(self.handle, data) } == 0 {
                return ENUM_DONE;
            }
            if data.attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
                let n = from_find_name(data);
                if n == "." || n == ".." {
                    continue;
                }
            }
            return ENUM_MORE;
        }
    }
}

impl Drop for Enumerator {
    fn drop(&mut self) {
        self.reset();
    }
}

fn from_find_name(d: &Win32FindDataW) -> String {
    let n = d.file_name.iter().position(|&c| c == 0).unwrap_or(d.file_name.len());
    String::from_utf16_lossy(&d.file_name[..n])
}

fn ft(v: [u32; 2]) -> u64 {
    ((v[1] as u64) << 32) | v[0] as u64
}

fn symlink_target_info(name: &str, info: &mut FileInfo) {
    let Ok(p) = host_path(name) else { return };
    let h = unsafe { CreateFileW(p.wide.as_ptr(), 0x8000_0000, 1, core::ptr::null_mut(), 3, 0x80, 0) };
    if h == INVALID_HANDLE_VALUE {
        return;
    }
    let mut bh = ByHandleFileInformation::default();
    let ok = unsafe { GetFileInformationByHandle(h, &mut bh) };
    unsafe { CloseHandle(h) };
    if ok != 0 {
        info.creation = ft(bh.creation);
        info.access = ft(bh.access);
        info.write = ft(bh.write);
        info.size = ((bh.size_high as u64) << 32) | bh.size_low as u64;
    }
}

#[allow(clippy::too_many_arguments)]
pub fn find_files_recursive(
    root: &str,
    cur: &str,
    pattern: Option<&str>,
    include: u32,
    exclude: u32,
    recursion: u32,
    cb: &mut dyn FindCallback,
    mut counter: Option<&mut u32>,
) -> HRESULT {
    let mut hr = 'body: {
        let search = append_path(cur, pattern);
        let mut en = Enumerator::new();
        let mut wfd = Win32FindDataW::default();
        loop {
            let r = en.next(&search, &mut wfd);
            if r < 0 {
                break 'body r;
            }
            if r == ENUM_DONE {
                break;
            }
            if include != 0 && wfd.attributes & include == 0 {
                continue;
            }
            if wfd.attributes & exclude != 0 {
                continue;
            }
            let name = append_path(cur, Some(&from_find_name(&wfd)));
            let mut info = FileInfo {
                attributes: wfd.attributes,
                creation: ft(wfd.creation),
                access: ft(wfd.access),
                write: ft(wfd.write),
                size: ((wfd.size_high as u64) << 32) | wfd.size_low as u64,
            };
            if wfd.attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 && wfd.reserved0 == IO_REPARSE_TAG_SYMLINK {
                symlink_target_info(&name, &mut info);
            }
            if let Some(c) = counter.as_deref_mut() {
                *c += 1;
            }
            let r = cb.on_found_file(root, pattern, &FoundFile { file_name: name, info });
            if r < 0 {
                break 'body r;
            }
        }
        if recursion == 0 {
            return S_OK;
        }
        let search = append_path(cur, Some("*"));
        en.reset();
        loop {
            let r = en.next(&search, &mut wfd);
            if r < 0 {
                break 'body r;
            }
            if r == ENUM_DONE {
                return S_OK;
            }
            if wfd.attributes & FILE_ATTRIBUTE_DIRECTORY == 0 {
                continue;
            }
            let sub = append_path(cur, Some(&from_find_name(&wfd)));
            let mut r = find_files_recursive(root, &sub, pattern, include, exclude, recursion - 1, cb, counter.as_deref_mut());
            if r == HR_FILE_NOT_FOUND && wfd.attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                r = S_OK;
            }
            if r < 0 {
                break 'body r;
            }
        }
    };
    if hr == HR_PATH_NOT_FOUND {
        hr = HR_FILE_NOT_FOUND;
    }
    hr
}

pub fn find_files(search: &str, include: u32, exclude: u32, recursion: u32, cb: &mut dyn FindCallback) -> HRESULT {
    let mut root = match full_path_name(search) {
        Ok(r) => r,
        Err(e) => return if e == HR_PATH_NOT_FOUND { HR_FILE_NOT_FOUND } else { e },
    };
    let attrs = file::get_attributes_raw(&root);
    let split = if attrs == file::INVALID_FILE_ATTRIBUTES {
        let e = unsafe { file::GetLastError() };
        if e == 0x7b || (e == 2 && recursion != 0) {
            true
        } else if e == 3 {
            let first = root.chars().next().unwrap_or('\0');
            let w = wide(&format!("{first}:\\"));
            let e = if unsafe { console_sandbox::fs::get_drive_type_w(w.as_ptr()) } <= 1 { 0xf } else { 3 };
            let hr = hr_from_win32(e);
            return if hr == HR_PATH_NOT_FOUND { HR_FILE_NOT_FOUND } else { hr };
        } else if e == 2 {
            return HR_FILE_NOT_FOUND;
        } else if e != 0 {
            let hr = hr_from_win32(e);
            return if hr == HR_PATH_NOT_FOUND { HR_FILE_NOT_FOUND } else { hr };
        } else {
            false
        }
    } else {
        attrs & FILE_ATTRIBUTE_DIRECTORY == 0
    };
    let pattern: Option<String> = if split {
        match root.rfind('\\') {
            None => Some("*".to_string()),
            Some(p) => {
                let tail = root[p + 1..].to_string();
                let mut chars: Vec<char> = root[..=p].chars().collect();
                let mut i = chars.len() - 1;
                while chars[i] == '\\' && i > 2 {
                    chars.truncate(i);
                    i -= 1;
                }
                root = chars.into_iter().collect();
                if tail.is_empty() {
                    None
                } else {
                    Some(tail)
                }
            }
        }
    } else {
        Some("*".to_string())
    };
    let hr = find_files_recursive(&root, &root, pattern.as_deref(), include, exclude, recursion, cb, None);
    if hr == HR_PATH_NOT_FOUND {
        HR_FILE_NOT_FOUND
    } else {
        hr
    }
}

pub struct SessionFindCallback {
    pub session: *mut c_void,
    pub path_type: u32,
    pub first: bool,
}

pub unsafe fn send_more_message_ptr(session: *mut c_void, message: *mut c_void) -> HRESULT {
    if session.is_null() {
        return E_INVALIDARG;
    }
    (vt::<SessionVtbl>(session).send_more_message)(session, message)
}

pub unsafe fn send_more_message<T: Props>(session: *mut c_void, m: &T) -> HRESULT {
    let mut msg = Com::null();
    let hr = XbtpCreateMessage(&IID_XBTP_MESSAGE_IMPL, msg.out());
    if hr < 0 {
        return hr;
    }
    let hr = serialize_impl(m, msg.0);
    if hr < 0 {
        return hr;
    }
    send_more_message_ptr(session, msg.0)
}

fn bstr_opt(s: Option<&str>) -> Result<Bstr, HRESULT> {
    match s {
        Some(v) if !v.is_empty() => Bstr::from_str(v),
        _ => Ok(Bstr::null()),
    }
}

impl FindCallback for SessionFindCallback {
    fn on_found_file(&mut self, root: &str, pattern: Option<&str>, found: &FoundFile) -> HRESULT {
        unsafe {
            if self.first {
                self.first = false;
                let root_dir = match create_path(root, self.path_type).and_then(|p| Bstr::from_str(&p)) {
                    Ok(b) => b,
                    Err(e) => return e,
                };
                let search_pattern = match bstr_opt(pattern) {
                    Ok(b) => b,
                    Err(e) => return e,
                };
                let hr = send_more_message(self.session, &MsgFindFilesInfo { root_directory: root_dir, search_pattern });
                if hr < 0 {
                    return hr;
                }
            }
            let file_name = match create_path(&found.file_name, self.path_type).and_then(|p| Bstr::from_str(&p)) {
                Ok(b) => b,
                Err(e) => return e,
            };
            send_more_message(self.session, &MsgFindFilesEntry { file_name, info: found.info })
        }
    }
}

pub unsafe extern "system" fn find_files_handler(session: usize, request: *mut c_void, response: *mut c_void) -> HRESULT {
    let mut req = MsgFindFiles::default();
    let hr = 'op: {
        let hr = deserialize(&mut req, request);
        if hr < 0 {
            break 'op hr;
        }
        let (t, rel) = match local_path(req.file_name()) {
            Ok(v) => v,
            Err(e) => break 'op e,
        };
        if t == PATH_APPLICATION {
            break 'op HR_FILE_NOT_FOUND;
        }
        let full = match full_path_name(&rel) {
            Ok(f) => f,
            Err(e) => break 'op e,
        };
        let mut cb = SessionFindCallback { session: session as *mut c_void, path_type: t, first: true };
        find_files(&full, req.include, req.exclude, req.recursion, &mut cb)
    };
    serialize(&MsgResponse { result: hr }, response)
}

fn upper_unit(u: u16) -> u16 {
    match char::from_u32(u as u32) {
        Some(c) => {
            let mut it = c.to_uppercase();
            match (it.next(), it.next()) {
                (Some(x), None) if (x as u32) < 0x10000 => x as u16,
                _ => u,
            }
        }
        None => u,
    }
}

pub fn relative_file_name(root: &str, path: &str) -> Result<String, HRESULT> {
    let r: Vec<u16> = root.encode_utf16().collect();
    let p: Vec<u16> = path.encode_utf16().collect();
    if p.len() < r.len() || r.iter().zip(p.iter()).any(|(a, b)| upper_unit(*a) != upper_unit(*b)) {
        return Err(E_UNEXPECTED);
    }
    let mut i = r.len();
    while i < p.len() && p[i] == b'\\' as u16 {
        i += 1;
    }
    Ok(String::from_utf16_lossy(&p[i..]))
}
