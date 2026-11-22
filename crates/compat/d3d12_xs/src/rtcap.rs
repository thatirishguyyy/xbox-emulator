//! Diagnostic render-target capture (off unless `TOMBOX_RTCAP=1`).
//!
//! Purpose: find *where* a frame goes wrong. Every render-target / depth texture the title
//! creates is remembered; when the trigger file `%TEMP%\tombox_capture_now` exists at a
//! `PresentX`, each one is read back and written to `%TEMP%\tombox_rtcap\<n>\` as a BMP
//! preview plus one line of statistics (min / max / mean per channel, NaN / Inf counts) in
//! `report.txt`.
//!
//! This is a development tool, not part of the compatibility surface:
//!   * it holds a reference on every render target for the life of the process (leak);
//!   * the copy assumes the state recorded from the title's own `ResourceBarrier` calls in
//!     *recording* order, which can differ from execution order;
//!   * only subresource 0 (mip 0, slice 0) of non-multisampled 2D textures is captured;
//!   * targets are captured at end of frame, so pooled targets show their last use only.

use core::ffi::c_void;
use std::collections::HashMap;
use std::io::Write;
use std::sync::{Mutex, OnceLock};
use compat_common::record;

#[repr(C)]
struct Guid(u32, u16, u16, [u8; 8]);
const IID_DEVICE: Guid = Guid(0x189819f1, 0x1db6, 0x4b57, [0xbe, 0x54, 0x18, 0x21, 0x33, 0x9b, 0x85, 0xf7]);
const IID_ALLOCATOR: Guid = Guid(0x6102dee4, 0xaf59, 0x4b09, [0xb9, 0x99, 0xb4, 0x4d, 0x73, 0xf0, 0x9b, 0x24]);
const IID_GFX_LIST: Guid = Guid(0x5b160d0f, 0xac1b, 0x4185, [0x8b, 0xa8, 0xb3, 0xae, 0x42, 0xa5, 0xa4, 0x55]);
const IID_FENCE: Guid = Guid(0x0a753dcf, 0xc4d8, 0x4b91, [0xad, 0xf6, 0xbe, 0x5a, 0x60, 0xd9, 0x5a, 0x76]);
const IID_RESOURCE: Guid = Guid(0x696442be, 0xa72e, 0x4059, [0xbc, 0x79, 0x5b, 0x5c, 0x98, 0x04, 0x0f, 0xad]);

extern "system" {
    fn Sleep(ms: u32);
}

unsafe fn vt(obj: *mut c_void) -> *const usize {
    *(obj as *const *const usize)
}
macro_rules! vcall {
    ($obj:expr, $slot:expr, fn($($at:ty),*) -> $rt:ty $(, $arg:expr)*) => {{
        let f: unsafe extern "system" fn(*mut c_void $(, $at)*) -> $rt =
            core::mem::transmute(*vt($obj).add($slot));
        f($obj $(, $arg)*)
    }};
}

pub fn enabled() -> bool {
    static E: OnceLock<bool> = OnceLock::new();
    *E.get_or_init(|| {
        std::env::var("TOMBOX_RTCAP").map(|v| v == "1").unwrap_or(false)
            || std::env::var("TOMBOX_RECORD").map(|v| v == "1").unwrap_or(false)
            || compat_common::cfg("rtcap").as_deref() == Some("1")
    })
}

struct Target {
    res: usize,
    desc: [u8; 0x38],
    order: usize,
}
static TARGETS: Mutex<Vec<Target>> = Mutex::new(Vec::new());
/// Last known state per resource (from creation, then the title's transition barriers).
static STATES: Mutex<Option<HashMap<usize, u32>>> = Mutex::new(None);

/// Remember a freshly created render-target / depth texture (takes a reference).
pub unsafe fn register(res: *mut c_void, desc: &[u8], init_state: u32) {
    if !enabled() || res.is_null() || desc.len() < 0x38 {
        return;
    }
    let dim = u32::from_le_bytes(desc[0..4].try_into().unwrap());
    let flags = u32::from_le_bytes(desc[0x30..0x34].try_into().unwrap());
    if dim != 3 || flags & 7 == 0 {
        return;
    }
    vcall!(res, 1, fn() -> u32);
    let mut t = TARGETS.lock().unwrap();
    let order = t.len();
    t.push(Target { res: res as usize, desc: desc[..0x38].try_into().unwrap(), order });
    STATES.lock().unwrap().get_or_insert_with(HashMap::new).insert(res as usize, init_state);
}

pub unsafe fn register_view_target(res: *mut c_void, init_state: u32) {
    if !enabled() || res.is_null() || TARGETS.lock().unwrap().iter().any(|t| t.res == res as usize) {
        return;
    }
    let mut d = [0u8; 0x38];
    vcall!(res, 10, fn(*mut u8) -> *mut u8, d.as_mut_ptr());
    let before = TARGETS.lock().unwrap().len();
    register(res, &d, init_state);
    let t = TARGETS.lock().unwrap();
    if t.len() > before {
        let u = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
        record("d3d12_xs", "rtcap", &format!(
            "view target #{:03} {res:p} {}x{} fmt={} flags={:#x} was not created through a tracked path; registered from its view (assumed state {init_state:#x}, placed={})",
            before, u64::from_le_bytes(d[0x10..0x18].try_into().unwrap()), u(0x18), u(0x20), u(0x30), crate::placed::lookup(res).is_some()));
    }
}

pub fn state_of(res: usize) -> Option<u32> {
    STATES.lock().unwrap().as_ref().and_then(|m| m.get(&res).copied())
}

/// Record transition barriers issued by the title (D3D12_RESOURCE_BARRIER is 32 bytes).
pub unsafe fn note_barriers(n: u32, p: *const u8) {
    if !enabled() || p.is_null() {
        return;
    }
    let mut g = STATES.lock().unwrap();
    let Some(m) = g.as_mut() else { return };
    for i in 0..n as usize {
        let b = p.add(i * 32);
        if core::ptr::read_unaligned(b as *const u32) != 0 {
            continue; // not a transition
        }
        let res = core::ptr::read_unaligned(b.add(8) as *const usize);
        let after = core::ptr::read_unaligned(b.add(24) as *const u32);
        if let Some(s) = m.get_mut(&res) {
            *s = after;
        }
    }
}

fn half(h: u16) -> f32 {
    let (s, e, m) = ((h >> 15) & 1, (h >> 10) & 0x1f, h & 0x3ff);
    let v = if e == 0 {
        (m as f32) * 2f32.powi(-24)
    } else if e == 31 {
        if m == 0 { f32::INFINITY } else { f32::NAN }
    } else {
        (1.0 + m as f32 / 1024.0) * 2f32.powi(e as i32 - 15)
    };
    if s == 1 { -v } else { v }
}
fn ufloat(bits: u32, mant_bits: u32) -> f32 {
    let m = bits & ((1 << mant_bits) - 1);
    let e = bits >> mant_bits;
    if e == 0 {
        (m as f32) / (1u32 << mant_bits) as f32 * 2f32.powi(-14)
    } else if e == 31 {
        if m == 0 { f32::INFINITY } else { f32::NAN }
    } else {
        (1.0 + m as f32 / (1u32 << mant_bits) as f32) * 2f32.powi(e as i32 - 15)
    }
}

/// Decode one pixel to up to four floats. Returns (channels, is_float_or_depth, bytes/pixel).
fn layout(fmt: u32) -> Option<(usize, bool, usize, &'static str)> {
    Some(match fmt {
        1..=2 => (4, true, 16, "rgba32f"),
        9..=10 => (4, true, 8, "rgba16f"),
        11 => (4, false, 8, "rgba16"),
        15..=16 => (2, true, 8, "rg32f"),
        19..=22 => (1, true, 4, "d32s8"), // plane 0 of the copy is a 32-bit float
        23..=24 => (4, false, 4, "rgb10a2"),
        26 => (3, true, 4, "r11g11b10f"),
        27..=29 => (4, false, 4, "rgba8"),
        33..=34 => (2, true, 4, "rg16f"),
        35 => (2, false, 4, "rg16"),
        39..=41 => (1, true, 4, "r32f"),
        44..=47 => (1, false, 4, "d24s8"),
        48..=49 => (2, false, 2, "rg8"),
        // 53 is R16_TYPELESS: in practice a 16-bit depth target (shadow atlas), so UNORM.
        54 => (1, true, 2, "r16f"),
        53 | 55..=56 => (1, false, 2, "r16"),
        60..=61 => (1, false, 1, "r8"),
        87 | 90 | 91 => (4, false, 4, "bgra8"),
        _ => return None,
    })
}

fn decode(kind: &str, p: &[u8]) -> [f32; 4] {
    let u16at = |o: usize| u16::from_le_bytes([p[o], p[o + 1]]);
    let u32at = |o: usize| u32::from_le_bytes([p[o], p[o + 1], p[o + 2], p[o + 3]]);
    let f32at = |o: usize| f32::from_bits(u32at(o));
    match kind {
        "rgba32f" => [f32at(0), f32at(4), f32at(8), f32at(12)],
        "rgba16f" => [half(u16at(0)), half(u16at(2)), half(u16at(4)), half(u16at(6))],
        "rgba16" => [u16at(0) as f32 / 65535.0, u16at(2) as f32 / 65535.0, u16at(4) as f32 / 65535.0, u16at(6) as f32 / 65535.0],
        "rg32f" => [f32at(0), f32at(4), 0.0, 1.0],
        "d32s8" | "r32f" => [f32at(0), 0.0, 0.0, 1.0],
        "rgb10a2" => {
            let v = u32at(0);
            [(v & 1023) as f32 / 1023.0, ((v >> 10) & 1023) as f32 / 1023.0, ((v >> 20) & 1023) as f32 / 1023.0, (v >> 30) as f32 / 3.0]
        }
        "r11g11b10f" => {
            let v = u32at(0);
            [ufloat(v & 0x7ff, 6), ufloat((v >> 11) & 0x7ff, 6), ufloat(v >> 22, 5), 1.0]
        }
        "rgba8" => [p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0, p[3] as f32 / 255.0],
        "bgra8" => [p[2] as f32 / 255.0, p[1] as f32 / 255.0, p[0] as f32 / 255.0, p[3] as f32 / 255.0],
        "rg16f" => [half(u16at(0)), half(u16at(2)), 0.0, 1.0],
        "rg16" => [u16at(0) as f32 / 65535.0, u16at(2) as f32 / 65535.0, 0.0, 1.0],
        "d24s8" => [(u32at(0) & 0xff_ffff) as f32 / 16777215.0, 0.0, 0.0, 1.0],
        "rg8" => [p[0] as f32 / 255.0, p[1] as f32 / 255.0, 0.0, 1.0],
        "r16f" => [half(u16at(0)), 0.0, 0.0, 1.0],
        "r16" => [u16at(0) as f32 / 65535.0, 0.0, 0.0, 1.0],
        _ => [p[0] as f32 / 255.0, 0.0, 0.0, 1.0],
    }
}

fn write_bmp(path: &std::path::Path, w: usize, h: usize, rgb: &[u8]) {
    let row = (w * 3 + 3) & !3;
    let size = 54 + row * h;
    let mut f = Vec::with_capacity(size);
    f.extend_from_slice(b"BM");
    f.extend_from_slice(&(size as u32).to_le_bytes());
    f.extend_from_slice(&[0; 4]);
    f.extend_from_slice(&54u32.to_le_bytes());
    f.extend_from_slice(&40u32.to_le_bytes());
    f.extend_from_slice(&(w as i32).to_le_bytes());
    f.extend_from_slice(&(-(h as i32)).to_le_bytes()); // top-down
    f.extend_from_slice(&1u16.to_le_bytes());
    f.extend_from_slice(&24u16.to_le_bytes());
    f.extend_from_slice(&[0; 24]);
    for y in 0..h {
        for x in 0..w {
            let p = &rgb[(y * w + x) * 3..];
            f.extend_from_slice(&[p[2], p[1], p[0]]);
        }
        f.extend(std::iter::repeat(0).take(row - w * 3));
    }
    let _ = std::fs::write(path, f);
}

fn barrier(res: usize, before: u32, after: u32) -> [u64; 4] {
    [0, res as u64, 0xffff_ffffu64 | ((before as u64) << 32), after as u64]
}

/// Called at PresentX. Captures when the trigger file exists.
pub unsafe fn maybe_capture(queue: *mut c_void, presented: *mut c_void) {
    if !enabled() {
        return;
    }
    static REMAIN: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
    static RAW: Mutex<Vec<usize>> = Mutex::new(Vec::new());
    let trigger = std::env::temp_dir().join("tombox_capture_now");
    let raw_ids: Vec<usize> = if trigger.exists() {
        let text = std::fs::read_to_string(&trigger).unwrap_or_default();
        let _ = std::fs::remove_file(&trigger);
        let frames = text.split_whitespace().find_map(|w| w.strip_prefix("frames=").and_then(|n| n.parse::<usize>().ok())).unwrap_or(1).clamp(1, 8);
        REMAIN.store(frames - 1, core::sync::atomic::Ordering::Relaxed);
        let ids: Vec<usize> = text.split_whitespace().filter_map(|w| w.parse().ok()).collect();
        *RAW.lock().unwrap() = ids.clone();
        ids
    } else if REMAIN.load(core::sync::atomic::Ordering::Relaxed) > 0 {
        REMAIN.fetch_sub(1, core::sync::atomic::Ordering::Relaxed);
        RAW.lock().unwrap().clone()
    } else {
        end_frame(None);
        return;
    };
    static SEQ: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
    let seq = SEQ.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join("tombox_rtcap").join(format!("{seq}"));
    let _ = std::fs::create_dir_all(&dir);
    let mut report = match std::fs::File::create(dir.join("report.txt")) {
        Ok(f) => f,
        Err(_) => return,
    };

    let mut device: *mut c_void = core::ptr::null_mut();
    if vcall!(queue, 7, fn(*const Guid, *mut *mut c_void) -> i32, &IID_DEVICE, &mut device) < 0 {
        return;
    }
    let mut alloc: *mut c_void = core::ptr::null_mut();
    let mut list: *mut c_void = core::ptr::null_mut();
    let mut fence: *mut c_void = core::ptr::null_mut();
    if vcall!(device, 9, fn(u32, *const Guid, *mut *mut c_void) -> i32, 0, &IID_ALLOCATOR, &mut alloc) < 0
        || vcall!(device, 12, fn(u32, u32, *mut c_void, *mut c_void, *const Guid, *mut *mut c_void) -> i32,
            0, 0, alloc, core::ptr::null_mut(), &IID_GFX_LIST, &mut list) < 0
        || vcall!(device, 36, fn(u64, u32, *const Guid, *mut *mut c_void) -> i32, 0, 0, &IID_FENCE, &mut fence) < 0
    {
        let _ = writeln!(report, "could not create capture command objects");
        return;
    }
    vcall!(list, 9, fn() -> i32);
    let mut fence_value = 0u64;

    let targets: Vec<(usize, [u8; 0x38], usize)> = TARGETS.lock().unwrap().iter().map(|t| (t.res, t.desc, t.order)).collect();
    let states = STATES.lock().unwrap().clone().unwrap_or_default();
    let _ = writeln!(report, "{} render-target/depth textures known", targets.len());
    let presented_order = targets.iter().find(|t| t.0 == presented as usize).map(|t| format!("#{:03}", t.2)).unwrap_or_else(|| format!("{presented:p}"));
    let _ = writeln!(report, "presented this frame: {presented_order}");
    let mut done = 0;
    for (res, d, order) in targets {
        let u32at = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
        let (w, h) = (u64::from_le_bytes(d[0x10..0x18].try_into().unwrap()) as usize, u32at(0x18) as usize);
        let (fmt, samples, flags) = (u32at(0x20), u32at(0x24), u32at(0x30));
        let state = states.get(&res).copied().unwrap_or(0);
        let va = crate::placed::lookup(res as *mut c_void).map(|p| p.va).unwrap_or(0);
        let head = format!("#{order:03} {w}x{h} fmt={fmt} flags={flags:#x} state={state:#x} res={res:#x} va={va:#x}");
        let Some((ch, is_float, bpp, kind)) = layout(fmt) else {
            let _ = writeln!(report, "{head}: format not decoded - skipped");
            continue;
        };
        if samples != 1 || w == 0 || h == 0 {
            let _ = writeln!(report, "{head}: multisampled/empty - skipped");
            continue;
        }
        let r = res as *mut c_void;
        let mut pc_desc = [0u8; 0x38];
        vcall!(r, 10, fn(*mut u8) -> *mut u8, pc_desc.as_mut_ptr());
        let mut fp = [0u8; 32];
        let (mut rows, mut row_size, mut total) = (0u32, 0u64, 0u64);
        vcall!(device, 38, fn(*const u8, u32, u32, u64, *mut [u8; 32], *mut u32, *mut u64, *mut u64) -> (),
            pc_desc.as_ptr(), 0, 1, 0, &mut fp, &mut rows, &mut row_size, &mut total);
        if total == 0 || total == u64::MAX {
            let _ = writeln!(report, "{head}: no copyable footprint - skipped");
            continue;
        }
        let pitch = u32::from_le_bytes(fp[0x18..0x1c].try_into().unwrap()) as usize;
        // Readback buffer.
        let heap: [u32; 5] = [3 /* READBACK */, 0, 0, 1, 1];
        let mut bd = [0u8; 0x38];
        bd[0..4].copy_from_slice(&1u32.to_le_bytes());
        bd[0x10..0x18].copy_from_slice(&total.to_le_bytes());
        bd[0x18..0x1c].copy_from_slice(&1u32.to_le_bytes());
        bd[0x1c..0x1e].copy_from_slice(&1u16.to_le_bytes());
        bd[0x1e..0x20].copy_from_slice(&1u16.to_le_bytes());
        bd[0x24..0x28].copy_from_slice(&1u32.to_le_bytes());
        bd[0x2c..0x30].copy_from_slice(&1u32.to_le_bytes());
        let mut buf: *mut c_void = core::ptr::null_mut();
        let hr = vcall!(device, 27, fn(*const u32, u32, *const u8, u32, *const c_void, *const Guid, *mut *mut c_void) -> i32,
            heap.as_ptr(), 0, bd.as_ptr(), 0x400, core::ptr::null(), &IID_RESOURCE, &mut buf);
        if hr < 0 || buf.is_null() {
            let _ = writeln!(report, "{head}: readback buffer failed hr={hr:#010x}");
            continue;
        }
        const COPY_SOURCE: u32 = 0x800;
        vcall!(alloc, 8, fn() -> i32);
        vcall!(list, 10, fn(*mut c_void, *mut c_void) -> i32, alloc, core::ptr::null_mut());
        if state != COPY_SOURCE {
            let b = [barrier(res, state, COPY_SOURCE)];
            vcall!(list, 26, fn(u32, *const [u64; 4]) -> (), 1, b.as_ptr());
        }
        let mut dst = [0u8; 48];
        dst[0..8].copy_from_slice(&(buf as u64).to_le_bytes());
        dst[8..12].copy_from_slice(&1u32.to_le_bytes());
        dst[16..48].copy_from_slice(&fp);
        let mut src = [0u8; 48];
        src[0..8].copy_from_slice(&(res as u64).to_le_bytes());
        vcall!(list, 16, fn(*const u8, u32, u32, u32, *const u8, *const c_void) -> (), dst.as_ptr(), 0, 0, 0, src.as_ptr(), core::ptr::null());
        if state != COPY_SOURCE {
            let b = [barrier(res, COPY_SOURCE, state)];
            vcall!(list, 26, fn(u32, *const [u64; 4]) -> (), 1, b.as_ptr());
        }
        vcall!(list, 9, fn() -> i32);
        let lists = [list];
        vcall!(queue, 10, fn(u32, *const *mut c_void) -> (), 1, lists.as_ptr());
        fence_value += 1;
        vcall!(queue, 14, fn(*mut c_void, u64) -> i32, fence, fence_value);
        let mut spins = 0;
        while vcall!(fence, 8, fn() -> u64) < fence_value && spins < 5000 {
            Sleep(1);
            spins += 1;
        }
        let mut data: *mut u8 = core::ptr::null_mut();
        let hr = vcall!(buf, 8, fn(u32, *const c_void, *mut *mut u8) -> i32, 0, core::ptr::null(), &mut data);
        if hr < 0 || data.is_null() || spins >= 5000 {
            let _ = writeln!(report, "{head}: readback map failed hr={hr:#010x} (waited {spins} ms)");
            vcall!(buf, 2, fn() -> u32);
            continue;
        }
        if raw_ids.contains(&order) {
            let mut out = Vec::with_capacity(w * h * 16);
            for y in 0..h {
                let row = core::slice::from_raw_parts(data.add(y * pitch), w * bpp);
                for x in 0..w {
                    for c in decode(kind, &row[x * bpp..x * bpp + bpp]) {
                        out.extend_from_slice(&c.to_le_bytes());
                    }
                }
            }
            let _ = std::fs::write(dir.join(format!("rt{order:03}_{w}x{h}.f32")), out);
        }
        // Statistics + preview (downscaled to <= 960 wide).
        let step = (w / 960).max(1);
        let (pw, ph) = (w / step, h / step);
        let mut rgb = vec![0u8; pw * ph * 3];
        let (mut mn, mut mx, mut sum) = ([f32::MAX; 4], [f32::MIN; 4], [0f64; 4]);
        let (mut nan, mut inf, mut big, mut n) = (0u64, 0u64, 0u64, 0u64);
        let mut vals: Vec<[f32; 4]> = Vec::with_capacity(pw * ph);
        for y in (0..h).step_by(step).take(ph) {
            let row = core::slice::from_raw_parts(data.add(y * pitch), w * bpp);
            for x in (0..w).step_by(step).take(pw) {
                let v = decode(kind, &row[x * bpp..x * bpp + bpp]);
                for c in 0..ch {
                    if v[c].is_nan() {
                        nan += 1;
                    } else if v[c].is_infinite() {
                        inf += 1;
                    } else {
                        mn[c] = mn[c].min(v[c]);
                        mx[c] = mx[c].max(v[c]);
                        sum[c] += v[c] as f64;
                        if v[c].abs() > 100.0 {
                            big += 1;
                        }
                    }
                }
                n += 1;
                vals.push(v);
            }
        }
        // Preview: floats are tone-mapped v/(1+v); single-channel targets (depth) are shown
        // on a log scale between the smallest and largest positive value, because reversed-Z
        // depth is tiny almost everywhere and a linear scale shows only the nearest pixel.
        let hi = mx[0];
        let lo_pos = vals.iter().map(|v| v[0]).filter(|x| x.is_finite() && *x > 0.0).fold(f32::MAX, f32::min);
        let nonzero = vals.iter().filter(|v| v[0].is_finite() && v[0] != 0.0).count();
        for (i, v) in vals.iter().enumerate() {
            let px = if ch == 1 {
                let t = if v[0] > 0.0 && hi > lo_pos && lo_pos > 0.0 { 0.15 + 0.85 * (v[0] / lo_pos).ln() / (hi / lo_pos).ln() } else { 0.0 };
                [t, t, t]
            } else if is_float {
                [v[0].max(0.0) / (1.0 + v[0].max(0.0)), v[1].max(0.0) / (1.0 + v[1].max(0.0)), v[2].max(0.0) / (1.0 + v[2].max(0.0))]
            } else {
                [v[0], v[1], v[2]]
            };
            for c in 0..3 {
                let t = if px[c].is_finite() { px[c].clamp(0.0, 1.0) } else { 1.0 };
                rgb[i * 3 + c] = (t.powf(if is_float && ch > 1 { 1.0 / 2.2 } else { 1.0 }) * 255.0) as u8;
            }
        }
        write_bmp(&dir.join(format!("rt{order:03}_{w}x{h}_fmt{fmt}.bmp")), pw, ph, &rgb);
        let stat = |c: usize| format!("[{:.4} .. {:.4} mean {:.4}]", mn[c], mx[c], sum[c] / n.max(1) as f64);
        let chans: Vec<String> = (0..ch).map(stat).collect();
        let extra = if ch == 1 {
            format!(" | nonzero {nonzero}/{n} smallest-positive {lo_pos:e} max {hi:e} mean {:e}", sum[0] / n.max(1) as f64)
        } else {
            String::new()
        };
        let _ = writeln!(report, "{head} {kind}: {} nan={nan} inf={inf} |v|>100: {big}{extra}", chans.join(" "));
        vcall!(buf, 9, fn(u32, *const c_void) -> (), 0, core::ptr::null());
        vcall!(buf, 2, fn() -> u32);
        done += 1;
    }
    vcall!(list, 2, fn() -> u32);
    vcall!(alloc, 2, fn() -> u32);
    vcall!(fence, 2, fn() -> u32);
    record("d3d12_xs", "rtcap", &format!("captured {done} render targets into {}", dir.display()));
    end_frame(Some(&dir));
}

// ---- draw trace ------------------------------------------------------------------------
//
// While the capture is enabled, every draw / dispatch the title records is attributed to
// (pipeline state, bound render targets, bound depth target). A capture writes the list for
// the frame just recorded to `draws.txt` and saves the original (Xbox) shader containers of
// every pipeline involved under `shaders/`, so a wrong render target can be traced back to
// the shaders that wrote it. Recording order across command lists is not execution order.

#[derive(Default)]
struct ListState {
    pso: usize,
    rtv: Vec<usize>,
    dsv: usize,
}
#[derive(Default)]
struct Trace {
    /// CPU descriptor handle -> resource, for render-target and depth views.
    views: HashMap<usize, usize>,
    rtv_increment: usize,
    lists: HashMap<usize, ListState>,
    /// compute pipeline -> translated code pointer
    compute: HashMap<usize, usize>,
    /// (pso, render targets, depth target, kind) in first-seen order, with a draw count.
    draws: Vec<((usize, Vec<usize>, usize, u8), u32)>,
    index: HashMap<(usize, Vec<usize>, usize, u8), usize>,
    /// Shader hashes listed in `%TEMP%	ombox_skip_shaders.txt` (re-read every present):
    /// draws and dispatches using a pipeline with one of them are dropped. A bisection aid.
    skip_hashes: Vec<u64>,
    skip_pso: HashMap<usize, bool>,
    /// Shader hashes in `%TEMP%	ombox_trace_shaders.txt`: draws using them log their
    /// arguments and vertex-buffer bindings (first few per frame).
    trace_hashes: Vec<u64>,
    trace_pso: HashMap<usize, bool>,
    trace_budget: u32,
    /// Last IASetVertexBuffers per list: (start slot, views as (address, size, stride)).
    vbs: HashMap<usize, Vec<(u32, u64, u32, u32)>>,
}
static TRACE: Mutex<Option<Trace>> = Mutex::new(None);

fn list_key(list: usize) -> usize {
    crate::list_split::origin_of(list as *mut core::ffi::c_void)
}

fn with_trace<R>(f: impl FnOnce(&mut Trace) -> R) -> R {
    let mut g = TRACE.lock().unwrap();
    f(g.get_or_insert_with(Trace::default))
}

pub fn note_view(handle: usize, res: usize, rtv_increment: usize) {
    with_trace(|t| {
        t.views.insert(handle, res);
        if rtv_increment != 0 {
            t.rtv_increment = rtv_increment;
        }
    });
}
pub fn note_compute_pso(pso: usize, code: usize) {
    with_trace(|t| {
        t.compute.insert(pso, code);
    });
}
pub fn on_set_pso(list: usize, pso: usize) {
    with_trace(|t| t.lists.entry(list_key(list)).or_default().pso = pso);
}
pub unsafe fn on_set_targets(list: usize, n: u32, handles: *const usize, single_range: bool, dsv: *const usize) {
    with_trace(|t| {
        let mut rtv = Vec::new();
        for i in 0..(n as usize).min(8) {
            if handles.is_null() {
                break;
            }
            let h = if single_range { (*handles).wrapping_add(i * t.rtv_increment) } else { *handles.add(i) };
            rtv.push(t.views.get(&h).copied().unwrap_or(0));
        }
        let d = if dsv.is_null() { 0 } else { t.views.get(&*dsv).copied().unwrap_or(usize::MAX) };
        let s = t.lists.entry(list_key(list)).or_default();
        s.rtv = rtv;
        s.dsv = d;
    });
}
pub unsafe fn on_set_vbs(list: usize, start: u32, n: u32, views: *const u8) {
    with_trace(|t| {
        let v = t.vbs.entry(list_key(list)).or_default();
        for i in 0..(n as usize).min(16) {
            if views.is_null() {
                break;
            }
            let p = views.add(i * 16);
            let slot = start + i as u32;
            v.retain(|e| e.0 != slot);
            v.push((slot, core::ptr::read_unaligned(p as *const u64), core::ptr::read_unaligned(p.add(8) as *const u32), core::ptr::read_unaligned(p.add(12) as *const u32)));
        }
    });
}

/// Log one traced draw or dispatch (see `trace_hashes`): its arguments, vertex buffers and
/// everything bound through descriptor tables (constant-buffer contents included).
pub fn trace_draw(list: usize, compute: bool, text: &str) {
    let line = with_trace(|t| {
        if t.trace_hashes.is_empty() || t.trace_budget == 0 {
            return None;
        }
        let pso = t.lists.get(&list_key(list)).map(|s| s.pso).unwrap_or(0);
        let hash_of = |c: usize| if c == 0 { 0 } else { crate::shader_xlat::origin(c).map(|(h, _)| h).unwrap_or(0) };
        let codes: Vec<usize> = if compute {
            vec![t.compute.get(&pso).copied().unwrap_or(0)]
        } else {
            crate::device_wrap::pso_summary(pso).map(|(v, p, _)| vec![v, p]).unwrap_or_default()
        };
        let hit = match t.trace_pso.get(&pso) {
            Some(&b) => b,
            None => {
                let b = codes.iter().any(|&c| c != 0 && t.trace_hashes.contains(&hash_of(c)));
                t.trace_pso.insert(pso, b);
                b
            }
        };
        if !hit {
            return None;
        }
        t.trace_budget -= 1;
        let s = t.lists.get(&list_key(list));
        let targets = s.map(|s| (s.rtv.clone(), s.dsv)).unwrap_or_default();
        let mut vb = t.vbs.get(&list_key(list)).cloned().unwrap_or_default();
        vb.sort();
        let vbs: Vec<String> = vb.iter().map(|(s, a, sz, st)| format!("{s}:{a:#x}+{sz}/{st}")).collect();
        let names: Vec<String> = codes.iter().map(|&c| format!("{:016x}", hash_of(c))).collect();
        Some((format!("shaders=[{}] {text} vb=[{}]", names.join(","), vbs.join(" ")), targets))
    });
    if let Some((l, (rtv, dsv))) = line {
        let targets: HashMap<usize, (usize, [u8; 0x38])> = TARGETS.lock().unwrap().iter().map(|t| (t.res, (t.order, t.desc))).collect();
        let name = |r: usize| -> String {
            let d32 = |d: &[u8], o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
            if let Some((order, d)) = targets.get(&r) {
                return format!("#{order:03} {}x{} fmt={}", u64::from_le_bytes(d[0x10..0x18].try_into().unwrap()), d32(d, 0x18), d32(d, 0x20));
            }
            match crate::placed::lookup(r as *mut c_void) {
                Some(p) => format!("placed {r:#x} {}x{}x{} mips={} fmt={}", p.width(), p.height(), p.depth_or_array(), p.mips(), p.format()),
                None => format!("res {r:#x}"),
            }
        };
        let rts: Vec<String> = rtv.iter().map(|r| if *r == 0 { "-".into() } else { name(*r) }).collect();
        let ds = if dsv == 0 { "-".to_string() } else { name(dsv) };
        let b = unsafe { crate::bindtrace::dump(list, compute, &name) };
        record("d3d12_xs", "drawtrace", &format!("{l} rt=[{}] ds={ds}\n{b}", rts.join(" | ")));
    }
}

/// kind: 0 draw, 1 indexed draw, 2 dispatch, 3 indirect. Returns false when the draw must
/// be dropped (its pipeline uses a shader on the skip list).
pub fn on_draw(list: usize, kind: u8) -> bool {
    with_trace(|t| {
        let pso = t.lists.entry(list_key(list)).or_default().pso;
        if !t.skip_hashes.is_empty() {
            let skip = match t.skip_pso.get(&pso) {
                Some(&b) => b,
                None => {
                    let mut codes = vec![t.compute.get(&pso).copied().unwrap_or(0)];
                    if let Some((v, p, _)) = crate::device_wrap::pso_summary(pso) {
                        codes.push(v);
                        codes.push(p);
                    }
                    let b = codes.iter().any(|&c| c != 0 && crate::shader_xlat::origin(c).map(|(h, _)| t.skip_hashes.contains(&h)).unwrap_or(false));
                    t.skip_pso.insert(pso, b);
                    b
                }
            };
            if skip {
                return false;
            }
        }
        let s = t.lists.entry(list_key(list)).or_default();
        let key = if kind == 2 { (s.pso, Vec::new(), 0, kind) } else { (s.pso, s.rtv.clone(), s.dsv, kind) };
        match t.index.get(&key) {
            Some(&i) => t.draws[i].1 += 1,
            None => {
                t.index.insert(key.clone(), t.draws.len());
                t.draws.push((key, 1));
            }
        }
        true
    })
}

pub fn frame_draws() -> Vec<crate::present_probe::Draw> {
    with_trace(|t| t.draws.iter().map(|((_, rtv, dsv, kind), c)| crate::present_probe::Draw { rtv: rtv.clone(), dsv: *dsv, kind: *kind, count: *c }).collect())
}

pub fn label(r: usize) -> String {
    if r == 0 {
        return "-".into();
    }
    if r == usize::MAX {
        return "?".into();
    }
    let d32 = |d: &[u8], o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
    if let Some(t) = TARGETS.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|t| t.res == r) {
        return format!("rt#{:03} {}x{} fmt={}", t.order, u64::from_le_bytes(t.desc[0x10..0x18].try_into().unwrap()), d32(&t.desc, 0x18), d32(&t.desc, 0x20));
    }
    match crate::placed::lookup(r as *mut c_void) {
        Some(p) => format!("placed {r:#x} {}x{} fmt={}", p.width(), p.height(), p.format()),
        None => format!("res {r:#x} (not a registered RT)"),
    }
}

/// Forget the draws recorded so far (called at every present).
fn take_draws() -> (Vec<((usize, Vec<usize>, usize, u8), u32)>, HashMap<usize, usize>) {
    with_trace(|t| {
        t.index.clear();
        (std::mem::take(&mut t.draws), t.compute.clone())
    })
}

fn write_draws(dir: &std::path::Path, draws: &[((usize, Vec<usize>, usize, u8), u32)], compute: &HashMap<usize, usize>) {
    let order: HashMap<usize, usize> = TARGETS.lock().unwrap().iter().map(|t| (t.res, t.order)).collect();
    let name = |r: usize| match r {
        0 => "-".to_string(),
        usize::MAX => "?".to_string(),
        r => order.get(&r).map(|o| format!("#{o:03}")).unwrap_or_else(|| format!("{r:#x}")),
    };
    let sdir = dir.join("shaders");
    let _ = std::fs::create_dir_all(&sdir);
    let mut out = String::new();
    let mut shader = |code: usize| -> String {
        if code == 0 {
            return "-".into();
        }
        match crate::shader_xlat::origin(code) {
            Some((hash, blob)) => {
                let p = sdir.join(format!("{hash:016x}.dxbc"));
                if !p.exists() {
                    let _ = std::fs::write(p, &blob[..]);
                }
                format!("{hash:016x}")
            }
            None => "unknown".into(),
        }
    };
    for (i, ((pso, rtv, dsv, kind), count)) in draws.iter().enumerate() {
        let k = ["draw", "drawidx", "dispatch", "indirect"][(*kind as usize).min(3)];
        if *kind == 2 {
            let cs = compute.get(pso).copied().unwrap_or(0);
            out.push_str(&format!("{i:04} {k} x{count} cs={}\n", shader(cs)));
            continue;
        }
        let rts: Vec<String> = rtv.iter().map(|r| name(*r)).collect();
        let (vs, ps, state) = match crate::device_wrap::pso_summary(*pso) {
            Some((v, p, s)) => (shader(v), shader(p), s),
            None => ("?".into(), "?".into(), String::new()),
        };
        out.push_str(&format!("{i:04} {k} x{count} rt=[{}] ds={} vs={vs} ps={ps} {state}\n", rts.join(","), name(*dsv)));
    }
    let _ = std::fs::write(dir.join("draws.txt"), out);
}

/// Called at every PresentX after `maybe_capture`'s own work: writes the trace when a
/// capture was just taken into `dir`, and starts a fresh frame.
pub fn end_frame(captured_into: Option<&std::path::Path>) {
    if !enabled() {
        return;
    }
    let (draws, compute) = take_draws();
    unsafe { crate::bindtrace::end_frame() };
    let list: Vec<u64> = std::fs::read_to_string(std::env::temp_dir().join("tombox_skip_shaders.txt"))
        .map(|s| s.split_whitespace().filter_map(|w| u64::from_str_radix(w, 16).ok()).collect())
        .unwrap_or_default();
    let tl: Vec<u64> = std::fs::read_to_string(std::env::temp_dir().join("tombox_trace_shaders.txt"))
        .map(|s| s.split_whitespace().filter_map(|w| u64::from_str_radix(w, 16).ok()).collect())
        .unwrap_or_default();
    with_trace(|t| {
        if t.skip_hashes != list {
            t.skip_hashes = list;
            t.skip_pso.clear();
        }
        if t.trace_hashes != tl {
            t.trace_hashes = tl;
            t.trace_pso.clear();
            t.trace_budget = 14;
        }
    });
    if let Some(dir) = captured_into {
        write_draws(dir, &draws, &compute);
    }
}
