use core::ffi::c_void;
use std::path::PathBuf;

use crate::wstr::{from_ptr, put_wide, wide};

pub const WINDOWS_DIR: &str = "C:\\Windows";
pub const CURRENT_DIR: &str = "C:\\Windows\\System32";
pub const ERROR_PATH_NOT_FOUND: u32 = 3;
pub const ERROR_FILE_NOT_FOUND: u32 = 2;
const INVALID_HANDLE_VALUE: isize = -1;
const FILE_RENAME_INFO: u32 = 3;
const FILE_RENAME_INFO_EX: u32 = 22;

#[link(name = "kernel32")]
extern "system" {
    fn CreateFileW(name: *const u16, access: u32, share: u32, sa: *const c_void, disposition: u32, flags: u32, template: isize) -> isize;
    fn GetFileAttributesW(name: *const u16) -> u32;
    fn DeleteFileW(name: *const u16) -> i32;
    fn SetFileInformationByHandle(h: isize, class: u32, info: *const u8, size: u32) -> i32;
    fn SetLastError(e: u32);
    fn GetFullPathNameW(name: *const u16, len: u32, buf: *mut u16, part: *mut *mut u16) -> u32;
}

pub fn fs_root() -> PathBuf {
    crate::root().join("fs")
}

pub fn drive_dir(letter: char) -> PathBuf {
    fs_root().join(letter.to_ascii_uppercase().to_string())
}

pub fn ensure_system_drive() {
    let _ = std::fs::create_dir_all(drive_dir('C').join("Windows").join("System32"));
}

pub const STATUS_OBJECT_NAME_INVALID: u32 = 0xC000_0033;
pub const STATUS_OBJECT_NAME_NOT_FOUND: u32 = 0xC000_0034;
pub const STATUS_OBJECT_PATH_NOT_FOUND: u32 = 0xC000_003A;
pub const STATUS_ACCESS_DENIED: u32 = 0xC000_0022;
pub const STATUS_BAD_NETWORK_PATH: u32 = 0xC000_00BE;
pub const ERROR_INVALID_NAME: u32 = 123;
const ERROR_ACCESS_DENIED: u32 = 5;
const ERROR_BAD_NETPATH: u32 = 53;
const ERROR_INVALID_PARAMETER: u32 = 87;

pub fn win32_error(status: u32) -> u32 {
    match status {
        STATUS_OBJECT_NAME_INVALID => ERROR_INVALID_NAME,
        STATUS_OBJECT_NAME_NOT_FOUND => ERROR_FILE_NOT_FOUND,
        STATUS_ACCESS_DENIED => ERROR_ACCESS_DENIED,
        STATUS_BAD_NETWORK_PATH => ERROR_BAD_NETPATH,
        _ => ERROR_PATH_NOT_FOUND,
    }
}

pub fn resolve(console: &str) -> Option<(String, PathBuf)> {
    try_resolve(console).ok()
}

pub(crate) fn full_path_name(path: &str) -> Option<String> {
    let w = wide(path);
    let mut cap = 512usize;
    loop {
        let mut buf = vec![0u16; cap];
        let n = unsafe { GetFullPathNameW(w.as_ptr(), cap as u32, buf.as_mut_ptr(), core::ptr::null_mut()) } as usize;
        if n == 0 {
            return None;
        }
        if n < cap {
            return Some(String::from_utf16_lossy(&buf[..n]));
        }
        cap = n + 1;
    }
}

fn drive_of(s: &str) -> Option<char> {
    let b = s.as_bytes();
    (b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic()).then(|| (b[0] as char).to_ascii_uppercase())
}

fn bad_char(c: char) -> bool {
    (c as u32) < 32 || matches!(c, '"' | '*' | '<' | '>' | '?' | '|' | '/')
}

fn check_component(c: &str, last: bool) -> Result<(), u32> {
    if c.chars().any(bad_char) {
        return Err(STATUS_OBJECT_NAME_INVALID);
    }
    if !c.contains(':') {
        return Ok(());
    }
    if !last {
        return Err(STATUS_OBJECT_NAME_INVALID);
    }
    let parts: Vec<&str> = c.split(':').collect();
    let ok = match parts.as_slice() {
        [name, stream] => !name.is_empty() && !stream.is_empty(),
        [name, stream, kind] => !name.is_empty() && kind.len() > 1 && kind.starts_with('$') && (!stream.is_empty() || kind.eq_ignore_ascii_case("$DATA") || kind.eq_ignore_ascii_case("$INDEX_ALLOCATION")),
        _ => false,
    };
    if ok {
        Ok(())
    } else {
        Err(STATUS_OBJECT_NAME_INVALID)
    }
}

fn verbatim_components(rest: &str) -> Result<Vec<String>, u32> {
    let body = rest.strip_prefix('\\').unwrap_or(rest);
    if body.is_empty() {
        return Ok(Vec::new());
    }
    let body = body.strip_suffix('\\').unwrap_or(body);
    let parts: Vec<&str> = body.split('\\').collect();
    let n = parts.len();
    let mut out = Vec::with_capacity(n);
    for (i, c) in parts.into_iter().enumerate() {
        match c {
            "" | ".." => return Err(STATUS_OBJECT_NAME_INVALID),
            "." => return Err(if i + 1 == n { STATUS_OBJECT_NAME_NOT_FOUND } else { STATUS_OBJECT_PATH_NOT_FOUND }),
            c => out.push(c.to_string()),
        }
    }
    Ok(out)
}

fn normalized_components(drive: char, rest: &str) -> Result<Vec<String>, u32> {
    let full = full_path_name(&format!("{drive}:{rest}")).ok_or(STATUS_OBJECT_NAME_INVALID)?;
    if drive_of(&full).is_none() || !full[2..].starts_with('\\') {
        return Err(STATUS_OBJECT_NAME_INVALID);
    }
    Ok(full[3..].split('\\').filter(|c| !c.is_empty()).map(|c| c.to_string()).collect())
}

pub fn try_resolve(console: &str) -> Result<(String, PathBuf), u32> {
    let s = console.replace('/', "\\");
    if s.is_empty() {
        return Err(STATUS_OBJECT_NAME_INVALID);
    }
    let (drive, comps) = if let Some(r) = s.strip_prefix("\\\\?\\").or_else(|| s.strip_prefix("\\??\\")) {
        if r.as_bytes().get(..4).is_some_and(|p| p.eq_ignore_ascii_case(b"UNC\\")) {
            return Err(STATUS_BAD_NETWORK_PATH);
        }
        let d = drive_of(r).ok_or(STATUS_OBJECT_PATH_NOT_FOUND)?;
        let rest = &r[2..];
        if rest.is_empty() {
            return Err(STATUS_ACCESS_DENIED);
        }
        if !rest.starts_with('\\') {
            return Err(STATUS_OBJECT_NAME_INVALID);
        }
        (d, verbatim_components(rest)?)
    } else if let Some(r) = s.strip_prefix("\\\\.\\") {
        drive_of(r).ok_or(STATUS_OBJECT_NAME_NOT_FOUND)?;
        let full = full_path_name(&s).ok_or(STATUS_OBJECT_NAME_INVALID)?;
        let fr = full.get(4..).unwrap_or("");
        let d = drive_of(fr).ok_or(STATUS_OBJECT_NAME_NOT_FOUND)?;
        let rest = &fr[2..];
        if rest.is_empty() {
            return Err(STATUS_ACCESS_DENIED);
        }
        if !rest.starts_with('\\') {
            return Err(STATUS_OBJECT_NAME_NOT_FOUND);
        }
        (d, normalized_components(d, rest)?)
    } else if s.starts_with("\\\\") {
        return Err(STATUS_BAD_NETWORK_PATH);
    } else if let Some(d) = drive_of(&s) {
        let rest = &s[2..];
        let rest = if rest.starts_with('\\') {
            rest.to_string()
        } else if d == 'C' {
            format!("{}\\{rest}", &CURRENT_DIR[2..])
        } else {
            format!("\\{rest}")
        };
        (d, normalized_components(d, &rest)?)
    } else if s.starts_with('\\') {
        ('C', normalized_components('C', &s)?)
    } else {
        ('C', normalized_components('C', &format!("{}\\{s}", &CURRENT_DIR[2..]))?)
    };
    let n = comps.len();
    for (i, c) in comps.iter().enumerate() {
        check_component(c, i + 1 == n)?;
    }
    let mut host = drive_dir(drive).to_string_lossy().trim_end_matches('\\').to_string();
    for c in &comps {
        host.push('\\');
        host.push_str(c);
    }
    if full_path_name(&host).as_deref() != Some(host.as_str()) {
        return Err(STATUS_OBJECT_NAME_INVALID);
    }
    Ok((format!("{drive}:\\{}", comps.join("\\")), PathBuf::from(host)))
}

pub fn verbatim(host: &std::path::Path) -> Vec<u16> {
    let s = host.to_string_lossy();
    if s.starts_with("\\\\?\\") {
        wide(&s)
    } else if let Some(u) = s.strip_prefix("\\\\") {
        wide(&format!("\\\\?\\UNC\\{u}"))
    } else {
        wide(&format!("\\\\?\\{s}"))
    }
}

fn host_wide(console: *const u16) -> Result<(String, Vec<u16>), u32> {
    if console.is_null() {
        return Err(STATUS_OBJECT_NAME_INVALID);
    }
    let (c, h) = try_resolve(&unsafe { from_ptr(console) })?;
    Ok((c, verbatim(&h)))
}

pub unsafe extern "system" fn create_file_w(name: *const u16, access: u32, share: u32, sa: *const c_void, disposition: u32, flags: u32, template: isize) -> isize {
    if !name.is_null() && crate::logan::is_device_path(&from_ptr(name)) {
        return crate::logan::open();
    }
    if !name.is_null() && crate::vab::is_device_path(&from_ptr(name)) {
        return crate::vab::open();
    }
    match host_wide(name) {
        Ok((_, h)) => CreateFileW(h.as_ptr(), access, share, sa, disposition, flags, template),
        Err(st) => {
            SetLastError(win32_error(st));
            INVALID_HANDLE_VALUE
        }
    }
}

pub unsafe extern "system" fn get_file_attributes_w(name: *const u16) -> u32 {
    match host_wide(name) {
        Ok((_, h)) => GetFileAttributesW(h.as_ptr()),
        Err(st) => {
            SetLastError(win32_error(st));
            u32::MAX
        }
    }
}

pub unsafe extern "system" fn delete_file_w(name: *const u16) -> i32 {
    match host_wide(name) {
        Ok((_, h)) => DeleteFileW(h.as_ptr()),
        Err(st) => {
            SetLastError(win32_error(st));
            0
        }
    }
}

pub unsafe extern "system" fn set_file_information_by_handle(h: isize, class: u32, info: *const u8, size: u32) -> i32 {
    if (class != FILE_RENAME_INFO && class != FILE_RENAME_INFO_EX) || info.is_null() || size < 20 {
        return SetFileInformationByHandle(h, class, info, size);
    }
    let bytes = (info.add(16) as *const u32).read_unaligned() as usize;
    if bytes % 2 != 0 || 20 + bytes > size as usize {
        SetLastError(ERROR_INVALID_PARAMETER);
        return 0;
    }
    let name: Vec<u16> = std::slice::from_raw_parts(info.add(20) as *const u16, bytes / 2).to_vec();
    let text = String::from_utf16_lossy(&name);
    let root_dir = (info.add(8) as *const isize).read_unaligned();
    if root_dir != 0 || (!text.contains(['\\', '/']) && drive_of(&text).is_none()) {
        return SetFileInformationByHandle(h, class, info, size);
    }
    let host = match try_resolve(&text) {
        Ok((_, host)) => host,
        Err(st) => {
            SetLastError(win32_error(st));
            return 0;
        }
    };
    let mut hw = verbatim(&host);
    hw.pop();
    let mut buf = vec![0u8; 20 + hw.len() * 2 + 2];
    buf[..16].copy_from_slice(std::slice::from_raw_parts(info, 16));
    buf[16..20].copy_from_slice(&((hw.len() * 2) as u32).to_le_bytes());
    for (i, c) in hw.iter().enumerate() {
        buf[20 + i * 2..22 + i * 2].copy_from_slice(&c.to_le_bytes());
    }
    let mut aligned = vec![0u64; buf.len().div_ceil(8)];
    core::ptr::copy_nonoverlapping(buf.as_ptr(), aligned.as_mut_ptr() as *mut u8, buf.len());
    SetFileInformationByHandle(h, class, aligned.as_ptr() as *const u8, buf.len() as u32)
}

pub unsafe extern "system" fn get_drive_type_w(root: *const u16) -> u32 {
    let r = if root.is_null() { "C:\\".to_string() } else { from_ptr(root) };
    let b = r.as_bytes();
    if b.len() < 2 || b[1] != b':' || !(b.len() == 2 || (b[2] == b'\\' && b.len() == 3)) {
        return 1;
    }
    if drive_dir(b[0] as char).is_dir() {
        3
    } else {
        1
    }
}

pub unsafe extern "system" fn get_system_windows_directory_w(buf: *mut u16, size: u32) -> u32 {
    let w: Vec<u16> = WINDOWS_DIR.encode_utf16().collect();
    if buf.is_null() || (size as usize) < w.len() + 1 {
        return w.len() as u32 + 1;
    }
    put_wide(buf, &w);
    w.len() as u32
}

pub unsafe extern "system" fn search_path_w(path: *const u16, file: *const u16, ext: *const u16, len: u32, buf: *mut u16, part: *mut *mut u16) -> u32 {
    let f = from_ptr(file);
    if f.is_empty() {
        SetLastError(87);
        return 0;
    }
    let e = from_ptr(ext);
    let names: Vec<String> = if !e.is_empty() && !f.rsplit('\\').next().unwrap_or("").contains('.') { vec![format!("{f}{e}")] } else { vec![f.clone()] };
    let has_dir = f.contains('\\') || f.contains(':');
    let dirs: Vec<String> = if has_dir {
        vec![String::new()]
    } else if !path.is_null() {
        from_ptr(path).split(';').filter(|d| !d.is_empty()).map(|d| d.to_string()).collect()
    } else {
        vec![CURRENT_DIR.to_string(), WINDOWS_DIR.to_string()]
    };
    for d in &dirs {
        for n in &names {
            let full = if d.is_empty() { n.clone() } else { format!("{}\\{n}", d.trim_end_matches('\\')) };
            if let Some((console, host)) = resolve(&full) {
                if host.is_file() || host.is_dir() {
                    let w: Vec<u16> = console.encode_utf16().collect();
                    if buf.is_null() || (len as usize) < w.len() + 1 {
                        return w.len() as u32 + 1;
                    }
                    put_wide(buf, &w);
                    if !part.is_null() {
                        let slash = w.iter().rposition(|&c| c == b'\\' as u16).map(|i| i + 1).unwrap_or(0);
                        *part = buf.add(slash);
                    }
                    return w.len() as u32;
                }
            }
        }
    }
    SetLastError(ERROR_FILE_NOT_FOUND);
    0
}
