use super::*;

const T: u32 = 8;
const GROUPS: u32 = 2;
const BYTES: u64 = 4096;
const CONSTS: [u32; 5] = [0x1000_0001, 0x0000_0101, 0xdead_0002, 0xbeef_0003, 0x5a5a_a5a5];

fn input(i: u32) -> u32 {
    i.wrapping_mul(0x9e37_79b9) ^ 0x0bad_f00d
}

fn want(i: u32) -> u32 {
    input(i).wrapping_add(CONSTS[0]).wrapping_add(CONSTS[1].wrapping_mul(i)) ^ CONSTS[4]
}

fn mubuf_op(op: u32, vaddr: u32, vdata: u32, srsrc: u32) -> [u32; 2] {
    [0xe000_0000 | (op << 18) | (1 << 12), vaddr | (vdata << 8) | ((srsrc / 4) << 16) | (0x80 << 24)]
}

fn root(params: Vec<RootParam>) -> RootSig {
    RootSig { version: 2, flags: 0, params, num_static_samplers: 0, static_samplers: Vec::new() }
}

fn desc(kind: u32, register: u32) -> RootParam {
    RootParam { visibility: 0, kind: RootParamKind::Descriptor { kind, register, space: 0 } }
}

fn rootmem_root() -> RootSig {
    root(vec![desc(4, 0), RootParam { visibility: 0, kind: RootParamKind::Constants { register: 0, space: 0, num32: 5 } }, desc(3, 0)])
}

fn prog() -> Vec<u32> {
    let mut p = Prog::new();
    p.raw(&[sop2(0x0e, 3, 1, LIT), 0xff, sop1(3, 2, 0)]);
    p.raw(&smem(1, 8, 2, 0x00, SNULL)).raw(&smem(2, 12, 2, 0x08, SNULL)).raw(&smem(0, 16, 2, 0x18, SNULL)).raw(&smem(1, 10, 2, 0x1c, SNULL));
    p.raw(&[0xbf8c_c07f]);
    p.raw(&[sop1(3, 0, LIT), 0x1101_4fac]);
    p.raw(&[sop1(3, 20, 8), sop2(0x10, 21, 9, LIT), 0x4_0000, sop1(3, 22, ic(-1)), sop1(3, 23, 0)]);
    p.raw(&[sop1(3, 24, 10), sop2(0x10, 25, 11, LIT), 0x4_0000, sop1(3, 26, ic(-1)), sop1(3, 27, 0)]);
    p.raw(&vop3(0x346, 0, [4, ic(3), v(0)]));
    p.raw(&[vop2(0x1a, 1, ic(2), 0)]);
    p.raw(&mubuf_op(0x0c, 1, 2, 24));
    p.raw(&[0xbf8c_3f70]);
    p.raw(&[vop2(0x25, 2, 12, 2)]);
    p.raw(&vop3(0x169, 3, [13, v(0), 0]));
    p.raw(&[vop2(0x25, 2, v(3), 2), vop2(0x1d, 2, 16, 2)]);
    p.raw(&mubuf_op(0x1c, 1, 2, 20));
    let mut w = p.w;
    w.push(S_ENDPGM);
    w
}

fn extract_rm(words: &[u32], root: RootSig) -> ShaderExtract {
    let mut ex = shader_root(words, root);
    ex.stage = Stage::Compute;
    ex.isa_preamble = [0, T, 1, 1];
    ex.output_sig = vec![];
    ex.has_output_sig = false;
    ex.has_input_sig = false;
    ex
}

unsafe fn ser_params(params: &[[u32; 8]]) -> Result<Vec<u8>, String> {
    let d3d = LoadLibraryA(b"d3d12.dll\0".as_ptr());
    let ser: unsafe extern "system" fn(*const u8, u32, *mut *mut c_void, *mut *mut c_void) -> i32 =
        core::mem::transmute(GetProcAddress(d3d, b"D3D12SerializeRootSignature\0".as_ptr()));
    let ps: Vec<u8> = params.iter().flat_map(|p| p.iter().flat_map(|w| w.to_le_bytes())).collect();
    let mut desc = [0u8; 40];
    desc[0..4].copy_from_slice(&(params.len() as u32).to_le_bytes());
    desc[8..16].copy_from_slice(&(ps.as_ptr() as u64).to_le_bytes());
    let mut blob: *mut c_void = core::ptr::null_mut();
    let mut err: *mut c_void = core::ptr::null_mut();
    if ser(desc.as_ptr(), 1, &mut blob, &mut err) < 0 {
        let msg = if err.is_null() { String::new() } else { std::ffi::CStr::from_ptr(vcall!(err, 3, fn() -> *const i8)).to_string_lossy().into_owned() };
        return Err(format!("serialize root signature: {msg}"));
    }
    let p = vcall!(blob, 3, fn() -> *const u8);
    let n = vcall!(blob, 4, fn() -> usize);
    Ok(core::slice::from_raw_parts(p, n).to_vec())
}

unsafe fn run(c: &mut Ctx, code: &[u8]) -> Result<Vec<u32>, String> {
    let g = &c.gpu;
    let blob = ser_params(&[[4, 0, 0, 0, 0, 0, 0, 0], [1, 0, 0, 0, 5, 0, 0, 0], [3, 0, 0, 0, 0, 0, 0, 0]])?;
    let mut rs: *mut c_void = core::ptr::null_mut();
    let hr = vcall!(g.device, 16, fn(u32, *const u8, usize, *const Guid, *mut *mut c_void) -> i32, 0, blob.as_ptr(), blob.len(), &d3d::IID_ROOTSIG, &mut rs);
    if hr < 0 {
        return Err(format!("CreateRootSignature {hr:#x}"));
    }
    let mut d = [0u8; 56];
    d[0..8].copy_from_slice(&(rs as u64).to_le_bytes());
    d[8..16].copy_from_slice(&(code.as_ptr() as u64).to_le_bytes());
    d[16..24].copy_from_slice(&(code.len() as u64).to_le_bytes());
    let mut pso: *mut c_void = core::ptr::null_mut();
    let hr = vcall!(g.device, 11, fn(*const u8, *const Guid, *mut *mut c_void) -> i32, d.as_ptr(), &d3d::IID_PSO, &mut pso);
    if hr < 0 {
        d3d::release(rs);
        return Err(format!("CreateComputePipelineState hr={:#010x}{}", hr as u32, g.messages()));
    }
    let ubuf = g.buffer(BYTES, 1, d3d::COPY_DEST, true)?;
    let sbuf = g.buffer(BYTES, 1, d3d::COPY_DEST, false)?;
    let up = g.buffer(2 * BYTES, 2, d3d::GENERIC_READ, false)?;
    let rb = g.buffer(BYTES, 3, d3d::COPY_DEST, false)?;
    let mut bytes = vec![0u8; 2 * BYTES as usize];
    for i in 0..(BYTES / 4) as u32 {
        let o = BYTES as usize + 4 * i as usize;
        bytes[o..o + 4].copy_from_slice(&input(i).to_le_bytes());
    }
    Gpu::map_write(up, &bytes);
    c.gpu.begin();
    let l = c.gpu.list;
    vcall!(l, 15, fn(*mut c_void, u64, *mut c_void, u64, u64) -> (), ubuf, 0, up, 0, BYTES);
    vcall!(l, 15, fn(*mut c_void, u64, *mut c_void, u64, u64) -> (), sbuf, 0, up, BYTES, BYTES);
    c.gpu.barrier(ubuf, d3d::COPY_DEST, 8);
    c.gpu.barrier(sbuf, d3d::COPY_DEST, 0x40);
    vcall!(l, 29, fn(*mut c_void) -> (), rs);
    vcall!(l, 41, fn(u32, u64) -> (), 0, Gpu::gpu_va(ubuf));
    vcall!(l, 35, fn(u32, u32, *const u32, u32) -> (), 1, 5, CONSTS.as_ptr(), 0);
    vcall!(l, 39, fn(u32, u64) -> (), 2, Gpu::gpu_va(sbuf));
    vcall!(l, 25, fn(*mut c_void) -> (), pso);
    vcall!(l, 14, fn(u32, u32, u32) -> (), GROUPS, 1, 1);
    c.gpu.barrier(ubuf, 8, d3d::COPY_SOURCE);
    vcall!(l, 15, fn(*mut c_void, u64, *mut c_void, u64, u64) -> (), rb, 0, ubuf, 0, BYTES);
    let r = c.gpu.submit();
    let out = r.and_then(|_| Gpu::map_read(rb, BYTES as usize).ok_or_else(|| "map".to_string())).map(|b| b.chunks(4).map(|x| u32::from_le_bytes(x.try_into().unwrap())).collect());
    for k in [pso, rs, ubuf, sbuf, up, rb] {
        d3d::release(k);
    }
    out
}

#[test]
fn rootmem_descriptors_and_constants() {
    let t = translate(&extract_rm(&prog(), rootmem_root())).unwrap_or_else(|e| panic!("translate: {e}"));
    for need in ["register(u0)", "register(t0)", "cbuffer TB_RC1 : register(b0, space0)", "s4 = gid.x"] {
        assert!(t.hlsl.contains(need), "missing {need}\n{}", t.hlsl);
    }
    let code = dxc::compile(&t.hlsl, &dxc::profile_for(t.profile)).unwrap_or_else(|e| panic!("dxc: {e}\n{}", t.hlsl));
    let m = CTX.get_or_init(|| Mutex::new(unsafe { ctx_new() }));
    let mut c = m.lock().unwrap_or_else(|e| e.into_inner());
    let out = unsafe { run(&mut c, &code) }.unwrap_or_else(|e| panic!("{e}\n{}", t.hlsl));
    let wrong: Vec<String> = (0..T * GROUPS).filter(|&i| out[i as usize] != want(i)).map(|i| format!("[{i}] {:08x} want {:08x}", out[i as usize], want(i))).collect();
    assert!(wrong.is_empty(), "{}\n{}", wrong.join(", "), t.hlsl);
    assert!(out[(T * GROUPS) as usize..].iter().all(|&w| w == 0));
}

#[test]
fn rootmem_refuses_unresolvable() {
    let table = RootParam { visibility: 0, kind: RootParamKind::Table(vec![RootRange { range_type: 1, num: 1, base_register: 0, space: 0, offset_in_table: u32::MAX }]) };
    let r = root(vec![table, desc(4, 0), RootParam { visibility: 0, kind: RootParamKind::Constants { register: 0, space: 0, num32: 5 } }, desc(3, 0)]);
    match translate(&extract_rm(&prog(), r)) {
        Ok(t) => panic!("translated a root-memory read past a descriptor table\n{}", t.hlsl),
        Err(e) => assert!(e.contains("root"), "{e}"),
    }
    let short = root(vec![desc(4, 0), RootParam { visibility: 0, kind: RootParamKind::Constants { register: 0, space: 0, num32: 2 } }]);
    match translate(&extract_rm(&prog(), short)) {
        Ok(t) => panic!("translated a root-memory read outside the root signature\n{}", t.hlsl),
        Err(e) => assert!(e.contains("outside the root signature"), "{e}"),
    }
}

#[test]
fn rootmem_xdxr_blobs() {
    let Some(dir) = std::env::var_os("TOMBOX_XDXR_DIR") else {
        println!("TOMBOX_XDXR_DIR not set: skipped");
        return;
    };
    let mut done = 0;
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) != Some("dxbc") {
            continue;
        }
        let Ok(ex) = tombox_xshader::extract(&std::fs::read(&p).unwrap()) else { continue };
        let Some(r) = ex.root_sig.as_ref() else { continue };
        if ex.stage != Stage::Compute || r.params.iter().any(|x| matches!(x.kind, RootParamKind::Table(_))) {
            continue;
        }
        match translate(&ex) {
            Ok(t) => {
                dxc::compile(&t.hlsl, &dxc::profile_for(t.profile)).unwrap_or_else(|e| panic!("{}: dxc: {e}", p.display()));
                println!("{}: ok, numthreads {:?}", p.display(), &ex.isa_preamble[1..]);
                done += 1;
            }
            Err(e) => println!("{}: {e}", p.display()),
        }
    }
    println!("{done} root-memory shaders translated and compiled");
}
