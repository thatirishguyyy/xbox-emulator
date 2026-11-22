use super::bits::{seg_info_checked, sizes_differ};
use super::px::{get, neighbor, set, Coord};
use super::{Dcc, OBJ};
use crate::layout::E_INVALIDARG;

const RAN_OUT: &str = "Ran out of compressed block data";
const VRS: &str = "hasBase == 0 and isConstant == 0 ... Should be VRS mode ... ";

fn bt(v: u32, i: u32) -> bool {
    (v >> (i & 31)) & 1 != 0
}

fn shl8(v: u8, c: u32) -> u8 {
    let c = c & 31;
    if c >= 8 { 0 } else { v << c }
}

fn shr8(v: u8, c: u32) -> u8 {
    let c = c & 31;
    if c >= 8 { 0 } else { v >> c }
}

fn m64(n: u32) -> u64 {
    1u64.wrapping_shl(n).wrapping_sub(1)
}

fn at(o: usize, n: usize) -> Result<usize, i32> {
    if o.checked_add(n).map_or(false, |e| e <= OBJ) { Ok(o) } else { Err(E_INVALIDARG) }
}

fn rd(d: &Dcc, o: usize) -> Result<u32, i32> {
    Ok(d.r32(at(o, 4)?))
}

fn bix(i: u32) -> Result<usize, i32> {
    if i < 256 { Ok(i as usize) } else { Err(E_INVALIDARG) }
}

fn differ(meta: u32, off: u32) -> Result<bool, i32> {
    sizes_differ(meta, off).ok_or(E_INVALIDARG)
}

fn seg_bit(d: &Dcc, k: u32) -> Result<bool, i32> {
    Ok(bt(rd(d, 0x5c + 4 * (k >> 5) as usize)?, k))
}

fn set_bit(d: &mut Dcc, o: usize, i: u32, on: bool) -> Result<(), i32> {
    let w = rd(d, o)?;
    let b = 1u32 << (i & 31);
    d.w32(o, if on { w | b } else { w & !b });
    Ok(())
}

fn dbg(d: &Dcc, s: &str) {
    if d.r8(0x46) != 0 {
        print!("DCC_EMU_DEBUG: {}", s);
    }
}

fn log(d: &Dcc, name: &str, k: usize) {
    if d.r8(0x46) == 0 {
        return;
    }
    dbg(d, &format!(" {} {{31B:0B}}\n", name));
    for row in 0..8u32 {
        print!("{:<55}", " ");
        let on = bt(d.r32(0x5c + 4 * ((row >> 1) >> 5) as usize), row >> 1);
        for e in (1..=32u32).rev() {
            let bx = row * 32 + e - 1;
            if on {
                print!("{:02x}", d.b[k][bx as usize]);
            } else {
                print!("xx");
            }
            if bx & 1 == 0 && bx & 0x1f != 0 {
                print!("_");
            }
        }
        print!("\n");
    }
    print!("\n");
}

fn msg(d: &mut Dcc, sector: u32, text: &str) -> Result<(), i32> {
    let s = format!("Key={:02x} Sector={}: {}", d.r8(0x58), sector as i32, text);
    let b = s.as_bytes();
    let o = at(0x13c8, b.len() + 1)?;
    d.m[o..o + b.len()].copy_from_slice(b);
    d.m[o + b.len()] = 0;
    if d.r8(0x69) != 0 {
        if d.r8(0x68) != 0 {
            print!("DCC ERROR: {}\n", s);
        } else {
            print!("DCC WARNING: {}\n", s);
        }
    }
    if d.r64(0x13c0) == 0 {
        d.w64(0x13c0, 1);
    }
    Ok(())
}

fn narrow(d: &mut Dcc) -> (u32, u32) {
    let (b, s) = (d.r32(4), d.r32(0));
    let mut c = b;
    if b == 16 {
        d.w32(4, 8);
        c = 8;
    }
    if s == 9 {
        d.w32(0, if c < 8 { 6 } else { 7 });
    }
    (b, s)
}

fn step(c: &mut Coord, y_first: bool) {
    if y_first {
        if c[1] != 0 {
            c[1] -= 1;
        } else if c[0] != 0 {
            c[0] -= 1;
        }
    } else if c[0] != 0 {
        c[0] -= 1;
    } else if c[1] != 0 {
        c[1] -= 1;
    }
}

pub(crate) fn prev(d: &Dcc, sel: u32, addr: u32, mode: u32) -> Result<u32, i32> {
    match sel {
        0 => neighbor(d, addr, mode),
        1 | 2 => {
            let mut c = get(d, addr, mode)?;
            step(&mut c, sel == 2);
            let a = set(d, &c, mode)?;
            if (a ^ addr) & !63 != 0 {
                neighbor(d, addr, mode)
            } else {
                set(d, &c, mode)
            }
        }
        _ => Ok(0),
    }
}

const TAINT: usize = 0x6a;
const UNDEF0: usize = 0x6b;

fn undef(d: &Dcc, buf: usize, k: u32) -> bool {
    match buf {
        0 => d.r8(UNDEF0) >> k & 1 != 0,
        2 => d.r8(TAINT) >> k & 1 != 0,
        _ => false,
    }
}

fn mark(d: &mut Dcc, buf: usize, k: u32, u: bool) {
    let o = match buf {
        0 => UNDEF0,
        2 => TAINT,
        _ => return,
    };
    let v = d.r8(o);
    d.w8(o, if u { v | 1 << k } else { v & !(1u8 << k) });
}

#[cfg(test)]
thread_local! {
    pub(crate) static FEED: std::cell::RefCell<Option<[std::collections::VecDeque<[u8; 64]>; 4]>> = const { std::cell::RefCell::new(None) };
    pub(crate) static SITE: std::cell::Cell<usize> = const { std::cell::Cell::new(9) };
}

#[cfg(test)]
fn fed() -> Option<[u8; 64]> {
    let site = SITE.with(|s| s.replace(9));
    FEED.with(|f| f.borrow_mut().as_mut().and_then(|q| q.get_mut(site).map(|q| q.pop_front().unwrap_or([0; 64]))))
}

#[cfg(not(test))]
fn fed() -> Option<[u8; 64]> {
    None
}

#[cfg(test)]
fn site(k: usize) {
    SITE.with(|s| s.set(k));
}

#[cfg(not(test))]
fn site(_: usize) {}

pub(crate) fn remap(d: &mut Dcc, dst: usize, src: usize, maps: &[Option<(u32, u32)>; 4]) -> Result<(), i32> {
    let meta = d.r32(0x58);
    let s = d.b[src];
    let mut tmp = [0u8; 64];
    let mut ever = [false; 64];
    if maps.iter().any(|m| m.is_some()) {
        if let Some(b) = fed() {
            tmp = b;
            ever = [true; 64];
        }
    }
    for k in 0..4u32 {
        if !bt(d.r32(0x5c), k) {
            continue;
        }
        let o = (k * 64) as usize;
        let Some((from, to)) = maps[k as usize] else {
            let u = undef(d, src, k);
            if u && (dst != 2 || differ(meta, k * 64)?) {
                return Err(E_INVALIDARG);
            }
            d.b[dst][o..o + 64].copy_from_slice(&s[o..o + 64]);
            mark(d, dst, k, u);
            continue;
        };
        for a in o..o + 64 {
            let c = get(d, a as u32, from)?;
            let b = (set(d, &c, to)? & 0x3f) as usize;
            tmp[b] = s[a];
            ever[b] = true;
        }
        let holes = ever.iter().any(|&x| !x);
        let tainted = undef(d, src, k);
        if (holes || tainted) && (dst != 2 || differ(meta, k * 64)?) {
            return Err(E_INVALIDARG);
        }
        d.b[dst][o..o + 64].copy_from_slice(&tmp);
        mark(d, dst, k, holes || tainted);
    }
    Ok(())
}

pub(crate) fn relay(d: &mut Dcc, dst: usize, src: usize, mode: u32) -> Result<(), i32> {
    let from = d.r32(0xf0);
    site(match mode {
        2 => 1,
        1 => 2,
        0 => 3,
        _ => 9,
    });
    remap(d, dst, src, &[Some((from, mode)); 4])?;
    d.w32(0xf0, mode);
    Ok(())
}

fn lowest(v: u32) -> u32 {
    (0..32).find(|&i| (v >> i) & 1 != 0).unwrap_or(u32::MAX)
}

fn highest(v: u32) -> u32 {
    (0..32).rev().find(|&i| (v >> i) & 1 != 0).unwrap_or(u32::MAX)
}

pub(crate) fn unpack(d: &mut Dcc) -> Result<(), i32> {
    log(d, "data [Input Compressed Data]", 1);
    let meta = d.r32(0x58);
    if meta & 0xf == 0 {
        return Ok(());
    }
    let (sb, s0) = narrow(d);
    let segs = seg_info_checked(meta).ok_or(E_INVALIDARG)?;
    let old = d.r32(0x5c);
    let mut m5c = 0u32;
    for i in 0..4 {
        if bt(old, i) {
            m5c |= segs[i as usize];
        }
    }
    d.w32(0x5c, m5c);
    d.w32(0x12c0, 0);
    let r = unpack_segs(d, meta, &segs);
    d.w32(4, sb);
    d.w32(0, s0);
    if r? {
        log(d, "Decoder: Post Unpack", 2);
    }
    Ok(())
}

fn unpack_segs(d: &mut Dcc, meta: u32, segs: &[u32; 8]) -> Result<bool, i32> {
    for k in 0..4u32 {
        let base = (k * 64) as usize;
        let copy = !seg_bit(d, k)? || !differ(meta, k * 64)?;
        if copy {
            let s: [u8; 64] = d.b[1][base..base + 64].try_into().unwrap();
            d.b[0][base..base + 64].copy_from_slice(&s);
            mark(d, 0, k, false);
            continue;
        }
        let hm = segs[4 + k as usize];
        let lo = segs[k as usize];
        let (lw, hg) = (lowest(lo), highest(lo));
        if lw != k {
            continue;
        }
        if hg > 3 {
            return Err(E_INVALIDARG);
        }
        let mut cur = k * 64;
        for r8 in lw..=hg {
            let o = r8 as usize;
            let v = d.b[1][bix(cur)?];
            d.w32(0x1368 + 4 * o, v as u32);
            let v = d.b[1][bix(cur + 1)?];
            d.w32(0x12cc + 4 * o, v as u32);
            cur += 2;
            d.b[2][o * 64..o * 64 + 64].fill(0);
            d.w8(TAINT, d.r8(TAINT) & !(1u8 << r8));
        }
        let ko = k as usize;
        let s30 = if d.r32(0x1368 + 4 * ko) & 1 == 0 && d.r32(0x12cc + 4 * ko) & 1 == 0 {
            let v = d.b[1][bix(cur)?];
            cur += 1;
            d.w32(0x12c0, d.r32(0x12c0) | v as u32);
            d.w32(0x1368 + 4 * ko, d.r32(0x1368 + 4 * ko) | 1);
            d.w32(0x12cc + 4 * ko, d.r32(0x12cc + 4 * ko) & !1);
            false
        } else {
            true
        };
        if d.r8(0x46) != 0 {
            print!("\n");
            print!("{:<55}", " ");
            print!("ddsMode                 : {:02x}\n", d.r32(0x12c0));
        }
        for j in lw..=hg {
            let jo = j as usize;
            let s34 = j.wrapping_mul(64).wrapping_neg();
            d.w8(0x1c8 + jo, j as u8);
            for i in 0..8u32 {
                let esi = (j * 8 + i) * 8;
                d.w32(0xf0, 4);
                let r14 = prev(d, d.r32(0x12c8), esi, 4)?;
                let r10 = r14 >> 6;
                let dl = r10 >= lw && r10 <= hg;
                d.w8(0x395 + jo, dl as u8);
                if r10 != j && r10 != d.r8(0x1c8 + jo) as u32 {
                    d.w8(0x1c8 + jo, r10 as u8);
                }
                let c2c = d.r32(0x2c);
                let al = c2c.wrapping_sub(1) & !2 == 0 || (c2c == 2 && s34 & 0x7f == 0);
                let mut r11 = dl && (r10 == j || !al);
                d.w32(0x64, if c2c == 2 { if j < 2 { 3 } else { 0xc } } else { 0xf });
                if (j as i32) >= (r10 as i32) {
                    let mut r8 = j as i64;
                    for _ in 0..(j - r10 + 1) {
                        let w = (r8 as u64 >> 5) as usize;
                        let b = (r8 as u32) & 31;
                        if !(bt(rd(d, 0x5c + 4 * w)?, b) && bt(rd(d, 0x60 + 4 * w)?, b)) {
                            r11 = false;
                        }
                        r8 -= 1;
                    }
                }
                let w = (r10 >> 5) as usize;
                let dl2 = esi != r14 && bt(rd(d, 0x5c + 4 * w)?, r10) && r11 && bt(rd(d, 0x64 + 4 * w)?, r10);
                set_bit(d, 0x158 + 4 * jo, i, dl2 && r10 != j)?;
            }
        }
        let mut t = [[0u8; 8]; 4];
        for j in lw..=hg {
            let jo = j as usize;
            let mode = (d.r8(0x12c0) as u32 >> (2 * j)) & 3;
            if mode != 0 {
                t[jo] = [0; 8];
                for i in 0..8u32 {
                    let c = get(d, (j * 64 + 8 * i) & 0xff, 4)?;
                    let sm = match mode {
                        2 => 10,
                        3 => 9,
                        _ => 8,
                    };
                    let a = set(d, &c, sm)?;
                    if bt(d.r32(0x158 + 4 * jo), i) {
                        let al = a & 0xff;
                        t[(al >> 6) as usize][((al >> 3) & 7) as usize] = 1;
                    }
                }
            } else {
                let w = d.r32(0x158 + 4 * jo);
                for b in 0..8 {
                    t[jo][b] = ((w >> b) & 1) as u8;
                }
            }
        }
        let lim = (hg + 1) << 6;
        let mut s50 = cur;
        for j in lw..=hg {
            let jo = j as usize;
            d.w8(0x1c8 + jo, j as u8);
            for i in 0..8u32 {
                let io = i as usize;
                let r9 = 1u32 << i;
                let p34 = 0x34c + 8 * jo + io;
                if d.r32(0x12cc + 4 * jo) & r9 != 0 {
                    d.w8(p34, 0);
                }
                let r11 = ((8 * j + i) * 8) as usize;
                if d.r32(0x1368 + 4 * jo) & r9 != 0 {
                    if s50 >= lim || !bt(hm, s50 >> 5) {
                        msg(d, j, RAN_OUT)?;
                        return Ok(false);
                    }
                    let mut r8 = d.b[1][bix(s50)?];
                    s50 += 1;
                    if !s30 && j == lw && i == 0 && r8 == 0 {
                        r8 = d.b[1][bix(s50)?];
                        s50 += 1;
                        d.w32(0x12cc + 4 * jo, d.r32(0x12cc + 4 * jo) | 1);
                        d.w8(p34, 0);
                    }
                    if d.r32(0x12cc + 4 * jo) & r9 == 0 {
                        let tz = (0..32u32).find(|&e| shr8(r8, e) & 1 != 0).unwrap_or(u32::MAX);
                        let dl = tz.wrapping_add(1) & 0xff;
                        d.w8(p34, dl as u8);
                        r8 = shl8(shr8(r8, dl), dl);
                    }
                    d.w8(0x1a8 + 8 * jo + io, r8);
                    d.b[2][r11] = r8;
                } else if d.r32(0x12cc + 4 * jo) & r9 != 0 {
                    d.w8(0x1a8 + 8 * jo + io, 0);
                    d.b[2][r11] = 0;
                } else if t[jo][io] != 0 {
                    if j == 0 {
                        return Err(E_INVALIDARG);
                    }
                    let v = d.r8(0x34c + 8 * (jo - 1) + io);
                    d.w8(p34, v);
                } else if i != 0 {
                    let v = d.r8(p34 - 1);
                    d.w8(p34, v);
                } else {
                    msg(d, j, VRS)?;
                }
            }
        }
        for j in lw..=hg {
            let jo = j as usize;
            if (d.r8(0x12c0) as u32 >> (2 * j)) & 3 == 0 {
                continue;
            }
            let on = d.r8(0x395 + jo) != 0 && j == lw;
            let o = 0x1368 + 4 * jo;
            let mut w = d.r32(o);
            w = if on { w | 1 } else { w & !1 };
            for b in 1..8u32 {
                let bpe = d.r32(4);
                if bpe == 0 {
                    return Err(E_INVALIDARG);
                }
                let q = 8 / bpe;
                if q == 0 {
                    return Err(E_INVALIDARG);
                }
                if b % q == 0 && on {
                    w |= 1 << b;
                } else {
                    w &= !(1 << b);
                }
            }
            d.w32(o, w);
        }
        let mut cur = s50;
        for j in lw..=hg {
            let jo = j as usize;
            for i in 0..8usize {
                let cnt = d.r8(0x34c + 8 * jo + i) as i32;
                let mut n = 0i32;
                while n < cnt {
                    if cur >= lim || !bt(hm, cur >> 5) {
                        msg(d, j, RAN_OUT)?;
                        return Ok(false);
                    }
                    let v = d.b[1][bix(cur)?];
                    cur += 1;
                    let base = (i + jo * 8) * 8;
                    for b in 0..8 {
                        d.b[2][base + b] |= shl8((v >> b) & 1, n as u32);
                    }
                    n += 1;
                }
            }
        }
    }
    Ok(true)
}

pub(crate) fn tid_swizzle(d: &mut Dcc) -> Result<(), i32> {
    let meta = d.r32(0x58);
    if meta & 0xf == 0 {
        return Ok(());
    }
    let sb = d.r32(4);
    if sb == 16 {
        d.w32(4, 8);
    }
    let r = tid_body(d, meta);
    d.w32(4, sb);
    r?;
    log(d, "Decoder: Post Sector to TID Swizzle", 2);
    Ok(())
}

fn tid_body(d: &mut Dcc, meta: u32) -> Result<(), i32> {
    let al = d.r8(0x12c0) as u32;
    let mut gm = [4u32; 4];
    for k in 0..4 {
        gm[k] = match (al >> (2 * k)) & 3 {
            1 => 8,
            2 => 10,
            3 => 9,
            _ => 4,
        };
    }
    let mut maps = [None; 4];
    for k in 0..4 {
        if gm[k] != 3 {
            maps[k] = Some((gm[k], 3));
        }
    }
    site(0);
    remap(d, 2, 2, &maps)?;
    for q in 0..8u32 {
        if !seg_bit(d, q >> 1)? || !differ(meta, 32 * q)? {
            continue;
        }
        let qo = q as usize;
        for i in 0..8u32 {
            let c = get(d, 32 * q + 4 * i, 3)?;
            let a = set(d, &c, 4)?;
            let r8 = (a >> 3) & 7;
            if a & 7 == 0 {
                let r10 = (a >> 6) as usize;
                let src = at(0x1a8 + r10 * 8 + r8 as usize, 1)?;
                let v = d.m[src];
                d.w8(0x168 + 8 * qo + i as usize, v);
                let b = bt(rd(d, 0x1368 + 4 * r10)?, r8);
                set_bit(d, 0x1378 + 4 * qo, i, b)?;
                let b = bt(rd(d, 0x158 + 4 * r10)?, r8);
                set_bit(d, 0x138 + 4 * qo, i, b)?;
            } else {
                set_bit(d, 0x1378 + 4 * qo, i, false)?;
                set_bit(d, 0x138 + 4 * qo, i, false)?;
            }
        }
    }
    Ok(())
}

fn flip(v: u8) -> u8 {
    let h = v >> 1;
    if v & 1 != 0 { !h } else { h }
}

pub(crate) fn dedelta_transform(d: &mut Dcc) -> Result<(), i32> {
    let meta = d.r32(0x58);
    if meta & 0xf == 0 {
        return Ok(());
    }
    for k in 0..4u32 {
        let mode = (d.r8(0x12c0) as u32 >> (2 * k)) & 3;
        if !seg_bit(d, k)? || !differ(meta, k * 64)? {
            continue;
        }
        for j in 0..32u32 {
            let e = k * 64 + 2 * j;
            for x in [e, e + 1] {
                let skip = mode == 0 && x & 3 == 0 && bt(rd(d, 0x1378 + 4 * (x >> 5) as usize)?, (x >> 2) & 7);
                if !skip {
                    let xo = x as usize;
                    d.b[2][xo] = flip(d.b[2][xo]);
                }
            }
        }
    }
    log(d, "Decoder: Post DeDelta Transform", 2);
    Ok(())
}

pub(crate) fn dedelta_calc(d: &mut Dcc) -> Result<(), i32> {
    let meta = d.r32(0x58);
    if meta & 0xf == 0 {
        return Ok(());
    }
    let (sb, s0) = narrow(d);
    let r = calc_body(d, meta);
    d.w32(4, sb);
    d.w32(0, s0);
    r?;
    log(d, "Decoder: Post DeDelta Calculation", 2);
    Ok(())
}

fn calc_body(d: &mut Dcc, meta: u32) -> Result<(), i32> {
    let bpe = d.r32(4);
    let pl = super::px::planes(d);
    if pl == 0 {
        return Err(E_INVALIDARG);
    }
    let q = bpe / pl;
    let f69 = d.r32(0x48).wrapping_sub(1) > 0xe && d.r8(0x10) != 0;
    let thr = if f69 { bpe.wrapping_sub(q) } else { q };
    let mut r15 = 0u32;
    for k in 0..4u32 {
        let m = (d.r8(0x12c0) as u32 >> (2 * k)) & 3;
        d.w32(0x12c8, if m == 3 { 2 } else { 0 });
        if seg_bit(d, k)? && differ(meta, k * 64)? {
            let segs = seg_info_checked(meta).ok_or(E_INVALIDARG)?;
            let lo = segs[k as usize];
            for i in 0..64u32 {
                let esi = k * 64 + i;
                let mode = d.r32(0xf0);
                let r10 = prev(d, d.r32(0x12c8), esi, mode)?;
                let ns = r10 >> 6;
                let mut r11 = ns != k;
                let c2c = d.r32(0x2c);
                let al = c2c.wrapping_sub(1) & !2 == 0 || (c2c == 2 && r15 & 0x7f == 0);
                let r8 = !(ns != k && al);
                if ns != k && ns >= 32 {
                    return Err(E_INVALIDARG);
                }
                let dl = ns != k && !bt(lo, ns);
                let eax = r8 && !dl;
                let r9 = if esi != r10 && eax {
                    true
                } else {
                    r11 = false;
                    false
                };
                if esi & 7 == 0 {
                    set_bit(d, 0x138 + 4 * (esi >> 5) as usize, (esi >> 2) & 7, r11)?;
                }
                if !r9 {
                    continue;
                }
                if esi & 3 == 0 && bt(rd(d, 0x1378 + 4 * (esi >> 5) as usize)?, (esi >> 2) & 7) {
                    continue;
                }
                let mut p = d.b[2][esi as usize];
                let rf = d.b[2][bix(r10)?];
                if d.r32(0x50) != 0 || d.r32(0x54) != 0 {
                    let c = get(d, esi, d.r32(0xf0))?;
                    let mut sel = c[3] >= thr;
                    if !f69 {
                        sel = !sel;
                    }
                    let v = if sel { d.r32(0x50) } else { d.r32(0x54) };
                    if q == 0 {
                        return Err(E_INVALIDARG);
                    }
                    let sh = (v.wrapping_sub((c[3] % q) << 3) as i32).max(0);
                    p = shl8(p, sh as u32);
                }
                d.b[2][esi as usize] = p.wrapping_add(rf);
                if undef(d, 2, r10 >> 6) {
                    mark(d, 2, esi >> 6, true);
                }
            }
        }
        r15 = r15.wrapping_sub(64);
    }
    Ok(())
}

pub(crate) fn color(d: &mut Dcc) -> Result<(), i32> {
    let meta = d.r32(0x58);
    if meta & 0xf == 0 {
        return Ok(());
    }
    let mut r15 = d.r32(0x4c);
    if r15 == 0 {
        if d.r32(8).wrapping_sub(1) <= 1 {
            return Ok(());
        }
        let v = d.i32(0x48);
        if v > 0 && v <= 0xb {
            r15 = 3;
        } else if v > 0 && v <= 0xf {
            return Ok(());
        } else {
            r15 = if d.r8(0x10) != 0 { 2 } else { 3 };
        }
    }
    if r15 == 1 {
        return Ok(());
    }
    let q4 = d.r32(4) >> 2;
    for k in 0..8u32 {
        if !seg_bit(d, k >> 1)? || !differ(meta, k * 32)? {
            continue;
        }
        if q4 == 0 {
            return Err(E_INVALIDARG);
        }
        let (eax, esi, r14): (i32, i32, i32) = if r15 == 2 { (8, -8, 8) } else { (16, 8, -8) };
        let b = (k * 32) as i32 + eax;
        for t in 0..8i32 {
            let p = (b + t) as usize;
            let ps = (b + t + esi) as usize;
            let pr = (b + t + r14) as usize;
            let rem = (t as u32) % q4;
            let sh = ((d.r32(0x54).wrapping_sub(rem << 3)) as i32).max(0) as u32;
            let a = d.b[2][ps];
            let dd = shl8(shr8(a, sh & 0xff), sh & 0xff);
            let e = d.b[2][p];
            d.b[2][pr] = d.b[2][pr].wrapping_add(dd);
            d.b[2][p] = a;
            d.b[2][ps] = e.wrapping_add(dd);
        }
    }
    log(d, "Decoder: Post Color Transform", 2);
    Ok(())
}

pub(crate) fn signfix(d: &mut Dcc) -> Result<(), i32> {
    let v = d.i32(0x48);
    let r8 = v > 0 && (v <= 6 || (v as u32).wrapping_sub(8) <= 3);
    if d.r32(0xc) != 0 {
        return Ok(());
    }
    let s = d.r32(0);
    let edx = !(s.wrapping_sub(6) <= 4 || s == 0x13);
    if edx && r8 {
        return Ok(());
    }
    if v == 7 {
        return Ok(());
    }
    let cm = d.r32(8);
    if cm == 0 {
        return Err(E_INVALIDARG);
    }
    let ebx = d.r32(4) / cm;
    let mut r9 = 0u32;
    let mut st = d.r32(4);
    while r9 < 0x100 {
        if seg_bit(d, r9 >> 6)? {
            let mut c = 0u32;
            while c < d.r32(4) {
                let sel = if ebx == 1 {
                    c & 3 != 0
                } else {
                    if ebx == 0 {
                        return Err(E_INVALIDARG);
                    }
                    c % ebx == ebx - 1
                };
                if sel {
                    let o = bix(c.wrapping_add(r9))?;
                    let b = d.b[2][o];
                    if (b as i8) < 0 {
                        d.b[2][o] = b ^ 0x7f;
                    }
                }
                c += 1;
            }
            st = d.r32(4);
        }
        if st == 0 {
            return Err(E_INVALIDARG);
        }
        r9 = r9.wrapping_add(st);
    }
    Ok(())
}

pub(crate) fn channel(d: &mut Dcc) -> Result<(), i32> {
    let meta = d.r32(0x58);
    if meta & 0xf == 0 {
        return Ok(());
    }
    let v48 = d.r32(0x48);
    if v48 == 0 || v48 == 0xc {
        return Ok(());
    }
    let (a29, a21, a25, p7f, b6f, b77): (u32, u32, u32, u32, bool, bool) = match v48 {
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
    };
    let mut o = 0u32;
    while o < 0x100 {
        if seg_bit(d, o >> 6)? && differ(meta, (o >> 6) << 6)? {
            let bpe = d.r32(4);
            let x = o as usize;
            let mut l = [0u32; 9];
            l[8] = o;
            let mut edi = 0u32;
            if bpe == 2 {
                edi = d.b[2][bix(o + 1)?] as u32;
                l[0] = d.b[2][x] as u32;
                l[3] = edi;
            } else if bpe == 4 {
                l[1] = d.b[2][bix(o + 1)?] as u32;
                edi = d.b[2][bix(o + 2)?] as u32;
                l[7] = d.b[2][bix(o + 3)?] as u32;
                l[0] = d.b[2][x] as u32;
                l[3] = edi;
            }
            let rb = |l: &[u32; 9], k: u32| -> Result<bool, i32> {
                let w = (k >> 5) as usize;
                if w >= 9 {
                    return Err(E_INVALIDARG);
                }
                Ok(bt(l[w], k))
            };
            let (xx, yy, zz, ww);
            if p7f < 8 {
                let mut cx = edi & 1;
                let mut cy = (edi >> 1) & 1;
                let mut k = 0u32;
                for j in 1..=4u32 {
                    if a21 > j {
                        if rb(&l, k)? { cx |= 1 << j } else { cx &= !(1 << j) }
                        k += 1;
                    }
                    if a29 > j {
                        if rb(&l, k)? { cy |= 1 << j } else { cy &= !(1 << j) }
                        k += 1;
                    }
                }
                ww = (a25 != 0 && edi >= 0x80) as u32;
                zz = (edi >> 2) & 1u32.wrapping_shl(6u32.wrapping_sub(a25)).wrapping_sub(1);
                xx = cx;
                yy = cy;
            } else {
                let (r15, r13, es) = (a29 as i32 - 8, a21 as i32 - 8, p7f as i32 - 8);
                let mut k = 0u32;
                for j in 0..=2i32 {
                    let order: [(i32, usize); 3] = if v48 == 7 { [(es, 4), (r15, 5), (r13, 2)] } else { [(r13, 2), (es, 4), (r15, 5)] };
                    for (lim, slot) in order {
                        if j < lim {
                            let b = rb(&l, k)?;
                            k += 1;
                            let m = 1u32 << j;
                            l[slot] = if b { l[slot] | m } else { l[slot] & !m };
                        }
                    }
                }
                let mut w = 0u32;
                if a25 != 0 {
                    w = rb(&l, k)? as u32;
                    k += 1;
                }
                for (n, bitv) in [(1u32, 2u32), (2, 4), (3, 8)] {
                    if a25 > n {
                        if rb(&l, k)? {
                            w |= bitv;
                        }
                        k += 1;
                    }
                }
                if a25 > 4 && rb(&l, k)? {
                    w |= 0x10;
                }
                ww = w;
                if b6f {
                    zz = l[3] | (l[4] << 8);
                    xx = (l[2] << 8) | l[1];
                    yy = (l[5] << 8) | l[7];
                } else {
                    xx = l[2] | l[1].wrapping_shl(a21 - 8);
                    zz = l[3].wrapping_shl(p7f - 8) | l[4];
                    yy = l[5] | l[7].wrapping_shl(a29 - 8);
                }
            }
            let v = if b77 {
                ((((ww.wrapping_shl(a29) | yy).wrapping_shl(p7f)) | zz).wrapping_shl(a21)) | xx
            } else {
                (((((yy.wrapping_shl(p7f)) | zz).wrapping_shl(a21)) | xx).wrapping_shl(a25)) | ww
            };
            let bpe = d.r32(4);
            if bpe == 2 || bpe == 4 {
                let vb = v.to_le_bytes();
                d.b[2][x] = vb[0];
                d.b[2][bix(o + 1)?] = vb[1];
                if bpe == 4 {
                    d.b[2][bix(o + 2)?] = vb[2];
                    d.b[2][bix(o + 3)?] = vb[3];
                }
            }
        }
        let bpe = d.r32(4);
        if bpe == 0 {
            return Err(E_INVALIDARG);
        }
        o = o.wrapping_add(bpe);
    }
    Ok(())
}

pub(crate) fn widths(d: &Dcc) -> u64 {
    let (mut r9, mut r8, mut r10, mut r11) = (0u32, 0u32, 0u32, 0u32);
    let mut bl = d.r8(0x10) as u32;
    let c = d.r32(0xc);
    let b = bl != 0;
    match d.r32(0x48) {
        1 | 8 => {
            r8 = 11;
            r9 = 10;
            r11 = 11;
            bl = 0;
        }
        2 | 9 => {
            r9 = 11;
            r11 = 10;
            r8 = 11;
            bl = 0;
        }
        3 | 10 => {
            r9 = if b { 0 } else { 10 };
            r10 = if b { 0 } else { 2 };
            r8 = r9;
            r11 = r9;
        }
        4 | 11 => {
            if b {
                r10 = 2;
                if c == 4 {
                    r9 = 0x1e;
                } else {
                    r9 = 10;
                    r8 = 10;
                    r11 = 10;
                }
            }
        }
        5 | 6 => {
            r8 = if b { 10 } else { 0 };
            r11 = r8;
            r9 = r8;
            r10 = if b { 2 } else { 0 };
        }
        7 => {
            r9 = 9;
            r10 = 5;
            r8 = 9;
            r11 = 9;
            bl = 1;
        }
        13 => {
            r9 = 5;
            r8 = 6;
            r11 = 5;
            bl = 0;
        }
        14 => {
            r8 = if b { 0 } else { 5 };
            r10 = (!b) as u32;
            r11 = r8;
            r9 = r8;
        }
        15 => {
            r11 = if b { 5 } else { 0 };
            r8 = r11;
            r9 = r11;
            r10 = b as u32;
        }
        _ => {
            let cm = d.r32(8);
            match d.r32(4) {
                1 => {
                    r9 = if b { 0 } else { 8 };
                    r10 = if b { 8 } else { 0 };
                    if c == 6 {
                        r10 = 0;
                        r9 = 8;
                    }
                }
                2 => {
                    if cm == 2 {
                        if c != 7 {
                            r10 = 8;
                        } else {
                            r8 = 8;
                        }
                        r9 = 8;
                    } else if cm == 4 {
                        r9 = 4;
                        r8 = 4;
                        r10 = 4;
                        r11 = 4;
                    } else {
                        r9 = if b { 0 } else { 16 };
                        r10 = if b { 16 } else { 0 };
                        if c == 6 {
                            r10 = 0;
                            r9 = 16;
                        }
                    }
                }
                4 => {
                    if cm == 1 {
                        if d.r8(0x12) != 0 {
                            bl = 0;
                            r9 = 0x20;
                        } else if !b {
                            r9 = 0x20;
                        } else {
                            r10 = 0x20;
                        }
                    } else if cm == 2 {
                        if c == 7 {
                            r9 = 0x10;
                            r8 = 0x10;
                        } else if c == 5 {
                            r9 = 8;
                            r8 = 8;
                            r10 = 8;
                            r11 = 8;
                        } else {
                            r10 = 0x10;
                            r9 = 0x10;
                        }
                    } else {
                        r10 = 8;
                        if c == 4 {
                            r9 = 0x18;
                        } else {
                            r9 = 8;
                            r8 = 8;
                            r11 = 8;
                        }
                    }
                }
                8 => {
                    if cm == 2 {
                        let cl = d.r8(0x12);
                        r9 = if cl == 0 { 0x20 } else { 0x40 };
                        if cl != 0 {
                            bl = 1;
                        }
                        r10 = if cl == 0 { 0x20 } else { 0 };
                    } else {
                        r10 = 0x10;
                        if c != 4 {
                            r9 = 0x10;
                            r8 = 0x10;
                            r11 = 0x10;
                        } else {
                            r9 = 0x30;
                        }
                    }
                }
                _ => {
                    r9 = 0x20;
                    r8 = 0x20;
                    r10 = 0x20;
                    r11 = 0x20;
                }
            }
        }
    }
    ((bl & 0xff) as u64) << 32 | ((r10 & 0xff) as u64) << 24 | ((r11 & 0xff) as u64) << 16 | ((r8 & 0xff) as u64) << 8 | (r9 & 0xff) as u64
}

fn lo796(w: u32) -> u64 {
    match w {
        8 => 0x80,
        16 => 0x8000,
        _ => 0,
    }
}

fn lo7a4(w: u32) -> u64 {
    match w {
        8 => 0x10,
        16 => 0x1000,
        _ => 0,
    }
}

pub(crate) fn clear_lo(d: &Dcc, w: u8, hi: bool) -> u64 {
    if d.r8(0x11) == 0 {
        return 0;
    }
    let w = w as u32;
    match (hi, d.r32(0xc)) {
        (true, 5) => lo7a4(w),
        (true, 7) => lo796(w),
        (true, _) => 0,
        (false, 4) => match w {
            0x18 => 0x801080,
            0x1e => 0x4080200,
            0x30 => 0x8000_1000_8000,
            _ => 0,
        },
        (false, 5) | (false, 7) => lo796(w),
        (false, 6) => lo7a4(w),
        _ => 0,
    }
}

fn hi874(w: u32) -> u64 {
    match w {
        8 => 0xf0,
        16 => 0xf000,
        _ => 0xffff_ffff,
    }
}

fn hi898(w: u32) -> u64 {
    match w {
        8 => 0xeb,
        16 => 0xeb00,
        _ => 0xffff_ffff,
    }
}

pub(crate) fn clear_hi(d: &Dcc, w: u8, hi: bool) -> u64 {
    let r10 = 0xffff_ffffu64;
    if d.r8(0x12) != 0 && d.r32(8) == 2 && d.r32(4) == 8 {
        return 0xff_3f80_0000;
    }
    let al = d.r8(0x11);
    let c = d.r32(0xc);
    let w = w as u32;
    if al != 0 {
        return match (hi, c) {
            (true, 4) => match w {
                2 => 3,
                8 => 0xff,
                16 => 0xffff,
                _ => r10,
            },
            (true, 5) | (false, 6) => hi898(w),
            (true, 7) | (false, 5) | (false, 7) => hi874(w),
            (false, 4) => match w {
                0x18 => 0x80eb80,
                0x1e => 0x3ac80200,
                0x30 => 0x8000_eb00_8000,
                _ => r10,
            },
            _ => r10,
        };
    }
    if c != 0 {
        if c == 3 {
            return if w == 0 { r10 } else { m64(w) };
        }
        let r = 1u64.wrapping_shl(w.wrapping_sub(1) & 0xff);
        return if w == 1 { r } else { r.wrapping_sub(1) };
    }
    if hi {
        match w {
            0 => 0,
            2 => 3,
            5 => 0x1f,
            16 => 0x3c00,
            32 => 0x3f80_0000,
            _ => r10,
        }
    } else {
        match w {
            0 => 0,
            9 => 0x1ff,
            10 => {
                if d.r32(8) != 4 {
                    0x1e0
                } else {
                    match d.i32(0x48) {
                        4 | 5 => 0x1c0,
                        6 => 0x180,
                        _ => r10,
                    }
                }
            }
            11 => 0x3c0,
            16 => 0x3c00,
            32 => 0x3f80_0000,
            _ => r10,
        }
    }
}

pub(crate) fn constant(d: &mut Dcc) -> Result<(), i32> {
    let meta = d.r32(0x58);
    if meta & 0xf != 0 {
        return Ok(());
    }
    let wd = widths(d);
    let (w0, w1, w2, w3) = ((wd & 0xff) as u32, ((wd >> 8) & 0xff) as u32, ((wd >> 16) & 0xff) as u32, ((wd >> 24) & 0xff) as u32);
    let rev = (wd >> 32) & 1 != 0;
    let bpe = d.r32(4);
    if bpe == 0 {
        return Err(E_INVALIDARG);
    }
    let n = 0x100 / bpe;
    let bu = bpe as usize;
    let wr = if (meta >> 4) & 0x3 != 0 || bpe <= 8 || bpe == 16 { n * bpe } else { 0 };
    for k in 0..(wr / 64).min(4) {
        mark(d, 0, k, false);
    }
    if (meta >> 4) & 1 != 0 {
        let src: Vec<u8> = d.b[1][..bu.min(256)].to_vec();
        for i in 0..n as usize {
            d.b[0][bu * i..bu * i + bu].copy_from_slice(&src);
        }
        return Ok(());
    }
    if (meta >> 5) & 1 != 0 {
        if bpe == 16 {
            let rc = d.r64(0x38);
            let r8 = (rc << 32) | (rc & 0xffff_ffff);
            let (even, odd) = if rev { (r8, rc) } else { (rc.rotate_left(32), r8) };
            for e in 0..(n * 2) as usize {
                let v = if e & 1 != 0 { odd } else { even };
                d.b[0][e * 8..e * 8 + 8].copy_from_slice(&v.to_le_bytes());
            }
        } else {
            let o = at(0x38, bu)?;
            let src: Vec<u8> = d.m[o..o + bu].to_vec();
            for i in 0..n as usize {
                d.b[0][bu * i..bu * i + bu].copy_from_slice(&src);
            }
        }
        return Ok(());
    }
    let b7 = (meta >> 7) & 1 != 0;
    let f = |d: &Dcc, w: u32| if b7 { clear_hi(d, w as u8, false) } else { clear_lo(d, w as u8, false) };
    let (c0, c1, c2) = (f(d, w0), f(d, w1), f(d, w2));
    let use_a = if d.r32(0x48) == 7 { b7 } else { (meta >> 6) & 1 != 0 };
    let a = if use_a { clear_hi(d, w3 as u8, true) } else { clear_lo(d, w3 as u8, true) };
    let r = c0;
    let five = d.r32(0xc) == 5;
    let g = if five { a } else { c1 };
    let b = if five { c0 } else { c2 };
    dbg(d, &format!("TB EMU: a = {:x}, r = {:x}, g = {:x}, b = {:x}\n", a, r, g, b));
    if bpe <= 8 {
        let mut r9 = 0u64;
        if rev {
            r9 = if w3 == 0x40 { a } else { a & m64(w3) };
        }
        r9 = r9.wrapping_shl(w0);
        r9 |= if w0 == 0x40 { r } else { r & m64(w0) };
        r9 = r9.wrapping_shl(w1);
        let mut px = if w1 == 0x40 {
            (g | r9).wrapping_shl(w2) | g
        } else {
            ((m64(w1) & g) | r9).wrapping_shl(w2) | (m64(w2) & b)
        };
        if !rev {
            px = px.wrapping_shl(w3) | if w3 == 0x40 { a } else { a & m64(w3) };
        }
        dbg(d, &format!("\nTB EMU: aW={}, rW={}, gW={}, bW={}, pixel_lower_64b={:x}, pixel_upper_64b={:x}\n", w3, w0, w1, w2, px, 0));
        let pb = px.to_le_bytes();
        for i in 0..n as usize {
            d.b[0][bu * i..bu * i + bu].copy_from_slice(&pb[..bu]);
        }
    } else if bpe == 16 {
        let r9 = m64(w0) & r;
        let rdx = m64(w1) & g;
        let rax = m64(w2) & b;
        let r8 = m64(w3) & a;
        let (lo, hi) = if rev {
            (rax | rdx.wrapping_shl(w2), r8.wrapping_shl(w0) | r9)
        } else {
            (rax.wrapping_shl(w3) | r8, r9.wrapping_shl(w1) | rdx)
        };
        for e in 0..16usize {
            d.b[0][16 * e..16 * e + 8].copy_from_slice(&lo.to_le_bytes());
            d.b[0][16 * e + 8..16 * e + 16].copy_from_slice(&hi.to_le_bytes());
        }
    }
    Ok(())
}

pub fn run(d: &mut Dcc) -> Result<(), i32> {
    if d.r8(UNDEF0) & 0x80 == 0 {
        d.w8(UNDEF0, 0x8f);
        d.w8(TAINT, d.r8(TAINT) | 0xf);
    }
    stages(d)?;
    if d.r8(UNDEF0) & 0xf != 0 {
        return Err(E_INVALIDARG);
    }
    Ok(())
}

fn stages(d: &mut Dcc) -> Result<(), i32> {
    d.w64(0x13c0, 0);
    unpack(d)?;
    tid_swizzle(d)?;
    if d.r32(0x58) & 0xf != 0 {
        remap(d, 2, 2, &[None; 4])?;
        log(d, "Decoder: Post DeDeltaBase Swizzle", 2);
    }
    d.w32(0xf0, 3);
    dedelta_transform(d)?;
    dedelta_calc(d)?;
    if d.r32(0x58) & 0xf == 0 {
        d.w32(0xf0, 2);
    } else {
        relay(d, 2, 2, 2)?;
        log(d, "Decoder: Post DeBytePerChannel Separation", 2);
    }
    color(d)?;
    if d.r32(0x58) & 0xf == 0 {
        d.w32(0xf0, 1);
    } else {
        relay(d, 2, 2, 1)?;
    }
    if d.r32(0x58) & 0xf != 0 {
        signfix(d)?;
    }
    channel(d)?;
    constant(d)?;
    if d.r32(0x58) & 0xf == 0 {
        d.w32(0xf0, 0);
        return Ok(());
    }
    let saved = d.r32(0x5c);
    let meta = d.r32(0x58);
    for k in 0..4u32 {
        let on = bt(saved, k) && differ(meta, k * 64)?;
        set_bit(d, 0x5c, k, on)?;
    }
    let r = relay(d, 0, 2, 0);
    d.w32(0x5c, saved);
    r
}
