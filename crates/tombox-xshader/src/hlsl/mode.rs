use super::{d64, f, hi64, name, r, Emitter};
use crate::rdna2::{Enc, Insn, Opnd};
use std::collections::BTreeSet;

pub(super) const MAD_HELPERS: &str = r#"float tb_ftz(float x) { return ((asuint(x) & 0x7f800000u) == 0u) ? asfloat(asuint(x) & 0x80000000u) : x; }
float tb_madz(float a, float b, float c) { precise float p = tb_ftz(tb_ftz(a) * tb_ftz(b)); precise float s = p + tb_ftz(c); return tb_qn(tb_ftz(s)); }
float tb_madlz(float a, float b, float c) { float x = tb_ftz(a), y = tb_ftz(b); precise float p = tb_ftz((x == 0.0 || y == 0.0) ? 0.0 : x * y); precise float s = p + tb_ftz(c); return tb_qn(tb_ftz(s)); }
float tb_hz(float x) { return (abs(x) < 6.103515625e-05) ? asfloat(asuint(x) & 0x80000000u) : x; }
uint tb_hzb(uint b) { return ((b & 0x7c00u) == 0u) ? (b & 0x8000u) : b; }
"#;

pub(super) const MODE_HELPERS: &str = r#"float tb_dz(float x) { return ((asuint(x) & 0x7f800000u) == 0u) ? asfloat(asuint(x) & 0x80000000u) : x; }
float tb_din(float x, uint m) { return ((m & 0x10u) != 0u) ? x : tb_dz(x); }
float tb_dout(float x, uint m) { return ((m & 0x20u) != 0u) ? x : tb_dz(x); }
float tb_hin(float x, uint m) { return ((m & 0x40u) != 0u || !(abs(x) < 6.103515625e-05)) ? x : asfloat(asuint(x) & 0x80000000u); }
uint tb_fi32(uint x, uint m) { return ((m & 0x10u) != 0u || (x & 0x7f800000u) != 0u) ? x : (x & 0x80000000u); }
uint tb_fo32(uint x, uint m) { return ((m & 0x20u) != 0u || (x & 0x7f800000u) != 0u) ? x : (x & 0x80000000u); }
uint tb_f16z(uint x, uint sel) { uint lo = x & 0xffffu, hi = x >> 16; if ((sel & 1u) != 0u && (lo & 0x7c00u) == 0u) lo &= 0x8000u; if ((sel & 2u) != 0u && (hi & 0x7c00u) == 0u) hi &= 0x8000u; return lo | (hi << 16); }
uint tb_fi16(uint x, uint m, uint sel) { return ((m & 0x40u) != 0u) ? x : tb_f16z(x, sel); }
uint tb_fo16(uint x, uint m, uint sel) { return ((m & 0x80u) != 0u) ? x : tb_f16z(x, sel); }
void tb_f64z(inout uint lo, inout uint hi, uint m, uint bit) { if ((m & bit) == 0u && (hi & 0x7ff00000u) == 0u) { lo = 0u; hi &= 0x80000000u; } }
float tb_nup(float r) { uint b = asuint(r); if ((b & 0x7fffffffu) == 0u) return asfloat(1u); if (b == 0x7f800000u) return r; return asfloat((b >> 31) != 0u ? b - 1u : b + 1u); }
float tb_ndn(float r) { uint b = asuint(r); if ((b & 0x7fffffffu) == 0u) return asfloat(0x80000001u); if (b == 0xff800000u) return r; return asfloat((b >> 31) != 0u ? b + 1u : b - 1u); }
float tb_rfix(float r, double e, uint m) { uint k = m & 3u; if (k == 0u || tb_nan(r) || !(abs(e) > 0.0)) return r; if (k == 1u) return e > 0.0 ? tb_nup(r) : r; if (k == 2u) return e < 0.0 ? tb_ndn(r) : r; if ((asuint(r) >> 31) == 0u) return e < 0.0 ? tb_ndn(r) : r; return e > 0.0 ? tb_nup(r) : r; }
double tb_res2(double x, double y, float r) { precise double s = x + y; precise double bb = s - x; precise double er = (x - (s - bb)) + (y - bb); precise double d = s - (double)r; return d + er; }
float tb_m_add(float a, float b, uint m) { precise float r = a + b; precise double e = tb_res2((double)a, (double)b, r); float q = tb_rfix(r, e, m); if ((m & 3u) == 2u && asuint(q) == 0u && !(abs(e) > 0.0) && (asuint(a) | asuint(b)) != 0u) q = asfloat(0x80000000u); return q; }
float tb_m_mul(float a, float b, uint m) { precise double p = (double)a * (double)b; precise float r = (float)p; precise double e = p - (double)r; return tb_rfix(r, e, m); }
float tb_m_mull(float a, float b, uint m) { return (a == 0.0 || b == 0.0) ? 0.0 : tb_m_mul(a, b, m); }
float tb_m_fma(float a, float b, float c, uint m) { precise double p = (double)a * (double)b; precise float r = tb_fma(a, b, c); precise double e = tb_res2(p, (double)c, r); float q = tb_rfix(r, e, m); if ((m & 3u) == 2u && asuint(q) == 0u && !(abs(e) > 0.0) && !(p == 0.0 && ((asuint(a) ^ asuint(b)) >> 31) == 0u && asuint(c) == 0u)) q = asfloat(0x80000000u); return q; }
float tb_m_cvti(int x, uint m) { precise float r = (float)x; double e = (r >= 2147483648.0) ? -1.0 : (((int)r < x) ? 1.0 : (((int)r > x) ? -1.0 : 0.0)); return tb_rfix(r, e, m); }
float tb_m_cvtu(uint x, uint m) { precise float r = tb_u2f(x); double e = (r >= 4294967296.0) ? -1.0 : (((uint)r < x) ? 1.0 : (((uint)r > x) ? -1.0 : 0.0)); return tb_rfix(r, e, m); }
float tb_m_cvtd(double x, uint m) { precise float r = (float)x; precise double e = x - (double)r; return tb_rfix(r, e, m); }
double tb_p2(int n) { return asdouble(0u, (uint)(clamp(n, -1000, 1000) + 1023) << 20); }
float tb_m_ldexp(float a, int n, uint m) { int k = clamp(n, -300, 300); precise double x = (double)a * tb_p2(k >> 1) * tb_p2(k - (k >> 1)); precise float r = (float)x; precise double e = x - (double)r; return tb_rfix(r, e, m); }
uint tb_h_rnd(double x, double er, uint k, uint zs) { if (x != x) return tb_f2h((float)x); if (x == 0.0 && er == 0.0) return zs << 15; bool ng = (x < 0.0) || (x == 0.0 && er < 0.0); double ax = ng ? -x : x; double ea = ng ? -er : er; uint sg = ng ? 0x8000u : 0u; uint lo, hi; asuint(ax, lo, hi); bool ov = (k == 0u) || (k == 1u && !ng) || (k == 2u && ng); int e = (int)((hi >> 20) & 0x7ffu) - 1023; if (e >= 16) return sg | (ov ? 0x7c00u : 0x7bffu); int ue = max(e - 10, -24); double sc = asdouble(0u, (uint)(1023 - ue) << 20); double mm = ax * sc; uint n = (uint)mm; double fr = mm - (double)n; double es = ea * sc; if (fr == 0.0 && es < 0.0) { n -= 1u; fr = 1.0; es = 0.0; } bool nz = fr > 0.0 || es > 0.0; bool up = false; if (k == 0u) up = fr > 0.5 || (fr == 0.5 && (es > 0.0 || (es == 0.0 && (n & 1u) != 0u))); else if (k != 3u) up = nz && ((k == 1u) != ng); if (up) n += 1u; uint b = (uint)((ue + 24) << 10) + n; if (b >= 0x7c00u) b = ov ? 0x7c00u : 0x7bffu; return sg | b; }
uint tb_zs(float a, float b, uint k) { uint sa = asuint(a) >> 31, sb = asuint(b) >> 31; return (sa & sb) | ((k == 2u) ? (sa | sb) : 0u); }
uint tb_h_add(float a, float b, uint m) { uint k = (m >> 2) & 3u; return tb_h_rnd((double)a + (double)b, 0.0, k, tb_zs(a, b, k)); }
uint tb_h_mul(float a, float b, uint m) { return tb_h_rnd((double)a * (double)b, 0.0, (m >> 2) & 3u, (asuint(a) ^ asuint(b)) >> 31); }
uint tb_h_fma(float a, float b, float c, uint m) { uint k = (m >> 2) & 3u; precise double p = (double)a * (double)b; precise double s = p + (double)c; precise double bb = s - p; precise double er = (p - (s - bb)) + ((double)c - bb); uint sp = (asuint(a) ^ asuint(b)) >> 31, sc = asuint(c) >> 31; return tb_h_rnd(s, er, k, (sp & sc) | ((k == 2u) ? (sp | sc) : 0u)); }
uint tb_h_cvt(double x, uint m, uint zs) { return tb_h_rnd(x, 0.0, (m >> 2) & 3u, zs); }
"#;

pub(super) const MODE_F64_HELPERS: &str = r#"bool tb_dneg(double r) { uint lo, hi; asuint(r, lo, hi); return (hi >> 31) != 0u; }
double tb_dnext(double r, bool up) { uint lo, hi; asuint(r, lo, hi); bool ng = (hi >> 31) != 0u; if (((hi & 0x7fffffffu) | lo) == 0u) return up ? asdouble(1u, 0u) : asdouble(1u, 0x80000000u); if ((hi & 0x7fffffffu) == 0x7ff00000u && lo == 0u && ng != up) return r; if (ng != up) { lo += 1u; if (lo == 0u) hi += 1u; } else { if (lo == 0u) hi -= 1u; lo -= 1u; } return asdouble(lo, hi); }
double tb_drfix(double r, double e, uint m) { uint k = (m >> 2) & 3u; if (k == 0u || r != r || !(abs(e) > 0.0L)) return r; if (k == 1u) return e > 0.0L ? tb_dnext(r, true) : r; if (k == 2u) return e < 0.0L ? tb_dnext(r, false) : r; if (!tb_dneg(r)) return e < 0.0L ? tb_dnext(r, false) : r; return e > 0.0L ? tb_dnext(r, true) : r; }
double tb_dz0(double q, double e, uint m, bool pz) { if (((m >> 2) & 3u) == 2u && q == 0.0L && !tb_dneg(q) && e == 0.0L && !pz) return asdouble(0u, 0x80000000u); return q; }
double tb_d_add(double a, double b, uint m) { precise double s = a + b; precise double bb = s - a; precise double e = (a - (s - bb)) + (b - bb); if (tb_isinf64(s) && !tb_isinf64(a) && !tb_isinf64(b)) e = -s; return tb_dz0(tb_drfix(s, e, m), e, m, a == 0.0L && !tb_dneg(a) && b == 0.0L && !tb_dneg(b)); }
double tb_d_mul(double a, double b, uint m) { precise double p = a * b; precise double e = fma(a, b, -p); if (abs(p) < asdouble(0u, 0x03500000u)) { bool sa = abs(a) <= abs(b); double sc = asdouble(0u, 0x46300000u); e = fma((sa ? a : b) * sc, sa ? b : a, -(p * sc)); } if (tb_isinf64(p) && !tb_isinf64(a) && !tb_isinf64(b)) e = -p; return tb_drfix(p, e, m); }
double tb_d_fmae(double a, double b, double c, double r) { precise double p = a * b; precise double pe = fma(a, b, -p); precise double s = p + c; precise double bb = s - p; precise double se = (p - (s - bb)) + (c - bb); precise double e = ((s - r) + se) + pe; if (tb_isinf64(r) && !tb_isinf64(a) && !tb_isinf64(b) && !tb_isinf64(c)) e = -r; return e; }
double tb_d_fma(double a, double b, double c, uint m) { precise double r = fma(a, b, c); double e = tb_d_fmae(a, b, c, r); return tb_dz0(tb_drfix(r, e, m), e, m, a * b == 0.0L && tb_dneg(a) == tb_dneg(b) && c == 0.0L && !tb_dneg(c)); }
double tb_d_ldexp(double a, int n, uint m) { double r = tb_ldexp64(a, n); double e = (tb_isinf64(r) && !tb_isinf64(a)) ? -r : (a - tb_ldexp64(r, -n)); return tb_drfix(r, e, m); }
double tb_d_fmas(double a, double b, double c, uint vcc, uint m) { double r = tb_div_fmas64(a, b, c, vcc); double e = tb_d_fmae(a, b, c, fma(a, b, c)); return tb_drfix(r, e, m); }
"#;

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(super) struct ModeUse {
    pub sp: bool,
    pub dp: bool,
    pub spr: bool,
    pub spd: bool,
    pub dpr: bool,
    pub dpd: bool,
}

fn write_of(i: &Insn) -> Option<Option<(u32, u32)>> {
    match (i.enc, i.mnemonic.as_str()) {
        (Enc::Sopp, "s_round_mode") => Some(Some((0x0f, i.aux.imm as u32 & 0x0f))),
        (Enc::Sopp, "s_denorm_mode") => Some(Some((0xf0, (i.aux.imm as u32 & 0x0f) << 4))),
        (Enc::Sopk, "s_setreg_imm32_b32" | "s_setreg_b32") => {
            let sim = i.aux.imm as u32 & 0xffff;
            let (id, off, size) = (sim & 63, (sim >> 6) & 31, ((sim >> 11) & 31) + 1);
            if id != 1 || off > 7 {
                return None;
            }
            let fmask = if size >= 32 { u32::MAX } else { (1u32 << size) - 1 };
            let mask = (fmask << off) & 0xff;
            if mask == 0 {
                return None;
            }
            if i.mnemonic == "s_setreg_b32" {
                return Some(None);
            }
            let v = i.raw.get(1).copied().unwrap_or(0);
            Some(Some((mask, (v << off) & mask)))
        }
        _ => None,
    }
}

pub(super) fn reachable(insns: &[Insn]) -> BTreeSet<u8> {
    let mut writes = Vec::new();
    for i in insns {
        match write_of(i) {
            Some(Some(w)) => writes.push(w),
            Some(None) => return (0..=255u8).collect(),
            None => {}
        }
    }
    let mut set: BTreeSet<u8> = [0xf0u8].into_iter().collect();
    let mut work = vec![0xf0u32];
    while let Some(s) = work.pop() {
        for &(mask, v) in &writes {
            let n = ((s & !mask) | v) as u8;
            if set.insert(n) {
                work.push(n as u32);
            }
        }
    }
    set
}

pub(super) fn analyze(insns: &[Insn]) -> ModeUse {
    let set = reachable(insns);
    let spr = set.iter().any(|&s| s & 3 != 0);
    let spd = set.iter().any(|&s| (s >> 4) & 3 != 3);
    let dpr = set.iter().any(|&s| (s >> 2) & 3 != 0);
    let dpd = set.iter().any(|&s| (s >> 6) & 3 != 3);
    ModeUse { sp: spr || spd, dp: dpr || dpd, spr, spd, dpr, dpd }
}

pub(super) fn uses_mad(insns: &[Insn]) -> bool {
    insns.iter().any(|i| is_mad(&i.mnemonic))
}

pub(super) fn is_mad(m: &str) -> bool {
    matches!(
        m,
        "v_mac_f32" | "v_mac_legacy_f32" | "v_mad_f32" | "v_mad_legacy_f32" | "v_madmk_f32" | "v_madak_f32"
    )
}

pub(super) fn din(x: String) -> String {
    format!("tb_din({x}, tb_hw1)")
}

pub(super) fn dout(e: String) -> String {
    format!("tb_dout({e}, tb_hw1)")
}

pub(super) fn arith(m: &str, legacy: bool, a: &[String], dn: &str, k: &str) -> Option<String> {
    let acc = din(format!("asfloat({dn})"));
    let mul = |x: &str, y: &str| if legacy { format!("tb_m_mull({x}, {y}, tb_hw1)") } else { format!("tb_m_mul({x}, {y}, tb_hw1)") };
    let add = |x: &str, y: &str| format!("tb_m_add({x}, {y}, tb_hw1)");
    let mad = |x: &str, y: &str, z: &str| format!("tb_ftz({})", add(&format!("tb_ftz({})", mul(&format!("tb_ftz({x})"), &format!("tb_ftz({y})"))), &format!("tb_ftz({z})")));
    let fma = |x: &str, y: &str, z: &str| format!("tb_m_fma({x}, {y}, {z}, tb_hw1)");
    let g = |n: usize| a.get(n).cloned().unwrap_or_default();
    Some(match m {
        "v_add_f32" => add(&g(0), &g(1)),
        "v_sub_f32" => add(&g(0), &format!("(-{})", g(1))),
        "v_subrev_f32" => add(&g(1), &format!("(-{})", g(0))),
        "v_mul_f32" => mul(&g(0), &g(1)),
        "v_mac_f32" => mad(&g(0), &g(1), &acc),
        "v_madmk_f32" => mad(&g(0), k, &g(1)),
        "v_madak_f32" => mad(&g(0), &g(1), k),
        "v_mad_f32" => mad(&g(0), &g(1), &g(2)),
        "v_fma_f32" => fma(&g(0), &g(1), &g(2)),
        "v_fmac_f32" => fma(&g(0), &g(1), &acc),
        "v_fmamk_f32" => fma(&g(0), k, &g(1)),
        "v_fmaak_f32" => fma(&g(0), &g(1), k),
        _ => return None,
    })
}

pub(super) fn cvt(m: &str, a: &str) -> Option<String> {
    match m {
        "v_cvt_f32_i32" => Some(format!("tb_m_cvti(asint({a}), tb_hw1)")),
        "v_cvt_f32_u32" => Some(format!("tb_m_cvtu({a}, tb_hw1)")),
        _ => None,
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum P {
    S,
    D,
    H(u32),
}

fn ty(t: &str) -> Option<P> {
    match t {
        "f16" => Some(P::H(1)),
        "f32" => Some(P::S),
        "f64" => Some(P::D),
        _ => None,
    }
}

fn is_reg(o: &Opnd) -> bool {
    matches!(o, Opnd::Sgpr(_) | Opnd::Vgpr(_) | Opnd::VccLo | Opnd::VccHi | Opnd::M0 | Opnd::ExecLo | Opnd::ExecHi | Opnd::Temp(_))
}

fn pk_sel(ins: &Insn) -> [u32; 2] {
    let (w0, w1) = (ins.raw[0], ins.raw.get(1).copied().unwrap_or(0));
    [(w0 >> 11) & 7, ((w1 >> 27) & 3) | (((w0 >> 14) & 1) << 2)]
}

fn hsel(ins: &Insn, i: usize) -> u32 {
    if let Some(w) = ins.aux.sdwa {
        let sel = if i == 0 { (w >> 16) & 7 } else { (w >> 24) & 7 };
        return if sel == 5 { 2 } else { 1 };
    }
    if ins.enc == Enc::Vop3 && i < 3 && (ins.raw[0] >> (11 + i)) & 1 != 0 { 2 } else { 1 }
}

impl Emitter<'_> {
    fn mode_dst_half(&self, ins: &Insn) -> Option<u32> {
        let m = ins.mnemonic.as_str();
        if m.starts_with("v_pk_") || m == "v_cvt_pkrtz_f16_f32" {
            return Some(3);
        }
        if m == "v_fma_mixlo_f16" {
            return Some(1);
        }
        if m == "v_fma_mixhi_f16" {
            return Some(2);
        }
        if let Some((_, hi)) = &self.d16 {
            return Some(if *hi { 2 } else { 1 });
        }
        if let Some(w) = ins.aux.sdwa {
            return match (w >> 8) & 7 {
                5 => Some(2),
                4 | 6 => Some(1),
                _ => None,
            };
        }
        Some(1)
    }

    fn mode_spec(&self, ins: &Insn) -> Option<(Vec<(Opnd, P)>, Option<(Opnd, P)>)> {
        let m = ins.mnemonic.as_str();
        if !m.starts_with("v_") || !(m.contains("f16") || m.contains("f32") || m.contains("f64")) {
            return None;
        }
        if m.contains("class") || m == "v_pack_b32_f16" || m.starts_with("v_interp") {
            return None;
        }
        let s = &ins.src;
        let d = ins.dst.first().cloned();
        let half = |p: P, i: usize| match p {
            P::H(_) => P::H(hsel(ins, i)),
            o => o,
        };
        let dhalf = |p: P| match p {
            P::H(_) => self.mode_dst_half(ins).map(P::H),
            o => Some(o),
        };
        let all = |p: P, n: usize| -> Vec<(Opnd, P)> { s.iter().take(n).enumerate().map(|(i, o)| (o.clone(), half(p, i))).collect() };
        let suffix = ty(m.rsplit('_').next().unwrap_or(""));
        if let Some(rest) = m.strip_prefix("v_cvt_") {
            let parts: Vec<&str> = rest.split('_').collect();
            let st = ty(parts.last().copied().unwrap_or(""));
            let first = parts[0];
            let dt = match first {
                "pkrtz" => Some(P::H(3)),
                "pknorm" | "norm" | "flr" | "rpi" | "pk" => None,
                t => ty(t),
            };
            let n = match first {
                "pkrtz" | "pknorm" => 2,
                _ => 1,
            };
            let srcs = st.map(|p| all(p, n)).unwrap_or_default();
            let dst = dt.and_then(|p| if p == P::H(3) { Some(p) } else { dhalf(p) }).and_then(|p| d.clone().map(|o| (o, p)));
            return Some((srcs, dst));
        }
        let p = suffix?;
        if m.starts_with("v_cmp") {
            return Some((all(p, 2), None));
        }
        if m.starts_with("v_frexp_exp") {
            return Some((all(p, 1), None));
        }
        if m.starts_with("v_ldexp") || m == "v_trig_preop_f64" {
            return Some((all(p, 1), dhalf(p).and_then(|q| d.clone().map(|o| (o, q)))));
        }
        if m.starts_with("v_fma_mix") {
            let (w0, w1) = (ins.raw[0], ins.raw.get(1).copied().unwrap_or(0));
            let (opsel, oph) = ((w0 >> 11) & 7, ((w1 >> 27) & 3) | (((w0 >> 14) & 1) << 2));
            let srcs = s.iter().take(3).enumerate().map(|(i, o)| (o.clone(), if (oph >> i) & 1 != 0 { P::H(if (opsel >> i) & 1 != 0 { 2 } else { 1 }) } else { P::S })).collect();
            let dp = if m == "v_fma_mix_f32" { P::S } else { P::H(self.mode_dst_half(ins)?) };
            return Some((srcs, d.map(|o| (o, dp))));
        }
        if m.starts_with("v_pk_") {
            let sel = pk_sel(ins);
            let n = if m == "v_pk_fma_f16" { 3 } else { 2 };
            let mut srcs: Vec<(Opnd, P)> = s
                .iter()
                .take(n)
                .enumerate()
                .map(|(i, o)| {
                    let mk = if m == "v_pk_fmac_f16" { 3 } else { (if (sel[0] >> i) & 1 != 0 { 2 } else { 1 }) | (if (sel[1] >> i) & 1 != 0 { 2 } else { 1 }) };
                    (o.clone(), P::H(mk))
                })
                .collect();
            if m == "v_pk_fmac_f16" {
                if let Some(o) = d.clone() {
                    srcs.push((o, P::H(3)));
                }
            }
            return Some((srcs, d.map(|o| (o, P::H(3)))));
        }
        let mut srcs = all(p, s.len().min(3));
        let dp = dhalf(p)?;
        if matches!(m, "v_mac_f32" | "v_fmac_f32" | "v_mac_legacy_f32" | "v_fmac_f16") {
            if let Some(o) = d.clone() {
                srcs.push((o, dp));
            }
        }
        Some((srcs, d.map(|o| (o, dp))))
    }

    fn mode_active(&self, p: P) -> bool {
        match p {
            P::S => self.mode_use.spd,
            _ => self.mode_use.dpd,
        }
    }

    pub(super) fn emit_vec_mode(&mut self, ins: &Insn) -> Result<(), String> {
        if !(self.mode_use.sp || self.mode_use.dp) {
            return self.emit_vec(ins);
        }
        let Some((srcs, dst)) = self.mode_spec(ins) else {
            return self.emit_vec(ins);
        };
        let mut saved: Vec<(String, String, P)> = Vec::new();
        let add = |saved: &mut Vec<(String, String, P)>, n: String, p: P| {
            if let Some(e) = saved.iter_mut().find(|e| e.0 == n) {
                if let (P::H(a), P::H(b)) = (e.2, p) {
                    e.2 = P::H(a | b);
                }
                return;
            }
            saved.push((n, String::new(), p));
        };
        for (o, p) in &srcs {
            if let Opnd::Literal(v) = o {
                let den = match p {
                    P::S => v & 0x7f80_0000 == 0 && v & 0x7fff_ffff != 0,
                    P::H(_) => v & 0x7c00 == 0 && v & 0x7fff != 0,
                    P::D => false,
                };
                if den && self.mode_active(*p) && !matches!(ins.mnemonic.as_str(), "v_madmk_f32" | "v_madak_f32" | "v_fmamk_f32" | "v_fmaak_f32") {
                    return Err(format!("{}: denormal literal operand {v:#x} under a shader-written denormal MODE", ins.mnemonic));
                }
                continue;
            }
            if !is_reg(o) || !self.mode_active(*p) {
                continue;
            }
            if *p != P::D {
                add(&mut saved, name(o)?, *p);
            }
        }
        let t = self.tmp;
        self.tmp += 1;
        for (k, (n, sv, p)) in saved.iter_mut().enumerate() {
            *sv = format!("tbm{t}_{k}");
            match p {
                P::S => self.line(&format!("uint {sv} = {n}; {n} = tb_fi32({n}, tb_hw1);")),
                P::H(sel) => self.line(&format!("uint {sv} = {n}; {n} = tb_fi16({n}, tb_hw1, {sel}u);")),
                P::D => {}
            }
            self.regs.insert(n.clone());
        }
        let mut pairs: Vec<(String, String, String, String)> = Vec::new();
        for (o, p) in &srcs {
            if *p == P::D && is_reg(o) && self.mode_active(P::D) {
                let (lo, hi) = (name(o)?, name(&hi64(o))?);
                if pairs.iter().any(|e| e.0 == lo) {
                    continue;
                }
                let k = pairs.len();
                let (sl, sh) = (format!("tbm{t}_dl{k}"), format!("tbm{t}_dh{k}"));
                self.touch(o);
                self.touch(&hi64(o));
                self.line(&format!("uint {sl} = {lo}, {sh} = {hi}; tb_f64z({lo}, {hi}, tb_hw1, 0x40u);"));
                pairs.push((lo, hi, sl, sh));
            }
        }
        self.emit_vec(ins)?;
        let mut dregs: Vec<(String, u32)> = Vec::new();
        if let Some((o, p)) = &dst {
            if is_reg(o) {
                match p {
                    P::S => dregs.push((name(o)?, 0xffff_ffff)),
                    P::H(sel) => dregs.push((name(o)?, (if sel & 1 != 0 { 0xffff } else { 0 }) | (if sel & 2 != 0 { 0xffff_0000 } else { 0 }))),
                    P::D => {
                        dregs.push((name(o)?, 0xffff_ffff));
                        dregs.push((name(&hi64(o))?, 0xffff_ffff));
                    }
                }
                if self.mode_active(*p) {
                    match p {
                        P::S => {
                            let n = name(o)?;
                            self.line(&format!("if (exec_lo & 1u) {n} = tb_fo32({n}, tb_hw1);"));
                        }
                        P::H(sel) => {
                            let n = name(o)?;
                            self.line(&format!("if (exec_lo & 1u) {n} = tb_fo16({n}, tb_hw1, {sel}u);"));
                        }
                        P::D => {
                            let (lo, hi) = (name(o)?, name(&hi64(o))?);
                            self.line(&format!("if (exec_lo & 1u) tb_f64z({lo}, {hi}, tb_hw1, 0x80u);"));
                        }
                    }
                }
            }
        }
        let restore = |me: &mut Self, n: &str, sv: &str| {
            match dregs.iter().find(|e| e.0 == n) {
                Some(&(_, mask)) if mask == 0xffff_ffff => me.line(&format!("if ((exec_lo & 1u) == 0u) {n} = {sv};")),
                Some(&(_, mask)) => me.line(&format!("{n} = ((exec_lo & 1u) == 0u) ? {sv} : (({n} & {mask:#x}u) | ({sv} & {:#x}u));", !mask)),
                None => me.line(&format!("{n} = {sv};")),
            }
        };
        for (n, sv, _) in &saved {
            restore(self, n, sv);
        }
        for (lo, hi, sl, sh) in &pairs {
            restore(self, lo, sl);
            restore(self, hi, sh);
        }
        Ok(())
    }

    fn hval(&self, ins: &Insn, i: usize, sel: Option<u32>) -> Result<String, String> {
        let o = ins.src.get(i).ok_or("missing operand")?;
        let base = match o {
            Opnd::ConstF32(_) => f(o)?,
            Opnd::Literal(v) => format!("f16tof32({:#x}u)", v & 0xffff),
            _ => {
                let hi = sel.unwrap_or_else(|| hsel(ins, i)) == 2;
                if hi { format!("f16tof32(({}) >> 16)", r(o)?) } else { format!("f16tof32(({}) & 0xffffu)", r(o)?) }
            }
        };
        Ok(format!("tb_hin({base}, tb_hw1)"))
    }

    fn vop3_mods(&self, ins: &Insn, i: usize, e: String) -> String {
        let mut e = e;
        if ins.enc == Enc::Vop3 {
            if (ins.aux.mask >> i) & 1 != 0 {
                e = format!("abs({e})");
            }
            if (ins.aux.mask >> (3 + i)) & 1 != 0 {
                e = format!("(-{e})");
            }
        }
        e
    }

    fn hres_mode(&self, ins: &Insn, bits: String) -> String {
        if ins.enc != Enc::Vop3 || (ins.aux.imm2 == 0 && ins.aux.imm == 0) {
            return bits;
        }
        let mut e = format!("f16tof32({bits})");
        e = match ins.aux.imm2 {
            1 => format!("(({e}) * 2.0)"),
            2 => format!("(({e}) * 4.0)"),
            3 => format!("(({e}) * 0.5)"),
            _ => e,
        };
        if ins.aux.imm != 0 {
            e = format!("saturate({e})");
        }
        format!("tb_f2h({e})")
    }

    pub(super) fn mode_own(&mut self, ins: &Insn) -> Result<bool, String> {
        let m = ins.mnemonic.as_str();
        let u = self.mode_use;
        let f16_ops = matches!(
            m,
            "v_add_f16" | "v_sub_f16" | "v_subrev_f16" | "v_mul_f16" | "v_fma_f16" | "v_fmac_f16" | "v_fmamk_f16" | "v_fmaak_f16" | "v_ldexp_f16" | "v_cvt_f16_f32"
                | "v_cvt_f16_i16" | "v_cvt_f16_u16" | "v_pk_add_f16" | "v_pk_mul_f16" | "v_pk_fma_f16" | "v_pk_fmac_f16" | "v_fma_mixlo_f16" | "v_fma_mixhi_f16"
        );
        let f64_ops = matches!(m, "v_add_f64" | "v_mul_f64" | "v_fma_f64" | "v_ldexp_f64" | "v_div_fmas_f64");
        let f32_ops = matches!(m, "v_fma_mix_f32" | "v_cvt_f32_f64" | "v_mullit_f32" | "v_ldexp_f32");
        if !((f16_ops && u.dp) || (f64_ops && u.dpr) || (f32_ops && u.sp)) {
            return Ok(false);
        }
        if ins.aux.sdwa.is_some() {
            return Err(format!("{m}: SDWA operand selection under a shader-written MODE is not supported"));
        }
        self.hw_regs.insert(1);
        let s = &ins.src;
        let d = ins.dst.first().cloned().ok_or("no dst")?;
        let t = self.tmp;
        self.tmp += 1;
        if f64_ops {
            self.uses_f64 = true;
            for o in s.iter().chain(ins.dst.iter()) {
                if matches!(o, Opnd::Vgpr(_) | Opnd::Sgpr(_) | Opnd::VccLo | Opnd::ExecLo) {
                    self.touch(o);
                    self.touch(&hi64(o));
                }
            }
            let a = |i: usize| -> Result<String, String> { Ok(self.vop3_mods(ins, i, d64(s.get(i).ok_or("missing operand")?)?)) };
            let e = match m {
                "v_add_f64" => format!("tb_d_add({}, {}, tb_hw1)", a(0)?, a(1)?),
                "v_mul_f64" => format!("tb_d_mul({}, {}, tb_hw1)", a(0)?, a(1)?),
                "v_fma_f64" => format!("tb_d_fma({}, {}, {}, tb_hw1)", a(0)?, a(1)?, a(2)?),
                "v_ldexp_f64" => format!("tb_d_ldexp({}, asint({}), tb_hw1)", a(0)?, r(&s[1])?),
                _ => format!("tb_d_fmas({}, {}, {}, vcc_lo, tb_hw1)", a(0)?, a(1)?, a(2)?),
            };
            let mut e = e;
            if ins.enc == Enc::Vop3 {
                e = match ins.aux.imm2 {
                    1 => format!("(({e}) * 2.0L)"),
                    2 => format!("(({e}) * 4.0L)"),
                    3 => format!("(({e}) * 0.5L)"),
                    _ => e,
                };
                if ins.aux.imm != 0 {
                    e = format!("clamp({e}, 0.0L, 1.0L)");
                }
            }
            self.line(&format!("double t{t} = {e}; uint t{t}l, t{t}h; asuint(t{t}, t{t}l, t{t}h);"));
            self.wv(&d, &format!("t{t}l"))?;
            self.wv(&hi64(&d), &format!("t{t}h"))?;
            return Ok(true);
        }
        if f32_ops {
            let e = match m {
                "v_cvt_f32_f64" => {
                    self.uses_f64 = true;
                    self.touch(&s[0]);
                    self.touch(&hi64(&s[0]));
                    let x = self.vop3_mods(ins, 0, d64(&s[0])?);
                    format!("tb_m_cvtd({x}, tb_hw1)")
                }
                "v_ldexp_f32" => format!("tb_m_ldexp({}, asint({}), tb_hw1)", din(self.fm(ins, 0)?), r(&s[1])?),
                "v_mullit_f32" => {
                    let (a, b, c) = (din(self.fm(ins, 0)?), din(self.fm(ins, 1)?), din(self.fm(ins, 2)?));
                    self.line(&format!("float t{t}a = {a}, t{t}b = {b}, t{t}c = {c};"));
                    format!("((asuint(t{t}b) == 0xff7fffffu || asuint(t{t}b) == 0xff800000u || tb_nan(t{t}b) || (asuint(t{t}c) & 0x7fffffffu) == 0u || (asuint(t{t}c) >> 31) != 0u || tb_nan(t{t}c)) ? -3.402823466e+38 : tb_m_mul(t{t}a, t{t}b, tb_hw1))")
                }
                _ => {
                    let v = self.mix_vals(ins)?;
                    let e = format!("tb_m_fma({}, {}, {}, tb_hw1)", v[0], v[1], v[2]);
                    let clamp = (ins.raw[0] >> 15) & 1 != 0;
                    return self.wv(&d, &format!("asuint({})", if clamp { format!("saturate({e})") } else { e })).map(|_| true);
                }
            };
            let e = self.fres(ins, dout(e));
            self.wv(&d, &e)?;
            return Ok(true);
        }
        let h = |me: &Self, i: usize| -> Result<String, String> { Ok(me.vop3_mods(ins, i, me.hval(ins, i, None)?)) };
        let bits = match m {
            "v_add_f16" => format!("tb_h_add({}, {}, tb_hw1)", h(self, 0)?, h(self, 1)?),
            "v_sub_f16" => format!("tb_h_add({}, (-{}), tb_hw1)", h(self, 0)?, h(self, 1)?),
            "v_subrev_f16" => format!("tb_h_add({}, (-{}), tb_hw1)", h(self, 1)?, h(self, 0)?),
            "v_mul_f16" => format!("tb_h_mul({}, {}, tb_hw1)", h(self, 0)?, h(self, 1)?),
            "v_fma_f16" => format!("tb_h_fma({}, {}, {}, tb_hw1)", h(self, 0)?, h(self, 1)?, h(self, 2)?),
            "v_ldexp_f16" => format!("tb_h_cvt((double)({}) * tb_p2(clamp(asint(({}) << 16) >> 16, -64, 64)), tb_hw1, asuint({}) >> 31)", h(self, 0)?, r(&s[1])?, h(self, 0)?),
            "v_cvt_f16_f32" => {
                let x = din(self.fm(ins, 0)?);
                format!("tb_h_cvt((double)({x}), tb_hw1, asuint({x}) >> 31)")
            }
            "v_cvt_f16_i16" => format!("tb_h_cvt((double)(asint(({}) << 16) >> 16), tb_hw1, 0u)", r(&s[0])?),
            "v_cvt_f16_u16" => format!("tb_h_cvt((double)(({}) & 0xffffu), tb_hw1, 0u)", r(&s[0])?),
            "v_fmac_f16" => {
                self.touch(&d);
                let hi = matches!(self.d16, Some((_, true)));
                let acc = format!("tb_hin(f16tof32(({}){}), tb_hw1)", r(&d)?, if hi { " >> 16" } else { " & 0xffffu" });
                format!("tb_h_fma({}, {}, {acc}, tb_hw1)", h(self, 0)?, h(self, 1)?)
            }
            "v_fmamk_f16" | "v_fmaak_f16" => {
                let k = format!("tb_hin(f16tof32({:#x}u), tb_hw1)", ins.raw.last().copied().unwrap_or(0) & 0xffff);
                let (a, b) = (h(self, 0)?, h(self, 1)?);
                if m == "v_fmaak_f16" { format!("tb_h_fma({a}, {b}, {k}, tb_hw1)") } else { format!("tb_h_fma({a}, {k}, {b}, tb_hw1)") }
            }
            "v_fma_mixlo_f16" | "v_fma_mixhi_f16" => {
                let v = self.mix_vals(ins)?;
                let mut b = format!("tb_h_fma({}, {}, {}, tb_hw1)", v[0], v[1], v[2]);
                if (ins.raw[0] >> 15) & 1 != 0 {
                    b = format!("tb_f2h(saturate(f16tof32({b})))");
                }
                self.touch(&d);
                let dn = r(&d)?;
                let e = if m == "v_fma_mixlo_f16" { format!("(({dn}) & 0xffff0000u) | {b}") } else { format!("(({dn}) & 0xffffu) | (({b}) << 16)") };
                self.wv(&d, &e)?;
                return Ok(true);
            }
            _ => {
                let sel = pk_sel(ins);
                let (w0, w1) = (ins.raw[0], ins.raw.get(1).copied().unwrap_or(0));
                let neg = [(w1 >> 29) & 7, (w0 >> 8) & 7];
                let nsrc = if m == "v_pk_fma_f16" { 3 } else { 2 };
                if m == "v_pk_fmac_f16" {
                    self.touch(&d);
                }
                let mut halves = Vec::new();
                for hh in 0..2usize {
                    let mut v = Vec::new();
                    for i in 0..nsrc {
                        let o = &s[i];
                        let hi = if m == "v_pk_fmac_f16" { hh == 1 } else { (sel[hh] >> i) & 1 != 0 };
                        let base = match o {
                            Opnd::ConstF32(_) => f(o)?,
                            _ => {
                                let x = r(o)?;
                                if hi { format!("f16tof32(({x}) >> 16)") } else { format!("f16tof32(({x}) & 0xffffu)") }
                            }
                        };
                        let mut e = format!("tb_hin({base}, tb_hw1)");
                        if m != "v_pk_fmac_f16" && (neg[hh] >> i) & 1 != 0 {
                            e = format!("(-{e})");
                        }
                        v.push(e);
                    }
                    if m == "v_pk_fmac_f16" {
                        let x = r(&d)?;
                        v.push(format!("tb_hin({}, tb_hw1)", if hh == 1 { format!("f16tof32(({x}) >> 16)") } else { format!("f16tof32(({x}) & 0xffffu)") }));
                    }
                    let mut b = match m {
                        "v_pk_add_f16" => format!("tb_h_add({}, {}, tb_hw1)", v[0], v[1]),
                        "v_pk_mul_f16" => format!("tb_h_mul({}, {}, tb_hw1)", v[0], v[1]),
                        _ => format!("tb_h_fma({}, {}, {}, tb_hw1)", v[0], v[1], v[2]),
                    };
                    if ins.enc == Enc::Vop3 && (w0 >> 15) & 1 != 0 {
                        b = format!("tb_f2h(saturate(f16tof32({b})))");
                    }
                    halves.push(b);
                }
                self.wv(&d, &format!("(({}) | (({}) << 16))", halves[0], halves[1]))?;
                return Ok(true);
            }
        };
        let b = self.hres_mode(ins, bits);
        self.wv(&d, &b)?;
        Ok(true)
    }

    fn mix_vals(&self, ins: &Insn) -> Result<Vec<String>, String> {
        let (w0, w1) = (ins.raw[0], ins.raw.get(1).copied().unwrap_or(0));
        let opsel = (w0 >> 11) & 7;
        let opsel_hi = ((w1 >> 27) & 3) | (((w0 >> 14) & 1) << 2);
        let (neg, abs) = ((w1 >> 29) & 7, (w0 >> 8) & 7);
        let mut v = Vec::new();
        for i in 0..3 {
            let x = r(&ins.src[i])?;
            let mut e = if (opsel_hi >> i) & 1 != 0 {
                let b = if (opsel >> i) & 1 != 0 { format!("f16tof32(({x}) >> 16)") } else { format!("f16tof32(({x}) & 0xffffu)") };
                format!("tb_hin({b}, tb_hw1)")
            } else {
                din(format!("asfloat({x})"))
            };
            if (abs >> i) & 1 != 0 {
                e = format!("abs({e})");
            }
            if (neg >> i) & 1 != 0 {
                e = format!("(-{e})");
            }
            v.push(e);
        }
        Ok(v)
    }
}
