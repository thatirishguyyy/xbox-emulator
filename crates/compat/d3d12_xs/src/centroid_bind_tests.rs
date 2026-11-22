use super::*;
use tombox_xshader::centroid_abi::{pack, CONSOLE_DEFAULT, TB_CENTROID_FLAG_CUSTOM, TB_CENTROID_FLAG_POSITIONS};

fn p(ty: u32, n: u32) -> [u8; 32] {
    let mut v = [0u8; 32];
    v[0..4].copy_from_slice(&ty.to_le_bytes());
    v[16..20].copy_from_slice(&n.to_le_bytes());
    v
}

#[test]
fn centroid_constants_are_appended_when_they_fit() {
    let mut ps = Vec::new();
    ps.extend_from_slice(&p(0, 3));
    ps.extend_from_slice(&p(PARAM_CONSTANTS, 8));
    let (v, idx) = append_plan(&ps, 0).unwrap();
    assert_eq!(idx, 2);
    assert_eq!(&v[64..96], &param());
    assert_eq!(u32_at(&v, 64), PARAM_CONSTANTS);
    assert_eq!(u32_at(&v, 72), TB_CENTROID_REGISTER);
    assert_eq!(u32_at(&v, 76), TB_CENTROID_SPACE);
    assert_eq!(u32_at(&v, 80), TB_CENTROID_ROOT_DWORDS);
    assert_eq!(u32_at(&v, 88), VISIBILITY_PIXEL);
    assert!(append_plan(&ps, ROOT_FLAG_LOCAL).is_err());
    assert!(append_plan(&ps, ROOT_FLAG_DENY_PIXEL).is_err());
    assert!(append_plan(&p(PARAM_CONSTANTS, 64 - TB_CENTROID_ROOT_DWORDS + 1), 0).is_err());
    assert!(append_plan(&p(PARAM_CONSTANTS, 64 - TB_CENTROID_ROOT_DWORDS), 0).is_ok());
    assert_eq!(append_plan(&[], 0).unwrap().1, 0);
}

#[test]
fn psos_without_the_constants_select_the_pc_centroid_variant() {
    note_root_signature(0x7100 as *mut c_void, 1);
    assert_eq!(tag_rtfmt(0x7100, [1, 0, 0, 0, 0, 0, 0, 2]), [1, 0, 0, 0, 0, 0, 0, 2]);
    let t = tag_rtfmt(0x7200, [1, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(t[0], 1 | RTFMT_PC_CENTROID);
    assert_eq!(tag_rtfmt(0, [0; 8])[0], RTFMT_PC_CENTROID);
    assert_eq!(pc_define(&t), format!("#define {TB_CENTROID_PC_DEFINE}\n"));
    assert_eq!(pc_define(&[2; 8]), "");
    assert_ne!(crate::shader_xlat::cache_key(5, &t), crate::shader_xlat::cache_key(5, &[1, 0, 0, 0, 0, 0, 0, 0]));
    forget(0x7100 as *mut c_void);
    assert_eq!(tag_rtfmt(0x7100, [0; 8])[0], RTFMT_PC_CENTROID);
    assert_eq!(root_index(0x7100), None);
}

#[test]
fn default_priority_is_the_console_standard_pattern() {
    use crate::sample_pattern::{SAMPLE_PATTERN, SAMPLE_PATTERN_FIRST};
    assert_eq!(CONSOLE_DEFAULT[0], [0, 0]);
    for li in 1..5usize {
        let e = &SAMPLE_PATTERN[SAMPLE_PATTERN_FIRST[li] as usize + 1];
        assert_eq!([e[5], e[6]], CONSOLE_DEFAULT[li], "{}x standard pattern", 1 << li);
        if li <= 3 {
            for q in [0u32, 0xffff_ffff] {
                let sp = crate::device_ext_a::sample_pattern(1 << li, q).unwrap();
                assert_eq!(pack(&sp.centroid), CONSOLE_DEFAULT[li], "{}x quality {q:#x}", 1 << li);
            }
        }
    }
}

static CALLS: Mutex<Vec<(usize, u32, Vec<u32>)>> = Mutex::new(Vec::new());

unsafe extern "system" fn rec_constants(this: *mut c_void, idx: u32, n: u32, v: *const u32, off: u32) {
    assert_eq!(off, 0);
    let c = unsafe { core::slice::from_raw_parts(v, n as usize) }.to_vec();
    CALLS.lock().unwrap().push((this as usize, idx, c));
}

unsafe extern "system" fn prev_groot(_this: *mut c_void, _rs: *mut c_void) {}

fn calls_for(list: usize) -> Vec<(u32, Vec<u32>)> {
    CALLS.lock().unwrap().iter().filter(|c| c.0 == list).map(|c| (c.1, c.2.clone())).collect()
}

#[test]
fn constants_follow_root_signature_and_msaa_state() {
    let mut vt = vec![0usize; 64];
    vt[PC_SET_GRAPHICS_ROOT_CONSTANTS] = rec_constants as usize;
    let obj = Box::new(vt.as_ptr());
    let list = &*obj as *const *const usize as *mut c_void;
    let xvt: Vec<AtomicUsize> = (0..64).map(|_| AtomicUsize::new(0)).collect();
    xvt[XBOX_SET_GRAPHICS_ROOT_SIGNATURE].store(prev_groot as usize, Ordering::Relaxed);
    install(&xvt);
    assert_eq!(xvt[XBOX_SET_GRAPHICS_ROOT_SIGNATURE].load(Ordering::Relaxed), h_groot as usize);
    let rs = 0x7300usize;
    note_root_signature(rs as *mut c_void, 4);
    unsafe { h_groot(list, 0x7400 as *mut c_void) };
    assert!(calls_for(list as usize).is_empty());
    unsafe { h_groot(list, rs as *mut c_void) };
    let c = calls_for(list as usize);
    assert_eq!(c, vec![(4, vec![0; 12])]);
    let prio = [3u8, 2, 1, 0, 3, 2, 1, 0, 3, 2, 1, 0, 3, 2, 1, 0];
    unsafe { on_msaa(list, Some(prio), None) };
    let last = calls_for(list as usize).pop().unwrap().1;
    assert_eq!(&last[..3], &[0x0123_0123, 0x0123_0123, TB_CENTROID_FLAG_CUSTOM]);
    let mut pos = [0u8; 128];
    pos[0] = (-7i8) as u8;
    pos[1] = 5;
    unsafe { on_msaa(list, None, Some((pos, 4))) };
    let last = calls_for(list as usize).pop().unwrap().1;
    assert_eq!(last[2], TB_CENTROID_FLAG_CUSTOM | TB_CENTROID_FLAG_POSITIONS);
    assert_eq!(last[4] & 0xff, 0x59);
    unsafe { h_groot(list, rs as *mut c_void) };
    assert_eq!(calls_for(list as usize).pop().unwrap().1, last);
    on_close(list);
    unsafe { h_groot(list, rs as *mut c_void) };
    assert_eq!(calls_for(list as usize).pop().unwrap().1, vec![0; 12]);
    on_close(list);
}

fn ps() -> tombox_xshader::ShaderExtract {
    use tombox_xshader::meta::SigElement;
    let words = [0xc800_0000u32, 0xc801_0001, 0xc804_0100, 0xc805_0101, 0x7e04_0280, 0x7e06_0280, 0xf800_180f, 0x0302_0100, 0xbf81_0000];
    let isa: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    let el = |name: &str, sysval: u32, register: u32, mask: u8, rw: u8| SigElement { name: name.into(), index: 0, sysval, comp_type: 3, register, mask, rw_mask: rw };
    tombox_xshader::ShaderExtract {
        stage: tombox_xshader::Stage::Pixel,
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
        input_sig: vec![el("SV_Position", 1, 0, 0xf, 0), el("TEXCOORD_centroid", 0, 1, 0x3, 0x3)],
        output_sig: vec![el("SV_Target", 64, 0, 0xf, 0xf)],
        resources: Vec::new(),
        gs_info: None,
        root_sig: None,
    }
}

extern "system" {
    fn LoadLibraryA(n: *const u8) -> *mut c_void;
    fn GetProcAddress(h: *mut c_void, n: *const u8) -> *mut c_void;
}

type D3DCompile = unsafe extern "system" fn(*const u8, usize, *const u8, *const c_void, *const c_void, *const u8, *const u8, u32, u32, *mut *mut c_void, *mut *mut c_void) -> i32;

fn fxc(src: &str, profile: &str) -> Result<(), String> {
    let h = unsafe { LoadLibraryA(b"d3dcompiler_47.dll\0".as_ptr()) };
    assert!(!h.is_null(), "d3dcompiler_47.dll");
    let f: D3DCompile = unsafe { core::mem::transmute(GetProcAddress(h, b"D3DCompile\0".as_ptr())) };
    let prof = format!("{profile}\0");
    let (mut code, mut err): (*mut c_void, *mut c_void) = (core::ptr::null_mut(), core::ptr::null_mut());
    let hr = unsafe { f(src.as_ptr(), src.len(), core::ptr::null(), core::ptr::null(), core::ptr::null(), b"main\0".as_ptr(), prof.as_ptr(), 0, 0, &mut code, &mut err) };
    let msg = if err.is_null() {
        String::new()
    } else {
        let ptr: unsafe extern "system" fn(*mut c_void) -> *const i8 = unsafe { core::mem::transmute(vslot(err, 3)) };
        unsafe { std::ffi::CStr::from_ptr(ptr(err)) }.to_string_lossy().into_owned()
    };
    unsafe {
        release(code);
        release(err);
    }
    if hr < 0 { Err(format!("hr={hr:#010x} {msg}")) } else { Ok(()) }
}

#[test]
fn lowered_pixel_shader_compiles_with_fxc_in_both_variants() {
    let t = tombox_xshader::hlsl::translate(&ps()).unwrap();
    assert!(t.hlsl.contains("EvaluateAttributeSnapped(i.i1, tb_co)"), "{}", t.hlsl);
    for profile in [t.profile, "ps_5_1"] {
        fxc(&t.hlsl, profile).unwrap_or_else(|e| panic!("{profile}: {e}
{}", t.hlsl));
        fxc(&(pc_define(&[RTFMT_PC_CENTROID, 0, 0, 0, 0, 0, 0, 0]) + &t.hlsl), profile).unwrap_or_else(|e| panic!("{profile} pc: {e}"));
    }
}

#[test]
fn slot80_validation_messages_match_the_debug_umd() {
    let mut bad = [0u8; 128];
    bad[127] = 8;
    assert_eq!(crate::cl_ext::sample_positions_error(&bad, None), Some("Invalid value encountered in D3D11X_MSAA_SAMPLE_POSITIONS. Values should be signed integers in [-8..7] range."));
    assert_eq!(crate::cl_ext::sample_positions_error(&[0; 128], Some([0, 0x2000])), Some("Reserved fields must be zero"));
    assert_eq!(crate::cl_ext::sample_positions_error(&[0; 128], Some([0xc00, 0])), Some("DetailToExposedMode can't take the value of 3"));
    assert_eq!(crate::cl_ext::sample_positions_error(&[0xf8; 128], Some([0x1802, 0x1d000])), None);
}

#[test]
fn host_root_signature_round_trips_with_the_constants() {
    let ser = crate::host_proc(b"D3D12SerializeRootSignature\0");
    assert_ne!(ser, 0);
    let ser: unsafe extern "system" fn(*const u8, u32, *mut *mut c_void, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(ser) };
    let mut prm = p(PARAM_CONSTANTS, 4);
    prm[24..28].copy_from_slice(&0u32.to_le_bytes());
    let mut desc = [0u8; 40];
    desc[0..4].copy_from_slice(&1u32.to_le_bytes());
    desc[8..16].copy_from_slice(&(prm.as_ptr() as u64).to_le_bytes());
    let (mut b, mut e): (*mut c_void, *mut c_void) = (core::ptr::null_mut(), core::ptr::null_mut());
    assert!(unsafe { ser(desc.as_ptr(), 1, &mut b, &mut e) } >= 0);
    let ptr: unsafe extern "system" fn(*mut c_void) -> *const u8 = unsafe { core::mem::transmute(unsafe { vslot(b, 3) }) };
    let size: unsafe extern "system" fn(*mut c_void) -> usize = unsafe { core::mem::transmute(unsafe { vslot(b, 4) }) };
    let blob = unsafe { core::slice::from_raw_parts(ptr(b), size(b)) }.to_vec();
    unsafe { release(b) };
    let (patched, idx) = unsafe { patch_root_signature(blob.as_ptr(), blob.len()) }.unwrap();
    assert_eq!(idx, 1);
    assert!(patched.len() > blob.len());
    let again = unsafe { patch_root_signature(patched.as_ptr(), patched.len()) };
    assert!(again.as_ref().is_err_and(|e| e.contains("overlaps")), "{again:?}");
    desc[32..36].copy_from_slice(&ROOT_FLAG_DENY_PIXEL.to_le_bytes());
    let (mut b, mut e2): (*mut c_void, *mut c_void) = (core::ptr::null_mut(), core::ptr::null_mut());
    assert!(unsafe { ser(desc.as_ptr(), 1, &mut b, &mut e2) } >= 0);
    let blob = unsafe { core::slice::from_raw_parts(ptr(b), size(b)) }.to_vec();
    unsafe { release(b) };
    assert!(unsafe { patch_root_signature(blob.as_ptr(), blob.len()) }.is_err());
    unsafe {
        release(e);
        release(e2);
    }
}
