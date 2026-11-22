use tombox_xshader::hlsl::translate;
use tombox_xshader::meta::{RootParam, RootParamKind, RootRange, RootSig, SigElement};
use tombox_xshader::{ShaderExtract, Stage};

fn shader(words: &[u32], root: RootSig) -> ShaderExtract {
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
        has_input_sig: true,
        has_output_sig: true,
        has_psv0: false,
        has_root_sig: true,
        input_sig: vec![],
        output_sig: vec![SigElement { name: "SV_Target".into(), index: 0, sysval: 64, comp_type: 3, register: 0, mask: 0xf, rw_mask: 0 }],
        resources: Vec::new(),
        gs_info: None,
        root_sig: Some(root),
    }
}

fn uav_then_srv() -> RootSig {
    let r = |range_type: u32| RootRange { range_type, num: 1, base_register: 0, space: 0, offset_in_table: u32::MAX };
    RootSig {
        version: 2,
        flags: 0,
        params: vec![RootParam { visibility: 0, kind: RootParamKind::Table(vec![r(1), r(0)]) }],
        num_static_samplers: 0,
        static_samplers: Vec::new(),
    }
}

fn compile(label: &str, hlsl: &str, profile: &str) {
    #[cfg(windows)]
    {
        assert!(tombox_xshader::dxc::available(), "{label}: DXC (dxcompiler.dll + dxil.dll) not found");
        let profile = tombox_xshader::dxc::profile_for(profile);
        let code = tombox_xshader::dxc::compile(hlsl, &profile).unwrap_or_else(|e| panic!("{label}: dxc: {e}\n{hlsl}"));
        assert!(code.len() > 4 && &code[..4] == b"DXBC", "{label}: DXC returned no container");
    }
    #[cfg(not(windows))]
    let _ = (label, hlsl, profile);
}

const HEAD: [u32; 4] = [0xf408_0104, 0xfa00_0000, 0xf40c_0304, 0xfa00_0020];
const BUF_LOAD_V4: [u32; 2] = [0xe030_1000, 0x8001_0401];
const BUF_STORE_V8: [u32; 2] = [0xe070_1000, 0x8001_0801];
const IMG_LOAD_V8: [u32; 2] = [0xf000_0f08, 0x0003_0801];
const TAIL: [u32; 4] = [0x7e00_0304, 0xf800_180f, 0x0302_0100, 0xbf81_0000];

fn program(parts: &[&[u32]]) -> Vec<u32> {
    let mut w: Vec<u32> = HEAD.to_vec();
    for p in parts {
        w.extend_from_slice(p);
    }
    w.extend_from_slice(&TAIL);
    w
}

#[test]
fn read_written_buffer_and_texture_bind_by_range() {
    let words = program(&[&BUF_LOAD_V4, &IMG_LOAD_V8, &BUF_STORE_V8]);
    let t = translate(&shader(&words, uav_then_srv())).unwrap_or_else(|e| panic!("translate failed: {e}"));
    assert!(t.hlsl.contains("RWStructuredBuffer<TbElem_ubuf0> ubuf0 : register(u0);"), "{}", t.hlsl);
    assert!(t.hlsl.contains("Texture2D<float4> tex0 : register(t0);"), "{}", t.hlsl);
    assert!(!t.hlsl.contains("sbuf0"), "load of the UAV-range buffer bound as an SRV:\n{}", t.hlsl);
    compile("rw buffer + texture", &t.hlsl, t.profile);
}

#[test]
fn read_only_buffer_in_uav_range_binds_as_uav() {
    let words = program(&[&BUF_LOAD_V4, &IMG_LOAD_V8]);
    let t = translate(&shader(&words, uav_then_srv())).unwrap_or_else(|e| panic!("translate failed: {e}"));
    assert!(t.hlsl.contains("RWStructuredBuffer<TbElem_ubuf0> ubuf0 : register(u0);"), "{}", t.hlsl);
    assert!(t.hlsl.contains("Texture2D<float4> tex0 : register(t0);"), "{}", t.hlsl);
    compile("read-only buffer + texture", &t.hlsl, t.profile);
}

#[test]
fn store_to_srv_range_is_an_error() {
    let words = vec![0xf408_0104, 0xfa00_0020, 0xe070_1000, 0x8001_0801, 0xbf81_0000];
    let r = translate(&shader(&words, uav_then_srv()));
    assert!(matches!(&r, Err(e) if e.contains("SRV range")), "{:?}", r.map(|t| t.hlsl));
}
