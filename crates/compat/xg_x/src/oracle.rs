use core::ffi::c_void;
use std::sync::{Mutex, MutexGuard};

extern "system" {
    fn LoadLibraryW(p: *const u16) -> *mut c_void;
    fn GetProcAddress(h: *mut c_void, n: *const u8) -> *mut c_void;
}

static SERIAL: Mutex<()> = Mutex::new(());

pub fn serial() -> MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub struct Oracle(pub *mut c_void, #[allow(dead_code)] MutexGuard<'static, ()>);

pub fn oracle() -> Option<Oracle> {
    let guard = serial();
    let xmem = r"D:\xbox-pc-port\emulator\target\release\xmem.dll";
    let dll = xone_exports::reference_dlls("xg_x.dll").into_iter().next().expect("no console xg_x.dll in the game dumps; the console oracle tests need one");
    assert!(std::path::Path::new(xmem).is_file(), "{xmem} missing; the console oracle tests need it");
    unsafe {
        assert!(!LoadLibraryW(wide(xmem).as_ptr()).is_null(), "LoadLibrary failed for {xmem}");
        let h = LoadLibraryW(wide(&dll.to_string_lossy()).as_ptr());
        assert!(!h.is_null(), "LoadLibrary failed for {}", dll.display());
        Some(Oracle(h, guard))
    }
}

impl Oracle {
    pub fn f(&self, name: &str) -> usize {
        let n = format!("{name}\0");
        let p = unsafe { GetProcAddress(self.0, n.as_ptr()) } as usize;
        assert!(p != 0, "{name}");
        p
    }

    #[allow(dead_code)]
    pub fn rva(&self, rva: usize) -> usize {
        self.0 as usize + rva
    }

    pub fn hardware_version(&self, version: i32) {
        let f: extern "system" fn(i32) = unsafe { core::mem::transmute(self.f("XGSetHardwareVersion")) };
        f(version);
        crate::XGSetHardwareVersion(version);
    }
}

impl Drop for Oracle {
    fn drop(&mut self) {
        let f: extern "system" fn(i32) = unsafe { core::mem::transmute(self.f("XGSetHardwareVersion")) };
        f(0);
        crate::XGSetHardwareVersion(0);
    }
}
