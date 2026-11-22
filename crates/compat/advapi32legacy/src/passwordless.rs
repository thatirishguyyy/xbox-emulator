use std::path::{Path, PathBuf};

use crate::NtStatus;

pub const OPNUM_ENABLE: u16 = 0x7e;
pub const OPNUM_DISABLE: u16 = 0x7f;
pub const RPC_PROTSEQ_LRPC: u32 = 3;
pub const RPC_C_AUTHN_LEVEL_PKT_PRIVACY: u32 = 6;
pub const RPC_C_AUTHN_WINNT: u32 = 10;
pub const RPC_C_AUTHN_NETLOGON: u32 = 0x44;
pub const ERROR_ACCESS_DENIED: u32 = 5;
pub const RPC_S_PROTSEQ_NOT_SUPPORTED: u32 = 0x6a7;
pub const RPC_S_PROCNUM_OUT_OF_RANGE: u32 = 0x6d1;
pub const STATUS_UNSUCCESSFUL: NtStatus = 0xC000_0001u32 as i32;
pub const E_NOTIMPL: i32 = 0x8000_4001u32 as i32;

pub const SID_AUTHENTICATED_USERS: &str = "S-1-5-11";
pub const DEVICE_KEY: &str = "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\PasswordLess\\Device";
pub const DEVICE_VALUE: &str = "DevicePasswordLessBuildVersion";
pub const USER_CONFIG_KEY: &str = "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\PasswordLess\\UserConfig\\";
pub const USER_CONFIG_VALUE: &str = "PasswordLess";
pub const IMAGE_CUSTOMIZATION_KEY: &str = "SYSTEM\\CurrentControlSet\\Control\\Cryptography\\PwdlessPlat\\ImageCustomization";
pub const FORCE_DISABLE_VALUE: &str = "ForceDisablePasswordlessNewDevices";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpnumPolicy {
    pub protseq_mask: u32,
    pub flags: u8,
}

const fn p(protseq_mask: u32, flags: u8) -> OpnumPolicy {
    OpnumPolicy { protseq_mask, flags }
}

pub const LSARPC_OPNUM_LIMIT: u16 = 0x86;
pub const LSARPC_POLICY_RVA: u32 = 0x1654c0;
pub const LSARPC_POLICY: [OpnumPolicy; LSARPC_OPNUM_LIMIT as usize] = [p(12, 0), p(12, 2), p(12, 0), p(12, 0), p(12, 2), p(12, 0), p(12, 0), p(12, 0), p(12, 2), p(12, 0), p(12, 2), p(12, 0), p(12, 2), p(12, 0), p(12, 0), p(12, 0), p(12, 2), p(12, 0), p(12, 0), p(12, 2), p(12, 2), p(12, 0), p(12, 2), p(12, 0), p(12, 2), p(12, 0), p(12, 0), p(12, 2), p(12, 2), p(12, 6), p(12, 6), p(12, 0), p(12, 0), p(12, 0), p(12, 2), p(12, 0), p(12, 0), p(12, 2), p(12, 2), p(12, 0), p(12, 2), p(12, 2), p(12, 6), p(12, 6), p(12, 0), p(12, 0), p(12, 0), p(12, 2), p(12, 0), p(12, 2), p(12, 0), p(12, 2), p(12, 0), p(12, 0), p(12, 2), p(12, 0), p(12, 2), p(12, 0), p(12, 0), p(12, 2), p(8, 11), p(8, 11), p(8, 11), p(8, 11), p(8, 11), p(8, 11), p(8, 3), p(8, 3), p(12, 0), p(8, 11), p(8, 3), p(8, 3), p(8, 3), p(12, 0), p(12, 2), p(8, 3), p(2, 0), p(2, 0), p(8, 3), p(8, 3), p(8, 3), p(8, 3), p(8, 11), p(8, 3), p(8, 3), p(8, 3), p(8, 3), p(8, 3), p(8, 3), p(8, 3), p(8, 3), p(8, 3), p(8, 3), p(8, 3), p(8, 3), p(8, 3), p(8, 3), p(8, 7), p(8, 7), p(8, 7), p(8, 7), p(8, 7), p(8, 3), p(8, 3), p(8, 3), p(8, 3), p(8, 3), p(8, 3), p(8, 3), p(8, 3), p(8, 3), p(8, 3), p(8, 3), p(8, 3), p(8, 3), p(8, 3), p(8, 3), p(8, 3), p(8, 3), p(8, 3), p(8, 3), p(8, 3), p(8, 3), p(8, 3), p(8, 3), p(8, 3), p(8, 3), p(8, 3), p(8, 3), p(12, 2), p(12, 0), p(8, 3), p(12, 2), p(12, 2)];
pub const LSARPC_POLICY_7E: OpnumPolicy = LSARPC_POLICY[OPNUM_ENABLE as usize];
pub const LSARPC_POLICY_7F: OpnumPolicy = LSARPC_POLICY[OPNUM_DISABLE as usize];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CallAttributes {
    pub opnum: u16,
    pub protocol_sequence: u32,
    pub is_client_local: u32,
    pub authentication_level: u32,
    pub authentication_service: u32,
}

impl CallAttributes {
    pub const fn console_client(opnum: u16) -> CallAttributes {
        CallAttributes {
            opnum,
            protocol_sequence: RPC_PROTSEQ_LRPC,
            is_client_local: 1,
            authentication_level: RPC_C_AUTHN_LEVEL_PKT_PRIVACY,
            authentication_service: RPC_C_AUTHN_WINNT,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TitleToken {
    pub user: String,
    pub groups: Vec<String>,
    pub is_app_container: bool,
    pub membership_error: u32,
}

impl TitleToken {
    pub fn console_title() -> TitleToken {
        TitleToken {
            user: "S-1-5-18".to_string(),
            groups: vec!["S-1-5-32-544".to_string(), "S-1-1-0".to_string(), SID_AUTHENTICATED_USERS.to_string()],
            is_app_container: false,
            membership_error: 0,
        }
    }

    pub fn caller() -> TitleToken {
        unsafe { caller_token() }
    }

    pub fn check_membership(&self, sid: &str) -> bool {
        !self.is_app_container && (self.user == sid || self.groups.iter().any(|g| g == sid))
    }
}

pub fn opnum_policy(opnum: u16) -> Option<OpnumPolicy> {
    LSARPC_POLICY.get(opnum as usize).copied()
}

#[link(name = "advapi32")]
extern "system" {
    fn OpenThreadToken(thread: isize, access: u32, as_self: i32, token: *mut isize) -> i32;
    fn OpenProcessToken(process: isize, access: u32, token: *mut isize) -> i32;
    fn GetTokenInformation(token: isize, class: u32, buf: *mut u8, len: u32, ret: *mut u32) -> i32;
    fn ConvertSidToStringSidW(sid: *const u8, out: *mut *mut u16) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentThread() -> isize;
    fn GetCurrentProcess() -> isize;
    fn CloseHandle(h: isize) -> i32;
    fn LocalFree(p: *mut core::ffi::c_void) -> *mut core::ffi::c_void;
    fn GetLastError() -> u32;
}

const TOKEN_QUERY: u32 = 8;
const TOKEN_USER: u32 = 1;
const TOKEN_GROUPS: u32 = 2;
const TOKEN_IS_APP_CONTAINER: u32 = 29;
const SE_GROUP_ENABLED: u32 = 4;
const SE_GROUP_USE_FOR_DENY_ONLY: u32 = 0x10;

unsafe fn sid_text(sid: *const u8) -> Option<String> {
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

unsafe fn token_info(token: isize, class: u32) -> Result<Vec<u64>, u32> {
    let mut need = 0u32;
    GetTokenInformation(token, class, core::ptr::null_mut(), 0, &mut need);
    let mut buf = vec![0u64; (need as usize).div_ceil(8).max(1)];
    if GetTokenInformation(token, class, buf.as_mut_ptr() as *mut u8, (buf.len() * 8) as u32, &mut need) == 0 {
        return Err(GetLastError());
    }
    Ok(buf)
}

pub unsafe fn token_of(token: isize) -> TitleToken {
    let denied = TitleToken { user: String::new(), groups: Vec::new(), is_app_container: false, membership_error: ERROR_ACCESS_DENIED };
    let user = match token_info(token, TOKEN_USER) {
        Ok(b) => match sid_text(b[0] as *const u8) {
            Some(s) => s,
            None => return denied,
        },
        Err(e) => return TitleToken { membership_error: e, ..denied },
    };
    let groups = match token_info(token, TOKEN_GROUPS) {
        Ok(b) => {
            let base = b.as_ptr() as *const u8;
            let count = (base as *const u32).read_unaligned() as usize;
            (0..count)
                .filter_map(|i| {
                    let e = base.add(8 + i * 16);
                    let attrs = (e.add(8) as *const u32).read_unaligned();
                    (attrs & SE_GROUP_ENABLED != 0 && attrs & SE_GROUP_USE_FOR_DENY_ONLY == 0).then(|| sid_text((e as *const usize).read_unaligned() as *const u8)).flatten()
                })
                .collect()
        }
        Err(e) => return TitleToken { user, membership_error: e, ..denied },
    };
    let is_app_container = match token_info(token, TOKEN_IS_APP_CONTAINER) {
        Ok(b) => b[0] as u32 != 0,
        Err(_) => return TitleToken { user, groups, ..denied },
    };
    TitleToken { user, groups, is_app_container, membership_error: 0 }
}

pub unsafe fn caller_token() -> TitleToken {
    let mut t = 0isize;
    if OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, 1, &mut t) == 0 && OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut t) == 0 {
        return TitleToken { user: String::new(), groups: Vec::new(), is_app_container: false, membership_error: ERROR_ACCESS_DENIED };
    }
    let out = token_of(t);
    CloseHandle(t);
    out
}

pub fn security_callback(call: &CallAttributes, policy: Option<OpnumPolicy>, token: &TitleToken) -> u32 {
    if call.opnum >= LSARPC_OPNUM_LIMIT {
        return RPC_S_PROCNUM_OUT_OF_RANGE;
    }
    let p = policy.unwrap_or(OpnumPolicy { protseq_mask: 0, flags: 4 });
    if p.flags & 4 != 0 {
        return RPC_S_PROCNUM_OUT_OF_RANGE;
    }
    if p.protseq_mask & (1u32 << (call.protocol_sequence & 31)) == 0 {
        return RPC_S_PROTSEQ_NOT_SUPPORTED;
    }
    if p.flags & 1 != 0 && call.is_client_local != 1 {
        return ERROR_ACCESS_DENIED;
    }
    if (2..=4).contains(&call.authentication_level) {
        return ERROR_ACCESS_DENIED;
    }
    if p.flags & 2 == 0 {
        return 0;
    }
    if call.authentication_service == RPC_C_AUTHN_NETLOGON && call.authentication_level >= 5 {
        return 0;
    }
    if token.membership_error != 0 {
        return token.membership_error;
    }
    if !token.check_membership(SID_AUTHENTICATED_USERS) {
        return ERROR_ACCESS_DENIED;
    }
    if token.is_app_container && p.flags & 8 == 0 {
        return ERROR_ACCESS_DENIED;
    }
    0
}

pub fn hklm_root() -> PathBuf {
    let base = match std::env::var("TOMBOX_GDK_ROOT") {
        Ok(p) => PathBuf::from(p),
        Err(_) => match std::env::var_os("LOCALAPPDATA") {
            Some(p) => PathBuf::from(p).join("XboxEmulator"),
            None => std::env::temp_dir().join("XboxEmulator"),
        },
    };
    base.join("hklm")
}

pub struct Hklm {
    root: PathBuf,
}

fn win32_of(e: &std::io::Error) -> u32 {
    match e.raw_os_error() {
        Some(c) if c > 0 => c as u32,
        _ => 0x1f,
    }
}

impl Hklm {
    pub fn open() -> Hklm {
        Hklm { root: hklm_root() }
    }

    pub fn at(root: impl Into<PathBuf>) -> Hklm {
        Hklm { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn key_path(&self, key: &str) -> PathBuf {
        key.split('\\').filter(|c| !c.is_empty()).fold(self.root.clone(), |a, c| a.join(c))
    }

    pub fn value_path(&self, key: &str, value: &str) -> PathBuf {
        self.key_path(key).join(format!("{value}.dword"))
    }

    pub fn get_dword(&self, key: &str, value: &str) -> Result<u32, u32> {
        let text = std::fs::read_to_string(self.value_path(key, value)).map_err(|e| win32_of(&e))?;
        text.trim().parse::<u32>().map_err(|_| 13)
    }

    pub fn create_key(&self, key: &str) -> Result<(), u32> {
        std::fs::create_dir_all(self.key_path(key)).map_err(|e| win32_of(&e))
    }

    pub fn set_dword(&self, key: &str, value: &str, data: u32) -> Result<(), u32> {
        std::fs::write(self.value_path(key, value), format!("{data}\n")).map_err(|e| win32_of(&e))
    }
}

fn hresult_from_win32(e: u32) -> i32 {
    if e as i32 <= 0 {
        e as i32
    } else {
        ((e & 0xffff) | 0x8007_0000) as i32
    }
}

pub fn force_disable_unset(h: &Hklm) -> bool {
    match h.get_dword(IMAGE_CUSTOMIZATION_KEY, FORCE_DISABLE_VALUE) {
        Ok(v) => v == 0,
        Err(_) => true,
    }
}

pub fn write_device(h: &Hklm, version: u32) -> i32 {
    if let Err(e) = h.create_key(DEVICE_KEY) {
        return hresult_from_win32(e);
    }
    match h.set_dword(DEVICE_KEY, DEVICE_VALUE, version) {
        Ok(()) => 0,
        Err(e) => hresult_from_win32(e),
    }
}

pub fn write_user_config(h: &Hklm, sid: &str, mode: u32) -> i32 {
    if force_disable_unset(h) {
        return E_NOTIMPL;
    }
    let data = if mode == 1 { 1 } else if mode == 2 { 2 } else { 0 };
    let mut key = String::from(USER_CONFIG_KEY);
    if key.len() + sid.len() >= 0x104 {
        return 0x8007_007Au32 as i32;
    }
    key.push_str(sid);
    if let Err(e) = h.create_key(&key) {
        return hresult_from_win32(e);
    }
    match h.set_dword(&key, USER_CONFIG_VALUE, data) {
        Ok(()) => 0,
        Err(e) => hresult_from_win32(e),
    }
}

pub fn server_body(h: &Hklm, enable: bool, token: &TitleToken) -> NtStatus {
    let hr = if force_disable_unset(h) {
        write_device(h, if enable { 2 } else { 0 })
    } else {
        write_user_config(h, &token.user, if enable { 1 } else { 2 })
    };
    if hr >= 0 {
        return 0;
    }
    let low = (hr as u32) & 0xffff;
    if low != 0 {
        (low | 0xC007_0000) as i32
    } else {
        0
    }
}

#[link(name = "rpcrt4")]
extern "system" {
    fn I_RpcMapWin32Status(status: u32) -> i32;
}

pub fn client_exception_status(code: u32) -> NtStatus {
    let m = unsafe { I_RpcMapWin32Status(code) };
    if m < 0 {
        m
    } else {
        STATUS_UNSUCCESSFUL
    }
}

pub fn lsarpc_call(h: &Hklm, opnum: u16, call: &CallAttributes, token: &TitleToken) -> NtStatus {
    let denied = security_callback(call, opnum_policy(opnum), token);
    if denied != 0 {
        return client_exception_status(denied);
    }
    server_body(h, opnum == OPNUM_ENABLE, token)
}

#[no_mangle]
pub unsafe extern "system" fn LsaEnablePasswordLessCurrentUser() -> NtStatus {
    lsarpc_call(&Hklm::open(), OPNUM_ENABLE, &CallAttributes::console_client(OPNUM_ENABLE), &TitleToken::caller())
}

#[no_mangle]
pub unsafe extern "system" fn LsaDisablePasswordLessCurrentUser() -> NtStatus {
    lsarpc_call(&Hklm::open(), OPNUM_DISABLE, &CallAttributes::console_client(OPNUM_DISABLE), &TitleToken::caller())
}
