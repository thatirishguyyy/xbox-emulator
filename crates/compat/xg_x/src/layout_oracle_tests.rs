use super::*;
use core::ffi::c_void;

fn oracle() -> crate::oracle::Oracle {
    crate::oracle::oracle().expect("console xg_x.dll and xmem.dll are needed for the oracle tests")
}

type Layout = extern "system" fn(*const u8, *mut u8) -> i32;
type Create = extern "system" fn(*const u8, *mut *mut c_void) -> i32;

const FILL: u8 = 0xcd;

fn words(v: &[u32]) -> Vec<u8> {
    let mut b: Vec<u8> = v.iter().flat_map(|w| w.to_le_bytes()).collect();
    b.resize(0x60, 0);
    b
}

fn desc1d(w: u32, mips: u32, arr: u32, fmt: u32, mode: u32) -> Vec<u8> {
    words(&[w, mips, arr, fmt, 0, 8, 0, 0, 0, 0, mode, 0])
}

fn desc2d(w: u32, h: u32, mips: u32, arr: u32, fmt: u32, mode: u32) -> Vec<u8> {
    words(&[w, h, mips, arr, fmt, 1, 0, 0, 8, 0, 0, 0, 0, mode, 0])
}

fn desc3d(w: u32, h: u32, d: u32, mips: u32, fmt: u32, mode: u32) -> Vec<u8> {
    words(&[w, h, d, mips, fmt, 0, 8, 0, 0, 0, 0, mode, 0])
}

fn both_layouts(theirs: Layout, ours: Layout, d: &[u8]) -> (i32, i32, Vec<u8>, Vec<u8>) {
    let mut a = vec![FILL; layout::LAYOUT_BYTES];
    let mut b = vec![FILL; layout::LAYOUT_BYTES];
    let ha = theirs(d.as_ptr(), a.as_mut_ptr());
    let hb = ours(d.as_ptr(), b.as_mut_ptr());
    (ha, hb, a, b)
}

fn assert_same_layout(name: &str, theirs: Layout, ours: Layout, d: &[u8]) -> bool {
    let (ha, hb, a, b) = both_layouts(theirs, ours, d);
    assert_eq!(ha < 0, hb < 0, "{name} hr console={ha:#x} ours={hb:#x} desc={:x?}", &d[..0x34]);
    if ha < 0 {
        return false;
    }
    let diff = a.iter().zip(&b).position(|(x, y)| x != y);
    assert_eq!(diff, None, "{name} desc={:x?} first difference at {:#x}: console={:x?} ours={:x?}", &d[..0x34], diff.unwrap_or(0), diff.map(|i| &a[i & !7..((i & !7) + 16).min(a.len())]), diff.map(|i| &b[i & !7..((i & !7) + 16).min(b.len())]));
    true
}

const FMTS: [u32; 9] = [28, 61, 2, 10, 71, 77, 87, 24, 56];

#[test]
fn texture1d_layout_matches_console() {
    let o = oracle();
    let theirs: Layout = unsafe { core::mem::transmute(o.f("XGComputeTexture1DLayout")) };
    let mut ok = 0;
    for w in [1, 7, 64, 100, 256, 1000, 4096] {
        for (mips, arr) in [(0, 1), (1, 1), (3, 2)] {
            for fmt in FMTS {
                for mode in [8, 13, 0] {
                    ok += assert_same_layout("1D", theirs, XGComputeTexture1DLayout, &desc1d(w, mips, arr, fmt, mode)) as u32;
                }
            }
        }
    }
    assert!(ok > 50, "only {ok} valid 1D descs");
}

#[test]
fn texture2d_layout_export_matches_console() {
    let o = oracle();
    let theirs: Layout = unsafe { core::mem::transmute(o.f("XGComputeTexture2DLayout")) };
    let mut ok = 0;
    for (w, h) in [(1, 1), (64, 64), (100, 37), (1920, 1080), (513, 7)] {
        for (mips, arr) in [(0, 1), (1, 1), (2, 6)] {
            for fmt in FMTS {
                for mode in [8, 13, 14, 31, 0, 2, 4, 5, 7, 9, 10, 16, 17, 20, 24, 27, 32] {
                    ok += assert_same_layout("2D", theirs, XGComputeTexture2DLayout, &desc2d(w, h, mips, arr, fmt, mode)) as u32;
                }
            }
        }
    }
    assert!(ok > 100, "only {ok} valid 2D descs");
}

#[test]
fn texture3d_layout_matches_console() {
    let o = oracle();
    let theirs: Layout = unsafe { core::mem::transmute(o.f("XGComputeTexture3DLayout")) };
    let mut ok = 0;
    for (w, h, d) in [(1, 1, 1), (16, 16, 16), (100, 37, 5), (256, 128, 64), (33, 65, 3)] {
        for mips in [0, 1, 3] {
            for fmt in FMTS {
                for mode in [8, 13, 31, 0] {
                    ok += assert_same_layout("3D", theirs, XGComputeTexture3DLayout, &desc3d(w, h, d, mips, fmt, mode)) as u32;
                }
            }
        }
    }
    assert!(ok > 50, "only {ok} valid 3D descs");
}

#[test]
fn texture2d_planar_zero_and_hw2_layouts_match_console() {
    let o = oracle();
    let theirs: Layout = unsafe { core::mem::transmute(o.f("XGComputeTexture2DLayout")) };
    let mut ok = 0;
    for version in [0, 2] {
        o.hardware_version(version);
        for (w, h) in [(1, 1), (100, 37), (300, 70), (1920, 1080)] {
            for (mips, arr) in [(0, 1), (1, 1), (3, 2)] {
                for fmt in [0, 19, 20, 44, 45, 103, 104, 105, 106, 110, 118, 120, 28, 71, 98] {
                    for mode in [8, 13, 14, 0, 10, 17, 31] {
                        ok += assert_same_layout("2D planar/hw", theirs, XGComputeTexture2DLayout, &desc2d(w, h, mips, arr, fmt, mode)) as u32;
                    }
                }
            }
        }
    }
    o.hardware_version(0);
    assert!(ok > 200, "only {ok} valid descs");
}

#[test]
fn layout_exports_reject_null_like_console() {
    let o = oracle();
    let d = desc2d(64, 64, 1, 1, 28, 13);
    let mut out = vec![0u8; layout::LAYOUT_BYTES];
    for (name, ours) in [("XGComputeTexture1DLayout", XGComputeTexture1DLayout as Layout), ("XGComputeTexture2DLayout", XGComputeTexture2DLayout), ("XGComputeTexture3DLayout", XGComputeTexture3DLayout), ("XGComputeBufferLayout", XGComputeBufferLayout)] {
        let theirs: Layout = unsafe { core::mem::transmute(o.f(name)) };
        assert_eq!(ours(core::ptr::null(), out.as_mut_ptr()), theirs(core::ptr::null(), out.as_mut_ptr()), "{name} null desc");
        assert_eq!(ours(d.as_ptr(), core::ptr::null_mut()), theirs(d.as_ptr(), core::ptr::null_mut()), "{name} null out");
    }
}

#[test]
fn buffer_layout_matches_console() {
    let o = oracle();
    let theirs: Layout = unsafe { core::mem::transmute(o.f("XGComputeBufferLayout")) };
    let mut ok = 0;
    for size in [0u32, 1, 4, 255, 256, 65536, 0x2_0000, 0x7fff_ffff] {
        for bind in [0u32, 1, 8, 0x80, 0x100, 0x200, 0x400, 0x800] {
            for misc in [0u32, 2, 8, 0x80, 0x400_0000, 0x800_0000, 0xc00_0000] {
                let d = words(&[size, 0, bind, 0, misc, 0, 0, 0]);
                ok += assert_same_layout("buffer", theirs, XGComputeBufferLayout, &d) as u32;
            }
        }
    }
    assert!(ok > 100, "only {ok} valid buffer descs");
}

fn computer_layout(c: *mut c_void) -> Vec<u8> {
    let mut v = vec![FILL; layout::LAYOUT_BYTES];
    unsafe {
        let f: extern "system" fn(*mut c_void, *mut u8) -> i32 = core::mem::transmute(*(*(c as *const *const usize)).add(2));
        assert_eq!(f(c, v.as_mut_ptr()), 0);
    }
    v
}

fn release(c: *mut c_void) {
    unsafe {
        let f: extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(*(*(c as *const *const usize)).add(1));
        f(c);
    }
}

fn assert_same_computer(name: &str, theirs: Create, ours: Create, d: &[u8]) -> bool {
    let (mut t, mut u): (*mut c_void, *mut c_void) = (1 as *mut c_void, 1 as *mut c_void);
    let (ht, hu) = (theirs(d.as_ptr(), &mut t), ours(d.as_ptr(), &mut u));
    assert_eq!(ht < 0, hu < 0, "{name} hr console={ht:#x} ours={hu:#x}");
    if ht < 0 {
        assert_eq!(t.is_null(), u.is_null(), "{name} out pointer on failure");
        return false;
    }
    let (a, b) = (computer_layout(t), computer_layout(u));
    release(t);
    release(u);
    let diff = a.iter().zip(&b).position(|(x, y)| x != y);
    assert_eq!(diff, None, "{name} desc={:x?} first difference at {:#x}", &d[..0x34], diff.unwrap_or(0));
    true
}

#[test]
fn texture1d_computer_matches_console() {
    let o = oracle();
    let theirs: Create = unsafe { core::mem::transmute(o.f("XGCreateTexture1DComputer")) };
    let mut ok = 0;
    for w in [1, 64, 100, 4096] {
        for (mips, arr) in [(0, 1), (2, 3)] {
            for fmt in FMTS {
                for mode in [8, 13] {
                    ok += assert_same_computer("1D", theirs, XGCreateTexture1DComputer, &desc1d(w, mips, arr, fmt, mode)) as u32;
                }
            }
        }
    }
    assert!(ok > 20);
    let mut p = 1 as *mut c_void;
    assert_eq!(XGCreateTexture1DComputer(core::ptr::null(), &mut p), theirs(core::ptr::null(), &mut 1 as *mut _ as *mut *mut c_void));
    assert!(p.is_null());
}

#[test]
fn texture3d_computer_matches_console() {
    let o = oracle();
    let theirs: Create = unsafe { core::mem::transmute(o.f("XGCreateTexture3DComputer")) };
    let mut ok = 0;
    for (w, h, d) in [(16, 16, 16), (100, 37, 5), (256, 128, 64)] {
        for mips in [0, 2] {
            for fmt in FMTS {
                for mode in [8, 13, 31] {
                    ok += assert_same_computer("3D", theirs, XGCreateTexture3DComputer, &desc3d(w, h, d, mips, fmt, mode)) as u32;
                }
            }
        }
    }
    assert!(ok > 20);
}

fn resource_desc_bytes(w: u64, h: u32, fmt: u32, lay: u32) -> Vec<u8> {
    let mut d = vec![0u8; 0x60];
    d[0..4].copy_from_slice(&3u32.to_le_bytes());
    d[0x10..0x18].copy_from_slice(&w.to_le_bytes());
    d[0x18..0x1c].copy_from_slice(&h.to_le_bytes());
    d[0x1c..0x1e].copy_from_slice(&1u16.to_le_bytes());
    d[0x1e..0x20].copy_from_slice(&1u16.to_le_bytes());
    d[0x20..0x24].copy_from_slice(&fmt.to_le_bytes());
    d[0x24..0x28].copy_from_slice(&1u32.to_le_bytes());
    d[0x2c..0x30].copy_from_slice(&lay.to_le_bytes());
    d
}

#[test]
fn texture_computer_with_pitch_matches_console() {
    let o = oracle();
    let theirs: extern "system" fn(*const u8, u32, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(o.f("XGCreateTextureComputerWithPitch")) };
    for (w, h, fmt) in [(100u64, 37u32, 28u32), (640, 360, 87), (64, 64, 61), (300, 70, 10)] {
        for lay in [0u32, 0x108, 0x10d, 0x100] {
            for pitch in [0u32, 1, 64, 256, 512, 1024, 4096, 0x10000] {
                let d = resource_desc_bytes(w, h, fmt, lay);
                let (mut t, mut u): (*mut c_void, *mut c_void) = (core::ptr::null_mut(), core::ptr::null_mut());
                let (ht, hu) = (theirs(d.as_ptr(), pitch, &mut t), XGCreateTextureComputerWithPitch(d.as_ptr(), pitch, &mut u));
                assert_eq!(ht < 0, hu < 0, "w={w} h={h} fmt={fmt} lay={lay:#x} pitch={pitch} console={ht:#x} ours={hu:#x}");
                if ht < 0 {
                    continue;
                }
                let (a, b) = (computer_layout(t), computer_layout(u));
                release(t);
                release(u);
                let diff = a.iter().zip(&b).position(|(x, y)| x != y);
                assert_eq!(diff, None, "w={w} h={h} fmt={fmt} lay={lay:#x} pitch={pitch} first difference at {:#x}", diff.unwrap_or(0));
            }
        }
    }
}

#[test]
fn texture_computer_with_pitch_chains_and_planes_match_console() {
    let o = oracle();
    let theirs: extern "system" fn(*const u8, u32, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(o.f("XGCreateTextureComputerWithPitch")) };
    let mut ok = 0;
    for (dim, w, h, da) in [(2u32, 300u64, 1u32, 2u16), (3, 7, 1, 1), (3, 100, 37, 1), (3, 64, 360, 2), (4, 100, 37, 5)] {
        for mips in [0u16, 1, 3] {
            for fmt in [28u32, 71, 98, 61, 0, 20, 44, 103, 104, 105] {
                for lay in [0u32, 0x100, 0x105, 0x106, 0x107, 0x108, 0x10a, 0x10d, 0x10e, 0x110, 0x111, 0x11f] {
                    for pitch in [0u32, 1, 64, 65, 129, 256, 300, 16385, 20000] {
                        let mut d = resource_desc_bytes(w, h, fmt, lay);
                        d[0..4].copy_from_slice(&dim.to_le_bytes());
                        d[0x1c..0x1e].copy_from_slice(&da.to_le_bytes());
                        d[0x1e..0x20].copy_from_slice(&mips.to_le_bytes());
                        let (mut t, mut u): (*mut c_void, *mut c_void) = (core::ptr::null_mut(), core::ptr::null_mut());
                        let (ht, hu) = (theirs(d.as_ptr(), pitch, &mut t), XGCreateTextureComputerWithPitch(d.as_ptr(), pitch, &mut u));
                        let tag = format!("dim={dim} {w}x{h}x{da} mips={mips} fmt={fmt} lay={lay:#x} pitch={pitch}");
                        assert_eq!(ht < 0, hu < 0, "{tag} console={ht:#x} ours={hu:#x}");
                        if ht < 0 {
                            continue;
                        }
                        let (a, b) = (computer_layout(t), computer_layout(u));
                        release(t);
                        release(u);
                        let diff = a.iter().zip(&b).position(|(x, y)| x != y);
                        assert_eq!(diff, None, "{tag} first difference at {:#x}", diff.unwrap_or(0));
                        ok += 1;
                    }
                }
            }
        }
    }
    assert!(ok > 2000, "only {ok} valid descs");
}

#[test]
fn texture2d_display_msaa_hw2_matches_console() {
    let o = oracle();
    let theirs: Layout = unsafe { core::mem::transmute(o.f("XGComputeTexture2DLayout")) };
    let mut ok = 0;
    for version in [0, 2] {
        o.hardware_version(version);
        for fmt in [2u32, 10, 24, 28, 61, 77, 83, 98] {
            for samples in [1u32, 2, 4, 8] {
                for (w, h) in [(64, 64), (100, 37), (300, 70), (1920, 1080)] {
                    for mode in [9, 10, 11, 12, 14] {
                        let mut d = desc2d(w, h, 1, 1, fmt, mode);
                        d[0x14..0x18].copy_from_slice(&samples.to_le_bytes());
                        ok += assert_same_layout("2D display msaa", theirs, XGComputeTexture2DLayout, &d) as u32;
                    }
                }
            }
        }
    }
    o.hardware_version(0);
    assert!(ok > 400, "only {ok} valid descs");
}

#[test]
fn texture2d_depth_stencil_msaa_matches_console() {
    let o = oracle();
    let theirs: Layout = unsafe { core::mem::transmute(o.f("XGComputeTexture2DLayout")) };
    let mut ok = 0;
    for version in [0, 2] {
        o.hardware_version(version);
        for fmt in [19u32, 20, 21, 22, 44, 45, 46, 47] {
            for samples in [1u32, 2, 4, 8] {
                for (w, h) in [(64, 64), (100, 37), (300, 70), (1920, 1080)] {
                    for mode in [0, 1, 2, 3, 4, 5, 6, 7] {
                        let mut d = desc2d(w, h, 1, 1, fmt, mode);
                        d[0x14..0x18].copy_from_slice(&samples.to_le_bytes());
                        ok += assert_same_layout("2D depth msaa", theirs, XGComputeTexture2DLayout, &d) as u32;
                    }
                }
            }
        }
    }
    o.hardware_version(0);
    assert!(ok > 100, "only {ok} valid descs");
}

fn depth_desc_bytes(w: u64, h: u32, fmt: u32, samples: u32, lay: u32, flags: u32) -> Vec<u8> {
    let mut d = resource_desc_bytes(w, h, fmt, lay);
    d[0x24..0x28].copy_from_slice(&samples.to_le_bytes());
    d[0x30..0x34].copy_from_slice(&flags.to_le_bytes());
    d
}

fn plane_bytes(l: &[u8], i: usize) -> (u64, u64, u32, Vec<u8>) {
    let b = 0x20 + i * layout::PLANE_PITCH;
    let mut mip = l[b + 0x20..b + 0x20 + layout::MIP_STRIDE].to_vec();
    mip[8..16].fill(0);
    (u64::from_le_bytes(l[b..b + 8].try_into().unwrap()), u64::from_le_bytes(l[b + 0x10..b + 0x18].try_into().unwrap()), u32::from_le_bytes(l[b + 0x18..b + 0x1c].try_into().unwrap()), mip)
}

fn plane_offset(l: &[u8], i: usize) -> u64 {
    let b = 0x20 + i * layout::PLANE_PITCH;
    u64::from_le_bytes(l[b + 8..b + 0x10].try_into().unwrap())
}

#[test]
fn depth_bind_planar_msaa_planes_match_console() {
    let o = oracle();
    let theirs: Create = unsafe { core::mem::transmute(o.f("XGCreateTextureComputer")) };
    let (mut checked, mut rejected) = (0, 0);
    for version in [0, 2] {
        o.hardware_version(version);
        for fmt in [19u32, 20, 22, 44, 45, 47, 118, 120] {
            for samples in [2u32, 4, 8] {
                for (w, h) in [(64u64, 64u32), (100, 37), (1280, 720), (1600, 900), (1920, 1080), (3840, 2160)] {
                    for arr in [1u16, 6] {
                        let name = format!("fmt {fmt} {samples}x {w}x{h} a{arr} hw{version}");
                        let mut d = depth_desc_bytes(w, h, fmt, samples, 0, 2);
                        d[0x1c..0x1e].copy_from_slice(&arr.to_le_bytes());
                        let (mut t, mut u): (*mut c_void, *mut c_void) = (core::ptr::null_mut(), core::ptr::null_mut());
                        let (ht, hu) = (theirs(d.as_ptr(), &mut t), XGCreateTextureComputer(d.as_ptr(), &mut u));
                        if matches!(fmt, 22 | 47 | 120) {
                            assert!(hu < 0 && u.is_null(), "{name} ours={hu:#x}: the stencil-only views have no depth plane row to lay out");
                            if ht >= 0 {
                                release(t);
                            }
                            rejected += 1;
                            continue;
                        }
                        assert_eq!(ht < 0, hu < 0, "{name} console={ht:#x} ours={hu:#x}");
                        if ht < 0 {
                            rejected += 1;
                            continue;
                        }
                        let (a, b) = (computer_layout(t), computer_layout(u));
                        release(t);
                        release(u);
                        let ri = |v: &[u8], i: usize| u32::from_le_bytes(v[i..i + 4].try_into().unwrap());
                        assert_eq!((ri(&a, 0x14), ri(&a, 0x18), ri(&b, 0x14)), (3, 4, 2), "{name} plane counts");
                        let htile = plane_bytes(&a, 0).0;
                        let shift = htile.div_ceil(plane_bytes(&b, 0).1) * plane_bytes(&b, 0).1;
                        for (cp, op) in [(1, 0), (2, 1)] {
                            assert_eq!(plane_bytes(&a, cp), plane_bytes(&b, op), "{name} plane {op}");
                            assert_eq!(plane_offset(&a, cp), plane_offset(&b, op) + shift, "{name} plane {op} offset after the console HTILE plane");
                        }
                        let q = |v: &[u8], i: usize| u64::from_le_bytes(v[i..i + 8].try_into().unwrap());
                        assert_eq!(q(&a, 0), q(&b, 0) + shift, "{name} total");
                        assert_eq!(q(&a, 8), q(&b, 8), "{name} align");
                        checked += 1;
                    }
                }
            }
        }
    }
    o.hardware_version(0);
    eprintln!("checked={checked} rejected={rejected}");
    assert!(checked >= 288, "checked={checked} rejected={rejected}");
}

#[test]
fn planar_msaa_rejections_match_console() {
    let o = oracle();
    let theirs: Create = unsafe { core::mem::transmute(o.f("XGCreateTextureComputer")) };
    for fmt in [19u32, 20, 44, 45] {
        for samples in [2u32, 4, 8] {
            let full = {
                let mut d = depth_desc_bytes(1920, 1080, fmt, samples, 0, 2);
                d[0x1e..0x20].copy_from_slice(&0u16.to_le_bytes());
                d
            };
            for d in [depth_desc_bytes(1920, 1080, fmt, samples, 0, 0), depth_desc_bytes(1920, 1080, fmt, samples, 0, 1), full] {
                let (mut t, mut u): (*mut c_void, *mut c_void) = (core::ptr::null_mut(), core::ptr::null_mut());
                let (ht, hu) = (theirs(d.as_ptr(), &mut t), XGCreateTextureComputer(d.as_ptr(), &mut u));
                assert!(ht < 0 && hu < 0 && u.is_null(), "fmt {fmt} {samples}x console={ht:#x} ours={hu:#x} desc={:x?}", &d[..0x34]);
            }
        }
    }
}

#[test]
fn compute_depth_msaa_only_lays_out_single_mip_planar_depth() {
    let d = |dxgi: u32, samples: u32, mips: u32| Desc { dimension: 3, width: 1920, height: 1080, depth_or_array: 1, mips, dxgi, samples, mode: 1, pitch: 0 };
    let l = layout::compute_depth_msaa(&d(19, 2, 1)).expect("2x fmt 19");
    let planes: Vec<&layout::Plane> = l.planes.iter().flatten().collect();
    assert_eq!(planes.iter().map(|p| (p.bpe, p.offset)).collect::<Vec<_>>(), vec![(4, 0), (1, planes[0].size.div_ceil(planes[1].align) * planes[1].align)]);
    assert_eq!(l.size, planes[1].offset + planes[1].size);
    assert!(planes.iter().all(|p| p.mips.len() == 1 && p.mips[0].samples == 2));
    for bad in [d(19, 1, 1), d(19, 2, 0), d(19, 2, 2), d(40, 4, 1), d(28, 4, 1)] {
        assert_eq!(layout::compute_depth_msaa(&bad), None, "{bad:?}");
    }
    assert_eq!(compute(&d(19, 2, 1)), None);
}
