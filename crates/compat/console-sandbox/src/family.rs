pub const DEVICEFAMILYINFOENUM_DESKTOP: u32 = 3;
pub const DEVICEFAMILYINFOENUM_XBOX: u32 = 5;
pub const CURRENT_VERSION: &str = "\\Registry\\Machine\\Software\\Microsoft\\Windows NT\\CurrentVersion";
pub const OEM: &str = "\\Registry\\Machine\\Software\\Microsoft\\Windows NT\\CurrentVersion\\OEM";
pub const MAJOR: u64 = 10;
pub const MINOR: u64 = 0;

fn dword(path: &str, name: &str) -> Option<u32> {
    let v = crate::registry::get(path, name)?;
    (v.kind == crate::registry::REG_DWORD && v.data.len() == 4).then(|| u32::from_le_bytes([v.data[0], v.data[1], v.data[2], v.data[3]]))
}

pub fn platform() -> u32 {
    DEVICEFAMILYINFOENUM_XBOX
}

pub fn version() -> u64 {
    let build = compat_common::os_host::routes::current_build() as u64 & 0xffff;
    let ubr = dword(CURRENT_VERSION, "UBR").unwrap_or(0) as u64 & 0xffff;
    (MAJOR << 48) | (MINOR << 32) | (build << 16) | ubr
}

pub fn form() -> u32 {
    dword(OEM, "DeviceForm").unwrap_or(0)
}

pub unsafe extern "system" fn rtl_get_device_family_info_enum(version_out: *mut u64, family: *mut u32, form_out: *mut u32) {
    if !version_out.is_null() {
        *version_out = version();
    }
    if !family.is_null() {
        *family = platform();
    }
    if !form_out.is_null() {
        *form_out = form();
    }
}
