use compat_common::record;
use core::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

pub fn period() -> u64 {
    static P: OnceLock<u64> = OnceLock::new();
    *P.get_or_init(|| parse_period(std::env::var("TOMBOX_PRESENT_PROBE").ok().as_deref()))
}

pub fn parse_period(v: Option<&str>) -> u64 {
    match v.map(str::trim) {
        None | Some("") | Some("0") => 0,
        Some("1") => 30,
        Some(s) => s.parse::<u64>().unwrap_or(30).max(1),
    }
}

pub fn bpp(format: u32) -> Option<usize> {
    match format {
        24 | 25 | 26 | 27 | 28 | 29 | 87 | 88 | 90 | 91 => Some(4),
        10 => Some(8),
        _ => None,
    }
}

pub fn pitch(width: u32, bpp: usize) -> usize {
    (width as usize * bpp).div_ceil(256) * 256
}

fn half(h: u16) -> f32 {
    let s = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let e = ((h >> 10) & 0x1f) as i32;
    let m = (h & 0x3ff) as f32;
    match e {
        0 => s * m * (2.0f32).powi(-24),
        31 => if m == 0.0 { s * f32::INFINITY } else { f32::NAN },
        _ => s * (1.0 + m / 1024.0) * (2.0f32).powi(e - 15),
    }
}

fn uf(bits: u32, mant: u32) -> f32 {
    let e = (bits >> mant) & 0x1f;
    let m = (bits & ((1 << mant) - 1)) as f32 / (1u32 << mant) as f32;
    match e {
        0 => m * (2.0f32).powi(-14),
        31 => if m == 0.0 { f32::INFINITY } else { f32::NAN },
        _ => (1.0 + m) * (2.0f32).powi(e as i32 - 15),
    }
}

pub fn decode(px: &[u8], format: u32) -> [f32; 4] {
    let u = |i: usize| px[i] as f32 / 255.0;
    match format {
        87 | 88 | 90 | 91 => [u(2), u(1), u(0), if format == 88 { 1.0 } else { u(3) }],
        24 | 25 => {
            let v = u32::from_le_bytes([px[0], px[1], px[2], px[3]]);
            let c = |s: u32| ((v >> s) & 0x3ff) as f32 / 1023.0;
            [c(0), c(10), c(20), (v >> 30) as f32 / 3.0]
        }
        26 => {
            let v = u32::from_le_bytes([px[0], px[1], px[2], px[3]]);
            [uf(v & 0x7ff, 6), uf((v >> 11) & 0x7ff, 6), uf(v >> 22, 5), 1.0]
        }
        10 => {
            let h = |i: usize| half(u16::from_le_bytes([px[i], px[i + 1]]));
            [h(0), h(2), h(4), h(6)]
        }
        _ => [u(0), u(1), u(2), u(3)],
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stats {
    pub samples: u64,
    pub mean: [f32; 4],
    pub max: [f32; 3],
    pub nonblack: u64,
    pub nonfinite: u64,
    pub uniform: bool,
}

pub fn stats(data: &[u8], pitch: usize, width: usize, height: usize, format: u32, step: usize) -> Option<Stats> {
    let b = bpp(format)?;
    let step = step.max(1);
    let mut sum = [0f64; 4];
    let mut max = [0f32; 3];
    let (mut n, mut nonblack, mut nonfinite) = (0u64, 0u64, 0u64);
    let mut first: Option<&[u8]> = None;
    let mut uniform = true;
    for y in (0..height).step_by(step) {
        let row = data.get(y * pitch..y * pitch + width * b)?;
        for x in (0..width).step_by(step) {
            let p = &row[x * b..x * b + b];
            match first {
                None => first = Some(p),
                Some(f) => uniform &= f == p,
            }
            let c = decode(p, format);
            n += 1;
            if c.iter().any(|v| !v.is_finite()) {
                nonfinite += 1;
                continue;
            }
            for i in 0..4 {
                sum[i] += c[i] as f64;
            }
            for i in 0..3 {
                max[i] = max[i].max(c[i]);
            }
            if c[..3].iter().any(|v| *v >= 0.5 / 255.0) {
                nonblack += 1;
            }
        }
    }
    let f = (n - nonfinite).max(1) as f64;
    Some(Stats { samples: n, mean: sum.map(|s| (s / f) as f32), max, nonblack, nonfinite, uniform })
}

pub fn stats_line(s: &Stats) -> String {
    format!(
        "avg rgba=({:.4},{:.4},{:.4},{:.4}) max rgb=({:.3},{:.3},{:.3}) non-black {:.2}% of {} samples{}{}",
        s.mean[0], s.mean[1], s.mean[2], s.mean[3], s.max[0], s.max[1], s.max[2],
        100.0 * s.nonblack as f64 / s.samples.max(1) as f64, s.samples,
        if s.nonfinite != 0 { format!(", {} NaN/Inf", s.nonfinite) } else { String::new() },
        if s.uniform { ", uniform" } else { "" })
}

pub struct Draw {
    pub rtv: Vec<usize>,
    pub dsv: usize,
    pub kind: u8,
    pub count: u32,
}

pub fn draw_summary(draws: &[Draw], presented: usize, name: &dyn Fn(usize) -> String) -> String {
    let mut gfx = 0u64;
    let mut dispatch = 0u64;
    let mut to_presented = 0u64;
    let mut no_rt = 0u64;
    let mut depth_only = 0u64;
    let mut per: Vec<(usize, u64)> = Vec::new();
    for d in draws {
        let c = d.count as u64;
        if d.kind == 2 {
            dispatch += c;
            continue;
        }
        gfx += c;
        if d.rtv.contains(&presented) {
            to_presented += c;
        }
        let rts: Vec<usize> = d.rtv.iter().copied().filter(|r| *r != 0).collect();
        if rts.is_empty() {
            no_rt += c;
            if d.dsv != 0 {
                depth_only += c;
            }
        }
        for r in rts {
            match per.iter_mut().find(|e| e.0 == r) {
                Some(e) => e.1 += c,
                None => per.push((r, c)),
            }
        }
    }
    per.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let top: Vec<String> = per.iter().take(6).map(|(r, c)| format!("{}{} x{c}", if *r == presented { "*" } else { "" }, name(*r))).collect();
    format!(
        "draws={gfx} dispatches={dispatch} draws_into_presented={to_presented} draws_without_rt={no_rt} depth_only={depth_only} rt_targets={} top=[{}]",
        per.len(), top.join("; "))
}

pub fn pc_topology(t: u32) -> bool {
    matches!(t, 0..=5 | 10..=13 | 33..=64)
}

static TOPO_ODD: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
static TOPO_VAL: [AtomicU64; 4] = [const { AtomicU64::new(u64::MAX) }; 4];

pub fn note_topology(t: u32) {
    if pc_topology(t) {
        return;
    }
    for i in 0..4 {
        let v = TOPO_VAL[i].load(Ordering::Relaxed);
        if v == t as u64 {
            TOPO_ODD[i].fetch_add(1, Ordering::Relaxed);
            return;
        }
        if v == u64::MAX && TOPO_VAL[i].compare_exchange(u64::MAX, t as u64, Ordering::Relaxed, Ordering::Relaxed).is_ok() {
            TOPO_ODD[i].fetch_add(1, Ordering::Relaxed);
            record("d3d12_xs", "PresentX.probe", &format!("title set primitive topology {t}, which is not a PC D3D12 topology; it is forwarded unchanged"));
            return;
        }
    }
}

fn topology_report() -> String {
    let v: Vec<String> = (0..4)
        .filter_map(|i| {
            let t = TOPO_VAL[i].load(Ordering::Relaxed);
            (t != u64::MAX).then(|| format!("{t}:{}", TOPO_ODD[i].swap(0, Ordering::Relaxed)))
        })
        .collect();
    if v.is_empty() { String::new() } else { format!(" non-PC topology sets=[{}]", v.join(" ")) }
}

struct Pending {
    rb: usize,
    frame: u64,
    src: usize,
    format: u32,
    width: u32,
    height: u32,
    meta: String,
}

struct Probe {
    rb: usize,
    size: usize,
    pending: Option<Pending>,
    meta: Option<String>,
    failed: bool,
}

unsafe impl Send for Probe {}

static PROBE: Mutex<Probe> = Mutex::new(Probe { rb: 0, size: 0, pending: None, meta: None, failed: false });
static PRESENTS: AtomicU64 = AtomicU64::new(0);

fn guard() -> std::sync::MutexGuard<'static, Probe> {
    PROBE.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn due(n: u64, period: u64) -> bool {
    period != 0 && n != 0 && n % period == 0
}

pub unsafe fn on_present(src: *mut c_void, plane_count: u32, planes: *const u8) {
    let p = period();
    if p == 0 {
        return;
    }
    let n = PRESENTS.fetch_add(1, Ordering::Relaxed) + 1;
    if !due(n, p) {
        return;
    }
    let mut meta = format!("planes={plane_count}");
    if !planes.is_null() {
        let q: Vec<String> = (0..(plane_count as usize * 4).clamp(4, 16))
            .map(|i| format!("{:x}", unsafe { core::ptr::read_unaligned((planes as *const u64).add(i)) }))
            .collect();
        meta.push_str(&format!(" raw=[{}]", q.join(" ")));
    }
    meta.push_str(&format!(" src={src:p} {}", crate::rtcap::label(src as usize)));
    meta.push_str(&topology_report());
    meta.push_str(&format!(" unbound-heap tables={}", crate::title_heap::UNBOUND_TABLES.swap(0, Ordering::Relaxed)));
    if crate::rtcap::enabled() {
        let draws = crate::rtcap::frame_draws();
        meta.push_str(&format!(" | {}", draw_summary(&draws, src as usize, &|r| crate::rtcap::label(r))));
    } else {
        meta.push_str(" | draw attribution needs TOMBOX_RTCAP=1");
    }
    guard().meta = Some(meta);
}

unsafe fn readback(device: *mut c_void, size: usize) -> Option<usize> {
    let heap: [u32; 5] = [3, 0, 0, 1, 1];
    let mut bd = [0u8; 0x38];
    bd[0..4].copy_from_slice(&1u32.to_le_bytes());
    bd[0x10..0x18].copy_from_slice(&(size as u64).to_le_bytes());
    bd[0x18..0x1c].copy_from_slice(&1u32.to_le_bytes());
    bd[0x1c..0x1e].copy_from_slice(&1u16.to_le_bytes());
    bd[0x1e..0x20].copy_from_slice(&1u16.to_le_bytes());
    bd[0x24..0x28].copy_from_slice(&1u32.to_le_bytes());
    bd[0x2c..0x30].copy_from_slice(&1u32.to_le_bytes());
    let vt = unsafe { *(device as *const *const usize) };
    let f: unsafe extern "system" fn(*mut c_void, *const u32, u32, *const u8, u32, *const c_void, *const crate::alias::Guid, *mut *mut c_void) -> i32 =
        unsafe { core::mem::transmute(*vt.add(27)) };
    let mut out: *mut c_void = core::ptr::null_mut();
    let hr = unsafe { f(device, heap.as_ptr(), 0, bd.as_ptr(), 0x400, core::ptr::null(), &crate::alias::IID_RESOURCE, &mut out) };
    if hr < 0 || out.is_null() {
        record("d3d12_xs", "PresentX.probe", &format!("readback buffer {size:#x} hr={hr:#010x}; probe off"));
        return None;
    }
    Some(out as usize)
}

pub unsafe fn record_copy(queue: *mut c_void, list: *mut c_void, src: *mut c_void, format: u32, width: u32, height: u32, frame: u64) {
    let mut g = guard();
    let Some(meta) = g.meta.take() else { return };
    if g.failed || g.pending.is_some() {
        return;
    }
    let Some(b) = bpp(format) else {
        record("d3d12_xs", "PresentX.probe", &format!("frame {frame}: format {format} not decoded | {meta}"));
        return;
    };
    let size = pitch(width, b) * height as usize;
    if g.rb == 0 || g.size < size {
        let mut device: *mut c_void = core::ptr::null_mut();
        let qvt = unsafe { *(queue as *const *const usize) };
        let get: unsafe extern "system" fn(*mut c_void, *const crate::alias::Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(*qvt.add(7)) };
        if unsafe { get(queue, &crate::alias::IID_DEVICE, &mut device) } < 0 || device.is_null() {
            g.failed = true;
            return;
        }
        let rb = unsafe { readback(device, size) };
        let dvt = unsafe { *(device as *const *const usize) };
        let release: unsafe extern "system" fn(*mut c_void) -> u32 = unsafe { core::mem::transmute(*dvt.add(2)) };
        unsafe { release(device) };
        match rb {
            Some(r) => {
                if g.rb != 0 {
                    let rvt = unsafe { *(g.rb as *const *const usize) };
                    let rel: unsafe extern "system" fn(*mut c_void) -> u32 = unsafe { core::mem::transmute(*rvt.add(2)) };
                    unsafe { rel(g.rb as *mut c_void) };
                }
                g.rb = r;
                g.size = size;
            }
            None => {
                g.failed = true;
                return;
            }
        }
    }
    let mut dst = [0u8; 0x30];
    dst[0..8].copy_from_slice(&(g.rb as u64).to_le_bytes());
    dst[8..12].copy_from_slice(&1u32.to_le_bytes());
    dst[0x18..0x1c].copy_from_slice(&format.to_le_bytes());
    dst[0x1c..0x20].copy_from_slice(&width.to_le_bytes());
    dst[0x20..0x24].copy_from_slice(&height.to_le_bytes());
    dst[0x24..0x28].copy_from_slice(&1u32.to_le_bytes());
    dst[0x28..0x2c].copy_from_slice(&(pitch(width, b) as u32).to_le_bytes());
    let s = crate::present_snap::subresource_location(src);
    let lvt = unsafe { *(list as *const *const usize) };
    let copy: unsafe extern "system" fn(*mut c_void, *const u8, u32, u32, u32, *const u8, *const c_void) = unsafe { core::mem::transmute(*lvt.add(16)) };
    unsafe { copy(list, dst.as_ptr(), 0, 0, 0, s.as_ptr(), core::ptr::null()) };
    let rb = g.rb;
    g.pending = Some(Pending { rb, frame, src: src as usize, format, width, height, meta });
}

pub unsafe fn collect() {
    let Some(p) = guard().pending.take() else { return };
    let Some(b) = bpp(p.format) else { return };
    let pt = pitch(p.width, b);
    let len = pt * p.height as usize;
    let vt = unsafe { *(p.rb as *const *const usize) };
    let map: unsafe extern "system" fn(*mut c_void, u32, *const c_void, *mut *mut u8) -> i32 = unsafe { core::mem::transmute(*vt.add(8)) };
    let unmap: unsafe extern "system" fn(*mut c_void, u32, *const c_void) = unsafe { core::mem::transmute(*vt.add(9)) };
    let mut ptr: *mut u8 = core::ptr::null_mut();
    if unsafe { map(p.rb as *mut c_void, 0, core::ptr::null(), &mut ptr) } < 0 || ptr.is_null() {
        record("d3d12_xs", "PresentX.probe", &format!("frame {}: readback map failed | {}", p.frame, p.meta));
        return;
    }
    let data = unsafe { core::slice::from_raw_parts(ptr, len) };
    let line = match stats(data, pt, p.width as usize, p.height as usize, p.format, 4) {
        Some(s) => stats_line(&s),
        None => "stats unavailable".to_string(),
    };
    let range = [0usize; 2];
    unsafe { unmap(p.rb as *mut c_void, 0, range.as_ptr() as *const c_void) };
    record("d3d12_xs", "PresentX.probe", &format!("frame {} {}x{} fmt={} src={:#x}: {line} | {}", p.frame, p.width, p.height, p.format, p.src, p.meta));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn period_parsing() {
        assert_eq!(parse_period(None), 0);
        assert_eq!(parse_period(Some("0")), 0);
        assert_eq!(parse_period(Some("1")), 30);
        assert_eq!(parse_period(Some("5")), 5);
        assert_eq!(parse_period(Some("x")), 30);
        assert!(due(30, 30) && !due(31, 30) && !due(0, 30) && !due(30, 0));
    }

    #[test]
    fn decodes_present_formats() {
        assert_eq!(decode(&[255, 0, 51, 255], 28), [1.0, 0.0, 0.2, 1.0]);
        assert_eq!(decode(&[255, 0, 51, 255], 87), [0.2, 0.0, 1.0, 1.0]);
        let v: u32 = 1023 | (3 << 30);
        assert_eq!(decode(&v.to_le_bytes(), 24), [1.0, 0.0, 0.0, 1.0]);
        let h = [0x00u8, 0x3c, 0x00, 0x38, 0x00, 0x00, 0x00, 0x3c];
        assert_eq!(decode(&h, 10), [1.0, 0.5, 0.0, 1.0]);
        let r11 = (0x3c0u32) | (0x3c0 << 11) | (0x1e0 << 22);
        let c = decode(&r11.to_le_bytes(), 26);
        assert!((c[0] - 1.0).abs() < 1e-6 && (c[1] - 1.0).abs() < 1e-6 && (c[2] - 1.0).abs() < 1e-6);
        assert!(decode(&[0, 0x7e, 0, 0, 0, 0, 0, 0], 10)[0].is_nan());
        assert_eq!(bpp(28), Some(4));
        assert_eq!(bpp(10), Some(8));
        assert_eq!(bpp(2), None);
        assert_eq!(pitch(1920, 4), 7680);
        assert_eq!(pitch(1920, 8), 15360);
        assert_eq!(pitch(3, 4), 256);
    }

    #[test]
    fn stats_report_black_and_colour() {
        let pt = 256;
        let black = vec![0u8; pt * 4];
        let s = stats(&black, pt, 4, 4, 28, 1).unwrap();
        assert_eq!(s.samples, 16);
        assert_eq!(s.nonblack, 0);
        assert!(s.uniform);
        assert_eq!(s.mean, [0.0; 4]);
        let mut d = black.clone();
        d[pt + 4..pt + 8].copy_from_slice(&[255, 128, 0, 255]);
        let s = stats(&d, pt, 4, 4, 28, 1).unwrap();
        assert_eq!(s.nonblack, 1);
        assert!(!s.uniform);
        assert!((s.mean[0] - 1.0 / 16.0).abs() < 1e-6);
        assert_eq!(s.max[0], 1.0);
        let s2 = stats(&d, pt, 4, 4, 28, 2).unwrap();
        assert_eq!(s2.samples, 4);
        assert_eq!(s2.nonblack, 0);
        let mut f = vec![0u8; pt];
        f[0..8].copy_from_slice(&[0, 0x7c, 0, 0, 0, 0, 0, 0x3c]);
        let s3 = stats(&f, pt, 2, 1, 10, 1).unwrap();
        assert_eq!(s3.nonfinite, 1);
        assert!(stats_line(&s3).contains("1 NaN/Inf"));
        assert!(stats(&d, pt, 4, 8, 28, 1).is_none());
        assert!(stats(&d, pt, 4, 4, 2, 1).is_none());
        assert!(stats_line(&s).contains("non-black 6.25% of 16 samples"));
    }

    #[test]
    fn pc_topologies() {
        for t in [1, 2, 3, 4, 5, 10, 13, 33, 64] {
            assert!(pc_topology(t), "{t}");
        }
        for t in [6, 9, 14, 19, 32, 65] {
            assert!(!pc_topology(t), "{t}");
        }
        assert!(crate::title_heap::table_heap_bound(&[0x10, 0x20], 0x20));
        assert!(!crate::title_heap::table_heap_bound(&[], 0x20));
        assert!(!crate::title_heap::table_heap_bound(&[0x10], 0x20));
    }

    #[test]
    fn draw_summary_attributes_targets() {
        let draws = vec![
            Draw { rtv: vec![0x10, 0x20], dsv: 0x30, kind: 1, count: 40 },
            Draw { rtv: vec![0x20], dsv: 0, kind: 0, count: 5 },
            Draw { rtv: vec![], dsv: 0x30, kind: 1, count: 7 },
            Draw { rtv: vec![], dsv: 0, kind: 2, count: 3 },
            Draw { rtv: vec![0x99], dsv: 0, kind: 0, count: 2 },
        ];
        let name = |r: usize| format!("r{r:x}");
        let s = draw_summary(&draws, 0x99, &name);
        assert!(s.starts_with("draws=54 dispatches=3 draws_into_presented=2 draws_without_rt=7 depth_only=7 rt_targets=3"), "{s}");
        assert!(s.contains("top=[r20 x45; r10 x40; *r99 x2]"), "{s}");
        let none = draw_summary(&[], 1, &name);
        assert!(none.contains("draws=0") && none.contains("top=[]"));
    }
}
