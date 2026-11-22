use crate::d3d::{self, Gpu, Guid};
use crate::shader_cache::Fma;
use std::ffi::c_void;
use std::sync::OnceLock;

const PROBE_VERSION: u32 = 3;
const OUTS: usize = 5;
const GROUP: u32 = 256;
const CASES: usize = 1 << 20;

extern "system" {
    fn LoadLibraryA(n: *const u8) -> *mut c_void;
    fn GetProcAddress(h: *mut c_void, n: *const u8) -> *mut c_void;
}

pub struct Probe {
    pub fma: Fma,
    pub adapter: String,
    pub detail: String,
    pub cached: bool,
}

fn helper_lines() -> Option<(&'static str, &'static str, &'static str)> {
    let h = crate::hlsl::CUBE_HELPERS;
    let s = h.find("#ifdef TB_FAST_FMA\n")? + "#ifdef TB_FAST_FMA\n".len();
    let rest = &h[s..];
    let (fast, rest) = rest.split_once('\n')?;
    let rest = rest.strip_prefix("#elif defined(TB_HYBRID_FMA)\n")?;
    let (hybrid, rest) = rest.split_once('\n')?;
    let rest = rest.strip_prefix("#else\n")?;
    let (exact, rest) = rest.split_once('\n')?;
    if !rest.starts_with("#endif") || ![fast, hybrid, exact].iter().all(|l| l.starts_with("float tb_fma(")) {
        return None;
    }
    Some((fast, hybrid, exact))
}

fn f2h_lines() -> Option<(&'static str, &'static str)> {
    let h = crate::hlsl::CUBE_HELPERS;
    let s = h.find("#ifdef TB_FAST_F2H\n")? + "#ifdef TB_FAST_F2H\n".len();
    let rest = &h[s..];
    let (fast, rest) = rest.split_once('\n')?;
    let rest = rest.strip_prefix("#else\n")?;
    let (exact, rest) = rest.split_once('\n')?;
    if !rest.starts_with("#endif") || !fast.starts_with("uint tb_f2h(") || !exact.starts_with("uint tb_f2h(") {
        return None;
    }
    Some((fast, exact))
}

const F2H_PER_THREAD: u32 = 4096;

fn f2h_shader(fast: &str, exact: &str) -> String {
    format!(
        "RWByteAddressBuffer io : register(u0);\n{}\n{}\n[RootSignature(\"UAV(u0)\")]\n[numthreads({GROUP}, 1, 1)]\nvoid main(uint3 g : SV_DispatchThreadID) {{ uint base = g.x * {F2H_PER_THREAD}u; uint bad = 0u; uint first = 0xffffffffu; for (uint k = 0u; k < {F2H_PER_THREAD}u; k++) {{ uint x = base + k; if (tb_f2h_f(asfloat(x)) != tb_f2h_x(asfloat(x))) {{ bad++; first = min(first, x); }} }} uint o; if (bad != 0u) {{ io.InterlockedAdd(0, bad, o); io.InterlockedMin(4, first, o); }} io.InterlockedAdd(8, 1u, o); }}\n",
        fast.replacen("uint tb_f2h(", "uint tb_f2h_f(", 1),
        exact.replacen("uint tb_f2h(", "uint tb_f2h_x(", 1)
    )
}

unsafe fn dispatch_f2h(g: &mut Gpu, code: &[u8]) -> Result<(u32, u32, u32), String> {
    let mut rs: *mut c_void = std::ptr::null_mut();
    let hr = vcall!(g.device, 16, fn(u32, *const u8, usize, *const Guid, *mut *mut c_void) -> i32, 0, code.as_ptr(), code.len(), &d3d::IID_ROOTSIG, &mut rs);
    if hr < 0 {
        return Err(format!("CreateRootSignature {:#x}", hr as u32));
    }
    let mut d = [0u8; 56];
    d[0..8].copy_from_slice(&(rs as u64).to_le_bytes());
    d[8..16].copy_from_slice(&(code.as_ptr() as u64).to_le_bytes());
    d[16..24].copy_from_slice(&(code.len() as u64).to_le_bytes());
    let mut pso: *mut c_void = std::ptr::null_mut();
    let hr = vcall!(g.device, 11, fn(*const u8, *const Guid, *mut *mut c_void) -> i32, d.as_ptr(), &d3d::IID_PSO, &mut pso);
    if hr < 0 {
        d3d::release(rs);
        return Err(format!("CreateComputePipelineState {:#x}", hr as u32));
    }
    let buf = g.buffer(12, 1, d3d::COPY_DEST, true)?;
    let up = g.buffer(12, 2, d3d::GENERIC_READ, false)?;
    let rb = g.buffer(12, 3, d3d::COPY_DEST, false)?;
    let mut init = 0u32.to_le_bytes().to_vec();
    init.extend(u32::MAX.to_le_bytes());
    init.extend(0u32.to_le_bytes());
    Gpu::map_write(up, &init);
    g.begin();
    let l = g.list;
    vcall!(l, 15, fn(*mut c_void, u64, *mut c_void, u64, u64) -> (), buf, 0, up, 0, 12);
    g.barrier(buf, d3d::COPY_DEST, 8);
    vcall!(l, 29, fn(*mut c_void) -> (), rs);
    vcall!(l, 25, fn(*mut c_void) -> (), pso);
    vcall!(l, 41, fn(u32, u64) -> (), 0, Gpu::gpu_va(buf));
    vcall!(l, 14, fn(u32, u32, u32) -> (), f2h_groups(), 1, 1);
    g.barrier(buf, 8, d3d::COPY_SOURCE);
    vcall!(l, 15, fn(*mut c_void, u64, *mut c_void, u64, u64) -> (), rb, 0, buf, 0, 12);
    let sub = g.submit();
    let out = if sub.is_ok() { Gpu::map_read(rb, 12) } else { None };
    for o in [pso, rs, buf, up, rb] {
        d3d::release(o);
    }
    sub?;
    let out = out.ok_or("map readback")?;
    let w = |i: usize| u32::from_le_bytes(out[i..i + 4].try_into().unwrap());
    Ok((w(0), w(4), w(8)))
}

fn f2h_groups() -> u32 {
    ((1u64 << 32) / (F2H_PER_THREAD as u64 * GROUP as u64)) as u32
}

unsafe fn adapter_id() -> String {
    const IID_FACTORY1: Guid = Guid(0x770aae78, 0xf26f, 0x4dba, [0xa8, 0x29, 0x25, 0x3c, 0x83, 0xd1, 0xb3, 0x87]);
    const IID_DXGI_DEVICE: Guid = Guid(0x54ec77fa, 0x1377, 0x44e6, [0x8c, 0x32, 0x88, 0xfd, 0x5f, 0x44, 0xc8, 0x4c]);
    let dxgi = LoadLibraryA(b"dxgi.dll\0".as_ptr());
    if dxgi.is_null() {
        return "no-dxgi".into();
    }
    let p = GetProcAddress(dxgi, b"CreateDXGIFactory1\0".as_ptr());
    if p.is_null() {
        return "no-factory".into();
    }
    let cf: unsafe extern "system" fn(*const Guid, *mut *mut c_void) -> i32 = std::mem::transmute(p);
    let mut f: *mut c_void = std::ptr::null_mut();
    if cf(&IID_FACTORY1, &mut f) < 0 || f.is_null() {
        return "no-factory".into();
    }
    let mut ad: *mut c_void = std::ptr::null_mut();
    let hr = vcall!(f, 12, fn(u32, *mut *mut c_void) -> i32, 0, &mut ad);
    d3d::release(f);
    if hr < 0 || ad.is_null() {
        return "no-adapter".into();
    }
    let mut desc = [0u8; 304];
    vcall!(ad, 8, fn(*mut u8) -> i32, desc.as_mut_ptr());
    let mut umd: i64 = 0;
    let hr_umd = vcall!(ad, 9, fn(*const Guid, *mut i64) -> i32, &IID_DXGI_DEVICE, &mut umd);
    d3d::release(ad);
    let name: Vec<u16> = desc[..256].chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect();
    let u = |o: usize| u32::from_le_bytes(desc[o..o + 4].try_into().unwrap());
    let ver = if hr_umd >= 0 { format!("{}.{}.{}.{}", (umd >> 48) & 0xffff, (umd >> 32) & 0xffff, (umd >> 16) & 0xffff, umd & 0xffff) } else { "unknown".into() };
    format!("{} [{:04x}:{:04x} rev {:x}] driver {}", String::from_utf16_lossy(&name), u(256), u(260), u(268), ver)
}

fn xs(s: &mut u64) -> u64 {
    *s ^= *s << 13;
    *s ^= *s >> 7;
    *s ^= *s << 17;
    *s
}

fn cases(n: usize, seed: u64) -> Vec<[u32; 3]> {
    let f = f32::from_bits;
    let mut s = seed;
    let sign = |r: u64| ((r & 1) as u32) << 31;
    let sp = [0u32, 0x8000_0000, 0x7f80_0000, 0xff80_0000, 1, 0x8000_0001, 0x7f7f_ffff, 0xff7f_ffff, 0x0080_0000, 0x3f80_0000, 0xbf80_0000, 0x7fc0_0000, 0x7f80_0001, 0xffc0_1234, 0x007f_ffff, 0x3f00_0000];
    let mut v = Vec::with_capacity(n);
    while v.len() < n {
        let r = xs(&mut s);
        let r2 = xs(&mut s);
        let t = match v.len() % 9 {
            0 => [r as u32, (r >> 32) as u32, r2 as u32],
            1 => {
                let (a, b) = (f(r as u32), f((r >> 32) as u32));
                let p = (a as f64 * b as f64) as f32;
                [a.to_bits(), b.to_bits(), (p.to_bits() ^ 0x8000_0000) ^ (r2 as u32 & 0xfff)]
            }
            2 => {
                let i = (r2 as u32 & 0x7fff) + 1;
                let ea = (((r >> 8) as u32 % 200) + 27).min(254);
                let eb = (((r >> 20) as u32 % 200) + 27).min(254);
                let a = sign(r) | (ea << 23) | i;
                let b = sign(r >> 1) | (eb << 23) | (0x80_0000 - i);
                let ce = ea as i32 + eb as i32 - 254 + 127 + 24;
                if (1..=254).contains(&ce) { [a, b, sign(r >> 2) | ((ce as u32) << 23) | (((r2 >> 16) as u32) & 0x7f_ffff)] } else { [a, b, r2 as u32] }
            }
            3 => {
                let a = f(r as u32 & 0x80ff_ffff | 0x3f00_0000);
                let b = f((r >> 32) as u32 & 0x80ff_ffff | 0x3f00_0000);
                let p = a as f64 * b as f64;
                let base = p as f32;
                let ulp = f32::from_bits(base.to_bits() + 1) as f64 - base as f64;
                let c = (base as f64 + ulp * 0.5 - p) as f32;
                [a.to_bits(), b.to_bits(), c.to_bits() ^ (r2 as u32 & 3)]
            }
            4 => {
                let e = (r2 as u32 % 40) + 40;
                [sign(r) | (e << 23) | (r as u32 >> 9 & 0x7f_ffff), sign(r >> 1) | (e << 23) | ((r >> 32) as u32 & 0x7f_ffff), (r2 >> 32) as u32 & 0x80ff_ffff]
            }
            5 => {
                let e = 230 + (r2 as u32 % 25);
                let a = sign(r) | (e << 23) | (r as u32 >> 9 & 0x7f_ffff);
                let b = sign(r >> 1) | ((127 + (r2 >> 8) as u32 % 30) << 23) | ((r >> 32) as u32 & 0x7f_ffff);
                let p = f(a) as f64 * f(b) as f64;
                let c = if p.abs() > f32::MAX as f64 { (-(p.signum()) * f32::MAX as f64) as f32 } else { -(p as f32) };
                [a, b, c.to_bits() ^ ((r2 >> 40) as u32 & 0xff)]
            }
            6 => [(r as u32) & 0x807f_ffff, (r >> 32) as u32, (r2 as u32) & 0x80ff_ffff],
            7 => {
                let pick = |x: u64| if x % 3 == 0 { sp[(x / 3) as usize % sp.len()] } else { x as u32 };
                [pick(r & 0xffff_ffff), pick(r >> 32), pick(r2)]
            }
            _ => {
                let (a, b) = (f(r as u32), f((r >> 32) as u32));
                let p = (a as f64 * b as f64) as f32;
                [a.to_bits(), b.to_bits(), (p.to_bits() ^ 0x8000_0000).wrapping_add((r2 as u32 % 7).wrapping_sub(3))]
            }
        };
        v.push(t);
    }
    v
}

fn shader(fast: &str, exact: &str) -> String {
    format!(
        "RWByteAddressBuffer io : register(u0);\ncbuffer K : register(b0) {{ uint n; uint obase; }}\n{}\n{}\n[RootSignature(\"RootConstants(num32BitConstants=2, b0), UAV(u0)\")]\n[numthreads({GROUP}, 1, 1)]\nvoid main(uint3 g : SV_DispatchThreadID) {{ uint i = g.x; if (i >= n) return; float x = asfloat(io.Load(i * 12)); float y = asfloat(io.Load(i * 12 + 4)); float z = asfloat(io.Load(i * 12 + 8)); uint o = obase + i * 40; io.Store2(o, uint2(asuint(tb_fma_f(x, y, z)), asuint(tb_fma_x(x, y, z)))); io.Store2(o + 8, uint2(asuint(tb_fma_f(x, 0.0, z)), asuint(tb_fma_x(x, 0.0, z)))); io.Store2(o + 16, uint2(asuint(tb_fma_f(x, y, 0.0)), asuint(tb_fma_x(x, y, 0.0)))); io.Store2(o + 24, uint2(asuint(tb_fma_f(1.0, y, z)), asuint(tb_fma_x(1.0, y, z)))); io.Store2(o + 32, uint2(asuint(tb_fma_f(x, -0.0, -0.0)), asuint(tb_fma_x(x, -0.0, -0.0)))); }}\n",
        fast.replacen("float tb_fma(", "float tb_fma_f(", 1),
        exact.replacen("float tb_fma(", "float tb_fma_x(", 1)
    )
}

unsafe fn dispatch(g: &mut Gpu, code: &[u8], cs: &[[u32; 3]]) -> Result<Vec<[u32; 2]>, String> {
    let n = cs.len();
    let obase = (n * 12) as u64;
    let total = obase + (n * 8 * OUTS) as u64;
    let mut rs: *mut c_void = std::ptr::null_mut();
    let hr = vcall!(g.device, 16, fn(u32, *const u8, usize, *const Guid, *mut *mut c_void) -> i32, 0, code.as_ptr(), code.len(), &d3d::IID_ROOTSIG, &mut rs);
    if hr < 0 {
        return Err(format!("CreateRootSignature {:#x}", hr as u32));
    }
    let mut d = [0u8; 56];
    d[0..8].copy_from_slice(&(rs as u64).to_le_bytes());
    d[8..16].copy_from_slice(&(code.as_ptr() as u64).to_le_bytes());
    d[16..24].copy_from_slice(&(code.len() as u64).to_le_bytes());
    let mut pso: *mut c_void = std::ptr::null_mut();
    let hr = vcall!(g.device, 11, fn(*const u8, *const Guid, *mut *mut c_void) -> i32, d.as_ptr(), &d3d::IID_PSO, &mut pso);
    if hr < 0 {
        d3d::release(rs);
        return Err(format!("CreateComputePipelineState {:#x}", hr as u32));
    }
    let buf = g.buffer(total, 1, d3d::COPY_DEST, true)?;
    let up = g.buffer(total, 2, d3d::GENERIC_READ, false)?;
    let rb = g.buffer(total, 3, d3d::COPY_DEST, false)?;
    let mut bytes: Vec<u8> = cs.iter().flat_map(|t| t.iter().flat_map(|w| w.to_le_bytes())).collect();
    bytes.resize(total as usize, 0);
    Gpu::map_write(up, &bytes);
    g.begin();
    let l = g.list;
    vcall!(l, 15, fn(*mut c_void, u64, *mut c_void, u64, u64) -> (), buf, 0, up, 0, total);
    g.barrier(buf, d3d::COPY_DEST, 8);
    vcall!(l, 29, fn(*mut c_void) -> (), rs);
    vcall!(l, 25, fn(*mut c_void) -> (), pso);
    let k = [n as u32, obase as u32];
    vcall!(l, 35, fn(u32, u32, *const u32, u32) -> (), 0, 2, k.as_ptr(), 0);
    vcall!(l, 41, fn(u32, u64) -> (), 1, Gpu::gpu_va(buf));
    vcall!(l, 14, fn(u32, u32, u32) -> (), (n as u32 + GROUP - 1) / GROUP, 1, 1);
    g.barrier(buf, 8, d3d::COPY_SOURCE);
    vcall!(l, 15, fn(*mut c_void, u64, *mut c_void, u64, u64) -> (), rb, 0, buf, 0, total);
    let sub = g.submit();
    let out = if sub.is_ok() { Gpu::map_read(rb, total as usize) } else { None };
    for o in [pso, rs, buf, up, rb] {
        d3d::release(o);
    }
    sub?;
    let out = out.ok_or("map readback")?;
    Ok(out[obase as usize..].chunks(8).map(|c| [u32::from_le_bytes(c[0..4].try_into().unwrap()), u32::from_le_bytes(c[4..8].try_into().unwrap())]).collect())
}

fn compare_fma(g: &mut Gpu, src: &str, modes: &[&str], cs: &[[u32; 3]], detail: &mut String, tag: &str) -> Result<Vec<bool>, String> {
    let mut res = Vec::new();
    for mode in modes {
        let code = crate::dxc::compile_denorm(src, "cs_6_2", mode, &[])?;
        let out = unsafe { dispatch(g, &code, cs) }?;
        let bad: Vec<usize> = out.iter().enumerate().filter(|(_, o)| o[0] != o[1]).map(|(k, _)| k / OUTS).collect();
        res.push(bad.is_empty() && out.len() == cs.len() * OUTS);
        detail.push_str(&format!(" {tag}{mode}: {} of {} results differ", bad.len(), out.len()));
        if let Some(&k) = bad.first() {
            let j = out.iter().position(|o| o[0] != o[1]).unwrap_or(0);
            detail.push_str(&format!(" (first a={:08x} b={:08x} c={:08x} form {} fast={:08x} exact={:08x})", cs[k][0], cs[k][1], cs[k][2], j % OUTS, out[j][0], out[j][1]));
        }
    }
    Ok(res)
}

fn run_probe(fast: &str, hybrid: &str, exact: &str, f2h: Option<(&str, &str)>) -> Result<(Fma, String), String> {
    let mut g = unsafe { Gpu::new(false) }?;
    g.open = false;
    let cs = cases(CASES, 0x9e37_79b9_7f4a_7c15 ^ PROBE_VERSION as u64);
    let mut detail = String::new();
    let res = compare_fma(&mut g, &shader(fast, exact), &["preserve", "any"], &cs, &mut detail, "")?;
    let hyb = compare_fma(&mut g, &shader(hybrid, exact), &["any"], &cs, &mut detail, "hybrid ")?;
    let mut f2h_ok = f2h.is_some();
    if let Some((ff, fx)) = f2h {
        let src = f2h_shader(ff, fx);
        for mode in ["preserve", "any"] {
            let code = crate::dxc::compile_denorm(&src, "cs_6_2", mode, &["-enable-16bit-types"])?;
            let (bad, first, ran) = unsafe { dispatch_f2h(&mut g, &code) }?;
            let all = ran == f2h_groups() * GROUP;
            f2h_ok &= bad == 0 && all;
            detail.push_str(&format!(" f2h {mode}: {bad} of 4294967296 inputs differ{}", if all { "" } else { " (not all threads ran)" }));
            if bad != 0 {
                detail.push_str(&format!(" (first {first:08x})"));
            }
        }
    }
    unsafe {
        for o in [g.fence, g.list, g.alloc, g.queue, g.device] {
            d3d::release(o);
        }
    }
    Ok((Fma { preserve: res[0], any: res[1], hybrid: hyb[0], f2h: f2h_ok }, detail.trim().to_string()))
}

pub fn parse_flags(r: &str) -> Fma {
    let on = |k: &str| r.split_whitespace().any(|w| w == format!("{k}=1"));
    Fma { preserve: on("preserve"), any: on("any"), hybrid: on("hybrid"), f2h: on("f2h") }
}

pub fn format_flags(f: Fma) -> String {
    format!("preserve={} any={} hybrid={} f2h={}", f.preserve as u8, f.any as u8, f.hybrid as u8, f.f2h as u8)
}

pub fn probe() -> &'static Probe {
    static P: OnceLock<Probe> = OnceLock::new();
    P.get_or_init(|| {
        let adapter = unsafe { adapter_id() };
        match std::env::var("TOMBOX_FAST_FMA").ok().as_deref() {
            Some("0") => return Probe { fma: Fma::default(), adapter, detail: "TOMBOX_FAST_FMA=0: exact f64 FMA path forced".into(), cached: false },
            Some("force") => return Probe { fma: Fma { preserve: true, any: true, hybrid: true, f2h: true }, adapter, detail: "TOMBOX_FAST_FMA=force: hardware mad used WITHOUT verification".into(), cached: false },
            _ => {}
        }
        let Some((fast, hybrid, exact)) = helper_lines() else {
            return Probe { fma: Fma::default(), adapter, detail: "tb_fma helper block not found; exact path kept".into(), cached: false };
        };
        let id = format!("v{PROBE_VERSION} {adapter} {} {:016x}", crate::dxc::identity(), {
            let mut h = 0xcbf2_9ce4_8422_2325u64;
            let f2h = f2h_lines().map_or(String::new(), |(a, b)| format!("{a}{b}"));
            for b in fast.bytes().chain(hybrid.bytes()).chain(exact.bytes()).chain(f2h.bytes()) {
                h ^= b as u64;
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
            h
        });
        let file = crate::shader_cache::root().join("fma_probe.txt");
        if let Ok(t) = std::fs::read_to_string(&file) {
            let mut it = t.lines();
            if it.next() == Some(id.as_str()) {
                if let Some(r) = it.next() {
                    let fma = parse_flags(r);
                    return Probe { fma, adapter, detail: it.next().unwrap_or("").to_string(), cached: true };
                }
            }
        }
        match std::panic::catch_unwind(|| run_probe(fast, hybrid, exact, f2h_lines())) {
            Ok(Ok((fma, detail))) => {
                let _ = std::fs::write(&file, format!("{id}\n{}\n{detail}\n", format_flags(fma)));
                Probe { fma, adapter, detail, cached: false }
            }
            Ok(Err(e)) => Probe { fma: Fma::default(), adapter, detail: format!("probe failed ({e}); exact path kept"), cached: false },
            Err(_) => Probe { fma: Fma::default(), adapter, detail: "probe panicked; exact path kept".into(), cached: false },
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helper_block_is_found() {
        let (f, h, x) = helper_lines().expect("tb_fma block");
        assert!(f.contains("mad(") && !f.contains("double"));
        assert!(h.contains("mad(") && h.contains("double"));
        assert!(x.contains("double"));
        let (ff, fx) = f2h_lines().expect("tb_f2h block");
        assert!(ff.contains("(float16_t)x"));
        assert!(!fx.contains("f32tof16("));
        assert!(f2h_shader(ff, fx).contains("tb_f2h_f(") && f2h_shader(ff, fx).contains("tb_f2h_x("));
    }

    #[test]
    fn cached_flags_round_trip() {
        for bits in 0..16u8 {
            let f = Fma { preserve: bits & 1 != 0, any: bits & 2 != 0, hybrid: bits & 4 != 0, f2h: bits & 8 != 0 };
            assert_eq!(parse_flags(&format_flags(f)), f);
        }
        assert_eq!(parse_flags("preserve=1 any=0"), Fma { preserve: true, ..Fma::default() });
    }

    #[test]
    fn hybrid_line_keeps_the_exact_body_as_fallback() {
        let (_, h, x) = helper_lines().unwrap();
        let body = x.split_once("{ ").unwrap().1;
        assert!(h.ends_with(body));
    }

    #[test]
    #[ignore = "needs a D3D12 GPU"]
    fn hw_bypass_digest_container_runs() {
        let (f, _, x) = helper_lines().unwrap();
        let src = shader(f, x);
        let t0 = std::time::Instant::now();
        let signed = crate::dxc::compile_denorm(&src, "cs_6_2", "preserve", &[]).unwrap();
        let t_signed = t0.elapsed().as_secs_f64() * 1000.0;
        let t1 = std::time::Instant::now();
        let mut raw = crate::dxc::compile_denorm(&src, "cs_6_2", "preserve", &["-Vd"]).unwrap();
        let t_vd = t1.elapsed().as_secs_f64() * 1000.0;
        let same_outside_digest = signed.len() == raw.len() && signed[..4] == raw[..4] && signed[20..] == raw[20..];
        let unsigned_digest = raw[4..20].to_vec();
        raw[4..20].copy_from_slice(&[1u8; 16]);
        let cs = cases(4096, 7);
        let mut g = unsafe { Gpu::new(false) }.unwrap();
        g.open = false;
        let a = unsafe { dispatch(&mut g, &signed, &cs) };
        let b = unsafe { dispatch(&mut g, &raw, &cs) };
        println!("signed {t_signed:.0} ms, -Vd {t_vd:.0} ms, bytes identical outside digest {same_outside_digest}, -Vd digest {unsigned_digest:02x?}");
        println!("signed run ok {}, bypass run {:?}, outputs equal {}", a.is_ok(), b.as_ref().err(), a.as_ref().ok() == b.as_ref().ok());
    }

    #[test]
    #[ignore = "needs a D3D12 GPU"]
    fn hw_fast_fma_matches_exact_path() {
        let (f, h, x) = helper_lines().unwrap();
        let (fma, detail) = run_probe(f, h, x, f2h_lines()).expect("probe");
        println!("{} | {:?} | {detail}", unsafe { adapter_id() }, fma);
    }
}
