//! HLSL → DXIL through Microsoft's `dxcompiler.dll` (Windows only).
//!
//! Why DXC rather than FXC: the HLSL this crate generates (flag-based forward branches,
//! guarded writes, loops) is pathological for FXC — minutes for some compute shaders — and
//! compiles in about a second with DXC. DXC output is DXIL (shader model 6.0), which D3D12
//! accepts when it has been validated and signed; `dxil.dll` next to `dxcompiler.dll` does
//! that as part of the compile.
//!
//! The two DLLs are looked for in: `%TOMBOX_DXC_DIR%`, the directory of the running
//! executable, then the newest `Windows Kits\10\bin\<version>\x64`.

use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::OnceLock;

extern "system" {
    fn LoadLibraryW(n: *const u16) -> *mut c_void;
    fn GetProcAddress(h: *mut c_void, n: *const u8) -> *mut c_void;
}

#[repr(C)]
struct Guid(u32, u16, u16, [u8; 8]);
const CLSID_DXC_COMPILER: Guid = Guid(0x73e22d93, 0xe6ce, 0x47f3, [0xb5, 0xbf, 0xf0, 0x66, 0x4f, 0x39, 0xc1, 0xb0]);
const IID_IDXC_COMPILER3: Guid = Guid(0x228b4687, 0x5a6a, 0x4730, [0x90, 0x0c, 0x97, 0x02, 0xb2, 0x20, 0x3f, 0x54]);
const IID_IDXC_RESULT: Guid = Guid(0x58346cda, 0xdde7, 0x4497, [0x94, 0x61, 0x6f, 0x87, 0xaf, 0x5e, 0x06, 0x59]);

#[repr(C)]
struct DxcBuffer {
    ptr: *const c_void,
    size: usize,
    encoding: u32,
}

type CreateInstance = unsafe extern "system" fn(*const Guid, *const Guid, *mut *mut c_void) -> i32;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn candidate_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Ok(d) = std::env::var("TOMBOX_DXC_DIR") {
        v.push(PathBuf::from(d));
    }
    if let Some(d) = std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.to_path_buf())) {
        v.push(d);
    }
    let kits = PathBuf::from(r"C:\Program Files (x86)\Windows Kits\10\bin");
    if let Ok(rd) = std::fs::read_dir(&kits) {
        let mut vers: Vec<PathBuf> = rd.flatten().map(|e| e.path().join("x64")).filter(|p| p.join("dxcompiler.dll").exists()).collect();
        vers.sort();
        vers.reverse();
        v.extend(vers);
    }
    v
}

/// `DxcCreateInstance`, or why it is unavailable.
fn create_instance() -> Result<CreateInstance, String> {
    static F: OnceLock<Result<usize, String>> = OnceLock::new();
    F.get_or_init(|| {
        for dir in candidate_dirs() {
            let (comp, dxil) = (dir.join("dxcompiler.dll"), dir.join("dxil.dll"));
            if !comp.exists() || !dxil.exists() {
                continue;
            }
            unsafe {
                // dxil.dll first, by full path, so the compiler finds the validator/signer.
                let _ = LoadLibraryW(wide(&dxil.to_string_lossy()).as_ptr());
                let h = LoadLibraryW(wide(&comp.to_string_lossy()).as_ptr());
                if h.is_null() {
                    continue;
                }
                let p = GetProcAddress(h, b"DxcCreateInstance\0".as_ptr());
                if !p.is_null() {
                    return Ok(p as usize);
                }
            }
        }
        Err("dxcompiler.dll + dxil.dll not found (set TOMBOX_DXC_DIR or install the Windows SDK)".to_string())
    })
    .clone()
    .map(|p| unsafe { std::mem::transmute::<usize, CreateInstance>(p) })
}

pub fn available() -> bool {
    create_instance().is_ok()
}

unsafe fn vt(obj: *mut c_void) -> *const usize {
    *(obj as *const *const usize)
}
unsafe fn release(obj: *mut c_void) {
    if !obj.is_null() {
        let f: unsafe extern "system" fn(*mut c_void) -> u32 = std::mem::transmute(*vt(obj).add(2));
        f(obj);
    }
}
unsafe fn blob_bytes(blob: *mut c_void) -> Vec<u8> {
    if blob.is_null() {
        return Vec::new();
    }
    let ptr: unsafe extern "system" fn(*mut c_void) -> *const u8 = std::mem::transmute(*vt(blob).add(3));
    let len: unsafe extern "system" fn(*mut c_void) -> usize = std::mem::transmute(*vt(blob).add(4));
    let (p, n) = (ptr(blob), len(blob));
    if p.is_null() || n == 0 { Vec::new() } else { std::slice::from_raw_parts(p, n).to_vec() }
}

pub fn profile_for(fxc_profile: &str) -> String {
    let mut it = fxc_profile.split('_');
    let stage = it.next().unwrap_or("ps");
    let minor = if it.next() == Some("6") { it.next().and_then(|m| m.parse::<u32>().ok()).unwrap_or(0) } else { 0 };
    format!("{stage}_6_{}", minor.max(2))
}

/// Compile `hlsl` (entry `main`) to signed DXIL. `profile` is e.g. `cs_6_0`.
pub fn compile(hlsl: &str, profile: &str) -> Result<Vec<u8>, String> {
    compile_with(hlsl, profile, &[])
}

pub fn compile_with(hlsl: &str, profile: &str, extra: &[&str]) -> Result<Vec<u8>, String> {
    let minor = profile.rsplit('_').next().and_then(|m| m.parse::<u32>().ok()).unwrap_or(0);
    if minor < 2 || extra.contains(&"-denorm") {
        return compile_args(hlsl, profile, &[], extra);
    }
    let bf = branch_free_source(hlsl);
    let code = compile_bf(hlsl, bf.as_deref(), profile, &["-denorm", "preserve"], extra)?;
    if profile.starts_with("cs_") && disassemble(&code).map(|t| has_preserve_unselectable(&t)).unwrap_or(false) {
        return compile_bf(hlsl, bf.as_deref(), profile, &["-denorm", "any"], extra);
    }
    Ok(code)
}

pub fn fma_defines(fma: crate::shader_cache::Fma, any_mode: bool) -> Vec<&'static str> {
    let mut d = Vec::new();
    if if any_mode { fma.any } else { fma.preserve } {
        d.extend(["-D", "TB_FAST_FMA"]);
    } else if any_mode && fma.hybrid {
        d.extend(["-D", "TB_HYBRID_FMA"]);
    }
    if fma.f2h {
        d.extend(["-D", "TB_FAST_F2H", "-enable-16bit-types"]);
    }
    d
}

pub fn compile_fma(hlsl: &str, profile: &str, fma: crate::shader_cache::Fma, extra: &[&str]) -> Result<Vec<u8>, String> {
    let minor = profile.rsplit('_').next().and_then(|m| m.parse::<u32>().ok()).unwrap_or(0);
    let fma = crate::shader_cache::Fma { f2h: fma.f2h && minor >= 2, ..fma };
    let with = |any_mode: bool| -> Vec<&str> { extra.iter().copied().chain(fma_defines(fma, any_mode)).collect() };
    let bf = branch_free_source(hlsl);
    if minor < 2 {
        return compile_bf(hlsl, bf.as_deref(), profile, &[], &with(true));
    }
    let early = (profile.starts_with("cs_") && cs_parallel_enabled() && may_need_denorm_any(hlsl)).then(|| {
        let (h, b, p) = (hlsl.to_string(), bf.clone(), profile.to_string());
        let args: Vec<String> = with(true).into_iter().map(String::from).collect();
        std::thread::Builder::new()
            .stack_size(64 << 20)
            .spawn(move || {
                let a: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
                compile_bf(&h, b.as_deref(), &p, &["-denorm", "any"], &a)
            })
            .ok()
    }).flatten();
    let code = compile_bf(hlsl, bf.as_deref(), profile, &["-denorm", "preserve"], &with(false))?;
    if profile.starts_with("cs_") && disassemble(&code).map(|t| has_preserve_unselectable(&t)).unwrap_or(false) {
        if let Some(r) = early.and_then(|t| t.join().ok()) {
            return r;
        }
        return compile_bf(hlsl, bf.as_deref(), profile, &["-denorm", "any"], &with(true));
    }
    Ok(code)
}

pub fn cs_parallel_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("TOMBOX_DXC_CS_PARALLEL").map(|v| v != "0").unwrap_or(true))
}

pub fn may_need_denorm_any(hlsl: &str) -> bool {
    let r = crate::shader_cache::reduced(hlsl);
    let b = r.as_bytes();
    ["log", "log2", "log10", "sqrt", "pow", "length", "distance", "refract"].iter().any(|n| {
        r.match_indices(n).any(|(i, _)| {
            let before = i.checked_sub(1).map(|k| b[k]).unwrap_or(b' ');
            let after = &r[i + n.len()..];
            !(before.is_ascii_alphanumeric() || before == b'_' || before == b'.') && after.trim_start().starts_with('(')
        })
    })
}

pub fn branch_free_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("TOMBOX_DXC_BRANCH_FREE").map(|v| v != "0").unwrap_or(true))
}

fn branch_free_source(hlsl: &str) -> Option<String> {
    if !branch_free_enabled() {
        return None;
    }
    let r = crate::branch_free::rewrite(hlsl);
    (r.len() != hlsl.len() + crate::branch_free::PRELUDE.len() || r[crate::branch_free::PRELUDE.len()..] != *hlsl).then_some(r)
}

fn compile_bf(hlsl: &str, bf: Option<&str>, profile: &str, denorm: &[&str], extra: &[&str]) -> Result<Vec<u8>, String> {
    if let Some(src) = bf {
        match compile_args(src, profile, denorm, extra) {
            Ok(c) => return Ok(c),
            Err(e) => {
                static WARNED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
                if WARNED.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 8 {
                    eprintln!("dxc: branch-free source failed, compiling the original: {}", e.lines().next().unwrap_or(""));
                }
            }
        }
    }
    compile_args(hlsl, profile, denorm, extra)
}

pub fn compile_denorm(hlsl: &str, profile: &str, denorm: &str, extra: &[&str]) -> Result<Vec<u8>, String> {
    compile_args(hlsl, profile, &["-denorm", denorm], extra)
}

pub fn identity() -> &'static str {
    static I: OnceLock<String> = OnceLock::new();
    I.get_or_init(|| {
        for dir in candidate_dirs() {
            let (comp, dxil) = (dir.join("dxcompiler.dll"), dir.join("dxil.dll"));
            if let (Ok(a), Ok(b)) = (std::fs::metadata(&comp), std::fs::metadata(&dxil)) {
                return format!("dxcompiler {} dxil {}", a.len(), b.len());
            }
        }
        String::from("none")
    })
}

pub const COMPILE_FLAVOR: &str = "denorm-preserve; cs log/sqrt any";

pub fn has_preserve_unselectable(disasm: &str) -> bool {
    disasm.contains("@dx.op.unary.f32(i32 23,") || disasm.contains("@dx.op.unary.f32(i32 24,")
}

pub fn disassemble(code: &[u8]) -> Result<String, String> {
    let create = create_instance()?;
    unsafe {
        let mut compiler: *mut c_void = std::ptr::null_mut();
        let hr = create(&CLSID_DXC_COMPILER, &IID_IDXC_COMPILER3, &mut compiler);
        if hr < 0 || compiler.is_null() {
            return Err(format!("dxc: DxcCreateInstance(IDxcCompiler3) hr={hr:#010x}"));
        }
        let buf = DxcBuffer { ptr: code.as_ptr() as *const c_void, size: code.len(), encoding: 0 };
        let f: unsafe extern "system" fn(*mut c_void, *const DxcBuffer, *const Guid, *mut *mut c_void) -> i32 = std::mem::transmute(*vt(compiler).add(4));
        let mut result: *mut c_void = std::ptr::null_mut();
        let hr = f(compiler, &buf, &IID_IDXC_RESULT, &mut result);
        release(compiler);
        if hr < 0 || result.is_null() {
            return Err(format!("dxc: Disassemble hr={hr:#010x}"));
        }
        let mut status = 0i32;
        let get_status: unsafe extern "system" fn(*mut c_void, *mut i32) -> i32 = std::mem::transmute(*vt(result).add(3));
        get_status(result, &mut status);
        let get_result: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32 = std::mem::transmute(*vt(result).add(4));
        let mut b: *mut c_void = std::ptr::null_mut();
        get_result(result, &mut b);
        let text = String::from_utf8_lossy(&blob_bytes(b)).into_owned();
        release(b);
        release(result);
        if status < 0 || text.is_empty() {
            return Err(format!("dxc: Disassemble status={status:#010x}"));
        }
        Ok(text)
    }
}

fn compile_args(hlsl: &str, profile: &str, denorm: &[&str], extra: &[&str]) -> Result<Vec<u8>, String> {
    let create = create_instance()?;
    unsafe {
        let mut compiler: *mut c_void = std::ptr::null_mut();
        let hr = create(&CLSID_DXC_COMPILER, &IID_IDXC_COMPILER3, &mut compiler);
        if hr < 0 || compiler.is_null() {
            return Err(format!("dxc: DxcCreateInstance(IDxcCompiler3) hr={hr:#010x}"));
        }
        let args_w: Vec<Vec<u16>> = ["-T", profile, "-E", "main"].iter().chain(denorm.iter()).chain(extra.iter()).map(|a| wide(a)).collect();
        let args: Vec<*const u16> = args_w.iter().map(|a| a.as_ptr()).collect();
        let src = DxcBuffer { ptr: hlsl.as_ptr() as *const c_void, size: hlsl.len(), encoding: 65001 };
        let mut result: *mut c_void = std::ptr::null_mut();
        // IDxcCompiler3::Compile (slot 3)
        let f: unsafe extern "system" fn(*mut c_void, *const DxcBuffer, *const *const u16, u32, *mut c_void, *const Guid, *mut *mut c_void) -> i32 =
            std::mem::transmute(*vt(compiler).add(3));
        let hr = f(compiler, &src, args.as_ptr(), args.len() as u32, std::ptr::null_mut(), &IID_IDXC_RESULT, &mut result);
        release(compiler);
        if hr < 0 || result.is_null() {
            return Err(format!("dxc: Compile hr={hr:#010x}"));
        }
        // IDxcOperationResult: 3 GetStatus, 4 GetResult, 5 GetErrorBuffer.
        let mut status = 0i32;
        let get_status: unsafe extern "system" fn(*mut c_void, *mut i32) -> i32 = std::mem::transmute(*vt(result).add(3));
        get_status(result, &mut status);
        let get_blob = |slot: usize| -> *mut c_void {
            let g: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32 = std::mem::transmute(*vt(result).add(slot));
            let mut b: *mut c_void = std::ptr::null_mut();
            g(result, &mut b);
            b
        };
        let out = if status >= 0 {
            let b = get_blob(4);
            let v = blob_bytes(b);
            release(b);
            if v.is_empty() { Err("dxc: empty result".to_string()) } else { Ok(v) }
        } else {
            let b = get_blob(5);
            let text = String::from_utf8_lossy(&blob_bytes(b)).into_owned();
            release(b);
            let line = text.lines().find(|l| l.contains("error")).unwrap_or(text.lines().next().unwrap_or("")).to_string();
            // "hlsl.hlsl:12:5: error: message" -> "message"
            Err(format!("dxc: {}", line.split("error: ").nth(1).unwrap_or(&line)))
        };
        release(result);
        out
    }
}

/// Location of a compiled shader in the on-disk cache shared by the offline sweep
/// (`xscan --compile`) and the runtime (`d3d12_xs`). Keyed by the generated HLSL and the
/// profile, so any change to a shader's translation misses the cache.
pub fn cache_path(hlsl: &str, profile: &str) -> PathBuf {
    cache_file(cache_key(hlsl, profile))
}

fn fnv1a(b: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for &x in b {
        h ^= x as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn cache_key(hlsl: &str, profile: &str) -> u64 {
    fnv1a(hlsl.as_bytes()) ^ fnv1a(profile.as_bytes()).rotate_left(17)
}

fn cache_file(key: u64) -> PathBuf {
    std::env::temp_dir().join("tombox_shader_cache").join("by_hlsl").join(format!("{key:016x}.cso"))
}

fn flavored_cache_path(hlsl: &str, profile: &str) -> PathBuf {
    cache_file(cache_key(hlsl, profile) ^ fnv1a(COMPILE_FLAVOR.as_bytes()).rotate_left(31))
}

fn flags_differ_for(profile: &str) -> bool {
    profile.starts_with("cs_") && profile.rsplit('_').next().and_then(|m| m.parse::<u32>().ok()).unwrap_or(0) >= 2
}

pub fn cached_matches_flags(code: &[u8], profile: &str) -> bool {
    if !flags_differ_for(profile) {
        return true;
    }
    match disassemble(code) {
        Ok(t) => !(has_preserve_unselectable(&t) && t.contains("\"fp32-denorm-mode\"=\"preserve\"")),
        Err(_) => false,
    }
}

pub fn cache_load(hlsl: &str, profile: &str) -> Option<Vec<u8>> {
    let p = cache_path(hlsl, profile);
    if let Ok(v) = std::fs::read(&p) {
        if !v.is_empty() && cached_matches_flags(&v, profile) {
            return Some(v);
        }
    }
    let v = std::fs::read(flavored_cache_path(hlsl, profile)).ok().filter(|v| !v.is_empty())?;
    let _ = std::fs::write(&p, &v);
    Some(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PRESERVE: &str = "\"fp32-denorm-mode\"=\"preserve\"";

    fn cs(body: &str) -> String {
        format!("RWTexture2D<float4> u : register(u0);\n[numthreads(8, 8, 1)]\nvoid main(uint3 t : SV_DispatchThreadID) {{ float x = asfloat(t.x); float y = asfloat(t.y); u[t.xy] = float4({body}, 0, 0, 0); }}\n")
    }

    fn denorm_of(src: &str, profile: &str) -> (bool, String) {
        let code = compile(src, profile).expect("dxc compile");
        let text = disassemble(&code).expect("dxc disassemble");
        (text.contains(PRESERVE), text)
    }

    #[test]
    fn compute_log_and_sqrt_leave_denorm_preserve() {
        for body in ["log2(x)", "sqrt(x)", "pow(x, y)", "log(abs(x)) + y"] {
            let (preserve, text) = denorm_of(&cs(body), "cs_6_2");
            assert!(has_preserve_unselectable(&text), "{body}: expected a Log/Sqrt op");
            assert!(!preserve, "{body}: compute shader with Log/Sqrt still compiled with fp32 denorm preserve");
        }
    }

    #[test]
    fn denorm_any_prediction_follows_reachable_intrinsics() {
        assert!(may_need_denorm_any(&cs("log2(x)")));
        assert!(may_need_denorm_any(&cs("length(float2(x, y))")));
        assert!(!may_need_denorm_any(&cs("exp2(x) * rsqrt(y)")));
        assert!(!may_need_denorm_any(&cs("tb_log_x + x.length")));
    }

    #[test]
    fn parallel_compute_compile_matches_the_sequential_flow_byte_for_byte() {
        let fma = crate::shader_cache::Fma { preserve: true, any: false, hybrid: false, f2h: false };
        for body in ["log2(x)", "sqrt(x) + y", "pow(x, y)", "length(float2(x, y))", "exp2(x) * rsqrt(y)"] {
            let src = cs(body);
            let par = compile_fma(&src, "cs_6_2", fma, &[]).expect("parallel flow");
            let pre = compile_args(&src, "cs_6_2", &["-denorm", "preserve"], &fma_defines(fma, false)).expect("preserve");
            let seq = if disassemble(&pre).map(|t| has_preserve_unselectable(&t)).unwrap_or(false) {
                compile_args(&src, "cs_6_2", &["-denorm", "any"], &fma_defines(fma, true)).expect("any")
            } else {
                pre
            };
            let bf = branch_free_source(&src);
            let seq_bf = match &bf {
                Some(b) => {
                    let pre = compile_args(b, "cs_6_2", &["-denorm", "preserve"], &fma_defines(fma, false)).expect("preserve bf");
                    if disassemble(&pre).map(|t| has_preserve_unselectable(&t)).unwrap_or(false) {
                        compile_args(b, "cs_6_2", &["-denorm", "any"], &fma_defines(fma, true)).expect("any bf")
                    } else {
                        pre
                    }
                }
                None => seq,
            };
            assert!(par == seq_bf, "{body}: parallel compute compile differs from the sequential flow");
        }
    }

    #[test]
    fn other_shaders_keep_denorm_preserve() {
        let (preserve, text) = denorm_of(&cs("exp2(x) * rsqrt(y)"), "cs_6_2");
        assert!(!has_preserve_unselectable(&text));
        assert!(preserve, "compute shader without Log/Sqrt lost fp32 denorm preserve");
        let ps = "float4 main(float4 p : SV_Position) : SV_Target { return float4(log2(p.x), sqrt(p.y), 0, 1); }";
        let (preserve, text) = denorm_of(ps, "ps_6_2");
        assert!(has_preserve_unselectable(&text));
        assert!(preserve, "pixel shader lost fp32 denorm preserve");
        let (preserve, _) = denorm_of(&cs("log2(x)"), "cs_6_0");
        assert!(!preserve, "shader model 6.0 has no denorm mode");
    }

    #[test]
    fn explicit_denorm_mode_is_respected() {
        let code = compile_with(&cs("log2(x)"), "cs_6_2", &["-denorm", "ftz"]).expect("dxc compile");
        let text = disassemble(&code).expect("dxc disassemble");
        assert!(text.contains("\"fp32-denorm-mode\"=\"ftz\""));
    }

    #[test]
    fn cache_key_unchanged_for_shaders_whose_flags_did_not_change() {
        let p = cache_path("float4 main() : SV_Target { return 0; }", "ps_6_2");
        let name = p.file_stem().unwrap().to_string_lossy().into_owned();
        let k = u64::from_str_radix(&name, 16).unwrap();
        let f = |b: &[u8]| b.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &x| (h ^ x as u64).wrapping_mul(0x0000_0100_0000_01b3));
        let plain = f(b"float4 main() : SV_Target { return 0; }") ^ f(b"ps_6_2").rotate_left(17);
        assert_eq!(k, plain);
    }

    #[test]
    fn stale_compute_entries_compiled_with_preserve_are_rejected() {
        let src = cs("log2(x)");
        let stale = compile_args(&src, "cs_6_2", &["-denorm", "preserve"], &[]).expect("dxc compile");
        assert!(!cached_matches_flags(&stale, "cs_6_2"));
        let fresh = compile(&src, "cs_6_2").expect("dxc compile");
        assert!(cached_matches_flags(&fresh, "cs_6_2"));
        let plain = compile(&cs("exp2(x)"), "cs_6_2").expect("dxc compile");
        assert!(cached_matches_flags(&plain, "cs_6_2"));
        assert!(cached_matches_flags(&stale, "ps_6_2"));
        let tag = format!("float4 main() : SV_Target {{ return {}; }}", std::process::id() as f32 + 0.25);
        let p = cache_path(&tag, "cs_6_2");
        let _ = std::fs::create_dir_all(p.parent().unwrap());
        std::fs::write(&p, &stale).unwrap();
        assert!(cache_load(&tag, "cs_6_2").is_none());
        std::fs::write(&p, &fresh).unwrap();
        assert_eq!(cache_load(&tag, "cs_6_2").as_deref(), Some(&fresh[..]));
        let _ = std::fs::remove_file(&p);
        let q = flavored_cache_path(&tag, "ps_6_2");
        std::fs::write(&q, &plain).unwrap();
        assert_eq!(cache_load(&tag, "ps_6_2").as_deref(), Some(&plain[..]));
        let _ = std::fs::remove_file(&q);
        let _ = std::fs::remove_file(cache_path(&tag, "ps_6_2"));
    }
}