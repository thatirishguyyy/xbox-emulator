use core::ffi::c_void;

pub const CONDRV: &str = "\\Device\\ConDrv";
pub const DISPLAY: &str = "\\Device\\ConDrv\\Display";
const STATUS_OBJECT_NAME_NOT_FOUND: i32 = 0xC000_0034u32 as i32;

unsafe fn object_name(oa: *const u8) -> Option<String> {
    if oa.is_null() || (oa.add(8) as *const usize).read_unaligned() != 0 {
        return None;
    }
    let u = (oa.add(16) as *const usize).read_unaligned();
    (u != 0).then(|| crate::wstr::from_unicode_string(u as *const u8))
}

pub fn is_condrv(name: &str) -> bool {
    let n = name.trim_end_matches('\\');
    n.len() >= CONDRV.len() && n[..CONDRV.len()].eq_ignore_ascii_case(CONDRV) && (n.len() == CONDRV.len() || n.as_bytes()[CONDRV.len()] == b'\\')
}

fn absent(name: &str, func: &str) -> i32 {
    compat_common::record_raw("condrv", func, &format!("{name}: the console driver ConDrv (condrv.sys) is not part of the title dumps and is not emulated, so the device is not present (STATUS_OBJECT_NAME_NOT_FOUND)"));
    STATUS_OBJECT_NAME_NOT_FOUND
}

pub unsafe extern "system" fn nt_open_file(h: *mut usize, access: u32, oa: *const u8, io: *mut usize, share: u32, opts: u32) -> i32 {
    if let Some(n) = object_name(oa) {
        if is_condrv(&n) {
            return absent(&n, "NtOpenFile");
        }
        if crate::conkbd::is_keyboard_name(&n) {
            return crate::conkbd::open(&n, h, io);
        }
    }
    crate::xvcmgmt::nt_open_file(h, access, oa, io, share, opts)
}

pub unsafe extern "system" fn nt_create_file(h: *mut usize, access: u32, oa: *const u8, io: *mut usize, alloc: *const i64, attrs: u32, share: u32, disp: u32, opts: u32, ea: *const c_void, ea_len: u32) -> i32 {
    if let Some(n) = object_name(oa) {
        if is_condrv(&n) {
            return absent(&n, "NtCreateFile");
        }
        if crate::conkbd::is_keyboard_name(&n) {
            return if disp == 1 || disp == 3 { crate::conkbd::open(&n, h, io) } else { STATUS_OBJECT_NAME_NOT_FOUND };
        }
    }
    crate::xvcmgmt::nt_create_file(h, access, oa, io, alloc, attrs, share, disp, opts, ea, ea_len)
}
