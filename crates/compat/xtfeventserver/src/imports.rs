use crate::sys::*;
use core::ffi::c_void;
use std::sync::OnceLock;

pub type CreateFn = unsafe extern "C" fn(*mut *mut c_void) -> HRESULT;
pub type CreateIidFn = unsafe extern "C" fn(*const GUID, *mut *mut c_void) -> HRESULT;
pub type ParseUtf8Fn = unsafe extern "C" fn(*const u8, *mut *mut c_void) -> HRESULT;
pub type LogVFn = unsafe extern "C" fn(*const u8, *mut u8);

#[derive(Clone, Copy)]
pub struct Imports {
    pub create_extension_metadata: CreateIidFn,
    pub get_json_object_from_utf8: ParseUtf8Fn,
    pub create_json_object: CreateFn,
    pub log_info_v: LogVFn,
    pub create_message: CreateIidFn,
}

static IMPORTS: OnceLock<Result<Imports, HRESULT>> = OnceLock::new();

#[cfg(not(test))]
unsafe fn proc_addr<T: Copy>(m: HANDLE, name: &str) -> Result<T, HRESULT> {
    let mut n = name.as_bytes().to_vec();
    n.push(0);
    let p = GetProcAddress(m, n.as_ptr());
    if p.is_null() {
        Err(hr_last_error())
    } else {
        Ok(core::mem::transmute_copy(&p))
    }
}

#[cfg(not(test))]
fn load() -> Result<Imports, HRESULT> {
    unsafe {
        let w = wz("xbtp.dll");
        let mut x = GetModuleHandleW(w.as_ptr());
        if x.is_null() {
            x = LoadLibraryExW(w.as_ptr(), core::ptr::null_mut(), 0);
        }
        if x.is_null() {
            return Err(hr_last_error());
        }
        Ok(Imports {
            create_extension_metadata: proc_addr(x, "XbtpCreateExtensionMetadata")?,
            get_json_object_from_utf8: proc_addr(x, "XbtpGetJsonObjectFromUtf8")?,
            create_json_object: proc_addr(x, "XbtpCreateJsonObject")?,
            log_info_v: proc_addr(x, "?LogInfoV@@YAXPEBDPEAD@Z")?,
            create_message: proc_addr(x, "XbtpCreateMessage")?,
        })
    }
}

#[cfg(test)]
fn load() -> Result<Imports, HRESULT> {
    crate::tests::test_imports()
}

pub fn get() -> Result<&'static Imports, HRESULT> {
    match IMPORTS.get_or_init(load) {
        Ok(i) => Ok(i),
        Err(e) => Err(*e),
    }
}

pub unsafe fn log_info(fmt: &[u8], args: &[u64]) {
    if let Ok(i) = get() {
        (i.log_info_v)(fmt.as_ptr(), args.as_ptr() as *mut u8);
    }
}

pub unsafe fn chk(hr: HRESULT, what: &[u8], func: &[u8], line: u32) -> HRESULT {
    if hr < 0 {
        log_info(b"XtfCHK: %s: %d 0x%X (%s)\n\0", &[func.as_ptr() as u64, line as u64, hr as u32 as u64, what.as_ptr() as u64]);
    }
    hr
}
