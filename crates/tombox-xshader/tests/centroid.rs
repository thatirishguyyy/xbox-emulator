#![cfg(windows)]
#![allow(dead_code)]

#[path = "../../tools/xreplay/src/d3d.rs"]
mod d3d;

use core::ffi::c_void;
use d3d::{Gpu, Guid};
use tombox_xshader::centroid_abi::{pick_for, root_constants, snapped_offset, Constants, TB_CENTROID_CENTER, TB_CENTROID_PC_DEFINE, TB_CENTROID_REGISTER, TB_CENTROID_ROOT_DWORDS, TB_CENTROID_SPACE};
use tombox_xshader::hlsl::translate;
use tombox_xshader::meta::SigElement;
use tombox_xshader::{dxc, ShaderExtract, Stage};

extern "system" {
    fn LoadLibraryA(n: *const u8) -> *mut c_void;
    fn GetProcAddress(h: *mut c_void, n: *const u8) -> *mut c_void;
}

const W: u32 = 16;
const SAMPLES: u32 = 4;
const FMT_RG32F: u32 = 16;
const QUAD: [[[i8; 2]; 4]; 4] = [
    [[-7, -3], [5, -7], [7, 4], [-4, 6]],
    [[-6, -5], [6, -6], [4, 5], [-5, 3]],
    [[-3, -7], [7, -2], [2, 7], [-7, 2]],
    [[-5, -1], [3, -5], [6, 6], [-2, 4]],
];
const STANDARD: [[i8; 2]; 4] = [[-2, -6], [6, -2], [-6, 2], [2, 6]];

#[derive(Clone, Copy, PartialEq)]
enum Layout {
    Programmed,
    Standard,
}

fn position(l: Layout, px: u32, py: u32, s: usize) -> [i8; 2] {
    match l {
        Layout::Programmed => QUAD[((px & 1) | ((py & 1) << 1)) as usize][s],
        Layout::Standard => STANDARD[s],
    }
}

fn console_positions() -> [u8; 128] {
    let mut pos = [0u8; 128];
    for (p, q) in QUAD.iter().enumerate() {
        for (s, xy) in q.iter().enumerate() {
            pos[32 * p + 2 * s] = xy[0] as u8;
            pos[32 * p + 2 * s + 1] = xy[1] as u8;
        }
    }
    pos
}
const TRI: [[f64; 2]; 3] = [[1.3, 0.7], [15.1, 3.9], [4.6, 15.4]];
const RESOLVE_SOURCE: u32 = 0x2000;
const RESOLVE_DEST: u32 = 0x1000;
const RENDER_TARGET: u32 = 4;

fn vs_hlsl() -> String {
    let p: Vec<String> = TRI.iter().map(|v| format!("float2({}, {})", v[0], v[1])).collect();
    format!(
        "static const float2 P[3] = {{ {} }};\nvoid main(uint id : SV_VertexID, out float4 o : SV_Position, out float2 t : TEXCOORD_centroid0) {{ float2 q = P[id]; t = q; o = float4(q.x / {h} - 1.0, 1.0 - q.y / {h}, 0.5, 1.0); }}",
        p.join(", "),
        h = W as f64 / 2.0
    )
}

fn interp(op: u32, vdst: u32, chan: u32, vsrc: u32) -> u32 {
    0xc800_0000 | (vdst << 18) | (op << 16) | (chan << 8) | vsrc
}

fn ps_extract() -> ShaderExtract {
    let words = [
        interp(0, 0, 0, 0),
        interp(1, 0, 0, 1),
        interp(0, 1, 1, 0),
        interp(1, 1, 1, 1),
        0x7e00_0000 | (2 << 17) | (1 << 9) | 128,
        0x7e00_0000 | (3 << 17) | (1 << 9) | 128,
        0xf800_180f,
        0x0302_0100,
        0xbf81_0000,
    ];
    let isa: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    ShaderExtract {
        stage: Stage::Pixel,
        shader_model: (6, 0),
        bitcode_stripped: true,
        fssm: Vec::new(),
        entry: Some("main".into()),
        isa_data: isa.clone(),
        isa,
        isa_offset_in_fssm: 0,
        isa_preamble: [0; 4],
        psv_numthreads: None,
        has_input_sig: true,
        has_output_sig: true,
        has_psv0: false,
        has_root_sig: false,
        input_sig: vec![
            SigElement { name: "SV_Position".into(), index: 0, sysval: 1, comp_type: 3, register: 0, mask: 0xf, rw_mask: 0 },
            SigElement { name: "TEXCOORD_centroid".into(), index: 0, sysval: 0, comp_type: 3, register: 1, mask: 0x3, rw_mask: 0x3 },
        ],
        output_sig: vec![SigElement { name: "SV_Target".into(), index: 0, sysval: 64, comp_type: 3, register: 0, mask: 0xf, rw_mask: 0xf }],
        resources: Vec::new(),
        gs_info: None,
        root_sig: None,
    }
}

fn edge(a: [f64; 2], b: [f64; 2], p: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
}

fn sample_at(l: Layout, px: u32, py: u32, s: usize) -> [f64; 2] {
    let o = position(l, px, py, s);
    [px as f64 + 0.5 + o[0] as f64 / 16.0, py as f64 + 0.5 + o[1] as f64 / 16.0]
}

fn coverage(l: Layout, px: u32, py: u32) -> u32 {
    let mut m = 0;
    for s in 0..SAMPLES as usize {
        let p = sample_at(l, px, py, s);
        let e = [edge(TRI[0], TRI[1], p), edge(TRI[1], TRI[2], p), edge(TRI[2], TRI[0], p)];
        assert!(e.iter().all(|v| v.abs() > 1e-3), "sample on an edge: test geometry is ambiguous");
        if e.iter().all(|&v| v > 0.0) || e.iter().all(|&v| v < 0.0) {
            m |= 1 << s;
        }
    }
    m
}

fn reference(l: Layout, c: &Constants) -> Vec<Option<[f64; 2]>> {
    let mut out = Vec::new();
    for py in 0..W {
        for px in 0..W {
            let cov = coverage(l, px, py);
            if cov == 0 {
                out.push(None);
                continue;
            }
            let s = pick_for(c, cov, SAMPLES);
            let loc = if s == TB_CENTROID_CENTER {
                [px as f64 + 0.5, py as f64 + 0.5]
            } else {
                let o = snapped_offset(c, s, px, py).unwrap_or(position(Layout::Standard, px, py, s as usize));
                assert_eq!(o, position(l, px, py, s as usize));
                [px as f64 + 0.5 + o[0] as f64 / 16.0, py as f64 + 0.5 + o[1] as f64 / 16.0]
            };
            let k = cov.count_ones() as f64 / SAMPLES as f64;
            out.push(Some([loc[0] * k, loc[1] * k]));
        }
    }
    out
}

unsafe fn warp() -> Gpu {
    const IID_FACTORY4: Guid = Guid(0x1bc6ea02, 0xef36, 0x464f, [0xbf, 0x0c, 0x21, 0xca, 0x39, 0xe5, 0x16, 0x8a]);
    const IID_ADAPTER: Guid = Guid(0x2411e7e1, 0x12ac, 0x4ccf, [0xbd, 0x14, 0x97, 0x98, 0xe8, 0x53, 0x4d, 0xc0]);
    let dxgi = LoadLibraryA(b"dxgi.dll\0".as_ptr());
    let cf: unsafe extern "system" fn(*const Guid, *mut *mut c_void) -> i32 = core::mem::transmute(GetProcAddress(dxgi, b"CreateDXGIFactory1\0".as_ptr()));
    let mut f: *mut c_void = core::ptr::null_mut();
    assert!(cf(&IID_FACTORY4, &mut f) >= 0);
    let mut ad: *mut c_void = core::ptr::null_mut();
    assert!(vcall!(f, 27, fn(*const Guid, *mut *mut c_void) -> i32, &IID_ADAPTER, &mut ad) >= 0);
    let d = LoadLibraryA(b"d3d12.dll\0".as_ptr());
    let create: unsafe extern "system" fn(*mut c_void, u32, *const Guid, *mut *mut c_void) -> i32 = core::mem::transmute(GetProcAddress(d, b"D3D12CreateDevice\0".as_ptr()));
    let mut device: *mut c_void = core::ptr::null_mut();
    assert!(create(ad, 0xc100, &d3d::IID_DEVICE, &mut device) >= 0);
    let qd: [u32; 4] = [0, 0, 0, 0];
    let mut queue: *mut c_void = core::ptr::null_mut();
    vcall!(device, 8, fn(*const u32, *const Guid, *mut *mut c_void) -> i32, qd.as_ptr(), &d3d::IID_QUEUE, &mut queue);
    let mut alloc: *mut c_void = core::ptr::null_mut();
    vcall!(device, 9, fn(u32, *const Guid, *mut *mut c_void) -> i32, 0, &d3d::IID_ALLOCATOR, &mut alloc);
    let mut list: *mut c_void = core::ptr::null_mut();
    vcall!(device, 12, fn(u32, u32, *mut c_void, *mut c_void, *const Guid, *mut *mut c_void) -> i32, 0, 0, alloc, core::ptr::null_mut(), &d3d::IID_GFX_LIST, &mut list);
    vcall!(list, 9, fn() -> i32);
    let mut fence: *mut c_void = core::ptr::null_mut();
    vcall!(device, 36, fn(u64, u32, *const Guid, *mut *mut c_void) -> i32, 0, 0, &d3d::IID_FENCE, &mut fence);
    Gpu { device, queue, alloc, list, fence, value: 0, info: core::ptr::null_mut(), open: false }
}

unsafe fn root_signature(g: &Gpu) -> *mut c_void {
    let d = LoadLibraryA(b"d3d12.dll\0".as_ptr());
    let ser: unsafe extern "system" fn(*const u8, u32, *mut *mut c_void, *mut *mut c_void) -> i32 =
        core::mem::transmute(GetProcAddress(d, b"D3D12SerializeRootSignature\0".as_ptr()));
    let mut p = [0u8; 32];
    p[0..4].copy_from_slice(&1u32.to_le_bytes());
    p[8..12].copy_from_slice(&TB_CENTROID_REGISTER.to_le_bytes());
    p[12..16].copy_from_slice(&TB_CENTROID_SPACE.to_le_bytes());
    p[16..20].copy_from_slice(&TB_CENTROID_ROOT_DWORDS.to_le_bytes());
    p[24..28].copy_from_slice(&5u32.to_le_bytes());
    let mut desc = [0u8; 40];
    desc[0..4].copy_from_slice(&1u32.to_le_bytes());
    desc[8..16].copy_from_slice(&(p.as_ptr() as u64).to_le_bytes());
    let (mut blob, mut err): (*mut c_void, *mut c_void) = (core::ptr::null_mut(), core::ptr::null_mut());
    assert!(ser(desc.as_ptr(), 1, &mut blob, &mut err) >= 0, "serialize root signature");
    let bp = vcall!(blob, 3, fn() -> *const u8);
    let bn = vcall!(blob, 4, fn() -> usize);
    let mut rs: *mut c_void = core::ptr::null_mut();
    assert!(vcall!(g.device, 16, fn(u32, *const u8, usize, *const Guid, *mut *mut c_void) -> i32, 0, bp, bn, &d3d::IID_ROOTSIG, &mut rs) >= 0);
    rs
}

unsafe fn sample_positions_tier(g: &Gpu) -> u32 {
    let mut o = [0u32; 2];
    if vcall!(g.device, 13, fn(u32, *mut c_void, u32) -> i32, 18, o.as_mut_ptr() as *mut c_void, 8) < 0 {
        return 0;
    }
    o[1]
}

fn tex_desc(samples: u32, rt: bool) -> [u8; 0x38] {
    let mut td = [0u8; 0x38];
    td[0..4].copy_from_slice(&3u32.to_le_bytes());
    td[0x10..0x18].copy_from_slice(&(W as u64).to_le_bytes());
    td[0x18..0x1c].copy_from_slice(&W.to_le_bytes());
    td[0x1c..0x1e].copy_from_slice(&1u16.to_le_bytes());
    td[0x1e..0x20].copy_from_slice(&1u16.to_le_bytes());
    td[0x20..0x24].copy_from_slice(&FMT_RG32F.to_le_bytes());
    td[0x24..0x28].copy_from_slice(&samples.to_le_bytes());
    if rt {
        td[0x30..0x34].copy_from_slice(&1u32.to_le_bytes());
    }
    td
}

unsafe fn render(g: &mut Gpu, rs: *mut c_void, vs: &[u8], ps: &[u8], consts: &Constants, lay: Layout) -> Result<Vec<[f32; 2]>, String> {
    let mut d = vec![0u8; 656];
    let put32 = |d: &mut Vec<u8>, o: usize, v: u32| d[o..o + 4].copy_from_slice(&v.to_le_bytes());
    let put64 = |d: &mut Vec<u8>, o: usize, v: u64| d[o..o + 8].copy_from_slice(&v.to_le_bytes());
    put64(&mut d, 0, rs as u64);
    put64(&mut d, 8, vs.as_ptr() as u64);
    put64(&mut d, 16, vs.len() as u64);
    put64(&mut d, 24, ps.as_ptr() as u64);
    put64(&mut d, 32, ps.len() as u64);
    for i in 0..8 {
        let o = 128 + i * 40;
        put32(&mut d, o + 8, 2);
        put32(&mut d, o + 12, 1);
        put32(&mut d, o + 16, 1);
        put32(&mut d, o + 20, 2);
        put32(&mut d, o + 24, 1);
        put32(&mut d, o + 28, 1);
        put32(&mut d, o + 32, 4);
        d[o + 36] = 0xf;
    }
    put32(&mut d, 448, 0xffff_ffff);
    put32(&mut d, 452, 3);
    put32(&mut d, 456, 1);
    put32(&mut d, 476, 1);
    put32(&mut d, 480, 1);
    put32(&mut d, 504, 8);
    for f in [516usize, 532] {
        put32(&mut d, f, 1);
        put32(&mut d, f + 4, 1);
        put32(&mut d, f + 8, 1);
        put32(&mut d, f + 12, 8);
    }
    put32(&mut d, 572, 3);
    put32(&mut d, 576, 1);
    put32(&mut d, 580, FMT_RG32F);
    put32(&mut d, 616, SAMPLES);
    let mut pso: *mut c_void = core::ptr::null_mut();
    let hr = vcall!(g.device, 10, fn(*const u8, *const Guid, *mut *mut c_void) -> i32, d.as_ptr(), &d3d::IID_PSO, &mut pso);
    if hr < 0 {
        return Err(format!("CreateGraphicsPipelineState hr={:#010x}{}", hr as u32, g.messages()));
    }
    let ms = g.committed(&tex_desc(SAMPLES, true), d3d::default_heap(), RENDER_TARGET)?;
    let rt1 = g.committed(&tex_desc(1, false), d3d::default_heap(), RESOLVE_DEST)?;
    let rb = g.buffer(256 * W as u64, 3, d3d::COPY_DEST, false)?;
    let (heap, rtv, _, _) = g.heap(2, 1, false)?;
    vcall!(g.device, 20, fn(*mut c_void, *const c_void, usize) -> (), ms, core::ptr::null(), rtv);
    g.begin();
    let l = g.list;
    let clear = [0f32; 4];
    vcall!(l, 48, fn(usize, *const f32, u32, *const c_void) -> (), rtv, clear.as_ptr(), 0, core::ptr::null());
    vcall!(l, 46, fn(u32, *const usize, i32, *const usize) -> (), 1, &rtv, 0, core::ptr::null());
    vcall!(l, 30, fn(*mut c_void) -> (), rs);
    vcall!(l, 36, fn(u32, u32, *const u32, u32) -> (), 0, TB_CENTROID_ROOT_DWORDS, consts.as_ptr(), 0);
    vcall!(l, 25, fn(*mut c_void) -> (), pso);
    const IID_LIST1: Guid = Guid(0x553103fb, 0x1fe7, 0x4557, [0xbb, 0x38, 0x94, 0x6d, 0x7d, 0x0e, 0x7c, 0xa7]);
    let mut l1: *mut c_void = core::ptr::null_mut();
    if vcall!(l, 0, fn(*const Guid, *mut *mut c_void) -> i32, &IID_LIST1, &mut l1) < 0 || l1.is_null() {
        return Err("ID3D12GraphicsCommandList1 not supported".into());
    }
    if lay == Layout::Programmed {
        vcall!(l1, 63, fn(u32, u32, *const [i8; 2]) -> (), SAMPLES, 4, QUAD.as_ptr() as *const [i8; 2]);
    }
    d3d::release(l1);
    let vp = [0f32, 0., W as f32, W as f32, 0., 1.];
    vcall!(l, 21, fn(u32, *const f32) -> (), 1, vp.as_ptr());
    let sc = [0i32, 0, W as i32, W as i32];
    vcall!(l, 22, fn(u32, *const i32) -> (), 1, sc.as_ptr());
    vcall!(l, 20, fn(u32) -> (), 4);
    vcall!(l, 12, fn(u32, u32, u32, u32) -> (), 3, 1, 0, 0);
    g.barrier(ms, RENDER_TARGET, RESOLVE_SOURCE);
    vcall!(l, 19, fn(*mut c_void, u32, *mut c_void, u32, u32) -> (), rt1, 0, ms, 0, FMT_RG32F);
    g.barrier(rt1, RESOLVE_DEST, d3d::COPY_SOURCE);
    let mut dst = [0u8; 48];
    dst[0..8].copy_from_slice(&(rb as u64).to_le_bytes());
    dst[8..12].copy_from_slice(&1u32.to_le_bytes());
    dst[24..28].copy_from_slice(&FMT_RG32F.to_le_bytes());
    dst[28..32].copy_from_slice(&W.to_le_bytes());
    dst[32..36].copy_from_slice(&W.to_le_bytes());
    dst[36..40].copy_from_slice(&1u32.to_le_bytes());
    dst[40..44].copy_from_slice(&256u32.to_le_bytes());
    let mut src = [0u8; 48];
    src[0..8].copy_from_slice(&(rt1 as u64).to_le_bytes());
    vcall!(l, 16, fn(*const u8, u32, u32, u32, *const u8, *const c_void) -> (), dst.as_ptr(), 0, 0, 0, src.as_ptr(), core::ptr::null());
    let r = g.submit();
    let out = r.and_then(|_| {
        let b = Gpu::map_read(rb, 256 * W as usize).ok_or("map readback")?;
        let mut v = Vec::new();
        for y in 0..W as usize {
            for x in 0..W as usize {
                let o = y * 256 + x * 8;
                v.push([f32::from_le_bytes(b[o..o + 4].try_into().unwrap()), f32::from_le_bytes(b[o + 4..o + 8].try_into().unwrap())]);
            }
        }
        Ok(v)
    });
    for o in [pso, ms, rt1, rb, heap] {
        d3d::release(o);
    }
    out
}

fn mismatches(l: Layout, got: &[[f32; 2]], want: &[Option<[f64; 2]>]) -> Vec<String> {
    let mut bad = Vec::new();
    for (i, (g, w)) in got.iter().zip(want).enumerate() {
        let w = w.unwrap_or([0.0, 0.0]);
        if (g[0] as f64 - w[0]).abs() > 2e-3 || (g[1] as f64 - w[1]).abs() > 2e-3 {
            bad.push(format!("pixel ({}, {}): got {:?} want {:?} cov {:#x}", i as u32 % W, i as u32 / W, g, w, coverage(l, i as u32 % W, i as u32 / W)));
        }
    }
    bad
}

fn partial_pixels(l: Layout) -> usize {
    (0..W * W).filter(|i| {
        let c = coverage(l, i % W, i / W);
        c != 0 && c != (1 << SAMPLES) - 1
    }).count()
}

#[test]
fn centroid_priority_lowering_matches_cpu_reference() {
    let ex = ps_extract();
    let t = translate(&ex).expect("translate");
    assert!(t.hlsl.contains("EvaluateAttributeSnapped(i.i1, tb_co)") && t.hlsl.contains("EvaluateAttributeAtSample(i.i1, tb_cs)") && t.hlsl.contains("SV_Coverage"), "{}", t.hlsl);
    let prof = dxc::profile_for(t.profile);
    let ps = dxc::compile(&t.hlsl, &prof).map_err(|e| format!("{e}
{}", t.hlsl)).expect("ps compile");
    let ps_pc = dxc::compile(&format!("#define {TB_CENTROID_PC_DEFINE}
{}", t.hlsl), &prof).map_err(|e| format!("{e}
{}", t.hlsl)).expect("pc-variant compile");
    let vs = dxc::compile(&vs_hlsl(), "vs_6_0").expect("vs compile");
    let hw = std::env::var_os("TOMBOX_EXEC_HW").is_some();
    let mut g = unsafe { if hw { Gpu::new(std::env::var_os("TOMBOX_EXEC_DEBUG").is_some()).expect("d3d12 device") } else { warp() } };
    g.open = false;
    let tier = unsafe { sample_positions_tier(&g) };
    assert!(!hw || tier >= 2, "hardware adapter reports ProgrammableSamplePositionsTier {tier}");
    let layouts: &[Layout] = if hw { &[Layout::Programmed, Layout::Standard] } else { &[Layout::Standard] };
    for &l in layouts {
        assert!(partial_pixels(l) >= 20, "geometry yields too few partially covered pixels");
    }
    let rs = unsafe { root_signature(&g) };
    let pos = console_positions();
    let tables: [Option<[u8; 16]>; 3] = [Some([3, 2, 1, 0, 3, 2, 1, 0, 3, 2, 1, 0, 3, 2, 1, 0]), Some([1, 0, 3, 2, 9, 9, 9, 9, 1, 0, 3, 2, 1, 0, 3, 2]), None];
    let mut wants = Vec::new();
    for &l in layouts {
        for tbl in tables {
            let c = root_constants(tbl, (l == Layout::Programmed).then_some((&pos, SAMPLES)));
            let want = reference(l, &c);
            let got = unsafe { render(&mut g, rs, &vs, &ps, &c, l) }.expect("render");
            let bad = mismatches(l, &got, &want);
            assert!(bad.is_empty(), "table {tbl:?} positions {} tier {tier}: {} mismatches
{}", l == Layout::Programmed, bad.len(), bad.join("
"));
            wants.push(want);
        }
    }
    let differ = |a: &[Option<[f64; 2]>], b: &[Option<[f64; 2]>]| a.iter().zip(b).filter(|(x, y)| x != y).count();
    assert!(differ(&wants[0], &wants[1]) > 0 && differ(&wants[0], &wants[2]) > 0, "priority tables do not discriminate");
    let l0 = layouts[0];
    let c = root_constants(None, (l0 == Layout::Programmed).then_some((&pos, SAMPLES)));
    let pc = unsafe { render(&mut g, rs, &vs, &ps_pc, &c, l0) }.expect("render pc variant");
    let (m0, m1) = (mismatches(l0, &pc, &wants[0]).len(), mismatches(l0, &pc, &wants[1]).len());
    assert!(m0 > 0 || m1 > 0, "PC centroid matched two different console priority tables");
    eprintln!("hw {hw} tier {tier}: {} partial pixels; PC hardware centroid differs from console table A on {m0} and table B on {m1} pixels", partial_pixels(l0));
    unsafe { d3d::release(rs) };
}
