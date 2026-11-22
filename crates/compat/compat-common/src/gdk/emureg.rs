use std::path::{Path, PathBuf};

pub const REG_DWORD: u32 = 4;
pub const REG_QWORD: u32 = 11;
pub const ERROR_FILE_NOT_FOUND: u32 = 2;
pub const ERROR_ACCESS_DENIED: u32 = 5;
pub const ERROR_INVALID_DATA: u32 = 13;
pub const ERROR_INVALID_PARAMETER: u32 = 87;
pub const ERROR_KEY_HAS_CHILDREN: u32 = 1020;

pub const FAMILIES: &str = "Software\\Classes\\Local Settings\\Software\\Microsoft\\Windows\\CurrentVersion\\AppModel\\Repository\\Families";
pub const PASSWORDLESS_ROOT: &str = "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\PasswordLess";
pub const IMAGE_CUSTOMIZATION_KEY: &str = "SYSTEM\\CurrentControlSet\\Control\\Cryptography\\PwdlessPlat\\ImageCustomization";
pub const LOADED_SYSTEM_HIVES: [&str; 4] = [".DEFAULT", "S-1-5-18", "S-1-5-19", "S-1-5-20"];

#[link(name = "advapi32")]
extern "system" {
    fn GetTokenInformation(token: isize, class: u32, buf: *mut u8, len: u32, ret: *mut u32) -> i32;
    fn ConvertSidToStringSidW(sid: *const u8, out: *mut *mut u16) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn LocalFree(p: *mut core::ffi::c_void) -> *mut core::ffi::c_void;
}

const CURRENT_THREAD_EFFECTIVE_TOKEN: isize = -6;

pub unsafe fn sid_to_string(sid: *const u8) -> Option<String> {
    let mut s: *mut u16 = core::ptr::null_mut();
    if sid.is_null() || ConvertSidToStringSidW(sid, &mut s) == 0 {
        return None;
    }
    let mut n = 0;
    while *s.add(n) != 0 {
        n += 1;
    }
    let out = String::from_utf16_lossy(core::slice::from_raw_parts(s, n));
    LocalFree(s as *mut _);
    Some(out)
}

pub unsafe fn token_user(token: isize) -> Option<String> {
    let mut buf = [0u64; 64];
    let mut ret = 0u32;
    if GetTokenInformation(token, 1, buf.as_mut_ptr() as *mut u8, 512, &mut ret) == 0 {
        return None;
    }
    sid_to_string(buf[0] as *const u8)
}

pub fn effective_user() -> Option<String> {
    unsafe { token_user(CURRENT_THREAD_EFFECTIVE_TOKEN) }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Hive {
    Hklm,
    Hku,
}

fn hive_dir(root: &Path, hive: Hive) -> PathBuf {
    root.join(match hive {
        Hive::Hklm => "hklm",
        Hive::Hku => "hku",
    })
}

pub fn components(sub: &str) -> Option<Vec<&str>> {
    let parts: Vec<&str> = sub.split('\\').filter(|c| !c.is_empty()).collect();
    let bad = |c: &str| c == "." || c == ".." || c.ends_with(['.', ' ']) || c.chars().any(|ch| (ch as u32) < 32 || matches!(ch, '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|'));
    if parts.iter().any(|c| bad(c)) {
        return None;
    }
    Some(parts)
}

fn under(path: &[&str], prefix: &str) -> bool {
    let p: Vec<&str> = prefix.split('\\').collect();
    path.len() >= p.len() && path.iter().zip(p.iter()).all(|(a, b)| a.eq_ignore_ascii_case(b))
}

pub fn is_emulated(hive: Hive, sub: &str) -> bool {
    let Some(c) = components(sub) else { return false };
    match hive {
        Hive::Hklm => under(&c, PASSWORDLESS_ROOT) || under(&c, IMAGE_CUSTOMIZATION_KEY),
        Hive::Hku => c.len() >= 2 && under(&c[1..], FAMILIES),
    }
}

pub fn key_dir(root: &Path, hive: Hive, sub: &str) -> Option<PathBuf> {
    Some(components(sub)?.into_iter().fold(hive_dir(root, hive), |a, c| a.join(c)))
}

fn value_name_ok(name: &str) -> bool {
    !name.chars().any(|ch| (ch as u32) < 32 || matches!(ch, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|')) && !name.ends_with(['.', ' '])
}

fn io_error(e: &std::io::Error) -> u32 {
    match e.raw_os_error() {
        Some(c) if c > 0 => c as u32,
        _ => 0x1f,
    }
}

pub fn key_exists(root: &Path, hive: Hive, sub: &str) -> bool {
    key_dir(root, hive, sub).map(|d| d.is_dir()).unwrap_or(false)
}

pub fn user_hive_loaded(root: &Path, sid: &str, current_user: Option<&str>) -> bool {
    LOADED_SYSTEM_HIVES.iter().any(|s| s.eq_ignore_ascii_case(sid)) || current_user.map(|u| u.eq_ignore_ascii_case(sid)).unwrap_or(false) || key_exists(root, Hive::Hku, sid)
}

pub fn create_key(root: &Path, hive: Hive, sub: &str) -> Result<bool, u32> {
    let dir = key_dir(root, hive, sub).ok_or(ERROR_INVALID_PARAMETER)?;
    if dir.is_dir() {
        return Ok(false);
    }
    std::fs::create_dir_all(&dir).map_err(|e| io_error(&e))?;
    Ok(true)
}

pub fn delete_key(root: &Path, hive: Hive, sub: &str) -> Result<(), u32> {
    let dir = key_dir(root, hive, sub).ok_or(ERROR_INVALID_PARAMETER)?;
    if !dir.is_dir() {
        return Err(ERROR_FILE_NOT_FOUND);
    }
    if !subkeys(root, hive, sub).is_empty() {
        return Err(ERROR_KEY_HAS_CHILDREN);
    }
    std::fs::remove_dir_all(&dir).map_err(|e| io_error(&e))
}

pub fn subkeys(root: &Path, hive: Hive, sub: &str) -> Vec<String> {
    let Some(dir) = key_dir(root, hive, sub) else { return Vec::new() };
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut v: Vec<String> = rd.filter_map(|e| e.ok()).filter(|e| e.path().is_dir()).map(|e| e.file_name().to_string_lossy().to_string()).collect();
    v.sort_by_key(|s| s.to_ascii_lowercase());
    v
}

fn value_file(root: &Path, hive: Hive, sub: &str, name: &str, ext: &str) -> Result<PathBuf, u32> {
    if !value_name_ok(name) {
        return Err(ERROR_INVALID_PARAMETER);
    }
    let dir = key_dir(root, hive, sub).ok_or(ERROR_INVALID_PARAMETER)?;
    if !dir.is_dir() {
        return Err(ERROR_FILE_NOT_FOUND);
    }
    Ok(dir.join(format!("{name}.{ext}")))
}

pub fn set_value(root: &Path, hive: Hive, sub: &str, name: &str, kind: u32, data: &[u8]) -> Result<(), u32> {
    let (ext, text) = match (kind, data.len()) {
        (REG_DWORD, 4) => ("dword", u32::from_le_bytes(data.try_into().unwrap_or_default()).to_string()),
        (REG_QWORD, 8) => ("qword", u64::from_le_bytes(data.try_into().unwrap_or_default()).to_string()),
        _ => return Err(ERROR_INVALID_PARAMETER),
    };
    let other = if ext == "dword" { "qword" } else { "dword" };
    let f = value_file(root, hive, sub, name, ext)?;
    let _ = std::fs::remove_file(value_file(root, hive, sub, name, other)?);
    std::fs::write(f, format!("{text}\n")).map_err(|e| io_error(&e))
}

pub fn get_value(root: &Path, hive: Hive, sub: &str, name: &str) -> Result<(u32, Vec<u8>), u32> {
    for (ext, kind) in [("dword", REG_DWORD), ("qword", REG_QWORD)] {
        let f = value_file(root, hive, sub, name, ext)?;
        match std::fs::read_to_string(&f) {
            Ok(t) => {
                let t = t.trim();
                return match kind {
                    REG_DWORD => t.parse::<u32>().map(|v| (kind, v.to_le_bytes().to_vec())).map_err(|_| ERROR_INVALID_DATA),
                    _ => t.parse::<u64>().map(|v| (kind, v.to_le_bytes().to_vec())).map_err(|_| ERROR_INVALID_DATA),
                };
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io_error(&e)),
        }
    }
    Err(ERROR_FILE_NOT_FOUND)
}

fn security_file(root: &Path, hive: Hive, sub: &str) -> Result<PathBuf, u32> {
    let dir = key_dir(root, hive, sub).ok_or(ERROR_INVALID_PARAMETER)?;
    if !dir.is_dir() {
        return Err(ERROR_FILE_NOT_FOUND);
    }
    Ok(dir.join(".security.sddl"))
}

pub fn set_security(root: &Path, hive: Hive, sub: &str, sddl: &str) -> Result<(), u32> {
    std::fs::write(security_file(root, hive, sub)?, sddl).map_err(|e| io_error(&e))
}

pub fn get_security(root: &Path, hive: Hive, sub: &str) -> Option<String> {
    std::fs::read_to_string(security_file(root, hive, sub).ok()?).ok()
}

pub unsafe fn query_into(kind_data: Result<(u32, Vec<u8>), u32>, kind: *mut u32, data: *mut u8, cb: *mut u32) -> i32 {
    let (k, bytes) = match kind_data {
        Ok(v) => v,
        Err(e) => return e as i32,
    };
    if !kind.is_null() {
        *kind = k;
    }
    if !data.is_null() {
        if cb.is_null() {
            return ERROR_INVALID_PARAMETER as i32;
        }
        if (*cb as usize) < bytes.len() {
            *cb = bytes.len() as u32;
            return 234;
        }
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), data, bytes.len());
    }
    if !cb.is_null() {
        *cb = bytes.len() as u32;
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(tag: &str) -> PathBuf {
        let r = std::env::temp_dir().join(format!("tombox_emureg_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&r);
        r
    }

    #[test]
    fn values_round_trip_and_follow_reg_query_sizes() {
        let r = root("rt");
        let key = format!("S-1-5-21-1\\{FAMILIES}\\Fam_x\\Fam_1.0_x64__x");
        assert!(is_emulated(Hive::Hku, &key));
        assert!(!is_emulated(Hive::Hku, "S-1-5-21-1\\Software\\Other"));
        assert!(!is_emulated(Hive::Hku, &format!("S-1-5-21-1\\{FAMILIES}\\..\\x")));
        assert_eq!(get_value(&r, Hive::Hku, &key, "Flags"), Err(ERROR_FILE_NOT_FOUND));
        assert_eq!(create_key(&r, Hive::Hku, &key), Ok(true));
        assert_eq!(create_key(&r, Hive::Hku, &key), Ok(false));
        set_value(&r, Hive::Hku, &key, "Flags", REG_DWORD, &7u32.to_le_bytes()).unwrap();
        set_value(&r, Hive::Hku, &key, "InstallTime", REG_QWORD, &0x01d9_0000_0000_0001u64.to_le_bytes()).unwrap();
        assert_eq!(get_value(&r, Hive::Hku, &key, "Flags"), Ok((REG_DWORD, 7u32.to_le_bytes().to_vec())));
        unsafe {
            let (mut kind, mut cb) = (0u32, 0u32);
            assert_eq!(query_into(get_value(&r, Hive::Hku, &key, "InstallTime"), &mut kind, core::ptr::null_mut(), &mut cb), 0);
            assert_eq!((kind, cb), (REG_QWORD, 8));
            let mut small = [0u8; 4];
            cb = 4;
            assert_eq!(query_into(get_value(&r, Hive::Hku, &key, "InstallTime"), &mut kind, small.as_mut_ptr(), &mut cb), 234);
            assert_eq!(cb, 8);
            let mut big = [0u8; 8];
            cb = 8;
            assert_eq!(query_into(get_value(&r, Hive::Hku, &key, "InstallTime"), &mut kind, big.as_mut_ptr(), &mut cb), 0);
            assert_eq!(u64::from_le_bytes(big), 0x01d9_0000_0000_0001);
        }
        let parent = format!("S-1-5-21-1\\{FAMILIES}\\Fam_x");
        assert_eq!(delete_key(&r, Hive::Hku, &parent), Err(ERROR_KEY_HAS_CHILDREN));
        delete_key(&r, Hive::Hku, &key).unwrap();
        delete_key(&r, Hive::Hku, &parent).unwrap();
        assert!(!key_exists(&r, Hive::Hku, &parent));
        let _ = std::fs::remove_dir_all(&r);
    }
}
