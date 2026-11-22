use super::*;

const F: [f32; 12] = [1.0, -2.5, 0.375, 3.0e7, -1.0e-3, 65504.0, 1.0e-40, -7.0e-39, 0.0, -0.0, f32::INFINITY, f32::NEG_INFINITY];
const U: [u32; 10] = [0, 1, 2, 31, 0x7fff_ffff, 0x8000_0000, 0xffff_ffff, 0x1234_5678, 0xfedc_ba98, 0x0080_0001];

fn qnan(x: f32) -> u32 {
    if x.is_nan() { 0x7fc0_0000 } else { x.to_bits() }
}

fn v2(c: &mut Cases, m: &'static str, op: u32, a: u32, b: u32, exp: u32) {
    vc(c, m, &[(1, a), (2, b)], &[vop2(op, 10, v(1), 2)], &[10], &[exp]);
}

fn fmin(a: f32, b: f32) -> f32 {
    if a.is_nan() { b } else if b.is_nan() { a } else if a == b { if a.is_sign_negative() { a } else { b } } else if a < b { a } else { b }
}

fn fmax(a: f32, b: f32) -> f32 {
    if a.is_nan() { b } else if b.is_nan() { a } else if a == b { if a.is_sign_negative() { b } else { a } } else if a > b { a } else { b }
}

fn legacy_mul(a: f32, b: f32) -> f32 {
    if a == 0.0 || b == 0.0 { 0.0 } else { a * b }
}

fn ftz(x: f32) -> f32 {
    if x != 0.0 && x.abs() < f32::MIN_POSITIVE { if x < 0.0 { -0.0 } else { 0.0 } } else { x }
}

#[test]
fn vop2_basic() {
    let mut c = Cases::new();
    for &a in &F {
        for &b in &F {
            let (x, y) = (fb(a), fb(b));
            v2(&mut c, "v_sub_f32", 0x04, x, y, qnan(a - b));
            v2(&mut c, "v_subrev_f32", 0x05, x, y, qnan(b - a));
            v2(&mut c, "v_mul_legacy_f32", 0x07, x, y, qnan(legacy_mul(a, b)));
            if a != b || a != 0.0 {
                v2(&mut c, "v_min_f32", 0x0f, x, y, qnan(fmin(a, b)));
                v2(&mut c, "v_max_f32", 0x10, x, y, qnan(fmax(a, b)));
            }
        }
    }
    v2(&mut c, "v_min_f32", 0x0f, 0x7fc0_0000, fb(2.0), fb(2.0));
    v2(&mut c, "v_max_f32", 0x10, fb(-3.0), 0x7fc0_0000, fb(-3.0));
    for &a in &U {
        for &b in &U {
            v2(&mut c, "v_mul_u32_u24", 0x0b, a, b, (a & 0xff_ffff).wrapping_mul(b & 0xff_ffff));
            v2(&mut c, "v_min_i32", 0x11, a, b, (a as i32).min(b as i32) as u32);
            v2(&mut c, "v_max_i32", 0x12, a, b, (a as i32).max(b as i32) as u32);
            v2(&mut c, "v_min_u32", 0x13, a, b, a.min(b));
            v2(&mut c, "v_max_u32", 0x14, a, b, a.max(b));
            v2(&mut c, "v_lshrrev_b32", 0x16, a, b, b >> (a & 31));
            v2(&mut c, "v_ashrrev_i32", 0x18, a, b, ((b as i32) >> (a & 31)) as u32);
            v2(&mut c, "v_lshlrev_b32", 0x1a, a, b, b << (a & 31));
            v2(&mut c, "v_and_b32", 0x1b, a, b, a & b);
            v2(&mut c, "v_or_b32", 0x1c, a, b, a | b);
            v2(&mut c, "v_xor_b32", 0x1d, a, b, a ^ b);
            v2(&mut c, "v_sub_nc_u32", 0x26, a, b, a.wrapping_sub(b));
            v2(&mut c, "v_subrev_nc_u32", 0x27, a, b, b.wrapping_sub(a));
        }
    }
    for &(a, b) in &[(5u32, 7u32), (0xffff_ffff, 0), (0xffff_ffff, 1), (0x8000_0000, 0x8000_0000), (0, 0)] {
        for cin in 0..2u32 {
            let s = a as u64 + b as u64 + cin as u64;
            let mut pre = pr();
            pre.vv(1, a).vv(2, b).sv(106, cin).sv(107, 0);
            let mut post = pr();
            post.raw(&[vop1(1, 0, v(10))]).vcc_to_v(1);
            c.add("v_add_co_ci_u32", &pre, &[vop2(0x28, 10, v(1), 2)], &post, &[s as u32, (s >> 32) as u32]);
            let d = (b as u64).wrapping_sub(a as u64).wrapping_sub(cin as u64);
            let borrow = ((a as u64 + cin as u64) > b as u64) as u32;
            let mut post = pr();
            post.raw(&[vop1(1, 0, v(10))]).vcc_to_v(1);
            c.add("v_subrev_co_ci_u32", &pre, &[vop2(0x2a, 10, v(1), 2)], &post, &[d as u32, borrow]);
        }
    }
    for &(a, b) in &[(0x1111u32, 0x2222u32), (0xdead_beef, 0x0bad_f00d)] {
        for vcc in 0..2u32 {
            let mut pre = pr();
            pre.vv(1, a).vv(2, b).sv(106, vcc).sv(107, 0);
            let mut post = pr();
            post.raw(&[vop1(1, 0, v(10))]);
            c.add("v_cndmask_b32", &pre, &[vop2(0x01, 10, v(1), 2)], &post, &[if vcc != 0 { b } else { a }]);
            let mut post = pr();
            post.raw(&[vop1(1, 0, v(10))]);
            let e = vop3(0x101, 10, [v(1), v(2), 20]);
            let mut pre3 = pr();
            pre3.vv(1, a).vv(2, b).sv(20, vcc).sv(21, 0);
            c.add("v_cndmask_b32", &pre3, &e, &post, &[if vcc != 0 { b } else { a }]);
        }
    }
    for &(a, b) in &[(1.0f32, -2.0f32), (1.0 + 2f32.powi(-12), 3.0), (65520.0, -70000.0), (1.0e-6, 6.0e-8), (f32::INFINITY, f32::NEG_INFINITY), (0.1, 0.2), (-0.0, 0.0)] {
        let rz = |x: f32| {
            let h = f2h(x);
            if h & 0x7c00 == 0x7c00 && !x.is_infinite() { (h & 0x8000) | 0x7bff } else if h2f(h).abs() > x.abs() { h - 1 } else { h }
        };
        v2(&mut c, "v_cvt_pkrtz_f16_f32", 0x2f, fb(a), fb(b), rz(a) | (rz(b) << 16));
    }
    let hv: [f32; 7] = [1.0, -0.5, 2.25, 1000.0, -65504.0, 0.00006103515625, 3.0];
    for &a in &hv {
        for &b in &hv {
            let (x, y) = (0x7777_0000 | f2h(a), 0x9999_0000 | f2h(b));
            v2(&mut c, "v_sub_f16", 0x33, x, y, D16 | f2h(a - b));
            v2(&mut c, "v_mul_f16", 0x35, x, y, D16 | f2h(a * b));
            v2(&mut c, "v_max_f16", 0x39, x, y, D16 | f2h(fmax(a, b)));
            v2(&mut c, "v_min_f16", 0x3a, x, y, D16 | f2h(fmin(a, b)));
        }
    }
    c.run_both("vop2_basic");
}

fn rne(x: f32) -> f32 {
    x.round_ties_even()
}

#[test]
fn vop1_basic() {
    let mut c = Cases::new();
    for &a in &U {
        v1c(&mut c, "v_cvt_f32_i32", 0x05, a, fb(a as i32 as f32));
        v1c(&mut c, "v_cvt_f32_u32", 0x06, a, fb(a as f32));
        v1c(&mut c, "v_cvt_f32_ubyte0", 0x11, a, fb((a & 0xff) as f32));
        v1c(&mut c, "v_cvt_f32_ubyte1", 0x12, a, fb(((a >> 8) & 0xff) as f32));
        v1c(&mut c, "v_cvt_f32_ubyte2", 0x13, a, fb(((a >> 16) & 0xff) as f32));
        v1c(&mut c, "v_cvt_f32_ubyte3", 0x14, a, fb((a >> 24) as f32));
        v1c(&mut c, "v_not_b32", 0x37, a, !a);
        v1c(&mut c, "v_bfrev_b32", 0x38, a, a.reverse_bits());
    }
    let fs: [f32; 16] = [0.0, -0.0, 0.5, -0.5, 1.5, 2.5, -2.5, 3.7, -3.7, 4.0e9, -3.0e9, 1.0e-40, 123456.79, f32::INFINITY, f32::NEG_INFINITY, f32::NAN];
    for &a in &fs {
        v1c(&mut c, "v_cvt_u32_f32", 0x07, fb(a), a as u32);
        v1c(&mut c, "v_cvt_i32_f32", 0x08, fb(a), a as i32 as u32);
        v1c(&mut c, "v_trunc_f32", 0x21, fb(a), qnan(a.trunc()));
        v1c(&mut c, "v_ceil_f32", 0x22, fb(a), qnan(a.ceil()));
        v1c(&mut c, "v_rndne_f32", 0x23, fb(a), qnan(rne(a)));
        v1c(&mut c, "v_floor_f32", 0x24, fb(a), qnan(a.floor()));
        let fr = if a.is_nan() || a.is_infinite() { f32::NAN } else { (a - a.floor()).min(f32::from_bits(0x3f7f_ffff)) };
        v1c(&mut c, "v_fract_f32", 0x20, fb(a), qnan(fr));
    }
    v1c(&mut c, "v_fract_f32", 0x20, fb(-1.0e-10), 0x3f7f_ffff);
    for &h in &[0x3c00u32, 0xc100, 0x0001, 0x8200, 0x7bff, 0x7c00, 0xfc00, 0x3555, 0x0000, 0x8000] {
        v1c(&mut c, "v_cvt_f32_f16", 0x0b, 0x5a5a_0000 | h, fb(h2f(h)));
    }
    for &(x, r) in &[(0.0f32, 1.0f32), (3.0, 8.0), (-2.0, 0.25), (10.0, 1024.0), (f32::NEG_INFINITY, 0.0), (f32::INFINITY, f32::INFINITY), (-0.0, 1.0)] {
        v1c(&mut c, "v_exp_f32", 0x25, fb(x), fb(r));
    }
    for &(x, r) in &[(1.0f32, 0.0f32), (8.0, 3.0), (0.25, -2.0), (1024.0, 10.0), (0.0, f32::NEG_INFINITY), (f32::INFINITY, f32::INFINITY), (-0.0, f32::NEG_INFINITY)] {
        v1c(&mut c, "v_log_f32", 0x27, fb(x), fb(r));
    }
    v1c(&mut c, "v_log_f32", 0x27, fb(-1.0), 0x7fc0_0000);
    for &(x, r) in &[(1.0f32, 1.0f32), (2.0, 0.5), (-4.0, -0.25), (0.0, f32::INFINITY), (-0.0, f32::NEG_INFINITY), (f32::INFINITY, 0.0), (0.125, 8.0)] {
        v1c(&mut c, "v_rcp_f32", 0x2a, fb(x), fb(r));
        v1c(&mut c, "v_rcp_iflag_f32", 0x2b, fb(x), fb(r));
    }
    for &(x, r) in &[(1.0f32, 1.0f32), (4.0, 0.5), (0.25, 2.0), (0.0, f32::INFINITY), (f32::INFINITY, 0.0), (16.0, 0.25)] {
        v1c(&mut c, "v_rsq_f32", 0x2e, fb(x), fb(r));
    }
    v1c(&mut c, "v_rsq_f32", 0x2e, fb(-1.0), 0x7fc0_0000);
    for &(x, r) in &[(4.0f32, 2.0f32), (0.0, 0.0), (-0.0, -0.0), (2.25, 1.5), (f32::INFINITY, f32::INFINITY), (1.0e10, 1.0e5), (0.0625, 0.25)] {
        v1c(&mut c, "v_sqrt_f32", 0x33, fb(x), fb(r));
    }
    v1c(&mut c, "v_sqrt_f32", 0x33, fb(-4.0), 0x7fc0_0000);
    for &(x, s, co) in &[(0.0f32, 0.0f32, 1.0f32), (0.25, 1.0, 0.0), (-0.25, -1.0, 0.0), (0.5, 0.0, -1.0), (1.0, 0.0, 1.0)] {
        if s == 0.0 && x != 0.0 {
            v1c(&mut c, "v_cos_f32", 0x36, fb(x), fb(co));
        } else if co == 0.0 {
            v1c(&mut c, "v_sin_f32", 0x35, fb(x), fb(s));
        } else {
            v1c(&mut c, "v_sin_f32", 0x35, fb(x), fb(s));
            v1c(&mut c, "v_cos_f32", 0x36, fb(x), fb(co));
        }
    }
    for &(h, r) in &[(0x3c00u32, 0x3c00u32), (0x4000, 0x3800), (0xc400, 0xb400), (0x0000, 0x7c00), (0x8000, 0xfc00), (0x7c00, 0x0000), (0x3400, 0x4400)] {
        v16(&mut c, "v_rcp_f16", 0x54, 0x1234_0000 | h, r);
    }
    for &x in &[1.5f32, -1.5, 2.0, -0.25, 0.75, 1000.5, -1000.5, 0.0] {
        v16(&mut c, "v_floor_f16", 0x5b, 0x4321_0000 | f2h(x), f2h(x.floor()));
        v16(&mut c, "v_ceil_f16", 0x5c, 0x4321_0000 | f2h(x), f2h(x.ceil()));
    }
    v16(&mut c, "v_ceil_f16", 0x5c, 0x4321_8001, 0x8000);
    v16(&mut c, "v_ceil_f16", 0x5c, 0x4321_0001, 0x3c00);
    for &(a, b) in &[(0x1111_1111u32, 0x2222_2222u32), (0, 0xffff_ffff)] {
        let mut pre = pr();
        pre.vv(1, a).vv(10, b);
        let mut post = pr();
        post.raw(&[vop1(1, 0, v(10)), vop1(1, 1, v(1))]);
        c.add("v_swap_b32", &pre, &[vop1(0x65, 10, v(1))], &post, &[a, b]);
    }
    for k in 0..4u32 {
        let mut pre = pr();
        for r in 0..4u32 {
            pre.vv(20 + r, 0x100 * (r + 1));
        }
        pre.vv(10, 0).sv(124, k);
        let mut post = pr();
        post.raw(&[vop1(1, 0, v(10))]);
        c.add("v_movrels_b32", &pre, &[vop1(0x43, 10, v(20))], &post, &[0x100 * (k + 1)]);
        let mut pre = pr();
        for r in 0..4u32 {
            pre.vv(20 + r, 0x5a);
        }
        pre.vv(1, 0xabc0 + k).sv(124, k);
        let mut post = pr();
        post.raw(&[vop1(1, 0, v(20)), vop1(1, 1, v(21)), vop1(1, 2, v(22)), vop1(1, 3, v(23))]);
        let exp: Vec<u32> = (0..4).map(|r| if r == k { 0xabc0 + k } else { 0x5a }).collect();
        c.add("v_movreld_b32", &pre, &[vop1(0x42, 20, v(1))], &post, &exp);
    }
    for &a in &[0x1234_5678u32, 0] {
        let mut pre = pr();
        pre.vv(1, a).sv(30, 0x5555);
        let mut post = pr();
        post.s2v(0, 30);
        c.add("v_readfirstlane_b32", &pre, &[vop1(0x02, 30, v(1))], &post, &[a]);
        let mut pre = pr();
        pre.vv(1, a).vv(10, 0x77);
        let mut post = pr();
        post.raw(&[vop1(1, 0, v(10)), vop1(1, 1, v(1))]);
        c.add("v_nop", &pre, &[vop1(0x00, 0, 0)], &post, &[0x77, a]);
    }
    c.run_both("vop1_basic");
}

fn cube(x: f32, y: f32, z: f32) -> [f32; 4] {
    let (ax, ay, az) = (x.abs(), y.abs(), z.abs());
    if az >= ax && az >= ay {
        [if z < 0.0 { 5.0 } else { 4.0 }, if z < 0.0 { -x } else { x }, -y, 2.0 * z]
    } else if ay >= ax {
        [if y < 0.0 { 3.0 } else { 2.0 }, x, if y < 0.0 { -z } else { z }, 2.0 * y]
    } else {
        [if x < 0.0 { 1.0 } else { 0.0 }, if x < 0.0 { z } else { -z }, -y, 2.0 * x]
    }
}

fn perm_byte(s0: u32, s1: u32, sel: u32) -> u32 {
    let both = ((s0 as u64) << 32) | s1 as u64;
    match sel {
        0..=7 => ((both >> (sel * 8)) & 0xff) as u32,
        8 => ((s1 >> 15) & 1) * 0xff,
        9 => ((s1 >> 31) & 1) * 0xff,
        10 => ((s0 >> 15) & 1) * 0xff,
        11 => ((s0 >> 31) & 1) * 0xff,
        12 => 0,
        _ => 0xff,
    }
}

fn med3(a: f32, b: f32, cc: f32) -> f32 {
    fmax(fmin(a, b), fmin(fmax(a, b), cc))
}

fn bfe(a: u32, o: u32, w: u32, signed: bool) -> u32 {
    let (o, w) = (o & 31, w & 31);
    if w == 0 {
        return 0;
    }
    if signed {
        let x = (a as i32) >> o;
        if w + o >= 32 { x as u32 } else { (((x as u32) << (32 - w)) as i32 >> (32 - w)) as u32 }
    } else {
        (a >> o) & if w == 32 { u32::MAX } else { (1 << w) - 1 }
    }
}

fn sbfe(a: u32, b: u32, signed: bool) -> u32 {
    let (o, w) = (b & 31, (b >> 16) & 127);
    if w == 0 {
        return 0;
    }
    let x = if signed { ((a as i32) >> o) as u32 } else { a >> o };
    if w >= 32 {
        return x;
    }
    let v = x & ((1 << w) - 1);
    if signed && (v >> (w - 1)) & 1 != 0 { v | !((1 << w) - 1) } else { v }
}

#[test]
fn vop3_basic() {
    let mut c = Cases::new();
    let s24 = |x: u32| ((x << 8) as i32 >> 8) as i64;
    for &a in &U {
        for &b in &[1u32, 0x00ff_ffff, 0x0080_0000, 0xffff_fff0, 7] {
            let cc = a.rotate_left(7) ^ b;
            v3c(&mut c, "v_mad_i32_i24", 0x142, a, b, cc, (s24(a) * s24(b) + cc as i32 as i64) as u32);
            v3c(&mut c, "v_mad_u32_u24", 0x143, a, b, cc, (a & 0xff_ffff).wrapping_mul(b & 0xff_ffff).wrapping_add(cc));
            v3c(&mut c, "v_bfe_u32", 0x148, a, b & 31, b >> 3, bfe(a, b, b >> 3, false));
            v3c(&mut c, "v_bfe_i32", 0x149, a, b & 31, b >> 3, bfe(a, b, b >> 3, true));
            v3c(&mut c, "v_alignbit_b32", 0x14e, a, b, cc, ((((a as u64) << 32) | b as u64) >> (cc & 31)) as u32);
            v3c(&mut c, "v_min3_i32", 0x152, a, b, cc, (a as i32).min(b as i32).min(cc as i32) as u32);
            v3c(&mut c, "v_max3_i32", 0x155, a, b, cc, (a as i32).max(b as i32).max(cc as i32) as u32);
            let mut s = [a as i32, b as i32, cc as i32];
            s.sort();
            v3c(&mut c, "v_med3_i32", 0x158, a, b, cc, s[1] as u32);
            v3c(&mut c, "v_mul_lo_u32", 0x169, a, b, 0, a.wrapping_mul(b));
            v3c(&mut c, "v_mul_hi_u32", 0x16a, a, b, 0, ((a as u64 * b as u64) >> 32) as u32);
            v3c(&mut c, "v_mul_hi_i32", 0x16c, a, b, 0, ((a as i32 as i64 * b as i32 as i64) >> 32) as u32);
            v3c(&mut c, "v_xad_u32", 0x345, a, b, cc, (a ^ b).wrapping_add(cc));
            v3c(&mut c, "v_lshl_add_u32", 0x346, a, b, cc, (a << (b & 31)).wrapping_add(cc));
            v3c(&mut c, "v_add_lshl_u32", 0x347, a, b, cc, a.wrapping_add(b) << (cc & 31));
            v3c(&mut c, "v_add3_u32", 0x36d, a, b, cc, a.wrapping_add(b).wrapping_add(cc));
            v3c(&mut c, "v_lshl_or_b32", 0x36f, a, b, cc, (a << (b & 31)) | cc);
            v3c(&mut c, "v_and_or_b32", 0x371, a, b, cc, (a & b) | cc);
            v3c(&mut c, "v_or3_b32", 0x372, a, b, cc, a | b | cc);
            v3c(&mut c, "v_bcnt_u32_b32", 0x364, a, b, 0, a.count_ones().wrapping_add(b));
            let sel = (b.wrapping_mul(0x0705_0301) ^ cc) & 0x0f0f_0f0f;
            v3c(&mut c, "v_perm_b32", 0x344, a, cc, sel, (0..4).map(|k| perm_byte(a, cc, (sel >> (8 * k)) & 0xff) << (8 * k)).fold(0, |x, y| x | y));
            let s = a as u64 + b as u64;
            let mut pre = pr();
            pre.vv(1, a).vv(2, b).sv(20, 0x5555).sv(21, 0x5555);
            let mut post = pr();
            post.raw(&[vop1(1, 0, v(10))]).s2v(1, 20);
            c.add("v_add_co_u32", &pre, &vop3b(0x30f, 10, 20, [v(1), v(2), 0]), &post, &[s as u32, (s >> 32) as u32]);
        }
        for &sh in &[0u32, 1, 31, 32, 33, 63, 64] {
            let x = ((a as u64) << 32) | (a.rotate_left(13) as u64);
            let r = x << (sh & 63);
            vc(&mut c, "v_lshlrev_b64", &[(1, sh), (2, x as u32), (3, (x >> 32) as u32)], &vop3(0x2ff, 10, [v(1), v(2), 0]), &[10, 11], &[r as u32, (r >> 32) as u32]);
        }
    }
    for sel in [0x0c0c_0c0cu32, 0x0d0e_0f10, 0x0809_0a0b, 0x0706_0504, 0x0302_0100] {
        v3c(&mut c, "v_perm_b32", 0x344, 0x8001_7f80, 0x0080_8000, sel, (0..4).map(|k| perm_byte(0x8001_7f80, 0x0080_8000, (sel >> (8 * k)) & 0xff) << (8 * k)).fold(0, |x, y| x | y));
    }
    let fv: [f32; 9] = [1.0, -2.5, 0.375, 3.0e7, -1.0e-3, 0.0, 7.5, f32::INFINITY, f32::NEG_INFINITY];
    for (i, &a) in fv.iter().enumerate() {
        for (j, &b) in fv.iter().enumerate() {
            let cc = fv[(i * 3 + j * 5 + 1) % fv.len()];
            let (x, y, z) = (fb(a), fb(b), fb(cc));
            v3c(&mut c, "v_mad_legacy_f32", 0x140, x, y, z, qnan(ftz(ftz(legacy_mul(a, b)) + cc)));
            v3c(&mut c, "v_min3_f32", 0x151, x, y, z, qnan(fmin(fmin(a, b), cc)));
            v3c(&mut c, "v_max3_f32", 0x154, x, y, z, qnan(fmax(fmax(a, b), cc)));
            if !(a == 0.0 && b == 0.0) {
                v3c(&mut c, "v_med3_f32", 0x157, x, y, z, qnan(med3(a, b, cc)));
            }
            if a.is_finite() && b.is_finite() && cc.is_finite() && a != b && b != cc && a != cc && a.abs() != b.abs() && b.abs() != cc.abs() && a.abs() != cc.abs() {
                let r = cube(a, b, cc);
                v3c(&mut c, "v_cubeid_f32", 0x144, x, y, z, fb(r[0]));
                v3c(&mut c, "v_cubesc_f32", 0x145, x, y, z, fb(r[1]));
                v3c(&mut c, "v_cubetc_f32", 0x146, x, y, z, fb(r[2]));
                v3c(&mut c, "v_cubema_f32", 0x147, x, y, z, fb(r[3]));
            }
        }
    }
    let hv: [f32; 6] = [1.0, -0.5, 2.25, 1000.0, -65504.0, 3.0];
    for (i, &a) in hv.iter().enumerate() {
        for (j, &b) in hv.iter().enumerate() {
            let cc = hv[(i + 2 * j + 1) % hv.len()];
            let (x, y, z) = (0x1111_0000 | f2h(a), 0x2222_0000 | f2h(b), 0x3333_0000 | f2h(cc));
            vc(&mut c, "v_min3_f16", &[(1, x), (2, y), (3, z)], &vop3(0x351, 10, [v(1), v(2), v(3)]), &[10], &[D16 | f2h(fmin(fmin(a, b), cc))]);
            vc(&mut c, "v_max3_f16", &[(1, x), (2, y), (3, z)], &vop3(0x354, 10, [v(1), v(2), v(3)]), &[10], &[D16 | f2h(fmax(fmax(a, b), cc))]);
            vc(&mut c, "v_med3_f16", &[(1, x), (2, y), (3, z)], &vop3(0x357, 10, [v(1), v(2), v(3)]), &[10], &[D16 | f2h(med3(a, b, cc))]);
            let f = h_round(a as f64 * b as f64 + cc as f64, 0);
            vc(&mut c, "v_fma_f16", &[(1, x), (2, y), (3, z)], &vop3(0x34b, 10, [v(1), v(2), v(3)]), &[10], &[D16 | f]);
            let (p, q, r) = (f2h(a) | (f2h(b) << 16), f2h(b) | (f2h(cc) << 16), f2h(cc) | (f2h(a) << 16));
            let lo = h_round(a as f64 * b as f64 + cc as f64, 0);
            let hi = h_round(b as f64 * cc as f64 + a as f64, 0);
            let pf = vop3p(0x0e, 10, [v(1), v(2), v(3)], 0, 7, 0, 0);
            vc(&mut c, "v_pk_fma_f16", &[(1, p), (2, q), (3, r)], &pf, &[10], &[lo | (hi << 16)]);
            let pm = vop3p(0x10, 10, [v(1), v(2), 0], 0, 3, 0, 0);
            vc(&mut c, "v_pk_mul_f16", &[(1, p), (2, q)], &pm, &[10], &[h_round(a as f64 * b as f64, 0) | (h_round(b as f64 * cc as f64, 0) << 16)]);
            let pn = vop3p(0x11, 10, [v(1), v(2), 0], 0, 3, 0, 0);
            vc(&mut c, "v_pk_min_f16", &[(1, p), (2, q)], &pn, &[10], &[f2h(fmin(a, b)) | (f2h(fmin(b, cc)) << 16)]);
            let px = vop3p(0x12, 10, [v(1), v(2), 0], 0, 3, 0, 0);
            vc(&mut c, "v_pk_max_f16", &[(1, p), (2, q)], &px, &[10], &[f2h(fmax(a, b)) | (f2h(fmax(b, cc)) << 16)]);
            let mf = vop3p(0x20, 10, [v(1), v(2), v(3)], 0, 7, 0, 0);
            vc(&mut c, "v_fma_mix_f32", &[(1, p), (2, q), (3, r)], &mf, &[10], &[fm(a, b, cc)]);
            let ms = vop3p(0x20, 10, [v(1), v(2), v(3)], 7, 7, 0, 0);
            vc(&mut c, "v_fma_mix_f32", &[(1, p), (2, q), (3, r)], &ms, &[10], &[fm(b, cc, a)]);
            let ml = vop3p(0x21, 10, [v(1), v(2), v(3)], 0, 7, 0, 0);
            vc(&mut c, "v_fma_mixlo_f16", &[(1, p), (2, q), (3, r)], &ml, &[10], &[D16 | lo]);
            let mh = vop3p(0x22, 10, [v(1), v(2), v(3)], 0, 7, 0, 0);
            vc(&mut c, "v_fma_mixhi_f16", &[(1, p), (2, q), (3, r)], &mh, &[10], &[(D16 & 0xffff) | (lo << 16)]);
        }
    }
    let m32 = vop3p(0x20, 10, [v(1), v(2), v(3)], 0, 0, 0, 0);
    let x32 = [(1, fb(1.0 + 2f32.powi(-23))), (2, fb(1.0 - 2f32.powi(-24))), (3, fb(-1.0))];
    vc(&mut c, "v_fma_mix_f32", &x32, &m32, &[10], &[fm(1.0 + 2f32.powi(-23), 1.0 - 2f32.powi(-24), -1.0)]);
    vc(&mut c, "v_fma_mixlo_f16", &x32, &vop3p(0x21, 10, [v(1), v(2), v(3)], 0, 0, 0, 0), &[10], &[D16 | 1]);
    vc(&mut c, "v_fma_mixhi_f16", &x32, &vop3p(0x22, 10, [v(1), v(2), v(3)], 0, 0, 0, 0), &[10], &[(D16 & 0xffff) | (1 << 16)]);
    let x16 = [(1, 0x1111_4149), (2, 0x2222_3e2d), (3, 0x3333_0563)];
    vc(&mut c, "v_fma_mixlo_f16", &x16, &vop3p(0x21, 10, [v(1), v(2), v(3)], 0, 7, 0, 0), &[10], &[D16 | 0x4415]);
    vc(&mut c, "v_fma_mixhi_f16", &x16, &vop3p(0x22, 10, [v(1), v(2), v(3)], 0, 7, 0, 0), &[10], &[(D16 & 0xffff) | (0x4415 << 16)]);
    vc(&mut c, "v_fma_mixlo_f16", &[(1, 0x1111_4149), (2, 0x2222_3e2d), (3, fb(8.219480514526367e-05))], &vop3p(0x21, 10, [v(1), v(2), v(3)], 0, 3, 0, 0), &[10], &[D16 | 0x4415]);
    vc(&mut c, "v_fma_mixlo_f16", &[(1, fb(2f32.powi(-60))), (2, fb(1.0)), (3, fb(1.0 + 2f32.powi(-11)))], &vop3p(0x21, 10, [v(1), v(2), v(3)], 0, 0, 0, 0), &[10], &[D16 | 0x3c01]);
    let (hx, hk) = (0x3e01, 0xc080);
    vc(&mut c, "v_fmac_f16", &[(1, hx), (2, hx), (10, D16 | hk)], &[vop2(0x36, 10, v(1), 2)], &[10], &[D16 | 0x1a00]);
    vc(&mut c, "v_fmac_f16", &[(1, hx), (2, hx), (10, D16 | hk)], &vop3(0x136, 10, [v(1), v(2), 0]), &[10], &[D16 | 0x1a00]);
    vc(&mut c, "v_fmamk_f16", &[(1, hx), (2, hk)], &[vop2(0x37, 10, v(1), 2), hx], &[10], &[D16 | 0x1a00]);
    vc(&mut c, "v_fmaak_f16", &[(1, hx), (2, hx)], &[vop2(0x38, 10, v(1), 2), hk], &[10], &[D16 | 0x1a00]);
    c.run_both("vop3_basic");
}

#[test]
fn salu_basic() {
    let mut c = Cases::new();
    let w: [u32; 8] = [0, 1, 5, 31, 0x7fff_ffff, 0x8000_0000, 0xffff_ffff, 0x1234_5678];
    for &a in &w {
        for &b in &w {
            let s = a as u64 + b as u64;
            s2(&mut c, "s_add_u32", 0x00, a, b, 0, s as u32, (s >> 32) as u32);
            for cin in 0..2u32 {
                let s = a as u64 + b as u64 + cin as u64;
                s2(&mut c, "s_addc_u32", 0x04, a, b, cin, s as u32, (s >> 32) as u32);
                s2(&mut c, "s_cselect_b32", 0x0a, a, b, cin, if cin != 0 { a } else { b }, cin);
            }
            s2(&mut c, "s_and_b32", 0x0e, a, b, 0, a & b, nz((a & b) as u64));
            s2(&mut c, "s_or_b32", 0x10, a, b, 0, a | b, nz((a | b) as u64));
            s2(&mut c, "s_xor_b32", 0x12, a, b, 0, a ^ b, nz((a ^ b) as u64));
            s2(&mut c, "s_lshl_b32", 0x1e, a, b, 0, a << (b & 31), nz((a << (b & 31)) as u64));
            s2(&mut c, "s_lshr_b32", 0x20, a, b, 0, a >> (b & 31), nz((a >> (b & 31)) as u64));
            let r = ((a as i32) >> (b & 31)) as u32;
            s2(&mut c, "s_ashr_i32", 0x22, a, b, 0, r, nz(r as u64));
            s2(&mut c, "s_mul_i32", 0x26, a, b, 1, a.wrapping_mul(b), 1);
            let bb = (b & 31) | ((a.rotate_left(5) & 0x3f) << 16);
            let r = sbfe(a, bb, false);
            s2(&mut c, "s_bfe_u32", 0x27, a, bb, 0, r, nz(r as u64));
            let r = sbfe(a, bb, true);
            s2(&mut c, "s_bfe_i32", 0x28, a, bb, 0, r, nz(r as u64));
            scmp(&mut c, "s_cmp_gt_i32", 0x02, a as u64, b as u64, ((a as i32) > (b as i32)) as u32);
            scmp(&mut c, "s_cmp_ge_i32", 0x03, a as u64, b as u64, ((a as i32) >= (b as i32)) as u32);
            scmp(&mut c, "s_cmp_lt_i32", 0x04, a as u64, b as u64, ((a as i32) < (b as i32)) as u32);
            scmp(&mut c, "s_cmp_eq_u32", 0x06, a as u64, b as u64, (a == b) as u32);
            scmp(&mut c, "s_cmp_lg_u32", 0x07, a as u64, b as u64, (a != b) as u32);
            scmp(&mut c, "s_cmp_gt_u32", 0x08, a as u64, b as u64, (a > b) as u32);
            scmp(&mut c, "s_cmp_ge_u32", 0x09, a as u64, b as u64, (a >= b) as u32);
            scmp(&mut c, "s_cmp_lt_u32", 0x0a, a as u64, b as u64, (a < b) as u32);
            scmp(&mut c, "s_cmp_le_u32", 0x0b, a as u64, b as u64, (a <= b) as u32);
        }
        s1(&mut c, "s_mov_b32", 0x03, a, 0x5555, 1, a, 1);
        s1(&mut c, "s_brev_b32", 0x0b, a, 0x5555, 1, a.reverse_bits(), 1);
        for &d0 in &[0u32, 0xffff_ffff, 0x8000_0001] {
            s1(&mut c, "s_bitset0_b32", 0x1b, a, d0, 1, d0 & !(1 << (a & 31)), 1);
        }
    }
    let q: [u64; 6] = [0, 1, 0x8000_0000_0000_0000, u64::MAX, 0x0123_4567_89ab_cdef, 0x0000_0001_0000_0000];
    for &a in &q {
        for &b in &q {
            for cin in 0..2u32 {
                s2w(&mut c, "s_cselect_b64", 0x0b, a, b, cin, if cin != 0 { a } else { b }, cin);
            }
            s2w(&mut c, "s_and_b64", 0x0f, a, b, 0, a & b, nz(a & b));
            s2w(&mut c, "s_or_b64", 0x11, a, b, 0, a | b, nz(a | b));
            s2w(&mut c, "s_xor_b64", 0x13, a, b, 0, a ^ b, nz(a ^ b));
            s2w(&mut c, "s_andn2_b64", 0x15, a, b, 0, a & !b, nz(a & !b));
            let sh = b & 0x7f;
            s2w(&mut c, "s_lshl_b64", 0x1f, a, sh, 0, a << (sh & 63), nz(a << (sh & 63)));
            let (bw, bo) = ((a as u32) & 0x7f, (b as u32) & 0x7f);
            let r = if bw & 63 == 0 { 0 } else { (if bw & 63 == 0 { 0 } else { u64::MAX >> (64 - (bw & 63)) }) << (bo & 63) };
            s2w(&mut c, "s_bfm_b64", 0x25, bw as u64, bo as u64, 1, r, 1);
        }
        s1w(&mut c, "s_mov_b64", 0x04, a, 0x5555, 1, a, 1);
        let wq = (wqm(a as u32) as u64) | ((wqm((a >> 32) as u32) as u64) << 32);
        s1w(&mut c, "s_wqm_b64", 0x0a, a, 0x5555, 0, wq, nz(wq));
        s1n(&mut c, "s_bcnt1_i32_b64", 0x10, a, 0, a.count_ones(), nz(a.count_ones() as u64));
    }
    for &d0 in &[0u32, 5, 0x7fff_ffff, 0xffff_fff0, 0x8000_0000] {
        for &imm in &[0u16, 5, 0x7fff, 0x8000, 0xfff0] {
            let si = imm as i16 as i32;
            sk(&mut c, "s_movk_i32", 0x00, d0, imm, 1, si as u32, 1);
            sk(&mut c, "s_cmpk_eq_i32", 0x03, d0, imm, 0, d0, ((d0 as i32) == si) as u32);
            sk(&mut c, "s_cmpk_gt_u32", 0x0b, d0, imm, 0, d0, (d0 > imm as u32) as u32);
            let (r, o) = (d0 as i32).overflowing_add(si);
            sk(&mut c, "s_addk_i32", 0x0f, d0, imm, 0, r as u32, o as u32);
        }
    }
    let ops: [(&'static str, u32, fn(u32, u32) -> u32); 2] = [("s_and_saveexec_b64", 0x24, |a, e| a & e), ("s_nand_saveexec_b64", 0x29, |a, e| !(a & e))];
    for (m, op, f) in ops {
        for a in [0u32, 1, 0xffff_fffe, 0xffff_ffff] {
            saveexec(&mut c, m, op, a, f);
        }
    }
    let mut pre = pr();
    pre.sv(30, 0x5555);
    let mut post = pr();
    post.raw(&[sop2(0x01, 40, 22, 20)]).s2v(0, 40).raw(&[sop2(0x01, 41, 23, 21)]).s2v(1, 41);
    c.add("s_getpc_b64", &pre, &[sop1(0x1f, 20, 0), sop1(0x1f, 22, 0)], &post, &[4, 0]);
    c.run_both("salu_basic");
}
