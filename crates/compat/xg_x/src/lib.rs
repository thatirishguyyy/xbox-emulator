mod computer;
mod dcc;
mod dcc_codec;
mod descriptor;
mod jpeg;
pub mod layout;
mod tileshape;

use compat_common::{record, S_OK};
use computer::E_INVALIDARG;
use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicUsize, Ordering};
use layout::{compute, Desc};

const M: &str = "xg_x";
pub const XG_TILE_MODE_INVALID: u32 = 0xffff_ffff;
pub const XG_TILE_MODE_LINEAR_GENERAL: u32 = 0x1f;
const E_FAIL: i32 = 0x8000_4005u32 as i32;
const BIND_RENDER_TARGET: u32 = 0x20;
const BIND_DEPTH_STENCIL: u32 = 0x40;
const MISC_DEPTH_HTILE_TC: u32 = 0x8000;
const MISC_64KB_SWIZZLE: u32 = 0x0800_0000;

pub(crate) static HARDWARE_FLAG: AtomicBool = AtomicBool::new(false);

fn log_limited(counter: &AtomicU32, n: u32, name: &str, msg: &str) {
    if counter.fetch_add(1, Ordering::Relaxed) < n {
        record(M, name, msg);
    }
}

fn has_stencil(dxgi: u32) -> bool {
    matches!(dxgi, 19 | 20 | 22 | 44 | 45 | 47 | 62 | 118 | 119 | 120)
}

fn depth_samples_mode(samples: u32) -> u32 {
    match samples {
        2 | 4 => 1,
        8 => 2,
        _ => 0,
    }
}

fn depth_mode(format: u32, samples: u32, tc_compatible: bool, htile: bool) -> u32 {
    if tc_compatible || htile {
        if has_stencil(format) {
            record(M, "XGComputeOptimalDepthStencilTileModes", &format!("format {format} has stencil; compressed depth layout requested -> XG_TILE_MODE_INVALID"));
            return XG_TILE_MODE_INVALID;
        }
        return if tc_compatible { 7 } else { 4 };
    }
    depth_samples_mode(samples)
}

#[no_mangle]
pub extern "system" fn XGComputeOptimalTileMode(dimension: u32, format: u32, width: u32, height: u32, array: u32, samples: u32, bind: u32, misc: u32) -> u32 {
    let mode = if dimension == 1 {
        XG_TILE_MODE_LINEAR_GENERAL
    } else if bind & BIND_DEPTH_STENCIL != 0 {
        depth_mode(format, samples, misc & MISC_64KB_SWIZZLE != 0, misc & MISC_DEPTH_HTILE_TC != 0)
    } else {
        let rt = (bind & BIND_RENDER_TARGET != 0) as u32;
        match dimension {
            2 => layout::MODE_LINEAR_ALIGNED,
            3 => 13 + rt + if misc & MISC_64KB_SWIZZLE != 0 { 3 } else { 0 },
            4 => 13 + rt,
            _ => XG_TILE_MODE_LINEAR_GENERAL,
        }
    };
    static LOG: AtomicU32 = AtomicU32::new(0);
    log_limited(&LOG, 16, "XGComputeOptimalTileMode", &format!("dim={dimension} fmt={format} {width}x{height} array={array} samples={samples} bind={bind:#x} misc={misc:#x} -> {mode}"));
    mode
}

#[no_mangle]
pub extern "system" fn XGComputeOptimalDepthStencilTileModes(
    format: u32,
    _width: u32,
    _height: u32,
    _array: u32,
    samples: u32,
    _reserved: u32,
    tc_compatible: i32,
    htile: i32,
    depth: *mut u32,
    stencil: *mut u32,
) -> u32 {
    let mode = depth_mode(format, samples, tc_compatible != 0, htile != 0);
    unsafe {
        if !depth.is_null() {
            *depth = mode;
        }
        if !stencil.is_null() {
            *stencil = mode;
        }
    }
    mode
}

fn depth_format_of(format: u32) -> u32 {
    match format {
        39 | 41 => 40,
        53 | 56 => 55,
        19 | 21 | 22 => 20,
        44 | 46 | 47 => 45,
        119 | 120 => 118,
        f => f,
    }
}

#[no_mangle]
pub extern "system" fn XGValidateDepthStencilTileMode(format: u32, stencil_mode: u32, depth_mode: u32) -> i32 {
    if depth_mode > 7 {
        return E_INVALIDARG;
    }
    let f = depth_format_of(format);
    if !has_stencil(f) {
        return S_OK;
    }
    let wide_depth = match f {
        20 | 45 => true,
        118 => false,
        _ => return E_INVALIDARG,
    };
    if stencil_mode == 0 {
        return if depth_mode >= 6 { S_OK } else { E_INVALIDARG };
    }
    if stencil_mode >= 17 {
        return S_OK;
    }
    let bad: u32 = match stencil_mode {
        1 => 0xde,
        2 => 0xdc,
        3 | 4 if !HARDWARE_FLAG.load(Ordering::Relaxed) || !wide_depth => 0x80,
        _ if HARDWARE_FLAG.load(Ordering::Relaxed) => 0x90,
        _ => 0x80,
    };
    if bad & (1 << depth_mode) != 0 { E_FAIL } else { S_OK }
}

#[no_mangle]
pub extern "system" fn XGSetHardwareVersion(version: i32) {
    HARDWARE_FLAG.store(version >= 2, Ordering::Relaxed);
    record(M, "XGSetHardwareVersion", &format!("{version} -> revision flag {}", version >= 2));
}

#[repr(C)]
struct CriticalSection([usize; 5]);

extern "system" {
    fn InitializeCriticalSection(cs: *mut CriticalSection);
    fn EnterCriticalSection(cs: *mut CriticalSection);
    fn LeaveCriticalSection(cs: *mut CriticalSection);
}

static LOCK: AtomicUsize = AtomicUsize::new(0);
static LOCKING: AtomicI32 = AtomicI32::new(0);

#[no_mangle]
pub extern "system" fn XGEnableExclusiveLocking(enable: i32) {
    if LOCK.load(Ordering::Acquire) == 0 {
        let cs = Box::into_raw(Box::new(CriticalSection([0; 5])));
        unsafe { InitializeCriticalSection(cs) };
        if LOCK.compare_exchange(0, cs as usize, Ordering::AcqRel, Ordering::Acquire).is_err() {
            drop(unsafe { Box::from_raw(cs) });
        }
    }
    LOCKING.store(enable, Ordering::Release);
}

#[no_mangle]
pub extern "system" fn XGEnterExclusiveLock() -> i32 {
    let cs = LOCK.load(Ordering::Acquire);
    if LOCKING.load(Ordering::Acquire) == 0 || cs == 0 {
        return 0;
    }
    unsafe { EnterCriticalSection(cs as *mut CriticalSection) };
    1
}

#[no_mangle]
pub extern "system" fn XGLeaveExclusiveLock() {
    let cs = LOCK.load(Ordering::Acquire);
    if cs != 0 {
        unsafe { LeaveCriticalSection(cs as *mut CriticalSection) };
    }
}

#[no_mangle]
pub extern "system" fn XGValidateJpegFile(data: *const u8, size: u32, out: *mut u32) -> i32 {
    if out.is_null() || data.is_null() || size == 0 {
        return E_INVALIDARG;
    }
    let o = unsafe { core::slice::from_raw_parts_mut(out, 4) };
    o.fill(0);
    if size > 0x3f_ffff {
        return E_INVALIDARG;
    }
    jpeg::validate(unsafe { core::slice::from_raw_parts(data, size as usize) }, o)
}

fn rd(p: *const u8, off: usize) -> u32 {
    unsafe { (p.add(off) as *const u32).read_unaligned() }
}

fn finish(name: &str, d: Option<Desc>, pp: *mut *mut c_void) -> i32 {
    finish_with(name, d, pp, compute)
}

fn finish_with(name: &str, d: Option<Desc>, pp: *mut *mut c_void, lay: fn(&Desc) -> Option<layout::Layout>) -> i32 {
    static LOG: AtomicU32 = AtomicU32::new(0);
    let Some(d) = d else {
        log_limited(&LOG, 24, name, "descriptor rejected by validation -> E_INVALIDARG");
        return E_INVALIDARG;
    };
    match lay(&d) {
        Some(l) => {
            log_limited(&LOG, 24, name, &format!("dim={} {}x{}x{} mips={} fmt={} mode={} -> size={:#x} align={:#x}", d.dimension, d.width, d.height, d.depth_or_array, l.mips.len(), d.dxgi, d.mode, l.size, l.align));
            unsafe { *pp = computer::create(l) };
            S_OK
        }
        None => {
            log_limited(&LOG, 24, name, &format!("dim={} {}x{}x{} mips={} fmt={} mode={} -> E_INVALIDARG", d.dimension, d.width, d.height, d.depth_or_array, d.mips, d.dxgi, d.mode));
            E_INVALIDARG
        }
    }
}

fn start(desc: *const u8, pp: *mut *mut c_void) -> bool {
    if !pp.is_null() {
        unsafe { *pp = core::ptr::null_mut() };
    }
    !desc.is_null() && !pp.is_null()
}

const MISC_REJECT_2D: u32 = 0xffc5_ed0a;
const MISC_REJECT_1D_3D: u32 = 0xffc5_6d0a;

fn tex1d(p: *const u8) -> Option<Desc> {
    if rd(p, 0x1c) & MISC_REJECT_1D_3D != 0 {
        return None;
    }
    Some(Desc { dimension: 2, width: rd(p, 0) as u64, height: 1, depth_or_array: rd(p, 8), mips: rd(p, 4), dxgi: rd(p, 0xc), samples: 1, mode: rd(p, 0x28), pitch: rd(p, 0x2c) })
}

fn tex2d(p: *const u8) -> Option<Desc> {
    if rd(p, 0x28) & MISC_REJECT_2D != 0 {
        return None;
    }
    Some(Desc { dimension: 3, width: rd(p, 0) as u64, height: rd(p, 4), depth_or_array: rd(p, 0xc), mips: rd(p, 8), dxgi: rd(p, 0x10), samples: rd(p, 0x14), mode: rd(p, 0x34), pitch: rd(p, 0x38) })
}

fn tex3d(p: *const u8) -> Option<Desc> {
    if rd(p, 0x20) & MISC_REJECT_1D_3D != 0 {
        return None;
    }
    Some(Desc { dimension: 4, width: rd(p, 0) as u64, height: rd(p, 4), depth_or_array: rd(p, 8), mips: rd(p, 0xc), dxgi: rd(p, 0x10), samples: 1, mode: rd(p, 0x2c), pitch: rd(p, 0x30) })
}

fn resource_desc(p: *const u8) -> Option<Desc> {
    let dimension = rd(p, 0);
    let width = unsafe { (p.add(0x10) as *const u64).read_unaligned() };
    let height = rd(p, 0x18);
    let da = rd(p, 0x1c) & 0xffff;
    let mips = rd(p, 0x1c) >> 16;
    let dxgi = rd(p, 0x20);
    let samples = rd(p, 0x24);
    let lay = rd(p, 0x2c);
    let flags = rd(p, 0x30);
    if !(2..=4).contains(&dimension) {
        return None;
    }
    let bind = if flags & 2 != 0 { BIND_DEPTH_STENCIL } else if flags & 1 != 0 { BIND_RENDER_TARGET } else { 8 };
    let mode = match lay {
        0 => XGComputeOptimalTileMode(dimension, dxgi, width as u32, height, da, samples, bind, 0),
        2 => XGComputeOptimalTileMode(dimension, dxgi, width as u32, height, da, samples, bind, MISC_64KB_SWIZZLE),
        0x100..=0x11f => lay - 0x100,
        _ => return None,
    };
    if (lay == 2 && dimension == 4) || (dimension == 3 && flags & 2 != 0 && dxgi == 62) {
        return None;
    }
    if (lay >= 0x100 || lay == 2) && dimension >= 3 && (mode == 6 || (matches!(mode, 7 | 16 | 17) && (dimension == 4 || layout::is_planar(dxgi)))) {
        return None;
    }
    if lay >= 0x100 && dimension == 3 && flags & 2 != 0 {
        let allowed = if has_stencil(dxgi) { matches!(mode, 0 | 5) } else { mode <= 5 || mode == 7 };
        if !allowed {
            return None;
        }
    }
    Some(Desc { dimension, width, height, depth_or_array: da, mips, dxgi, samples, mode, pitch: 0 })
}

#[no_mangle]
pub extern "system" fn XGCreateTexture1DComputer(desc: *const u8, pp: *mut *mut c_void) -> i32 {
    if !start(desc, pp) {
        return E_INVALIDARG;
    }
    finish("XGCreateTexture1DComputer", tex1d(desc), pp)
}

#[no_mangle]
pub extern "system" fn XGCreateTexture2DComputer(desc: *const u8, pp: *mut *mut c_void) -> i32 {
    if !start(desc, pp) {
        return E_INVALIDARG;
    }
    finish("XGCreateTexture2DComputer", tex2d(desc), pp)
}

#[no_mangle]
pub extern "system" fn XGCreateTexture3DComputer(desc: *const u8, pp: *mut *mut c_void) -> i32 {
    if !start(desc, pp) {
        return E_INVALIDARG;
    }
    finish("XGCreateTexture3DComputer", tex3d(desc), pp)
}

#[no_mangle]
pub extern "system" fn XGCreateTextureComputerWithPitch(desc: *const u8, pitch: u32, pp: *mut *mut c_void) -> i32 {
    if !start(desc, pp) {
        return E_INVALIDARG;
    }
    let Some(mut d) = resource_desc(desc) else {
        record(M, "XGCreateTextureComputer", &format!("unsupported XG_RESOURCE_DESC dimension={} layout={:#x} -> E_INVALIDARG", rd(desc, 0), rd(desc, 0x2c)));
        return E_INVALIDARG;
    };
    d.pitch = pitch;
    if rd(desc, 0x30) & 2 != 0 && d.samples > 1 && layout::is_depth_stencil_planar(d.dxgi) {
        return finish_with("XGCreateTextureComputer", Some(d), pp, layout::compute_depth_msaa);
    }
    finish("XGCreateTextureComputer", Some(d), pp)
}

#[no_mangle]
pub extern "system" fn XGCreateTextureComputer(desc: *const u8, pp: *mut *mut c_void) -> i32 {
    XGCreateTextureComputerWithPitch(desc, 0, pp)
}

fn layout_into(d: Option<Desc>, out: *mut u8) -> i32 {
    if out.is_null() {
        return E_INVALIDARG;
    }
    match d.and_then(|d| compute(&d)) {
        Some(l) => {
            layout::write_layout(&l, unsafe { core::slice::from_raw_parts_mut(out, layout::LAYOUT_BYTES) });
            S_OK
        }
        None => E_INVALIDARG,
    }
}

#[no_mangle]
pub extern "system" fn XGComputeTexture1DLayout(desc: *const u8, out: *mut u8) -> i32 {
    layout_into(if desc.is_null() { None } else { tex1d(desc) }, out)
}

#[no_mangle]
pub extern "system" fn XGComputeTexture2DLayout(desc: *const u8, out: *mut u8) -> i32 {
    layout_into(if desc.is_null() { None } else { tex2d(desc) }, out)
}

#[no_mangle]
pub extern "system" fn XGComputeTexture3DLayout(desc: *const u8, out: *mut u8) -> i32 {
    layout_into(if desc.is_null() { None } else { tex3d(desc) }, out)
}

#[no_mangle]
pub extern "system" fn XGComputeBufferLayout(desc: *const u8, out: *mut u8) -> i32 {
    if desc.is_null() || out.is_null() {
        return E_INVALIDARG;
    }
    let bind = rd(desc, 8);
    let misc = rd(desc, 0x10);
    if bind & 0xffff_f900 != 0 {
        record(M, "XGComputeBufferLayout", "Invalid bind flags.");
        return E_INVALIDARG;
    }
    if misc & 0xf385_6d0a != 0 {
        record(M, "XGComputeBufferLayout", "Invalid misc flags.");
        return E_INVALIDARG;
    }
    if misc & 0x400_0000 != 0 && rd(desc, 0) & 0xffff != 0 {
        record(M, "XGComputeBufferLayout", "ByteWidth of tile pool buffers must be a multiple of 64 KB.");
        return E_INVALIDARG;
    }
    let size = rd(desc, 0) as u64;
    let align: u64 = if misc & 0xc00_0000 != 0 { 0x10000 } else { 4 };
    let buf = unsafe { core::slice::from_raw_parts_mut(out, layout::LAYOUT_BYTES) };
    buf.fill(0);
    buf[0..8].copy_from_slice(&size.to_le_bytes());
    buf[8..16].copy_from_slice(&align.to_le_bytes());
    buf[0x10..0x18].copy_from_slice(&1u64.to_le_bytes());
    buf[0x1738..0x173c].copy_from_slice(&1u32.to_le_bytes());
    S_OK
}

#[cfg(test)]
mod layout_oracle_tests;
#[cfg(test)]
pub(crate) mod oracle;
#[cfg(test)]
mod tests;
