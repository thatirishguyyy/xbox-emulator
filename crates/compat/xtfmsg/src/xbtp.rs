use crate::sys::*;
use core::ffi::c_void;
use std::sync::OnceLock;

pub const LOG_INFO_V: &[u8] = b"?LogInfoV@@YAXPEBDPEAD@Z\0";
pub const LOG_WARNING_V: &[u8] = b"?LogWarningV@@YAXPEBDPEAD@Z\0";
pub const GET_PIPE_NAME_TO_SYSTEM: &[u8] = b"?XbtpGetPipeNameToSystem@@YAJPEAPEAG@Z\0";
pub const OPEN_PIPE_SESSION: &[u8] = b"?XbtpOpenPipeSession@@YAJPEBGAEBU_GUID@@1PEAPEAX@Z\0";
pub const OPEN_SESSION2: &[u8] = b"XbtpOpenSession2\0";

static MODULE: OnceLock<usize> = OnceLock::new();

pub fn module() -> usize {
    *MODULE.get_or_init(|| unsafe {
        let name = wide("xbtp.dll");
        let mut h = GetModuleHandleW(name.as_ptr());
        if h.is_null() {
            h = LoadLibraryW(name.as_ptr());
        }
        h as usize
    })
}

pub fn preload(path: &std::path::Path) -> bool {
    let w: Vec<u16> = path.as_os_str().to_string_lossy().encode_utf16().chain(Some(0)).collect();
    unsafe { !LoadLibraryW(w.as_ptr()).is_null() }
}

pub type StaticImports = fn(&[u8]) -> Option<usize>;

static STATIC_IMPORTS: OnceLock<StaticImports> = OnceLock::new();

pub fn bind_static_imports(table: StaticImports) -> bool {
    *STATIC_IMPORTS.get_or_init(|| table) as usize == table as usize
}

pub fn static_import(name: &[u8]) -> Option<usize> {
    STATIC_IMPORTS.get().and_then(|t| t(name))
}

pub fn proc(name: &[u8]) -> Result<usize, HRESULT> {
    if let Some(p) = static_import(name) {
        return Ok(p);
    }
    let m = module();
    if m == 0 {
        return Err(hr_last_error_or_fail());
    }
    let p = unsafe { GetProcAddress(m as HANDLE, name.as_ptr()) };
    if p.is_null() {
        Err(hr_last_error_or_fail())
    } else {
        Ok(p as usize)
    }
}

macro_rules! late {
    ($name:ident, $sym:expr, ($($a:ident: $t:ty),*)) => {
        pub unsafe fn $name($($a: $t),*) -> HRESULT {
            match proc($sym) {
                Ok(p) => {
                    let f: unsafe extern "system" fn($($t),*) -> HRESULT = core::mem::transmute(p);
                    f($($a),*)
                }
                Err(e) => e,
            }
        }
    };
}

late!(create_extension_metadata, b"XbtpCreateExtensionMetadata\0", (iid: *const GUID, out: *mut *mut c_void));
late!(create_json_object, b"XbtpCreateJsonObject\0", (out: *mut *mut c_void));
late!(create_json_array, b"XbtpCreateJsonArray\0", (out: *mut *mut c_void));
late!(create_message, b"XbtpCreateMessage\0", (iid: *const GUID, out: *mut *mut c_void));
late!(get_json_object_from_utf8, b"XbtpGetJsonObjectFromUtf8\0", (utf8: *const u8, out: *mut *mut c_void));
late!(get_current_tls_event_data, b"XbtpGetCurrentTlsEventData\0", (out: *mut *mut c_void));
late!(get_pipe_name_to_system, GET_PIPE_NAME_TO_SYSTEM, (out: *mut *mut u16));
late!(open_pipe_session, OPEN_PIPE_SESSION, (pipe: *const u16, ext: *const GUID, iid: *const GUID, out: *mut *mut c_void));

pub unsafe fn log_v(sym: &[u8], fmt: *const u8, args: *const usize) {
    if let Ok(p) = proc(sym) {
        let f: unsafe extern "C" fn(*const u8, *const usize) = core::mem::transmute(p);
        f(fmt, args);
    }
}

pub unsafe fn log_info(fmt: *const u8, args: &[usize]) {
    if fmt.is_null() {
        return;
    }
    log_v(LOG_INFO_V, fmt, args.as_ptr());
}
