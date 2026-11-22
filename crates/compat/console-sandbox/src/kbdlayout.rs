use core::ffi::c_void;
use std::path::PathBuf;

#[link(name = "kernel32")]
extern "system" {
    fn LoadLibraryExW(name: *const u16, file: *mut c_void, flags: u32) -> usize;
}

const LOAD_WITH_ALTERED_SEARCH_PATH: u32 = 8;

pub fn is_layout_name(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    !n.contains(['\\', '/', ':']) && n.starts_with("kbd") && n.ends_with(".dll") && n.len() > 7
}

pub fn console_copy(name: &str) -> Option<PathBuf> {
    if !is_layout_name(name) {
        return None;
    }
    let mut dirs: Vec<PathBuf> = Vec::new();
    for (m, _) in crate::modules::attached() {
        if let Some(d) = crate::modules::module_file(m).and_then(|p| p.parent().map(|d| d.to_path_buf())) {
            if !dirs.contains(&d) {
                dirs.push(d);
            }
        }
    }
    for d in dirs {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            if e.file_name().to_string_lossy().eq_ignore_ascii_case(name) && e.path().is_file() {
                return Some(e.path());
            }
        }
    }
    None
}

pub unsafe extern "system" fn load_library_ex_w(name: *const u16, file: *mut c_void, flags: u32) -> usize {
    if !name.is_null() {
        let n = crate::wstr::from_ptr(name);
        if let Some(p) = console_copy(&n) {
            let w: Vec<u16> = p.to_string_lossy().encode_utf16().chain(Some(0)).collect();
            let h = LoadLibraryExW(w.as_ptr(), core::ptr::null_mut(), (flags & !0x1f00) | LOAD_WITH_ALTERED_SEARCH_PATH);
            compat_common::record_raw("kbdlayout", "LoadLibraryExW", &format!("{n} -> console layout {} = {h:#x}", p.display()));
            return h;
        }
    }
    LoadLibraryExW(name, file, flags)
}
