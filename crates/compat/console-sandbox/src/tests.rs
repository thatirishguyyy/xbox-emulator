use super::*;
use std::sync::{Mutex, Once};

static TEST_LOCK: Mutex<()> = Mutex::new(());

fn init() -> std::sync::MutexGuard<'static, ()> {
    static I: Once = Once::new();
    I.call_once(|| {
        let d = std::env::temp_dir().join(format!("tombox-sandbox-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::env::set_var("TOMBOX_GDK_ROOT", &d);
    });
    TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn w(s: &str) -> Vec<u16> {
    wstr::wide(s)
}

#[test]
fn paths_stay_inside_the_sandbox() {
    let _g = init();
    let (c, h) = fs::resolve(r"c:\windows\..\..\..\..\x\y.txt").unwrap();
    assert_eq!(c, r"C:\x\y.txt");
    assert!(h.starts_with(fs::fs_root()));
    assert_eq!(fs::resolve(r"boot\a.bsc").unwrap().0, r"C:\Windows\System32\boot\a.bsc");
    assert!(fs::resolve(r"\\server\share\f").is_none());
    assert!(fs::resolve(r"\\.\PhysicalDrive0").is_none());
}

fn windows_last_error() -> u32 {
    unsafe { GetLastError() }
}

fn close_handle(h: isize) {
    unsafe { CloseHandle(h as usize) };
}

fn assert_contained(input: &str, console: &str, host: &std::path::Path) {
    let drive = console.chars().next().unwrap();
    let root = fs::drive_dir(drive).to_string_lossy().to_ascii_lowercase();
    let h = host.to_string_lossy().to_string();
    for probe in [h.clone(), format!("\\\\?\\{h}")] {
        let full = fs::full_path_name(&probe).unwrap_or_default().to_ascii_lowercase();
        let full = full.strip_prefix("\\\\?\\").unwrap_or(&full).to_string();
        assert!(full == root || full.starts_with(&format!("{root}\\")), "{input:?} -> {console} -> {h} normalizes to {full} outside {root}");
        let tail = &full[root.len()..];
        assert!(tail.split('\\').skip(1).all(|c| !c.contains(':') || c.matches(':').count() <= 2), "{input:?} -> {full}");
    }
    assert!(console.len() >= 3 && console.as_bytes()[1] == b':' && console.as_bytes()[2] == b'\\', "{input:?} -> {console}");
}

#[test]
fn drive_style_components_mid_path_cannot_escape() {
    let _g = init();
    let me = std::env::current_dir().unwrap().join("Cargo.toml").to_string_lossy().to_string();
    let cases = [
        format!(r"C:\a\{me}"),
        r"C:\a\D:\xbox-pc-port\emulator\Cargo.toml".to_string(),
        r"C:\a\D:xbox-pc-port\emulator\Cargo.toml".to_string(),
        r"C:\x\..\D:\xbox-pc-port\emulator\Cargo.toml".to_string(),
        r"C:\a\b:c\d".to_string(),
        r"D:\a\C:\Windows\win.ini".to_string(),
        r"D:\a\c:\Windows\win.ini".to_string(),
        r"\.\C:\x".to_string(),
        r"C:\a\\?\D:\b".to_string(),
        r"C:\a\x:foo\b".to_string(),
        r"C:\a\D:".to_string(),
        r"C:\x\C:".to_string(),
        r"\\?\C:\a\D:\b".to_string(),
        r"\??\C:\a\D:\b".to_string(),
        r"C:\a\?\b".to_string(),
        r"C:\a\*".to_string(),
        r"C:\Windows\win.ini:".to_string(),
        r"C:\Windows\win.ini:s:t:u".to_string(),
    ];
    for c in &cases {
        assert_eq!(fs::try_resolve(c).map(|r| r.0), Err(fs::STATUS_OBJECT_NAME_INVALID), "{c}");
    }
    unsafe {
        let n = w(r"C:\a\D:\xbox-pc-port\emulator\Cargo.toml");
        assert_eq!(fs::create_file_w(n.as_ptr(), 0x8000_0000, 7, core::ptr::null(), 3, 0, 0), -1);
        assert_eq!(windows_last_error(), fs::ERROR_INVALID_NAME);
        let n = w(&format!(r"C:\a\{me}"));
        assert_eq!(fs::get_file_attributes_w(n.as_ptr()), u32::MAX);
        assert_eq!(windows_last_error(), fs::ERROR_INVALID_NAME);
    }
}

#[test]
fn console_path_rules_match_win32_and_ntfs() {
    let _g = init();
    let ok = |s: &str| fs::try_resolve(s).unwrap_or_else(|e| panic!("{s}: {e:#x}")).0;
    assert_eq!(ok(r"C:\Windows\win.ini:stream"), r"C:\Windows\win.ini:stream");
    assert_eq!(ok(r"C:\Windows\win.ini::$DATA"), r"C:\Windows\win.ini::$DATA");
    assert_eq!(ok(r"C:\Windows\win.ini:s:$DATA"), r"C:\Windows\win.ini:s:$DATA");
    assert_eq!(ok(r"C:\a\x.  "), r"C:\a\x");
    assert_eq!(ok(r"C:\a\x.\b"), r"C:\a\x\b");
    assert_eq!(ok(r"C:\a\...\b"), r"C:\a\...\b");
    assert_eq!(ok(r"C:\a\.. .\b"), r"C:\a\.. \b");
    assert_eq!(ok(r"C:\a\..  "), r"C:\a");
    assert_eq!(ok(r"C:Windows\x"), r"C:\Windows\System32\Windows\x");
    assert_eq!(ok(r"D:foo\bar"), r"D:\foo\bar");
    assert_eq!(ok(r"\\.\C:\a\b"), r"C:\a\b");
    assert_eq!(ok(r"\\?\D:\x\y"), r"D:\x\y");
    assert_eq!(ok(r"\??\D:\x\y\"), r"D:\x\y");
    assert_eq!(ok(r"\\?\C:\Windows\...\x"), r"C:\Windows\...\x");
    assert_eq!(fs::try_resolve(r"\\?\C:\Windows\...").map(|r| r.0), Err(fs::STATUS_OBJECT_NAME_INVALID));
    assert_eq!(fs::try_resolve(r"\\?\C:\Windows\..\..\x").map(|r| r.0), Err(fs::STATUS_OBJECT_NAME_INVALID));
    assert_eq!(fs::try_resolve(r"\??\D:\x\..\..\..\y").map(|r| r.0), Err(fs::STATUS_OBJECT_NAME_INVALID));
    assert_eq!(fs::try_resolve(r"\\?\C:\a\\b").map(|r| r.0), Err(fs::STATUS_OBJECT_NAME_INVALID));
    assert_eq!(fs::try_resolve(r"\\?\C:\a\.\b").map(|r| r.0), Err(fs::STATUS_OBJECT_PATH_NOT_FOUND));
    assert_eq!(fs::try_resolve(r"\\?\C:\a\x.").map(|r| r.0), Err(fs::STATUS_OBJECT_NAME_INVALID));
    assert_eq!(fs::try_resolve(r"\\?\UNC\server\share\f").map(|r| r.0), Err(fs::STATUS_BAD_NETWORK_PATH));
    assert_eq!(fs::try_resolve(r"\??\UNC\server\share\f").map(|r| r.0), Err(fs::STATUS_BAD_NETWORK_PATH));
    assert_eq!(fs::try_resolve(r"//server/share/f").map(|r| r.0), Err(fs::STATUS_BAD_NETWORK_PATH));
    assert_eq!(fs::try_resolve(r"\\?\GLOBALROOT\Device\HarddiskVolume1\Windows").map(|r| r.0), Err(fs::STATUS_OBJECT_PATH_NOT_FOUND));
    assert_eq!(fs::try_resolve(r"\\.\C:\a\..\..\x").map(|r| r.0), Err(fs::STATUS_OBJECT_NAME_NOT_FOUND));
    assert_eq!(fs::try_resolve(r"\\.\C:").map(|r| r.0), Err(fs::STATUS_ACCESS_DENIED));
    assert_eq!(fs::try_resolve(r"\\?\\??\C:\x").map(|r| r.0), Err(fs::STATUS_OBJECT_PATH_NOT_FOUND));
    assert_eq!(fs::try_resolve("C:\\a\\b\u{1}c").map(|r| r.0), Err(fs::STATUS_OBJECT_NAME_INVALID));
    assert_eq!(fs::try_resolve("\\\\?\\aé€x").map(|r| r.0), Err(fs::STATUS_OBJECT_PATH_NOT_FOUND));
    assert_eq!(fs::try_resolve("\\??\\é").map(|r| r.0), Err(fs::STATUS_OBJECT_PATH_NOT_FOUND));
    assert_eq!(fs::try_resolve("\\\\.\\é").map(|r| r.0), Err(fs::STATUS_OBJECT_NAME_NOT_FOUND));
    assert_eq!(ok("C:\\a\\x\u{FF1A}y"), "C:\\a\\x\u{FF1A}y");
}

#[test]
fn rename_target_is_resolved_inside_the_sandbox() {
    let _g = init();
    fs::ensure_system_drive();
    let src = fs::drive_dir('C').join("ren_src.txt");
    std::fs::write(&src, b"x").unwrap();
    let dst = fs::drive_dir('C').join("ren_dst.txt");
    let _ = std::fs::remove_file(&dst);
    let rename = |h: isize, target: &str| unsafe {
        let name: Vec<u16> = target.encode_utf16().collect();
        let mut buf = vec![0u64; (24 + name.len() * 2).div_ceil(8)];
        let p = buf.as_mut_ptr() as *mut u8;
        (p.add(16) as *mut u32).write_unaligned((name.len() * 2) as u32);
        core::ptr::copy_nonoverlapping(name.as_ptr() as *const u8, p.add(20), name.len() * 2);
        fs::set_file_information_by_handle(h, 3, p, (20 + name.len() * 2 + 2) as u32)
    };
    unsafe {
        let n = w(r"C:\ren_src.txt");
        let h = fs::create_file_w(n.as_ptr(), 0x8001_0000, 7, core::ptr::null(), 3, 0, 0);
        assert_ne!(h, -1);
        assert_eq!(rename(h, r"C:\a\D:\ren_escape.txt"), 0);
        assert_eq!(windows_last_error(), fs::ERROR_INVALID_NAME);
        assert_ne!(rename(h, r"C:\ren_dst.txt"), 0);
        close_handle(h);
    }
    assert!(dst.is_file() && !src.exists());
    let _ = std::fs::remove_file(&dst);
}

#[test]
fn fuzzed_console_paths_never_leave_their_drive() {
    let _g = init();
    let alphabet = ["a", "b", "C", "D", "x", ":", "\\", "/", ".", " ", "?", "*", "CON", "..", "\\\\?\\", "\\??\\", "\\\\.\\", "C:", "d:", "$DATA", "\\..\\", "...", "nul", ". ", "é", "€", "\u{FF1A}", "\u{2215}", "\u{FF3C}"];
    let mut state = 0x9e37_79b9_7f4a_7c15u64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let mut accepted = 0usize;
    for _ in 0..50_000 {
        let len = (next() % 40) as usize;
        let mut s = String::new();
        for _ in 0..len {
            s.push_str(alphabet[(next() % alphabet.len() as u64) as usize]);
        }
        if let Ok((console, host)) = fs::try_resolve(&s) {
            accepted += 1;
            assert_contained(&s, &console, &host);
        }
    }
    assert!(accepted > 1000, "fuzz accepted only {accepted}");
}

#[test]
fn reg_get_value_reports_sizes_types_and_missing_values() {
    let _g = init();
    let key = r"\Registry\Machine\SOFTWARE\TomBoxSandboxTest";
    registry::set_string(key, "Name", "abc");
    registry::set(key, "Num", registry::REG_DWORD, &7u32.to_le_bytes());
    let mut h = 0usize;
    unsafe {
        assert_eq!(registry::reg_open_key_ex_w(registry::HKEY_LOCAL_MACHINE, w(r"SOFTWARE\TomBoxSandboxTest").as_ptr(), 0, 0x20019, &mut h), 0);
        let mut size = 0u32;
        let mut kind = 0u32;
        assert_eq!(registry::reg_get_value_w(h, core::ptr::null(), w("Name").as_ptr(), 2, &mut kind, core::ptr::null_mut(), &mut size), 0);
        assert_eq!((kind, size), (registry::REG_SZ, 8));
        let mut small = [0u8; 4];
        size = 4;
        assert_eq!(registry::reg_get_value_w(h, core::ptr::null(), w("Name").as_ptr(), 2, &mut kind, small.as_mut_ptr(), &mut size), registry::ERROR_MORE_DATA);
        assert_eq!(size, 8);
        assert_eq!(registry::reg_get_value_w(h, core::ptr::null(), w("Num").as_ptr(), 2, &mut kind, core::ptr::null_mut(), &mut size), registry::ERROR_UNSUPPORTED_TYPE);
        assert_eq!(registry::reg_get_value_w(h, core::ptr::null(), w("Gone").as_ptr(), 0xffff, &mut kind, core::ptr::null_mut(), &mut size), registry::ERROR_FILE_NOT_FOUND);
        let mut name = [0u16; 16];
        let mut nlen = 16u32;
        assert_eq!(registry::reg_enum_value_w(h, 1, name.as_mut_ptr(), &mut nlen, core::ptr::null_mut(), &mut kind, core::ptr::null_mut(), core::ptr::null_mut()), 0);
        assert_eq!(String::from_utf16_lossy(&name[..nlen as usize]), "Num");
        nlen = 16;
        assert_eq!(registry::reg_enum_value_w(h, 2, name.as_mut_ptr(), &mut nlen, core::ptr::null_mut(), &mut kind, core::ptr::null_mut(), core::ptr::null_mut()), registry::ERROR_NO_MORE_ITEMS);
        assert_eq!(registry::reg_close_key(h), 0);
        assert_eq!(registry::reg_close_key(h), registry::ERROR_INVALID_HANDLE);
    }
}

#[test]
fn scm_queries_answer_from_the_in_process_host() {
    let _g = init();
    host::add_service("TomBoxSvc");
    unsafe {
        let scm = host::open_sc_manager_w(core::ptr::null(), core::ptr::null(), 1);
        assert_ne!(scm, 0);
        assert_eq!(host::open_service_w(scm, w("NoSuchSvc").as_ptr(), 4), 0);
        let s = host::open_service_w(scm, w("TomBoxSvc").as_ptr(), 4);
        assert_ne!(s, 0);
        let mut buf = [0u32; 9];
        let mut need = 0u32;
        assert_eq!(host::query_service_status_ex(s, 0, buf.as_mut_ptr() as *mut u8, 8, &mut need), 0);
        assert_eq!(need, 36);
        assert_eq!(host::query_service_status_ex(s, 0, buf.as_mut_ptr() as *mut u8, 36, &mut need), 1);
        assert_eq!(buf[1], host::SERVICE_STOPPED);
        assert_eq!(host::close_service_handle(s), 1);
        assert_eq!(host::close_service_handle(s), 0);
    }
}

#[test]
fn hostname_size_contract_matches_get_computer_name_ex() {
    let _g = init();
    registry::set_string(system::COMPUTER_NAME, "ComputerName", "SANDBOXPC");
    let mut b = [0u16; 4];
    let mut n = 4u32;
    unsafe {
        assert_eq!(system::get_computer_name_ex_w(4, b.as_mut_ptr(), &mut n), 0);
        assert_eq!(n, 10);
        let mut big = [0u16; 16];
        n = 16;
        assert_eq!(system::get_computer_name_ex_w(4, big.as_mut_ptr(), &mut n), 1);
        assert_eq!(String::from_utf16_lossy(&big[..n as usize]), "SANDBOXPC");
        assert_eq!(system::get_computer_name_ex_w(9, big.as_mut_ptr(), &mut n), 0);
    }
}
#[test]
fn import_walk_includes_delay_loads() {
    let _g = init();
    let m = console_image::load_path(std::path::Path::new(r"D:\xbox-pc-port\games\Fortnite\extracted\EmbeddedXvd\Windows\System32\netprofmsvc_c.dll"));
    if m == 0 {
        return;
    }
    let imps = unsafe { imports(m) };
    assert!(imps.iter().any(|(d, n)| d.eq_ignore_ascii_case("NSI.dll") && n == "NsiSetAllParameters"), "delay import listed");
}
#[test]
fn reg_query_enum_and_delete_stay_in_the_emulated_hive() {
    let _g = init();
    let key = r"\Registry\Machine\SOFTWARE\TomBoxRegW";
    registry::set(key, "Bin", registry::REG_BINARY, &[1, 2, 3]);
    registry::create_key(&format!(r"{key}\Beta"));
    registry::create_key(&format!(r"{key}\Alpha"));
    let mut h = 0usize;
    unsafe {
        assert_eq!(registry::reg_open_key_ex_w(registry::HKEY_LOCAL_MACHINE, w(r"SOFTWARE\TomBoxRegW").as_ptr(), 0, 0x20019, &mut h), 0);
        let (mut kind, mut size) = (0u32, 0u32);
        assert_eq!(regw::reg_query_value_ex_w(h, w("bin").as_ptr(), core::ptr::null_mut(), &mut kind, core::ptr::null_mut(), &mut size), 0);
        assert_eq!((kind, size), (registry::REG_BINARY, 3));
        let mut buf = [0u8; 2];
        size = 2;
        assert_eq!(regw::reg_query_value_ex_w(h, w("Bin").as_ptr(), core::ptr::null_mut(), core::ptr::null_mut(), buf.as_mut_ptr(), &mut size), registry::ERROR_MORE_DATA);
        assert_eq!(size, 3);
        let mut big = [0u8; 8];
        size = 8;
        assert_eq!(regw::reg_query_value_ex_w(h, w("Bin").as_ptr(), core::ptr::null_mut(), core::ptr::null_mut(), big.as_mut_ptr(), &mut size), 0);
        assert_eq!(&big[..3], &[1, 2, 3]);
        assert_eq!(regw::reg_query_value_ex_w(h, w("Nope").as_ptr(), core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut(), &mut size), registry::ERROR_FILE_NOT_FOUND);
        assert_eq!(regw::reg_query_value_ex_w(h, w("Bin").as_ptr(), core::ptr::null_mut(), core::ptr::null_mut(), big.as_mut_ptr(), core::ptr::null_mut()), registry::ERROR_INVALID_PARAMETER);
        let mut name = [0u16; 16];
        let mut n = 16u32;
        assert_eq!(regw::reg_enum_key_ex_w(h, 0, name.as_mut_ptr(), &mut n, core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut()), 0);
        assert_eq!(String::from_utf16_lossy(&name[..n as usize]), "Alpha");
        let mut small = [0u16; 3];
        n = 3;
        assert_eq!(regw::reg_enum_key_ex_w(h, 1, small.as_mut_ptr(), &mut n, core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut()), registry::ERROR_MORE_DATA);
        assert_eq!(regw::reg_enum_key_w(h, 1, name.as_mut_ptr(), 16), 0);
        assert_eq!(String::from_utf16_lossy(&name[..4]), "Beta");
        assert_eq!(regw::reg_enum_key_w(h, 2, name.as_mut_ptr(), 16), registry::ERROR_NO_MORE_ITEMS);
        assert_eq!(regw::reg_delete_value_w(h, w("Bin").as_ptr()), 0);
        assert_eq!(regw::reg_delete_value_w(h, w("Bin").as_ptr()), registry::ERROR_FILE_NOT_FOUND);
        assert!(registry::get(key, "Bin").is_none());
        assert_eq!(regw::reg_delete_key_w(registry::HKEY_LOCAL_MACHINE, w(r"SOFTWARE\TomBoxRegW").as_ptr()), regw::ERROR_ACCESS_DENIED);
        assert_eq!(regw::reg_delete_key_w(h, w("Alpha").as_ptr()), 0);
        assert!(!registry::key_exists(&format!(r"{key}\Alpha")));
        assert_eq!(regw::reg_delete_key_w(h, w("Alpha").as_ptr()), registry::ERROR_FILE_NOT_FOUND);
        assert_eq!(regw::reg_enum_key_w(h, 0, name.as_mut_ptr(), 16), 0);
        assert_eq!(String::from_utf16_lossy(&name[..4]), "Beta");
        registry::reg_close_key(h);
    }
}

static NOTIFIED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

unsafe extern "system" fn on_notify(buffer: *mut core::ffi::c_void) {
    let b = buffer as *const u8;
    let flag = (b.add(0x10) as *const *mut u8).read_unaligned();
    *flag = 1;
    NOTIFIED.store(((b.add(0x40) as *const u32).read_unaligned() as usize) << 8 | (b.add(0x20) as *const u32).read_unaligned() as usize, std::sync::atomic::Ordering::SeqCst);
}

#[link(name = "kernel32")]
extern "system" {
    fn SleepEx(ms: u32, alertable: i32) -> u32;
}

unsafe extern "system" fn no_handler(_c: u32, _t: u32, _d: *mut core::ffi::c_void, _x: *mut core::ffi::c_void) -> u32 {
    0
}

#[test]
fn scm_a_and_status_notifications_follow_win32() {
    let _g = init();
    unsafe {
        let scm = notify::open_sc_manager_a(core::ptr::null(), core::ptr::null(), 4);
        assert_ne!(scm, 0);
        assert_eq!(notify::open_service_a(scm, b"No/Such\0".as_ptr(), 5), 0);
        let rpcss = notify::open_service_a(scm, b"RpcSs\0".as_ptr(), 5);
        assert_ne!(rpcss, 0, "RpcSs is the live in-process RPC runtime");
        let mut flag = 0u8;
        let mut buf = [0u8; 0x50];
        (buf.as_mut_ptr() as *mut u32).write_unaligned(2);
        (buf.as_mut_ptr().add(8) as *mut usize).write_unaligned(on_notify as *const () as usize);
        (buf.as_mut_ptr().add(0x10) as *mut usize).write_unaligned(&mut flag as *mut u8 as usize);
        assert_eq!(notify::notify_service_status_change(rpcss, 0, buf.as_mut_ptr().cast()), host::ERROR_INVALID_PARAMETER);
        assert_eq!(notify::notify_service_status_change(0x1234, 8, buf.as_mut_ptr().cast()), host::ERROR_INVALID_HANDLE);
        assert_eq!(notify::notify_service_status_change(rpcss, 8, buf.as_mut_ptr().cast()), 0);
        assert_eq!(flag, 0, "the callback is an APC, not a direct call");
        assert_eq!(SleepEx(5000, 1), 0xc0, "WAIT_IO_COMPLETION");
        assert_eq!(flag, 1);
        assert_eq!(NOTIFIED.load(std::sync::atomic::Ordering::SeqCst), (8 << 8) | host::SERVICE_RUNNING as usize);
        assert_eq!(&buf[0x18..0x1c], &[0, 0, 0, 0]);

        host::add_service("TomBoxNotifySvc");
        let svc = notify::open_service_a(scm, b"TomBoxNotifySvc\0".as_ptr(), 5);
        assert_ne!(svc, 0);
        flag = 0;
        assert_eq!(notify::notify_service_status_change(svc, 8, buf.as_mut_ptr().cast()), 0);
        assert_eq!(SleepEx(0, 1), 0, "stopped service: nothing delivered yet");
        let name = w("TomBoxNotifySvc");
        let st = host::register_service_ctrl_handler_ex_w(name.as_ptr(), no_handler as *mut core::ffi::c_void, core::ptr::null_mut());
        assert_ne!(st, 0);
        let start_pending = [0x10u32, 2, 0, 0, 0, 0, 0];
        assert_eq!(host::set_service_status(st, start_pending.as_ptr()), 1);
        assert_eq!(SleepEx(0, 1), 0, "START_PENDING is not in the RUNNING mask");
        let running = [0x10u32, 4, 1, 0, 0, 0, 0];
        assert_eq!(host::set_service_status(st, running.as_ptr()), 1);
        assert_eq!(SleepEx(5000, 1), 0xc0);
        assert_eq!(flag, 1);
        flag = 0;
        assert_eq!(notify::notify_service_status_change(svc, 1, buf.as_mut_ptr().cast()), 0);
        assert_eq!(host::close_service_handle(svc), 1);
        let stopped = [0x10u32, 1, 0, 0, 0, 0, 0];
        assert_eq!(host::set_service_status(st, stopped.as_ptr()), 1);
        assert_eq!(SleepEx(0, 1), 0, "closing the handle cancels its notification");
        assert_eq!(flag, 0);
        host::close_service_handle(rpcss);
        host::close_service_handle(scm);
    }
}

#[link(name = "kernel32")]
extern "system" {
    fn CreateEventW(sa: *const core::ffi::c_void, manual: i32, initial: i32, name: *const u16) -> usize;
    fn WaitForSingleObject(h: usize, ms: u32) -> u32;
    fn CloseHandle(h: usize) -> i32;
    fn OpenProcessToken(p: usize, a: u32, h: *mut usize) -> i32;
    fn GetCurrentProcess() -> usize;
    fn GetLastError() -> u32;
}

fn open_xvcmgmt() -> usize {
    let name = w(xvcmgmt::DEVICE);
    let bytes = (name.len() - 1) * 2;
    let us: [usize; 2] = [bytes | (bytes << 16), name.as_ptr() as usize];
    let oa: [usize; 6] = [0x30, 0, us.as_ptr() as usize, 0x40, 0, 0];
    let mut h = 0usize;
    let mut io = [7usize; 2];
    assert_eq!(unsafe { xvcmgmt::nt_open_file(&mut h, 0x100003, oa.as_ptr() as *const u8, io.as_mut_ptr(), 3, 0) }, 0);
    assert_eq!(io, [0, 1], "IOSB: STATUS_SUCCESS, FILE_OPENED");
    assert!(xvcmgmt::is_device(h));
    h
}

struct Call {
    io: Box<[usize; 2]>,
    ev: usize,
}

impl Call {
    fn new() -> Call {
        Call { io: Box::new([0x55; 2]), ev: unsafe { CreateEventW(core::ptr::null(), 1, 0, core::ptr::null()) } }
    }
    fn run(&mut self, h: usize, code: u32, inb: &[u8], out: &mut [u8]) -> i32 {
        let o = if out.is_empty() { core::ptr::null_mut() } else { out.as_mut_ptr() };
        unsafe { xvcmgmt::nt_device_io_control_file(h, self.ev, 0, 0, self.io.as_mut_ptr(), code, inb.as_ptr(), inb.len() as u32, o, out.len() as u32) }
    }
    fn wait(&self, ms: u32) -> Option<(i32, usize)> {
        (unsafe { WaitForSingleObject(self.ev, ms) } == 0).then(|| (self.io[0] as u32 as i32, self.io[1]))
    }
}

impl Drop for Call {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.ev) };
    }
}

fn create(h: usize, vm: [u8; 16], svc: [u8; 16]) -> u64 {
    let mut inb = vm.to_vec();
    inb.extend_from_slice(&svc);
    let mut out = [0u8; 8];
    let mut c = Call::new();
    assert_eq!(c.run(h, xvcmgmt::IOCTL_CREATE, &inb, &mut out), 0);
    assert_eq!(c.wait(0), Some((0, 8)));
    u64::from_le_bytes(out)
}

fn pend(h: usize, code: u32, id: u64, out: &mut [u8]) -> Call {
    let mut c = Call::new();
    assert_eq!(c.run(h, code, &id.to_le_bytes(), out), xvcmgmt::STATUS_PENDING, "{code:#x} pends like the driver's worker");
    c
}

fn send(h: usize, id: u64, data: &[u8]) -> i32 {
    let mut inb = id.to_le_bytes().to_vec();
    inb.extend_from_slice(&(data.as_ptr() as u64).to_le_bytes());
    inb.extend_from_slice(&(data.len() as u32).to_le_bytes());
    Call::new().run(h, xvcmgmt::IOCTL_SEND, &inb, &mut [])
}

#[test]
fn xvcmgmt_queue_follows_the_driver() {
    let _g = init();
    let h = open_xvcmgmt();
    let svc = [0x42u8; 16];
    let mut c = Call::new();
    assert_eq!(c.run(h, xvcmgmt::IOCTL_CREATE, &[0; 0x1f], &mut [0; 8]), xvcmgmt::STATUS_INVALID_PARAMETER);
    assert_eq!(c.wait(0), None, "a synchronous failure leaves the event and IOSB alone");
    assert_eq!(c.run(h, xvcmgmt::IOCTL_RECEIVE, &7u64.to_le_bytes(), &mut []), xvcmgmt::STATUS_BUFFER_TOO_SMALL);
    assert_eq!(Call::new().run(h, xvcmgmt::IOCTL_ACCEPT, &[0; 8], &mut [0; 4]), xvcmgmt::STATUS_BUFFER_TOO_SMALL);
    assert_eq!(Call::new().run(h, xvcmgmt::IOCTL_SEND, &[0; 0x15], &mut []), xvcmgmt::STATUS_INVALID_PARAMETER);
    assert_eq!(Call::new().run(h, xvcmgmt::IOCTL_CLOSE, &0xdeadu64.to_le_bytes(), &mut []), 0, "close of an unknown id is STATUS_SUCCESS");
    assert_eq!(Call::new().run(h, xvcmgmt::IOCTL_CONNECT, &0xdeadu64.to_le_bytes(), &mut []), xvcmgmt::STATUS_INVALID_HANDLE);

    let lone = create(h, xvcmgmt::LOOPBACK_VM, [0x99; 16]);
    assert_eq!(pend(h, xvcmgmt::IOCTL_CONNECT, lone, &mut []).wait(5000), Some((xvcmgmt::STATUS_CONNECTION_REFUSED, 0)), "no listener");

    let server = create(h, xvcmgmt::LOOPBACK_VM, svc);
    let a = create(h, xvcmgmt::LOOPBACK_VM, svc);
    assert!(a > server && server >= 2);
    assert_eq!(pend(h, xvcmgmt::IOCTL_LISTEN, server, &mut []).wait(5000), Some((0, 0)));
    let mut accepted = [0u8; 8];
    let mut acc = Call::new();
    assert_eq!(acc.run(h, xvcmgmt::IOCTL_ACCEPT, &server.to_le_bytes(), &mut accepted), xvcmgmt::STATUS_PENDING);
    assert_eq!(pend(h, xvcmgmt::IOCTL_CONNECT, a, &mut []).wait(5000), Some((0, 0)));
    assert_eq!(acc.wait(5000), Some((0, 8)));
    let b = u64::from_le_bytes(accepted);
    assert!(b > a);

    let mut buf = [0u8; 4];
    let r = pend(h, xvcmgmt::IOCTL_RECEIVE, b, &mut buf);
    assert_eq!(r.wait(100), None, "receive stays pending until bytes arrive");
    assert_eq!(send(h, a, b"hello"), 0);
    assert_eq!(r.wait(5000), Some((0, 4)), "stream semantics: as many bytes as fit");
    assert_eq!(&buf, b"hell");
    let mut rest = [0u8; 16];
    assert_eq!(pend(h, xvcmgmt::IOCTL_RECEIVE, b, &mut rest).wait(5000), Some((0, 1)));
    assert_eq!(rest[0], b'o');

    assert_eq!(Call::new().run(h, xvcmgmt::IOCTL_SHUTDOWN, &a.to_le_bytes(), &mut []), 0);
    assert_eq!(pend(h, xvcmgmt::IOCTL_RECEIVE, b, &mut rest).wait(5000), Some((0, 0)), "graceful disconnect reads as 0 bytes");
    assert_eq!(Call::new().run(h, xvcmgmt::IOCTL_SHUTDOWN, &a.to_le_bytes(), &mut []), xvcmgmt::STATUS_NOT_CONNECTED);

    let mut pa = Call::new();
    assert_eq!(pa.run(h, xvcmgmt::IOCTL_ACCEPT, &server.to_le_bytes(), &mut accepted), xvcmgmt::STATUS_PENDING);
    assert_eq!(Call::new().run(h, xvcmgmt::IOCTL_CLOSE, &server.to_le_bytes(), &mut []), 0);
    assert_eq!(pa.wait(5000), Some((xvcmgmt::STATUS_LOCAL_DISCONNECT, 0)), "pending accepts complete with STATUS_LOCAL_DISCONNECT");

    let earlier = xvcmgmt::host_connections(claims::launch_instance());
    let host = create(h, xvcmgmt::HOST_VM, claims::launch_instance());
    assert_eq!(pend(h, xvcmgmt::IOCTL_CONNECT, host, &mut []).wait(5000), Some((0, 0)), "the emulated host listens on c12a416b + XBOX://LI");
    assert_eq!(Call::new().run(h, xvcmgmt::IOCTL_SHUTDOWN, &host.to_le_bytes(), &mut []), xvcmgmt::STATUS_NOT_IMPLEMENTED_PEER, "graceful disconnect is loopback-only");
    let conns: Vec<u64> = xvcmgmt::host_connections(claims::launch_instance()).into_iter().filter(|c| !earlier.contains(c)).collect();
    assert_eq!(conns.len(), 1);
    assert_eq!(send(h, host, b"to-host"), 0);
    assert_eq!(xvcmgmt::host_take_received(conns[0]), b"to-host");
    let mut got = [0u8; 8];
    let hr = pend(h, xvcmgmt::IOCTL_RECEIVE, host, &mut got);
    assert_eq!(xvcmgmt::host_send(conns[0], b"reply"), 0);
    assert_eq!(hr.wait(5000), Some((0, 5)));
    assert_eq!(&got[..5], b"reply");
    let hr2 = pend(h, xvcmgmt::IOCTL_RECEIVE, host, &mut got);
    xvcmgmt::host_close(conns[0]);
    assert_eq!(hr2.wait(5000), Some((xvcmgmt::STATUS_CONNECTION_RESET, 0)), "abortive peer close");
    assert_eq!(send(h, host, b"x"), xvcmgmt::STATUS_CONNECTION_INVALID);
    let cancelled = pend(h, xvcmgmt::IOCTL_RECEIVE, lone, &mut got);
    assert_eq!(Call::new().run(h, xvcmgmt::IOCTL_CLOSE, &lone.to_le_bytes(), &mut []), 0);
    assert_eq!(cancelled.wait(5000), Some((xvcmgmt::STATUS_CANCELLED, 0)), "close cancels in-flight receives");
    assert_eq!(Call::new().run(h, 0x150054, &[0; 8], &mut [0; 4]), xvcmgmt::STATUS_INVALID_DEVICE_REQUEST);
    let q = create(h, xvcmgmt::HOST_VM, claims::launch_instance());
    assert_eq!(pend(h, xvcmgmt::IOCTL_CONNECT, q, &mut []).wait(5000), Some((0, 0)));
    let mut rb = [0u8; 8];
    let mut rb2 = [0u8; 8];
    let one = pend(h, xvcmgmt::IOCTL_RECEIVE, q, &mut rb);
    let two = pend(h, xvcmgmt::IOCTL_RECEIVE, q, &mut rb2);
    assert_eq!(unsafe { xvcmgmt::cancel_io_ex(h, one.io.as_ptr() as *mut core::ffi::c_void) }, 1);
    assert_eq!(one.wait(5000), Some((xvcmgmt::STATUS_CANCELLED, 0)), "CancelIoEx cancels exactly that request");
    assert_eq!(two.wait(50), None);
    assert_eq!(unsafe { xvcmgmt::cancel_io_ex(h, one.io.as_ptr() as *mut core::ffi::c_void) }, 0, "nothing left to cancel: ERROR_NOT_FOUND");
    let mut iosb = [9usize; 2];
    assert_eq!(unsafe { xvcmgmt::nt_cancel_io_file(h, iosb.as_mut_ptr()) }, 0);
    assert_eq!(iosb, [0, 0]);
    assert_eq!(two.wait(5000), Some((xvcmgmt::STATUS_CANCELLED, 0)), "NtCancelIoFile cancels the calling thread's I/O on the file");
    assert_eq!(unsafe { xvcmgmt::nt_cancel_io_file_ex(h, core::ptr::null_mut(), iosb.as_mut_ptr()) }, xvcmgmt::STATUS_NOT_FOUND);
    Call::new().run(h, xvcmgmt::IOCTL_CLOSE, &q.to_le_bytes(), &mut []);
    let mut dup = 0usize;
    extern "system" {
        fn DuplicateHandle(sp: usize, sh: usize, tp: usize, th: *mut usize, access: u32, inherit: i32, opts: u32) -> i32;
    }
    let opened = xvcmgmt::device_handles();
    assert_ne!(unsafe { DuplicateHandle(GetCurrentProcess(), h, GetCurrentProcess(), &mut dup, 0, 0, 2) }, 0);
    assert!(xvcmgmt::is_device(dup), "a duplicated device handle still reaches the device");
    assert_eq!(unsafe { nt_close(h) }, 0);
    assert!(!xvcmgmt::is_device(h), "a closed handle value is no longer the device");
    assert_eq!(xvcmgmt::device_handles(), opened, "the file object lives while the duplicate is open");
    assert_eq!(Call::new().run(dup, xvcmgmt::IOCTL_CLOSE, &0u64.to_le_bytes(), &mut []), 0);
    assert_eq!(unsafe { xvcmgmt::close_handle(dup) }, 1);
    assert!(!xvcmgmt::is_device(dup));
    assert_eq!(xvcmgmt::device_handles(), opened - 1, "the last close releases the private reference");
}

#[test]
fn launch_claim_is_merged_into_the_process_token_attributes() {
    let _g = init();
    let mut need = 0u32;
    assert_eq!(unsafe { claims::nt_query_information_token(usize::MAX - 3, claims::TOKEN_SECURITY_ATTRIBUTES, core::ptr::null_mut(), 0, &mut need) }, 0xC000_0023u32 as i32);
    let mut buf = vec![0u64; need as usize / 8 + 1];
    assert_eq!(unsafe { claims::nt_query_information_token(usize::MAX - 3, claims::TOKEN_SECURITY_ATTRIBUTES, buf.as_mut_ptr() as *mut u8, need, &mut need) }, 0);
    let attrs = unsafe { claims::parse(buf.as_ptr() as usize) }.unwrap();
    let li: Vec<&claims::Attribute> = attrs.iter().filter(|a| String::from_utf16_lossy(&a.name) == "XBOX://LI").collect();
    assert_eq!(li.len(), 1);
    assert_eq!(li[0].kind, claims::TYPE_OCTET_STRING);
    assert_eq!(li[0].values, vec![claims::Value::Bytes(claims::launch_instance().to_vec())]);
    let real = attrs.len() - 1;
    let round = claims::merge(attrs.clone(), claims::emulated());
    assert_eq!(round.len(), real + 1, "merging again replaces, never duplicates");
    let mut again = vec![0u64; claims::size_of(&round) / 8 + 1];
    unsafe { claims::write(&round, again.as_mut_ptr() as usize) };
    assert_eq!(unsafe { claims::parse(again.as_ptr() as usize) }.unwrap(), round);
    assert_eq!(claims::parse_guid(&claims::format_guid(&claims::launch_instance())), Some(claims::launch_instance()));
    assert_eq!(claims::parse_guid("c12a416b-f347-405c-b765-2de027a6249e"), Some(xvcmgmt::HOST_VM));
    assert_eq!(claims::parse_guid("e0e16197-dd56-4a10-9195-5ee7a155a838"), Some(xvcmgmt::LOOPBACK_VM));
    assert!(unsafe { claims::is_process_token(usize::MAX - 3) });
    let mut other = 0usize;
    assert_ne!(unsafe { OpenProcessToken(GetCurrentProcess(), 8, &mut other) }, 0);
    assert!(unsafe { claims::is_process_token(other) });
    unsafe { CloseHandle(other) };
}

#[test]
fn gamecore_host_answers_each_request_with_the_exact_reply_length() {
    use crate::gchost::{frame, on_bytes, opened, parse, forget, CHANNEL_OPEN, DISPLAY_MODE_1, DISPLAY_MODE_2, PROXY_APP};
    let id = 0xFFFF_0001;
    let key = 7u64.to_le_bytes();
    let mut proxy = key.to_vec();
    proxy.extend_from_slice(&[0x11; 0x1c]);
    let mut wire = frame(0, CHANNEL_OPEN, &[0, 0, 0, 0, 1, 0]);
    wire.extend(frame(6, DISPLAY_MODE_1, &key));
    wire.extend(frame(8, DISPLAY_MODE_2, &key));
    wire.extend(frame(8, PROXY_APP, &proxy));
    wire.extend(frame(0x24, 0x0003_0007, &key));
    assert_eq!(&wire[14..16], &[0, 0], "a 6-byte payload is followed by 2 pad bytes");
    let (half, rest) = wire.split_at(17);
    assert!(on_bytes(id, half).is_empty(), "a partial frame waits for more bytes");
    let out = on_bytes(id, rest);
    let (replies, used, _) = parse(&out, 0);
    assert_eq!(used, out.len());
    let got: Vec<(u32, usize)> = replies.iter().map(|(k, p)| (*k, p.len())).collect();
    assert_eq!(got, vec![(DISPLAY_MODE_1, 0x14), (DISPLAY_MODE_2, 0x35), (PROXY_APP, 0x10)], "the unknown 0x30007 gets no reply");
    assert!(replies.iter().all(|(_, p)| p[..8] == key), "every reply echoes the request key");
    assert_eq!(&replies[2].1[8..0x10], &[0; 8], "S_OK and a zero result");
    assert_eq!(&replies[1].1[0x11..0x14], &[0; 3]);
    assert_eq!(opened(id), vec![1]);
    assert_eq!(crate::gcreply::reply(DISPLAY_MODE_1, &[0; 9]), None, "a malformed request is not answered");
    assert_eq!(crate::gcreply::reply(PROXY_APP, &key), None);
    forget(id);
    assert!(opened(id).is_empty());
}

#[test]
fn xvcmgmt_system_codes_follow_the_driver_on_a_fresh_boot() {
    use crate::xvcsys::*;
    let h = open_xvcmgmt();
    let mut c = Call::new();
    let mut q = [0xAAu8; 8];
    assert_eq!(c.run(h, IOCTL_MODE_SFR_COUNTER, &[], &mut q), 0);
    assert_eq!(c.wait(0), Some((0, 8)));
    assert_eq!(q, [0; 8], "SFR counter 0x27 starts clear: no tools/debug flags");
    assert_eq!(Call::new().run(h, IOCTL_MODE_SFR_COUNTER, &[], &mut [0u8; 7]), xvcmgmt::STATUS_INVALID_PARAMETER);

    let mut upd = Vec::new();
    for (counter, op, value) in [(0x27u32, 0u32, 0x13u64), (0x27, 1, 0x11), (0x27, 2, 0x100)] {
        upd.extend_from_slice(&counter.to_le_bytes());
        upd.extend_from_slice(&op.to_le_bytes());
        upd.extend_from_slice(&value.to_le_bytes());
    }
    assert_eq!(Call::new().run(h, IOCTL_SFR_UPDATE, &upd, &mut []), 0);
    let mut g = [0u8; 8];
    assert_eq!(Call::new().run(h, IOCTL_SFR_GET, &0x27u32.to_le_bytes(), &mut g), 0);
    assert_eq!(u64::from_le_bytes(g), 0x111);
    let mut bad = upd[..16].to_vec();
    bad.extend_from_slice(&[0xff, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(Call::new().run(h, IOCTL_SFR_UPDATE, &bad, &mut []), xvcmgmt::STATUS_INVALID_PARAMETER, "counter 0x1ff is out of range");
    assert_eq!(sfr(0x27), 0x13, "entries before the bad one are applied");
    assert_eq!(Call::new().run(h, IOCTL_SFR_UPDATE, &[0u8; 8], &mut []), xvcmgmt::STATUS_INVALID_PARAMETER);
    assert_eq!(Call::new().run(h, IOCTL_SFR_UPDATE, &[0u32.to_le_bytes(), 0u32.to_le_bytes()].concat(), &mut []), xvcmgmt::STATUS_INVALID_PARAMETER);
    assert_eq!(Call::new().run(h, IOCTL_SFR_UPDATE, &upd[..16], &mut []), 0);
    assert_eq!(Call::new().run(h, IOCTL_SFR_UPDATE, &[0x27, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], &mut []), 0);
    assert_eq!(sfr(0x27), 0);

    let mut a = [0xAAu8; 4];
    assert_eq!(Call::new().run(h, IOCTL_AUDIO_FLAG, &[], &mut a), 0);
    assert_eq!(a, [0; 4]);
    assert_eq!(Call::new().run(h, IOCTL_POWER_STATE, &1u32.to_le_bytes(), &mut []), 0);
    assert_eq!(power_state(), 1);
    assert_eq!(Call::new().run(h, IOCTL_POWER_STATE, &0u32.to_le_bytes(), &mut []), 0);
    assert_eq!(Call::new().run(h, IOCTL_PIX_STATE, &1u32.to_le_bytes(), &mut []), 0);
    assert_eq!(pix_state(), 1);
    assert_eq!(Call::new().run(h, IOCTL_PIX_STATE, &[1, 0], &mut []), xvcmgmt::STATUS_INVALID_PARAMETER);

    let ev = unsafe { CreateEventW(core::ptr::null(), 0, 0, core::ptr::null()) };
    let before = slot_count();
    let mut slot = [0u8; 2];
    let mut c = Call::new();
    assert_eq!(c.run(h, IOCTL_SLOT_REGISTER, &(ev as u64).to_le_bytes(), &mut slot), 0);
    assert_eq!(c.wait(0), Some((0, 2)));
    let n = u16::from_le_bytes(slot);
    assert!((1..32).contains(&n));
    assert_eq!(slot_count(), before + 1);
    assert_eq!(Call::new().run(h, IOCTL_SLOT_REGISTER, &0u64.to_le_bytes(), &mut [0u8; 2]), xvcmgmt::STATUS_INVALID_HANDLE);
    assert_eq!(Call::new().run(h, IOCTL_SLOT_REGISTER, &(ev as u64).to_le_bytes(), &mut [0u8; 4]), xvcmgmt::STATUS_INVALID_PARAMETER);
    let mut rec = [0xAAu8; 16];
    let mut c = Call::new();
    assert_eq!(c.run(h, IOCTL_SLOT_DEQUEUE, &n.to_le_bytes(), &mut rec), 0x8000_001Au32 as i32, "nothing pending: STATUS_NO_MORE_ENTRIES");
    assert_eq!(c.wait(0), Some((0x8000_001Au32 as i32, 0)), "a warning still completes the IOSB");
    assert_eq!(rec, [0; 16], "the record is zeroed first");
    assert_eq!(Call::new().run(h, IOCTL_SLOT_DEQUEUE, &0x7fu16.to_le_bytes(), &mut rec), xvcmgmt::STATUS_INVALID_PARAMETER);
    let mut ret = 0u32;
    let ok = unsafe { xvcmgmt::device_io_control(h, IOCTL_SLOT_DEQUEUE, n.to_le_bytes().as_ptr(), 2, rec.as_mut_ptr(), 16, &mut ret, core::ptr::null_mut()) };
    assert_eq!((ok, unsafe { GetLastError() }), (0, 259), "the client's drain loop stops on FALSE / ERROR_NO_MORE_ITEMS");
    assert_eq!(unsafe { WaitForSingleObject(ev, 0) }, 0x102, "nothing signals the event without a host");
    let mut un = [0u8; 16];
    un[..8].copy_from_slice(&0x1000u64.to_le_bytes());
    assert_eq!(Call::new().run(h, IOCTL_SLOT_UNMAP, &un, &mut []), xvcmgmt::STATUS_NOT_FOUND);
    assert_eq!(Call::new().run(h, IOCTL_SLOT_UNREGISTER, &n.to_le_bytes(), &mut []), 0);
    assert_eq!(slot_count(), before);
    assert_eq!(Call::new().run(h, IOCTL_SLOT_UNREGISTER, &n.to_le_bytes(), &mut []), 0, "an unknown slot still succeeds");
    unsafe { CloseHandle(ev) };

    let part = [0u32.to_le_bytes(), 7u32.to_le_bytes(), 0u32.to_le_bytes()].concat();
    assert_eq!(Call::new().run(h, IOCTL_PARTITION_INFO, &part, &mut [0u8; 0xc]), xvcmgmt::STATUS_NOT_FOUND);
    assert_eq!(Call::new().run(h, IOCTL_PARTITION_INFO, &part[..8], &mut [0u8; 0xc]), xvcmgmt::STATUS_BUFFER_TOO_SMALL);
    assert_eq!(Call::new().run(h, IOCTL_TRANSFER_MEMORY, &part, &mut [0u8; 0x28]), xvcmgmt::STATUS_NOT_FOUND);
    let mut c = Call::new();
    assert_eq!(c.run(h, IOCTL_TRANSFER_MEMORY, &part, &mut [0u8; 0xc]), 0x8000_0005u32 as i32);
    assert_eq!(c.wait(0), Some((0x8000_0005u32 as i32, 4)));
    let p1 = [0u32.to_le_bytes(), 1u32.to_le_bytes(), 0u32.to_le_bytes()].concat();
    assert_eq!(Call::new().run(h, IOCTL_TRANSFER_MEMORY, &p1, &mut [0u8; 0x28]), 0xC000_00BBu32 as i32);
    assert_eq!(Call::new().run(h, IOCTL_GAME_READY, &[], &mut []), 0);
    for code in [0x150054u32, 0x15005c] {
        assert_eq!(Call::new().run(h, code, &[0u8; 8], &mut []), xvcmgmt::STATUS_INVALID_DEVICE_REQUEST, "{code:#x} is unknown to this driver build");
    }
    unsafe { xvcmgmt::close_handle(h) };
}

#[test]
fn emulated_host_broker_serves_the_gamecore_classes() {
    let _g = init();
    use crate::gcdcom::*;
    use core::ffi::c_void;
    unsafe {
        let name: Vec<u16> = HOST_SERVER.to_ascii_lowercase().encode_utf16().chain(Some(0)).collect();
        let si = ServerInfo { reserved1: 0, name: name.as_ptr(), auth: core::ptr::null_mut(), reserved2: 1 };
        let probe = Guid { data1: 0xCFE77662, data2: 0xB1A5, data3: 0x49C9, data4: [0x9D, 0x1A, 0x98, 0xAF, 0x16, 0xCF, 0x72, 0xF2] };
        let mut mqi = [MultiQi { iid: &IID_API_SERVICE, itf: core::ptr::null_mut(), hr: 1 }, MultiQi { iid: &probe, itf: core::ptr::null_mut(), hr: 1 }];
        assert_eq!(co_create_instance_ex(&CLSID_API_SERVICE, core::ptr::null_mut(), CLSCTX_REMOTE_SERVER, &si, 2, mqi.as_mut_ptr()), CO_S_NOTALLINTERFACES);
        assert_eq!((mqi[0].hr, mqi[1].hr), (S_OK, E_NOINTERFACE));
        assert!(mqi[1].itf.is_null());
        let broker = mqi[0].itf;
        let vt = *(broker as *const *const usize);
        let get: unsafe extern "system" fn(*mut c_void, *const Guid, *const Guid, *const Guid, *mut *mut c_void) -> i32 = core::mem::transmute(*vt.add(3));
        let release: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(*vt.add(2));
        let li = claims::launch_instance();
        let launch = li.as_ptr() as *const Guid;
        let mut o = 0x1usize as *mut c_void;
        assert_eq!(get(broker, launch, &probe, &probe, &mut o), REGDB_E_CLASSNOTREG);
        assert!(o.is_null());
        assert_eq!(get(broker, launch, &ID_PROXY_CONTROLLER2, &probe, &mut o), E_NOINTERFACE);

        assert_eq!(get(broker, launch, &ID_PROXY_CONTROLLER, &ID_PROXY_CONTROLLER, &mut o), S_OK);
        let cvt = *(o as *const *const usize);
        let open: unsafe extern "system" fn(*mut c_void, *const Guid, u16, u64, *mut u32) -> i32 = core::mem::transmute(*cvt.add(3));
        let close: unsafe extern "system" fn(*mut c_void, *const Guid, u16) -> i32 = core::mem::transmute(*cvt.add(4));
        let mut out = 9u32;
        assert_eq!(open(o, launch, 3, 4, &mut out), S_OK);
        assert_eq!(out, 0);
        assert_eq!(close(o, launch, 3), S_OK);
        release(o);

        assert_eq!(get(broker, launch, &CLSID_DISPLAY_MODE_SERVER, &IID_DISPLAY_MODE_SERVER3, &mut o), S_OK);
        let dvt = *(o as *const *const usize);
        let caps: unsafe extern "system" fn(*mut c_void, *mut DisplayCaps) -> i32 = core::mem::transmute(*dvt.add(6));
        let mut c = DisplayCaps { a: 1.0, b: 1.0, flags: 0xff };
        assert_eq!(caps(o, &mut c), S_OK);
        assert_eq!(c, display_caps_value());
        let qi: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32 = core::mem::transmute(*dvt.add(0));
        let mut insp = core::ptr::null_mut();
        assert_eq!(qi(o, &IID_IINSPECTABLE, &mut insp), S_OK);
        release(insp);
        release(o);
        release(broker);
        assert_eq!(all_refs(), [0; 4]);
        let mut m = [MultiQi { iid: &IID_API_SERVICE, itf: core::ptr::null_mut(), hr: 1 }];
        assert_eq!(co_create_instance_ex(&probe, core::ptr::null_mut(), CLSCTX_REMOTE_SERVER, &si, 1, m.as_mut_ptr()), REGDB_E_CLASSNOTREG);
    }
}

#[test]
fn host_serves_audio_cursor_and_hdr_classes() {
    let _g = init();
    use crate::gcdcom::*;
    use core::ffi::c_void;
    unsafe {
        let name: Vec<u16> = HOST_SERVER.encode_utf16().chain(Some(0)).collect();
        let si = ServerInfo { reserved1: 0, name: name.as_ptr(), auth: core::ptr::null_mut(), reserved2: 1 };
        let mut mqi = [MultiQi { iid: &IID_API_SERVICE, itf: core::ptr::null_mut(), hr: 1 }];
        assert_eq!(co_create_instance_ex(&CLSID_API_SERVICE, core::ptr::null_mut(), CLSCTX_REMOTE_SERVER, &si, 1, mqi.as_mut_ptr()), S_OK);
        let broker = mqi[0].itf;
        let slot = |o: *mut c_void, n: usize| *(*(o as *const *const usize)).add(n);
        let get: unsafe extern "system" fn(*mut c_void, *const Guid, *const Guid, *const Guid, *mut *mut c_void) -> i32 = core::mem::transmute(slot(broker, 3));
        let release: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(slot(broker, 2));
        let li = claims::launch_instance();
        let launch = li.as_ptr() as *const Guid;

        let mut a = core::ptr::null_mut();
        assert_eq!(get(broker, launch, &ID_CROSS_VM_AUDIO, &ID_CROSS_VM_AUDIO, &mut a), S_OK);
        let call: unsafe extern "system" fn(*mut c_void, i32, *mut u8) -> i32 = core::mem::transmute(slot(a, 3));
        let ack: unsafe extern "system" fn(*mut c_void, i16) -> i32 = core::mem::transmute(slot(a, 4));
        let mut env = vec![0u8; AUDIO_ENVELOPE as usize];
        env[8..12].copy_from_slice(&crate::crossvm::CREATE_OBJECT.to_le_bytes());
        assert_eq!(call(a, AUDIO_ENVELOPE, env.as_mut_ptr()), S_OK);
        assert_eq!(i32::from_le_bytes(env[4..8].try_into().unwrap()), S_OK, "CreateObject succeeds");
        assert_eq!(u32::from_le_bytes(env[8..12].try_into().unwrap()), crate::crossvm::CREATE_OBJECT, "the call id is echoed");
        let created = u32::from_le_bytes(env[0x1c..0x20].try_into().unwrap());
        assert_ne!(created, 0, "the new object id sits at env+0x1c");
        let mut env = vec![0u8; AUDIO_ENVELOPE as usize];
        env[8..12].copy_from_slice(&0x47u32.to_le_bytes());
        assert_eq!(call(a, AUDIO_ENVELOPE, env.as_mut_ptr()), S_OK);
        assert_eq!(i32::from_le_bytes(env[4..8].try_into().unwrap()), E_NOTIMPL, "ids with no decoded layout report E_NOTIMPL in the envelope");
        let mut env = vec![0u8; AUDIO_ENVELOPE as usize];
        env[8..12].copy_from_slice(&crate::crossvm::RELEASE_OBJECT.to_le_bytes());
        env[0xc..0x10].copy_from_slice(&created.to_le_bytes());
        assert_eq!(call(a, AUDIO_ENVELOPE, env.as_mut_ptr()), S_OK);
        assert_eq!(i32::from_le_bytes(env[4..8].try_into().unwrap()), S_OK);
        assert_eq!(call(a, 8, env.as_mut_ptr()), E_INVALIDARG);
        assert_eq!(ack(a, 1), S_OK);
        release(a);

        let mut c = core::ptr::null_mut();
        assert_eq!(get(broker, launch, &ID_CURSOR_CONTROLLER, &ID_CURSOR_CONTROLLER, &mut c), S_OK);
        let kind: unsafe extern "system" fn(*mut c_void, i16) -> i32 = core::mem::transmute(slot(c, 3));
        let image: unsafe extern "system" fn(*mut c_void, i32, i32, i32, i32, *const u8, i32) -> i32 = core::mem::transmute(slot(c, 4));
        assert_eq!(kind(c, 3), S_OK);
        let st = cursor_state();
        assert_eq!(st.kind, 3);
        assert_ne!(st.handle, 0, "hand maps to the PC IDC_HAND cursor");
        assert_eq!(kind(c, -1), S_OK);
        assert_eq!(cursor_state().handle, 0, "-1 hides the cursor");
        assert_eq!(kind(c, 99), E_INVALIDARG);
        let px = vec![0xFFu8; 16 * 16 * 4];
        assert_eq!(image(c, 16, 16, 1, 1, px.as_ptr(), px.len() as i32), S_OK);
        assert_eq!(image(c, 16, 16, 1, 1, px.as_ptr(), 10), E_INVALIDARG);
        assert_eq!(kind(c, 2), S_OK);
        let st = cursor_state();
        assert!(st.custom != 0 && st.handle == st.custom, "custom shows the uploaded image");
        assert_eq!(kind(c, 0), S_OK);
        release(c);

        let mut d = core::ptr::null_mut();
        assert_eq!(get(broker, launch, &CLSID_DISPLAY_MODE_SERVER, &IID_DISPLAY_MODE_SERVER4, &mut d), S_OK);
        let hdr: unsafe extern "system" fn(*mut c_void, u32, *mut HdrInfo) -> i32 = core::mem::transmute(slot(d, 6));
        let mut info = HdrInfo { min: 9.0, ..Default::default() };
        let mut floats = [0f32; 3];
        let want = compat_common::gdk::basic::try_enable_hdr(0, floats.as_mut_ptr());
        let hr = hdr(d, 0, &mut info);
        if want == compat_common::gdk::basic::HDR_UNKNOWN {
            assert_eq!(hr, E_FAIL);
        } else {
            assert_eq!(hr, S_OK);
            assert_eq!(info.enabled, (want == compat_common::gdk::basic::HDR_ENABLED) as u8, "IServer4 answers from the PC display HDR state");
        }
        assert_eq!(hdr(d, 7, &mut info), E_INVALIDARG);
        release(d);
        release(broker);
        assert_eq!(more_refs(), [0; 3]);
        assert_eq!(all_refs(), [0; 4]);
    }
}

#[test]
fn logan_audio_pool_table_follows_the_guest_format() {
    use crate::logan::*;
    let base = pool();
    assert_ne!(base, 0);
    let a = pool_alloc(3).unwrap() as usize;
    assert!(a >= FIRST_PAGE);
    assert_eq!(pool_entry(a), 3 | (1 << REF_SHIFT));
    assert_eq!((pool_entry(a + 1), pool_entry(a + 2)), (CONTINUATION, CONTINUATION), "every page of a run is marked");
    let b = pool_alloc(2).unwrap() as usize;
    assert!(b >= a + 3);
    pool_release(a);
    assert_eq!((pool_entry(a), pool_entry(a + 1), pool_entry(a + 2)), (0, 0, 0));
    assert_eq!(pool_alloc(0), None);
    pool_release(b);
    let h = unsafe { open() };
    assert!(h != -1 && is_device(h as usize));
    let mut inb = [0u8; 0x1c];
    inb[0] = 2;
    let mut out = [0u8; 0x18];
    let mut ret = 0u32;
    assert_eq!(unsafe { crate::xvcmgmt::device_io_control(h as usize, IOCTL_MAP_REGION, inb.as_ptr(), 0x1c, out.as_mut_ptr(), 0x18, &mut ret, core::ptr::null_mut()) }, 1);
    assert_eq!(u64::from_le_bytes(out[0..8].try_into().unwrap()) as usize, base);
    assert_eq!(u32::from_le_bytes(out[8..12].try_into().unwrap()) as usize, POOL_SIZE);
    assert_eq!(u32::from_le_bytes(inb[0x18..0x1c].try_into().unwrap()), PARTITION_ID);
    inb[0] = 1;
    assert_eq!(unsafe { crate::xvcmgmt::device_io_control(h as usize, IOCTL_MAP_REGION, inb.as_ptr(), 0x1c, out.as_mut_ptr(), 0x18, &mut ret, core::ptr::null_mut()) }, 0);
    unsafe { crate::xvcmgmt::close_handle(h as usize) };
    assert!(!is_device(h as usize));

    let id = 0xFFFF_0002;
    let out = crate::gchost::on_bytes(id, &crate::gchost::frame(0, 0x0002_0008, &[]));
    let (r, _, _) = crate::gchost::parse(&out, 0);
    assert_eq!(r, vec![(0x0002_0000, vec![])], "the host acks the audio hello with an empty short<<16 frame");
    crate::gchost::forget(id);
}

#[test]
fn host_pushes_input_records_to_the_title_queue() {
    let _g = init();
    let h = open_xvcmgmt();
    let q = create(h, xvcmgmt::HOST_VM, claims::launch_instance());
    assert_eq!(pend(h, xvcmgmt::IOCTL_CONNECT, q, &mut []).wait(5000), Some((0, 0)));
    let conns = xvcmgmt::host_connections(claims::launch_instance());
    let host = *conns.last().unwrap();
    assert_eq!(crate::gchost::push_input(host, &[(0x100, 0x41, 0x1e0001), (0x101, 0x41, 0xc01e0001)]), 0);
    assert_eq!(crate::gchost::push(host, 0x0002_0000, &[]), 0);
    let mut got = vec![0u8; 256];
    let c = pend(h, xvcmgmt::IOCTL_RECEIVE, q, &mut got);
    let (st, n) = c.wait(5000).unwrap();
    assert_eq!(st, 0);
    let (msgs, used, _) = crate::gchost::parse(&got[..n], 0);
    assert_eq!(used, n);
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0].0, crate::gchost::WINDOW_MESSAGING);
    assert_eq!(msgs[0].1.len(), 48, "two 24-byte {{msg, pad, wParam, lParam}} records");
    assert_eq!(&msgs[0].1[0..4], &0x100u32.to_le_bytes());
    assert_eq!(&msgs[0].1[8..16], &0x41u64.to_le_bytes());
    assert_eq!(msgs[1], (0x0002_0000, vec![]));
    assert_eq!(Call::new().run(h, xvcmgmt::IOCTL_CLOSE, &q.to_le_bytes(), &mut []), 0);
    unsafe { xvcmgmt::close_handle(h) };
}

#[test]
fn host_serves_package_v7_and_net_security_classes() {
    let _g = init();
    use crate::gcdcom::*;
    use core::ffi::c_void;
    unsafe {
        let name: Vec<u16> = HOST_SERVER.encode_utf16().chain(Some(0)).collect();
        let si = ServerInfo { reserved1: 0, name: name.as_ptr(), auth: core::ptr::null_mut(), reserved2: 1 };
        let mut mqi = [MultiQi { iid: &IID_API_SERVICE, itf: core::ptr::null_mut(), hr: 1 }];
        assert_eq!(co_create_instance_ex(&CLSID_API_SERVICE, core::ptr::null_mut(), CLSCTX_REMOTE_SERVER, &si, 1, mqi.as_mut_ptr()), S_OK);
        let broker = mqi[0].itf;
        let slot = |o: *mut c_void, n: usize| *(*(o as *const *const usize)).add(n);
        let get: unsafe extern "system" fn(*mut c_void, *const Guid, *const Guid, *const Guid, *mut *mut c_void) -> i32 = core::mem::transmute(slot(broker, 3));
        let release: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(slot(broker, 2));
        let qi: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32 = core::mem::transmute(slot(broker, 0));
        let li = claims::launch_instance();
        let launch = li.as_ptr() as *const Guid;
        let before = server_refs();

        let mut v7 = core::ptr::null_mut();
        assert_eq!(get(broker, launch, &CLSID_PACKAGE_V7, &IID_PACKAGE_V7, &mut v7), S_OK);
        let mut agile = core::ptr::null_mut();
        assert_eq!(qi(v7, &IID_IAGILE_OBJECT, &mut agile), S_OK, "IAgileObject keeps RoGetAgileReference off the marshalling path");
        release(agile);
        let pkg: Vec<u16> = compat_common::gdk::package::package_id().encode_utf16().chain(Some(0)).collect();
        let other: Vec<u16> = "NotThisPackage".encode_utf16().chain(Some(0)).collect();
        let estimate: unsafe extern "system" fn(*mut c_void, *const u16, u32, *const c_void, *mut DownloadEstimate) -> i32 = core::mem::transmute(slot(v7, 11));
        let mut est = DownloadEstimate { bytes: 7, confirm: 7 };
        assert_eq!(estimate(v7, pkg.as_ptr(), 0, core::ptr::null(), &mut est), S_OK);
        assert_eq!((est.bytes, est.confirm), (0, 0), "a fully installed package needs no download");
        assert_eq!(estimate(v7, other.as_ptr(), 0, core::ptr::null(), &mut est), compat_common::gdk::E_NOT_FOUND);
        let path: unsafe extern "system" fn(*mut c_void, *const u16, i32, *mut *mut c_void) -> i32 = core::mem::transmute(slot(v7, 8));
        let mut h = core::ptr::null_mut();
        assert_eq!(path(v7, pkg.as_ptr(), 0, &mut h), S_OK);
        assert_eq!(String::from_utf16_lossy(&hstring_units(h)), compat_common::gdk::package::mount_path(), "slot 8 returns the package path as an HSTRING");
        assert_eq!(path(v7, other.as_ptr(), 0, &mut h), compat_common::gdk::E_NOT_FOUND);
        let locale: unsafe extern "system" fn(*mut c_void, *const u16, *mut *mut c_void) -> i32 = core::mem::transmute(slot(v7, 15));
        assert_eq!(locale(v7, pkg.as_ptr(), &mut h), S_OK);
        assert_eq!(String::from_utf16_lossy(&hstring_units(h)), compat_common::gdk::package::user_locale());
        let features: unsafe extern "system" fn(*mut c_void, *const u16, i32, *mut *mut c_void) -> i32 = core::mem::transmute(slot(v7, 26));
        assert_eq!(features(v7, pkg.as_ptr(), 2, &mut h), S_OK);
        assert_eq!(hstring_units(h), features_record(compat_common::gdk::package::features()), "slot 26 carries the five-field feature records");
        let rec = features_record(&[compat_common::gdk::package::Feature { id: "a".into(), display: "A".into(), tags: "t".into(), hidden: true, store_id: "s".into() }]);
        assert_eq!(String::from_utf16_lossy(&rec), "a A T t s ");
        let uninstall: unsafe extern "system" fn(*mut c_void, *const u16) -> i32 = core::mem::transmute(slot(v7, 27));
        assert_eq!(uninstall(v7, pkg.as_ptr()), E_ACCESSDENIED, "the running package is the read-only dump and is never removed");
        assert_eq!(uninstall(v7, other.as_ptr()), compat_common::gdk::E_NOT_FOUND);
        let details: unsafe extern "system" fn(*mut c_void, *const u16, *mut PkgDetails) -> i32 = core::mem::transmute(slot(v7, 29));
        assert_eq!(core::mem::size_of::<PkgDetails>(), 0x48);
        let mut d: PkgDetails = core::mem::zeroed();
        assert_eq!(details(v7, pkg.as_ptr(), &mut d), S_OK);
        assert_eq!(String::from_utf16_lossy(&hstring_units(d.id)), compat_common::gdk::package::package_id());
        let t = compat_common::gdk::title();
        assert_eq!(d.version, [t.version[3], t.version[2], t.version[1], t.version[0]], "PkgDetails +0x10 is rev, build, minor, major");
        assert_eq!(String::from_utf16_lossy(&hstring_units(d.display)), t.display);
        let enumerate: unsafe extern "system" fn(*mut c_void, *const u16, i32, i32, *mut u32, *mut *mut PkgDetails) -> i32 = core::mem::transmute(slot(v7, 28));
        let (mut n, mut arr) = (9u32, core::ptr::null_mut());
        assert_eq!(enumerate(v7, core::ptr::null(), 0, 0, &mut n, &mut arr), S_OK);
        assert_eq!(n, 1);
        assert_eq!(String::from_utf16_lossy(&hstring_units((*arr).id)), compat_common::gdk::package::package_id());
        assert_eq!(enumerate(v7, core::ptr::null(), 1, 0, &mut n, &mut arr), S_OK);
        assert_eq!((n, arr.is_null()), (0, true), "no content packages are installed");
        let undecoded: unsafe extern "system" fn(*mut c_void) -> i32 = core::mem::transmute(slot(v7, 30));
        assert_eq!(undecoded(v7), E_NOTIMPL, "undecoded slots fail per call, never with a retryable RPC code");
        release(v7);

        let mut aux = core::ptr::null_mut();
        assert_eq!(get(broker, launch, &CLSID_PACKAGE_AUX, &IID_PACKAGE_AUX, &mut aux), REGDB_E_CLASSNOTREG, "d657678e is served in-process by the console, never through the broker");

        let mut net = core::ptr::null_mut();
        assert_eq!(get(broker, launch, &CLSID_NET_SECURITY, &IID_NET_SECURITY, &mut net), S_OK);
        let mut insp = core::ptr::null_mut();
        assert_eq!(qi(net, &IID_IINSPECTABLE, &mut insp), S_OK);
        release(insp);
        let query: unsafe extern "system" fn(*mut c_void, *mut c_void, u64, u64, *mut *mut c_void) -> i32 = core::mem::transmute(slot(net, 6));
        let mut r = core::ptr::null_mut();
        assert_eq!(query(net, core::ptr::null_mut(), 0, 0, &mut r), S_OK);
        let flags: unsafe extern "system" fn(*mut c_void, *mut [u32; 4]) -> i32 = core::mem::transmute(slot(r, 6));
        let mut f = [0u32; 4];
        assert_eq!(flags(r, &mut f), S_OK);
        assert_eq!(f[0], TLS_FLAGS, "same TLS 1.2/1.3 flags the gdk net.rs provider reports");
        for s in [7usize, 9, 10, 12] {
            let list: unsafe extern "system" fn(*mut c_void, *mut usize, *mut *mut c_void) -> i32 = core::mem::transmute(slot(r, s));
            let (mut n, mut p) = (9usize, 1usize as *mut c_void);
            assert_eq!(list(r, &mut n, &mut p), S_OK);
            assert_eq!((n, p.is_null()), (0, true), "slot {s}: no pinning entries");
        }
        for s in [8usize, 11] {
            let b: unsafe extern "system" fn(*mut c_void, *mut u8) -> i32 = core::mem::transmute(slot(r, s));
            let mut v = 9u8;
            assert_eq!(b(r, &mut v), S_OK);
            assert_eq!(v, 0);
        }
        release(r);
        release(net);
        release(broker);
        assert_eq!(server_refs(), before, "every served object is released back to its starting count");
    }
}

#[test]
fn virtual_audio_bus_creates_named_cross_vm_events() {
    let _g = init();
    #[link(name = "kernel32")]
    extern "system" {
        fn SetEvent(h: isize) -> i32;
        fn WaitForSingleObject(h: isize, ms: u32) -> u32;
    }
    unsafe {
        let path: Vec<u16> = crate::vab::DEVICE_PATH.encode_utf16().chain(Some(0)).collect();
        let h = crate::fs::create_file_w(path.as_ptr(), 0xC000_0000, 0, core::ptr::null(), 3, 0, 0);
        assert_ne!(h, -1);
        let base = format!("{}{}", crate::vab::GAMECORE_BASE, std::process::id() % 1000);
        let mut inb = vec![0u8; crate::vab::CREATE_EVENT_INPUT as usize];
        for (i, c) in base.encode_utf16().enumerate() {
            inb[i * 2..i * 2 + 2].copy_from_slice(&c.to_le_bytes());
        }
        inb[0x208..0x20c].copy_from_slice(&1u32.to_le_bytes());
        let mut ret = 7u32;
        assert_eq!(crate::xvcmgmt::device_io_control(h as usize, crate::vab::IOCTL_CREATE_EVENT, inb.as_ptr(), inb.len() as u32, core::ptr::null_mut(), 0, &mut ret, core::ptr::null_mut()), 1);
        assert_eq!(ret, 0);
        assert_eq!(crate::xvcmgmt::device_io_control(h as usize, crate::vab::IOCTL_CREATE_EVENT, inb.as_ptr(), inb.len() as u32, core::ptr::null_mut(), 0, &mut ret, core::ptr::null_mut()), 0);
        assert_eq!(GetLastError(), 183, "a second create reports ERROR_ALREADY_EXISTS, which the guest tolerates");
        assert_eq!(crate::xvcmgmt::device_io_control(h as usize, crate::vab::IOCTL_CREATE_EVENT, inb.as_ptr(), 0x20, core::ptr::null_mut(), 0, &mut ret, core::ptr::null_mut()), 0);
        assert_eq!(GetLastError(), 87);
        let full = format!("{base}{}", crate::vab::SUFFIX);
        let wide: Vec<u16> = full.encode_utf16().chain(Some(0)).collect();
        let g = crate::vab::open_event_w(0x1F_0003, 1, wide.as_ptr());
        assert_ne!(g, 0, "the guest opener finds {full}");
        assert!(crate::vab::opened_names().contains(&full));
        let host = crate::vab::event(&full).unwrap();
        assert_eq!(SetEvent(host), 1);
        assert_eq!(WaitForSingleObject(g, 1000), 0, "host and guest share the same event object");
        assert_eq!(WaitForSingleObject(g, 0), 258, "auto-reset");
        crate::xvcmgmt::close_handle(g as usize);
        crate::xvcmgmt::close_handle(h as usize);
        assert!(!crate::vab::is_device(h as usize));
    }
}
