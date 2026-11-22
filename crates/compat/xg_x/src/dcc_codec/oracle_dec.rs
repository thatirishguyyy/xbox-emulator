use super::addr::reorder_1dd10;
use super::bits::key_overflows;
use super::run::{self, fresh, Scratch, FILLS, UNSET};
use super::tests::{console, ours, same, Con, Rng, Tm};
use super::{dec, enc, Dcc, Stop, R};
use core::ffi::c_void;

pub(super) type St = extern "system" fn(*mut c_void);
pub(super) type Ro = extern "system" fn(*mut c_void, *mut u8, *const u8, u32);
pub(super) type DecF = unsafe extern "system" fn(u32, *const u8, *const u8, *mut u8, i32, i32, *const u32) -> i32;
pub(super) type EncF = unsafe extern "system" fn(u32, *const u8, *mut u8, *mut u8, *mut u32, i32, i32, *const u32) -> i32;


pub(super) fn block(r: &mut Rng) -> [u8; 256] {
    let kind = r.next();
    let mut b = [0u8; 256];
    match kind % 7 {
        0 => b.iter_mut().for_each(|x| *x = r.next() as u8),
        1 => b.fill(r.next() as u8),
        2 => {
            let per = [1usize, 2, 4, 8, 16][(r.next() % 5) as usize];
            let e: Vec<u8> = (0..per).map(|_| r.next() as u8).collect();
            for (i, x) in b.iter_mut().enumerate() {
                *x = e[i % per];
            }
        }
        3 => {
            let (base, step) = (r.next() as u8, (r.next() % 5) as u8);
            for (i, x) in b.iter_mut().enumerate() {
                *x = base.wrapping_add(step.wrapping_mul((i / [1usize, 2, 4][(r.next() % 3) as usize].max(1)) as u8));
            }
        }
        4 => {
            for _ in 0..(r.next() % 12) {
                b[(r.next() % 256) as usize] = r.next() as u8;
            }
        }
        5 => {
            let (a, c) = (r.next() as u8, r.next() as u8);
            b.iter_mut().for_each(|x| *x = if r.next() % 4 == 0 { c } else { a });
        }
        _ => {
            let base = r.next() as u8;
            b.iter_mut().for_each(|x| *x = base.wrapping_add((r.next() % 3) as u8));
        }
    }
    b
}

pub(super) fn params(r: &mut Rng) -> [u32; 2] {
    let p0 = match r.next() % 6 {
        0..=2 => r.next() % 3,
        3 => r.next() % 8,
        4 => 28 + r.next() % 8,
        _ => r.next(),
    };
    let p1 = match r.next() % 6 {
        0 | 1 => 0,
        2 => 1,
        3 => 2,
        4 => r.next() % 6,
        _ => r.next(),
    };
    [p0, p1]
}

pub(super) fn known(r: &mut Rng) -> Scratch {
    let mut s = Scratch { b: [0; 64], unknown: 0 };
    s.b.iter_mut().for_each(|x| *x = r.next() as u8);
    s
}

pub(super) fn seeded(f: i32, t: i32, p: [u32; 2], r: &mut Rng) -> Dcc {
    let mut d = ours(f, t, p).0;
    for (a, b) in UNSET {
        for i in a..b {
            d.o[i] = r.next() as u8;
        }
    }
    for s in 0..3 {
        d.s[s].iter_mut().for_each(|x| *x = r.next() as u8);
    }
    d.sc.iter_mut().for_each(|x| *x = r.next() as u8);
    d
}

pub(super) fn clear_err(d: &mut Dcc, c: &Con) {
    d.o[0x418..0x420].fill(0);
    c.put(0x418, 0);
    c.put(0x41c, 0);
}

pub(super) fn pull(c: &Con, d: &Dcc) -> Dcc {
    let mut n = d.clone();
    let img = c.image();
    for off in (0..0x40).chain(0xb0..0xb4).chain(0xf0..0x420) {
        n.o[off] = img[off];
    }
    for i in 0..3 {
        n.s[i] = c.get_surf(i);
    }
    unsafe { core::ptr::copy_nonoverlapping(c.or.sc() as *const u8, n.sc.as_mut_ptr(), 64) };
    n
}

#[derive(Default)]
pub(super) struct Tally {
    pub(super) ok: usize,
    pub(super) fault: usize,
    pub(super) undef: usize,
}

impl Tally {
    pub(super) fn stop(&mut self, s: Stop) {
        match s {
            Stop::Fault(_) => self.fault += 1,
            Stop::Undef(_) => self.undef += 1,
        }
    }
}

pub(super) fn stage(c: &Con, d: &mut Dcc, rva: usize, f: fn(&mut Dcc) -> R<()>, what: &str, t: &mut Tally) -> bool {
    clear_err(d, c);
    c.load(d);
    let mut n = d.clone();
    n.spill.clear();
    if let Err(e) = f(&mut n) {
        t.stop(e);
        return false;
    }
    if !n.spill.is_empty() {
        t.stop(super::Stop::Undef(super::run::OVERRUN));
        return false;
    }
    let g: St = unsafe { core::mem::transmute(c.f(rva)) };
    g(c.ptr());
    same(&n, c, what);
    t.ok += 1;
    *d = n;
    true
}

pub(super) fn reorder_stage(c: &Con, d: &mut Dcc, dst: usize, src: usize, ord: u32, what: &str, t: &mut Tally) {
    clear_err(d, c);
    c.load(d);
    let g: Ro = unsafe { core::mem::transmute(c.f(0x1dd10)) };
    g(c.ptr(), c.surf(dst), c.surf(src), ord);
    reorder_1dd10(d, dst, src, ord);
    same(d, c, what);
    t.ok += 1;
}

pub(super) fn cases(or: &Tm) -> Vec<(i32, i32, [u32; 2])> {
    let mut v = super::tests::configs();
    let mut seen = std::collections::HashSet::new();
    v.retain(|&(f, t, p)| {
        let (c, _) = Con::new(or, f, t, p);
        let img = c.image();
        seen.insert((0..0x40).map(|i| img[i]).collect::<Vec<u8>>())
    });
    v
}

fn dec_stage_chain(c: &Con, d: &mut Dcc, label: &str, t: &mut Tally) {
    let st: [(usize, fn(&mut Dcc) -> R<()>, &str); 4] = [(0x16be0, dec::unpack, "unpack 16be0"), (0x17430, dec::tid, "tid 17430"), (0x17680, |d| Ok(dec::dedelta(d)), "dedelta 17680"), (0x178c0, dec::calc, "calc 178c0")];
    for (rva, f, n) in st {
        if !stage(c, d, rva, f, &format!("{label} {n}"), t) {
            return;
        }
    }
    reorder_stage(c, d, 2, 2, 2, &format!("{label} reorder 2"), t);
    if !stage(c, d, 0x17d90, dec::color, &format!("{label} color 17d90"), t) {
        return;
    }
    reorder_stage(c, d, 2, 2, 1, &format!("{label} reorder 1"), t);
    if !stage(c, d, 0x19570, dec::signfix, &format!("{label} signfix 19570"), t) {
        return;
    }
    stage(c, d, 0x180c0, dec::channel, &format!("{label} channel 180c0"), t);
}

pub(super) fn native_encoded(f: i32, t: i32, p: [u32; 2], src: &[u8; 256]) -> Option<([u8; 256], u8)> {
    let mut d = fresh(0, &Scratch { b: [0; 64], unknown: 0 });
    if d.setup(f, t, p[0], p[1]) != 0 {
        return None;
    }
    d.s[0] = *src;
    enc::run_checked(&mut d).ok()?;
    Some((d.s[1], d.o[0x34]))
}

pub(super) fn dec_input(r: &mut Rng, f: i32, t: i32, p: [u32; 2]) -> ([u8; 256], u8) {
    if r.next() % 2 == 0 {
        let b = block(r);
        if let Some(e) = native_encoded(f, t, p, &b) {
            if r.next() % 5 != 0 {
                return e;
            }
            return (e.0, r.next() as u8);
        }
    }
    let mut m = r.next() as u8;
    while key_overflows(m as u32) {
        m = r.next() as u8;
    }
    (block(r), m)
}

#[test]
fn oracle_xgx_dcc_dec_stages() {
    let Some((_orc, ref or)) = console() else { return };
    let mut r = Rng(0x5eed_dec0_1234);
    let mut t = Tally::default();
    let cfg = cases(or);
    let mut blocks = 0;
    for &(f, tm, _) in &cfg {
        for _ in 0..24 {
            let p = params(&mut r);
            let (c, _) = Con::new(or, f, tm, p);
            let mut d = seeded(f, tm, p, &mut r);
            let (s1, m) = dec_input(&mut r, f, tm, p);
            d.s[1] = s1;
            d.p(0x34, m as u32);
            dec_stage_chain(&c, &mut d, &format!("fmt {f} tile {tm} params {p:x?} meta {m:#04x}"), &mut t);
            blocks += 1;
        }
    }
    println!("xg_x dcc dec stages: {} configs, {} blocks, {} stage calls matched the console byte-for-byte, {} stopped at a console fault, {} at undefined console state", cfg.len(), blocks, t.ok, t.fault, t.undef);
    assert!(t.ok > 10_000);
}

pub(super) fn prefill(c: &Con, d: &Dcc) {
    c.load(d);
    let tm = c.or;
    unsafe { core::ptr::copy_nonoverlapping(d.sc.as_ptr(), tm.sc() as *mut u8, 64) };
}

#[test]
fn oracle_xgx_dcc_dec_e2e() {
    let Some((_orc, ref or)) = console() else { return };
    let drv: St = unsafe { core::mem::transmute(or.rva(0x169d0)) };
    let mut r = Rng(0xe2e_dec_77);
    let cfg = cases(or);
    let (mut ok, mut stopped, mut msgs) = (0usize, 0usize, 0usize);
    for &(f, tm, _) in &cfg {
        for _ in 0..12 {
            let p = params(&mut r);
            let (c, _) = Con::new(or, f, tm, p);
            let mut d = seeded(f, tm, p, &mut r);
            prefill(&c, &d);
            for blk in 0..(1 + r.next() % 4) {
                let (s1, m) = dec_input(&mut r, f, tm, p);
                d.s[1] = s1;
                d.p(0x34, m as u32);
                c.set_surf(1, &s1);
                c.put(0x34, m as u32);
                let mut n = d.clone();
                if let Err(_) = dec::run(&mut n) {
                    stopped += 1;
                    break;
                }
                drv(c.ptr());
                same(&n, &c, &format!("e2e fmt {f} tile {tm} params {p:x?} block {blk} meta {m:#04x}"));
                msgs += n.msg.len();
                d = n;
                ok += 1;
            }
        }
    }
    println!("xg_x dcc dec e2e: {} blocks through console driver 0x169d0 matched native object, surfaces and scratch; {} chains stopped at a console fault or undefined state; {} codec error messages reproduced", ok, stopped, msgs);
    assert!(ok > 3000);
}

pub(super) fn set_scratch(tm: &Tm, s: &Scratch) {
    unsafe { core::ptr::copy_nonoverlapping(s.b.as_ptr(), tm.sc() as *mut u8, 64) };
    *run::SCRATCH.lock().unwrap() = Scratch { b: s.b, unknown: s.unknown };
}

pub(super) fn shim_format(r: &mut Rng, cfg: &[(i32, i32, [u32; 2])]) -> (i32, i32) {
    match r.next() % 10 {
        0 => ((r.next() % 0x7a) as i32, (r.next() % 0x21) as i32 - 1),
        _ => {
            let (f, t, _) = cfg[(r.next() as usize) % cfg.len()];
            (f, t)
        }
    }
}

#[test]
fn oracle_xgx_dcc_decode_export_shim() {
    let Some((_orc, ref or)) = console() else { return };
    let cdec: DecF = unsafe { core::mem::transmute(or.f("XGDecodeDCC")) };
    let mut r = Rng(0x5a1d_dec0_9e11);
    let cfg = super::tests::configs();
    let mut skips: std::collections::BTreeMap<String, usize> = Default::default();
    let (mut cases_n, mut blocks, mut same_hr, mut rejected, mut setup_rej) = (0usize, 0usize, 0usize, 0usize, 0usize);
    for _ in 0..6000 {
        let (f, tm) = shim_format(&mut r, &cfg);
        let p = params(&mut r);
        let n = 1 + (r.next() % 6) as usize;
        let mut src = vec![0u8; n * 256];
        let mut meta = vec![0u8; n];
        for i in 0..n {
            let (b, m) = dec_input(&mut r, f.clamp(1, 0x78), tm.clamp(0, 0x1a), p);
            src[i * 256..i * 256 + 256].copy_from_slice(&b);
            meta[i] = m;
        }
        let sc = known(&mut r);
        set_scratch(or, &sc);
        let mut nd = vec![0x3cu8; n * 256];
        let nh = unsafe { crate::dcc::XGDecodeDCC(n as u32, src.as_ptr(), meta.as_ptr(), nd.as_mut_ptr(), f, tm, p.as_ptr() as *const crate::dcc::DccParams) };
        cases_n += 1;
        if nh != 0 && f > 0 && f <= 0x78 && ours(f, tm, p).1 == 0 {
            rejected += 1;
            assert_eq!(nh, super::E_INVALIDARG);
            let mut runs = run::setup_runs(f, tm, p, &sc).unwrap();
            let (mut scratch_out, mut notes) = (vec![0u8; n * 256], Vec::new());
            match run::decode_blocks(&mut runs, &src, &meta, &mut scratch_out, &mut notes) {
                run::Outcome::Stopped { why, .. } => *skips.entry(why).or_default() += 1,
                run::Outcome::Done => panic!("native rejected fmt {f} tile {tm} without a codec stop"),
            }
            continue;
        }
        let mut cd = vec![0x3cu8; n * 256];
        let ch = unsafe { cdec(n as u32, src.as_ptr(), meta.as_ptr(), cd.as_mut_ptr(), f, tm, p.as_ptr()) };
        assert_eq!(nh, ch, "hr fmt {f} tile {tm} params {p:x?} n {n}");
        if nh != 0 {
            setup_rej += 1;
        }
        assert_eq!(nd, cd, "bytes fmt {f} tile {tm} params {p:x?} metas {meta:02x?}");
        same_hr += 1;
        blocks += n;
    }
    println!("xg_x dcc decode export shim: {} cases, {} compared with console XGDecodeDCC ({} decoded blocks, {} argument/setup rejections with equal HRESULT), 0 mismatches; {} native E_INVALIDARG for console faults or undefined output (console not called)", cases_n, same_hr, blocks, setup_rej, rejected);
    for (w, k) in &skips {
        println!("xg_x dcc decode export shim skip: {k} cases: {w}");
        assert!(PROVEN.iter().any(|p| w.contains(p)), "unproven skip reason: {w}");
    }
    assert!(same_hr > 4000);
}

pub(super) const PROVEN: [&str; 5] = ["bytes per element / 4", "/GS cookie", "never wrote", "overruns the console's 256-byte heap output surface", "after its stack arguments were overwritten"];

pub(super) fn child_exit(test: &str, var: &str, i: usize) -> u32 {
    let exe = std::env::current_exe().unwrap();
    let st = std::process::Command::new(&exe)
        .args([test, "--exact", "--test-threads=1", "--nocapture"])
        .env(var, i.to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    st.code().unwrap_or(0) as u32
}

pub(super) fn fault_cases_dec(or: &Tm) -> Vec<(i32, i32, [u32; 2], [u8; 256], u8, &'static str)> {
    let mut v = Vec::new();
    let mut r = Rng(0xfa17);
    let mut gs = 0;
    let mut de = 0;
    for &(f, t, _) in &cases(or) {
        for _ in 0..40 {
            let p = params(&mut r);
            let (s1, m) = if r.next() % 3 == 0 {
                let mut m = r.next() as u8;
                while !key_overflows(m as u32) {
                    m = r.next() as u8;
                }
                (block(&mut r), m)
            } else {
                dec_input(&mut r, f, t, p)
            };
            let mut d = fresh(0, &Scratch { b: [0; 64], unknown: 0 });
            d.setup(f, t, p[0], p[1]);
            d.s[1] = s1;
            d.p(0x34, m as u32);
            if let Err(Stop::Fault(w)) = dec::run(&mut d) {
                let is_gs = w.contains("/GS");
                if (is_gs && gs < 6) || (!is_gs && de < 10 && !v.iter().any(|c: &(i32, i32, [u32; 2], [u8; 256], u8, &str)| c.5 == w && c.0 == f)) {
                    if is_gs {
                        gs += 1;
                    } else {
                        de += 1;
                    }
                    v.push((f, t, p, s1, m, w));
                }
            }
        }
    }
    v
}

#[test]
fn oracle_xgx_dcc_dec_undefined_classes() {
    if let Ok(i) = std::env::var("XGX_DCC_DEC_CHILD") {
        let Some((_orc, ref or)) = console() else { std::process::exit(3) };
        let cdec: DecF = unsafe { core::mem::transmute(or.f("XGDecodeDCC")) };
        let (f, t, p, s1, m, _) = fault_cases_dec(or)[i.parse::<usize>().unwrap()];
        let mut out = [0u8; 256];
        unsafe { cdec(1, s1.as_ptr(), [m].as_ptr(), out.as_mut_ptr(), f, t, p.as_ptr()) };
        std::process::exit(0);
    }
    let Some((_orc, ref or)) = console() else { return };
    let fc = fault_cases_dec(or);
    let mut by: std::collections::BTreeMap<(&str, u32), usize> = Default::default();
    for (i, c) in fc.iter().enumerate() {
        let code = child_exit("dcc_codec::oracle_dec::oracle_xgx_dcc_dec_undefined_classes", "XGX_DCC_DEC_CHILD", i);
        let want = if c.5.contains("/GS") { 0xc000_0409 } else { 0xc000_0094 };
        assert_eq!(code, want, "fault case {i} fmt {} tile {} params {:x?} meta {:#04x}: {}", c.0, c.1, c.2, c.4, c.5);
        *by.entry((c.5, code)).or_default() += 1;
    }
    for ((w, code), n) in &by {
        println!("xg_x dcc dec fault class: {n} child processes running console XGDecodeDCC exited {code:#x}: {w}");
    }
    assert!(fc.iter().any(|c| c.5.contains("/GS")) && fc.iter().any(|c| !c.5.contains("/GS")));
    let drv: St = unsafe { core::mem::transmute(or.rva(0x169d0)) };
    let mut r = Rng(0x0dd_f111);
    let (mut undef, mut varied) = (0usize, 0usize);
    for &(f, t, _) in &cases(or) {
        for _ in 0..30 {
            let p = params(&mut r);
            let (s1, m) = dec_input(&mut r, f, t, p);
            let sc = known(&mut r);
            let mut runs = match run::setup_runs(f, t, p, &sc) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let mut dst = [0u8; 256];
            let mut notes = Vec::new();
            match run::decode_blocks(&mut runs, &s1, &[m], &mut dst, &mut notes) {
                run::Outcome::Stopped { why, .. } if why.contains("never wrote") => {}
                _ => continue,
            }
            undef += 1;
            let mut outs = Vec::new();
            for fl in 0..FILLS {
                let (c, _) = Con::new(or, f, t, p);
                let mut d = fresh(fl, &sc);
                d.setup(f, t, p[0], p[1]);
                prefill(&c, &d);
                c.set_surf(1, &s1);
                c.put(0x34, m as u32);
                drv(c.ptr());
                outs.push(pull(&c, &d).s[0]);
            }
            if outs.iter().any(|o| *o != outs[0]) {
                varied += 1;
            }
        }
    }
    println!("xg_x dcc dec undefined output: {undef} single-block cases where native sees output bytes the console never writes; console driver 0x169d0 output changed with the surface/object prefill in {varied} of them");
    assert_eq!(undef, varied);
}
