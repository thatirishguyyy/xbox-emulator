use core::ffi::c_void;
pub use xtfcommon::sys::{CloseHandle, MultiByteToWideChar, SetLastError, INFINITE, INVALID_HANDLE_VALUE};
use xtfmsg::sys::{BOOL, HANDLE, HRESULT};

pub const E_POINTER: HRESULT = 0x8000_4003u32 as i32;
pub const HANDLE_FLAG_INHERIT: u32 = 1;
pub const STARTF_USESTDHANDLES: u32 = 0x100;
pub const WAIT_OBJECT_0: u32 = 0;
pub const WAIT_IO_COMPLETION: u32 = 0xc0;
pub const WAIT_TIMEOUT: u32 = 0x102;
pub const WAIT_FAILED: u32 = 0xffff_ffff;
pub const CP_ACP: u32 = 0;
pub const PROC_THREAD_ATTRIBUTE_JOB_LIST: usize = 0x2000d;
pub const JOB_OBJECT_ALL_ACCESS: u32 = 0x1f003f;
pub const TOKEN_ALL_ACCESS: u32 = 0xf01ff;
pub const CREATE_SUSPENDED: u32 = 4;
pub const CREATE_NEW_CONSOLE: u32 = 0x10;
pub const CREATE_UNICODE_ENVIRONMENT: u32 = 0x400;
pub const EXTENDED_STARTUPINFO_PRESENT: u32 = 0x8_0000;
pub const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x100_0000;
pub const CREATE_NO_WINDOW: u32 = 0x800_0000;
pub const CONSOLE_CREATION_FLAGS: u32 = 0x108_0414;
pub const ERROR_FILE_NOT_FOUND: u32 = 2;
pub const ERROR_DIRECTORY: u32 = 267;
pub const DUPLICATE_SAME_ACCESS: u32 = 2;

#[repr(C)]
pub struct SecurityAttributes {
    pub length: u32,
    pub descriptor: *mut c_void,
    pub inherit: BOOL,
}

#[repr(C)]
pub struct StartupInfoW {
    pub cb: u32,
    pub reserved: *mut u16,
    pub desktop: *mut u16,
    pub title: *mut u16,
    pub x: u32,
    pub y: u32,
    pub x_size: u32,
    pub y_size: u32,
    pub x_count_chars: u32,
    pub y_count_chars: u32,
    pub fill_attribute: u32,
    pub flags: u32,
    pub show_window: u16,
    pub reserved2_len: u16,
    pub reserved2: *mut u8,
    pub std_input: HANDLE,
    pub std_output: HANDLE,
    pub std_error: HANDLE,
}

#[repr(C)]
pub struct StartupInfoExW {
    pub si: StartupInfoW,
    pub attribute_list: *mut c_void,
}

impl StartupInfoExW {
    pub fn zeroed() -> StartupInfoExW {
        unsafe { core::mem::zeroed() }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ProcessInformation {
    pub process: HANDLE,
    pub thread: HANDLE,
    pub process_id: u32,
    pub thread_id: u32,
}

impl ProcessInformation {
    pub fn zeroed() -> ProcessInformation {
        ProcessInformation { process: core::ptr::null_mut(), thread: core::ptr::null_mut(), process_id: 0, thread_id: 0 }
    }
}

#[link(name = "kernel32")]
extern "system" {
    pub fn CreatePipe(read: *mut HANDLE, write: *mut HANDLE, sa: *const SecurityAttributes, size: u32) -> BOOL;
    pub fn SetHandleInformation(h: HANDLE, mask: u32, flags: u32) -> BOOL;
    pub fn PeekNamedPipe(h: HANDLE, buf: *mut c_void, size: u32, read: *mut u32, avail: *mut u32, left: *mut u32) -> BOOL;
    pub fn ReadFile(h: HANDLE, buf: *mut c_void, n: u32, read: *mut u32, ov: *mut c_void) -> BOOL;
    pub fn WriteFile(h: HANDLE, buf: *const c_void, n: u32, written: *mut u32, ov: *mut c_void) -> BOOL;
    pub fn WaitForMultipleObjectsEx(n: u32, handles: *const HANDLE, all: BOOL, ms: u32, alertable: BOOL) -> u32;
    pub fn GetExitCodeProcess(h: HANDLE, code: *mut u32) -> BOOL;
    pub fn GetTickCount64() -> u64;
    pub fn WideCharToMultiByte(cp: u32, flags: u32, w: *const u16, wn: i32, s: *mut u8, n: i32, default: *const u8, used: *mut BOOL) -> i32;
    pub fn InitializeProcThreadAttributeList(list: *mut c_void, count: u32, flags: u32, size: *mut usize) -> BOOL;
    pub fn UpdateProcThreadAttribute(list: *mut c_void, flags: u32, attribute: usize, value: *const c_void, size: usize, previous: *mut c_void, ret: *mut usize) -> BOOL;
    pub fn DeleteProcThreadAttributeList(list: *mut c_void);
    pub fn GetProcessAffinityMask(h: HANDLE, process: *mut usize, system: *mut usize) -> BOOL;
    pub fn SetProcessAffinityMask(h: HANDLE, mask: usize) -> BOOL;
    pub fn ResumeThread(h: HANDLE) -> u32;
    pub fn GetCurrentProcess() -> HANDLE;
    pub fn Sleep(ms: u32);
    pub fn DuplicateHandle(sp: HANDLE, h: HANDLE, tp: HANDLE, out: *mut HANDLE, access: u32, inherit: BOOL, options: u32) -> BOOL;
    pub fn CreateJobObjectW(sa: *const c_void, name: *const u16) -> HANDLE;
    pub fn SetInformationJobObject(job: HANDLE, class: u32, info: *const c_void, len: u32) -> BOOL;
    pub fn TerminateProcess(h: HANDLE, code: u32) -> BOOL;
    pub fn WaitForSingleObject(h: HANDLE, ms: u32) -> u32;
}

#[link(name = "advapi32")]
extern "system" {
    pub fn OpenProcessToken(process: HANDLE, access: u32, token: *mut HANDLE) -> BOOL;
    pub fn CreateRestrictedToken(token: HANDLE, flags: u32, disable_count: u32, disable: *const c_void, delete_count: u32, delete: *const c_void, restrict_count: u32, restrict: *const c_void, out: *mut HANDLE) -> BOOL;
    pub fn CreateProcessAsUserW(token: HANDLE, app: *const u16, cmd: *mut u16, psa: *const c_void, tsa: *const c_void, inherit: BOOL, flags: u32, env: *const c_void, cwd: *const u16, si: *const StartupInfoExW, pi: *mut ProcessInformation) -> BOOL;
}

pub fn hr_from_wait(code: u32) -> HRESULT {
    if (code as i32) <= 0 {
        code as i32
    } else {
        ((code & 0xffff) | 0x8007_0000) as i32
    }
}

pub fn last_error_or_fail() -> HRESULT {
    let e = unsafe { xtfmsg::sys::GetLastError() };
    if e == 0 {
        xtfmsg::sys::E_FAIL
    } else {
        hr_from_wait(e)
    }
}

pub struct OwnedHandle(pub HANDLE);

impl OwnedHandle {
    pub fn null() -> OwnedHandle {
        OwnedHandle(core::ptr::null_mut())
    }
    pub fn get(&self) -> HANDLE {
        self.0
    }
    pub fn take(&mut self) -> HANDLE {
        core::mem::replace(&mut self.0, core::ptr::null_mut())
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        let h = self.take();
        if !h.is_null() {
            unsafe { CloseHandle(h) };
        }
    }
}
