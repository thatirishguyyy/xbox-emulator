use core::ffi::c_void;
use core::ptr::null_mut;

pub type HRESULT = i32;
pub type BSTR = *mut u16;

pub const S_OK: HRESULT = 0;
pub const E_FAIL: HRESULT = 0x8000_4005u32 as i32;
pub const E_INVALIDARG: HRESULT = 0x8007_0057u32 as i32;
pub const E_OUTOFMEMORY: HRESULT = 0x8007_000Eu32 as i32;
pub const E_BOUNDS: HRESULT = 0x8000_000Bu32 as i32;
pub const E_ACCESSDENIED: HRESULT = 0x8007_0005u32 as i32;
pub const HR_FILE_NOT_FOUND: HRESULT = 0x8007_0002u32 as i32;
pub const HR_PATH_NOT_FOUND: HRESULT = 0x8007_0003u32 as i32;
pub const HR_INVALID_DRIVE: HRESULT = 0x8007_000Fu32 as i32;
pub const E_JSON_NOT_FOUND: HRESULT = 0x8375_0009u32 as i32;

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GUID {
    pub data1: u32,
    pub data2: u16,
    pub data3: u16,
    pub data4: [u8; 8],
}

impl GUID {
    pub const fn new(data1: u32, data2: u16, data3: u16, data4: [u8; 8]) -> Self {
        GUID { data1, data2, data3, data4 }
    }
}

pub const IID_XBTP_EXTENSION_METADATA_IMPL: GUID = GUID::new(0xe2b6c063, 0x7d82, 0x414a, [0xbe, 0x86, 0x67, 0xed, 0x1b, 0x44, 0x77, 0x97]);
pub const IID_XBTP_MESSAGE_IMPL: GUID = GUID::new(0x6965e04a, 0x2829, 0x41a8, [0xb1, 0x94, 0x48, 0x73, 0x15, 0x07, 0x5e, 0x7a]);
pub const FILEIO_EXTENSION_ID: GUID = GUID::new(0x8a8b4d4c, 0x6da2, 0x4fc1, [0xb7, 0x8c, 0x3e, 0x45, 0x15, 0xcd, 0xf8, 0xed]);

pub type OperationFn = unsafe extern "system" fn(usize, *mut c_void, *mut c_void) -> HRESULT;

#[cfg(not(test))]
#[link(name = "xbtp", kind = "raw-dylib")]
extern "C" {
    pub fn XbtpCreateExtensionMetadata(iid: *const GUID, out: *mut *mut c_void) -> HRESULT;
    pub fn XbtpCreateMessage(iid: *const GUID, out: *mut *mut c_void) -> HRESULT;
    pub fn XbtpCreateJsonObject(out: *mut *mut c_void) -> HRESULT;
    pub fn XbtpCreateJsonArray(out: *mut *mut c_void) -> HRESULT;
    pub fn XbtpGetJsonObjectFromUtf8(utf8: *const u8, out: *mut *mut c_void) -> HRESULT;
    pub fn XbtpOpenSession2(address: *const u16, port: u32, options: *const c_void, extension: *const GUID, iid: *const GUID, out: *mut *mut c_void) -> HRESULT;
}

#[cfg(test)]
pub use xbtp_built::*;

#[cfg(test)]
mod xbtp_built {
    use super::{GUID, HRESULT};
    use core::ffi::c_void;
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;

    #[link(name = "kernel32")]
    extern "system" {
        fn LoadLibraryW(name: *const u16) -> *mut c_void;
        fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
    }

    fn newest_in_build_dir(build: &Path, name: &str) -> Option<PathBuf> {
        let stem = name.trim_end_matches(".dll");
        std::fs::read_dir(build.join(stem)).ok()?
            .flatten()
            .map(|e| e.path().join("out").join(name))
            .filter(|p| p.is_file())
            .max_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
    }

    pub fn built_dll(name: &str) -> PathBuf {
        let exe = std::env::current_exe().unwrap();
        exe.ancestors()
            .skip(1)
            .find_map(|a| [a.join(name), a.join("deps").join(name)].into_iter().find(|p| p.is_file()).or_else(|| newest_in_build_dir(&a.join("build"), name)))
            .unwrap_or_else(|| panic!("{name} is not built in the target dir above {}", exe.display()))
    }

    fn newest_build_copy() -> Option<PathBuf> {
        let exe = std::env::current_exe().ok()?;
        for a in exe.ancestors() {
            let Ok(rd) = std::fs::read_dir(a.join("build").join("xbtp")) else { continue };
            let src = rd.flatten().map(|e| e.path().join("out").join("xbtp.dll")).filter(|p| p.is_file()).max_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())?;
            let dir = std::env::temp_dir().join(format!("xtffileio-xbtp-{}", std::process::id()));
            std::fs::create_dir_all(&dir).ok()?;
            let copy = dir.join("xbtp.dll");
            std::fs::copy(&src, &copy).ok()?;
            return Some(copy);
        }
        None
    }

    fn module() -> *mut c_void {
        static XBTP: OnceLock<usize> = OnceLock::new();
        *XBTP.get_or_init(|| {
            let path = newest_build_copy().unwrap_or_else(|| built_dll("xbtp.dll"));
            let wide: Vec<u16> = path.as_os_str().to_string_lossy().encode_utf16().chain(Some(0)).collect();
            let h = unsafe { LoadLibraryW(wide.as_ptr()) };
            assert!(!h.is_null(), "LoadLibraryW({}) failed", path.display());
            h as usize
        }) as *mut c_void
    }

    pub unsafe fn xbtp_export(name: &[u8]) -> *mut c_void {
        export(name)
    }

    pub unsafe fn XbtpOpenSession2(address: *const u16, port: u32, options: *const c_void, extension: *const GUID, iid: *const GUID, out: *mut *mut c_void) -> HRESULT {
        let f: unsafe extern "C" fn(*const u16, u32, *const c_void, *const GUID, *const GUID, *mut *mut c_void) -> HRESULT = core::mem::transmute(export(b"XbtpOpenSession2\0"));
        f(address, port, options, extension, iid, out)
    }

    pub unsafe fn XbtpOpenSessionEx2(address: *const u16, port: u32, options: *const c_void, extension: *const GUID, iid: *const GUID, out: *mut *mut c_void) -> HRESULT {
        let f: unsafe extern "C" fn(*const u16, u32, *const c_void, *const GUID, *const GUID, *mut *mut c_void) -> HRESULT = core::mem::transmute(export(b"XbtpOpenSessionEx2\0"));
        f(address, port, options, extension, iid, out)
    }

    unsafe fn export(name: &[u8]) -> *mut c_void {
        let p = GetProcAddress(module(), name.as_ptr());
        assert!(!p.is_null(), "xbtp.dll does not export {}", String::from_utf8_lossy(&name[..name.len() - 1]));
        p
    }

    pub unsafe fn XbtpCreateExtensionMetadata(iid: *const GUID, out: *mut *mut c_void) -> HRESULT {
        let f: unsafe extern "C" fn(*const GUID, *mut *mut c_void) -> HRESULT = core::mem::transmute(export(b"XbtpCreateExtensionMetadata\0"));
        f(iid, out)
    }

    pub unsafe fn XbtpCreateMessage(iid: *const GUID, out: *mut *mut c_void) -> HRESULT {
        let f: unsafe extern "C" fn(*const GUID, *mut *mut c_void) -> HRESULT = core::mem::transmute(export(b"XbtpCreateMessage\0"));
        f(iid, out)
    }

    pub unsafe fn XbtpCreateJsonObject(out: *mut *mut c_void) -> HRESULT {
        let f: unsafe extern "C" fn(*mut *mut c_void) -> HRESULT = core::mem::transmute(export(b"XbtpCreateJsonObject\0"));
        f(out)
    }

    pub unsafe fn XbtpCreateJsonArray(out: *mut *mut c_void) -> HRESULT {
        let f: unsafe extern "C" fn(*mut *mut c_void) -> HRESULT = core::mem::transmute(export(b"XbtpCreateJsonArray\0"));
        f(out)
    }

    pub unsafe fn XbtpGetJsonObjectFromUtf8(utf8: *const u8, out: *mut *mut c_void) -> HRESULT {
        let f: unsafe extern "C" fn(*const u8, *mut *mut c_void) -> HRESULT = core::mem::transmute(export(b"XbtpGetJsonObjectFromUtf8\0"));
        f(utf8, out)
    }
}

#[link(name = "oleaut32")]
extern "system" {
    pub fn SysAllocString(s: *const u16) -> BSTR;
    pub fn SysAllocStringLen(s: *const u16, n: u32) -> BSTR;
    pub fn SysFreeString(s: BSTR);
    pub fn SysStringLen(s: BSTR) -> u32;
}

#[repr(C)]
pub struct UnknownVtbl {
    pub query_interface: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    pub add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    pub release: unsafe extern "system" fn(*mut c_void) -> u32,
}

#[repr(C)]
pub struct MetadataVtbl {
    pub base: UnknownVtbl,
    pub get_handler_count: usize,
    pub get_handlers: usize,
    pub add_handlers: usize,
    pub add_function: unsafe extern "system" fn(*mut c_void, u32, Option<OperationFn>) -> HRESULT,
}

#[repr(C)]
pub struct JsonObjectVtbl {
    pub base: UnknownVtbl,
    pub get_number: unsafe extern "system" fn(*mut c_void, *const u16, *mut f64) -> HRESULT,
    pub set_number: unsafe extern "system" fn(*mut c_void, *const u16, f64) -> HRESULT,
    pub get_string: unsafe extern "system" fn(*mut c_void, *const u16, *mut BSTR) -> HRESULT,
    pub set_string: unsafe extern "system" fn(*mut c_void, *const u16, *const u16) -> HRESULT,
    pub get_object_alias: unsafe extern "system" fn(*mut c_void, *const u16, *mut *mut c_void) -> HRESULT,
    pub get_object: unsafe extern "system" fn(*mut c_void, *const u16, *mut *mut c_void) -> HRESULT,
    pub set_object: unsafe extern "system" fn(*mut c_void, *const u16, *mut c_void) -> HRESULT,
    pub get_array: unsafe extern "system" fn(*mut c_void, *const u16, *mut *mut c_void) -> HRESULT,
    pub set_array: unsafe extern "system" fn(*mut c_void, *const u16, *mut c_void) -> HRESULT,
    pub stringify: unsafe extern "system" fn(*mut c_void, *mut BSTR) -> HRESULT,
}

#[repr(C)]
pub struct JsonArrayVtbl {
    pub base: UnknownVtbl,
    pub add_number: unsafe extern "system" fn(*mut c_void, f64) -> HRESULT,
    pub add_string: unsafe extern "system" fn(*mut c_void, *const u16) -> HRESULT,
    pub add_object: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    pub add_json: unsafe extern "system" fn(*mut c_void, *const u16) -> HRESULT,
    pub get_number: unsafe extern "system" fn(*mut c_void, u32, *mut f64) -> HRESULT,
    pub get_string: unsafe extern "system" fn(*mut c_void, u32, *mut BSTR) -> HRESULT,
    pub get_object: unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> HRESULT,
    pub get_value_type: unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> HRESULT,
    pub get_count: unsafe extern "system" fn(*mut c_void, *mut u32) -> HRESULT,
}

#[repr(C)]
pub struct MessageVtbl {
    pub base: UnknownVtbl,
    pub get_json_body: unsafe extern "system" fn(*mut c_void, *mut u32, *mut *mut u8) -> HRESULT,
    pub get_binary_body: unsafe extern "system" fn(*mut c_void, *mut u32, *mut *mut u8) -> HRESULT,
    pub set_json_body: unsafe extern "system" fn(*mut c_void, u32, *const u8) -> HRESULT,
    pub set_json_body_w: unsafe extern "system" fn(*mut c_void, *const u16) -> HRESULT,
    pub set_binary_body: unsafe extern "system" fn(*mut c_void, u32, *const u8) -> HRESULT,
}

#[repr(C)]
pub struct MessageImplVtbl {
    pub base: UnknownVtbl,
    pub get_json_body: unsafe extern "system" fn(*mut c_void, *mut u32, *mut *mut u8) -> HRESULT,
    pub get_binary_body: unsafe extern "system" fn(*mut c_void, *mut u32, *mut *mut u8) -> HRESULT,
    pub set_json_body: unsafe extern "system" fn(*mut c_void, u32, *const u8) -> HRESULT,
    pub set_json_body_copy: unsafe extern "system" fn(*mut c_void, u32, *const u8) -> HRESULT,
    pub set_json_body_w: unsafe extern "system" fn(*mut c_void, *const u16) -> HRESULT,
    pub set_binary_body: unsafe extern "system" fn(*mut c_void, u32, *const u8) -> HRESULT,
    pub set_binary_body_copy: unsafe extern "system" fn(*mut c_void, u32, *const u8) -> HRESULT,
}

#[repr(C)]
pub struct SessionVtbl {
    pub base: UnknownVtbl,
    pub send_more_message: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    pub send_more_message_async: unsafe extern "system" fn(*mut c_void, *mut c_void, Option<MessageSentCallback>, *mut c_void) -> HRESULT,
    pub receive_more_message: unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> HRESULT,
}

pub type MessageSentCallback = unsafe extern "system" fn(usize, usize, usize, *mut c_void);

pub const E_PENDING: HRESULT = 0x8000_000Au32 as i32;
pub const E_UNEXPECTED: HRESULT = 0x8000_FFFFu32 as i32;
pub const HR_CANCELLED: HRESULT = 0x8007_04C7u32 as i32;
pub const HR_WSAECONNRESET: HRESULT = 0x8007_2746u32 as i32;
pub const HR_IO_DEVICE: HRESULT = 0x8007_045Du32 as i32;
pub const HR_FILE_EXISTS: HRESULT = 0x8007_0050u32 as i32;
pub const XTF_RECEIVE_RECOMMENDED_TIMEOUT_MS: u32 = 120_000;

pub unsafe fn vt<T>(p: *mut c_void) -> &'static T {
    &**(p as *const *const T)
}

pub unsafe fn release(p: *mut c_void) {
    if !p.is_null() {
        (vt::<UnknownVtbl>(p).release)(p);
    }
}

pub struct Com(pub *mut c_void);

impl Com {
    pub fn null() -> Self {
        Com(null_mut())
    }

    pub fn out(&mut self) -> *mut *mut c_void {
        unsafe { release(core::mem::replace(&mut self.0, null_mut())) };
        &mut self.0
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        unsafe { release(core::mem::replace(&mut self.0, null_mut())) };
    }
}

pub struct Bstr(pub BSTR);

impl Bstr {
    pub fn null() -> Self {
        Bstr(null_mut())
    }

    pub fn from_str(s: &str) -> Result<Self, HRESULT> {
        let w = wide(s);
        let b = unsafe { SysAllocString(w.as_ptr()) };
        if b.is_null() {
            Err(E_OUTOFMEMORY)
        } else {
            Ok(Bstr(b))
        }
    }

    pub fn to_string(&self) -> String {
        unsafe { from_wide(self.0) }
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_null() || unsafe { *self.0 == 0 }
    }

    pub fn out(&mut self) -> *mut BSTR {
        if !self.0.is_null() {
            unsafe { SysFreeString(self.0) };
            self.0 = null_mut();
        }
        &mut self.0
    }
}

impl Drop for Bstr {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { SysFreeString(self.0) };
        }
    }
}

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(core::iter::once(0)).collect()
}

pub unsafe fn from_wide(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut n = 0;
    while *p.add(n) != 0 {
        n += 1;
    }
    String::from_utf16_lossy(core::slice::from_raw_parts(p, n))
}

pub fn hr_from_win32(e: u32) -> HRESULT {
    if (e as i32) <= 0 {
        e as i32
    } else {
        ((e & 0xFFFF) | 0x8007_0000) as i32
    }
}

pub fn hr_from_last_error_or_fail(e: u32) -> HRESULT {
    if e == 0 {
        E_FAIL
    } else {
        hr_from_win32(e)
    }
}
