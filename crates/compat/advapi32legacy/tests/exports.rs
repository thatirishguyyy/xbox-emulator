mod common;

use common::*;
use core::ffi::c_void;

fn dll() -> *mut c_void {
    load_built("advapi32legacy.dll", "advapi32legacy.dll")
}

#[test]
fn export_table_matches_exports_txt() {
    check_table("advapi32legacy.dll");
}

#[test]
fn every_forwarded_export_resolves_to_its_pc_function() {
    let n = check_forwarders(dll(), &[]);
    assert_eq!(n, rows().iter().filter(|e| e.status == "forwarded").count());
    assert!(n >= 200);
}

#[test]
fn implemented_exports_are_this_crates_code() {
    let h = dll();
    for e in rows().iter().filter(|e| e.status == "implemented") {
        assert!(forward_target(e).is_none());
        assert!(!proc(h, &e.name).is_null(), "{}", e.name);
    }
    let f: unsafe extern "system" fn(*mut c_void) -> i32 = unsafe { core::mem::transmute(proc(h, "ElfFlushEventLog")) };
    assert_eq!(unsafe { f(core::ptr::null_mut()) }, 0xC000_0022u32 as i32);
}

#[test]
fn forwarded_calls_behave_as_pc() {
    let h = dll();
    unsafe {
        let get_user: unsafe extern "system" fn(*mut u16, *mut u32) -> i32 = core::mem::transmute(proc(h, "GetUserNameW"));
        let pc_user: unsafe extern "system" fn(*mut u16, *mut u32) -> i32 = core::mem::transmute(proc(system("advapi32.dll"), "GetUserNameW"));
        let (mut a, mut b) = ([0u16; 260], [0u16; 260]);
        let (mut na, mut nb) = (260u32, 260u32);
        assert_eq!(get_user(a.as_mut_ptr(), &mut na), 1);
        assert_eq!(pc_user(b.as_mut_ptr(), &mut nb), 1);
        assert_eq!(a[..na as usize], b[..nb as usize]);

        let lpv: unsafe extern "system" fn(*const u16, *const u16, *mut [u32; 2]) -> i32 = core::mem::transmute(proc(h, "LookupPrivilegeValueW"));
        let mut luid = [0u32; 2];
        assert_eq!(lpv(core::ptr::null(), wide("SeShutdownPrivilege").as_ptr(), &mut luid), 1);
        assert_eq!(luid, [19, 0]);

        let lsa_err: unsafe extern "system" fn(i32) -> u32 = core::mem::transmute(proc(h, "LsaNtStatusToWinError"));
        assert_eq!(lsa_err(0xC000_0022u32 as i32), 5);

        let itu: unsafe extern "system" fn(*const c_void, i32, *mut i32) -> i32 = core::mem::transmute(proc(h, "IsTextUnicode"));
        let text = wide("plain unicode text for the statistics test");
        assert_eq!(itu(text.as_ptr() as _, (text.len() * 2 - 2) as i32, core::ptr::null_mut()), 1);

        let open: unsafe extern "system" fn(isize, *const u16, *mut isize) -> u32 = core::mem::transmute(proc(h, "RegOpenKeyW"));
        let close: unsafe extern "system" fn(isize) -> u32 = core::mem::transmute(proc(system("advapi32.dll"), "RegCloseKey"));
        let mut hk = 0isize;
        assert_eq!(open(0x8000_0001u32 as i32 as isize, wide("Software").as_ptr(), &mut hk), 0);
        assert_ne!(hk, 0);
        assert_eq!(close(hk), 0);

        let del: unsafe extern "system" fn(isize, *const u16, *const u16) -> u32 = core::mem::transmute(proc(h, "RegDeleteKeyValueW"));
        assert_eq!(del(0x8000_0001u32 as i32 as isize, wide("Software\\TomBoxA32lNoSuchKey").as_ptr(), wide("x").as_ptr()), 2);

        let rnd: unsafe extern "system" fn(*mut u8, u32) -> u8 = core::mem::transmute(proc(h, "SystemFunction036"));
        let mut buf = [0u8; 32];
        assert_eq!(rnd(buf.as_mut_ptr(), 32), 1);
        assert!(buf.iter().any(|&b| b != 0));
    }
}

#[test]
fn lsa_password_less_exports_write_the_emulated_device_value() {
    let root = std::env::temp_dir().join(format!("tombox-a32l-pwdless-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::env::set_var("TOMBOX_GDK_ROOT", &root);
    let value = root.join("hklm").join("SOFTWARE").join("Microsoft").join("Windows NT").join("CurrentVersion").join("PasswordLess").join("Device").join("DevicePasswordLessBuildVersion.dword");
    let h = dll();
    unsafe {
        let enable: unsafe extern "system" fn() -> i32 = core::mem::transmute(proc(h, "LsaEnablePasswordLessCurrentUser"));
        let disable: unsafe extern "system" fn() -> i32 = core::mem::transmute(proc(h, "LsaDisablePasswordLessCurrentUser"));
        assert_eq!(enable(), 0);
        assert_eq!(std::fs::read_to_string(&value).unwrap().trim(), "2");
        assert_eq!(disable(), 0);
        assert_eq!(std::fs::read_to_string(&value).unwrap().trim(), "0");
        let ic = root.join("hklm").join(r"SYSTEM\CurrentControlSet\Control\Cryptography\PwdlessPlat\ImageCustomization");
        std::fs::create_dir_all(&ic).unwrap();
        std::fs::write(ic.join("ForceDisablePasswordlessNewDevices.dword"), "1\n").unwrap();
        let me = advapi32legacy::passwordless::TitleToken::caller().user;
        assert!(me.starts_with("S-1-"));
        let uc = root.join("hklm").join(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\PasswordLess\UserConfig").join(&me).join("PasswordLess.dword");
        assert_eq!(enable(), 0);
        assert_eq!(std::fs::read_to_string(&uc).unwrap().trim(), "1");
        assert_eq!(disable(), 0);
        assert_eq!(std::fs::read_to_string(&uc).unwrap().trim(), "2");
        assert_eq!(std::fs::read_to_string(&value).unwrap().trim(), "0");
    }
    let _ = std::fs::remove_dir_all(&root);
}
