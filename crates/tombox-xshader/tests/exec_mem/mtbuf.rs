use super::*;

pub fn mtbuf(op: u32, fmt: u32, offset: u32, offen: bool, vaddr: u32, vdata: u32, soff: u32) -> [u32; 2] {
    [
        0xe800_0000 | (offset & 0xfff) | ((offen as u32) << 12) | ((op & 7) << 16) | (fmt << 19),
        vaddr | (vdata << 8) | (2 << 16) | (((op >> 3) & 1) << 21) | (soff << 24),
    ]
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum N {
    Un,
    Sn,
    Us,
    Ss,
    Ui,
    Si,
    F,
}

fn fmt_ref(f: u32) -> (Vec<(u32, u32)>, u32, N) {
    let n6 = [N::Un, N::Sn, N::Us, N::Ss, N::Ui, N::Si];
    let n7 = [N::Un, N::Sn, N::Us, N::Ss, N::Ui, N::Si, N::F];
    let n3 = [N::Ui, N::Si, N::F];
    let g: [(u32, &[u32], &[N]); 14] = [
        (1, &[8], &n6),
        (7, &[16], &n7),
        (14, &[8, 8], &n6),
        (20, &[32], &n3),
        (23, &[16, 16], &n7),
        (30, &[11, 11, 10], &n7),
        (37, &[10, 11, 11], &n7),
        (44, &[2, 10, 10, 10], &n6),
        (50, &[10, 10, 10, 2], &n6),
        (56, &[8, 8, 8, 8], &n6),
        (62, &[32, 32], &n3),
        (65, &[16, 16, 16, 16], &n7),
        (72, &[32, 32, 32], &n3),
        (75, &[32, 32, 32, 32], &n3),
    ];
    for (first, widths, nums) in g {
        if f >= first && f < first + nums.len() as u32 {
            let mut off = 0;
            let mut fields = Vec::new();
            for &w in widths {
                fields.push((off, w));
                off += w;
            }
            return (fields, off / 8, nums[(f - first) as usize]);
        }
    }
    unreachable!()
}

fn mk(bits: u32) -> u64 {
    if bits >= 32 { 0xffff_ffff } else { (1u64 << bits) - 1 }
}

fn sx(v: u32, bits: u32) -> i32 {
    if bits >= 32 { v as i32 } else { ((v << (32 - bits)) as i32) >> (32 - bits) }
}

fn h2f(h: u32) -> f32 {
    let s = (h >> 15) & 1;
    let e = (h >> 10) & 31;
    let m = h & 0x3ff;
    let mag = if e == 0 {
        m as f32 * (2.0f32).powi(-24)
    } else if e == 31 {
        f32::from_bits(0x7f80_0000 | (m << 13))
    } else {
        f32::from_bits(((e + 112) << 23) | (m << 13))
    };
    if s == 1 { -mag } else { mag }
}

fn uf(v: u32, mb: u32) -> u32 {
    let e = (v >> mb) & 31;
    let m = v & ((1 << mb) - 1);
    if e == 0 {
        (m as f32 * (2.0f32).powi(-14 - mb as i32)).to_bits()
    } else if e == 31 {
        0x7f80_0000 | (m << (23 - mb))
    } else {
        ((e + 112) << 23) | (m << (23 - mb))
    }
}

fn conv(v: u32, bits: u32, n: N) -> u32 {
    let mx = mk(bits) as f32;
    match n {
        N::Un => (v as f32 / mx).to_bits(),
        N::Sn => (sx(v, bits) as f32 / mk(bits - 1) as f32).max(-1.0).to_bits(),
        N::Us => (v as f32).to_bits(),
        N::Ss => (sx(v, bits) as f32).to_bits(),
        N::Ui => v,
        N::Si => sx(v, bits) as u32,
        N::F => match bits {
            32 => v,
            16 => h2f(v).to_bits(),
            11 => uf(v, 6),
            _ => uf(v, 5),
        },
    }
}

fn bytes_of(init: &[u32]) -> Vec<u8> {
    init.iter().flat_map(|w| w.to_le_bytes()).collect()
}

fn elem(b: &[u8], at: usize, len: u32) -> u128 {
    let mut x = 0u128;
    for i in 0..len as usize {
        x |= (b[at + i] as u128) << (8 * i);
    }
    x
}

fn load_ref(init: &[u32], at: usize, f: u32, n: u32) -> Vec<Option<u32>> {
    let (fields, len, num) = fmt_ref(f);
    let x = elem(&bytes_of(init), at, len);
    (0..n)
        .map(|k| {
            if (k as usize) < fields.len() {
                let (o, b) = fields[k as usize];
                let r = conv(((x >> o) as u64 & mk(b)) as u32, b, num);
                (!(num == N::F && b < 32 && f32::from_bits(r).is_nan())).then_some(r)
            } else if k == 3 {
                Some(if matches!(num, N::Ui | N::Si) { 1 } else { 0x3f80_0000 })
            } else {
                Some(0)
            }
        })
        .collect()
}

fn data() -> Vec<u32> {
    (0..16u32).map(|i| (i.wrapping_mul(0x9e37_79b9) ^ 0x3c5a_96e1).rotate_left(i * 5)).collect()
}

const OUT: u32 = 1024;

fn store_out(vdata: u32, k: u32) -> [u32; 2] {
    [0xe000_0000 | (0x1c << 18) | (OUT + 4 * k), (vdata << 8) | (2 << 16) | (0x80 << 24)]
}

fn sop1_mov(s: u32) -> u32 {
    0xbe80_0300 | (s << 16) | 0xff
}

fn prog(pre: &[u32]) -> Vec<u32> {
    let mut w = smem(2, 8, 4, 0, SNULL).to_vec();
    w.extend_from_slice(pre);
    w
}

fn load_case(f: u32, n: u32, at: u32, mode: u32) -> CsCase {
    let init = data();
    let (vofs, imm, soff) = match mode {
        0 => (at, 0, 0x80),
        1 => (0, at, 0x80),
        _ => (1, 1, 0),
    };
    let op = mtbuf(n - 1, f, imm, true, 1, 10, soff);
    let mut w = prog(&[vop1(1, 1, LIT), vofs]);
    if soff == 0 {
        w.extend_from_slice(&[sop1_mov(0), at - 2]);
    }
    w.extend_from_slice(&op);
    for k in 0..n {
        w.extend_from_slice(&store_out(10 + k, k));
    }
    w.push(S_ENDPGM);
    let want = load_ref(&init, at as usize, f, n);
    let names = ["tbuffer_load_format_x", "tbuffer_load_format_xy", "tbuffer_load_format_xyz", "tbuffer_load_format_xyzw"];
    let mem = want.iter().enumerate().filter_map(|(k, v)| v.map(|v| ((OUT / 4) as usize + k, v))).collect();
    CsCase { m: names[n as usize - 1], op: op.to_vec(), words: w, threads: 1, mem, init }
}

#[test]
fn mtbuf_load_formats() {
    let mut cs = Vec::new();
    for f in 1..=77u32 {
        let (fields, len, _) = fmt_ref(f);
        let aligned = len.min(4);
        for n in 1..=4u32 {
            cs.push(load_case(f, n, 8, 0));
        }
        cs.push(load_case(f, fields.len() as u32, 16 + aligned, 1));
        cs.push(load_case(f, 4, 3 * aligned + 4, 2));
        cs.push(load_case(f, 4, 5, 0));
    }
    run_cs_cases("mtbuf_load", &cs);
}

fn f2h(x: f32) -> u32 {
    let b = x.to_bits();
    let s = (b >> 16) & 0x8000;
    let e = ((b >> 23) & 0xff) as i32 - 127 + 15;
    let m = b & 0x7f_ffff;
    assert!(e > 0 && e < 31 && m & 0x1fff == 0);
    s | ((e as u32) << 10) | (m >> 13)
}

fn pack_ref(v: u32, bits: u32, n: N) -> Option<u64> {
    let mx = mk(bits);
    let x = f32::from_bits(v);
    Some(match n {
        N::Un => {
            if x.is_nan() { 0 } else { (x.clamp(0.0, 1.0) * mx as f32).round_ties_even() as u64 }
        }
        N::Sn => {
            if x.is_nan() { 0 } else { ((x.clamp(-1.0, 1.0) * mk(bits - 1) as f32).round() as i64 as u64) & mx }
        }
        N::Ui => (v as u64).min(mx),
        N::Si if bits == 32 => v as u64,
        N::Si => ((v as i32 as i64).clamp(-(1i64 << (bits - 1)), (1i64 << (bits - 1)) - 1) as u64) & mx,
        N::F if bits == 32 => v as u64,
        N::F if bits == 16 => f2h(x) as u64,
        _ => return None,
    })
}

fn vals(num: N, bits: u32, k: u32) -> u32 {
    match num {
        N::Un => [0.25f32, 0.6, 1.5, -0.3][k as usize].to_bits(),
        N::Sn => [-0.6f32, 0.35, -2.0, f32::NAN][k as usize].to_bits(),
        N::Ui => [3u32, 0x1_0000, 7, 0xffff_ffff][k as usize],
        N::Si => [(-1i32) as u32, 0x7fff_0000, (-100000i32) as u32, 5][k as usize],
        N::F if bits == 32 => [0x4049_0fdbu32, 0xc000_0000, 0x7fc0_0001, 0x0000_0003][k as usize],
        _ => [1.5f32, -2.25, 65504.0, 2.0f32.powi(-14)][k as usize].to_bits(),
    }
}

fn store_case(f: u32, n: u32, at: u32, offen: bool) -> Option<CsCase> {
    let (fields, len, num) = fmt_ref(f);
    let w_n = n.min(fields.len() as u32);
    let mut bytes = bytes_of(&data());
    let mut packed = 0u128;
    for k in 0..w_n {
        let (o, b) = fields[k as usize];
        packed |= (pack_ref(vals(num, b, k), b, num)? as u128) << o;
    }
    for i in 0..len as usize {
        bytes[at as usize + i] = (packed >> (8 * i)) as u8;
    }
    let op = mtbuf(4 + n - 1, f, if offen { 0 } else { at }, offen, 1, 10, 0x80);
    let mut w = prog(&[vop1(1, 1, LIT), at]);
    for k in 0..n {
        w.extend_from_slice(&[vop1(1, 10 + k, LIT), vals(num, fields[k.min(fields.len() as u32 - 1) as usize].1, k)]);
    }
    w.extend_from_slice(&op);
    w.push(S_ENDPGM);
    let names = ["tbuffer_store_format_x", "tbuffer_store_format_xy", "tbuffer_store_format_xyz", "tbuffer_store_format_xyzw"];
    let mem = (0..16usize).map(|i| (i, u32::from_le_bytes(bytes[4 * i..4 * i + 4].try_into().unwrap()))).collect();
    Some(CsCase { m: names[n as usize - 1], op: op.to_vec(), words: w, threads: 1, mem, init: data() })
}

#[test]
fn mtbuf_store_formats() {
    let mut cs = Vec::new();
    for f in 1..=77u32 {
        let (_, len, _) = fmt_ref(f);
        for n in 1..=4u32 {
            for (at, offen) in [(8u32, false), (8 + len.min(4), true), (13, true)] {
                if let Some(c) = store_case(f, n, at, offen) {
                    cs.push(c);
                }
            }
        }
    }
    assert!(cs.len() > 400);
    run_cs_cases("mtbuf_store", &cs);
}

fn hw_store(f: u32, n: u32, vals: [u32; 4], want: &[u8]) -> CsCase {
    let op = mtbuf(4 + n - 1, f, 8, false, 1, 10, 0x80);
    let mut w = prog(&[vop1(1, 1, LIT), 8]);
    for k in 0..n {
        w.extend_from_slice(&[vop1(1, 10 + k, LIT), vals[k as usize]]);
    }
    w.extend_from_slice(&op);
    w.push(S_ENDPGM);
    let mut bytes = vec![0xaau8; 64];
    bytes[8..8 + want.len()].copy_from_slice(want);
    let mem = (0..16usize).map(|i| (i, u32::from_le_bytes(bytes[4 * i..4 * i + 4].try_into().unwrap()))).collect();
    let names = ["tbuffer_store_format_x", "tbuffer_store_format_xy", "tbuffer_store_format_xyz", "tbuffer_store_format_xyzw"];
    CsCase { m: names[n as usize - 1], op: op.to_vec(), words: w, threads: 1, mem, init: vec![0xaaaa_aaaa; 16] }
}

#[test]
fn mtbuf_partial_store_zero_fills_like_rdna2() {
    let q = [0.25f32, 0.5, 0.75, 1.0].map(f32::to_bits);
    let rows: [(u32, [&[u8]; 4]); 4] = [
        (56, [&[0x40, 0, 0, 0], &[0x40, 0x80, 0, 0], &[0x40, 0x80, 0xbf, 0], &[0x40, 0x80, 0xbf, 0xff]]),
        (60, [&[0xff, 0, 0, 0], &[0xff, 0xff, 0, 0], &[0xff, 0xff, 0xff, 0], &[0xff, 0xff, 0xff, 0xff]]),
        (65, [&[0, 0x40, 0, 0, 0, 0, 0, 0], &[0, 0x40, 0, 0x80, 0, 0, 0, 0], &[0, 0x40, 0, 0x80, 0xff, 0xbf, 0, 0], &[0, 0x40, 0, 0x80, 0xff, 0xbf, 0xff, 0xff]]),
        (44, [&[0x01, 0, 0, 0], &[0x01, 0x08, 0, 0], &[0x01, 0xf8, 0x2f, 0], &[0x01, 0xf8, 0xef, 0xff]]),
    ];
    let mut cs = Vec::new();
    for (f, want) in rows {
        for n in 1..=4u32 {
            cs.push(hw_store(f, n, q, want[n as usize - 1]));
        }
    }
    run_cs_cases("mtbuf_partial_rdna2", &cs);
}

#[test]
fn mtbuf_snorm2_ties_round_away() {
    let mut cs = Vec::new();
    for (f, at) in [(45u32, 0usize), (51, 3)] {
        let (fields, _, _) = fmt_ref(f);
        for (x, r) in [(0.5f32, 1u64), (-0.5, 3), (0.49999997, 0), (-0.75, 3)] {
            let mut vals = [1.0f32.to_bits(); 4];
            vals[at] = x.to_bits();
            let mut packed = 0u64;
            for (k, &(o, b)) in fields.iter().enumerate() {
                packed |= (if k == at { r } else { mk(b - 1) }) << o;
            }
            cs.push(hw_store(f, 4, vals, &(packed as u32).to_le_bytes()));
        }
    }
    run_cs_cases("mtbuf_snorm2_ties", &cs);
}

#[test]
fn mtbuf_refuses_unlowered() {
    for (op, f) in [(4u32, 3u32), (4, 36), (0, 0), (0, 78), (12, 3), (12, 36), (15, 68), (8, 0)] {
        let mut w = prog(&[vop1(1, 1, ic(0)), vop1(1, 10, ic(0))]);
        w.extend_from_slice(&mtbuf(op, f, 0, true, 1, 10, 0x80));
        w.push(S_ENDPGM);
        assert!(translate(&cs_extract(&w, 1)).is_err(), "op {op} fmt {f} should be refused");
    }
}

fn f16_q24(h: u32) -> u128 {
    let (e, m) = (h >> 10, h & 0x3ff);
    if e == 0 { m as u128 } else { ((m | 0x400) as u128) << (e - 1) }
}

fn q16_ref(neg: bool, num: u64, den: u64) -> u32 {
    let x = (num as u128) << 24;
    let le = |h: u32| f16_q24(h) * den as u128 <= x;
    let (mut lo, mut hi) = (0u32, 0x7bff);
    while lo < hi {
        let mid = (lo + hi + 1) / 2;
        if le(mid) { lo = mid } else { hi = mid - 1 }
    }
    let h = if lo == 0x7bff {
        0x7bff
    } else {
        let dl = x - f16_q24(lo) * den as u128;
        let dh = f16_q24(lo + 1) * den as u128 - x;
        if dl < dh || (dl == dh && lo & 1 == 0) { lo } else { lo + 1 }
    };
    if neg && h != 0 { h | 0x8000 } else { h }
}

fn rtz16_ref(u: u32) -> u32 {
    let x = f32::from_bits(u);
    let s = (u >> 16) & 0x8000;
    if x.is_nan() {
        return s | 0x7c00 | ((u & 0x7f_ffff) >> 13).max(1);
    }
    if x.is_infinite() {
        return s | 0x7c00;
    }
    let a = (x.abs() as f64) * (1u64 << 24) as f64;
    let (mut lo, mut hi) = (0u32, 0x7bff);
    while lo < hi {
        let mid = (lo + hi + 1) / 2;
        if f16_q24(mid) as f64 <= a { lo = mid } else { hi = mid - 1 }
    }
    s | lo
}

fn d16_ref(v: u32, bits: u32, n: N) -> u32 {
    let mx = mk(bits);
    let sv = sx(v, bits) as i64;
    match n {
        N::Un => q16_ref(false, v as u64, mx),
        N::Sn => {
            let mh = mk(bits - 1);
            q16_ref(sv < 0, sv.unsigned_abs().min(mh), mh)
        }
        N::Us => q16_ref(false, v as u64, 1),
        N::Ss => q16_ref(sv < 0, sv.unsigned_abs(), 1),
        N::Ui => v.min(0xffff),
        N::Si => sv.clamp(-32768, 32767) as u32 & 0xffff,
        N::F => match bits {
            32 => rtz16_ref(v),
            16 => v,
            _ => {
                let mb = bits - 5;
                ((v >> mb) << 10) | ((v & ((1 << mb) - 1)) << (10 - mb))
            }
        },
    }
}

fn pack16_ref(h: u32, bits: u32, n: N) -> Option<u64> {
    let mx = mk(bits);
    let x = h2f(h) as f64;
    Some(match n {
        N::Un if x.is_nan() => 0,
        N::Un => (x.clamp(0.0, 1.0) * mx as f64).round_ties_even() as u64,
        N::Sn if x.is_nan() => 0,
        N::Sn => ((x.clamp(-1.0, 1.0) * mk(bits - 1) as f64).round() as i64 as u64) & mx,
        N::Ui => (h as u64).min(mx),
        N::Si => ((h as u16 as i16 as i64).clamp(-(1i64 << (bits - 1)), (1i64 << (bits - 1)) - 1) as u64) & mx,
        N::F if bits == 32 => h2f(h).to_bits() as u64,
        N::F if bits == 16 => h as u64,
        _ => return None,
    })
}

fn store16_ref(f: u32, n: u32, hs: [u32; 4]) -> Option<u128> {
    let (fields, _, num) = fmt_ref(f);
    let mut x = 0u128;
    for (k, &(o, b)) in fields.iter().enumerate() {
        let h = if (k as u32) < n { hs[k] } else { 0 };
        x |= (pack16_ref(h, b, num)? as u128) << o;
    }
    Some(x)
}

fn load16_ref(f: u32, el: u128, one16: bool) -> [u32; 4] {
    let (fields, _, num) = fmt_ref(f);
    core::array::from_fn(|k| match fields.get(k) {
        Some(&(o, b)) => d16_ref(((el >> o) as u64 & mk(b)) as u32, b, num),
        None if k == 3 && one16 => if matches!(num, N::Ui | N::Si) { 1 } else { 0x3c00 },
        None => 0,
    })
}

fn el_of(w: [u32; 4]) -> u128 {
    w.iter().enumerate().fold(0u128, |a, (i, &x)| a | ((x as u128) << (32 * i)))
}

#[test]
fn mtbuf_d16_model_matches_rdna2() {
    use super::mtbuf_hw::{HW_D16_LOADS, HW_D16_STORES};
    let mut bad = Vec::new();
    for (f, el, regs) in HW_D16_LOADS {
        let h = load16_ref(f, el_of(el), false);
        let want = [h[0] | (h[1] << 16), h[2] | (h[3] << 16)];
        if want != regs {
            bad.push(format!("load fmt {f} {el:x?}: model {want:x?} hardware {regs:x?}"));
        }
    }
    for (f, n, hs, el) in HW_D16_STORES {
        let (_, len, _) = fmt_ref(f);
        let m = if len >= 16 { u128::MAX } else { (1u128 << (8 * len)) - 1 };
        let want = store16_ref(f, n, hs).expect("hardware table only has lowered formats");
        if want != el_of(el) & m {
            bad.push(format!("store fmt {f} n {n} {hs:x?}: model {want:x} hardware {:x}", el_of(el) & m));
        }
    }
    assert!(bad.is_empty(), "{} mismatches:\n{}", bad.len(), bad.join("\n"));
    assert!(HW_D16_LOADS.len() > 200 && HW_D16_STORES.len() > 300);
}

fn hash(a: u32, b: u32, c: u32) -> u32 {
    let mut x = a.wrapping_mul(0x9e37_79b9) ^ b.wrapping_mul(0x85eb_ca6b) ^ c.wrapping_mul(0xc2b2_ae35);
    x ^= x >> 15;
    x = x.wrapping_mul(0x2c1b_3c6d);
    x ^ (x >> 12)
}

fn raw_val(f: u32, g: u32, k: u32, bits: u32) -> u32 {
    let mx = mk(bits) as u32;
    if bits <= 8 {
        return (g + 37 * k) & mx;
    }
    if bits < 32 {
        let mut e = vec![0, 1, 2, mx, mx - 1, mx >> 1, (mx >> 1) + 1, (mx >> 1) + 2, mx - 2];
        if bits == 16 {
            e.extend([65519, 65455, 32759, 32777, 65520, 0x7c00, 0x7c01, 0xfc00, 0xfe00, 0x7bff, 0xfbff, 0x3c00, 0x03ff, 0x0400, 0x8001]);
        }
        return if (g as usize) < e.len() { e[(g as usize + k as usize) % e.len()] } else { hash(f, g, k) & mx };
    }
    let fl = [0.0f32, -0.0, 1.0, -1.0, 65504.0, 65505.0, 65519.0, 65519.99, 65520.0, 65536.0, 1e10, -65520.0, f32::INFINITY, f32::NEG_INFINITY, 2f32.powi(-24), 2f32.powi(-25), 3.0 * 2f32.powi(-26), 2f32.powi(-14), 2f32.powi(-15), 1.0 + 2f32.powi(-11), 1.0 + 3.0 * 2f32.powi(-11), 0.1, 2f32.powi(-140)];
    let mut e: Vec<u32> = vec![0, 1, 0x7fff, 0x8000, 0xffff, 0x10000, 0x7fff_ffff, 0x8000_0000, 0xffff_ffff, 0xffff_8000, 0xffff_7fff, 0x7fc0_0000, 0x7f80_0001, 0xffc0_0001, 0x7fa0_0000, 0x807f_ffff];
    e.extend(fl.iter().map(|x| x.to_bits()));
    if (g as usize) < e.len() {
        return e[(g as usize + k as usize) % e.len()];
    }
    let r = hash(f, g, k);
    if g % 2 == 0 { r } else { (r & 0x8000_0000) | ((((r >> 23) % 44) + 100) << 23) | (r & 0x7f_ffff) }
}

const T16: u32 = 64;
const OUT16: u32 = 1024;

fn buf_store_dword(vdata: u32, vaddr: u32, off: u32) -> [u32; 2] {
    [0xe000_0000 | (0x1c << 18) | (1 << 12) | off, vaddr | (vdata << 8) | (2 << 16) | (0x80 << 24)]
}

fn buf_load_dword(vdata: u32, vaddr: u32, off: u32) -> [u32; 2] {
    [0xe000_0000 | (0x0c << 18) | (1 << 12) | off, vaddr | (vdata << 8) | (2 << 16) | (0x80 << 24)]
}

const D16_LOADS: [&str; 4] = ["tbuffer_load_format_d16_x", "tbuffer_load_format_d16_xy", "tbuffer_load_format_d16_xyz", "tbuffer_load_format_d16_xyzw"];
const D16_STORES: [&str; 4] = ["tbuffer_store_format_d16_x", "tbuffer_store_format_d16_xy", "tbuffer_store_format_d16_xyz", "tbuffer_store_format_d16_xyzw"];

fn d16_load_case(f: u32, n: u32, case: u32) -> CsCase {
    let (fields, len, _) = fmt_ref(f);
    let mut bytes = vec![0u8; (T16 * len) as usize];
    let mut mem = Vec::new();
    for i in 0..T16 {
        let g = case * T16 + i;
        let mut el = 0u128;
        for (k, &(o, b)) in fields.iter().enumerate() {
            el |= (raw_val(f, g, k as u32, b) as u128) << o;
        }
        for j in 0..len {
            bytes[(i * len + j) as usize] = (el >> (8 * j)) as u8;
        }
        let h = load16_ref(f, el, true);
        let r0 = if n >= 2 { h[0] | (h[1] << 16) } else { 0xdead_0000 | h[0] };
        let r1 = match n {
            4 => h[2] | (h[3] << 16),
            3 => 0xcafe_0000 | h[2],
            _ => 0xcafe_f00d,
        };
        mem.push(((OUT16 / 4 + 2 * i) as usize, r0));
        mem.push(((OUT16 / 4 + 2 * i + 1) as usize, r1));
    }
    bytes.resize(bytes.len().div_ceil(4) * 4, 0);
    let init: Vec<u32> = bytes.chunks(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
    let op = mtbuf(8 + n - 1, f, 0, true, 1, 10, 0x80);
    let mut w = prog(&[vop2(0x0b, 1, LIT, 0), len, vop2(0x1a, 2, ic(3), 0), vop1(1, 10, LIT), 0xdead_beef, vop1(1, 11, LIT), 0xcafe_f00d]);
    w.extend_from_slice(&op);
    w.extend_from_slice(&buf_store_dword(10, 2, OUT16));
    w.extend_from_slice(&buf_store_dword(11, 2, OUT16 + 4));
    w.push(S_ENDPGM);
    CsCase { m: D16_LOADS[n as usize - 1], op: op.to_vec(), words: w, threads: T16, mem, init }
}

#[test]
fn mtbuf_d16_load_formats() {
    let mut cs = Vec::new();
    for f in 1..=77u32 {
        for case in 0..4 {
            cs.push(d16_load_case(f, 4, case));
        }
        for n in 1..=3 {
            cs.push(d16_load_case(f, n, 0));
        }
    }
    run_cs_cases("mtbuf_d16_load", &cs);
}

fn in16(f: u32, g: u32, k: u32, int: bool) -> u32 {
    let e: &[u32] = if int {
        &[0, 1, 0x7fff, 0x8000, 0xffff, 0xff, 0x100, 0xff80, 0x7f, 0x80, 0x3ff, 0x400, 0xfe00, 0x200, 0x1ff, 0xfdff]
    } else {
        &[0x3800, 0xb800, 0x7c01, 0xfe00, 0x7bff, 0xfbff, 0x8000, 0, 0x3c00, 0xbc00, 0x7c00, 0xfc00, 1, 0x83ff, 0x3555, 0x3bff, 0x2c00, 0x1c04]
    };
    if (g as usize) < e.len() { e[(g as usize + k as usize) % e.len()] } else { hash(f ^ 0x55, g, k) & 0xffff }
}

fn d16_store_case(f: u32, n: u32, case: u32) -> Option<CsCase> {
    let (_, len, num) = fmt_ref(f);
    const OUTS: u32 = 1024;
    let int = matches!(num, N::Ui | N::Si);
    let mut words = vec![0u32; 1024];
    let mut out = vec![0xaau8; (T16 * len) as usize];
    for i in 0..T16 {
        let g = case * T16 + i;
        let hs: [u32; 4] = core::array::from_fn(|k| in16(f, g, k as u32, int));
        words[2 * i as usize] = hs[0] | (hs[1] << 16);
        words[2 * i as usize + 1] = hs[2] | (hs[3] << 16);
        let el = store16_ref(f, n, hs)?;
        for j in 0..len {
            out[(i * len + j) as usize] = (el >> (8 * j)) as u8;
        }
    }
    out.resize(out.len().div_ceil(4) * 4, 0xaa);
    for (j, c) in out.chunks(4).enumerate() {
        words[OUTS as usize / 4 + j] = u32::from_le_bytes(c.try_into().unwrap());
    }
    let mem = (0..out.len() / 4).map(|j| (OUTS as usize / 4 + j, words[OUTS as usize / 4 + j])).collect();
    let mut init = words.clone();
    for w in init[OUTS as usize / 4..].iter_mut().take(out.len() / 4) {
        *w = 0xaaaa_aaaa;
    }
    init.truncate(OUTS as usize / 4 + out.len() / 4);
    let op = mtbuf(12 + n - 1, f, OUTS, true, 3, 10, 0x80);
    let mut w = prog(&[vop2(0x1a, 1, ic(3), 0), vop2(0x0b, 3, LIT, 0), len]);
    w.extend_from_slice(&buf_load_dword(10, 1, 0));
    w.extend_from_slice(&buf_load_dword(11, 1, 4));
    w.extend_from_slice(&op);
    w.push(S_ENDPGM);
    Some(CsCase { m: D16_STORES[n as usize - 1], op: op.to_vec(), words: w, threads: T16, mem, init })
}

#[test]
fn mtbuf_d16_store_formats() {
    let mut cs = Vec::new();
    for f in 1..=77u32 {
        for (n, case) in [(4, 0), (4, 1), (4, 2), (1, 0), (2, 0), (3, 0)] {
            if let Some(c) = d16_store_case(f, n, case) {
                cs.push(c);
            }
        }
    }
    assert!(cs.len() > 250);
    run_cs_cases("mtbuf_d16_store", &cs);
}

fn d16_load_addr_case(f: u32, n: u32, at: u32, mode: u32) -> CsCase {
    let (vofs, imm, soff) = match mode {
        0 => (at, 0, 0x80),
        1 => (0, at, 0x80),
        _ => (1, 1, 0),
    };
    let op = mtbuf(8 + n - 1, f, imm, true, 1, 10, soff);
    let mut w = prog(&[vop1(1, 1, LIT), vofs, vop1(1, 10, LIT), 0x1357_9bdf, vop1(1, 11, LIT), 0x2468_ace0]);
    if soff == 0 {
        w.extend_from_slice(&[sop1_mov(0), at - 2]);
    }
    w.extend_from_slice(&op);
    let nv = n.div_ceil(2);
    if n % 2 == 1 {
        w.extend_from_slice(&[vop2(0x1b, 10 + nv - 1, LIT, 10 + nv - 1), 0xffff]);
    }
    for j in 0..nv {
        w.extend_from_slice(&store_out(10 + j, j));
    }
    w.push(S_ENDPGM);
    let init = data();
    let (_, len, _) = fmt_ref(f);
    let h = load16_ref(f, elem(&bytes_of(&init), at as usize, len), true);
    let mem = (0..nv as usize)
        .map(|j| ((OUT / 4) as usize + j, if 2 * j + 1 < n as usize { h[2 * j] | (h[2 * j + 1] << 16) } else { h[2 * j] }))
        .collect();
    CsCase { m: D16_LOADS[n as usize - 1], op: op.to_vec(), words: w, threads: 1, mem, init }
}

#[test]
fn mtbuf_d16_load_addressing() {
    let mut cs = Vec::new();
    for f in 1..=77u32 {
        let (fields, len, _) = fmt_ref(f);
        let aligned = len.min(4);
        for (n, at, mode) in [(fields.len() as u32, 16 + aligned, 1), (4, 3 * aligned + 4, 2), (4, 5, 0), (1, 8, 1)] {
            cs.push(d16_load_addr_case(f, n, at, mode));
        }
    }
    run_cs_cases("mtbuf_d16_load_addr", &cs);
}

fn d16_store_addr_case(f: u32, n: u32, at: u32, offen: bool) -> Option<CsCase> {
    let (_, len, num) = fmt_ref(f);
    let e: &[u32] = match num {
        N::Un => &[0x3400, 0x38cd, 0x3e00, 0xb4cd],
        N::Sn => &[0xb8cd, 0x359a, 0xc000, 0x7e00],
        N::Ui => &[3, 0xffff, 0x100, 0x8000],
        N::Si => &[0xffff, 0x7fff, 0x8000, 0x0080],
        _ => &[0x3e00, 0xc080, 0x7bff, 0x0001],
    };
    let hs: [u32; 4] = core::array::from_fn(|k| if (k as u32) < n { e[k] } else { 0xdead });
    let el = store16_ref(f, n, hs)?;
    let op = mtbuf(12 + n - 1, f, if offen { 0 } else { at }, offen, 1, 10, 0x80);
    let mut w = prog(&[vop1(1, 1, LIT), at]);
    for j in 0..n.div_ceil(2) as usize {
        w.extend_from_slice(&[vop1(1, 10 + j as u32, LIT), hs[2 * j] | (hs[2 * j + 1] << 16)]);
    }
    w.extend_from_slice(&op);
    w.push(S_ENDPGM);
    let mut bytes = bytes_of(&data());
    for i in 0..len as usize {
        bytes[at as usize + i] = (el >> (8 * i)) as u8;
    }
    let mem = (0..16usize).map(|i| (i, u32::from_le_bytes(bytes[4 * i..4 * i + 4].try_into().unwrap()))).collect();
    Some(CsCase { m: D16_STORES[n as usize - 1], op: op.to_vec(), words: w, threads: 1, mem, init: data() })
}

#[test]
fn mtbuf_d16_store_addressing() {
    let mut cs = Vec::new();
    for f in 1..=77u32 {
        let (_, len, _) = fmt_ref(f);
        for n in 1..=4u32 {
            for (at, offen) in [(8u32, false), (8 + len.min(4), true), (13, true)] {
                cs.extend(d16_store_addr_case(f, n, at, offen));
            }
        }
    }
    assert!(cs.len() > 250, "{} cases", cs.len());
    run_cs_cases("mtbuf_d16_store_addr", &cs);
}
