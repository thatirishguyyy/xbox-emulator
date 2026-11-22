use core::ffi::c_void;

pub type HRESULT = i32;
pub type HANDLE = *mut c_void;
pub type BOOL = i32;
pub type BSTR = *mut u16;

pub const S_OK: HRESULT = 0;
pub const E_FAIL: HRESULT = 0x8000_4005u32 as i32;
pub const E_UNEXPECTED: HRESULT = 0x8000_FFFFu32 as i32;
pub const E_INVALIDARG: HRESULT = 0x8007_0057u32 as i32;
pub const E_OUTOFMEMORY: HRESULT = 0x8007_000Eu32 as i32;
pub const E_NO_MORE_SESSIONS: HRESULT = 0x8C11_051Eu32 as i32;
pub const E_PROTOCOL_VERSION_NOT_SUPPORTED: HRESULT = 0x8376_0005u32 as i32;

pub const INVALID_HANDLE_VALUE: HANDLE = usize::MAX as HANDLE;
pub const INFINITE: u32 = 0xFFFF_FFFF;
pub const WAIT_OBJECT_0: u32 = 0;
pub const WAIT_TIMEOUT: u32 = 0x102;
pub const WAIT_FAILED: u32 = 0xFFFF_FFFF;
pub const ERROR_FILE_NOT_FOUND: u32 = 2;
pub const ERROR_INVALID_HANDLE: u32 = 6;
pub const ERROR_CTX_CLOSE_PENDING: u32 = 0x1B5F;

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct GUID {
    pub d1: u32,
    pub d2: u16,
    pub d3: u16,
    pub d4: [u8; 8],
}

impl GUID {
    pub const fn new(d1: u32, d2: u16, d3: u16, d4: [u8; 8]) -> Self {
        GUID { d1, d2, d3, d4 }
    }
}

pub type ThreadProc = unsafe extern "system" fn(*mut c_void) -> u32;

#[link(name = "kernel32")]
extern "system" {
    pub fn GetLastError() -> u32;
    pub fn CloseHandle(h: HANDLE) -> BOOL;
    pub fn CreateEventW(sa: *mut c_void, manual: BOOL, initial: BOOL, name: *const u16) -> HANDLE;
    pub fn SetEvent(h: HANDLE) -> BOOL;
    pub fn WaitForSingleObject(h: HANDLE, ms: u32) -> u32;
    pub fn CreateThread(sa: *mut c_void, stack: usize, start: ThreadProc, param: *mut c_void, flags: u32, tid: *mut u32) -> HANDLE;
    pub fn GetExitCodeThread(h: HANDLE, code: *mut u32) -> BOOL;
    pub fn GetModuleHandleW(name: *const u16) -> HANDLE;
    pub fn LoadLibraryExW(name: *const u16, file: HANDLE, flags: u32) -> HANDLE;
    pub fn GetProcAddress(m: HANDLE, name: *const u8) -> *mut c_void;
    pub fn MultiByteToWideChar(cp: u32, flags: u32, s: *const u8, n: i32, w: *mut u16, wn: i32) -> i32;
    pub fn GetFileAttributesW(name: *const u16) -> u32;
    pub fn CreateDirectoryW(name: *const u16, sa: *mut c_void) -> BOOL;
}

#[link(name = "advapi32")]
extern "system" {
    pub fn StartTraceW(handle: *mut u64, name: *const u16, props: *mut u8) -> u32;
    pub fn EnableTraceEx2(handle: u64, provider: *const GUID, code: u32, level: u8, any: u64, all: u64, timeout: u32, params: *const c_void) -> u32;
    pub fn ControlTraceW(handle: u64, name: *const u16, props: *mut u8, code: u32) -> u32;
    pub fn OpenTraceW(logfile: *mut u8) -> u64;
    pub fn ProcessTrace(handles: *const u64, count: u32, start: *const c_void, end: *const c_void) -> u32;
    pub fn CloseTrace(handle: u64) -> u32;
}

#[link(name = "ole32")]
extern "system" {
    pub fn IIDFromString(s: *const u16, iid: *mut GUID) -> HRESULT;
    pub fn StringFromCLSID(id: *const GUID, out: *mut *mut u16) -> HRESULT;
    pub fn CoTaskMemFree(p: *mut c_void);
}

#[link(name = "oleaut32")]
extern "system" {
    pub fn SysAllocString(s: *const u16) -> BSTR;
    pub fn SysAllocStringLen(s: *const u16, n: u32) -> BSTR;
    pub fn SysFreeString(s: BSTR);
    pub fn SysStringLen(s: BSTR) -> u32;
}

pub fn hr_from_win32(e: u32) -> HRESULT {
    if (e as i32) <= 0 {
        e as i32
    } else {
        ((e & 0xFFFF) | 0x8007_0000) as i32
    }
}

pub fn hr_last_error() -> HRESULT {
    hr_from_win32(unsafe { GetLastError() })
}

pub fn wz(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(core::iter::once(0)).collect()
}

pub unsafe fn wlen(p: *const u16) -> usize {
    let mut n = 0;
    while *p.add(n) != 0 {
        n += 1;
    }
    n
}

pub fn cvtt_i32(d: f64) -> i32 {
    if d.is_nan() || d >= 2_147_483_648.0 || d < -2_147_483_648.0 {
        i32::MIN
    } else {
        d as i32
    }
}

pub fn cvtt_i64(d: f64) -> i64 {
    if d.is_nan() || d >= 9.223_372_036_854_775_808e18 || d < -9.223_372_036_854_775_808e18 {
        i64::MIN
    } else {
        d as i64
    }
}
