use super::*;

const IID_DEV: Guid = Guid { d1: 0x1898_19f1, d2: 0x1db6, d3: 0x4b57, d4: [0xbe, 0x54, 0x18, 0x21, 0x33, 0x9b, 0x85, 0xf7] };
const IID_RS: Guid = Guid { d1: 0xc54a_6b66, d2: 0x72df, d3: 0x4ee8, d4: [0x8b, 0xe5, 0xa9, 0x46, 0xa1, 0x42, 0x92, 0x14] };
const IID_PSO: Guid = Guid { d1: 0x765a_30f3, d2: 0xf624, d3: 0x4c6f, d4: [0xa8, 0x28, 0xac, 0xe9, 0x48, 0x62, 0x24, 0x45] };

extern "system" {
    fn LoadLibraryA(n: *const u8) -> *mut c_void;
    fn GetProcAddress(h: *mut c_void, n: *const u8) -> *mut c_void;
}

fn put(d: &mut [u8], o: usize, v: u32) {
    d[o..o + 4].copy_from_slice(&v.to_le_bytes());
}

fn base_desc() -> Vec<u8> {
    let mut d = vec![0u8; GFX_DESC_SIZE];
    for i in 0..8 {
        let o = 0x80 + i * 40;
        for (k, v) in [1u32, 2, 1, 1, 2, 1, 4].iter().enumerate() {
            put(&mut d, o + 8 + 4 * k, *v);
        }
        d[o + 36] = 15;
    }
    put(&mut d, 0x1c0, 0xffff_ffff);
    put(&mut d, 0x1c4, 3);
    put(&mut d, 0x1c8, 1);
    put(&mut d, 0x1dc, 1);
    put(&mut d, DSS, 1);
    put(&mut d, DSS + 4, 1);
    put(&mut d, DSS + 8, 2);
    put(&mut d, DSS + 0xc, 1);
    d[DSS + 0x10] = 0xff;
    d[DSS + 0x11] = 0xff;
    for f in [0x14usize, 0x24] {
        for (k, v) in [1u32, 1, 1, 8].iter().enumerate() {
            put(&mut d, DSS + f + 4 * k, *v);
        }
    }
    put(&mut d, 0x23c, 3);
    put(&mut d, 0x264, 45);
    put(&mut d, 0x268, 1);
    d
}

fn fortnite_ext(back_read: u8, back_write: u8, bounds_off: u32) -> Vec<u8> {
    let mut e = vec![0u8; EXT_STRIDE];
    put(&mut e, 0, EXT_DEPTH_STENCIL);
    put(&mut e, 8, bounds_off);
    e[4 + 0x10] = back_read;
    e[4 + 0x11] = back_write;
    e[4 + 0x14] = 1;
    e[4 + 0x15] = 1;
    e
}

#[test]
fn ext_type1_decodes_like_db_depth_control_and_stencilrefmask_0x701cb() {
    let d = base_desc();
    let dflt = default_ext(&d);
    assert_eq!((dflt.depth_bounds_disable, dflt.back_read, dflt.back_write, dflt.front_op, dflt.back_op), (1, 0xff, 0xff, 1, 1));
    let (e, other) = parse_ext(&d, &fortnite_ext(0x0f, 0xf0, 0), 1);
    assert!(other.is_empty());
    assert_eq!((e.depth_bounds_disable, e.back_read, e.back_write, e.backface_disable), (0, 0x0f, 0xf0, 0));
    assert!(unmodelled(&d, &e).is_empty());
    let p = plan(&d, &e, true, true);
    assert!(p.depth_bounds && p.per_face);
    let p = plan(&d, &e, false, false);
    assert!(!p.depth_bounds && !p.per_face);
    let (same, _) = parse_ext(&d, &fortnite_ext(0xff, 0xff, 1), 1);
    let p = plan(&d, &same, true, true);
    assert!(!p.depth_bounds && !p.per_face);
    let mut two = fortnite_ext(1, 2, 1);
    let mut t3 = vec![0u8; EXT_STRIDE];
    put(&mut t3, 0, 3);
    two.extend(t3);
    assert_eq!(parse_ext(&d, &two, 2).1, vec![3]);
    let mut gap = e;
    gap.front_op = 0;
    gap.color_on_depth_fail = 1;
    assert_eq!(unmodelled(&d, &gap).len(), 2);
}

#[test]
fn depth_stencil2_puts_the_ext_back_masks_on_the_back_face() {
    let d = base_desc();
    let (e, _) = parse_ext(&d, &fortnite_ext(0x0f, 0xf0, 0), 1);
    let v = depth_stencil2(&d, &e, true);
    assert_eq!(v.len(), 0x3c);
    assert_eq!((v[0x20], v[0x21]), (0xff, 0xff));
    assert_eq!((v[0x34], v[0x35]), (0x0f, 0xf0));
    assert_eq!(u32::from_le_bytes(v[0x38..0x3c].try_into().unwrap()), 1);
    let mut off = e;
    off.backface_disable = 1;
    let v = depth_stencil2(&d, &off, false);
    assert_eq!((v[0x34], v[0x35]), (0xff, 0xff));
    let s = stream(&d, &e, &Plan { depth_bounds: true, per_face: false });
    assert_eq!(s.len() % 8, 0);
    assert_eq!(u32::from_le_bytes(s[0..4].try_into().unwrap()), 0);
}

#[test]
fn ext_types_0_2_3_and_compute_type0_decode_to_their_console_registers() {
    assert_eq!((Block::PsOptions.size(), Block::ShaderLimits.size(), Block::Tessellation.size(), Block::ComputeLimits.size()), (0x20, 0x3c, 0xc, 0x18));
    assert_eq!(Block::of_derived_tag(4), Block::of_ext(0));
    assert_eq!(Block::of_derived_tag(6), Block::of_ext(2));
    assert_eq!(Block::of_derived_tag(7), Block::of_ext(3));
    assert_eq!(Block::of_derived_tag(5), None);
    let mut lim = vec![0u8; 0x3c];
    put(&mut lim, 0, SHADER_LIMITS_DEFAULT_LATE_ALLOC);
    assert_eq!(Block::ShaderLimits.describe(&lim), None);
    put(&mut lim, 0x10, 0xf0);
    put(&mut lim, 0x24, 0x200);
    let d = Block::ShaderLimits.describe(&lim).unwrap();
    assert!(d.contains("PSDisableFlags=0xf0 (SPI_SHADER_PGM_RSRC3_PS.CU_EN)") && d.contains("PSMaxWaves=0x200 (SPI_SHADER_PGM_RSRC3_PS.WAVE_LIMIT)") && d.contains("rendered results are unaffected"), "{d}");
    let mut ps = vec![0u8; 0x20];
    assert_eq!(Block::PsOptions.describe(&ps), None);
    put(&mut ps, 4, 2);
    put(&mut ps, 0x1c, 3);
    let d = Block::PsOptions.non_default(&ps);
    assert_eq!(d.len(), 2);
    assert!(d[0].starts_with("ZOrder=0x2 (DB_SHADER_CONTROL.Z_ORDER; 2/4 = PS variant with [earlydepthstencil]") && d[1].starts_with("OutOfOrderRasterization=0x3 (PA_SC_MODE_CNTL_1.OUT_OF_ORDER_PRIMITIVE_ENABLE+WATER_MARK; no PC equivalent"), "{d:?}");
    let mut entries = vec![0u8; EXT_STRIDE * 4];
    put(&mut entries, 0, EXT_PS_OPTIONS);
    entries[8..12].copy_from_slice(&1u32.to_le_bytes());
    put(&mut entries, EXT_STRIDE, EXT_DEPTH_STENCIL);
    put(&mut entries, EXT_STRIDE * 2, EXT_SHADER_LIMITS);
    put(&mut entries, EXT_STRIDE * 2 + 4, SHADER_LIMITS_DEFAULT_LATE_ALLOC);
    put(&mut entries, EXT_STRIDE * 3, EXT_TESSELLATION);
    put(&mut entries, EXT_STRIDE * 3 + 4, 16);
    let v = describe_entries(&entries, 4);
    assert_eq!(v.len(), 2, "{v:?}");
    assert!(v[0].starts_with("D3D12XBOX_PIXEL_SHADER_OPTIONS_DESC: ZOrder=0x1"));
    assert!(v[1].starts_with("D3D12XBOX_PIPELINE_TESSELLATION_PARAMETERS_DESC: PatchesPerThreadgroup=0x10"));
    let mut c = vec![0u8; COMPUTE_EXT_STRIDE * 2];
    put(&mut c, 4, 0xff);
    put(&mut c, 8, 4);
    put(&mut c, COMPUTE_EXT_STRIDE, 9);
    let v = describe_compute(&c, 2);
    assert_eq!(v.len(), 2);
    assert!(v[0].starts_with("D3D12XBOX_COMPUTE_SHADER_LIMITS_DESC: DisableFlags=0xff (COMPUTE_STATIC_THREAD_MGMT_SE*.SH*_CU_EN), MaxWaves=0x4"), "{}", v[0]);
    assert!(v[1].contains("type 9"));
}

#[test]
fn derived_tag5_payload_is_the_same_block_and_inherits_from_the_base() {
    let d = base_desc();
    let fx = fortnite_ext(0x0f, 0xf0, 0);
    let e = entry_from_payload(&fx[4..4 + 0x18]);
    assert_eq!(&e[..], &fx[..]);
    let base = 0x5150 as *mut c_void;
    unsafe { note(base, 1, fx.as_ptr()) };
    assert_eq!(entry_of(base), Some(e));
    assert_eq!(entry_of(0x5158 as *mut c_void), None);
    unsafe { note(base, 0, core::ptr::null()) };
    assert_eq!(entry_of(base), None);
    unsafe { note(base, 1, fx.as_ptr()) };
    let (x, _) = parse_ext(&d, &e, 1);
    assert_eq!((x.back_read, x.back_write, x.depth_bounds_disable), (0x0f, 0xf0, 0));
}

unsafe fn vslot(o: *mut c_void, i: usize) -> usize {
    unsafe { *(*(o as *const *const usize)).add(i) }
}

unsafe fn release(o: *mut c_void) {
    if !o.is_null() {
        let f: unsafe extern "system" fn(*mut c_void) -> u32 = unsafe { core::mem::transmute(vslot(o, 2)) };
        unsafe { f(o) };
    }
}

unsafe fn blob_bytes(b: *mut c_void) -> Vec<u8> {
    let ptr: unsafe extern "system" fn(*mut c_void) -> *const u8 = unsafe { core::mem::transmute(vslot(b, 3)) };
    let len: unsafe extern "system" fn(*mut c_void) -> usize = unsafe { core::mem::transmute(vslot(b, 4)) };
    unsafe { core::slice::from_raw_parts(ptr(b), len(b)).to_vec() }
}

struct Gpu {
    dev: *mut c_void,
    objs: Vec<*mut c_void>,
    desc: Vec<u8>,
    _vs: Vec<u8>,
}

unsafe fn gpu() -> Gpu {
    unsafe {
        let mk: extern "system" fn(*mut c_void, u32, *const Guid, *mut *mut c_void) -> i32 = core::mem::transmute(crate::host_proc(b"D3D12CreateDevice\0"));
        let mut dev: *mut c_void = core::ptr::null_mut();
        assert!(mk(core::ptr::null_mut(), 0xb000, &IID_DEV, &mut dev) >= 0 && !dev.is_null());
        let ser: unsafe extern "system" fn(*const u8, u32, *mut *mut c_void, *mut *mut c_void) -> i32 = core::mem::transmute(crate::host_proc(b"D3D12SerializeRootSignature\0"));
        let rsd = [0u8; 0x28];
        let (mut rb, mut eb): (*mut c_void, *mut c_void) = (core::ptr::null_mut(), core::ptr::null_mut());
        assert!(ser(rsd.as_ptr(), 1, &mut rb, &mut eb) >= 0);
        let rbytes = blob_bytes(rb);
        let crs: unsafe extern "system" fn(*mut c_void, u32, *const u8, usize, *const Guid, *mut *mut c_void) -> i32 = core::mem::transmute(vslot(dev, 16));
        let mut rs: *mut c_void = core::ptr::null_mut();
        assert!(crs(dev, 0, rbytes.as_ptr(), rbytes.len(), &IID_RS, &mut rs) >= 0);
        let h = LoadLibraryA(b"d3dcompiler_47.dll\0".as_ptr());
        assert!(!h.is_null());
        type Compile = unsafe extern "system" fn(*const u8, usize, *const u8, *const c_void, *const c_void, *const u8, *const u8, u32, u32, *mut *mut c_void, *mut *mut c_void) -> i32;
        let fxc: Compile = core::mem::transmute(GetProcAddress(h, b"D3DCompile\0".as_ptr()));
        let src = "float4 main(uint i : SV_VertexID) : SV_Position { return float4(i & 1, i >> 1, 0.5, 1); }";
        let (mut code, mut err): (*mut c_void, *mut c_void) = (core::ptr::null_mut(), core::ptr::null_mut());
        assert!(fxc(src.as_ptr(), src.len(), core::ptr::null(), core::ptr::null(), core::ptr::null(), b"main\0".as_ptr(), b"vs_5_0\0".as_ptr(), 0, 0, &mut code, &mut err) >= 0);
        let vs = blob_bytes(code);
        let mut desc = base_desc();
        desc[0..8].copy_from_slice(&(rs as u64).to_le_bytes());
        desc[8..0x10].copy_from_slice(&(vs.as_ptr() as u64).to_le_bytes());
        desc[0x10..0x18].copy_from_slice(&(vs.len() as u64).to_le_bytes());
        Gpu { dev, objs: vec![code, err, rb, eb, rs], desc, _vs: vs }
    }
}

#[test]
fn wrapped_slot55_tag5_and_inherited_ext_reach_the_pc_pipeline() {
    unsafe {
        let g = gpu();
        let w = crate::device_wrap::wrap_device(g.dev);
        type Gfx = unsafe extern "system" fn(*mut c_void, *const u8, u32, *const u8, *const Guid, *mut *mut c_void) -> i32;
        let derived: Gfx = core::mem::transmute(vslot(w, 55));
        let ext = fortnite_ext(0x0f, 0xf0, 0);
        let mut base: *mut c_void = core::ptr::null_mut();
        let hr = create(g.dev, &g.desc, 1, ext.as_ptr(), &IID_PSO, &mut base).unwrap_or_else(|| {
            let plain: unsafe extern "system" fn(*mut c_void, *const u8, *const Guid, *mut *mut c_void) -> i32 = core::mem::transmute(vslot(g.dev, 10));
            plain(g.dev, g.desc.as_ptr(), &IID_PSO, &mut base)
        });
        assert!(hr >= 0 && !base.is_null(), "base hr={hr:#010x}");
        note(base, 1, ext.as_ptr());
        crate::device_wrap::register_gfx_pso(base, g.desc.clone(), Vec::new());
        assert_eq!(entry_of(base).map(|e| (e[0x14], e[0x15])), Some((0x0f, 0xf0)));
        let gfx_dv = |pso: *mut c_void, num: u32, patches: &[u8]| {
            let mut out: *mut c_void = core::ptr::null_mut();
            let hr = derived(w, pso as *const u8, num, if num == 0 { core::ptr::null() } else { patches.as_ptr() }, &IID_PSO, &mut out);
            assert!(hr >= 0 && !out.is_null(), "slot 55 num={num} hr={hr:#010x}");
            out
        };
        let inherit = gfx_dv(base, 0, &[]);
        assert_eq!(entry_of(inherit).map(|e| (e[0x14], e[0x15])), Some((0x0f, 0xf0)));
        let mut patch = vec![0u8; 0x14c];
        put(&mut patch, 0, 5);
        patch[4 + 0x10] = 0x33;
        patch[4 + 0x11] = 0xcc;
        patch[4 + 0x14] = 1;
        patch[4 + 0x15] = 1;
        let tagged = gfx_dv(base, 1, &patch);
        assert_eq!(entry_of(tagged).map(|e| (e[0x14], e[0x15], e[8])), Some((0x33, 0xcc, 0)));
        let mut blocks = vec![0u8; 0x14c * 3];
        for (i, tag) in [4u32, 6, 7].iter().enumerate() {
            put(&mut blocks, i * 0x14c, *tag);
        }
        put(&mut blocks, 0x14c + 4, SHADER_LIMITS_DEFAULT_LATE_ALLOC);
        put(&mut blocks, 4 + 4, 1);
        let opts = gfx_dv(base, 3, &blocks);
        assert_eq!(entry_of(opts).map(|e| (e[0x14], e[0x15])), Some((0x0f, 0xf0)));
        for o in [opts, tagged, inherit, base] {
            release(o);
        }
        for o in g.objs {
            release(o);
        }
    }
}

#[test]
fn fortnite_depth_stencil_ext_creates_a_pc_pipeline_with_bounds_and_back_masks() {
    unsafe {
        let mk: extern "system" fn(*mut c_void, u32, *const Guid, *mut *mut c_void) -> i32 = core::mem::transmute(crate::host_proc(b"D3D12CreateDevice\0"));
        let mut dev: *mut c_void = core::ptr::null_mut();
        assert!(mk(core::ptr::null_mut(), 0xb000, &IID_DEV, &mut dev) >= 0 && !dev.is_null());
        let ser: unsafe extern "system" fn(*const u8, u32, *mut *mut c_void, *mut *mut c_void) -> i32 = core::mem::transmute(crate::host_proc(b"D3D12SerializeRootSignature\0"));
        let rsd = [0u8; 0x28];
        let (mut rb, mut eb): (*mut c_void, *mut c_void) = (core::ptr::null_mut(), core::ptr::null_mut());
        assert!(ser(rsd.as_ptr(), 1, &mut rb, &mut eb) >= 0);
        let rbytes = blob_bytes(rb);
        let crs: unsafe extern "system" fn(*mut c_void, u32, *const u8, usize, *const Guid, *mut *mut c_void) -> i32 = core::mem::transmute(vslot(dev, 16));
        let mut rs: *mut c_void = core::ptr::null_mut();
        assert!(crs(dev, 0, rbytes.as_ptr(), rbytes.len(), &IID_RS, &mut rs) >= 0);
        let h = LoadLibraryA(b"d3dcompiler_47.dll\0".as_ptr());
        assert!(!h.is_null());
        type Compile = unsafe extern "system" fn(*const u8, usize, *const u8, *const c_void, *const c_void, *const u8, *const u8, u32, u32, *mut *mut c_void, *mut *mut c_void) -> i32;
        let fxc: Compile = core::mem::transmute(GetProcAddress(h, b"D3DCompile\0".as_ptr()));
        let src = "float4 main(uint i : SV_VertexID) : SV_Position { return float4(i & 1, i >> 1, 0.5, 1); }";
        let (mut code, mut err): (*mut c_void, *mut c_void) = (core::ptr::null_mut(), core::ptr::null_mut());
        assert!(fxc(src.as_ptr(), src.len(), core::ptr::null(), core::ptr::null(), core::ptr::null(), b"main\0".as_ptr(), b"vs_5_0\0".as_ptr(), 0, 0, &mut code, &mut err) >= 0);
        let vs = blob_bytes(code);
        let mut d = base_desc();
        d[0..8].copy_from_slice(&(rs as u64).to_le_bytes());
        d[8..0x10].copy_from_slice(&(vs.as_ptr() as u64).to_le_bytes());
        d[0x10..0x18].copy_from_slice(&(vs.len() as u64).to_le_bytes());
        let plain: unsafe extern "system" fn(*mut c_void, *const u8, *const Guid, *mut *mut c_void) -> i32 = core::mem::transmute(vslot(dev, 10));
        let mut p0: *mut c_void = core::ptr::null_mut();
        let hr0 = plain(dev, d.as_ptr(), &IID_PSO, &mut p0);
        assert!(hr0 >= 0, "baseline pso hr={hr0:#010x}");
        release(p0);
        let (bounds_ok, per_face_ok) = caps(dev);
        let ext = fortnite_ext(0x0f, 0xf0, 0);
        let mut p1: *mut c_void = core::ptr::null_mut();
        let r = create(dev, &d, 1, ext.as_ptr(), &IID_PSO, &mut p1);
        if bounds_ok || per_face_ok {
            assert!(matches!(r, Some(hr) if hr >= 0) && !p1.is_null(), "stream pso {r:?} bounds={bounds_ok} perFace={per_face_ok}");
        } else {
            assert!(r.is_none());
        }
        release(p1);
        for (pf, db) in [(true, false), (false, true)] {
            if (pf && !per_face_ok) || (db && !bounds_ok) {
                continue;
            }
            let (e, _) = parse_ext(&d, &ext, 1);
            let s = stream(&d, &e, &Plan { depth_bounds: db, per_face: pf });
            let qi: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32 = core::mem::transmute(vslot(dev, 0));
            let mut d2: *mut c_void = core::ptr::null_mut();
            assert!(qi(dev, &IID_DEVICE2, &mut d2) >= 0);
            let sd: [usize; 2] = [s.len(), s.as_ptr() as usize];
            let f: unsafe extern "system" fn(*mut c_void, *const [usize; 2], *const Guid, *mut *mut c_void) -> i32 = core::mem::transmute(vslot(d2, DEV2_CREATE_PIPELINE_STATE));
            let mut p2: *mut c_void = core::ptr::null_mut();
            let hr = f(d2, &sd, &IID_PSO, &mut p2);
            assert!(hr >= 0, "stream perFace={pf} bounds={db} hr={hr:#010x}");
            release(p2);
            release(d2);
        }
        for o in [code, err, rb, eb, rs, dev] {
            release(o);
        }
    }
}

fn patch(tag: u32, fill: impl Fn(&mut [u8])) -> Vec<u8> {
    let mut p = vec![0u8; DERIVED_PATCH_STRIDE];
    p[0..4].copy_from_slice(&tag.to_le_bytes());
    fill(&mut p[4..]);
    p
}

fn run(patches: &[u8], desc: &mut [u8], ext: &mut Option<[u8; EXT_STRIDE]>) -> usize {
    patches.chunks_exact(DERIVED_PATCH_STRIDE).filter(|p| apply_derived_patch(u32at(p, 0), &p[4..], desc, ext)).count()
}

#[test]
fn derived_tag0_after_inherited_tag5_follows_0x5b180_register_merge() {
    let mut desc = vec![0u8; GFX_DESC_SIZE];
    let mut tag5 = [0u8; 0x18];
    tag5[0x10] = 0x11;
    tag5[0x11] = 0x22;
    tag5[0x12] = 0x33;
    tag5[0x13] = 0x44;
    tag5[0x14] = 0x02;
    tag5[0x15] = 0x03;
    put(&mut tag5, 4, 0);
    let mut ext = Some(entry_from_payload(&tag5));
    let t0 = patch(0, |p| {
        put(p, 0xc, 1);
        p[0x10] = 0xaa;
        p[0x11] = 0xbb;
    });
    assert_eq!(run(&t0, &mut desc, &mut ext), 1);
    let e = ext.unwrap();
    let (pe, _) = parse_ext(&desc, &e, 1);
    assert_eq!((pe.back_read, pe.back_write), (0xaa, 0xbb));
    assert_eq!((pe.front_ref, pe.back_ref, pe.front_op, pe.back_op), (0x33, 0x44, 0x02, 0x03));
    assert_eq!(pe.depth_bounds_disable, 0);
    assert_eq!((desc[DSS + 0x10], desc[DSS + 0x11]), (0xaa, 0xbb));
    let t5 = patch(5, |p| p[..0x18].copy_from_slice(&tag5));
    let mut both = t0.clone();
    both.extend(&t5);
    let mut ext2 = None;
    assert_eq!(run(&both, &mut desc, &mut ext2), 2);
    let (pe2, _) = parse_ext(&desc, &ext2.unwrap(), 1);
    assert_eq!((pe2.back_read, pe2.back_write), (0x11, 0x22));
    let mut none = None;
    run(&t0, &mut desc, &mut none);
    assert!(none.is_none());
    assert_eq!((default_ext(&desc).back_read, default_ext(&desc).back_write), (0xaa, 0xbb));
}

#[test]
fn derived_patches_have_no_count_limit_like_0x5b180() {
    let mut desc = vec![0u8; GFX_DESC_SIZE];
    let mut all = Vec::new();
    for i in 0..40u32 {
        all.extend(patch(3, |p| put(p, 0, 0x1000 + i)));
    }
    let mut ext = None;
    assert_eq!(run(&all, &mut desc, &mut ext), 40);
    assert_eq!(u32at(&desc, 0x1c0), 0x1000 + 39);
    assert!(!apply_derived_patch(8, &[0u8; 0x148], &mut desc, &mut ext));
}

#[test]
fn registered_pso_owns_semantic_names_and_drops_the_cached_blob() {
    let mut name = b"POSITION\0".to_vec();
    let mut layout = vec![0u8; 32];
    layout[0..8].copy_from_slice(&(name.as_ptr() as u64).to_le_bytes());
    let cached = vec![7u8; 64];
    let mut desc = vec![0u8; crate::pso_serial::GFX_DESC_SIZE];
    desc[0x228..0x230].copy_from_slice(&(layout.as_ptr() as u64).to_le_bytes());
    desc[0x230..0x234].copy_from_slice(&1u32.to_le_bytes());
    desc[0x278..0x280].copy_from_slice(&(cached.as_ptr() as u64).to_le_bytes());
    desc[0x280..0x288].copy_from_slice(&64u64.to_le_bytes());
    let key = 0x7e57_0001usize as *mut c_void;
    unsafe { crate::device_wrap::register_gfx_pso(key, desc, Vec::new()) };
    name.iter_mut().for_each(|b| *b = 0xdd);
    layout.iter_mut().for_each(|b| *b = 0xee);
    let (d, _keep) = crate::device_wrap::gfx_pso_desc(key as usize).expect("registered");
    let lp = u64::from_le_bytes(d[0x228..0x230].try_into().unwrap()) as *const u8;
    assert_ne!(lp, layout.as_ptr());
    let np = unsafe { core::ptr::read_unaligned(lp as *const u64) } as *const u8;
    assert_ne!(np, name.as_ptr());
    let got = unsafe { core::ffi::CStr::from_ptr(np as *const core::ffi::c_char) }.to_bytes().to_vec();
    assert_eq!(got, b"POSITION");
    assert!(d[0x278..0x288].iter().all(|&b| b == 0));
}
