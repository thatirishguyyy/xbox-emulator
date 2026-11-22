#![cfg(windows)]
#![allow(dead_code)]

#[path = "../../tools/xreplay/src/d3d.rs"]
mod d3d;

use core::ffi::c_void;
use d3d::{Gpu, Guid};
use tombox_xshader::hlsl::translate;
use tombox_xshader::meta::SigElement;
use tombox_xshader::{dxc, ShaderExtract, Stage};

extern "system" {
    fn LoadLibraryA(n: *const u8) -> *mut c_void;
    fn GetProcAddress(h: *mut c_void, n: *const u8) -> *mut c_void;
}

const VS: &str = "void main(uint id : SV_VertexID, out float4 p : SV_Position, out float4 t0 : TEXCOORD0, out float c : SV_ClipDistance0, out float3 t1 : TEXCOORD1) { float2 q = float2(id & 1, id >> 1); p = float4(q, 0.5, 1); t0 = p; c = 1; t1 = p.xyz; }";

fn interp(op: u32, vdst: u32, chan: u32, vsrc: u32) -> u32 {
    0xc800_0000 | (vdst << 18) | (op << 16) | (chan << 8) | vsrc
}

fn el(name: &str, index: u32, sysval: u32, register: u32, mask: u8, rw: u8) -> SigElement {
    SigElement { name: name.into(), index, sysval, comp_type: 3, register, mask, rw_mask: rw }
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
            el("SV_Position", 0, 1, 0, 0xf, 0),
            el("TEXCOORD", 0, 0, 1, 0xf, 0x3),
            el("SV_ClipDistance", 0, 2, 2, 0x1, 0),
            el("TEXCOORD", 1, 0, 3, 0x7, 0),
        ],
        output_sig: vec![el("SV_Target", 0, 64, 0, 0xf, 0xf)],
        resources: Vec::new(),
        gs_info: None,
        root_sig: None,
    }
}

fn sig(blob: &[u8], tag: &[u8; 4]) -> Vec<(String, u32)> {
    let u = |o: usize| u32::from_le_bytes(blob[o..o + 4].try_into().unwrap()) as usize;
    let mut out = Vec::new();
    for i in 0..u(28) {
        let c = u(32 + 4 * i);
        if &blob[c..c + 4] != tag {
            continue;
        }
        let b = c + 8;
        for k in 0..u(b) {
            let e = b + u(b + 4) + 32 * k;
            let s = b + u(e + 4);
            let n = blob[s..].iter().position(|&x| x == 0).unwrap();
            out.push((format!("{}{}", String::from_utf8_lossy(&blob[s..s + n]), u(e + 8)), u(e + 20) as u32));
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
    Gpu { device, queue: core::ptr::null_mut(), alloc: core::ptr::null_mut(), list: core::ptr::null_mut(), fence: core::ptr::null_mut(), value: 0, info: core::ptr::null_mut(), open: false }
}

unsafe fn empty_root_signature(g: &Gpu) -> *mut c_void {
    let d = LoadLibraryA(b"d3d12.dll\0".as_ptr());
    let ser: unsafe extern "system" fn(*const u8, u32, *mut *mut c_void, *mut *mut c_void) -> i32 =
        core::mem::transmute(GetProcAddress(d, b"D3D12SerializeRootSignature\0".as_ptr()));
    let desc = [0u8; 40];
    let (mut blob, mut err): (*mut c_void, *mut c_void) = (core::ptr::null_mut(), core::ptr::null_mut());
    assert!(ser(desc.as_ptr(), 1, &mut blob, &mut err) >= 0, "serialize root signature");
    let bp = vcall!(blob, 3, fn() -> *const u8);
    let bn = vcall!(blob, 4, fn() -> usize);
    let mut rs: *mut c_void = core::ptr::null_mut();
    assert!(vcall!(g.device, 16, fn(u32, *const u8, usize, *const Guid, *mut *mut c_void) -> i32, 0, bp, bn, &d3d::IID_ROOTSIG, &mut rs) >= 0);
    rs
}

unsafe fn create_pso(g: &Gpu, rs: *mut c_void, vs: &[u8], ps: &[u8], samples: u32) -> i32 {
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
    put32(&mut d, 496, 1);
    put32(&mut d, 500, 1);
    put32(&mut d, 504, 4);
    for f in [516usize, 532] {
        put32(&mut d, f, 1);
        put32(&mut d, f + 4, 1);
        put32(&mut d, f + 8, 1);
        put32(&mut d, f + 12, 8);
    }
    put32(&mut d, 572, 3);
    put32(&mut d, 576, 1);
    put32(&mut d, 580, 26);
    put32(&mut d, 612, 20);
    put32(&mut d, 616, samples);
    let mut pso: *mut c_void = core::ptr::null_mut();
    let hr = vcall!(g.device, 10, fn(*const u8, *const Guid, *mut *mut c_void) -> i32, d.as_ptr(), &d3d::IID_PSO, &mut pso);
    d3d::release(pso);
    hr
}

#[test]
fn ps_clip_distance_input_keeps_the_vs_output_register() {
    let ex = ps_extract();
    let t = translate(&ex).expect("translate");
    let ps = dxc::compile(&t.hlsl, &dxc::profile_for(t.profile)).map_err(|e| format!("{e}\n{}", t.hlsl)).expect("ps compile");
    let vs = dxc::compile(VS, "vs_6_0").expect("vs compile");
    let vout = sig(&vs, b"OSG1");
    let pin = sig(&ps, b"ISG1");
    assert_eq!(pin.len(), 4, "{pin:?}\n{}", t.hlsl);
    for (name, reg) in &pin {
        let v = vout.iter().find(|(n, _)| n == name).unwrap_or_else(|| panic!("VS lacks {name}: {vout:?}"));
        assert_eq!(v.1, *reg, "{name}: VS output register {} but PS input register {reg}\nVS {vout:?}\nPS {pin:?}\n{}", v.1, t.hlsl);
    }
    let g = unsafe { warp() };
    let rs = unsafe { empty_root_signature(&g) };
    for samples in [1u32, 2] {
        let hr = unsafe { create_pso(&g, rs, &vs, &ps, samples) };
        assert!(hr >= 0, "CreateGraphicsPipelineState samples={samples} hr={:#010x}\nVS {vout:?}\nPS {pin:?}", hr as u32);
    }
    unsafe { d3d::release(rs) };
}
