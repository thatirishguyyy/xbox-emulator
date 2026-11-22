use super::addr::{coord_1b540, index_1c960, mode_1b440, reorder_1dd10};
use super::dec::{chan_table, color_shift, kd, neighbor, shift_of, shr8, shl8, signfix, split};
use super::{Dcc, Stop, R};

fn put(wv: &mut u32, k: &mut u32, on: bool) {
    let m = 1u32 << (*k & 31);
    if on {
        *wv |= m;
    } else {
        *wv &= !m;
    }
    *k += 1;
}

fn lmask(n: u32) -> u32 {
    if (n.wrapping_sub(1) as i32) < 0 {
        0
    } else if n == 32 {
        u32::MAX
    } else {
        1u32.wrapping_shl(n).wrapping_sub(1)
    }
}

pub(crate) fn channel(d: &mut Dcc) -> R<()> {
    let v = d.g(0x24);
    if v == 0 || v == 9 {
        return Ok(());
    }
    let (a, b, cw, p, b6f, b77) = chan_table(v)?;
    let mut o = 0u32;
    loop {
        if d.mask_bit(o >> 6) {
            let bpe = d.g(4);
            if bpe == 2 || bpe == 4 {
                let ou = o as usize;
                let s = &d.s[2];
                let mut r10 = s[ou] as u32 | (s[ou + 1] as u32) << 8;
                if bpe == 4 {
                    r10 |= (s[ou + 2] as u32 | (s[ou + 3] as u32) << 8) << 16;
                }
                let mut r9 = r10;
                let mut r12 = 0u32;
                if !b77 {
                    r12 = lmask(cw) & r10;
                    r9 = r9.wrapping_shr(cw);
                }
                let edx = lmask(a) & r9;
                r9 = r9.wrapping_shr(a);
                let eax = lmask(p) & r9;
                r9 = r9.wrapping_shr(p);
                let r8 = lmask(b) & r9;
                r9 = r9.wrapping_shr(b);
                if b77 {
                    r12 = lmask(cw + 1) & r9;
                }
                let mut wv = 0u32;
                let out = if (p as i32) < 8 {
                    let hi = (((r12 & 1) << 6 | (r8 & 1)) << 1) | eax << 2 | (edx & 1);
                    let mut k = 0u32;
                    if a > 1 {
                        k = 1;
                        if edx & 2 != 0 {
                            wv = 1;
                        }
                    }
                    for (lvl, bitv) in [(1u32, 2u32), (2, 4), (3, 8), (4, 0x10)] {
                        if lvl > 1 && a > lvl {
                            put(&mut wv, &mut k, edx & bitv != 0);
                        }
                        if b > lvl {
                            put(&mut wv, &mut k, r8 & bitv != 0);
                        }
                    }
                    hi << 8 | wv
                } else {
                    let (sx, sz, sy) = if b6f { (0, 0, 0) } else { (a - 8, p - 8, b - 8) };
                    let b0 = edx.wrapping_shr(sx) & 0xff;
                    let bz = eax.wrapping_shr(sz) & 0xff;
                    let by = r8.wrapping_shr(sy) & 0xff;
                    let (x, z, y) = if b6f { (edx >> 8, eax >> 8, r8 >> 8) } else { (edx, eax, r8) };
                    let (e8, p8, b8) = ((a as i32) - 8, (p as i32) - 8, (b as i32) - 8);
                    let mut k = 0u32;
                    if e8 > 0 {
                        k = 1;
                        if x & 1 != 0 {
                            wv |= 1;
                        } else {
                            wv &= !1;
                        }
                    }
                    if p8 > 0 {
                        put(&mut wv, &mut k, z & 1 != 0);
                    }
                    if b8 > 0 {
                        put(&mut wv, &mut k, y & 1 != 0);
                    }
                    for (lvl, bitv) in [(1i32, 2u32), (2, 4)] {
                        if e8 > lvl {
                            put(&mut wv, &mut k, x & bitv != 0);
                        }
                        if p8 > lvl {
                            put(&mut wv, &mut k, z & bitv != 0);
                        }
                        if b8 > lvl {
                            put(&mut wv, &mut k, y & bitv != 0);
                        }
                    }
                    if cw as i32 > 0 {
                        put(&mut wv, &mut k, r12 & 1 != 0);
                    }
                    if cw as i32 > 1 {
                        put(&mut wv, &mut k, r12 & 2 != 0);
                    }
                    if p as i32 <= 8 {
                        bz << 8 | wv
                    } else {
                        ((((by << 8) | bz) << 8 | b0) << 8) | wv
                    }
                };
                let vb = out.to_le_bytes();
                d.s[2][ou] = vb[0];
                d.s[2][ou + 1] = vb[1];
                if d.g(4) == 4 {
                    d.s[2][ou + 2] = vb[2];
                    d.s[2][ou + 3] = vb[3];
                }
            }
        }
        let st = d.g(4);
        if st == 0 {
            return Err(Stop::Fault("channel pass never advances with 0 bytes per element"));
        }
        o = o.wrapping_add(st);
        if o >= 0x100 {
            return Ok(());
        }
    }
}

pub(crate) fn color(d: &mut Dcc) -> R<()> {
    let m = mode_1b440(d, 0x14);
    if m == 1 {
        return Ok(());
    }
    let q4 = d.g(4) >> 2;
    for t in 0..8u32 {
        if !d.mask_bit(t >> 1) {
            continue;
        }
        let (ea, es, er): (u32, u32, u32) = if m == 2 { (8, 8u32.wrapping_neg(), 8) } else { (16, 8, 8u32.wrapping_neg()) };
        let base = t * 32 + ea;
        for idx in 0..8u32 {
            let sh = color_shift(d, q4, idx)?;
            let p = (base + idx) as usize;
            let ps = (base + idx).wrapping_add(es) as usize;
            let pr = (base + idx).wrapping_add(er) as usize;
            let av = d.s[2][p];
            let dd = shl8(shr8(av, sh), sh);
            let bv = d.s[2][ps];
            let cv = d.s[2][pr];
            d.s[2][ps] = av;
            d.s[2][p] = bv.wrapping_sub(dd);
            d.s[2][pr] = cv.wrapping_sub(dd);
        }
    }
    Ok(())
}

pub(crate) fn delta(d: &mut Dcc) -> R<()> {
    let (sb, sm) = (d.g(4), d.g(0));
    let mut c = sb;
    if sb == 16 {
        c = 8;
        d.p(4, 8);
    }
    if sm == 3 {
        d.p(0, (c >= 8) as u32);
    }
    let r = delta_body(d);
    d.p(4, sb);
    d.p(0, sm);
    r
}

fn delta_body(d: &mut Dcc) -> R<()> {
    let (q, f, thr) = split(d)?;
    for k in (0..4u32).rev() {
        if !d.mask_bit(k) {
            continue;
        }
        let ku = k as usize;
        d.o[0x180 + ku] = k as u8;
        for e in (k * 64..k * 64 + 64).rev() {
            let n = neighbor(d, e, d.g(0xb0));
            let ns = n >> 6;
            let r15 = ns != k;
            let mut r11 = if ns != k {
                if d.o[0x180 + ku] as u32 != ns {
                    d.o[0x180 + ku] = ns as u8;
                }
                d.o[0x20] == 0
            } else {
                true
            };
            if k as i32 >= ns as i32 {
                for s in ns..=k {
                    r11 = r11 && d.mask_bit(s);
                }
            }
            let r10 = e != n && d.mask_bit(ns) && r11;
            let flag = r10 && r15;
            if e & 3 == 0 {
                let g = ((e >> 2) & 7) as usize;
                let w = (e >> 5) as usize;
                d.o[0x120 + w * 8 + g] = d.s[2][e as usize];
                d.put_bit(0xf0 + 4 * w, g as u32, flag)?;
            }
            let mut sv = d.s[2][e as usize];
            if r10 {
                let mut r12 = d.s[2][n as usize];
                if d.g(0x2c) != 0 || d.g(0x30) != 0 {
                    let sh = shift_of(d, e, q, f, thr)?;
                    sv = shr8(sv, sh);
                    r12 = shr8(r12, sh);
                }
                sv = sv.wrapping_sub(r12);
            }
            d.s[2][e as usize] = sv;
        }
    }
    Ok(())
}

pub(crate) fn transform(d: &mut Dcc) {
    for k in 0..4u32 {
        if !d.mask_bit(k) {
            continue;
        }
        for e in (k * 64) as usize..(k * 64 + 64) as usize {
            let v = d.s[2][e];
            let w = if v & 0x80 != 0 { !v } else { v };
            d.s[2][e] = (w << 1) | (v >> 7);
        }
    }
}

pub(crate) fn stats(d: &mut Dcc) {
    for k in 0..4u32 {
        if !d.mask_bit(k) {
            continue;
        }
        for e in (k * 64) as usize..(k * 64 + 64) as usize {
            let v = d.s[2][e];
            let bl = (8 - v.leading_zeros()) as u8;
            d.o[0x184 + e] = bl;
            let g = (e >> 5) * 8 + ((e >> 2) & 7);
            if e & 3 == 0 {
                d.o[0x284 + g] = 0;
                d.o[0x2c4 + g] = bl;
            } else {
                d.o[0x284 + g] = d.o[0x284 + g].max(bl);
                d.o[0x2c4 + g] = d.o[0x2c4 + g].max(bl);
            }
        }
    }
}

pub(crate) fn tid(d: &mut Dcc) -> R<()> {
    let sb = d.g(4);
    if sb == 16 {
        d.p(4, 8);
    }
    let r = tid_body(d);
    d.p(4, sb);
    r
}

fn tid_body(d: &mut Dcc) -> R<()> {
    reorder_1dd10(d, 2, 2, 4);
    for q in 0..8usize {
        if !d.mask_bit((q >> 1) as u32) {
            continue;
        }
        for i in 0..8usize {
            let c = coord_1b540(d, (q * 32 + 4 * i) as u32, 3);
            let a = index_1c960(d, &c, 4);
            let r9 = ((a >> 3) & 7) as usize;
            let r10 = (a >> 6) as usize;
            if r10 >= 4 {
                return Err(Stop::Undef("tid swizzle index beyond the block"));
            }
            let g = r10 * 8 + r9;
            let hi = d.o[0x2c4 + 8 * q + i];
            if a & 7 == 0 {
                d.o[0x304 + g] = d.o[0x284 + 8 * q + i];
                d.o[0x324 + g] = hi;
                let on = d.bit(0xf0 + 4 * q, i as u32);
                d.put_bit(0x110 + 4 * r10, r9 as u32, on)?;
                d.o[0x160 + g] = d.o[0x120 + 8 * q + i];
            } else {
                d.o[0x304 + g] = d.o[0x304 + g].max(hi);
                d.o[0x324 + g] = d.o[0x324 + g].max(hi);
            }
        }
    }
    Ok(())
}

pub(crate) fn plan(d: &mut Dcc) -> R<()> {
    for k in 0..4usize {
        if !d.mask_bit(k as u32) {
            continue;
        }
        let w = 4 * k;
        d.o[0x3a4 + k] = 0;
        for i in 0..8u32 {
            let iu = i as usize;
            let bit = 1u32 << i;
            let a = d.o[0x304 + 8 * k + iu];
            let b = d.o[0x324 + 8 * k + iu];
            d.put_bit(0x344 + w, i, a == 0)?;
            d.put_bit(0x354 + w, i, b == 0)?;
            d.put_bit(0x364 + w, i, b > a)?;
            let hb = d.g(0x110 + w) & bit != 0;
            let on = !hb && d.g(0x364 + w) & bit == 0 && i != 0;
            d.put_bit(0x374 + w, i, on)?;
            let on = d.g(0x374 + w) & bit != 0 && a == d.o[0x304 + 8 * k + iu - 1];
            d.put_bit(0x384 + w, i, on)?;
            let on = (d.g(0x354 + w) & bit == 0 || hb) && d.g(0x384 + w) & bit == 0;
            d.put_bit(0x394 + w, i, on)?;
            d.o[0x3a4 + k] = d.o[0x3a4 + k].wrapping_add(a);
        }
        let r8 = d.o[0x3a4 + k].wrapping_add(2);
        d.o[0x3a4 + k] = r8;
        let pc = d.g(0x394 + w).count_ones() as u8;
        d.o[0x3a8 + k] = pc.wrapping_add(r8);
        d.o[0x3dc + k] = r8;
        for i in 0..8u32 {
            let iu = i as usize;
            let bit = 1u32 << i;
            let on = d.g(0x110 + w) & bit != 0 && d.g(0x364 + w) & bit == 0;
            d.put_bit(0x3ac + w, i, on)?;
            let on = if d.g(0x3ac + w) & bit != 0 {
                if k == 0 {
                    return Err(Stop::Fault("size planner compares against sector -1 (access violation)"));
                }
                d.o[0x304 + 8 * k + iu] == d.o[0x304 + 8 * (k - 1) + iu]
            } else {
                false
            };
            d.put_bit(0x3bc + w, i, on)?;
            let on = d.g(0x354 + w) & bit == 0 && d.g(0x384 + w) & bit == 0 && d.g(0x3bc + w) & bit == 0;
            d.put_bit(0x3cc + w, i, on)?;
        }
        let pc = d.g(0x3cc + w).count_ones() as u8;
        d.o[0x3dc + k] = d.o[0x3dc + k].wrapping_add(pc);
    }
    Ok(())
}

fn key_bit(d: &mut Dcc, i: u32, on: bool) -> R<()> {
    if i >= 64 * 8 {
        return Err(Stop::Undef("key encoder bit index outside the codec object"));
    }
    d.put_bit(0x34, i, on)
}

pub(crate) fn key_encode(d: &mut Dcc, off: u32, len: u32, size: u32) -> R<()> {
    let ebp = len.wrapping_add(0x1e) >> 5;
    let r13 = len.wrapping_add(0x3f) >> 6;
    let r15 = size.wrapping_add(0x1f) >> 5;
    let esi = off >> 6;
    let r14 = (ebp << 5).wrapping_add(off).wrapping_sub(1) >> 6;
    let ecx = r14.wrapping_sub(esi).wrapping_add(1);
    let m = (if ecx == 32 { u32::MAX } else { 1u32.wrapping_shl(ecx).wrapping_sub(1) }).wrapping_shl(esi) & 0xf;
    let k0 = kd(d.g(0x34))?;
    let mut r8 = 0u32;
    for i in 0..4 {
        if m >> i & 1 != 0 {
            r8 |= k0[i];
        }
    }
    let mut key = d.g(0x34);
    for b in 0..4 {
        if r8 >> b & 1 != 0 {
            key |= 3 << (2 * b);
        }
    }
    d.p(0x34, key);
    if r15 >= ebp {
        return Ok(());
    }
    let mut r9 = esi.wrapping_mul(2);
    let r11 = r14.wrapping_mul(2).wrapping_add(1);
    if r9 <= r11 {
        if r11 - r9 > 64 {
            return Err(Stop::Undef("key encoder clears bits outside the codec object"));
        }
        for b in r9..=r11 {
            key_bit(d, b, false)?;
        }
    }
    let cnt = r13.wrapping_sub(1);
    if cnt > 64 {
        return Err(Stop::Fault("key encoder clear loop runs outside the codec object"));
    }
    for _ in 0..cnt {
        key_bit(d, r9, false)?;
        r9 += 1;
    }
    key_bit(d, r9, true)?;
    r9 += 1;
    if r13 == 4 {
        key_bit(d, r9, false)?;
        r9 += 1;
    }
    let v = (r15.wrapping_sub(1).wrapping_shl(r9)) & 0xff;
    d.p(0x34, d.g(0x34) | v);
    Ok(())
}

fn obj_dword(d: &Dcc, off: usize) -> R<u32> {
    if off + 4 > 0x420 {
        return Err(Stop::Undef("packer bit-plane loop reads past the codec object after its stack arguments were overwritten"));
    }
    Ok(d.g(off))
}

fn obj_byte(d: &Dcc, off: usize) -> R<u8> {
    if off >= 0x420 {
        return Err(Stop::Undef("packer bit-plane loop reads past the codec object after its stack arguments were overwritten"));
    }
    Ok(d.o[off])
}

fn s1(d: &mut Dcc, at: u32, v: u8) -> R<()> {
    if at >= 256 {
        d.spill.push((at - 256, v));
        return Ok(());
    }
    d.s[1][at as usize] = v;
    Ok(())
}

pub(crate) fn emit(d: &mut Dcc, first: u32, last: u32, size: u32) -> R<()> {
    let r8 = d.g(0x18);
    if r8 == 0 {
        return Err(Stop::Fault("integer divide by zero in the packer: segment size 0"));
    }
    let r10 = last.wrapping_sub(first).wrapping_add(1).wrapping_shl(6);
    let r11 = first.wrapping_shl(6);
    let need = r8.wrapping_add(size).wrapping_sub(1) / r8;
    let have = r10 / r8;
    if first > last || last >= 4 {
        return Err(Stop::Undef("packer sector range outside the block"));
    }
    if need >= have {
        for s in first..=last {
            let o = (s * 64) as usize;
            let src = d.s[0];
            d.s[1][o..o + 64].copy_from_slice(&src[o..o + 64]);
            key_encode(d, r11, r10, r10)?;
            d.o[0x410 + s as usize] = 0x40;
        }
        return Ok(());
    }
    let mut esi = r11;
    for s in first..=last {
        let o = (s * 64) as usize;
        d.s[1][o..o + 64].fill(0xcc);
        let su = s as usize;
        s1(d, esi, d.o[0x3e0 + 4 * su])?;
        s1(d, esi + 1, d.o[0x344 + 4 * su])?;
        esi += 2;
    }
    let mut home = [first, last, size];
    let mut j = first;
    loop {
        if j >= 16 {
            return Err(Stop::Undef("packer base loop reads past the codec object after its stack arguments were overwritten"));
        }
        let ju = j as usize;
        for i in 0..8u32 {
            let iu = i as usize;
            let bit = 1u32 << i;
            if d.g(0x3e0 + 4 * ju) & bit == 0 {
                continue;
            }
            let bp = d.o[0x160 + 8 * ju + iu];
            if d.g(0x344 + 4 * ju) & bit != 0 {
                s1(d, esi, bp)?;
            } else {
                let r9 = d.o[0x304 + 8 * ju + iu] as u32;
                if r9 == 0 {
                    return Err(Stop::Fault("packer sets bit -1 of its stack word (non-canonical address)"));
                }
                let slot = (r9 - 1) >> 5;
                let tmp = if slot == 0 { 1u32 << (r9 - 1) } else { 0 };
                if slot & 1 == 0 && slot != 0 {
                    let h = (slot / 2 - 1) as usize;
                    home[h] |= 1 << ((r9 - 1) & 31);
                }
                let eax = if r9 > 7 { 0 } else { (bp as u32 >> r9) & lmask(8 - r9) };
                let v = (eax.wrapping_shl(r9) & 0xff) | tmp;
                s1(d, esi, v as u8)?;
            }
            esi += 1;
        }
        j += 1;
        if j > home[1] {
            break;
        }
    }
    let mut j = home[0];
    loop {
        for i in 0..8u32 {
            let iu = i as usize;
            let sk = obj_dword(d, 0x344 + 4 * j as usize)?;
            if sk & (1 << i) != 0 {
                continue;
            }
            let cnt = obj_byte(d, 0x304 + 8 * j as usize + iu)? as i32;
            let mut n = 0i32;
            while n < cnt {
                let base = (iu + j as usize * 8) * 8;
                let mut t = 0u8;
                for b in 0..8usize {
                    let first_base = b == 0 && obj_dword(d, 0x3e0 + 4 * j as usize)? & (1 << i) != 0;
                    let src = if first_base {
                        obj_byte(d, 0x160 + 8 * j as usize + iu)?
                    } else if base + b < 256 {
                        d.s[2][base + b]
                    } else {
                        return Err(Stop::Undef("packer bit-plane loop reads past the 256-byte surface 2 heap block after its stack arguments were overwritten"));
                    };
                    if shr8(src, n as u32) & 1 != 0 {
                        t |= 1 << b;
                    }
                }
                s1(d, esi, t)?;
                esi += 1;
                n += 1;
            }
        }
        j = j.wrapping_add(1);
        if j > home[1] {
            break;
        }
    }
    let size = home[2];
    key_encode(d, r11, r10, size)?;
    let mut rem = size;
    let mut s = home[0];
    loop {
        if s >= 8 {
            return Err(Stop::Undef("packer size loop writes over the codec message pointer at 0x418 after its stack arguments were overwritten"));
        }
        let c = if rem < 0x40 { rem & 0xff } else { 0x40 };
        rem = rem.wrapping_sub(c);
        d.o[0x410 + s as usize] = c as u8;
        s += 1;
        if s > home[1] {
            break;
        }
    }
    Ok(())
}

pub(crate) fn pack(d: &mut Dcc) -> R<()> {
    let g14 = d.g(0x14);
    if g14 == 0 {
        return Err(Stop::Fault("integer divide by zero in the packer: sector granularity 0"));
    }
    let (mut newg, mut start, mut units, mut nsec, mut size) = (true, 0u32, 0u32, 0u32, 0u32);
    for k in 0..4u32 {
        if !d.mask_bit(k) {
            continue;
        }
        let ku = k as usize;
        let last = k == 3 || !d.mask_bit(k + 1);
        let st = if newg { k } else { start };
        let u_prev = if newg { 0 } else { units };
        let n_prev = if newg { 0 } else { nsec };
        let sz_prev = if newg { 0 } else { size };
        let a = (k * 64).wrapping_sub(64) / g14;
        let b = (k * 64) / g14;
        let r9b = a == b && st <= d.o[0x180 + ku] as u32;
        let src = if r9b { 0x3dc + ku } else { 0x3a8 + ku };
        let sz = (d.o[src] as u32).wrapping_add(sz_prev);
        let mut ok = sz <= d.g(0x1c) && a == b;
        let r12 = d.g(0x18);
        if r12 == 0 {
            return Err(Stop::Fault("integer divide by zero in the packer: segment size 0"));
        }
        let nu = r12.wrapping_add(sz).wrapping_sub(1) / r12;
        let ecx = n_prev.wrapping_add(1).wrapping_shl(6);
        let al = last && (0x40 / r12).wrapping_add(u_prev) == nu;
        ok = ok && sz < ecx && !al;
        let sel = if r9b && ok { 0x3cc } else { 0x394 } + 4 * ku;
        let v = d.g(sel);
        d.p(0x3e0 + 4 * ku, v);
        for bb in 0..8usize {
            if d.o[0x3e0 + 4 * ku] >> bb & 1 != 0 {
                d.s[2][ku * 64 + 8 * bb] = d.o[0x160 + 8 * ku + bb];
            }
        }
        if ok {
            start = st;
            nsec = n_prev + 1;
            units = nu;
            size = sz;
        } else {
            if n_prev != 0 {
                emit(d, st, k.wrapping_sub(1), sz_prev)?;
            }
            nsec = 1;
            start = k;
            size = d.o[0x3a8 + ku] as u32;
            units = r12.wrapping_add(size).wrapping_sub(1) / r12;
        }
        newg = false;
        if last {
            emit(d, start, k, size)?;
            newg = true;
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn run_checked(d: &mut Dcc) -> R<()> {
    run(d)?;
    if d.spill.is_empty() {
        Ok(())
    } else {
        Err(Stop::Undef(super::run::OVERRUN))
    }
}

pub(crate) fn run(d: &mut Dcc) -> R<()> {
    d.spill.clear();
    d.o[0x418..0x420].fill(0);
    d.p(0xb0, 0);
    reorder_1dd10(d, 2, 0, 1);
    channel(d)?;
    signfix(d)?;
    reorder_1dd10(d, 2, 2, 2);
    color(d)?;
    reorder_1dd10(d, 2, 2, 3);
    delta(d)?;
    transform(d);
    stats(d);
    tid(d)?;
    plan(d)?;
    pack(d)
}
