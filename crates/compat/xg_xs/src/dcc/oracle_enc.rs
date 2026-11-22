use super::oracle_dec::{copy, defined, patterns, smear, states, DecF, Enc, Lay, Tally, P1S};
use super::{dec, enc, px, Dcc};
use crate::dcc_oracle::{configs, diff, holes, Con, Rng};
use crate::layout::E_INVALIDARG;
use crate::oracle::{oracle, Oracle, SERIAL};

#[derive(Clone, Copy)]
pub(super) enum St {
    Lay(usize, u32),
    C1(usize, fn(&mut Dcc) -> Result<(), i32>),
    Sub,
}

fn stages() -> Vec<(&'static str, St)> {
    vec![
        ("relayout b0->b2 mode1 0x4ac90", St::Lay(0, 1)),
        ("clear detect 0x26680", St::C1(0x26680, enc::clear_detect)),
        ("channel pack 0x27330", St::C1(0x27330, enc::channel)),
        ("signfix 0x27d10", St::C1(0x27d10, dec::signfix)),
        ("relayout mode2 0x4ac90", St::Lay(2, 2)),
        ("color fwd 0x27e50", St::C1(0x27e50, enc::color)),
        ("relayout mode3 0x4ac90", St::Lay(2, 3)),
        ("delta 0x28720", St::C1(0x28720, enc::delta)),
        ("vrs 0x28df0", St::C1(0x28df0, enc::vrs)),
        ("choose 0x292c0", St::C1(0x292c0, enc::choose)),
        ("far 0x29e40", St::C1(0x29e40, enc::far)),
        ("far base 0x29740 x4 + save/restore", St::Sub),
        ("plan 0x2a4e0", St::C1(0x2a4e0, enc::plan)),
        ("pack 0x2a8e0", St::C1(0x2a8e0, enc::pack)),
    ]
}

pub(super) fn console_stage(o: &Oracle, c: &Con, s: &Dcc, st: St) -> Dcc {
    c.push(s);
    match st {
        St::Lay(src, mode) => {
            let lay: Lay = unsafe { core::mem::transmute(o.h + 0x4ac90) };
            let dst = c.buf(2).as_mut_ptr();
            let sp = c.buf(src).as_ptr();
            unsafe { lay(c.p(), dst, sp, mode) };
            c.pull()
        }
        St::C1(rva, _) => {
            c.call1(rva);
            c.pull()
        }
        St::Sub => {
            for k in 0..4 {
                c.call2(0x29740, k);
            }
            let mut p = c.pull();
            let b = p.b[2];
            p.m[0x1180..0x1280].copy_from_slice(&b);
            let w: Vec<u8> = p.m[0x34c..0x38c].to_vec();
            p.m[0x1280..0x12c0].copy_from_slice(&w);
            p.b[2] = s.b[2];
            p.m[0x34c..0x38c].copy_from_slice(&s.m[0x34c..0x38c]);
            p
        }
    }
}

fn native_stage(s: &Dcc, st: St) -> Result<Dcc, i32> {
    let mut n = copy(s);
    match st {
        St::Lay(src, mode) => enc::relay(&mut n, 2, src, mode)?,
        St::C1(_, f) => f(&mut n)?,
        St::Sub => enc::subbed(&mut n)?,
    }
    Ok(n)
}

fn inputs(r: &mut Rng, bpe: usize) -> Vec<[u8; 256]> {
    let mut v = patterns(r, bpe);
    for _ in 0..3 {
        let mut b = [0u8; 256];
        let base = r.u() as u8;
        for (i, x) in b.iter_mut().enumerate() {
            *x = base.wrapping_add((r.below(5) as u8).wrapping_sub(2)).wrapping_add((i / 64) as u8 * 17);
        }
        v.push(b);
    }
    v
}

#[test]
fn oracle_dcc_enc_stages() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let Some(o) = oracle() else { return };
    let hl = holes(&o);
    let st = states();
    let covered: usize = st.iter().map(|x| x.1).sum();
    let mut r = Rng(0x5eed_e4c0_9876);
    let mut t = Tally::new();
    let (mut blocks, mut aborted, mut faults) = (0u64, 0u64, 0u64);
    let mut driver_bad = 0u64;
    for &((f, sw, dim, p0, p1), _) in &st {
        let Some(c) = Con::new(&o, f, sw, dim, p0, p1) else { continue };
        let base = defined(c.pull());
        let bpe = base.r32(4);
        let mut s = copy(&base);
        for inp in inputs(&mut r, bpe as usize) {
            blocks += 1;
            let ctx = format!("f={} sw={} dim={} p0={} p1={:#x}", f, sw, dim, p0, p1);
            s.b[0] = inp;
            s.w64(0x13c0, 0);
            s.w32(0xf0, 0);
            let start = copy(&s);
            let mut ok = true;
            for (name, stg) in stages() {
                if let St::C1(0x27e50, _) = stg {
                    if enc::color_faults(&s) {
                        faults += 1;
                        assert_eq!(enc::color(&mut copy(&s)), Err(E_INVALIDARG));
                        ok = false;
                        break;
                    }
                }
                let n = match native_stage(&s, stg) {
                    Ok(n) => n,
                    Err(_) => {
                        t.err(name);
                        ok = false;
                        break;
                    }
                };
                let p = console_stage(&o, &c, &s, stg);
                t.hit(name, diff(&n, &p, &hl, true), &ctx);
                s = p;
            }
            if !ok {
                aborted += 1;
                s = copy(&base);
                continue;
            }
            let mut n = copy(&start);
            enc::run(&mut n).expect("native driver failed after all native stages succeeded");
            c.push(&start);
            c.call1(0x26220);
            let p = c.pull();
            let m = diff(&n, &p, &hl, true);
            if m.is_some() {
                driver_bad += 1;
            }
            t.hit("driver 0x26220", m, &ctx);
            s = p;
        }
    }
    t.print("dcc enc stage");
    println!("dcc enc stages: {} unique states covering {} configs, {} blocks, {} aborted ({} color #DE predicted), driver mismatches {}", st.len(), covered, blocks, aborted, faults, driver_bad);
    assert_eq!(t.total_bad(), 0);
}

pub(super) fn console_encode_at(o: &Oracle, src: &[u8], count: usize, f: u32, sw: u32, dim: u32, p0: u32, p1: u32, fill: u8, sm: u8, shift: usize) -> (i32, Vec<u8>, Vec<u8>, Vec<u32>) {
    let enc: Enc = unsafe { core::mem::transmute(o.f("XGEncodeDCC")) };
    let prm = [p0, p1, 0, 0];
    let mut sbuf = vec![0u8; src.len() + shift + 64];
    sbuf[shift..shift + src.len()].copy_from_slice(src);
    let mut out = vec![fill; 256 * count + shift + 64];
    let mut meta = vec![fill; count + shift + 64];
    let mut sz = vec![u32::from_ne_bytes([fill; 4]); count + shift / 4 + 16];
    let _ = smear(sm);
    let r = unsafe { enc(count as u32, sbuf.as_ptr().add(shift), out.as_mut_ptr().add(shift), meta.as_mut_ptr().add(shift), sz.as_mut_ptr().add(shift / 4), f, sw, dim, prm.as_ptr()) };
    (r, out[shift..shift + 256 * count].to_vec(), meta[shift..shift + count].to_vec(), sz[shift / 4..shift / 4 + count].to_vec())
}

pub(super) fn console_encode(o: &Oracle, src: &[u8], count: usize, f: u32, sw: u32, dim: u32, p0: u32, p1: u32, fill: u8, sm: u8) -> (i32, Vec<u8>, Vec<u8>, Vec<u32>) {
    let enc: Enc = unsafe { core::mem::transmute(o.f("XGEncodeDCC")) };
    let prm = [p0, p1, 0, 0];
    let mut out = vec![fill; 256 * count];
    let mut meta = vec![fill; count];
    let mut sz = vec![u32::from_ne_bytes([fill; 4]); count];
    let _ = smear(sm);
    let r = unsafe { enc(count as u32, src.as_ptr(), out.as_mut_ptr(), meta.as_mut_ptr(), sz.as_mut_ptr(), f, sw, dim, prm.as_ptr()) };
    (r, out, meta, sz)
}

#[test]
fn oracle_dcc_enc_e2e() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let Some(o) = oracle() else { return };
    let cdec: DecF = unsafe { core::mem::transmute(o.f("XGDecodeDCC")) };
    let st = states();
    let mut r = Rng(0xe2e_e4c_31);
    let (mut calls, mut blocks, mut bad, mut undef, mut nerr, mut faults, mut rt, mut rtbad, mut lossy) = (0u64, 0u64, 0u64, 0u64, 0u64, 0u64, 0u64, 0u64, 0u64);
    let mut extra: Vec<(u32, u32, u32, u32, u32)> = configs();
    extra.retain(|_| r.below(30) == 0);
    let mut cfgs: Vec<(u32, u32, u32, u32, u32)> = st.iter().map(|x| x.0).collect();
    for (f, sw, dim, p0, _) in extra {
        cfgs.push((f, sw, dim, p0, P1S[r.below(5) as usize]));
    }
    for (f, sw, dim, p0, p1) in cfgs {
        let mut d = Dcc::new();
        if d.init(f, sw, dim, p0, p1).is_err() {
            continue;
        }
        let bpe = d.r32(4);
        let pats = inputs(&mut r, bpe as usize);
        for count in [1usize, 3, 8] {
            let mut src = vec![0u8; 256 * count];
            for i in 0..count {
                let p = pats[r.below(pats.len() as u32) as usize];
                src[i * 256..i * 256 + 256].copy_from_slice(&p);
            }
            let mut nd = vec![0u8; 256 * count];
            let mut nm = vec![0u8; count];
            let mut ns = vec![0u32; count];
            let nr = crate::dcc::encode(count as u32, &src, &mut nd, &mut nm, Some(&mut ns), f, sw, dim, p0, p1);
            if nr != 0 {
                assert_eq!(nr, E_INVALIDARG);
                if enc::color_faults(&d) {
                    faults += 1;
                } else {
                    nerr += 1;
                    if nerr <= 5 {
                        println!("e2e native E_INVALIDARG f={} sw={} dim={} p0={} p1={:#x} count={}", f, sw, dim, p0, p1, count);
                    }
                }
                continue;
            }
            let a = console_encode(&o, &src, count, f, sw, dim, p0, p1, 0xaa, 0xa5);
            let b = console_encode(&o, &src, count, f, sw, dim, p0, p1, 0x55, 0x5a);
            calls += 2;
            blocks += count as u64;
            if a != b {
                undef += 1;
                if undef <= 5 {
                    let j = (0..a.1.len()).find(|&j| a.1[j] != b.1[j]);
                    println!("e2e console output depends on prior memory f={} sw={} dim={} p0={} p1={:#x} count={} first dst diff {:?} meta {:02x?}/{:02x?}", f, sw, dim, p0, p1, count, j, a.2, b.2);
                }
            }
            if a.0 != 0 || a.1 != nd || a.2 != nm || a.3 != ns {
                bad += 1;
                if bad <= 10 {
                    let j = (0..nd.len()).find(|&j| a.1[j] != nd[j]);
                    println!("e2e f={} sw={} dim={} p0={} p1={:#x} count={}: hr={:#x} meta n{:02x?} c{:02x?} sizes n{:?} c{:?} first dst diff {:?}", f, sw, dim, p0, p1, count, a.0, nm, a.2, ns, a.3, j.map(|j| (j, nd[j], a.1[j])));
                }
            }
            let prm = [p0, p1, 0, 0];
            let mut cd = vec![0u8; 256 * count];
            let cr = unsafe { cdec(count as u32, nd.as_ptr(), nm.as_ptr(), cd.as_mut_ptr(), f, sw, dim, prm.as_ptr()) };
            let mut dd = vec![0u8; 256 * count];
            let dr = crate::dcc::decode(count as u32, &nd, &nm, &mut dd, f, sw, dim, p0, p1);
            rt += 1;
            if dr == 0 && (cr != 0 || cd != dd) {
                rtbad += 1;
                if rtbad <= 5 {
                    println!("round trip f={} sw={} dim={} p0={} p1={:#x} count={}: native decode differs from console decode", f, sw, dim, p0, p1, count);
                }
            }
            if cr == 0 && cd != src {
                lossy += 1;
            }
        }
    }
    println!("dcc enc e2e: {} XGEncodeDCC calls, {} blocks, {} mismatches, {} console results depending on prior memory, {} native E_INVALIDARG (non-fault), {} color #DE predicted, {} round trips, {} native/console decode mismatches, {} lossy round trips", calls, blocks, bad, undef, nerr, faults, rt, rtbad, lossy);
    assert_eq!(bad, 0);
    assert_eq!(undef, 0);
    assert_eq!(rtbad, 0);
}

pub(super) fn reject_cause(d0: &Dcc, inp: &[u8; 256]) -> &'static str {
    let mut d = copy(d0);
    d.b[0] = *inp;
    d.w64(0x13c0, 0);
    d.w32(0xf0, 0);
    let mut t = enc::Tmp::new();
    if enc::relay_in(&mut d, 2, 0, 1, &mut t).is_err() {
        return "relayout mode0->1 hole";
    }
    if enc::clear_detect(&mut d).is_err() {
        return "clear detect body";
    }
    let _ = enc::channel(&mut d);
    let _ = dec::signfix(&mut d);
    if enc::relay_in(&mut d, 2, 2, 2, &mut t).is_err() {
        return "relayout mode1->2 hole";
    }
    if enc::color_faults(&d) {
        return "color #DE";
    }
    let _ = enc::color_in(&mut d, &mut t);
    if enc::relay_in(&mut d, 2, 2, 3, &mut t).is_err() {
        return "relayout mode2->3 hole";
    }
    let steps: [(&'static str, fn(&mut Dcc) -> Result<(), i32>); 7] = [("delta", enc::delta), ("vrs convert 0x4adc0 hole", enc::vrs), ("choose", enc::choose), ("far", enc::far), ("far base", enc::subbed), ("plan", enc::plan), ("pack", enc::pack)];
    for (n, f) in steps {
        if f(&mut d).is_err() {
            return n;
        }
    }
    "none"
}

#[inline(never)]
fn drive_plain(c: &Con) -> Dcc {
    c.call1(0x26220);
    c.pull()
}

#[inline(never)]
fn drive_deep(c: &Con) -> Dcc {
    let mut pad = [0u8; 0x2340];
    for x in pad.iter_mut() {
        unsafe { core::ptr::write_volatile(x, 0x77) };
    }
    c.call1(0x26220);
    let p = c.pull();
    unsafe { core::ptr::read_volatile(&pad[0x1233]) };
    p
}

#[inline(never)]
fn drive_regs(c: &Con, v: u64) -> Dcc {
    let f = c.h + 0x26220;
    let obj = c.p();
    unsafe {
        core::arch::asm!(
            "push rbx", "push rsi", "push rdi", "push r12", "push r13", "push r14", "push r15", "push rbp",
            "mov rbp, rsp", "and rsp, -16", "sub rsp, 32",
            "mov rbx, rdx", "mov rsi, rdx", "mov rdi, rdx", "mov r12, rdx", "mov r13, rdx", "mov r14, rdx", "mov r15, rdx",
            "call rax",
            "mov rsp, rbp",
            "pop rbp", "pop r15", "pop r14", "pop r13", "pop r12", "pop rdi", "pop rsi", "pop rbx",
            inout("rax") f => _, inout("rcx") obj => _, inout("rdx") v => _, clobber_abi("system"),
        );
    }
    c.pull()
}

fn caller_outputs(c: &Con, s: &Dcc) -> Vec<(Vec<u8>, u8)> {
    let mut v = Vec::new();
    for k in 0..4 {
        c.push(s);
        let _ = smear(0x77);
        let p = match k {
            0 => drive_plain(c),
            1 => drive_deep(c),
            2 => drive_regs(c, 0x1111_2222_3333_4444),
            _ => drive_regs(c, 0x0000_0000_0000_0003),
        };
        v.push((p.b[1].to_vec(), p.r8(0x58)));
    }
    v
}

#[test]
fn oracle_dcc_enc_undefined_classes() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let Some(o) = oracle() else { return };
    let st = states();
    let mut r = Rng(0x0dd_e4c_5eed);
    let mut seen: std::collections::BTreeMap<&'static str, (u64, u64)> = std::collections::BTreeMap::new();
    let mut same = Vec::new();
    let mut callers: std::collections::BTreeMap<(&'static str, Option<usize>), u64> = std::collections::BTreeMap::new();
    let mut grp: std::collections::BTreeMap<String, (u64, u64, String)> = std::collections::BTreeMap::new();
    for &((f, sw, dim, p0, p1), _) in &st {
        let mut d = Dcc::new();
        if d.init(f, sw, dim, p0, p1).is_err() {
            continue;
        }
        let bpe = d.r32(4);
        for inp in inputs(&mut r, bpe as usize).into_iter().take(3) {
            let mut nd = [0u8; 256];
            let mut nm = [0u8; 1];
            let nr = crate::dcc::encode(1, &inp, &mut nd, &mut nm, None, f, sw, dim, p0, p1);
            if nr == 0 {
                continue;
            }
            let cause = reject_cause(&d, &inp);
            if cause == "color #DE" || enc::color_faults(&d) {
                continue;
            }
            let mut runs: Vec<_> = [(0xaau8, 0xa5u8), (0x55, 0x5a), (0x00, 0x00), (0xff, 0xff)].iter().map(|&(fl, sm)| console_encode(&o, &inp, 1, f, sw, dim, p0, p1, fl, sm)).collect();
            for sh in [8usize, 0x18, 0x1000 + 0x28, 0x10000 + 0x30] {
                runs.push(console_encode_at(&o, &inp, 1, f, sw, dim, p0, p1, 0xaa, 0xa5, sh));
            }
            let a = runs[0].clone();
            let mut b = runs.iter().find(|x| **x != a).cloned().unwrap_or_else(|| a.clone());
            if a == b {
                let c = Con::new(&o, f, sw, dim, p0, p1).unwrap();
                let mut s = defined(c.pull());
                s.b[0] = inp;
                let w = caller_outputs(&c, &s);
                let k = (1..w.len()).find(|&k| w[k] != w[0]);
                *callers.entry((cause, k)).or_insert(0u64) += 1;
                if let Some(k) = k {
                    b.1 = w[k].0.clone();
                    b.2 = vec![w[k].1];
                }
            }
            let e = seen.entry(cause).or_insert((0, 0));
            e.0 += 1;
            let mut hd = copy(&d);
            hd.w32(0xf0, 0);
            let hs = if cause == "relayout mode0->1 hole" { enc::relay_holes(&hd, 1) } else { Vec::new() };
            let g = grp.entry(format!("{} bpe={} s0={} dim={} s14={} v48={} holes={}", cause, d.r32(4), d.r32(0), dim, d.r32(0x14), d.r32(0x48), hs.len())).or_insert((0u64, 0u64, String::new()));
            g.0 += 1;
            if a != b {
                g.1 += 1;
                if g.2.is_empty() {
                    let j: Vec<usize> = (0..256).filter(|&j| a.1[j] != b.1[j]).take(8).collect();
                    g.2 = format!("diff dst {:?} meta {:02x?}/{:02x?} holes {:?}", j, a.2, b.2, &hs[..hs.len().min(12)]);
                }
            } else if g.2.is_empty() {
                g.2 = format!("stable meta {:02x?} holes {:?}", a.2, &hs[..hs.len().min(12)]);
            }
            if a != b {
                e.1 += 1;
            } else if same.len() < 10 {
                same.push(format!("{} f={} sw={} dim={} p0={} p1={:#x}", cause, f, sw, dim, p0, p1));
            }
        }
    }
    for (c, (n, dif)) in &seen {
        println!("dcc enc undefined class {}: {} native rejections, {} console outputs changed with stack smear, buffer address or caller frame/registers", c, n, dif);
    }
    for ((c, k), n) in &callers {
        println!("dcc enc caller-state check {}: {} smear-stable cases, first differing wrapper {:?}", c, n, k);
    }
    for (k, (n, dif, ex)) in &grp {
        println!("grp {}: {} rejections, {} changed; {}", k, n, dif, ex);
    }
    for s in &same {
        println!("dcc enc rejection with stable console output: {}", s);
    }
    assert!(seen.iter().all(|(_, &(n, d))| n == d));
}

#[test]
fn oracle_dcc_enc_relayout_holes() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let Some(o) = oracle() else { return };
    let lay: Lay = unsafe { core::mem::transmute(o.h + 0x4ac90) };
    let st = states();
    let (mut cases, mut bad, mut holed) = (0u64, 0u64, 0u64);
    for &((f, sw, dim, p0, p1), _) in &st {
        let Some(c) = Con::new(&o, f, sw, dim, p0, p1) else { continue };
        let base = c.pull();
        for (from, to, src) in [(0u32, 1u32, 0usize), (1, 2, 2), (2, 3, 2)] {
            let mut s = copy(&base);
            for j in 0..256 {
                s.b[0][j] = j as u8;
                s.b[2][j] = j as u8;
            }
            s.w32(0xf0, from);
            let mut outs = Vec::new();
            for v in [0xa5u8, 0x5a] {
                c.push(&s);
                let dst = c.buf(2).as_mut_ptr();
                let sp = c.buf(src).as_ptr();
                let _ = smear(v);
                unsafe { lay(c.p(), dst, sp, to) };
                outs.push(c.pull().b[2]);
            }
            let con: Vec<usize> = (0..256).filter(|&j| outs[0][j] != outs[1][j]).collect();
            let nat = enc::relay_holes(&s, to);
            cases += 1;
            if !nat.is_empty() {
                holed += 1;
            }
            if con != nat {
                bad += 1;
                if bad <= 8 {
                    println!("relayout holes f={} sw={} dim={} {}->{}: native {} {:?} console {} {:?}", f, sw, dim, from, to, nat.len(), &nat[..nat.len().min(10)], con.len(), &con[..con.len().min(10)]);
                }
            }
        }
    }
    println!("dcc enc relayout holes: {} cases, {} with holes, {} hole-set mismatches", cases, holed, bad);
    assert_eq!(bad, 0);
}

#[test]
fn oracle_dcc_enc_convert_holes() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let Some(o) = oracle() else { return };
    let conv: super::oracle_dec::Conv = unsafe { core::mem::transmute(o.h + 0x4adc0) };
    let (mut cases, mut bad, mut holed, mut vbad) = (0u64, 0u64, 0u64, 0u64);
    for &((f, sw, dim, p0, p1), _) in &states() {
        let Some(c) = Con::new(&o, f, sw, dim, p0, p1) else { continue };
        let base = c.pull();
        for sm in [4u32, 8, 9, 10] {
            let mut s = copy(&base);
            for j in 0..256 {
                s.b[2][j] = j as u8;
            }
            let gm = [3u32; 4];
            let smv = [sm; 4];
            let mut outs = Vec::new();
            for v in [0xa5u8, 0x5a] {
                c.push(&s);
                let dst = c.buf(2).as_mut_ptr();
                let sp = c.buf(2).as_ptr();
                let _ = smear(v);
                unsafe { conv(c.p(), dst, sp, gm.as_ptr(), smv.as_ptr()) };
                outs.push(c.pull().b[2]);
            }
            let con: Vec<usize> = (0..256).filter(|&j| outs[0][j] != outs[1][j]).collect();
            let mut n = copy(&s);
            let mut ever = [false; 64];
            let mut nat = Vec::new();
            let mut nb = s.b[2];
            for k in 0..4u32 {
                if (s.r32(0x5c) >> k) & 1 == 0 {
                    continue;
                }
                for a in k * 64..k * 64 + 64 {
                    let cc = px::get(&n, a, 3).unwrap();
                    let b = px::set(&n, &cc, sm).unwrap();
                    ever[(b & 63) as usize] = true;
                    nb[(k * 64 + (b & 63)) as usize] = s.b[2][a as usize];
                }
                for p in 0..64 {
                    if !ever[p] {
                        nat.push((k * 64) as usize + p);
                    }
                }
            }
            n.b[2] = nb;
            cases += 1;
            if !nat.is_empty() {
                holed += 1;
            }
            if con != nat {
                bad += 1;
                if bad <= 8 {
                    println!("convert holes f={} sw={} dim={} s0={} bpe={} 3->{}: native {} {:?} console {} {:?}", f, sw, dim, s.r32(0), s.r32(4), sm, nat.len(), &nat[..nat.len().min(10)], con.len(), &con[..con.len().min(10)]);
                }
            }
            if let Some(j) = (0..256).find(|&j| !nat.contains(&j) && !con.contains(&j) && nb[j] != outs[0][j]) {
                vbad += 1;
                if vbad <= 8 {
                    println!("convert value f={} sw={} dim={} s0={} bpe={} 3->{}: +{:#x} native {:#x} console {:#x}", f, sw, dim, s.r32(0), s.r32(4), sm, j, nb[j], outs[0][j]);
                }
            }
        }
    }
    println!("dcc enc convert holes: {} cases, {} with native holes, {} hole-set mismatches, {} value mismatches", cases, holed, bad, vbad);
    assert_eq!(bad + vbad, 0);
}

#[test]
fn oracle_dcc_encode_export_shim() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let Some(o) = oracle() else { return };
    let cenc: Enc = unsafe { core::mem::transmute(o.f("XGEncodeDCC")) };
    let mut r = Rng(0x5141_e4c);
    let mut cases = 0u64;
    let src = [0u8; 256 * 2];
    let prm = [0u32, 0, 0, 0];
    let (mut a, mut b) = ([0u8; 512], [0u8; 512]);
    let (mut ma, mut mb) = ([0u8; 2], [0u8; 2]);
    let (mut sa, mut sb) = ([0u32; 2], [0u32; 2]);
    let nul = core::ptr::null::<u8>();
    let nulm = core::ptr::null_mut::<u8>();
    let call = |n: bool, cnt: u32, s: *const u8, d: *mut u8, m: *mut u8, z: *mut u32, f: u32, sw: u32, dim: u32, p: *const u32| -> i32 {
        unsafe { if n { crate::XGEncodeDCC(cnt, s, d, m, z, f, sw, dim, p) } else { cenc(cnt, s, d, m, z, f, sw, dim, p) } }
    };
    for (s, d, m, pp) in [(nul, a.as_mut_ptr(), ma.as_mut_ptr(), prm.as_ptr()), (src.as_ptr(), nulm, ma.as_mut_ptr(), prm.as_ptr()), (src.as_ptr(), a.as_mut_ptr(), nulm, prm.as_ptr()), (src.as_ptr(), a.as_mut_ptr(), ma.as_mut_ptr(), core::ptr::null())] {
        cases += 1;
        let x = call(true, 1, s, d, m, sa.as_mut_ptr(), 28, 1, 0, pp);
        let y = call(false, 1, s, d, m, sb.as_mut_ptr(), 28, 1, 0, pp);
        assert_eq!(x, y);
    }
    for f in [0u32, 0x79, 0x7a, 0x100, 0x7fff_ffff] {
        for sw in [0u32, 1, 0x1f, 0x20, 0x7fff_ffff] {
            cases += 1;
            let x = call(true, 1, src.as_ptr(), a.as_mut_ptr(), ma.as_mut_ptr(), sa.as_mut_ptr(), f, sw, 0, prm.as_ptr());
            let y = call(false, 1, src.as_ptr(), b.as_mut_ptr(), mb.as_mut_ptr(), sb.as_mut_ptr(), f, sw, 0, prm.as_ptr());
            assert_eq!(x, y, "f={:#x} sw={:#x}", f, sw);
        }
    }
    let (mut asserts, mut skipped, mut nulls) = (0u64, 0u64, 0u64);
    for f in 1..=0x78u32 {
        if !crate::dcc::safe_format(f) {
            for cnt in [0u32, 1] {
                asserts += 1;
                assert_eq!(call(true, cnt, src.as_ptr(), a.as_mut_ptr(), ma.as_mut_ptr(), sa.as_mut_ptr(), f, 1, 0, prm.as_ptr()), E_INVALIDARG);
            }
            continue;
        }
        for sw in [0u32, 1, 2, 3, 0x10, 0x1f, 0xffff_fffe, 0x8000_0000] {
            for dim in [0u32, 3, 4, 9] {
                let p = [r.below(3), [0u32, 1, 3][r.below(3) as usize], 0, 0];
                for cnt in [0u32, 1, 2] {
                    let mut s = [0u8; 512];
                    let base = r.u() as u8;
                    for (i, x) in s.iter_mut().enumerate() {
                        *x = if r.below(2) == 0 { r.u() as u8 } else { base.wrapping_add((i / 64) as u8) };
                    }
                    let z = r.below(4) == 0;
                    a = [0x5a; 512];
                    b = [0x5a; 512];
                    ma = [0x5a; 2];
                    mb = [0x5a; 2];
                    sa = [0x5a5a_5a5a; 2];
                    sb = [0x5a5a_5a5a; 2];
                    let x = call(true, cnt, s.as_ptr(), a.as_mut_ptr(), ma.as_mut_ptr(), if z { core::ptr::null_mut() } else { sa.as_mut_ptr() }, f, sw, dim, p.as_ptr());
                    if x != 0 {
                        skipped += 1;
                        continue;
                    }
                    let y = call(false, cnt, s.as_ptr(), b.as_mut_ptr(), mb.as_mut_ptr(), if z { core::ptr::null_mut() } else { sb.as_mut_ptr() }, f, sw, dim, p.as_ptr());
                    cases += 1;
                    if z {
                        nulls += 1;
                    }
                    assert_eq!(x, y, "hr f={} sw={:#x} dim={} p={:?} cnt={}", f, sw, dim, p, cnt);
                    assert_eq!((a, ma, sa), (b, mb, sb), "bytes f={} sw={:#x} dim={} p={:?} cnt={}", f, sw, dim, p, cnt);
                }
            }
        }
    }
    println!("dcc encode export shim: {} cases match console (hr, dst, meta, sizes; {} with sizes=NULL), {} unsupported-format calls return E_INVALIDARG, {} native E_INVALIDARG (console not run: color #DE or stack-dependent output)", cases, nulls, asserts, skipped);
}

fn de_enc_cases(n: usize, fault: bool) -> Vec<((u32, u32, u32, u32, u32), [u8; 256])> {
    let mut r = Rng(0xde_e4c);
    let mut v = Vec::new();
    for &((f, sw, dim, p0, p1), _) in &states() {
        let mut d = Dcc::new();
        if d.init(f, sw, dim, p0, p1).is_err() {
            continue;
        }
        if d.r32(4) >= 4 || enc::color_faults(&d) != fault {
            continue;
        }
        let mut b = [0u8; 256];
        for x in b.iter_mut() {
            *x = r.u() as u8;
        }
        v.push(((f, sw, dim, p0, p1), b));
        if v.len() >= n {
            break;
        }
    }
    v
}

#[test]
fn oracle_dcc_enc_color_fault_child() {
    if let Ok(i) = std::env::var("DCC_DE_ENC_CASE") {
        let Some(o) = oracle() else { std::process::exit(3) };
        let fault = std::env::var("DCC_DE_ENC_FAULT").map_or(false, |v| v == "1");
        let ((f, sw, dim, p0, p1), b) = de_enc_cases(64, fault)[i.parse::<usize>().unwrap()];
        let r = console_encode(&o, &b, 1, f, sw, dim, p0, p1, 0, 0);
        std::process::exit(if r.0 == 0 { 0 } else { 4 });
    }
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let Some(_o) = oracle() else { return };
    let exe = std::env::current_exe().unwrap();
    let mut seen = [0usize; 2];
    for fault in [true, false] {
        let cases = de_enc_cases(16, fault);
        assert!(cases.len() >= 8);
        for (i, c) in cases.iter().enumerate() {
            let mut d = Dcc::new();
            d.init(c.0 .0, c.0 .1, c.0 .2, c.0 .3, c.0 .4).unwrap();
            if fault {
                let mut out = [0u8; 256];
                let mut m = [0u8; 1];
                assert_eq!(crate::dcc::encode(1, &c.1, &mut out, &mut m, None, c.0 .0, c.0 .1, c.0 .2, c.0 .3, c.0 .4), E_INVALIDARG);
            }
            let st = std::process::Command::new(&exe)
                .args(["dcc::oracle_enc::oracle_dcc_enc_color_fault_child", "--exact", "--test-threads=1"])
                .env("DCC_DE_ENC_CASE", i.to_string())
                .env("DCC_DE_ENC_FAULT", if fault { "1" } else { "0" })
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .unwrap();
            let code = st.code().unwrap_or(0) as u32;
            if fault {
                assert_eq!(code, 0xc000_0094, "case {} {:?} exit {:#x}", i, c.0, code);
            } else {
                assert_ne!(code, 0xc000_0094, "case {} {:?} faulted but native predicts no fault", i, c.0);
            }
            seen[fault as usize] += 1;
        }
    }
    println!("dcc enc color #DE: {} predicted cases raised STATUS_INTEGER_DIVIDE_BY_ZERO in child processes, {} bpe<4 non-fault cases ran without #DE", seen[1], seen[0]);
}

