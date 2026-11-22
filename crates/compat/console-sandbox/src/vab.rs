use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, MutexGuard, OnceLock};

pub const DEVICE_PATH: &str = r"\\.\VirtualAudioBus";
pub const IOCTL_CREATE_EVENT: u32 = 0x001d_006b;
pub const CREATE_EVENT_INPUT: u32 = 0x20c;
pub const NAME_CHARS: usize = 260;
pub const SUFFIX: &str = ":2";
pub const SRA_BASE: &str = "SraSignaledAudioEvent";
pub const GAMECORE_BASE: &str = "GamecoreSignaledAudioEvent";

const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_ALREADY_EXISTS: u32 = 183;
const ERROR_NOT_SUPPORTED: u32 = 50;

#[link(name = "kernel32")]
extern "system" {
    fn CreateFileW(name: *const u16, access: u32, share: u32, sa: *const core::ffi::c_void, disposition: u32, flags: u32, template: isize) -> isize;
    fn CreateEventW(sa: *const core::ffi::c_void, manual: i32, initial: i32, name: *const u16) -> isize;
    fn OpenEventW(access: u32, inherit: i32, name: *const u16) -> isize;
    fn GetLastError() -> u32;
    fn SetLastError(e: u32);
}

#[derive(Default)]
struct Bus {
    handles: HashSet<usize>,
    events: HashMap<String, isize>,
    opened: Vec<String>,
}

fn state() -> MutexGuard<'static, Bus> {
    static S: OnceLock<Mutex<Bus>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(Bus::default())).lock().unwrap_or_else(|e| e.into_inner())
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

unsafe fn from_ptr(p: *const u16) -> String {
    let mut n = 0;
    while *p.add(n) != 0 {
        n += 1;
    }
    String::from_utf16_lossy(core::slice::from_raw_parts(p, n))
}

pub fn is_device_path(p: &str) -> bool {
    p.eq_ignore_ascii_case(DEVICE_PATH)
}

pub unsafe fn open() -> isize {
    let nul = wide("NUL");
    let h = CreateFileW(nul.as_ptr(), 0xC000_0000, 3, core::ptr::null(), 3, 0, 0);
    if h != -1 {
        state().handles.insert(h as usize);
        compat_common::record_raw("vab", "CreateFileW", "opened the emulated VirtualAudioBus device");
    }
    h
}

pub fn is_device(h: usize) -> bool {
    state().handles.contains(&h)
}

pub fn forget(h: usize) {
    state().handles.remove(&h);
}

pub fn create_event(name: &str) -> Result<isize, u32> {
    let full = format!("{name}{SUFFIX}");
    let mut s = state();
    if s.events.contains_key(&full) {
        return Err(ERROR_ALREADY_EXISTS);
    }
    let w = wide(&full);
    let h = unsafe { CreateEventW(core::ptr::null(), 0, 0, w.as_ptr()) };
    if h == 0 {
        return Err(unsafe { GetLastError() });
    }
    s.events.insert(full.clone(), h);
    compat_common::record_raw("vab", "0x1d006b", &format!("created host event {full}"));
    Ok(h)
}

pub fn event(full: &str) -> Option<isize> {
    state().events.get(full).copied()
}

pub fn sra_events() -> Vec<isize> {
    state().events.iter().filter(|(k, _)| k.starts_with(SRA_BASE)).map(|(_, &h)| h).collect()
}

pub fn opened_names() -> Vec<String> {
    state().opened.clone()
}

pub unsafe fn ioctl(code: u32, inb: *const u8, inl: u32, ret: *mut u32) -> i32 {
    if code != IOCTL_CREATE_EVENT {
        compat_common::record_raw("vab", "DeviceIoControl", &format!("ioctl {code:#x} is not emulated; ERROR_NOT_SUPPORTED"));
        SetLastError(ERROR_NOT_SUPPORTED);
        return 0;
    }
    if inb.is_null() || inl != CREATE_EVENT_INPUT {
        SetLastError(ERROR_INVALID_PARAMETER);
        return 0;
    }
    let chars = core::slice::from_raw_parts(inb as *const u16, NAME_CHARS);
    let Some(n) = chars.iter().position(|&c| c == 0) else {
        SetLastError(ERROR_INVALID_PARAMETER);
        return 0;
    };
    if n == 0 {
        SetLastError(ERROR_INVALID_PARAMETER);
        return 0;
    }
    let name = String::from_utf16_lossy(&chars[..n]);
    if !ret.is_null() {
        *ret = 0;
    }
    match create_event(&name) {
        Ok(_) => 1,
        Err(e) => {
            SetLastError(e);
            0
        }
    }
}

pub unsafe extern "system" fn open_event_w(access: u32, inherit: i32, name: *const u16) -> isize {
    if !name.is_null() {
        let n = from_ptr(name);
        compat_common::record_raw("vab", "OpenEventW", &format!("{n} access {access:#x}"));
        state().opened.push(n);
    }
    OpenEventW(access, inherit, name)
}
