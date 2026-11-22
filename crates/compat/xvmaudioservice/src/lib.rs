#![allow(non_snake_case)]

use core::ffi::c_void;
use std::sync::OnceLock;

use console_image::ConsoleDll;

pub const S_OK: i32 = 0;
pub const E_INVALIDARG: i32 = 0x8007_0057u32 as i32;
pub const CLASS_E_CLASSNOTAVAILABLE: i32 = 0x8004_0111u32 as i32;

pub type HSTRING = *mut c_void;

#[link(name = "runtimeobject")]
extern "system" {
    fn WindowsIsStringEmpty(s: HSTRING) -> i32;
    fn WindowsStringHasEmbeddedNull(s: HSTRING, has: *mut i32) -> i32;
    fn RoOriginateError(error: i32, message: HSTRING) -> i32;
    fn RoOriginateErrorW(error: i32, length: u32, message: *const u16) -> i32;
}

pub const ACTIVATABLE_CLASS_ID_NAME: &str = "activatibleClassId";

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Guid {
    pub data1: u32,
    pub data2: u16,
    pub data3: u16,
    pub data4: [u8; 8],
}

pub const CLSID_LOGAN_BOUND: Guid = Guid { data1: 0xe782ced1, data2: 0x3b5d, data3: 0x43aa, data4: [0xa4, 0x5d, 0x82, 0x85, 0x2b, 0x57, 0xd8, 0x4c] };
pub const CLSID_FORWARDED: [Guid; 3] = [
    Guid { data1: 0x148ec3bf, data2: 0x349c, data3: 0x4eac, data4: [0xb3, 0x73, 0x9f, 0xd5, 0x7c, 0xb4, 0x6d, 0x88] },
    Guid { data1: 0x7f34f6c9, data2: 0xbed0, data3: 0x456d, data4: [0x97, 0x72, 0x0d, 0x5b, 0x52, 0x4a, 0x9d, 0x69] },
    Guid { data1: 0xb988be16, data2: 0xd129, data3: 0x46fc, data4: [0x8c, 0x5a, 0xc3, 0xce, 0xff, 0x59, 0x21, 0x37] },
];

pub static CONSOLE: ConsoleDll = ConsoleDll::new("XVMAudioService.dll", "TOMBOX_XVMAUDIOSERVICE_CONSOLE", "XVMAudioService_console.dll", "DllGetClassObject", &[]);
static ATTACHED: OnceLock<Vec<String>> = OnceLock::new();

fn own() -> usize {
    DllCanUnloadNow as *const () as usize
}

pub fn console_module() -> usize {
    let m = CONSOLE.module(own());
    if m != 0 {
        ATTACHED.get_or_init(|| unsafe { console_sandbox::attach(m) });
    }
    m
}

pub fn sandbox_hits() -> Vec<String> {
    ATTACHED.get().cloned().unwrap_or_default()
}

fn loaded_module() -> usize {
    match (CONSOLE.handle.get(), ATTACHED.get()) {
        (Some(&m), Some(_)) => m,
        _ => 0,
    }
}

type ClassObjectFn = unsafe extern "system" fn(*const Guid, *const Guid, *mut *mut c_void) -> i32;
type CanUnloadFn = unsafe extern "system" fn() -> i32;

#[no_mangle]
pub unsafe extern "system" fn DllGetClassObject(clsid: *const Guid, iid: *const Guid, ppv: *mut *mut c_void) -> i32 {
    match console_image::symbol(console_module(), "DllGetClassObject") {
        Some(p) => {
            let f: ClassObjectFn = core::mem::transmute(p);
            f(clsid, iid, ppv)
        }
        None => {
            if !ppv.is_null() {
                *ppv = core::ptr::null_mut();
            }
            CONSOLE.missing("DllGetClassObject", CLASS_E_CLASSNOTAVAILABLE as u32 as usize);
            CLASS_E_CLASSNOTAVAILABLE
        }
    }
}

#[no_mangle]
pub extern "system" fn DllCanUnloadNow() -> i32 {
    match console_image::symbol(loaded_module(), "DllCanUnloadNow") {
        Some(p) => unsafe {
            let f: CanUnloadFn = core::mem::transmute(p);
            f()
        },
        None => S_OK,
    }
}

#[no_mangle]
pub unsafe extern "system" fn DllGetActivationFactory(id: HSTRING, out: *mut *mut c_void) -> i32 {
    *out = core::ptr::null_mut();
    let mut has_null = 0;
    if WindowsIsStringEmpty(id) != 0 || WindowsStringHasEmbeddedNull(id, &mut has_null) < 0 || has_null == 1 {
        let name: Vec<u16> = ACTIVATABLE_CLASS_ID_NAME.encode_utf16().chain(Some(0)).collect();
        RoOriginateErrorW(E_INVALIDARG, 0x12, name.as_ptr());
        return E_INVALIDARG;
    }
    RoOriginateError(CLASS_E_CLASSNOTAVAILABLE, id);
    CLASS_E_CLASSNOTAVAILABLE
}

#[cfg(test)]
mod tests;
