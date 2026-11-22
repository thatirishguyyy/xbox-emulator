#![cfg(windows)]
#![allow(dead_code)]

#[path = "../../tools/xreplay/src/d3d.rs"]
mod d3d;

use core::ffi::c_void;
use d3d::{Gpu, Guid};
use tombox_xshader::hlsl::translate;
use tombox_xshader::{dxc, extract, rdna2, Stage};

extern "system" {
    fn LoadLibraryA(n: *const u8) -> *mut c_void;
    fn GetProcAddress(h: *mut c_void, n: *const u8) -> *mut c_void;
}

const IID_DEVICE2: Guid = Guid(0x30baa41e, 0xb15b, 0x475c, [0xa0, 0xbb, 0x1a, 0xf5, 0xc5, 0xb6, 0x43, 0x28]);
const IID_LIST6: Guid = Guid(0xc3827890, 0xe548, 0x4cfa, [0x96, 0xcf, 0x56, 0x89, 0xa9, 0x37, 0x0f, 0x80]);
const PS: &str = "uint4 main(float4 p : SV_Position, nointerpolation uint4 t : TEXCOORD0) : SV_Target { return t; }";
const W: usize = 8;
const UAV_STATE: u32 = 8;

fn fixture(name: &str) -> Vec<u8> {
    let mut p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("../../analysis/shaders/console_xsc");
    p.push(name);
    std::fs::read(&p).unwrap_or_else(|e| panic!("missing fixture {}: {e}", p.display()))
}

fn hlsl_of(name: &str) -> tombox_xshader::hlsl::Translated {
    let ex = extract(&fixture(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
    translate(&ex).unwrap_or_else(|e| panic!("{name}: {e}"))
}

#[test]
#[cfg_attr(not(tombox_fixtures_console_xsc), ignore = "needs analysis/shaders/console_xsc fixtures extracted from local title dumps")]
fn console_as_fixture_uses_the_decoded_ring_abi() {
    let ex = extract(&fixture("as3.dxbc")).unwrap();
    assert_eq!(ex.stage, Stage::Amplification);
    let m = ex.gs_info.and_then(|g| g.mesh).expect("PSV0 AS info");
    assert_eq!((m.stage, m.payload_bytes, m.num_threads), (14, 16, [32, 1, 1]));
    let insns = rdna2::disassemble(&ex.isa);
    let stores: Vec<_> = insns.iter().filter(|i| i.enc == rdna2::Enc::Flat && i.op & 0x7f == 30).collect();
    assert_eq!(stores.len(), 2);
    assert!(insns.iter().any(|i| i.mnemonic == "s_lshl_b64" && i.src.first() == Some(&rdna2::Opnd::Sgpr(2))));
    assert!(insns.iter().any(|i| i.mnemonic == "s_lshl_b64" && i.src.first() == Some(&rdna2::Opnd::Sgpr(6))));
    let ex2 = extract(&fixture("as2.dxbc")).unwrap();
    let i2 = rdna2::disassemble(&ex2.isa);
    assert!(i2.iter().any(|i| i.mnemonic == "s_mul_i32" && i.src.get(1) == Some(&rdna2::Opnd::ConstInt(48))), "40-byte payload uses a 48-byte ring stride");
}

#[test]
#[cfg_attr(not(tombox_fixtures_console_xsc), ignore = "needs analysis/shaders/console_xsc fixtures extracted from local title dumps")]
fn amp_hlsl_dispatches_from_the_args_entry() {
    let t = hlsl_of("as3.dxbc");
    assert_eq!(t.profile, "as_6_5");
    assert!(t.hlsl.lines().any(|l| l == "#define TB_AMP_GMEM 1"), "{}", t.hlsl);
    assert!(t.hlsl.contains("struct TbPayload { uint w[4]; };"), "{}", t.hlsl);
    assert!(t.hlsl.contains("DispatchMesh(tb_args[0], max(tb_args[1], 1u), max(tb_args[2], 1u), tb_pl);"), "{}", t.hlsl);
    assert!(t.hlsl.contains("s2 = 0x10000000u; s3 = 0x20000000u; s4 = gid.x;"), "{}", t.hlsl);
    let t2 = hlsl_of("as2.dxbc");
    assert!(t2.hlsl.contains("struct TbPayload { uint w[12]; };"), "{}", t2.hlsl);
    for t in [&t, &t2] {
        dxc::compile(&t.hlsl, &dxc::profile_for(t.profile)).unwrap_or_else(|e| panic!("{e}\n{}", t.hlsl));
    }
}

#[test]
#[cfg_attr(not(tombox_fixtures_console_xsc), ignore = "needs analysis/shaders/console_xsc fixtures extracted from local title dumps")]
fn paired_mesh_shader_reads_the_payload_window() {
    let t = hlsl_of("ms3.dxbc");
    assert!(t.hlsl.contains("in payload TbPayload tb_pl"), "{}", t.hlsl);
    assert!(t.hlsl.contains("s10 = 0x3u; s11 = 0u;"), "{}", t.hlsl);
    dxc::compile(&t.hlsl, &dxc::profile_for(t.profile)).unwrap_or_else(|e| panic!("{e}\n{}", t.hlsl));
}

unsafe fn warp_device() -> *mut c_void {
    const IID_FACTORY4: Guid = Guid(0x1bc6ea02, 0xef36, 0x464f, [0xbf, 0x0c, 0x21, 0xca, 0x39, 0xe5, 0x16, 0x8a]);
    const IID_ADAPTER: Guid = Guid(0x2411e7e1, 0x12ac, 0x4ccf, [0xbd, 0x14, 0x97, 0x98, 0xe8, 0x53, 0x4d, 0xc0]);
    let dxgi = LoadLibraryA(b"dxgi.dll\0".as_ptr());
    let cf: unsafe extern "system" fn(*const Guid, *mut *mut c_void) -> i32 = core::mem::transmute(GetProcAddress(dxgi, b"CreateDXGIFactory1\0".as_ptr()));
    let mut f: *mut c_void = core::ptr::null_mut();
    assert!(cf(&IID_FACTORY4, &mut f) >= 0, "CreateDXGIFactory1");
    let mut ad: *mut c_void = core::ptr::null_mut();
    assert!(vcall!(f, 27, fn(*const Guid, *mut *mut c_void) -> i32, &IID_ADAPTER, &mut ad) >= 0, "EnumWarpAdapter");
    let d3d = LoadLibraryA(b"d3d12.dll\0".as_ptr());
    let create: unsafe extern "system" fn(*mut c_void, u32, *const Guid, *mut *mut c_void) -> i32 = core::mem::transmute(GetProcAddress(d3d, b"D3D12CreateDevice\0".as_ptr()));
    let mut device: *mut c_void = core::ptr::null_mut();
    assert!(create(ad, 0xc100, &d3d::IID_DEVICE, &mut device) >= 0, "D3D12CreateDevice(WARP)");
    device
}

unsafe fn gpu_on(device: *mut c_void) -> Gpu {
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

unsafe fn mesh_tier(device: *mut c_void) -> u32 {
    let mut o7 = [0u32; 2];
    let hr = vcall!(device, 13, fn(u32, *mut c_void, u32) -> i32, 32, o7.as_mut_ptr() as *mut c_void, 8);
    if hr < 0 { 0 } else { o7[0] }
}

unsafe fn mesh_gpu() -> (Gpu, &'static str) {
    if std::env::var_os("TOMBOX_EXEC_HW").is_none() {
        let d = warp_device();
        assert!(mesh_tier(d) != 0, "WARP reports MeshShaderTier 0: rerun with TOMBOX_EXEC_HW=1 to use the hardware adapter");
        return (gpu_on(d), "WARP");
    }
    let d = hw_mesh_device().expect("no hardware adapter supports mesh shaders");
    (gpu_on(d), "hardware")
}

unsafe fn hw_mesh_device() -> Option<*mut c_void> {
    const IID_FACTORY6: Guid = Guid(0xc1b6694f, 0xff09, 0x44a9, [0xb0, 0x3c, 0x77, 0x90, 0x0a, 0x0a, 0x1d, 0x17]);
    const IID_ADAPTER1: Guid = Guid(0x29038f61, 0x3839, 0x4626, [0x91, 0xfd, 0x08, 0x68, 0x79, 0x01, 0x1a, 0x05]);
    const DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE: u32 = 2;
    const DXGI_ADAPTER_FLAG_SOFTWARE: u32 = 2;
    let dxgi = LoadLibraryA(b"dxgi.dll\0".as_ptr());
    let cf: unsafe extern "system" fn(*const Guid, *mut *mut c_void) -> i32 = core::mem::transmute(GetProcAddress(dxgi, b"CreateDXGIFactory1\0".as_ptr()));
    let mut f: *mut c_void = core::ptr::null_mut();
    assert!(cf(&IID_FACTORY6, &mut f) >= 0, "CreateDXGIFactory1(IDXGIFactory6)");
    let d3d = LoadLibraryA(b"d3d12.dll\0".as_ptr());
    let create: unsafe extern "system" fn(*mut c_void, u32, *const Guid, *mut *mut c_void) -> i32 = core::mem::transmute(GetProcAddress(d3d, b"D3D12CreateDevice\0".as_ptr()));
    for i in 0.. {
        let mut ad: *mut c_void = core::ptr::null_mut();
        if vcall!(f, 29, fn(u32, u32, *const Guid, *mut *mut c_void) -> i32, i, DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE, &IID_ADAPTER1, &mut ad) < 0 {
            return None;
        }
        let mut desc = [0u32; 78];
        vcall!(ad, 10, fn(*mut c_void) -> i32, desc.as_mut_ptr() as *mut c_void);
        if desc[76] & DXGI_ADAPTER_FLAG_SOFTWARE != 0 {
            continue;
        }
        let mut device: *mut c_void = core::ptr::null_mut();
        if create(ad, 0xc100, &d3d::IID_DEVICE, &mut device) >= 0 {
            if mesh_tier(device) != 0 {
                return Some(device);
            }
            vcall!(device, 2, fn() -> u32);
        }
    }
    None
}

fn sub(stream: &mut Vec<u8>, ty: u32, body: &[u8]) {
    stream.extend_from_slice(&ty.to_le_bytes());
    let align = if matches!(ty, 0..=6 | 19 | 24 | 25) { 8 } else { 4 };
    while stream.len() % align != 0 {
        stream.push(0);
    }
    stream.extend_from_slice(body);
    while stream.len() % 8 != 0 {
        stream.push(0);
    }
}

fn bytecode(b: &[u8]) -> Vec<u8> {
    let mut v = (b.as_ptr() as u64).to_le_bytes().to_vec();
    v.extend_from_slice(&(b.len() as u64).to_le_bytes());
    v
}

unsafe fn root_signature(gpu: &Gpu) -> *mut c_void {
    const SPACE: u32 = 4_000_003;
    let mut params = [0u8; 32 * 4];
    for (i, (ty, reg)) in [(2u32, 0u32), (4, 0), (4, 1), (4, 2)].iter().enumerate() {
        let p = &mut params[i * 32..i * 32 + 32];
        p[0..4].copy_from_slice(&ty.to_le_bytes());
        p[8..12].copy_from_slice(&reg.to_le_bytes());
        p[12..16].copy_from_slice(&SPACE.to_le_bytes());
    }
    let mut desc = [0u8; 40];
    desc[0..4].copy_from_slice(&4u32.to_le_bytes());
    desc[8..16].copy_from_slice(&(params.as_ptr() as u64).to_le_bytes());
    let d3d12 = LoadLibraryA(b"d3d12.dll\0".as_ptr());
    let ser: unsafe extern "system" fn(*const u8, u32, *mut *mut c_void, *mut *mut c_void) -> i32 =
        core::mem::transmute(GetProcAddress(d3d12, b"D3D12SerializeRootSignature\0".as_ptr()));
    let mut blob: *mut c_void = core::ptr::null_mut();
    let mut err: *mut c_void = core::ptr::null_mut();
    assert!(ser(desc.as_ptr(), 1, &mut blob, &mut err) >= 0, "serialize root signature");
    let mut rs: *mut c_void = core::ptr::null_mut();
    let hr = vcall!(gpu.device, 16, fn(u32, *const u8, usize, *const Guid, *mut *mut c_void) -> i32, 0, vcall!(blob, 3, fn() -> *const u8), vcall!(blob, 4, fn() -> usize), &d3d::IID_ROOTSIG, &mut rs);
    assert!(hr >= 0, "CreateRootSignature {hr:#x}");
    rs
}

unsafe fn render(as_name: &str, ms_name: &str, groups: u32) -> Result<([[u32; 4]; W], [u32; 3]), String> {
    let ta = hlsl_of(as_name);
    let tm = hlsl_of(ms_name);
    let asb = dxc::compile(&ta.hlsl, &dxc::profile_for(ta.profile)).map_err(|e| format!("{e}\n{}", ta.hlsl))?;
    let msb = dxc::compile(&tm.hlsl, &dxc::profile_for(tm.profile)).map_err(|e| format!("{e}\n{}", tm.hlsl))?;
    let psb = dxc::compile(PS, "ps_6_5")?;
    let (mut gpu, kind) = mesh_gpu();
    eprintln!("amplification render on {kind}");
    let rs = root_signature(&gpu);
    let mut s = Vec::new();
    sub(&mut s, 0, &(rs as u64).to_le_bytes());
    sub(&mut s, 24, &bytecode(&asb));
    sub(&mut s, 25, &bytecode(&msb));
    sub(&mut s, 2, &bytecode(&psb));
    let mut rast = [0u32; 11];
    rast[0] = 3;
    rast[1] = 1;
    rast[6] = 1;
    sub(&mut s, 10, &rast.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>());
    let mut ds = [0u32; 13];
    ds[2] = 8;
    ds[4] = 0xffff;
    for k in [5usize, 9] {
        ds[k] = 1;
        ds[k + 1] = 1;
        ds[k + 2] = 1;
        ds[k + 3] = 8;
    }
    sub(&mut s, 11, &ds.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>());
    let mut rtf = [0u32; 9];
    rtf[0] = 3;
    rtf[8] = 1;
    sub(&mut s, 15, &rtf.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>());
    sub(&mut s, 17, &[1, 0, 0, 0, 0, 0, 0, 0]);
    sub(&mut s, 9, &0xffff_ffffu32.to_le_bytes());
    let sd: [u64; 2] = [s.len() as u64, s.as_ptr() as u64];
    let mut dev2: *mut c_void = core::ptr::null_mut();
    assert!(vcall!(gpu.device, 0, fn(*const Guid, *mut *mut c_void) -> i32, &IID_DEVICE2, &mut dev2) >= 0, "ID3D12Device2");
    let mut pso: *mut c_void = core::ptr::null_mut();
    let hr = vcall!(dev2, 47, fn(*const u64, *const Guid, *mut *mut c_void) -> i32, sd.as_ptr(), &d3d::IID_PSO, &mut pso);
    if hr < 0 {
        return Err(format!("CreatePipelineState(AS+MS) hr={:#010x}{}\n{}\n{}", hr as u32, gpu.messages(), ta.hlsl, tm.hlsl));
    }
    let mut td = [0u8; 0x38];
    td[0..4].copy_from_slice(&3u32.to_le_bytes());
    td[0x10..0x18].copy_from_slice(&(W as u64).to_le_bytes());
    td[0x18..0x1c].copy_from_slice(&1u32.to_le_bytes());
    td[0x1c..0x1e].copy_from_slice(&1u16.to_le_bytes());
    td[0x1e..0x20].copy_from_slice(&1u16.to_le_bytes());
    td[0x20..0x24].copy_from_slice(&3u32.to_le_bytes());
    td[0x24..0x28].copy_from_slice(&1u32.to_le_bytes());
    td[0x30..0x34].copy_from_slice(&1u32.to_le_bytes());
    let rt = gpu.committed(&td, d3d::default_heap(), 4)?;
    let rb = gpu.buffer(512, 3, d3d::COPY_DEST, false)?;
    let cb = gpu.buffer(256, 2, d3d::GENERIC_READ, false)?;
    let win = gpu.buffer(256, 1, UAV_STATE, true)?;
    let fault = gpu.buffer(256, 1, UAV_STATE, true)?;
    let (_h, rtv, _, _) = gpu.heap(2, 1, false)?;
    vcall!(gpu.device, 20, fn(*mut c_void, *const c_void, usize) -> (), rt, core::ptr::null(), rtv);
    gpu.begin();
    let l = gpu.list;
    let clear = [5.0f32, 6.0, 7.0, 8.0];
    vcall!(l, 48, fn(usize, *const f32, u32, *const c_void) -> (), rtv, clear.as_ptr(), 0, core::ptr::null());
    vcall!(l, 46, fn(u32, *const usize, i32, *const usize) -> (), 1, &rtv, 0, core::ptr::null());
    vcall!(l, 30, fn(*mut c_void) -> (), rs);
    vcall!(l, 38, fn(u32, u64) -> (), 0, Gpu::gpu_va(cb));
    vcall!(l, 42, fn(u32, u64) -> (), 1, Gpu::gpu_va(win));
    vcall!(l, 42, fn(u32, u64) -> (), 2, Gpu::gpu_va(win));
    vcall!(l, 42, fn(u32, u64) -> (), 3, Gpu::gpu_va(fault));
    vcall!(l, 25, fn(*mut c_void) -> (), pso);
    let vp = [0f32, 0., W as f32, 1., 0., 1.];
    vcall!(l, 21, fn(u32, *const f32) -> (), 1, vp.as_ptr());
    let sc = [0i32, 0, W as i32, 1];
    vcall!(l, 22, fn(u32, *const i32) -> (), 1, sc.as_ptr());
    let mut l6: *mut c_void = core::ptr::null_mut();
    assert!(vcall!(l, 0, fn(*const Guid, *mut *mut c_void) -> i32, &IID_LIST6, &mut l6) >= 0, "ID3D12GraphicsCommandList6");
    vcall!(l6, 79, fn(u32, u32, u32) -> (), groups, 1, 1);
    d3d::release(l6);
    gpu.barrier(rt, 4, d3d::COPY_SOURCE);
    gpu.barrier(fault, UAV_STATE, d3d::COPY_SOURCE);
    let mut dst2 = [0u8; 48];
    dst2[0..8].copy_from_slice(&(rb as u64).to_le_bytes());
    dst2[8..12].copy_from_slice(&1u32.to_le_bytes());
    dst2[24..28].copy_from_slice(&3u32.to_le_bytes());
    dst2[28..32].copy_from_slice(&(W as u32).to_le_bytes());
    dst2[32..36].copy_from_slice(&1u32.to_le_bytes());
    dst2[36..40].copy_from_slice(&1u32.to_le_bytes());
    dst2[40..44].copy_from_slice(&256u32.to_le_bytes());
    let mut src = [0u8; 48];
    src[0..8].copy_from_slice(&(rt as u64).to_le_bytes());
    vcall!(l, 16, fn(*const u8, u32, u32, u32, *const u8, *const c_void) -> (), dst2.as_ptr(), 0, 0, 0, src.as_ptr(), core::ptr::null());
    vcall!(l, 15, fn(*mut c_void, u64, *mut c_void, u64, u64) -> (), rb, 256, fault, 0, 16);
    gpu.submit()?;
    let b = Gpu::map_read(rb, 512).ok_or("map readback")?;
    d3d::release(pso);
    d3d::release(dev2);
    let u = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
    let mut px = [[0u32; 4]; W];
    for (i, p) in px.iter_mut().enumerate() {
        for c in 0..4 {
            p[c] = u(i * 16 + c * 4);
        }
    }
    Ok((px, [u(256), u(260), u(264)]))
}

#[test]
#[cfg_attr(not(tombox_fixtures_console_xsc), ignore = "needs analysis/shaders/console_xsc fixtures extracted from local title dumps")]
fn gpu_console_amplification_feeds_payload_and_group_counts_to_the_mesh_stage() {
    let (px, fault) = unsafe { render("as3.dxbc", "ms3.dxbc", 2) }.unwrap_or_else(|e| panic!("{e}"));
    let clear = [5, 6, 7, 8];
    let mut want = [clear; W];
    want[0] = [0x100, 0, 0, 0x5a5a];
    want[4] = [0x101, 0, 1, 0x5a5a];
    want[5] = [0x101, 1, 1, 0x5a5a];
    assert_eq!(fault, [0, 0, 0], "global-memory misses");
    assert_eq!(px, want);
}
