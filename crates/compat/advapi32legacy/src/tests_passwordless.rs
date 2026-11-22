use super::passwordless::*;
use std::path::PathBuf;

const XD_LSASRV: &str = r"D:\xbox-pc-port\games\XDefiant\extracted\EmbeddedXvd\Windows\System32\lsasrv.dll";
const XD_ADVAPI32LEGACY: &str = r"D:\xbox-pc-port\games\XDefiant\extracted\EmbeddedXvd\Windows\System32\advapi32legacy.dll";

fn temp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("tombox_pwdless_{}_{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn rva_bytes(path: &str, rva: u32, n: usize) -> Option<Vec<u8>> {
    let f = std::fs::read(path).ok()?;
    let pe = u32::from_le_bytes(f[0x3c..0x40].try_into().unwrap()) as usize;
    let nsec = u16::from_le_bytes(f[pe + 6..pe + 8].try_into().unwrap()) as usize;
    let opt = u16::from_le_bytes(f[pe + 20..pe + 22].try_into().unwrap()) as usize;
    let sec = pe + 24 + opt;
    for i in 0..nsec {
        let s = sec + i * 40;
        let va = u32::from_le_bytes(f[s + 12..s + 16].try_into().unwrap());
        let vs = u32::from_le_bytes(f[s + 8..s + 12].try_into().unwrap());
        let raw = u32::from_le_bytes(f[s + 20..s + 24].try_into().unwrap());
        if rva >= va && rva < va + vs {
            let o = (raw + rva - va) as usize;
            return Some(f[o..o + n].to_vec());
        }
    }
    None
}

#[test]
fn opnum_policy_matches_xdefiant_lsasrv_table() {
    let Some(b) = rva_bytes(XD_LSASRV, LSARPC_POLICY_RVA, 8 * (LSARPC_OPNUM_LIMIT as usize + 1)) else {
        eprintln!("{XD_LSASRV} not present; table checked against the embedded copy only");
        assert_eq!(LSARPC_POLICY_7E, OpnumPolicy { protseq_mask: 8, flags: 3 });
        return;
    };
    for op in 0..LSARPC_OPNUM_LIMIT as usize {
        let o = op * 8;
        let flags = u32::from_le_bytes(b[o + 4..o + 8].try_into().unwrap());
        assert!(flags < 0x100, "opnum {op:#x} flags {flags:#x}");
        assert_eq!(LSARPC_POLICY[op], OpnumPolicy { protseq_mask: u32::from_le_bytes(b[o..o + 4].try_into().unwrap()), flags: flags as u8 }, "opnum {op:#x}");
    }
    let sentinel = LSARPC_OPNUM_LIMIT as usize * 8;
    assert_eq!(u32::from_le_bytes(b[sentinel + 4..sentinel + 8].try_into().unwrap()), u32::MAX);
    assert_eq!(LSARPC_POLICY_7E, OpnumPolicy { protseq_mask: 8, flags: 3 });
    assert_eq!(LSARPC_POLICY_7F, OpnumPolicy { protseq_mask: 8, flags: 3 });
    let code = rva_bytes(XD_LSASRV, 0x395e2, 9).unwrap();
    assert_eq!(&code[..5], &[0xb8, 0x86, 0x00, 0x00, 0x00]);
}

#[test]
fn caller_token_is_the_real_thread_or_process_token() {
    let t = TitleToken::caller();
    assert_eq!(t.membership_error, 0);
    assert!(t.user.starts_with("S-1-"), "{}", t.user);
    assert_ne!(t.user, "", "caller SID");
    assert!(t.check_membership(SID_AUTHENTICATED_USERS) || t.user == "S-1-5-18");
    assert!(!t.is_app_container);
    assert_eq!(security_callback(&CallAttributes::console_client(OPNUM_ENABLE), opnum_policy(OPNUM_ENABLE), &t), 0);
}

#[test]
fn user_config_branch_writes_under_the_caller_sid() {
    let d = temp("callersid");
    let h = Hklm::at(&d);
    h.create_key(IMAGE_CUSTOMIZATION_KEY).unwrap();
    h.set_dword(IMAGE_CUSTOMIZATION_KEY, FORCE_DISABLE_VALUE, 1).unwrap();
    let t = TitleToken::caller();
    assert_eq!(lsarpc_call(&h, OPNUM_ENABLE, &CallAttributes::console_client(OPNUM_ENABLE), &t), 0);
    assert_eq!(h.get_dword(&format!("{USER_CONFIG_KEY}{}", t.user), USER_CONFIG_VALUE), Ok(1));
    if t.user != "S-1-5-18" {
        assert!(h.get_dword(&format!("{USER_CONFIG_KEY}S-1-5-18"), USER_CONFIG_VALUE).is_err());
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn token_without_authenticated_users_is_denied() {
    let t = TitleToken::caller();
    let stripped = TitleToken { groups: t.groups.iter().filter(|g| *g != SID_AUTHENTICATED_USERS).cloned().collect(), user: "S-1-5-21-1-2-3-500".into(), ..t };
    assert_eq!(security_callback(&CallAttributes::console_client(OPNUM_ENABLE), opnum_policy(OPNUM_ENABLE), &stripped), ERROR_ACCESS_DENIED);
}

#[test]
fn console_client_binding_matches_console_security_template() {
    let Some(b) = rva_bytes(XD_ADVAPI32LEGACY, 0x41b00, 0x18) else { return };
    let c = CallAttributes::console_client(OPNUM_ENABLE);
    assert_eq!(u32::from_le_bytes(b[0x10..0x14].try_into().unwrap()), c.authentication_level);
    assert_eq!(u32::from_le_bytes(b[0x14..0x18].try_into().unwrap()), c.authentication_service);
    let name = rva_bytes(XD_ADVAPI32LEGACY, 0x4f960, 30).unwrap();
    let s: Vec<u16> = name.chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    assert_eq!(String::from_utf16_lossy(&s), "LSARPC_ENDPOINT");
}

#[test]
fn security_callback_follows_console_gate() {
    let t = TitleToken::console_title();
    let p = opnum_policy(OPNUM_ENABLE);
    let ok = CallAttributes::console_client(OPNUM_ENABLE);
    assert_eq!(security_callback(&ok, p, &t), 0);
    assert_eq!(security_callback(&CallAttributes::console_client(OPNUM_DISABLE), opnum_policy(OPNUM_DISABLE), &t), 0);
    assert_eq!(security_callback(&CallAttributes { protocol_sequence: 1, ..ok }, p, &t), RPC_S_PROTSEQ_NOT_SUPPORTED);
    assert_eq!(security_callback(&CallAttributes { is_client_local: 0, ..ok }, p, &t), ERROR_ACCESS_DENIED);
    for level in 2..=4 {
        assert_eq!(security_callback(&CallAttributes { authentication_level: level, ..ok }, p, &t), ERROR_ACCESS_DENIED);
    }
    assert_eq!(security_callback(&CallAttributes { authentication_level: 5, ..ok }, p, &t), 0);
    assert_eq!(security_callback(&CallAttributes { opnum: 0x86, ..ok }, opnum_policy(0x86), &t), RPC_S_PROCNUM_OUT_OF_RANGE);
    assert_eq!(security_callback(&CallAttributes { opnum: 0x81, ..ok }, opnum_policy(0x81), &t), 0);
    assert_eq!(security_callback(&ok, Some(OpnumPolicy { protseq_mask: 8, flags: 7 }), &t), RPC_S_PROCNUM_OUT_OF_RANGE);
    let app = TitleToken { is_app_container: true, ..t.clone() };
    assert_eq!(security_callback(&ok, p, &app), ERROR_ACCESS_DENIED);
    let outsider = TitleToken { user: "S-1-5-7".into(), groups: vec!["S-1-1-0".into()], is_app_container: false, membership_error: 0 };
    assert_eq!(security_callback(&ok, p, &outsider), ERROR_ACCESS_DENIED);
    let netlogon = CallAttributes { authentication_service: RPC_C_AUTHN_NETLOGON, ..ok };
    assert_eq!(security_callback(&netlogon, p, &outsider), 0);
    assert_eq!(security_callback(&CallAttributes { authentication_level: 1, ..netlogon }, p, &outsider), ERROR_ACCESS_DENIED);
    let failing = TitleToken { membership_error: 8, ..t.clone() };
    assert_eq!(security_callback(&ok, p, &failing), 8);
    assert_eq!(security_callback(&ok, Some(OpnumPolicy { protseq_mask: 8, flags: 1 }), &outsider), 0);
}

#[test]
fn denied_call_maps_to_console_client_status() {
    let h = Hklm::at(temp("denied"));
    let app = TitleToken { is_app_container: true, ..TitleToken::console_title() };
    assert_eq!(lsarpc_call(&h, OPNUM_ENABLE, &CallAttributes::console_client(OPNUM_ENABLE), &app), 0xC000_0022u32 as i32);
    assert!(h.get_dword(DEVICE_KEY, DEVICE_VALUE).is_err());
    assert_eq!(client_exception_status(0), STATUS_UNSUCCESSFUL);
}

#[test]
fn lsa_enable_and_disable_password_less_write_the_device_value() {
    let d = temp("device");
    let h = Hklm::at(&d);
    let t = TitleToken::console_title();
    assert!(h.get_dword(DEVICE_KEY, DEVICE_VALUE).is_err());
    assert_eq!(lsarpc_call(&h, OPNUM_ENABLE, &CallAttributes::console_client(OPNUM_ENABLE), &t), 0);
    assert_eq!(h.get_dword(DEVICE_KEY, DEVICE_VALUE), Ok(2));
    assert!(h.value_path(DEVICE_KEY, DEVICE_VALUE).starts_with(&d));
    assert_eq!(lsarpc_call(&h, OPNUM_DISABLE, &CallAttributes::console_client(OPNUM_DISABLE), &t), 0);
    assert_eq!(h.get_dword(DEVICE_KEY, DEVICE_VALUE), Ok(0));
    assert!(h.get_dword(&format!("{USER_CONFIG_KEY}{}", t.user), USER_CONFIG_VALUE).is_err());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn force_disable_set_writes_the_user_config_value() {
    let d = temp("userconfig");
    let h = Hklm::at(&d);
    let t = TitleToken::console_title();
    h.create_key(IMAGE_CUSTOMIZATION_KEY).unwrap();
    h.set_dword(IMAGE_CUSTOMIZATION_KEY, FORCE_DISABLE_VALUE, 0).unwrap();
    assert!(force_disable_unset(&h));
    h.set_dword(IMAGE_CUSTOMIZATION_KEY, FORCE_DISABLE_VALUE, 1).unwrap();
    assert!(!force_disable_unset(&h));
    let key = format!("{USER_CONFIG_KEY}{}", t.user);
    assert_eq!(server_body(&h, true, &t), 0);
    assert_eq!(h.get_dword(&key, USER_CONFIG_VALUE), Ok(1));
    assert_eq!(server_body(&h, false, &t), 0);
    assert_eq!(h.get_dword(&key, USER_CONFIG_VALUE), Ok(2));
    assert!(h.get_dword(DEVICE_KEY, DEVICE_VALUE).is_err());
    h.set_dword(IMAGE_CUSTOMIZATION_KEY, FORCE_DISABLE_VALUE, 0).unwrap();
    assert_eq!(write_user_config(&h, &t.user, 1), E_NOTIMPL);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn failed_registry_write_maps_like_the_console_server() {
    let d = temp("fail");
    std::fs::create_dir_all(&d).unwrap();
    let blocker = d.join("SOFTWARE");
    std::fs::write(&blocker, b"x").unwrap();
    let h = Hklm::at(&d);
    let st = server_body(&h, true, &TitleToken::console_title());
    assert!(st < 0 && (st as u32) & 0xffff_0000 == 0xC007_0000, "{st:#x}");
    let _ = std::fs::remove_dir_all(&d);
}

#[link(name = "advapi32")]
extern "system" {
    fn OpenProcessToken(process: isize, access: u32, token: *mut isize) -> i32;
    fn CreateRestrictedToken(existing: isize, flags: u32, disable_count: u32, disable: *const [usize; 2], delete_count: u32, delete: *const u8, restrict_count: u32, restrict: *const u8, out: *mut isize) -> i32;
    fn DuplicateToken(existing: isize, level: u32, out: *mut isize) -> i32;
    fn SetThreadToken(thread: *const isize, token: isize) -> i32;
    fn RevertToSelf() -> i32;
    fn ConvertStringSidToSidW(s: *const u16, sid: *mut *mut u8) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentProcess() -> isize;
    fn CloseHandle(h: isize) -> i32;
    fn LocalFree(p: *mut u8) -> *mut u8;
}

#[test]
fn real_caller_token_without_authenticated_users_is_denied_end_to_end() {
    let d = temp("restricted");
    std::env::set_var("TOMBOX_GDK_ROOT", &d);
    let h = Hklm::at(d.join("hklm"));
    unsafe {
        let mut proc_tok = 0isize;
        assert_ne!(OpenProcessToken(GetCurrentProcess(), 0x000b, &mut proc_tok), 0);
        let mut sid: *mut u8 = core::ptr::null_mut();
        let s: Vec<u16> = SID_AUTHENTICATED_USERS.encode_utf16().chain(Some(0)).collect();
        assert_ne!(ConvertStringSidToSidW(s.as_ptr(), &mut sid), 0);
        let disable = [sid as usize, 0usize];
        let mut restricted = 0isize;
        assert_ne!(CreateRestrictedToken(proc_tok, 0, 1, &disable, 0, core::ptr::null(), 0, core::ptr::null(), &mut restricted), 0);
        let mut imp = 0isize;
        assert_ne!(DuplicateToken(restricted, 2, &mut imp), 0);
        assert_ne!(SetThreadToken(core::ptr::null(), imp), 0);
        let t = TitleToken::caller();
        let st = lsarpc_call(&h, OPNUM_ENABLE, &CallAttributes::console_client(OPNUM_ENABLE), &t);
        let exported = LsaEnablePasswordLessCurrentUser();
        assert_ne!(RevertToSelf(), 0);
        assert!(!t.check_membership(SID_AUTHENTICATED_USERS), "{t:?}");
        assert_eq!(st, 0xC000_0022u32 as i32);
        assert_eq!(exported, 0xC000_0022u32 as i32);
        assert!(h.get_dword(DEVICE_KEY, DEVICE_VALUE).is_err());
        let after = TitleToken::caller();
        assert!(after.check_membership(SID_AUTHENTICATED_USERS));
        LocalFree(sid);
        CloseHandle(imp);
        CloseHandle(restricted);
        CloseHandle(proc_tok);
    }
    let _ = std::fs::remove_dir_all(&d);
}
