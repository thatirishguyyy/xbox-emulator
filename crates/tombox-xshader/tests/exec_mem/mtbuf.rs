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
    for (op, f) in [(4u32, 3u32), (4, 36), (8, 75), (12, 9), (0, 0), (0, 78)] {
        let mut w = prog(&[vop1(1, 1, ic(0)), vop1(1, 10, ic(0))]);
        w.extend_from_slice(&mtbuf(op, f, 0, true, 1, 10, 0x80));
        w.push(S_ENDPGM);
        assert!(translate(&cs_extract(&w, 1)).is_err(), "op {op} fmt {f} should be refused");
    }
}

fn f2h_exact(x: u32) -> u32 {
    let s = (x >> 16) & 0x8000;
    let e = ((x >> 23) & 0xff) as i32;
    let m = x & 0x7f_ffff;
    if e == 0xff {
        return s | 0x7c00 | (m >> 13);
    }
    if e == 0 && m == 0 {
        return s;
    }
    assert!(e != 0, "f32 denormal {x:#x}");
    let he = e - 127 + 15;
    assert!(he < 31, "{x:#x} overflows f16");
    if he >= 1 {
        assert!(m & 0x1fff == 0, "{x:#x} inexact in f16");
        return s | ((he as u32) << 10) | (m >> 13);
    }
    let sh = (14 - he) as u32;
    let full = m | 0x80_0000;
    assert!(full & ((1 << sh) - 1) == 0, "{x:#x} inexact in f16");
    s | (full >> sh)
}

fn d16_comp(c: u32, bits: u32, num: N) -> Option<Option<u32>> {
    Some(match num {
        N::F if bits < 32 => {
            let r = conv(c, bits, N::F);
            (!f32::from_bits(r).is_nan()).then(|| f2h_exact(r))
        }
        N::Ui if bits <= 16 => Some(c),
        N::Si if bits <= 16 => Some(sx(c, bits) as u32 & 0xffff),
        N::Us | N::Ss if bits <= 11 => Some(f2h_exact(conv(c, bits, num))),
        _ => return None,
    })
}

fn load_ref_d16(init: &[u32], at: usize, f: u32, n: u32) -> Option<Vec<Option<u32>>> {
    let (fields, len, num) = fmt_ref(f);
    let x = elem(&bytes_of(init), at, len);
    let mut h = Vec::new();
    for k in 0..n as usize {
        h.push(if k < fields.len() {
            let (o, b) = fields[k];
            d16_comp(((x >> o) as u64 & mk(b)) as u32, b, num)?
        } else if k == 3 {
            Some(if matches!(num, N::Ui | N::Si) { 1 } else { 0x3c00 })
        } else {
            Some(0)
        });
    }
    Some((0..n as usize).step_by(2).map(|k| if k + 1 < n as usize { Some(h[k]? | (h[k + 1]? << 16)) } else { h[k] }).collect())
}

const D16_LOADS: [&str; 4] = ["tbuffer_load_format_d16_x", "tbuffer_load_format_d16_xy", "tbuffer_load_format_d16_xyz", "tbuffer_load_format_d16_xyzw"];

fn load_words_d16(f: u32, n: u32, at: u32, mode: u32) -> ([u32; 2], Vec<u32>) {
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
    (op, w)
}

#[test]
fn mtbuf_d16_load_formats() {
    let mut cs = Vec::new();
    let mut refused = 0;
    for f in 1..=77u32 {
        let (fields, len, _) = fmt_ref(f);
        let aligned = len.min(4);
        let nf = fields.len() as u32;
        let mut plan: Vec<(u32, u32, u32)> = (1..=4u32).map(|n| (n, 8, 0)).collect();
        plan.extend([(nf, 16 + aligned, 1), (4, 3 * aligned + 4, 2), (4, 5, 0)]);
        for (n, at, mode) in plan {
            let (op, words) = load_words_d16(f, n, at, mode);
            let init = data();
            match load_ref_d16(&init, at as usize, f, n) {
                Some(want) => {
                    let mem = want.iter().enumerate().filter_map(|(j, v)| v.map(|v| ((OUT / 4) as usize + j, v))).collect();
                    cs.push(CsCase { m: D16_LOADS[n as usize - 1], op: op.to_vec(), words, threads: 1, mem, init });
                }
                None => {
                    let e = translate(&cs_extract(&words, 1)).err().unwrap_or_default();
                    assert!(e.contains("d16"), "fmt {f} n {n}: d16 load should be refused, got {e:?}");
                    refused += 1;
                }
            }
        }
    }
    assert!(cs.len() > 150 && refused > 150, "{} cases, {refused} refused", cs.len());
    run_cs_cases("mtbuf_d16_load", &cs);
}

fn vals16(num: N, k: u32) -> u32 {
    match num {
        N::Un => [0x3400, 0x38cd, 0x3e00, 0xb4cd][k as usize],
        N::Sn => [0xb8cd, 0x359a, 0xc000, 0x7e00][k as usize],
        N::Ui => [3, 0xffff, 0x100, 0x8000][k as usize],
        N::Si => [0xffff, 0x7fff, 0x8000, 0x0080][k as usize],
        _ => [0x3e00, 0xc080, 0x7bff, 0x0001][k as usize],
    }
}

fn pack_ref_d16(h: u32, bits: u32, n: N) -> Option<u64> {
    match n {
        N::F if bits == 16 => Some(h as u64),
        N::F if bits == 32 => Some(h2f(h).to_bits() as u64),
        N::Un | N::Sn => pack_ref(h2f(h).to_bits(), bits, n),
        N::Ui => pack_ref(h, bits, n),
        N::Si => pack_ref(sx(h, 16) as u32, bits, n),
        _ => None,
    }
}

const D16_STORES: [&str; 4] = ["tbuffer_store_format_d16_x", "tbuffer_store_format_d16_xy", "tbuffer_store_format_d16_xyz", "tbuffer_store_format_d16_xyzw"];

fn store_case_d16(f: u32, n: u32, at: u32, offen: bool) -> Result<CsCase, Vec<u32>> {
    let (fields, len, num) = fmt_ref(f);
    let nf = fields.len() as u32;
    let h: Vec<u32> = (0..n).map(|k| vals16(num, k)).collect();
    let op = mtbuf(12 + n - 1, f, if offen { 0 } else { at }, offen, 1, 10, 0x80);
    let mut w = prog(&[vop1(1, 1, LIT), at]);
    for j in 0..n.div_ceil(2) {
        let hi = h.get(2 * j as usize + 1).copied().unwrap_or(0xdead);
        w.extend_from_slice(&[vop1(1, 10 + j, LIT), h[2 * j as usize] | (hi << 16)]);
    }
    w.extend_from_slice(&op);
    w.push(S_ENDPGM);
    let w_n = n.min(nf);
    let mut packed = 0u128;
    for k in 0..w_n {
        let (o, b) = fields[k as usize];
        match pack_ref_d16(h[k as usize], b, num) {
            Some(p) => packed |= (p as u128) << o,
            None => return Err(w),
        }
    }
    let mut bytes = bytes_of(&data());
    for i in 0..len as usize {
        bytes[at as usize + i] = (packed >> (8 * i)) as u8;
    }
    let mem = (0..16usize).map(|i| (i, u32::from_le_bytes(bytes[4 * i..4 * i + 4].try_into().unwrap()))).collect();
    Ok(CsCase { m: D16_STORES[n as usize - 1], op: op.to_vec(), words: w, threads: 1, mem, init: data() })
}

#[test]
fn mtbuf_d16_store_formats() {
    let mut cs = Vec::new();
    let mut refused = 0;
    for f in 1..=77u32 {
        let (_, len, _) = fmt_ref(f);
        for n in 1..=4u32 {
            for (at, offen) in [(8u32, false), (8 + len.min(4), true), (13, true)] {
                match store_case_d16(f, n, at, offen) {
                    Ok(c) => cs.push(c),
                    Err(w) => {
                        assert!(translate(&cs_extract(&w, 1)).is_err(), "fmt {f} n {n}: d16 store should be refused");
                        refused += 1;
                    }
                }
            }
        }
    }
    assert!(cs.len() > 300 && refused > 0, "{} cases, {refused} refused", cs.len());
    run_cs_cases("mtbuf_d16_store", &cs);
}
