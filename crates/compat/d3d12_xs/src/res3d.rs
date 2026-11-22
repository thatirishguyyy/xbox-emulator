use core::ffi::c_void;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use compat_common::record;

static LIVE: Mutex<Option<HashMap<usize, u32>>> = Mutex::new(None);

pub fn enabled() -> bool {
    static E: OnceLock<bool> = OnceLock::new();
    *E.get_or_init(|| std::env::var("TOMBOX_RES3D").map(|v| v == "1").unwrap_or(false))
}

fn desc_line(d: &[u8]) -> String {
    let u = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
    let w = u64::from_le_bytes(d[0x10..0x18].try_into().unwrap());
    let h = u(0x18);
    let da = u16::from_le_bytes(d[0x1c..0x1e].try_into().unwrap());
    let mips = u16::from_le_bytes(d[0x1e..0x20].try_into().unwrap());
    format!("{w}x{h}x{da} mips={mips} fmt={} layout={} flags={:#x}", u(0x20), u(0x2c), u(0x30))
}

pub fn note_create(res: *mut c_void, desc: &[u8], how: &str) {
    if !enabled() || res.is_null() || desc.len() < 0x38 || u32::from_le_bytes(desc[0..4].try_into().unwrap()) != 4 {
        return;
    }
    LIVE.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(res as usize, 0);
    record("d3d12_xs", "res3d", &format!("create res={res:p} {} via {how}", desc_line(desc)));
}

pub fn is_3d(res: usize) -> bool {
    enabled() && LIVE.lock().unwrap_or_else(|e| e.into_inner()).as_ref().is_some_and(|m| m.contains_key(&res))
}

pub fn note(res: usize, op: &str, detail: &str) {
    if !enabled() {
        return;
    }
    let n = {
        let mut g = LIVE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(c) = g.as_mut().and_then(|m| m.get_mut(&res)) else { return };
        *c += 1;
        *c
    };
    if n <= 64 {
        record("d3d12_xs", "res3d", &format!("#{n} {op} res={res:#x} {detail}"));
    }
}

pub fn note_view(res: *mut c_void, kind: &str, desc: Option<&[u8; 40]>, dest: usize) {
    if res.is_null() || !is_3d(res as usize) {
        return;
    }
    let (fmt, dim) = match desc {
        Some(d) => (u32::from_le_bytes(d[0..4].try_into().unwrap()), u32::from_le_bytes(d[4..8].try_into().unwrap())),
        None => (0, 0),
    };
    note(res as usize, kind, &format!("view fmt={fmt} dim={dim} dest={dest:#x}"));
}

pub unsafe fn scan_grid_upload(dst: usize, src: *mut c_void, off: u64, fmt: u32, w: u32, h: u32, d: u32, pitch: u32) {
    if !is_3d(dst) || fmt != 17 || src.is_null() || w == 0 || h == 0 || d == 0 {
        return;
    }
    let vt = unsafe { *(src as *const *const usize) };
    let map: unsafe extern "system" fn(*mut c_void, u32, *const c_void, *mut *mut u8) -> i32 = unsafe { core::mem::transmute(*vt.add(8)) };
    let unmap: unsafe extern "system" fn(*mut c_void, u32, *const c_void) = unsafe { core::mem::transmute(*vt.add(9)) };
    let mut p: *mut u8 = core::ptr::null_mut();
    if unsafe { map(src, 0, core::ptr::null(), &mut p) } < 0 || p.is_null() {
        note(dst, "grid-scan", "Map of the upload source failed");
        return;
    }
    let (mut lo, mut hi, mut nz, mut xmax) = (0u32, 0u32, 0u32, 0u32);
    let mut first = String::new();
    for z in 0..d {
        for y in 0..h {
            for x in 0..w {
                let a = off as usize + (z as usize * h as usize + y as usize) * pitch as usize + x as usize * 8;
                let v = unsafe { core::ptr::read_unaligned(p.add(a) as *const [u32; 2]) };
                lo = lo.max(v[1] & 0xffff);
                hi = hi.max(v[1] >> 16);
                xmax = xmax.max(v[0]);
                if v != [0, 0] {
                    nz += 1;
                    if first.is_empty() {
                        first = format!("first ({x},{y},{z})={:#x},{:#x}", v[0], v[1]);
                    }
                }
            }
        }
    }
    unsafe { unmap(src, 0, core::ptr::null()) };
    static SCANS: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
    static BIG: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
    let n = SCANS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    let big = lo > 512 || hi > 512;
    if (big && BIG.fetch_add(1, core::sync::atomic::Ordering::Relaxed) < 200) || n % 50 == 0 {
        record("d3d12_xs", "res3d", &format!("grid-scan #{n} res={dst:#x} off={off:#x} nonzero={nz}/{} max count1={lo} count2={hi} max offset={xmax} {first}", w * h * d));
    }
}
