use std::collections::HashSet;
use std::sync::{Mutex, MutexGuard, OnceLock};

pub const DEVICE_PATH: &str = r"\\?\acpi_hal#logan#0#{f764922a-de30-40a5-a66e-c46cb191a1cd}";
pub const IOCTL_MAP_REGION: u32 = 0x8000_e023;
pub const REGION_AUDIO_POOL: u32 = 2;
pub const POOL_SIZE: usize = 0x10_0000;
pub const PAGE: usize = 0x400;
pub const PAGES: usize = POOL_SIZE / PAGE;
pub const FIRST_PAGE: usize = 2;
pub const RUN_MASK: u16 = 0x3ff;
pub const REF_SHIFT: u16 = 10;
pub const CONTINUATION: u16 = 0xffff;
pub const PARTITION_ID: u32 = 1;

const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
const ERROR_NOT_SUPPORTED: u32 = 50;

#[link(name = "kernel32")]
extern "system" {
    fn CreateFileW(name: *const u16, access: u32, share: u32, sa: *const core::ffi::c_void, disposition: u32, flags: u32, template: isize) -> isize;
    fn VirtualAlloc(addr: usize, size: usize, kind: u32, protect: u32) -> usize;
    fn SetLastError(e: u32);
}

#[derive(Default)]
struct Logan {
    handles: HashSet<usize>,
    pool: usize,
}

fn state() -> MutexGuard<'static, Logan> {
    static S: OnceLock<Mutex<Logan>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(Logan::default())).lock().unwrap_or_else(|e| e.into_inner())
}

pub fn is_device_path(p: &str) -> bool {
    p.eq_ignore_ascii_case(DEVICE_PATH)
}

pub unsafe fn open() -> isize {
    let nul: Vec<u16> = "NUL".encode_utf16().chain(Some(0)).collect();
    let h = CreateFileW(nul.as_ptr(), 0xC000_0000, 3, core::ptr::null(), 3, 0, 0);
    if h != -1 {
        state().handles.insert(h as usize);
        compat_common::record_raw("logan", "CreateFileW", "opened the emulated acpi_hal Logan device");
    }
    h
}

pub fn is_device(h: usize) -> bool {
    state().handles.contains(&h)
}

pub fn forget(h: usize) {
    state().handles.remove(&h);
}

pub fn pool() -> usize {
    let mut s = state();
    if s.pool == 0 {
        s.pool = unsafe { VirtualAlloc(0, POOL_SIZE, 0x3000, 4) };
    }
    s.pool
}

pub fn pool_alloc(pages: usize) -> Option<u16> {
    if pages == 0 || pages > RUN_MASK as usize {
        return None;
    }
    let base = pool();
    if base == 0 {
        return None;
    }
    let _guard = state();
    let table = base as *mut u16;
    unsafe {
        let mut i = FIRST_PAGE;
        while i + pages <= PAGES {
            if (0..pages).all(|k| table.add(i + k).read_volatile() == 0) {
                table.add(i).write_volatile(pages as u16 | (1 << REF_SHIFT));
                for k in 1..pages {
                    table.add(i + k).write_volatile(CONTINUATION);
                }
                core::ptr::write_bytes((base + i * PAGE) as *mut u8, 0, pages * PAGE);
                return Some(i as u16);
            }
            i += 1;
        }
    }
    None
}

pub fn pool_entry(index: usize) -> u16 {
    let base = pool();
    if base == 0 || index >= PAGES {
        return 0;
    }
    unsafe { (base as *const u16).add(index).read_volatile() }
}

pub fn pool_release(index: usize) {
    let base = pool();
    if base == 0 || !(FIRST_PAGE..PAGES).contains(&index) {
        return;
    }
    let _guard = state();
    let table = base as *mut u16;
    unsafe {
        let e = table.add(index).read_volatile();
        if e == 0 || e == CONTINUATION {
            return;
        }
        let refs = (e >> REF_SHIFT) & 0xf;
        if refs > 1 {
            table.add(index).write_volatile((e & RUN_MASK) | ((refs - 1) << REF_SHIFT));
            return;
        }
        for k in 0..(e & RUN_MASK) as usize {
            table.add(index + k).write_volatile(0);
        }
    }
}

pub unsafe fn ioctl(code: u32, inb: *mut u8, inl: u32, outb: *mut u8, outl: u32, ret: *mut u32) -> i32 {
    if code != IOCTL_MAP_REGION {
        compat_common::record_raw("logan", "DeviceIoControl", &format!("ioctl {code:#x} is not emulated; ERROR_NOT_SUPPORTED"));
        SetLastError(ERROR_NOT_SUPPORTED);
        return 0;
    }
    if inb.is_null() || outb.is_null() || inl < 0x1c || outl < 0x18 {
        SetLastError(if inl < 0x1c || outl < 0x18 { ERROR_INSUFFICIENT_BUFFER } else { ERROR_INVALID_PARAMETER });
        return 0;
    }
    let kind = (inb as *const u32).read_unaligned();
    if kind != REGION_AUDIO_POOL {
        compat_common::record_raw("logan", "DeviceIoControl", &format!("e023 region type {kind} is not emulated; ERROR_NOT_SUPPORTED"));
        SetLastError(ERROR_NOT_SUPPORTED);
        return 0;
    }
    let va = pool();
    if va == 0 {
        SetLastError(8);
        return 0;
    }
    (inb.add(0x18) as *mut u32).write_unaligned(PARTITION_ID);
    core::ptr::write_bytes(outb, 0, 0x18);
    (outb as *mut u64).write_unaligned(va as u64);
    (outb.add(8) as *mut u32).write_unaligned(POOL_SIZE as u32);
    (outb.add(0x10) as *mut u64).write_unaligned(0);
    if !ret.is_null() {
        *ret = 0x18;
    }
    compat_common::record_raw("logan", "DeviceIoControl", &format!("e023 audio pool -> {va:#x}, {POOL_SIZE:#x} bytes"));
    1
}
