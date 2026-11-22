use super::*;
use core::ffi::c_void;

const EXPORTS: &str = include_str!("../exports.txt");

use crate::oracle::{oracle, serial};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn pick<T: Copy>(&mut self, v: &[T]) -> T {
        v[(self.next() % v.len() as u64) as usize]
    }
}

#[test]
fn export_table_matches_reference_dll() {
    xone_exports::check_crate(env!("CARGO_MANIFEST_DIR"), "xg_x.dll", EXPORTS);
}

#[test]
fn built_dll_has_reference_ordinals() {
    xone_exports::check_built(env!("CARGO_MANIFEST_DIR"), "xg_x.dll", EXPORTS);
}

#[test]
fn optimal_tile_modes_match_console() {
    let Some(o) = oracle() else { return };
    let f: extern "system" fn(u32, u32, u32, u32, u32, u32, u32, u32) -> u32 = unsafe { core::mem::transmute(o.f("XGComputeOptimalTileMode")) };
    let mut n = 0;
    for dim in 0..7 {
        for fmt in (0..133).chain([200, 500]) {
            for bind in [0, 8, 0x20, 0x40, 0x60, 0x80] {
                for misc in [0, 4, 0x8000, 0x800_0000, 0x800_8000] {
                    for s in [0, 1, 2, 3, 4, 8, 16] {
                        assert_eq!(XGComputeOptimalTileMode(dim, fmt, 64, 64, 1, s, bind, misc), f(dim, fmt, 64, 64, 1, s, bind, misc), "dim={dim} fmt={fmt} bind={bind:#x} misc={misc:#x} s={s}");
                        n += 1;
                    }
                }
            }
        }
    }
    assert!(n > 10000);
}

#[test]
fn depth_stencil_modes_and_validation_match_console() {
    let Some(o) = oracle() else { return };
    type Ds = extern "system" fn(u32, u32, u32, u32, u32, u32, i32, i32, *mut u32, *mut u32) -> u32;
    let ds: Ds = unsafe { core::mem::transmute(o.f("XGComputeOptimalDepthStencilTileModes")) };
    let v: extern "system" fn(u32, u32, u32) -> i32 = unsafe { core::mem::transmute(o.f("XGValidateDepthStencilTileMode")) };
    let hv: extern "system" fn(i32) = unsafe { core::mem::transmute(o.f("XGSetHardwareVersion")) };
    for fmt in 0..140 {
        for s in [0, 1, 2, 4, 8, 16] {
            for (tc, ht) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let (mut a, mut b, mut c, mut d) = (9u32, 9u32, 9u32, 9u32);
                let r1 = ds(fmt, 64, 64, 1, s, 0, tc, ht, &mut a, &mut b);
                let r2 = XGComputeOptimalDepthStencilTileModes(fmt, 64, 64, 1, s, 0, tc, ht, &mut c, &mut d);
                assert_eq!((r1, a, b), (r2, c, d), "fmt={fmt} s={s} tc={tc} ht={ht}");
            }
        }
    }
    for version in [0, 1, 2, 3, -1] {
        hv(version);
        XGSetHardwareVersion(version);
        for fmt in 0..140 {
            for stencil in 1..32 {
                for depth in 0..10 {
                    assert_eq!(XGValidateDepthStencilTileMode(fmt, stencil, depth), v(fmt, stencil, depth), "hv={version} fmt={fmt} s={stencil} d={depth}");
                }
            }
            for depth in [6, 7, 8, 31] {
                assert_eq!(XGValidateDepthStencilTileMode(fmt, 0, depth), v(fmt, 0, depth), "hv={version} fmt={fmt} s=0 d={depth}");
            }
        }
    }
    hv(0);
    XGSetHardwareVersion(0);
}

fn desc2d(w: u32, h: u32, mips: u32, arr: u32, fmt: u32, mode: u32) -> [u32; 15] {
    [w, h, mips, arr, fmt, 1, 0, 0, 8, 0, 0, 0, 0, mode, 0]
}

#[test]
fn texture2d_layouts_match_console() {
    let Some(o) = oracle() else { return };
    let create: extern "system" fn(*const u8, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(o.f("XGCreateTexture2DComputer")) };
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let sizes = [1, 2, 3, 4, 7, 8, 16, 24, 31, 32, 33, 63, 64, 65, 100, 127, 128, 129, 200, 256, 500, 512, 1000, 1024, 1080, 1920, 2048, 4096];
    let fmts = [61, 56, 28, 10, 2, 71, 77, 83, 87, 24, 41, 49, 98];
    let mut checked = 0;
    for _ in 0..1500 {
        let d = desc2d(rng.pick(&sizes), rng.pick(&sizes), rng.pick(&[0, 1, 1, 2, 3]), rng.pick(&[1, 1, 2, 3, 6]), rng.pick(&fmts), rng.pick(&[8, 13, 14, 31]));
        let mut theirs: *mut c_void = core::ptr::null_mut();
        let hr_t = create(d.as_ptr() as *const u8, &mut theirs);
        let mut ours: *mut c_void = core::ptr::null_mut();
        let hr_o = XGCreateTexture2DComputer(d.as_ptr() as *const u8, &mut ours);
        assert_eq!(hr_t < 0, hr_o < 0, "{d:?}");
        if hr_t < 0 {
            continue;
        }
        let mut a = vec![0u8; layout::LAYOUT_BYTES];
        let mut b = vec![0u8; layout::LAYOUT_BYTES];
        unsafe {
            let call = |c: *mut c_void, out: *mut u8| {
                let f: extern "system" fn(*mut c_void, *mut u8) -> i32 = core::mem::transmute(*(*(c as *const *const usize)).add(2));
                f(c, out)
            };
            call(theirs, a.as_mut_ptr());
            call(ours, b.as_mut_ptr());
        }
        let mips = u32::from_le_bytes(a[0x10..0x14].try_into().unwrap()) as usize;
        let n = 0x40 + mips * layout::MIP_STRIDE;
        assert!(n <= a.len());
        let diff = a.iter().zip(&b).position(|(x, y)| x != y);
        assert_eq!(diff, None, "{d:?} n={n:#x} theirs={:x?} ours={:x?}", diff.map(|i| &a[i & !7..(i & !7) + 16]), diff.map(|i| &b[i & !7..(i & !7) + 16]));
        for c in [theirs, ours] {
            unsafe {
                let f: extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(*(*(c as *const *const usize)).add(1));
                f(c);
            }
        }
        checked += 1;
    }
    assert!(checked > 1000);
}

#[test]
fn micro_tiled_and_linear_texel_addresses_match_console() {
    let Some(o) = oracle() else { return };
    let create: extern "system" fn(*const u8, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(o.f("XGCreateTexture2DComputer")) };
    for (fmt, mode, w, h, mips, arr) in [(28, 13, 100, 37, 3, 2), (61, 13, 64, 64, 0, 1), (2, 13, 33, 17, 1, 3), (28, 31, 100, 37, 1, 1), (56, 8, 65, 9, 2, 1), (71, 13, 256, 128, 0, 1)] {
        let d = desc2d(w, h, mips, arr, fmt, mode);
        let (mut t, mut u): (*mut c_void, *mut c_void) = (core::ptr::null_mut(), core::ptr::null_mut());
        assert_eq!(create(d.as_ptr() as *const u8, &mut t), 0);
        assert_eq!(XGCreateTexture2DComputer(d.as_ptr() as *const u8, &mut u), 0);
        type Off = extern "system" fn(*mut c_void, u32, u32, u64, u32, u32, u32, *mut u32) -> u64;
        let ft: Off = unsafe { core::mem::transmute(*(*(t as *const *const usize)).add(6)) };
        let fu: Off = unsafe { core::mem::transmute(*(*(u as *const *const usize)).add(6)) };
        let l = layout::compute(&tex2d(d.as_ptr() as *const u8).unwrap()).unwrap();
        for (mi, m) in l.mips.iter().enumerate() {
            for z in 0..m.padded_d.min(2) {
                for y in 0..m.height {
                    for x in 0..m.width as u64 {
                        let (mut b1, mut b2) = (0, 0);
                        assert_eq!(fu(u, 0, mi as u32, x, y, z, 0, &mut b2), ft(t, 0, mi as u32, x, y, z, 0, &mut b1), "fmt={fmt} mode={mode} mip={mi} ({x},{y},{z})");
                    }
                }
            }
        }
    }
}

#[test]
fn copy_round_trips_through_tiled_memory() {
    let _g = serial();
    let d = desc2d(300, 70, 0, 2, 28, 14);
    let mut c: *mut c_void = core::ptr::null_mut();
    assert_eq!(XGCreateTexture2DComputer(d.as_ptr() as *const u8, &mut c), 0);
    let l = layout::compute(&tex2d(d.as_ptr() as *const u8).unwrap()).unwrap();
    let mut res = vec![0u8; l.size as usize];
    type Cp = extern "system" fn(*mut c_void, *mut u8, u32, u32, *const u8, u32, u32) -> i32;
    let into: Cp = unsafe { core::mem::transmute(*(*(c as *const *const usize)).add(8)) };
    let from: Cp = unsafe { core::mem::transmute(*(*(c as *const *const usize)).add(9)) };
    let mips = l.mips.len() as u32;
    for sub in 0..mips * 2 {
        let m = &l.mips[(sub % mips) as usize];
        let pitch = m.width * 4;
        let src: Vec<u8> = (0..(pitch * m.height) as usize).map(|i| (i as u32 * 7 + sub) as u8).collect();
        assert_eq!(into(c, res.as_mut_ptr(), 0, sub, src.as_ptr(), pitch, 0), 0);
        let mut back = vec![0u8; src.len()];
        assert_eq!(from(c, back.as_mut_ptr(), 0, sub, res.as_ptr(), pitch, 0), 0);
        assert_eq!(back, src, "sub {sub}");
    }
    assert_eq!(into(c, res.as_mut_ptr(), 0, mips * 2, res.as_ptr(), 4, 0), E_INVALIDARG);
}

#[test]
fn resource_desc_computer_matches_console() {
    let Some(o) = oracle() else { return };
    let create: extern "system" fn(*const u8, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(o.f("XGCreateTextureComputer")) };
    for (dim, w, h, da, mips, fmt, lay) in [(3u32, 256u64, 256u32, 1u16, 0u16, 28u32, 0u32), (3, 100, 37, 2, 3, 61, 0x10e), (3, 640, 360, 1, 1, 87, 0x10d), (2, 64, 1, 1, 1, 28, 0), (3, 64, 64, 1, 1, 28, 0x11f), (3, 64, 64, 1, 1, 28, 1), (1, 64, 1, 1, 1, 0, 0)] {
        let mut d = [0u8; 0x38];
        d[0..4].copy_from_slice(&dim.to_le_bytes());
        d[0x10..0x18].copy_from_slice(&w.to_le_bytes());
        d[0x18..0x1c].copy_from_slice(&h.to_le_bytes());
        d[0x1c..0x1e].copy_from_slice(&da.to_le_bytes());
        d[0x1e..0x20].copy_from_slice(&mips.to_le_bytes());
        d[0x20..0x24].copy_from_slice(&fmt.to_le_bytes());
        d[0x24..0x28].copy_from_slice(&1u32.to_le_bytes());
        d[0x2c..0x30].copy_from_slice(&lay.to_le_bytes());
        let (mut t, mut u): (*mut c_void, *mut c_void) = (core::ptr::null_mut(), core::ptr::null_mut());
        let (ht, hu) = (create(d.as_ptr(), &mut t), XGCreateTextureComputer(d.as_ptr(), &mut u));
        assert_eq!(ht < 0, hu < 0, "dim={dim} lay={lay:#x}");
        if ht < 0 {
            continue;
        }
        let (mut a, mut b) = (vec![0u8; layout::LAYOUT_BYTES], vec![0u8; layout::LAYOUT_BYTES]);
        unsafe {
            let f: extern "system" fn(*mut c_void, *mut u8) -> i32 = core::mem::transmute(*(*(t as *const *const usize)).add(2));
            f(t, a.as_mut_ptr());
            let f: extern "system" fn(*mut c_void, *mut u8) -> i32 = core::mem::transmute(*(*(u as *const *const usize)).add(2));
            f(u, b.as_mut_ptr());
        }
        let mipn = u32::from_le_bytes(a[0x10..0x14].try_into().unwrap()) as usize;
        assert!(0x40 + mipn * layout::MIP_STRIDE <= a.len());
        assert_eq!(a, b, "dim={dim} lay={lay:#x}");
    }
}

fn jpeg_stream(rng: &mut Rng) -> Vec<u8> {
    let mut v = vec![0xff, 0xd8];
    for _ in 0..rng.pick(&[1, 2, 3, 5]) {
        let marker: u16 = rng.pick(&[0xffc0, 0xffc0, 0xffc2, 0xffdb, 0xffc4, 0xffe0, 0xffe1, 0xffee, 0xfffe, 0xffda]);
        if marker == 0xffda {
            v.extend_from_slice(&[0xff, 0xda, 0, 12, rng.pick(&[1, 3]), 1, 0, 2, 0x11, 3, 0x11, 0, 0x3f, 0]);
            for _ in 0..rng.pick(&[0, 10, 40]) {
                v.push((rng.next() & 0x7f) as u8);
            }
            continue;
        }
        if marker == 0xffc0 {
            let (w, h): (u16, u16) = (rng.pick(&[1, 333, 8192, 8193, 16384]), rng.pick(&[1, 77, 8192, 9000]));
            let n = rng.pick(&[1u8, 3, 3, 4]);
            let mut seg = vec![rng.pick(&[8u8, 8, 12]), (h >> 8) as u8, h as u8, (w >> 8) as u8, w as u8, n];
            for c in 0..n {
                seg.extend_from_slice(&[c + 1, if c == 0 { rng.pick(&[0x11, 0x21, 0x22, 0x12]) } else { rng.pick(&[0x11, 0x11, 0x21]) }, 0]);
            }
            let len = seg.len() as u16 + 2;
            v.extend_from_slice(&marker.to_be_bytes());
            v.extend_from_slice(&len.to_be_bytes());
            v.extend_from_slice(&seg);
            continue;
        }
        let len = rng.pick(&[2u16, 6, 16, 30]);
        v.extend_from_slice(&marker.to_be_bytes());
        v.extend_from_slice(&len.to_be_bytes());
        v.extend(std::iter::repeat_n(0x55u8, len as usize - 2));
    }
    if rng.next() % 4 != 0 {
        v.extend_from_slice(&[0xff, 0xd9]);
    }
    if rng.next() % 5 == 0 {
        v.truncate(v.len() - (rng.next() % 6) as usize);
    }
    v
}

#[test]
fn jpeg_validation_matches_console() {
    let Some(o) = oracle() else { return };
    let f: extern "system" fn(*const u8, u32, *mut u32) -> i32 = unsafe { core::mem::transmute(o.f("XGValidateJpegFile")) };
    let mut rng = Rng(12345);
    let mut stalled = 0;
    for i in 0..4000 {
        let s = jpeg_stream(&mut rng);
        let mut w = [0u32; 4];
        if jpeg::walk(&s, &mut w).is_err() {
            stalled += 1;
            continue;
        }
        let mut padded = s.clone();
        padded.resize(s.len() + 64, 0);
        let (mut a, mut b) = ([7u32; 4], [7u32; 4]);
        let (ra, rb) = (f(padded.as_ptr(), s.len() as u32, a.as_mut_ptr()), XGValidateJpegFile(s.as_ptr(), s.len() as u32, b.as_mut_ptr()));
        assert_eq!((ra, a), (rb, b), "case {i}: {s:02x?}");
    }
    assert!(stalled > 0 && stalled < 400, "{stalled}");
    let mut out = [0u32; 4];
    assert_eq!(XGValidateJpegFile(core::ptr::null(), 4, out.as_mut_ptr()), E_INVALIDARG);
    assert_eq!(XGValidateJpegFile([0u8; 4].as_ptr(), 0, out.as_mut_ptr()), E_INVALIDARG);
}

#[test]
fn exclusive_lock_needs_enabling() {
    let _g = serial();
    XGEnableExclusiveLocking(0);
    assert_eq!(XGEnterExclusiveLock(), 0);
    XGEnableExclusiveLocking(1);
    assert_eq!(XGEnterExclusiveLock(), 1);
    assert_eq!(XGEnterExclusiveLock(), 1);
    XGLeaveExclusiveLock();
    XGLeaveExclusiveLock();
    XGEnableExclusiveLocking(0);
}

#[test]
fn layout_struct_size_and_dimension_match_console() {
    let Some(o) = oracle() else { return };
    let create: extern "system" fn(*const u8, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(o.f("XGCreateTexture2DComputer")) };
    let d = desc2d(256, 256, 0, 6, 28, 14);
    let mut t: *mut c_void = core::ptr::null_mut();
    let mut u: *mut c_void = core::ptr::null_mut();
    assert_eq!(create(d.as_ptr() as *const u8, &mut t), 0);
    assert_eq!(XGCreateTexture2DComputer(d.as_ptr() as *const u8, &mut u), 0);
    let mut a = vec![0xccu8; 0x4000];
    let mut b = vec![0xccu8; 0x4000];
    unsafe {
        let f: extern "system" fn(*mut c_void, *mut u8) -> i32 = core::mem::transmute(*(*(t as *const *const usize)).add(2));
        f(t, a.as_mut_ptr());
        let g: extern "system" fn(*mut c_void, *mut u8) -> i32 = core::mem::transmute(*(*(u as *const *const usize)).add(2));
        g(u, b.as_mut_ptr());
    }
    assert_eq!(a.iter().rposition(|&x| x != 0xcc), Some(layout::LAYOUT_BYTES - 1));
    assert_eq!(layout::LAYOUT_BYTES, 0x1740);
    assert_eq!(a, b);
    assert_eq!(u32::from_le_bytes(b[layout::DIMENSION_OFFSET..layout::DIMENSION_OFFSET + 4].try_into().unwrap()), 3);
}

#[test]
fn jpeg_truncated_scan_stalls_like_console() {
    let s = [0xffu8, 0xd8, 0xff, 0xda, 0, 12, 3, 1, 0, 2, 0x11, 3, 0x11, 0, 0x3f, 0];
    let mut w = [0u32; 4];
    assert_eq!(jpeg::walk(&s, &mut w), Err(jpeg::Halt::Stalled));
    let mut e = s;
    e[6] = 1;
    assert_eq!(jpeg::walk(&e, &mut w), Ok(0x8000_4005u32 as i32));
    let mut full = s.to_vec();
    full.extend_from_slice(&[0xff, 0xd9]);
    assert_eq!(jpeg::walk(&full, &mut w), Ok(0));
    assert_eq!(jpeg::walk(&[0xff, 0xd8, 0xff, 0xc0], &mut w), Err(jpeg::Halt::NullPayload));
    assert_eq!(jpeg::walk(&[0xff, 0xd8, 0xff, 0xda, 0], &mut w), Err(jpeg::Halt::NullPayload));
    assert_eq!(jpeg::walk(&[0xff], &mut w), Ok(0x8000_4005u32 as i32));
}
