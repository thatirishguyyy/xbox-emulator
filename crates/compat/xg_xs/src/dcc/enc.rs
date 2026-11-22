use super::bits::seg_info_checked;
use super::dec::{prev, signfix};
use super::px::{get, insert, set};
use super::{Dcc, OBJ};
use crate::layout::E_INVALIDARG;

const E: i32 = E_INVALIDARG;

fn bt(v: u32, i: u32) -> bool {
    (v >> (i & 31)) & 1 != 0
}

fn at(o: usize, n: usize) -> Result<usize, i32> {
    if o.checked_add(n).map_or(false, |e| e <= OBJ) { Ok(o) } else { Err(E) }
}

fn rd(d: &Dcc, o: usize) -> Result<u32, i32> {
    Ok(d.r32(at(o, 4)?))
}

fn rb(d: &Dcc, o: usize) -> Result<u8, i32> {
    Ok(d.m[at(o, 1)?])
}

fn wb(d: &mut Dcc, o: usize, v: u8) -> Result<(), i32> {
    let o = at(o, 1)?;
    d.m[o] = v;
    Ok(())
}

fn bit(d: &Dcc, o: usize, i: u32) -> Result<bool, i32> {
    Ok(bt(rd(d, o + 4 * (i >> 5) as usize)?, i))
}

fn sb(d: &mut Dcc, o: usize, i: u32, on: bool) -> Result<(), i32> {
    let o = at(o + 4 * (i >> 5) as usize, 4)?;
    let w = d.r32(o);
    let m = 1u32 << (i & 31);
    d.w32(o, if on { w | m } else { w & !m });
    Ok(())
}

fn seg(d: &Dcc, k: u32) -> Result<bool, i32> {
    bit(d, 0x5c, k)
}

fn shl8(v: u8, c: u32) -> u8 {
    let c = c & 31;
    if c >= 8 { 0 } else { v << c }
}

fn shr8(v: u8, c: u32) -> u8 {
    let c = c & 31;
    if c >= 8 { 0 } else { v >> c }
}

fn lmask(n: u32) -> u32 {
    if n == 0 { 0 } else { 1u32.wrapping_shl(n).wrapping_sub(1) }
}

fn bitlen(v: u8) -> u8 {
    (8 - v.leading_zeros()) as u8
}

fn eflip(b: u8) -> u8 {
    let s = b >> 7;
    let v = if s != 0 { !b } else { b };
    v.wrapping_add(v) | s
}

fn pop(d: &Dcc, o: usize) -> Result<u32, i32> {
    Ok(rd(d, o)?.count_ones())
}

fn bix(i: u32) -> Result<usize, i32> {
    if i < 256 { Ok(i as usize) } else { Err(E) }
}

pub(crate) struct Tmp {
    b: [u8; 64],
    ever: [bool; 64],
    fill: Option<[u8; 64]>,
    pub(crate) hit: bool,
}

impl Tmp {
    pub(crate) fn new() -> Tmp {
        Tmp::with(None)
    }
    pub(crate) fn with(fill: Option<[u8; 64]>) -> Tmp {
        Tmp { b: [0; 64], ever: [false; 64], fill, hit: false }
    }
}

fn lay_in(d: &mut Dcc, dst: usize, src: usize, maps: &[Option<(u32, u32)>; 4], t: &mut Tmp) -> Result<(), i32> {
    let s = d.b[src];
    for k in 0..4u32 {
        if !seg(d, k)? {
            continue;
        }
        let o = (k * 64) as usize;
        let Some((from, to)) = maps[k as usize] else {
            d.b[dst][o..o + 64].copy_from_slice(&s[o..o + 64]);
            continue;
        };
        for a in o..o + 64 {
            let c = get(d, a as u32, from)?;
            let b = (set(d, &c, to)? & 0x3f) as usize;
            t.b[b] = s[a];
            t.ever[b] = true;
        }
        if t.ever.iter().any(|&x| !x) {
            let Some(v) = t.fill else { return Err(E) };
            for p in 0..64 {
                if !t.ever[p] {
                    t.b[p] = v[p];
                    t.ever[p] = true;
                }
            }
            t.hit = true;
        }
        d.b[dst][o..o + 64].copy_from_slice(&t.b[..]);
    }
    Ok(())
}

pub(crate) fn relay(d: &mut Dcc, dst: usize, src: usize, mode: u32) -> Result<(), i32> {
    relay_in(d, dst, src, mode, &mut Tmp::new())
}

pub(crate) fn relay_in(d: &mut Dcc, dst: usize, src: usize, mode: u32, t: &mut Tmp) -> Result<(), i32> {
    let from = d.r32(0xf0);
    lay_in(d, dst, src, &[Some((from, mode)); 4], t)?;
    d.w32(0xf0, mode);
    Ok(())
}

pub(crate) fn relay_holes(d: &Dcc, mode: u32) -> Vec<usize> {
    let from = d.r32(0xf0);
    let mut ever = [false; 64];
    let mut v = Vec::new();
    for k in 0..4u32 {
        if (d.r32(0x5c) >> k) & 1 == 0 {
            continue;
        }
        for a in k * 64..k * 64 + 64 {
            if let (Ok(c), true) = (get(d, a, from), true) {
                if let Ok(b) = set(d, &c, mode) {
                    ever[(b & 63) as usize] = true;
                }
            }
        }
        for p in 0..64 {
            if !ever[p] {
                v.push((k * 64) as usize + p);
            }
        }
    }
    v
}

pub(crate) fn clear_detect(d: &mut Dcc) -> Result<(), i32> {
    d.w32(0x139e, 0);
    d.w16(0x139c, 0);
    if d.r8(0x35) == 0 && d.r8(0x32) == 0 && d.r8(0x33) == 0 && d.r8(0x34) == 0 {
        return Ok(());
    }
    Err(E)
}

fn chan_table(v48: u32) -> (u32, u32, u32, u32, bool, bool) {
    match v48 {
        8 => (10, 11, 0, 11, true, false),
        1 => (10, 11, 0, 11, false, false),
        9 => (11, 10, 0, 11, true, false),
        2 => (11, 10, 0, 11, false, false),
        10 => (10, 10, 2, 10, true, false),
        3 => (10, 10, 2, 10, false, false),
        11 => (10, 10, 2, 10, true, true),
        4 | 5 | 6 => (10, 10, 2, 10, false, true),
        7 => (9, 9, 5, 9, false, true),
        13 => (5, 5, 0, 6, false, false),
        14 => (5, 5, 1, 5, false, false),
        15 => (5, 5, 1, 5, false, true),
        _ => (0, 0, 0, 0, false, false),
    }
}

fn put(l: &mut u32, k: &mut u32, on: bool) {
    let m = 1u32 << (*k & 31);
    if on {
        *l |= m;
    } else {
        *l &= !m;
    }
    *k += 1;
}

pub(crate) fn channel(d: &mut Dcc) -> Result<(), i32> {
    let v48 = d.r32(0x48);
    if v48 == 0 || v48 == 0xc {
        return Ok(());
    }
    let (a29, a21, a25, p7f, b6f, b77) = chan_table(v48);
    let mut o = 0u32;
    while o < 0x100 {
        if seg(d, o >> 6)? {
            let bpe = d.r32(4);
            let wide = bpe == 2 || bpe == 4;
            let x0 = bix(o)?;
            let mut ebx = 0u32;
            if wide {
                ebx = d.b[2][x0] as u32 | (d.b[2][bix(o + 1)?] as u32) << 8;
                if bpe == 4 {
                    ebx |= (d.b[2][bix(o + 2)?] as u32 | (d.b[2][bix(o + 3)?] as u32) << 8) << 16;
                }
            }
            let mut w = 0u32;
            if !b77 {
                w = ebx & lmask(a25);
                ebx = ebx.wrapping_shr(a25);
            }
            let x = ebx & lmask(a21);
            ebx = ebx.wrapping_shr(a21);
            let z = ebx & lmask(p7f);
            ebx = ebx.wrapping_shr(p7f);
            let y = ebx & lmask(a29);
            ebx = ebx.wrapping_shr(a29);
            if b77 {
                w = 1u32.wrapping_shl(a25 + 1).wrapping_sub(1) & ebx;
            }
            let mut l0 = 0u32;
            let mut k = 0u32;
            let v;
            if p7f < 8 {
                let r11 = ((w & 1) << 7) | ((y & 1) << 1) | (z << 2) | (x & 1);
                for j in 1..=4u32 {
                    if a21 > j {
                        put(&mut l0, &mut k, bt(x, j));
                    }
                    if a29 > j {
                        put(&mut l0, &mut k, bt(y, j));
                    }
                }
                v = (r11 << 8) | l0;
            } else {
                let (l1, l3, l7, xb, zb, yb) = if b6f {
                    (x & 0xff, z & 0xff, y & 0xff, x >> 8, z >> 8, y >> 8)
                } else {
                    (x.wrapping_shr(a21 - 8) & 0xff, z.wrapping_shr(p7f - 8) & 0xff, y.wrapping_shr(a29 - 8) & 0xff, x, z, y)
                };
                let (es, r12, ed) = (p7f as i32 - 8, a29 as i32 - 8, a21 as i32 - 8);
                for j in 0..=2i32 {
                    let order = if v48 == 7 { [(es, zb), (r12, yb), (ed, xb)] } else { [(ed, xb), (es, zb), (r12, yb)] };
                    for (lim, src) in order {
                        if j < lim {
                            put(&mut l0, &mut k, bt(src, j as u32));
                        }
                    }
                }
                for n in 0..5u32 {
                    if a25 > n {
                        put(&mut l0, &mut k, bt(w, n));
                    }
                }
                let t = if p7f > 8 { (l7 << 16) | (l3 << 8) | l1 } else { l3 };
                v = (t << 8) | l0;
            }
            if wide {
                let vb = v.to_le_bytes();
                d.b[2][x0] = vb[0];
                d.b[2][bix(o + 1)?] = vb[1];
                if bpe == 4 {
                    d.b[2][bix(o + 2)?] = vb[2];
                    d.b[2][bix(o + 3)?] = vb[3];
                }
            }
        }
        let bpe = d.r32(4);
        if bpe == 0 {
            return Err(E);
        }
        o = o.wrapping_add(bpe);
    }
    Ok(())
}

pub(crate) fn color_mode(d: &Dcc) -> u32 {
    let mut r = d.r32(0x4c);
    if r == 0 {
        if d.r32(8).wrapping_sub(1) <= 1 {
            return 1;
        }
        let v = d.i32(0x48);
        if v > 0 && v <= 0xb {
            r = 3;
        } else if v > 0 && v <= 0xf {
            return 1;
        } else {
            r = if d.r8(0x10) != 0 { 2 } else { 3 };
        }
    }
    r
}

pub(crate) fn color_faults(d: &Dcc) -> bool {
    color_mode(d) != 1 && d.r32(4) >> 2 == 0 && (0..4).any(|k| (d.r32(0x5c) >> k) & 1 != 0)
}

pub(crate) fn color_in(d: &mut Dcc, t: &mut Tmp) -> Result<(), i32> {
    let cm = color_mode(d);
    color(d)?;
    if cm != 1 {
        if crate::dcc::xg_build() == 22000 {
            t.b[56..64].copy_from_slice(&8u64.to_le_bytes());
            t.b[54] = 0;
            t.b[55] = 0;
            for p in 0..64 {
                t.ever[p] = p >= 54 || (t.ever[p] && (16..48).contains(&p));
            }
        } else {
            t.ever = [false; 64];
        }
    }
    Ok(())
}

pub(crate) fn channel_in(d: &mut Dcc, t: &mut Tmp) -> Result<(), i32> {
    let v48 = d.r32(0x48);
    channel(d)?;
    if v48 != 0 && v48 != 0xc {
        let (written, zero): (&[core::ops::Range<usize>], &[usize]) = match crate::dcc::xg_build() {
            22000 => (&[0..64], &[39, 44, 45, 46, 47, 54, 55]),
            22621 => (&[0..36, 40..56], &[46, 47]),
            _ => (&[0..64], &[]),
        };
        for r in written {
            for p in r.clone() {
                t.ever[p] = false;
            }
        }
        for &p in zero {
            t.b[p] = 0;
            t.ever[p] = true;
        }
    }
    Ok(())
}

pub(crate) fn conv_residue(p: Option<usize>) -> ([u8; 64], [bool; 64]) {
    if crate::dcc::xg_build() == 19041 {
        return ([0; 64], [true; 64]);
    }
    let mut b = [0u8; 64];
    b[0] = 0x3d;
    b[8..16].fill(0xff);
    b[48] = 0x3d;
    b[56] = 0x32;
    let mut k = [true; 64];
    match p {
        Some(v) => b[16..24].copy_from_slice(&(v as u64).to_le_bytes()),
        None => k[16..22].fill(false),
    }
    (b, k)
}

pub(crate) fn run_model(d: &mut Dcc, fills: &[[u8; 64]; 3], conv: &[u8; 64]) -> Result<(), i32> {
    d.w64(0x13c0, 0);
    d.w32(0xf0, 0);
    let mut t = Tmp::with(Some(fills[0]));
    relay_in(d, 2, 0, 1, &mut t)?;
    clear_detect(d)?;
    channel_in(d, &mut t)?;
    signfix(d)?;
    t.fill = Some(fills[1]);
    relay_in(d, 2, 2, 2, &mut t)?;
    color_in(d, &mut t)?;
    t.fill = Some(fills[2]);
    relay_in(d, 2, 2, 3, &mut t)?;
    delta(d)?;
    vrs_with(d, Some(*conv))?;
    choose(d)?;
    far(d)?;
    subbed(d)?;
    plan(d)?;
    pack(d)?;
    Ok(())
}

pub(crate) fn color(d: &mut Dcc) -> Result<(), i32> {
    let r = color_mode(d);
    if r == 1 {
        return Ok(());
    }
    let q4 = d.r32(4) >> 2;
    for k in 0..8u32 {
        if !seg(d, k >> 1)? {
            continue;
        }
        if q4 == 0 {
            return Err(E);
        }
        let (base, eps, epr): (i32, i32, i32) = if r == 2 { (8, -8, 8) } else { (16, 8, -8) };
        let b = (k * 32) as i32 + base;
        for t in 0..8i32 {
            let p = (b + t) as usize;
            let ps = (b + t + eps) as usize;
            let pr = (b + t + epr) as usize;
            let rem = (t as u32) % q4;
            let sh = ((d.r32(0x54).wrapping_sub(rem << 3)) as i32).max(0) as u32;
            let a = d.b[2][p];
            let dd = shl8(shr8(a, sh & 0xff), sh & 0xff);
            let s = d.b[2][ps].wrapping_sub(dd);
            let q = d.b[2][pr].wrapping_sub(dd);
            d.b[2][ps] = a;
            d.b[2][p] = s;
            d.b[2][pr] = q;
        }
    }
    Ok(())
}

fn variant_flag(d: &Dcc) -> u8 {
    let al = if d.r32(0) <= 4 { d.r8(0x41) } else { 0 };
    if d.r32(0x14) == 1 { al } else { 0 }
}

pub(crate) fn delta_calc(d: &mut Dcc) -> Result<(), i32> {
    let (sb, s0) = (d.r32(4), d.r32(0));
    let mut bpe = sb;
    if sb == 16 {
        bpe = 8;
        d.w32(4, 8);
    }
    if s0 == 9 {
        d.w32(0, if bpe < 8 { 6 } else { 7 });
    }
    let r = delta_body(d, bpe);
    d.w32(4, sb);
    d.w32(0, s0);
    r
}

fn delta_body(d: &mut Dcc, bpe: u32) -> Result<(), i32> {
    let v = d.r32(0x48).wrapping_sub(1);
    let (q, f69) = if v <= 10 {
        (bpe >> 2, false)
    } else if v <= 14 {
        (bpe >> 1, false)
    } else {
        (bpe.checked_div(d.r32(8)).ok_or(E)?, d.r8(0x10) != 0)
    };
    let thr = if f69 { bpe.wrapping_sub(q) } else { q };
    for k in (0..4u32).rev() {
        let r8v = 0u32.wrapping_sub(64 * k);
        if !seg(d, k)? {
            continue;
        }
        let ko = k as usize;
        d.w8(0x1c8 + ko, k as u8);
        for i in (0..64u32).rev() {
            let a = k * 64 + i;
            let dir = d.r32(0x12c8);
            let mode = d.r32(0xf0);
            let r14 = prev(d, dir, a, mode)?;
            let ns = r14 >> 6;
            let sil = ns != k;
            if ns != k && ns != d.r8(0x1c8 + ko) as u32 {
                d.w8(0x1c8 + ko, ns as u8);
            }
            let c2c = d.r32(0x2c);
            let al = (c2c.wrapping_sub(1) & !2) == 0 || (c2c == 2 && r8v & 0x7f == 0);
            let mut r11 = ns == k || !al;
            d.w32(0x64, if c2c == 2 { if k < 2 { 3 } else { 0xc } } else { 0xf });
            if k >= ns {
                let mut w = k as i64;
                while w >= ns as i64 {
                    let wu = w as u32;
                    if !(bit(d, 0x5c, wu)? && bit(d, 0x60, wu)?) {
                        r11 = false;
                    }
                    w -= 1;
                }
            }
            let r9 = a != r14 && bit(d, 0x5c, ns)? && r11 && bit(d, 0x64, ns)?;
            let r10 = r9 && sil;
            let ai = a as usize;
            if a & 3 == 0 {
                let e = (a >> 2) & 7;
                let w = (a >> 5) as usize;
                let v = d.b[2][ai];
                wb(d, 0x168 + w * 8 + e as usize, v)?;
                sb(d, 0x138 + 4 * w, e, r10)?;
            }
            let mut v = d.b[2][ai];
            if r9 {
                let mut n = d.b[2][bix(r14)?];
                if d.r32(0x50) != 0 || d.r32(0x54) != 0 {
                    let c = get(d, a, d.r32(0xf0))?;
                    let mut sel = c[3] >= thr;
                    if !f69 {
                        sel = !sel;
                    }
                    let ecx = if sel { d.r32(0x50) } else { d.r32(0x54) };
                    let rem = c[3].checked_rem(q).ok_or(E)?;
                    let sh = (ecx.wrapping_sub(rem << 3) as i32).max(0) as u32;
                    v = shr8(v, sh);
                    n = shr8(n, sh);
                }
                v = v.wrapping_sub(n);
            }
            d.b[2][ai] = v;
        }
    }
    Ok(())
}

pub(crate) fn delta(d: &mut Dcc) -> Result<(), i32> {
    let orig = d.b[2];
    d.m[0x399..0x499].copy_from_slice(&orig);
    let fl = variant_flag(d);
    d.w8(0x394, fl);
    let top = if fl != 0 { 2usize } else { 0 };
    let en = [1u8, d.r8(0x44), d.r8(0x45)];
    for dir in (0..=top).rev() {
        if en[dir] == 0 {
            continue;
        }
        d.b[2].copy_from_slice(&d.m[0x399..0x499].to_vec());
        d.w32(0x12c8, dir as u32);
        delta_calc(d)?;
        for k in 0..4u32 {
            if !seg(d, k)? {
                continue;
            }
            for x in (k * 64) as usize..(k * 64 + 64) as usize {
                d.b[2][x] = eflip(d.b[2][x]);
            }
        }
        let o = 0x499 + dir * 0x100;
        let b = d.b[2];
        d.m[o..o + 256].copy_from_slice(&b);
    }
    Ok(())
}

fn seg_widths(d: &mut Dcc, k: u32) -> Result<(), i32> {
    for q in 2 * k..2 * k + 2 {
        if !seg(d, q >> 1)? {
            continue;
        }
        let qo = q as usize;
        for e in 0..8u32 {
            let eo = e as usize;
            wb(d, 0x2cc + 8 * qo + eo, 0)?;
            let addr = (e + q * 8) * 4;
            let es = ((addr >> 3) & 7) as usize;
            let w0 = bitlen(d.b[2][bix(addr)?]);
            wb(d, 0x30c + 8 * qo + eo, w0)?;
            let mut mx3 = 0u8;
            let mut mx = w0;
            for t in 1..4 {
                let w = bitlen(d.b[2][bix(addr + t)?]);
                mx3 = mx3.max(w);
                wb(d, 0x2cc + 8 * qo + eo, mx3)?;
                mx = mx.max(w);
                wb(d, 0x30c + 8 * qo + eo, mx)?;
            }
            let o = 8 * k as usize + es;
            if e & 1 == 0 {
                wb(d, 0x34c + o, mx3)?;
                wb(d, 0x36c + o, mx)?;
            } else {
                let a = rb(d, 0x34c + o)?.max(mx);
                wb(d, 0x34c + o, a)?;
                let b = rb(d, 0x36c + o)?.max(mx);
                wb(d, 0x36c + o, b)?;
            }
        }
    }
    Ok(())
}

fn tid_in(d: &mut Dcc, t: &mut Tmp) -> Result<(), i32> {
    let gm = [3u32; 4];
    let al = d.r32(0x12c0);
    let mut sm = [4u32; 4];
    for k in 0..4 {
        sm[k] = match (al as u8 as u32 >> (2 * k)) & 3 {
            1 => 8,
            2 => 10,
            3 => 9,
            _ => 4,
        };
    }
    let mut maps = [None; 4];
    for k in 0..4 {
        if gm[k] != sm[k] {
            maps[k] = Some((gm[k], sm[k]));
        }
    }
    lay_in(d, 2, 2, &maps, t)?;
    for q in 0..8u32 {
        if !seg(d, q >> 1)? {
            continue;
        }
        let (g, s) = (gm[(q >> 1) as usize], sm[(q >> 1) as usize]);
        for i in 0..8u32 {
            let c = get(d, 32 * q + 4 * i, g)?;
            let a = set(d, &c, s)?;
            if a & 7 != 0 {
                continue;
            }
            let r10 = (a >> 3) & 7;
            let r9 = (a >> 6) as usize;
            let b = bit(d, 0x138 + 4 * q as usize, i)?;
            sb(d, 0x158 + 4 * r9, r10, b)?;
            if d.r32(0x12c0) == 0 {
                let v = bit(d, 0x158 + 4 * r9, r10)?;
                wb(d, 0x115c + r9 * 8 + r10 as usize, v as u8)?;
            }
            let v = rb(d, 0x168 + 8 * q as usize + i as usize)?;
            wb(d, 0x1a8 + r9 * 8 + r10 as usize, v)?;
        }
    }
    for k in 0..4 {
        seg_widths(d, k)?;
    }
    Ok(())
}

pub(crate) fn vrs(d: &mut Dcc) -> Result<(), i32> {
    vrs_with(d, None).map(|_| ())
}

pub(crate) fn vrs_with(d: &mut Dcc, fill: Option<[u8; 64]>) -> Result<bool, i32> {
    let sb = d.r32(4);
    if sb == 16 {
        d.w32(4, 8);
    }
    let r = vrs_body(d, fill);
    d.w32(4, sb);
    r
}

fn vrs_body(d: &mut Dcc, fill: Option<[u8; 64]>) -> Result<bool, i32> {
    let mut hit = false;
    let fl = variant_flag(d);
    d.w8(0x394, fl);
    let top = if fl != 0 { 3usize } else { 0 };
    let en = [1u8, d.r8(0x43), d.r8(0x44), d.r8(0x45)];
    for v in (0..=top).rev() {
        if en[v] == 0 {
            continue;
        }
        let src = 0x499 + if v == 3 { 0x200 } else { 0 };
        let b: [u8; 256] = d.m[src..src + 256].try_into().unwrap();
        d.b[2] = b;
        d.w32(0xf0, 3);
        let (val, sm) = match v {
            1 => (0x55, 8),
            2 => (0xaa, 10),
            3 => (0xff, 9),
            _ => (0, 4),
        };
        d.w32(0x12c0, val);
        let mut t = Tmp::with(fill);
        tid_in(d, &mut t)?;
        hit |= t.hit;
        let o = 0xb99 + v * 0x100;
        let b = d.b[2];
        d.m[o..o + 256].copy_from_slice(&b);
        let a: Vec<u8> = d.m[0x34c..0x38c].to_vec();
        d.m[0xf99 + 32 * v..0xf99 + 32 * v + 32].copy_from_slice(&a[..32]);
        d.m[0x1019 + 32 * v..0x1019 + 32 * v + 32].copy_from_slice(&a[32..]);
        for e in 0..32u32 {
            let c = get(d, (8 * e) & 0xff, 4)?;
            let a = set(d, &c, sm)?;
            d.m[0x10dc + 32 * v + e as usize] = a as u8;
        }
    }
    Ok(hit)
}

pub(crate) fn choose(d: &mut Dcc) -> Result<(), i32> {
    let fl = variant_flag(d);
    d.w8(0x394, fl);
    let top = if fl != 0 { 3usize } else { 0 };
    let r13 = d.r8(0x42) != 0 || d.r8(0x41) == 0;
    let en = [r13, d.r8(0x43) != 0, d.r8(0x44) != 0, d.r8(0x45) != 0];
    for v in (0..=top).rev() {
        for j in 0..4usize {
            let cost = if en[v] && (d.r8(0x394) != 0 || v == 0) {
                let mut s = 0u32;
                for t in 0..8 {
                    s += d.m[0xf99 + 32 * v + 8 * j + t] as u32 + (d.m[0x1019 + 32 * v + 8 * j + t] != 0) as u32;
                }
                s
            } else {
                0x100
            };
            d.w32(0x109c + 16 * v + 4 * j, cost);
        }
    }
    d.w32(0x12c0, 0);
    for j in 0..4usize {
        let c0 = d.r32(0x109c + 4 * j);
        let mut best = c0;
        let mut sel = 0usize;
        if d.r8(0x394) != 0 {
            for v in 1..=3usize {
                let c = d.r32(0x109c + 16 * v + 4 * j);
                if en[v] && c <= best {
                    sel = v;
                    best = c;
                }
            }
        }
        if r13 && c0 <= best {
            sel = 0;
        }
        let e = insert(d.r32(0x12c0), sel as u32 & 3, 2 * j as i32 + 1, 2 * j as i32);
        d.w32(0x12c0, e & 0xff);
        let s: Vec<u8> = d.m[0xb99 + sel * 0x100 + 64 * j..0xb99 + sel * 0x100 + 64 * j + 64].to_vec();
        d.b[2][64 * j..64 * j + 64].copy_from_slice(&s);
        let a: Vec<u8> = d.m[0xf99 + 32 * sel + 8 * j..0xf99 + 32 * sel + 8 * j + 8].to_vec();
        d.m[0x34c + 8 * j..0x34c + 8 * j + 8].copy_from_slice(&a);
        let b: Vec<u8> = d.m[0x1019 + 32 * sel + 8 * j..0x1019 + 32 * sel + 8 * j + 8].to_vec();
        d.m[0x36c + 8 * j..0x36c + 8 * j + 8].copy_from_slice(&b);
    }
    Ok(())
}

fn vmode(d: &Dcc, k: u32) -> u32 {
    (d.r8(0x12c0) as u32 >> (2 * k)) & 3
}

pub(crate) fn far(d: &mut Dcc) -> Result<(), i32> {
    for j in 0..4u32 {
        let jo = j as usize;
        let mode = vmode(d, j);
        let o = 0x158 + 4 * jo;
        if mode != 0 {
            d.w32(o, d.r32(o) & 0xffff_ff00);
            for i in 0..8usize {
                if d.m[0x115c + 8 * jo + i] != 0 {
                    let a = d.m[0x10dc + 32 * mode as usize + 8 * jo + i] as usize;
                    let t = 0x158 + 4 * (a >> 6);
                    d.w32(t, d.r32(t) | 1 << ((a >> 3) & 7));
                }
            }
        } else {
            for i in 0..8u32 {
                let on = d.m[0x115c + 8 * jo + i as usize] != 0;
                sb(d, o, i, on)?;
            }
        }
    }
    Ok(())
}

pub(crate) fn far_base(d: &mut Dcc, k: u32) -> Result<(), i32> {
    let mode = vmode(d, k);
    if mode == 0 {
        return Ok(());
    }
    let ko = k as usize;
    for i in 0..8usize {
        if d.m[0x115c + 8 * ko + i] != 0 {
            let a = d.m[0x10dc + 8 * (ko + 4 * mode as usize) + i] as usize;
            d.b[2][a] = eflip(d.m[0x1a8 + 8 * ko + i]);
        }
    }
    seg_widths(d, k)
}

fn stats(d: &mut Dcc, k: u32, i: u32) -> Result<(), i32> {
    let ko = k as usize;
    let io = i as usize;
    let w7 = d.m[0x34c + 8 * ko + io];
    let w8 = d.m[0x36c + 8 * ko + io];
    let o = 4 * ko;
    sb(d, 0x12cc + o, i, w7 == 0)?;
    sb(d, 0x12dc + o, i, w8 == 0)?;
    let c = w8 > w7;
    sb(d, 0x12ec + o, i, c)?;
    let r14 = bit(d, 0x158 + o, i)?;
    let dd = !r14 && !c && i != 0;
    sb(d, 0x12fc + o, i, dd)?;
    let e = dd && w7 == d.m[0x34c + 8 * ko + io - 1];
    sb(d, 0x130c + o, i, e)?;
    let f = (!bit(d, 0x12dc + o, i)? || r14) && !e;
    sb(d, 0x131c + o, i, f)
}

fn stats2(d: &mut Dcc, k: u32) -> Result<(), i32> {
    let ko = k as usize;
    let o = 4 * ko;
    for i in 0..8u32 {
        let io = i as usize;
        let g = bit(d, 0x158 + o, i)? && !bit(d, 0x12ec + o, i)?;
        sb(d, 0x1334 + o, i, g)?;
        let h = if g {
            if k == 0 {
                return Err(E);
            }
            d.m[0x34c + 8 * ko + io] == d.m[0x34c + 8 * (ko - 1) + io]
        } else {
            false
        };
        sb(d, 0x1344 + o, i, h)?;
        let ii = !bit(d, 0x12dc + o, i)? && !bit(d, 0x130c + o, i)? && !h;
        sb(d, 0x1354 + o, i, ii)?;
    }
    Ok(())
}

pub(crate) fn plan(d: &mut Dcc) -> Result<(), i32> {
    for k in 0..4u32 {
        if !seg(d, k)? {
            continue;
        }
        let ko = k as usize;
        d.m[0x132c + ko] = 0;
        for i in 0..8u32 {
            stats(d, k, i)?;
            d.m[0x132c + ko] = d.m[0x132c + ko].wrapping_add(d.m[0x34c + 8 * ko + i as usize]);
        }
        let s = d.m[0x132c + ko].wrapping_add(2);
        d.m[0x132c + ko] = s;
        d.m[0x1330 + ko] = s.wrapping_add(pop(d, 0x131c + 4 * ko)? as u8);
        d.m[0x1364 + ko] = s;
        stats2(d, k)?;
        d.m[0x1364 + ko] = d.m[0x1364 + ko].wrapping_add(pop(d, 0x1354 + 4 * ko)? as u8);
    }
    Ok(())
}

pub(crate) fn restore(d: &mut Dcc, k: u32) {
    let ko = k as usize;
    let s: Vec<u8> = d.m[0x1180 + 64 * ko..0x1180 + 64 * ko + 64].to_vec();
    d.b[2][64 * ko..64 * ko + 64].copy_from_slice(&s);
    let a: Vec<u8> = d.m[0x1280 + 8 * ko..0x1280 + 8 * ko + 8].to_vec();
    d.m[0x34c + 8 * ko..0x34c + 8 * ko + 8].copy_from_slice(&a);
    let b: Vec<u8> = d.m[0x12a0 + 8 * ko..0x12a0 + 8 * ko + 8].to_vec();
    d.m[0x36c + 8 * ko..0x36c + 8 * ko + 8].copy_from_slice(&b);
}

pub(crate) fn replan(d: &mut Dcc, k: u32) -> Result<(), i32> {
    let ko = k as usize;
    d.m[0x132c + ko] = 2;
    for i in 0..8u32 {
        stats(d, k, i)?;
        d.m[0x132c + ko] = d.m[0x132c + ko].wrapping_add(d.m[0x34c + 8 * ko + i as usize]);
        d.m[0x1330 + ko] = d.m[0x132c + ko].wrapping_add(pop(d, 0x131c + 4 * ko)? as u8);
    }
    let hi = if k >= 3 { k } else { k + 1 };
    for s in k..=hi {
        let so = s as usize;
        d.m[0x1364 + so] = d.m[0x132c + so];
        stats2(d, s)?;
        d.m[0x1364 + so] = d.m[0x1364 + so].wrapping_add(pop(d, 0x1354 + 4 * so)? as u8);
    }
    Ok(())
}

fn obit(d: &mut Dcc, b: u32, on: bool) -> Result<(), i32> {
    sb(d, 0x58, b, on)
}

pub(crate) fn meta_gen(d: &mut Dcc, a: u32, b: u32, c: u32, dd: u32, f: [bool; 7]) -> Result<(), i32> {
    let [f6, f7, f8, f9, f10, f11, f12] = f;
    if f8 {
        if f10 {
            if f12 {
                d.w32(0x58, 0);
                return Ok(());
            }
            if f11 {
                d.w32(0x58, 0x40);
                return Ok(());
            }
        }
        let v = if f9 { if !f12 && f11 { 0xc0 } else { 0x80 } } else if f11 { 0x40 } else { 0 };
        d.w32(0x58, v);
        return Ok(());
    }
    if f7 {
        d.w32(0x58, 0x20);
        return Ok(());
    }
    if f6 {
        d.w32(0x58, 0x10);
        return Ok(());
    }
    let ebp = b.wrapping_add(0x1e) >> 5;
    let esi = a >> 6;
    let r13 = b.wrapping_add(0x3f) >> 6;
    let r14 = (ebp << 5).wrapping_add(a).wrapping_sub(1) >> 6;
    let q = c.wrapping_add(dd).wrapping_sub(1).checked_div(dd).ok_or(E)?;
    let r15 = q.wrapping_mul(dd >> 5);
    let cnt = r14.wrapping_sub(esi).wrapping_add(1);
    let mk = (if cnt == 32 { 0xffff_ffff } else { 1u32.wrapping_shl(cnt).wrapping_sub(1) }).wrapping_shl(esi) & 0xf;
    let segs = seg_info_checked(d.r32(0x58)).ok_or(E)?;
    let mut edx = 0u32;
    for k in 0..4 {
        if (mk >> k) & 1 != 0 {
            edx |= segs[k];
        }
    }
    let mut m = d.r32(0x58);
    for j in 0..4 {
        if (edx >> j) & 1 != 0 {
            m |= 3 << (2 * j);
        }
    }
    d.w32(0x58, m);
    if r15 < ebp {
        let mut r9 = esi.wrapping_mul(2);
        let r11 = r14.wrapping_mul(2).wrapping_add(1);
        if r11.wrapping_sub(r9) > 64 && r9 <= r11 {
            return Err(E);
        }
        let mut r10 = r9;
        while r10 <= r11 {
            obit(d, r10, false)?;
            r10 += 1;
        }
        let n = r13.wrapping_sub(1);
        if n > 64 {
            return Err(E);
        }
        for _ in 0..n {
            obit(d, r9, false)?;
            r9 += 1;
        }
        obit(d, r9, true)?;
        r9 += 1;
        if r13 == 4 {
            obit(d, r9, false)?;
            r9 += 1;
        }
        let v = (r15.wrapping_sub(1).wrapping_shl(r9)) & 0xff;
        d.w32(0x58, d.r32(0x58) | v);
    }
    Ok(())
}

fn b1w(d: &mut Dcc, cur: &mut u32, v: u8) -> Result<(), i32> {
    d.b[1][bix(*cur)?] = v;
    *cur += 1;
    Ok(())
}

pub(crate) fn emit(d: &mut Dcc, s: u32, e: u32, bits: u32, raw: bool, a6: bool) -> Result<(), i32> {
    if s > 3 || (s <= e && e > 3) {
        return Err(E);
    }
    for c in s..=e {
        d.m[0x117c + c as usize] = 0;
    }
    if raw {
        let h: [u8; 32] = d.b[0][..32].try_into().unwrap();
        d.b[1][..32].copy_from_slice(&h);
        d.m[0x1398] = 0x20;
        return Ok(());
    }
    let unit = d.r32(0x24);
    let r8 = unit.wrapping_add(bits).wrapping_sub(1).checked_div(unit).ok_or(E)?;
    let n = e.wrapping_sub(s);
    let r10 = n.wrapping_add(1).wrapping_shl(6);
    let lim = if d.r8(0x31) != 0 { d.r32(0x28) } else { r10 };
    let ecx = r8.wrapping_mul(unit);
    let al = unit == 0x20 && s & 1 == 0 && (n.wrapping_sub(1) & !2) == 0 && r8 == (r10 >> 5).wrapping_sub(1);
    if ecx >= lim || (al && d.r8(0x40) != 0) {
        let ebx = s * 64;
        for c in s..=e {
            let o = (c * 64) as usize;
            let h: [u8; 64] = d.b[0][o..o + 64].try_into().unwrap();
            d.b[1][o..o + 64].copy_from_slice(&h);
            meta_gen(d, ebx, r10, r10, unit, [false; 7])?;
            d.m[0x1398 + c as usize] = 0x40;
        }
        return Ok(());
    }
    let so = s as usize;
    let mut cur = s * 64;
    let mut sil = false;
    if !a6 {
        let v = d.r32(0x12cc + 4 * so);
        sil = v & 1 != 0;
        d.w32(0x1368 + 4 * so, d.r32(0x1368 + 4 * so) & !1);
        d.w32(0x12cc + 4 * so, v & !1);
    }
    d.w32(0x390, d.r32(0x390).wrapping_add(bits));
    for c in s..=e {
        let co = c as usize;
        d.b[1][64 * co..64 * co + 64].fill(0xcc);
        let h = d.m[0x1368 + 4 * co];
        b1w(d, &mut cur, h)?;
        let h = d.m[0x12cc + 4 * co];
        b1w(d, &mut cur, h)?;
        d.m[0x117c + co] = 1;
    }
    if !a6 {
        let mut mk = 0u32;
        for c in 0..4u32 {
            if c <= e && c >= s {
                mk |= 3 << (2 * c);
            }
        }
        let v = d.r32(0x12c0) & mk;
        d.w32(0x12c4, v);
        b1w(d, &mut cur, v as u8)?;
        d.w32(0x1368 + 4 * so, d.r32(0x1368 + 4 * so) | 1);
        sb(d, 0x12cc + 4 * so, 0, sil)?;
    }
    for c in s..=e {
        let co = c as usize;
        let mode = vmode(d, c);
        for i in 0..8u32 {
            let io = i as usize;
            if !bit(d, 0x1368 + 4 * co, i)? {
                continue;
            }
            let mut r11 = d.m[0x1a8 + 8 * co + io];
            if mode != 0 {
                r11 = d.b[2][(io + 8 * co) * 8];
            }
            if !a6 && c == s && i == 0 && bit(d, 0x12cc + 4 * co, 0)? {
                b1w(d, &mut cur, 0)?;
            }
            if bit(d, 0x12cc + 4 * co, i)? {
                b1w(d, &mut cur, r11)?;
            } else {
                let w = d.m[0x34c + 8 * co + io] as u32;
                if w == 0 || w > 8 {
                    return Err(E);
                }
                let t = 1u32 << (w - 1);
                let v = if w <= 7 { (r11 as u32 >> w) & lmask(8 - w) } else { 0 };
                b1w(d, &mut cur, ((v << w) as u8) | t as u8)?;
            }
        }
    }
    for c in s..=e {
        let co = c as usize;
        let mode = vmode(d, c);
        for i in 0..8u32 {
            let io = i as usize;
            if bit(d, 0x12cc + 4 * co, i)? {
                continue;
            }
            let w = d.m[0x34c + 8 * co + io] as u32;
            let hb = bit(d, 0x1368 + 4 * co, i)?;
            for n in 0..w {
                let mut acc = 0u8;
                for t in 0..8usize {
                    let mut ev = d.m[0x1a8 + 8 * co + io];
                    if mode != 0 && hb {
                        ev = d.b[2][64 * co + 8 * io];
                    }
                    if !(t == 0 && hb) {
                        ev = d.b[2][64 * co + 8 * io + t];
                    }
                    if (ev >> (n & 31)) & 1 != 0 {
                        acc |= 1 << t;
                    }
                }
                b1w(d, &mut cur, acc)?;
            }
        }
    }
    meta_gen(d, s * 64, r10, bits, unit, [false; 7])?;
    let mut left = bits;
    for c in s..=e {
        let take = left.min(0x40);
        d.m[0x1398 + c as usize] = take as u8;
        left -= take;
    }
    Ok(())
}

pub(crate) fn pack(d: &mut Dcc) -> Result<(), i32> {
    let a9c = d.r32(0x5c) & 0xf == 0xf;
    let a98 = d.r32(0x60) & 0xf == 0xf;
    let unit = d.r32(0x24);
    let mut b100 = true;
    let mut b108 = true;
    let mut newg = true;
    let (mut g64, mut g60, mut g6c, mut g68, mut g70) = (0u32, 0u32, 0u32, 0u32, 0u32);
    let mut r13 = 0u32;
    for esi in 0..4u32 {
        let so = esi as usize;
        let byte = d.r8(0x12c0) as u32;
        let mode = (byte >> (2 * esi)) & 3;
        let ebp = if esi == 3 { byte >> 6 } else { (byte >> (2 * esi + 2)) & 3 };
        if !bit(d, 0x5c, esi)? {
            emit(d, esi, esi, 0x100, false, false)?;
            newg = true;
            continue;
        }
        let last = esi == 3 || !bit(d, 0x5c, esi + 1)? || !bit(d, 0x60, esi)?;
        if newg {
            g64 = 0;
            r13 = esi;
            g60 = 0;
            g6c = 0;
            g68 = 0;
            g70 = esi;
            b100 = true;
            b108 = true;
        }
        if mode != 0 && ((d.r8(0x1c8 + so) as u32) < r13 || r13 == esi) {
            restore(d, esi);
            replan(d, esi)?;
        }
        if ebp != 0 && esi < 3 {
            let n = esi + 1;
            if (d.r8(0x1c8 + n as usize) as u32) < r13 || r13 == n {
                restore(d, n);
                replan(d, n)?;
            }
        }
        let ob20 = d.r32(0x20);
        let p94 = (esi * 64).wrapping_sub(64).checked_div(ob20).ok_or(E)?;
        let p90 = (esi * 64) / ob20;
        let (cl, use_i) = if p94 == p90 && r13 <= d.r8(0x1c8 + so) as u32 { (d.r8(0x1364 + so), true) } else { (d.r8(0x1330 + so), false) };
        let mut r11 = g64.wrapping_add(cl as u32);
        let pc = if mode != 0 && use_i { pop(d, 0x1354 + 4 * so)? } else { pop(d, 0x131c + 4 * so)? };
        let mut r14 = if newg { cl as u32 } else { g60.wrapping_add(pc & 0xff).wrapping_add(2) };
        g60 = r14;
        let dil0;
        let ro = r13 as usize;
        if mode != 0 {
            if b100 {
                let x = (!rd(d, 0x131c + 4 * ro)? & 1) + (rd(d, 0x12cc + 4 * ro)? & 1);
                r11 = r11.wrapping_add(x + 1);
                r14 = r14.wrapping_add(x + 1);
                g60 = r14;
            }
            dil0 = false;
            b108 = false;
        } else {
            dil0 = b108;
        }
        let c2c = d.r32(0x2c);
        let mut r8 = d.r32(0x28);
        if c2c.wrapping_sub(2) <= 1 {
            r8 = r8.wrapping_sub((r13 * 64).checked_rem(r8).ok_or(E)?);
        }
        let nx = if last { 0 } else { d.r8(0x1364 + so + 1) as u32 };
        let mut edx = nx.wrapping_add(r11);
        if dil0 && ebp != 0 {
            edx = edx.wrapping_add((rd(d, 0x12cc + 4 * ro)? & 1) + (!rd(d, 0x131c + 4 * ro)? & 1) + 1);
        }
        let al = !(c2c.wrapping_sub(2) <= 1 && esi & 1 == 0 && !last) || edx <= r8;
        let fits = r11 <= r8 && al;
        let r10 = g70;
        let n1 = so + 1;
        let pc2 = if ebp != 0 && rb(d, 0x1c8 + n1)? as u32 >= r10 { pop(d, 0x1354 + 4 * n1)? } else { pop(d, 0x131c + 4 * n1)? };
        let mut edx = (pc2 & 0xff).wrapping_add(r14).wrapping_add(2);
        if dil0 && ebp != 0 {
            let r = r10 as usize;
            edx = edx.wrapping_add((rd(d, 0x12cc + 4 * r)? & 1) + (!rd(d, 0x131c + 4 * r)? & 1) + 1);
        }
        let r8c = g6c;
        let cl2 = !(r8c != 0 && esi & 1 == 0 && edx > 0x40);
        let ebp2 = r8c.wrapping_add(1).wrapping_shl(6);
        let edi = g68.wrapping_add(0x40u32.checked_div(unit).ok_or(E)?);
        g68 = r11.wrapping_add(unit).wrapping_sub(1) / unit;
        let over = r14 > 0x40;
        let o30 = d.r8(0x30) != 0;
        let (f14, r10b) = if !over {
            (true, false)
        } else if o30 && r8c == 0 {
            (false, true)
        } else if o30 {
            (false, false)
        } else {
            (true, false)
        };
        let r11b = !(!cl2 && o30 && c2c >= 2);
        let r8b = last && esi & 1 == 0 && r8c != 0;
        let alq = esi & 1 == 0 && r8c != 0 && !bit(d, 0x60, esi + 1)? && bit(d, 0x5c, esi + 1)?;
        let cond_a = if last { edi <= g68 } else { edi < g68 };
        let q = r10b || ((r8b || alq) && c2c.wrapping_sub(2) <= 1);
        let p = if cond_a { c2c < 2 || q } else if r11 < ebp2 { q } else { c2c < 2 || q };
        let r15b = p && d.r8(0x31) == 0;
        let same = p94 == p90;
        let merge = same && ((fits && !r15b && f14 && r11b) || d.r8(0x31) != 0) && bit(d, 0x60, esi)?;
        r13 = esi;
        let src = if use_i && merge { 0x1354 + 4 * so } else { 0x131c + 4 * so };
        let hv = rd(d, src)?;
        d.w32(0x1368 + 4 * so, hv);
        if mode == 0 {
            for i in 0..8usize {
                if (d.r32(0x1368 + 4 * so) >> i) & 1 != 0 {
                    d.b[2][so * 64 + 8 * i] = d.m[0x1a8 + 8 * so + i];
                }
            }
        }
        let (f9c, f9d, f9e, f9f, fa0, fa1) = (d.m[0x139c] != 0, d.m[0x139d] != 0, d.m[0x139e] != 0, d.m[0x139f] != 0, d.m[0x13a0] != 0, d.m[0x13a1] != 0);
        let goto134 = if f9d || f9c { !f9e && !f9f } else { true };
        let a2 = if !goto134 {
            true
        } else {
            let to176 = match d.r32(8) {
                1 => !(f9d || f9c || f9e || f9f),
                2 if d.r8(0x12) == 0 => true,
                2 | 3 => !(f9d || f9c),
                _ => true,
            };
            !to176 || (d.r32(0x48) == 7 && (f9d || f9c))
        };
        let (bpl, dil1) = if a2 { (true, true) } else { (false, fa0 || fa1) };
        let dil = dil1 && a98 && a9c;
        let mut r14b;
        if merge || dil {
            g6c += 1;
            r14b = b108;
            r13 = g70;
            b100 = b108;
            g64 = r11;
        } else {
            if g6c != 0 {
                emit(d, g70, esi.wrapping_sub(1), g64, dil, b100)?;
            }
            g6c = 1;
            if !bit(d, 0x60, r13)? {
                r11 = 0x40;
                r14b = b100;
                g68 = unit.wrapping_add(0x3f) / unit;
            } else {
                if mode != 0 {
                    restore(d, esi);
                    replan(d, esi)?;
                    d.w32(0x1368 + 4 * so, d.r32(0x131c + 4 * so));
                    let x = (d.r32(0x12cc + 4 * so) & 1) + (!d.r32(0x1368 + 4 * so) & 1);
                    r14b = false;
                    r11 = x + d.m[0x1330 + so] as u32 + 1;
                } else {
                    d.w32(0x1368 + 4 * so, d.r32(0x131c + 4 * so));
                    r14b = true;
                    r11 = d.m[0x1330 + so] as u32;
                }
                b100 = r14b;
                b108 = r14b;
                g68 = unit.wrapping_add(r11).wrapping_sub(1) / unit;
            }
            g64 = r11;
            g60 = r11;
            r13 = esi;
            g70 = esi;
        }
        if !last {
            if !r15b {
                newg = false;
                continue;
            }
            r14b = b100;
        }
        if dil && last {
            meta_gen(d, 0, 0, 0, 0, [fa0, fa1, bpl, f9c, f9d, f9e, f9f])?;
            let mt = d.r32(0x58);
            if (mt >> 4) & 1 != 0 && mt & 0xf == 0 {
                emit(d, r13, esi, g64, true, r14b)?;
            }
        } else {
            emit(d, r13, esi, r11, false, r14b)?;
        }
        newg = true;
    }
    Ok(())
}

pub fn run(d: &mut Dcc) -> Result<(), i32> {
    run_with(d, None, None).map(|_| ())
}

pub fn run_with(d: &mut Dcc, fill: Option<[u8; 64]>, conv: Option<[u8; 64]>) -> Result<bool, i32> {
    d.w64(0x13c0, 0);
    d.w32(0xf0, 0);
    let mut t = Tmp::with(fill);
    relay_in(d, 2, 0, 1, &mut t)?;
    clear_detect(d)?;
    channel(d)?;
    signfix(d)?;
    relay_in(d, 2, 2, 2, &mut t)?;
    color_in(d, &mut t)?;
    relay_in(d, 2, 2, 3, &mut t)?;
    delta(d)?;
    let h = vrs_with(d, conv)?;
    choose(d)?;
    far(d)?;
    subbed(d)?;
    plan(d)?;
    pack(d)?;
    Ok(t.hit || h)
}

#[cfg(test)]
thread_local! {
    pub(crate) static RAND: std::cell::Cell<(u64, u32)> = const { std::cell::Cell::new((0, 0)) };
    pub(crate) static POS: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
fn scramble(site: usize, t: &mut Tmp, hs: &[usize]) {
    let (mut x, mask) = RAND.with(|r| r.get());
    if mask >> site & 1 == 0 {
        return;
    }
    let only = POS.with(|q| q.get());
    for &p in hs {
        if site == 3 && !(16..24).contains(&p) && only.is_none() {
            continue;
        }
        if only.is_some_and(|q| q != p) {
            continue;
        }
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        t.b[p] = x as u8;
    }
    RAND.with(|r| r.set((x, mask)));
}

#[cfg(test)]
pub(crate) fn lay_holes(d: &Dcc, maps: &[Option<(u32, u32)>; 4]) -> Vec<usize> {
    let mut ever = [false; 64];
    let mut first = true;
    let mut v = Vec::new();
    for k in 0..4u32 {
        if !seg(d, k).unwrap_or(false) {
            continue;
        }
        let Some((from, to)) = maps[k as usize] else { continue };
        for a in k * 64..k * 64 + 64 {
            if let Ok(c) = get(d, a, from) {
                if let Ok(b) = set(d, &c, to) {
                    ever[(b & 63) as usize] = true;
                }
            }
        }
        if first {
            v = (0..64).filter(|&p| !ever[p]).collect();
            first = false;
        }
    }
    v
}

#[cfg(test)]
pub(crate) fn run_feed(d: &mut Dcc, rel: &[[u8; 64]; 3], conv: &mut dyn FnMut() -> [u8; 64]) -> Result<(), i32> {
    run_feed_log(d, rel, conv, &mut Vec::new())
}

#[cfg(test)]
pub(crate) fn run_feed_log(d: &mut Dcc, rel: &[[u8; 64]; 3], conv: &mut dyn FnMut() -> [u8; 64], log: &mut Vec<(usize, Vec<usize>)>) -> Result<(), i32> {
    d.w64(0x13c0, 0);
    d.w32(0xf0, 0);
    let mut t = Tmp::with(None);
    let mut put = |t: &mut Tmp, b: &[u8; 64]| {
        t.b = *b;
        t.ever = [true; 64];
    };
    let h = |d: &Dcc, m: u32| lay_holes(d, &[Some((d.r32(0xf0), m)); 4]);
    put(&mut t, &rel[0]);
    log.push((0, h(d, 1)));
    scramble(0, &mut t, &log.last().unwrap().1.clone());
    relay_in(d, 2, 0, 1, &mut t)?;
    clear_detect(d)?;
    channel(d)?;
    signfix(d)?;
    put(&mut t, &rel[1]);
    log.push((1, h(d, 2)));
    scramble(1, &mut t, &log.last().unwrap().1.clone());
    relay_in(d, 2, 2, 2, &mut t)?;
    color(d)?;
    put(&mut t, &rel[2]);
    log.push((2, h(d, 3)));
    scramble(2, &mut t, &log.last().unwrap().1.clone());
    relay_in(d, 2, 2, 3, &mut t)?;
    delta(d)?;
    let sb = d.r32(4);
    if sb == 16 {
        d.w32(4, 8);
    }
    let r = vrs_feed(d, conv, log);
    d.w32(4, sb);
    r?;
    choose(d)?;
    far(d)?;
    subbed(d)?;
    plan(d)?;
    pack(d)?;
    Ok(())
}

#[cfg(test)]
fn vrs_feed(d: &mut Dcc, conv: &mut dyn FnMut() -> [u8; 64], log: &mut Vec<(usize, Vec<usize>)>) -> Result<(), i32> {
    let fl = variant_flag(d);
    d.w8(0x394, fl);
    let top = if fl != 0 { 3usize } else { 0 };
    let en = [1u8, d.r8(0x43), d.r8(0x44), d.r8(0x45)];
    for v in (0..=top).rev() {
        if en[v] == 0 {
            continue;
        }
        let src = 0x499 + if v == 3 { 0x200 } else { 0 };
        let b: [u8; 256] = d.m[src..src + 256].try_into().unwrap();
        d.b[2] = b;
        d.w32(0xf0, 3);
        let (val, sm) = match v {
            1 => (0x55, 8),
            2 => (0xaa, 10),
            3 => (0xff, 9),
            _ => (0, 4),
        };
        d.w32(0x12c0, val);
        let mut t = Tmp::with(None);
        t.b = conv();
        t.ever = [true; 64];
        let al = d.r32(0x12c0);
        let mut maps = [None; 4];
        for k in 0..4 {
            let sm = match (al as u8 as u32 >> (2 * k)) & 3 {
                1 => 8,
                2 => 10,
                3 => 9,
                _ => 4,
            };
            if sm != 3 {
                maps[k] = Some((3, sm));
            }
        }
        log.push((3, lay_holes(d, &maps)));
        scramble(3, &mut t, &log.last().unwrap().1.clone());
        tid_in(d, &mut t)?;
        let o = 0xb99 + v * 0x100;
        let b = d.b[2];
        d.m[o..o + 256].copy_from_slice(&b);
        let a: Vec<u8> = d.m[0x34c..0x38c].to_vec();
        d.m[0xf99 + 32 * v..0xf99 + 32 * v + 32].copy_from_slice(&a[..32]);
        d.m[0x1019 + 32 * v..0x1019 + 32 * v + 32].copy_from_slice(&a[32..]);
        for e in 0..32u32 {
            let c = get(d, (8 * e) & 0xff, 4)?;
            let a = set(d, &c, sm)?;
            d.m[0x10dc + 32 * v + e as usize] = a as u8;
        }
    }
    Ok(())
}

pub(crate) fn subbed(d: &mut Dcc) -> Result<(), i32> {
    let b2 = d.b[2];
    let w: Vec<u8> = d.m[0x34c..0x38c].to_vec();
    for k in 0..4 {
        far_base(d, k)?;
    }
    let n = d.b[2];
    d.m[0x1180..0x1280].copy_from_slice(&n);
    let a: Vec<u8> = d.m[0x34c..0x38c].to_vec();
    d.m[0x1280..0x12c0].copy_from_slice(&a);
    d.b[2] = b2;
    d.m[0x34c..0x38c].copy_from_slice(&w);
    Ok(())
}
