use core::ffi::c_void;
use core::ptr::null_mut;
use std::path::Path;

use compat_common::gdk::emureg::{self, Hive};

pub type Hkey = *mut c_void;

#[link(name = "kernel32")]
extern "system" {
    fn PackageFamilyNameFromFullName(full: *const u16, len: *mut u32, family: *mut u16) -> i32;
    fn PackageIdFromFullName(full: *const u16, flags: u32, len: *mut u32, buf: *mut u8) -> i32;
    fn PackageFamilyNameFromId(id: *const u8, len: *mut u32, family: *mut u16) -> i32;
    fn GetLastError() -> u32;
    fn LocalFree(p: *mut c_void) -> *mut c_void;
}

#[link(name = "advapi32")]
extern "system" {
    fn GetTokenInformation(token: *mut c_void, class: u32, buf: *mut c_void, len: u32, ret: *mut u32) -> i32;
    fn ConvertStringSecurityDescriptorToSecurityDescriptorW(s: *const u16, rev: u32, sd: *mut *mut c_void, len: *mut u32) -> i32;
    fn GetSecurityDescriptorDacl(sd: *mut c_void, present: *mut i32, dacl: *mut *mut c_void, defaulted: *mut i32) -> i32;
    fn FreeSid(sid: *mut c_void) -> *mut c_void;
}

#[link(name = "userenv")]
extern "system" {
    fn DeriveAppContainerSidFromAppContainerName(name: *const u16, sid: *mut *mut c_void) -> i32;
}

#[repr(C)]
struct UnicodeString {
    len: u16,
    max: u16,
    buf: *mut u16,
}

#[link(name = "ntdll")]
extern "system" {
    fn RtlConvertSidToUnicodeString(out: *mut UnicodeString, sid: *const c_void, alloc: u8) -> i32;
    fn RtlFreeUnicodeString(s: *mut UnicodeString);
}

pub const E_INVALIDARG: i32 = 0x8007_0057u32 as i32;
pub const ALL_APP_PACKAGES: &str = "S-1-15-2-1";
pub const CURRENT_THREAD_EFFECTIVE_TOKEN: *mut c_void = -6isize as *mut c_void;

pub fn hresult_from_win32(e: u32) -> i32 {
    if e as i32 <= 0 {
        e as i32
    } else {
        ((e & 0xffff) | 0x8007_0000) as i32
    }
}

unsafe fn wstr(p: *const u16) -> String {
    String::from_utf16_lossy(core::slice::from_raw_parts(p, wlen(p)))
}

fn wz(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

pub fn data_root() -> std::path::PathBuf {
    compat_common::gdk::data_root()
}

unsafe fn family_of(full: *const u16) -> Result<String, i32> {
    let mut id = [0u8; 0xf0];
    let mut len = id.len() as u32;
    let r = PackageIdFromFullName(full, 0, &mut len, id.as_mut_ptr());
    if r != 0 {
        return Err(hresult_from_win32(r as u32));
    }
    let mut family = [0u16; 0x41];
    let mut flen = family.len() as u32;
    let r = PackageFamilyNameFromId(id.as_ptr(), &mut flen, family.as_mut_ptr());
    if r != 0 {
        return Err(hresult_from_win32(r as u32));
    }
    Ok(wstr(family.as_ptr()))
}

pub unsafe fn sid_string(sid: *const c_void) -> Result<String, i32> {
    let mut u = UnicodeString { len: 0, max: 0, buf: null_mut() };
    let st = RtlConvertSidToUnicodeString(&mut u, sid, 1);
    if st < 0 {
        return Err(st);
    }
    let s = String::from_utf16_lossy(core::slice::from_raw_parts(u.buf, (u.len / 2) as usize));
    RtlFreeUnicodeString(&mut u);
    Ok(s)
}

pub unsafe fn effective_user() -> Result<String, i32> {
    let mut buf = [0u64; 64];
    let mut ret = 0u32;
    if GetTokenInformation(CURRENT_THREAD_EFFECTIVE_TOKEN, 1, buf.as_mut_ptr() as *mut c_void, 512, &mut ret) == 0 {
        return Err(hresult_from_win32(GetLastError()));
    }
    sid_string(buf[0] as *const c_void)
}

unsafe fn resolve_user(root: &Path, user: *const c_void) -> Result<String, i32> {
    let me = effective_user().ok();
    let s = if user.is_null() { effective_user()? } else { sid_string(user)? };
    if !emureg::user_hive_loaded(root, &s, me.as_deref()) {
        return Err(hresult_from_win32(emureg::ERROR_FILE_NOT_FOUND));
    }
    Ok(s)
}

pub fn capability_form(mut s: Vec<u16>) -> Vec<u16> {
    match s.len() {
        n if n > 7 => s[7] = b'3' as u16,
        7 => s.push(b'3' as u16),
        _ => {}
    }
    s
}

unsafe fn family_dacl(package_sid: *const u16, flags: u32, family: &str) -> Result<String, i32> {
    let base: Vec<u16> = if flags == 2 {
        ALL_APP_PACKAGES.encode_utf16().collect()
    } else if !package_sid.is_null() {
        core::slice::from_raw_parts(package_sid, wlen(package_sid)).to_vec()
    } else {
        let mut sid: *mut c_void = null_mut();
        let hr = DeriveAppContainerSidFromAppContainerName(wz(family).as_ptr(), &mut sid);
        if hr < 0 {
            return Err(hr);
        }
        let s = sid_string(sid);
        FreeSid(sid);
        s?.encode_utf16().collect()
    };
    let cap = String::from_utf16_lossy(&capability_form(base));
    let sddl = format!("D:AR(A;OICI;KR;;;{ALL_APP_PACKAGES})(A;OICI;KR;;;{cap})");
    let mut sd: *mut c_void = null_mut();
    if ConvertStringSecurityDescriptorToSecurityDescriptorW(wz(&sddl).as_ptr(), 1, &mut sd, null_mut()) == 0 {
        return Err(hresult_from_win32(GetLastError()));
    }
    let (mut present, mut defaulted) = (0i32, 0i32);
    let mut dacl: *mut c_void = null_mut();
    let ok = GetSecurityDescriptorDacl(sd, &mut present, &mut dacl, &mut defaulted);
    let err = GetLastError();
    LocalFree(sd);
    if ok == 0 {
        return Err(hresult_from_win32(err));
    }
    Ok(sddl)
}

pub fn families_key(sid: &str) -> String {
    format!("{sid}\\{}", emureg::FAMILIES)
}

pub fn add_to_family_xref(root: &Path, sid: &str, family: &str, full: &str, sddl: &str, flags: u32, install_time: u64) -> Result<(), u32> {
    let families = families_key(sid);
    emureg::create_key(root, Hive::Hku, &families)?;
    let fkey = format!("{families}\\{family}");
    if emureg::create_key(root, Hive::Hku, &fkey)? {
        emureg::set_security(root, Hive::Hku, &fkey, sddl)?;
    }
    let pkey = format!("{fkey}\\{full}");
    emureg::create_key(root, Hive::Hku, &pkey)?;
    emureg::set_value(root, Hive::Hku, &pkey, "Flags", emureg::REG_DWORD, &flags.to_le_bytes())?;
    emureg::set_value(root, Hive::Hku, &pkey, "InstallTime", emureg::REG_QWORD, &install_time.to_le_bytes())
}

pub fn remove_from_family_xref(root: &Path, sid: &str, family: &str, full: &str) {
    let fkey = format!("{}\\{family}", families_key(sid));
    if !emureg::key_exists(root, Hive::Hku, &fkey) {
        return;
    }
    let _ = emureg::delete_key(root, Hive::Hku, &format!("{fkey}\\{full}"));
    if emureg::subkeys(root, Hive::Hku, &fkey).is_empty() {
        let _ = emureg::delete_key(root, Hive::Hku, &fkey);
    }
}

#[no_mangle]
pub unsafe extern "system" fn AddPackageToFamilyXref(user: *const c_void, full: *const u16, package_sid: *const u16, flags: u32, install_time: u64) -> i32 {
    if full.is_null() {
        return E_INVALIDARG;
    }
    let family = match family_of(full) {
        Ok(f) => f,
        Err(hr) => return hr,
    };
    let root = data_root();
    let sid = match resolve_user(&root, user) {
        Ok(s) => s,
        Err(hr) => return hr,
    };
    let sddl = match family_dacl(package_sid, flags, &family) {
        Ok(s) => s,
        Err(hr) => return hr,
    };
    match add_to_family_xref(&root, &sid, &family, &wstr(full), &sddl, flags, install_time) {
        Ok(()) => 0,
        Err(e) => hresult_from_win32(e),
    }
}

#[no_mangle]
pub unsafe extern "system" fn RemovePackageFromFamilyXref(user: *const c_void, full: *const u16) -> i32 {
    if full.is_null() {
        return E_INVALIDARG;
    }
    let family = match family_of(full) {
        Ok(f) => f,
        Err(hr) => return hr,
    };
    let root = data_root();
    let sid = match resolve_user(&root, user) {
        Ok(s) => s,
        Err(hr) => return hr,
    };
    remove_from_family_xref(&root, &sid, &family, &wstr(full));
    0
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct FileTime {
    pub low: u32,
    pub high: u32,
}

pub const ERROR_FILE_NOT_FOUND: i32 = 2;
pub const ERROR_INVALID_PARAMETER: i32 = 0x57;
pub const ERROR_NOT_FOUND: i32 = 0x490;
pub const ERROR_REGISTRY_CORRUPT_TYPE: i32 = 0x3d55;
pub const REG_QWORD: u32 = 0xb;
pub const PATH_CAP: usize = 0x128;
pub const FAMILIES: &str = emureg::FAMILIES;

unsafe fn wlen(p: *const u16) -> usize {
    let mut n = 0;
    while *p.add(n) != 0 {
        n += 1;
    }
    n
}

pub unsafe fn install_time_in(root: &Path, user: &str, prefix: &str, full: *const u16, out: *mut FileTime) -> i32 {
    if full.is_null() || out.is_null() {
        return ERROR_INVALID_PARAMETER;
    }
    let mut family = [0u16; 65];
    let mut flen = family.len() as u32;
    let r = PackageFamilyNameFromFullName(full, &mut flen, family.as_mut_ptr());
    if r != 0 {
        return r;
    }
    let fam = String::from_utf16_lossy(&family[..wlen(family.as_ptr())]);
    let rel = format!("{prefix}\\{fam}\\{}", wstr(full));
    if rel.encode_utf16().count() >= PATH_CAP {
        return ERROR_INVALID_PARAMETER;
    }
    let key = format!("{user}\\{rel}");
    if !emureg::key_exists(root, Hive::Hku, &key) {
        return ERROR_NOT_FOUND;
    }
    let (ty, data) = match emureg::get_value(root, Hive::Hku, &key, "InstallTime") {
        Ok(v) => v,
        Err(e) => return e as i32,
    };
    if ty != REG_QWORD || data.len() != 8 {
        return ERROR_REGISTRY_CORRUPT_TYPE;
    }
    let v = u64::from_le_bytes(data.try_into().unwrap_or_default());
    (*out).low = v as u32;
    (*out).high = (v >> 32) as u32;
    0
}

#[no_mangle]
pub unsafe extern "system" fn GetPackageInstallTime(full: *const u16, out: *mut FileTime) -> i32 {
    if full.is_null() || out.is_null() {
        return ERROR_INVALID_PARAMETER;
    }
    let me = match effective_user() {
        Ok(s) => s,
        Err(hr) => return hr,
    };
    install_time_in(&data_root(), &me, FAMILIES, full, out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::ptr::null;
    use std::sync::{Mutex, MutexGuard, Once};

    const FULL: &str = "TomBox.Test_1.0.0.0_x64__8wekyb3d8bbwe";
    const FAMILY: &str = "TomBox.Test_8wekyb3d8bbwe";

    static LOCK: Mutex<()> = Mutex::new(());

    fn guard() -> MutexGuard<'static, ()> {
        static I: Once = Once::new();
        I.call_once(|| {
            let d = std::env::temp_dir().join(format!("tombox-kernelbase-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::env::set_var("TOMBOX_GDK_ROOT", &d);
        });
        LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    fn seed(user: &str, prefix: &str, ty: u32, data: &[u8]) {
        let key = format!("{user}\\{prefix}\\{FAMILY}\\{FULL}");
        emureg::create_key(&data_root(), Hive::Hku, &key).unwrap();
        emureg::set_value(&data_root(), Hive::Hku, &key, "InstallTime", ty, data).unwrap();
    }

    fn pkg_key(user: &str, family: &str, full: &str) -> String {
        format!("{}\\{family}\\{full}", families_key(user))
    }

    #[test]
    fn reads_qword_from_families_key() {
        let _g = guard();
        let p = format!("{FAMILIES}A{}", std::process::id());
        seed("S-1-5-21-9-9-9-1", &p, REG_QWORD, &0x01d9_1234_5678_9abcu64.to_le_bytes());
        let mut ft = FileTime::default();
        let r = unsafe { install_time_in(&data_root(), "S-1-5-21-9-9-9-1", &p, w(FULL).as_ptr(), &mut ft) };
        assert_eq!(r, 0);
        assert_eq!(ft, FileTime { low: 0x5678_9abc, high: 0x01d9_1234 });
    }

    #[test]
    fn wrong_type_is_corrupt_type() {
        let _g = guard();
        let p = format!("{FAMILIES}B{}", std::process::id());
        seed("S-1-5-21-9-9-9-1", &p, emureg::REG_DWORD, &7u32.to_le_bytes());
        let mut ft = FileTime { low: 9, high: 9 };
        let r = unsafe { install_time_in(&data_root(), "S-1-5-21-9-9-9-1", &p, w(FULL).as_ptr(), &mut ft) };
        assert_eq!(r, ERROR_REGISTRY_CORRUPT_TYPE);
        assert_eq!(ft, FileTime { low: 9, high: 9 });
    }

    #[test]
    fn absent_key_is_not_found() {
        let _g = guard();
        let mut ft = FileTime::default();
        assert_eq!(unsafe { install_time_in(&data_root(), "S-1-5-21-9-9-9-1", FAMILIES, w(FULL).as_ptr(), &mut ft) }, ERROR_NOT_FOUND);
    }

    fn unique(tag: &str) -> (String, String) {
        let n = format!("TomBox.Xref{tag}{}", std::process::id());
        (format!("{n}_1.0.0.0_x64__8wekyb3d8bbwe"), format!("{n}_8wekyb3d8bbwe"))
    }

    #[test]
    fn family_xref_writes_the_emulated_hku_store_with_dacl_flags_and_install_time() {
        let _g = guard();
        let (full, family) = unique("A");
        let fw = w(&full);
        let mut ft = FileTime::default();
        let root = data_root();
        unsafe {
            let me = effective_user().unwrap();
            assert_eq!(GetPackageInstallTime(fw.as_ptr(), &mut ft), ERROR_NOT_FOUND);
            assert_eq!(AddPackageToFamilyXref(null(), fw.as_ptr(), null(), 1, 0x01d9_1234_5678_9abc), 0);
            assert_eq!(GetPackageInstallTime(fw.as_ptr(), &mut ft), 0);
            assert_eq!(ft, FileTime { low: 0x5678_9abc, high: 0x01d9_1234 });
            let pk = pkg_key(&me, &family, &full);
            assert!(root.join("hku").join(&me).is_dir());
            assert_eq!(emureg::get_value(&root, Hive::Hku, &pk, "Flags"), Ok((emureg::REG_DWORD, 1u32.to_le_bytes().to_vec())));
            assert_eq!(emureg::get_value(&root, Hive::Hku, &pk, "InstallTime"), Ok((emureg::REG_QWORD, 0x01d9_1234_5678_9abcu64.to_le_bytes().to_vec())));
            let mut sid: *mut c_void = null_mut();
            assert!(DeriveAppContainerSidFromAppContainerName(w(&family).as_ptr(), &mut sid) >= 0);
            let cap = String::from_utf16_lossy(&capability_form(sid_string(sid).unwrap().encode_utf16().collect()));
            FreeSid(sid);
            assert!(cap.starts_with("S-1-15-3-"));
            let fkey = format!("{}\\{family}", families_key(&me));
            assert_eq!(emureg::get_security(&root, Hive::Hku, &fkey).unwrap(), format!("D:AR(A;OICI;KR;;;S-1-15-2-1)(A;OICI;KR;;;{cap})"));
            let h = compat_common::gdk::hook::emulated_open_at(&root, 0x8000_0003, &pk, Some(&me)).unwrap().unwrap();
            let (mut kind, mut cb, mut v) = (0u32, 8u32, 0u64);
            assert_eq!(compat_common::gdk::hook::emulated_query_at(&root, h, w("InstallTime").as_ptr(), &mut kind, &mut v as *mut u64 as *mut u8, &mut cb), Some(0));
            assert_eq!((kind, v), (REG_QWORD, 0x01d9_1234_5678_9abc));
            assert_eq!(compat_common::gdk::hook::emulated_close(h), Some(0));
            assert_eq!(AddPackageToFamilyXref(null(), fw.as_ptr(), w("S-1-15-2-1").as_ptr(), 0, 0x0000_0002_0000_0001), 0);
            assert_eq!(GetPackageInstallTime(fw.as_ptr(), &mut ft), 0);
            assert_eq!(ft, FileTime { low: 1, high: 2 });
            assert_eq!(emureg::get_security(&root, Hive::Hku, &fkey).unwrap(), format!("D:AR(A;OICI;KR;;;S-1-15-2-1)(A;OICI;KR;;;{cap})"), "the DACL is applied only when the family key is created");
            assert_eq!(RemovePackageFromFamilyXref(null(), fw.as_ptr()), 0);
            assert_eq!(GetPackageInstallTime(fw.as_ptr(), &mut ft), ERROR_NOT_FOUND);
            assert!(!emureg::key_exists(&root, Hive::Hku, &fkey));
            assert_eq!(RemovePackageFromFamilyXref(null(), fw.as_ptr()), 0);
        }
    }

    #[test]
    fn family_keeps_other_packages_on_remove() {
        let _g = guard();
        let n = format!("TomBox.XrefE{}", std::process::id());
        let a = format!("{n}_1.0.0.0_x64__8wekyb3d8bbwe");
        let b = format!("{n}_2.0.0.0_x64__8wekyb3d8bbwe");
        let family = format!("{n}_8wekyb3d8bbwe");
        unsafe {
            let me = effective_user().unwrap();
            assert_eq!(AddPackageToFamilyXref(null(), w(&a).as_ptr(), null(), 0, 1), 0);
            assert_eq!(AddPackageToFamilyXref(null(), w(&b).as_ptr(), null(), 0, 2), 0);
            assert_eq!(RemovePackageFromFamilyXref(null(), w(&a).as_ptr()), 0);
            let fkey = format!("{}\\{family}", families_key(&me));
            assert_eq!(emureg::subkeys(&data_root(), Hive::Hku, &fkey), vec![b.clone()]);
            assert_eq!(RemovePackageFromFamilyXref(null(), w(&b).as_ptr()), 0);
            assert!(!emureg::key_exists(&data_root(), Hive::Hku, &fkey));
        }
    }

    #[test]
    fn family_xref_flags_two_uses_all_app_packages() {
        let _g = guard();
        let (full, family) = unique("B");
        let fw = w(&full);
        unsafe {
            assert_eq!(AddPackageToFamilyXref(null(), fw.as_ptr(), w("not-a-sid").as_ptr(), 2, 5), 0);
            let fkey = format!("{}\\{family}", families_key(&effective_user().unwrap()));
            assert_eq!(emureg::get_security(&data_root(), Hive::Hku, &fkey).unwrap(), "D:AR(A;OICI;KR;;;S-1-15-2-1)(A;OICI;KR;;;S-1-15-3-1)");
            assert_eq!(RemovePackageFromFamilyXref(null(), fw.as_ptr()), 0);
        }
    }

    #[test]
    fn family_xref_bad_package_sid_is_converter_error() {
        let _g = guard();
        let (full, _) = unique("C");
        let fw = w(&full);
        let mut ft = FileTime::default();
        unsafe {
            let hr = AddPackageToFamilyXref(null(), fw.as_ptr(), w("S-1-15-2-zz").as_ptr(), 0, 5);
            assert_eq!(hr, hresult_from_win32(1337));
            assert_eq!(GetPackageInstallTime(fw.as_ptr(), &mut ft), ERROR_NOT_FOUND);
        }
    }

    #[test]
    fn family_xref_argument_and_name_errors_are_hresults() {
        let _g = guard();
        let bad = w("not a package name");
        unsafe {
            assert_eq!(AddPackageToFamilyXref(null(), null(), null(), 0, 0), E_INVALIDARG);
            assert_eq!(RemovePackageFromFamilyXref(null(), null()), E_INVALIDARG);
            let mut id = [0u8; 0xf0];
            let mut n = id.len() as u32;
            let pc = PackageIdFromFullName(bad.as_ptr(), 0, &mut n, id.as_mut_ptr());
            assert_ne!(pc, 0);
            assert_eq!(AddPackageToFamilyXref(null(), bad.as_ptr(), null(), 0, 0), hresult_from_win32(pc as u32));
            assert_eq!(RemovePackageFromFamilyXref(null(), bad.as_ptr()), hresult_from_win32(pc as u32));
        }
    }

    #[test]
    fn family_xref_other_user_goes_to_that_users_emulated_hive() {
        let _g = guard();
        let (full, family) = unique("D");
        let fw = w(&full);
        let mut ft = FileTime::default();
        let system: [u8; 12] = [1, 1, 0, 0, 0, 0, 0, 5, 18, 0, 0, 0];
        unsafe {
            assert_eq!(AddPackageToFamilyXref(system.as_ptr() as *const c_void, fw.as_ptr(), null(), 0, 9), 0);
            assert!(emureg::key_exists(&data_root(), Hive::Hku, &pkg_key("S-1-5-18", &family, &full)));
            assert_eq!(GetPackageInstallTime(fw.as_ptr(), &mut ft), ERROR_NOT_FOUND);
            let ghost: [u8; 16] = [1, 2, 0, 0, 0, 0, 0, 5, 21, 0, 0, 0, 0x78, 0x56, 0x34, 0x12];
            assert_eq!(AddPackageToFamilyXref(ghost.as_ptr() as *const c_void, fw.as_ptr(), null(), 0, 9), hresult_from_win32(2));
            assert_eq!(RemovePackageFromFamilyXref(system.as_ptr() as *const c_void, fw.as_ptr()), 0);
        }
    }

    #[test]
    fn capability_form_rewrites_char_seven() {
        let f = |s: &str| String::from_utf16_lossy(&capability_form(s.encode_utf16().collect()));
        assert_eq!(f("S-1-15-2-1"), "S-1-15-3-1");
        assert_eq!(f("S-1-15-"), "S-1-15-3");
        assert_eq!(f("S-1"), "S-1");
    }

    #[test]
    fn overlong_path_is_invalid_parameter() {
        let _g = guard();
        let p = "x".repeat(PATH_CAP);
        let mut ft = FileTime::default();
        assert_eq!(unsafe { install_time_in(&data_root(), "S-1-5-18", &p, w(FULL).as_ptr(), &mut ft) }, ERROR_INVALID_PARAMETER);
    }
}
