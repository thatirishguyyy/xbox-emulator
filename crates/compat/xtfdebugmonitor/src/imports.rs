pub const CONSOLE_XBTP_IMPORTS: [&str; 9] = [
    "XbtpCreateExtensionMetadata",
    "XbtpCreateJsonArray",
    "XbtpCreateJsonObject",
    "XbtpCreateMessage",
    "XbtpGetCurrentTlsEventData",
    "XbtpGetJsonObjectFromUtf8",
    "XbtpOpenSession2",
    "?LogInfoV@@YAXPEBDPEAD@Z",
    "?LogWarningV@@YAXPEBDPEAD@Z",
];

#[cfg(not(test))]
mod linked {
    use core::ffi::c_void;
    use xtfmsg::sys::{GUID, HRESULT};

    #[link(name = "xbtp", kind = "raw-dylib")]
    extern "C" {
        pub fn XbtpCreateExtensionMetadata(iid: *const GUID, out: *mut *mut c_void) -> HRESULT;
        pub fn XbtpCreateJsonArray(out: *mut *mut c_void) -> HRESULT;
        pub fn XbtpCreateJsonObject(out: *mut *mut c_void) -> HRESULT;
        pub fn XbtpCreateMessage(iid: *const GUID, out: *mut *mut c_void) -> HRESULT;
        pub fn XbtpGetCurrentTlsEventData(out: *mut *mut c_void) -> HRESULT;
        pub fn XbtpGetJsonObjectFromUtf8(utf8: *const u8, out: *mut *mut c_void) -> HRESULT;
        pub fn XbtpOpenSession2(address: *const u16, port: u32, options: *const c_void, extension: *const GUID, iid: *const GUID, out: *mut *mut c_void) -> HRESULT;
        #[link_name = "?LogInfoV@@YAXPEBDPEAD@Z"]
        pub fn LogInfoV(fmt: *const u8, args: *const usize);
        #[link_name = "?LogWarningV@@YAXPEBDPEAD@Z"]
        pub fn LogWarningV(fmt: *const u8, args: *const usize);
    }

    pub fn resolve(name: &[u8]) -> Option<usize> {
        let name = name.strip_suffix(&[0]).unwrap_or(name);
        Some(match name {
            b"XbtpCreateExtensionMetadata" => XbtpCreateExtensionMetadata as *const () as usize,
            b"XbtpCreateJsonArray" => XbtpCreateJsonArray as *const () as usize,
            b"XbtpCreateJsonObject" => XbtpCreateJsonObject as *const () as usize,
            b"XbtpCreateMessage" => XbtpCreateMessage as *const () as usize,
            b"XbtpGetCurrentTlsEventData" => XbtpGetCurrentTlsEventData as *const () as usize,
            b"XbtpGetJsonObjectFromUtf8" => XbtpGetJsonObjectFromUtf8 as *const () as usize,
            b"XbtpOpenSession2" => XbtpOpenSession2 as *const () as usize,
            b"?LogInfoV@@YAXPEBDPEAD@Z" => LogInfoV as *const () as usize,
            b"?LogWarningV@@YAXPEBDPEAD@Z" => LogWarningV as *const () as usize,
            _ => return None,
        })
    }
}

#[cfg(not(test))]
pub fn bind() {
    xtfmsg::xbtp::bind_static_imports(linked::resolve);
}

#[cfg(test)]
pub fn bind() {}
