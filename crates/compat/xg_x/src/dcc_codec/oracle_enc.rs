use super::oracle_dec::{block, cases, known, params, prefill, pull, reorder_stage, seeded, set_scratch, shim_format, stage, EncF, St, Tally};
use super::run::{self, fresh, FILLS};
use super::tests::{console, ours, same, Con, Rng};
use super::{dec, enc, Dcc, R};

fn enc_stage_chain(c: &Con, d: &mut Dcc, label: &str, t: &mut Tally) {
    d.o[0x418..0x420].fill(0);
    d.p(0xb0, 0);
    reorder_stage(c, d, 2, 0, 1, &format!("{label} reorder 1"), t);
    let a: [(usize, fn(&mut Dcc) -> R<()>, &str); 2] = [(0x18c70, enc::channel, "channel 18c70"), (0x19570, dec::signfix, "signfix 19570")];
    for (rva, f, n) in a {
        if !stage(c, d, rva, f, &format!("{label} {n}"), t) {
            return;
        }
    }
    reorder_stage(c, d, 2, 2, 2, &format!("{label} reorder 2"), t);
    if !stage(c, d, 0x19660, enc::color, &format!("{label} color 19660"), t) {
        return;
    }
    reorder_stage(c, d, 2, 2, 3, &format!("{label} reorder 3"), t);
    if !stage(c, d, 0x19980, enc::delta, &format!("{label} delta 19980"), t) {
        return;
    }
    enc::transform(d);
    let b: [(usize, fn(&mut Dcc) -> R<()>, &str); 4] = [(0x19e20, |d| Ok(enc::stats(d)), "stats 19e20"), (0x1a100, enc::tid, "tid 1a100"), (0x1a340, enc::plan, "plan 1a340"), (0x1a770, enc::pack, "pack 1a770")];
    for (rva, f, n) in b {
        if !stage(c, d, rva, f, &format!("{label} {n}"), t) {
            return;
        }
    }
}

#[test]
fn oracle_xgx_dcc_enc_stages() {
    let Some((_orc, ref or)) = console() else { return };
    let mut r = Rng(0x5eed_e1c0_4321);
    let mut t = Tally::default();
    let cfg = cases(or);
    let mut blocks = 0;
    for &(f, tm, _) in &cfg {
        for _ in 0..24 {
            let p = params(&mut r);
            let (c, _) = Con::new(or, f, tm, p);
            let mut d = seeded(f, tm, p, &mut r);
            d.s[0] = block(&mut r);
            if r.next() % 3 == 0 {
                d.p(0x34, r.next() & 0xff);
            }
            enc_stage_chain(&c, &mut d, &format!("fmt {f} tile {tm} params {p:x?}"), &mut t);
            blocks += 1;
        }
    }
    println!("xg_x dcc enc stages: {} configs, {} blocks, {} stage calls matched the console byte-for-byte, {} stopped at a console fault, {} at undefined console state", cfg.len(), blocks, t.ok, t.fault, t.undef);
    assert!(t.ok > 10_000);
}

#[test]
fn oracle_xgx_dcc_enc_e2e() {
    let Some((_orc, ref or)) = console() else { return };
    let drv: St = unsafe { core::mem::transmute(or.rva(0x189b0)) };
    let mut r = Rng(0xe2e_e1c_99);
    let cfg = cases(or);
    let (mut ok, mut stopped) = (0usize, 0usize);
    for &(f, tm, _) in &cfg {
        for _ in 0..12 {
            let p = params(&mut r);
            let (c, _) = Con::new(or, f, tm, p);
            let mut d = seeded(f, tm, p, &mut r);
            prefill(&c, &d);
            for blk in 0..(1 + r.next() % 4) {
                let s0 = block(&mut r);
                d.s[0] = s0;
                c.set_surf(0, &s0);
                let mut n = d.clone();
                if enc::run_checked(&mut n).is_err() {
                    stopped += 1;
                    break;
                }
                drv(c.ptr());
                same(&n, &c, &format!("e2e fmt {f} tile {tm} params {p:x?} block {blk}"));
                d = n;
                ok += 1;
            }
        }
    }
    println!("xg_x dcc enc e2e: {} blocks through console driver 0x189b0 matched native object, surfaces and scratch; {} chains stopped at a console fault or undefined state", ok, stopped);
    assert!(ok > 3000);
}

#[test]
fn oracle_xgx_dcc_encode_export_shim() {
    let Some((_orc, ref or)) = console() else { return };
    let cenc: EncF = unsafe { core::mem::transmute(or.f("XGEncodeDCC")) };
    let mut r = Rng(0x5a1d_e1c0_7e11);
    let cfg = super::tests::configs();
    let mut skips: std::collections::BTreeMap<String, usize> = Default::default();
    let (mut cases_n, mut blocks, mut same_hr, mut rejected, mut setup_rej, mut no_sizes) = (0usize, 0usize, 0usize, 0usize, 0usize, 0usize);
    for _ in 0..6000 {
        let (f, tm) = shim_format(&mut r, &cfg);
        let p = params(&mut r);
        let n = 1 + (r.next() % 6) as usize;
        let mut src = vec![0u8; n * 256];
        for i in 0..n {
            src[i * 256..i * 256 + 256].copy_from_slice(&block(&mut r));
        }
        let with_sizes = r.next() % 4 != 0;
        let sc = known(&mut r);
        set_scratch(or, &sc);
        let (mut nd, mut nm, mut ns) = (vec![0x3cu8; n * 256], vec![0x5au8; n], vec![0xdead_beefu32; n]);
        let nsz = if with_sizes { ns.as_mut_ptr() } else { core::ptr::null_mut() };
        let nh = unsafe { crate::dcc::XGEncodeDCC(n as u32, src.as_ptr(), nd.as_mut_ptr(), nm.as_mut_ptr(), nsz, f, tm, p.as_ptr() as *const crate::dcc::DccParams) };
        cases_n += 1;
        if nh != 0 && f > 0 && f <= 0x78 && ours(f, tm, p).1 == 0 {
            rejected += 1;
            assert_eq!(nh, super::E_INVALIDARG);
            let mut runs = run::setup_runs(f, tm, p, &sc).unwrap();
            let (mut a, mut b) = (vec![0u8; n * 256], vec![0u8; n]);
            match run::encode_blocks(&mut runs, &src, &mut a, &mut b, None) {
                run::Outcome::Stopped { why, .. } => *skips.entry(why).or_default() += 1,
                run::Outcome::Done => panic!("native rejected fmt {f} tile {tm} without a codec stop"),
            }
            continue;
        }
        let (mut cd, mut cm, mut cs) = (vec![0x3cu8; n * 256], vec![0x5au8; n], vec![0xdead_beefu32; n]);
        let csz = if with_sizes { cs.as_mut_ptr() } else { core::ptr::null_mut() };
        let ch = unsafe { cenc(n as u32, src.as_ptr(), cd.as_mut_ptr(), cm.as_mut_ptr(), csz, f, tm, p.as_ptr()) };
        assert_eq!(nh, ch, "hr fmt {f} tile {tm} params {p:x?} n {n}");
        if nh != 0 {
            setup_rej += 1;
        }
        assert_eq!(nm, cm, "meta fmt {f} tile {tm} params {p:x?}");
        assert_eq!(ns, cs, "sizes fmt {f} tile {tm} params {p:x?}");
        assert_eq!(nd, cd, "bytes fmt {f} tile {tm} params {p:x?}");
        if !with_sizes {
            no_sizes += 1;
        }
        same_hr += 1;
        blocks += n;
    }
    println!("xg_x dcc encode export shim: {} cases, {} compared with console XGEncodeDCC ({} encoded blocks, {} without a sizes array, {} argument/setup rejections with equal HRESULT), 0 mismatches in HRESULT, data, meta and sizes; {} native E_INVALIDARG for console faults or undefined output (console not called)", cases_n, same_hr, blocks, no_sizes, setup_rej, rejected);
    for (w, k) in &skips {
        println!("xg_x dcc encode export shim skip: {k} cases: {w}");
        assert!(super::oracle_dec::PROVEN.iter().any(|p| w.contains(p)), "unproven skip reason: {w}");
    }
    assert!(same_hr > 4000);
}

#[test]
fn oracle_xgx_dcc_enc_undefined_classes() {
    let Some((_orc, ref or)) = console() else { return };
    let drv: St = unsafe { core::mem::transmute(or.rva(0x189b0)) };
    let mut r = Rng(0x0dd_e1c0);
    let (mut undef, mut varied, mut faults) = (0usize, 0usize, std::collections::BTreeMap::<String, usize>::new());
    for &(f, t, _) in &cases(or) {
        for _ in 0..30 {
            let p = params(&mut r);
            let s0 = block(&mut r);
            let sc = known(&mut r);
            let mut runs = match run::setup_runs(f, t, p, &sc) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let (mut dst, mut meta) = ([0u8; 256], [0u8; 1]);
            match run::encode_blocks(&mut runs, &s0, &mut dst, &mut meta, None) {
                run::Outcome::Stopped { why, .. } if why.contains("never wrote") => {}
                run::Outcome::Stopped { why, .. } => {
                    *faults.entry(why).or_default() += 1;
                    continue;
                }
                run::Outcome::Done => continue,
            }
            let spills = (0..FILLS).any(|fl| {
                let mut nd = fresh(fl, &sc);
                nd.setup(f, t, p[0], p[1]);
                nd.s[0] = s0;
                matches!(enc::run_checked(&mut nd), Err(super::Stop::Undef(w)) if w.contains("overruns"))
            });
            if spills {
                *faults.entry(super::run::OVERRUN.into()).or_default() += 1;
                continue;
            }
            undef += 1;
            let mut outs = Vec::new();
            for fl in 0..FILLS {
                let (c, _) = Con::new(or, f, t, p);
                let mut d = fresh(fl, &sc);
                d.setup(f, t, p[0], p[1]);
                prefill(&c, &d);
                c.set_surf(0, &s0);
                drv(c.ptr());
                let q = pull(&c, &d);
                outs.push((q.s[1], q.o[0x34]));
            }
            if outs.iter().any(|o| *o != outs[0]) {
                varied += 1;
            }
        }
    }
    println!("xg_x dcc enc undefined output: {undef} single-block cases where native sees output bytes the console never writes; console driver 0x189b0 output changed with the prefill in {varied} of them; other stops {faults:?}");
    assert_eq!(undef, varied);
}

fn enc_fault_cases(or: &super::tests::Tm) -> Vec<(i32, i32, [u32; 2], [u8; 256], super::run::Scratch, String)> {
    let mut v: Vec<(i32, i32, [u32; 2], [u8; 256], super::run::Scratch, String)> = Vec::new();
    let mut r = Rng(0xe1c_fa17);
    for &(f, t, _) in &cases(or) {
        for _ in 0..30 {
            let p = params(&mut r);
            let s0 = block(&mut r);
            let sc = known(&mut r);
            let mut runs = match run::setup_runs(f, t, p, &sc) {
                Ok(x) => x,
                Err(_) => continue,
            };
            let (mut dst, mut meta) = ([0u8; 256], [0u8; 1]);
            if let run::Outcome::Stopped { why, .. } = run::encode_blocks(&mut runs, &s0, &mut dst, &mut meta, None) {
                if !why.contains("after its stack arguments") && v.iter().filter(|c| c.5 == why).count() < 5 && !v.iter().any(|c| c.5 == why && c.0 == f && c.1 == t) {
                    v.push((f, t, p, s0, sc, why));
                }
            }
        }
    }
    v
}

#[test]
fn oracle_xgx_dcc_enc_fault_classes() {
    if let Ok(i) = std::env::var("XGX_DCC_ENC_CHILD") {
        let Some((_orc, ref or)) = console() else { std::process::exit(3) };
        let cenc: EncF = unsafe { core::mem::transmute(or.f("XGEncodeDCC")) };
        let (f, t, p, s0, sc, _) = enc_fault_cases(or).swap_remove(i.parse::<usize>().unwrap());
        set_scratch(or, &sc);
        let (mut out, mut meta, mut size) = ([0u8; 256], [0u8; 1], [0u32; 1]);
        unsafe { cenc(1, s0.as_ptr(), out.as_mut_ptr(), meta.as_mut_ptr(), size.as_mut_ptr(), f, t, p.as_ptr()) };
        std::process::exit(0);
    }
    let Some((_orc, ref or)) = console() else { return };
    let fc = enc_fault_cases(or);
    let mut by: std::collections::BTreeMap<(String, u32), usize> = Default::default();
    for (i, c) in fc.iter().enumerate() {
        let code = super::oracle_dec::child_exit("dcc_codec::oracle_enc::oracle_xgx_dcc_enc_fault_classes", "XGX_DCC_ENC_CHILD", i);
        *by.entry((c.5.clone(), code)).or_default() += 1;
    }
    for ((w, code), n) in &by {
        println!("xg_x dcc enc fault class: {n} child processes running console XGEncodeDCC exited {code:#x}: {w}");
        if w.contains("bytes per element / 4") {
            assert_eq!(*code, 0xc000_0094, "{w}");
        } else if w.contains("overruns") {
            assert!(*code == 0 || *code == 0xc000_0005 || *code == 0xc000_0374, "{w}");
        } else {
            panic!("fault class without an expected console exit: {w}");
        }
    }
    assert!(by.keys().any(|k| k.0.contains("bytes per element / 4")) && by.keys().any(|k| k.0.contains("overruns")));
}

#[test]
fn oracle_xgx_dcc_enc_stream_overflow_layout() {
    let Some((_orc, ref or)) = console() else { return };
    let drv: St = unsafe { core::mem::transmute(or.rva(0x189b0)) };
    let mut r = Rng(0x0f10_e1c0);
    let (mut n, mut varied, mut reach) = (0usize, 0usize, 0usize);
    for &(f, t, _) in &cases(or) {
        for _ in 0..30 {
            let p = params(&mut r);
            let s0 = block(&mut r);
            let sc = known(&mut r);
            let Ok(mut runs) = run::setup_runs(f, t, p, &sc) else { continue };
            let (mut dst, mut meta) = ([0u8; 256], [0u8; 1]);
            match run::encode_blocks(&mut runs, &s0, &mut dst, &mut meta, None) {
                run::Outcome::Stopped { why, .. } if why.contains("overruns") => {}
                _ => continue,
            }
            let mut pick = None;
            for fl in 0..FILLS {
                let mut nd = fresh(fl, &sc);
                nd.setup(f, t, p[0], p[1]);
                nd.s[0] = s0;
                if let Err(super::Stop::Undef(w)) = enc::run_checked(&mut nd) {
                    if w.contains("overruns") {
                        pick = Some(fl);
                        break;
                    }
                }
            }
            let Some(fl) = pick else { continue };
            n += 1;
            let mut outs = Vec::new();
            let mut bufs = Vec::new();
            for lay in 0..3 {
                let (c, _) = Con::new(or, f, t, p);
                let mut d = fresh(fl, &sc);
                d.setup(f, t, p[0], p[1]);
                prefill(&c, &d);
                c.set_surf(0, &s0);
                let mut buf = vec![[0x00u8, 0xff, 0x00][lay]; 0x10000];
                let s2 = c.get_surf(2);
                let base = buf.as_mut_ptr();
                unsafe {
                    if lay == 0 {
                        core::ptr::copy_nonoverlapping(s2.as_ptr(), base.add(256), 256);
                        (c.ptr() as *mut u8).add(0xd8).cast::<*mut u8>().write_unaligned(base.add(256));
                    }
                    (c.ptr() as *mut u8).add(0x98).cast::<*mut u8>().write_unaligned(base);
                }
                drv(c.ptr());
                let img = c.image();
                outs.push((buf[..256].to_vec(), img[0x34]));
                unsafe {
                    (c.ptr() as *mut u8).add(0x98).cast::<*mut u8>().write_unaligned(core::ptr::null_mut());
                }
                core::mem::forget(c);
                bufs.push(buf);
            }
            assert_eq!(outs[1], outs[2], "fmt {f} tile {t} params {p:x?}: isolated layouts");
            let far = (256..0x10000).rev().find(|&i| bufs[1][i] == bufs[2][i]).unwrap_or(0);
            assert!(far >= 256, "fmt {f} tile {t} params {p:x?}: console never wrote past the surface");
            if outs[0] != outs[1] {
                varied += 1;
            }
            reach = reach.max(far + 1 - 256);
        }
    }
    println!("xg_x dcc enc stream overflow: {n} cases where native predicts the packed stream overruns the 256-byte output surface; console driver 0x189b0 wrote past the surface in all {n} (up to {reach} bytes past the end), and its in-bounds output changed when the next heap block was the codec's own surface 2 in {varied} of them");
    assert!(n > 0);
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap()).collect()
}

fn neighbour_class(w: &str) -> bool {
    w.contains("reads past") || w.contains("message pointer")
}

fn neighbour_cases(or: &super::tests::Tm) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = Vec::new();
    let mut r = Rng(0x3a1c_0f5e);
    for &(f, t, _) in &cases(or) {
        for _ in 0..60 {
            let p = params(&mut r);
            let s0 = block(&mut r);
            let sc = known(&mut r);
            for fl in 0..FILLS {
                let mut nd = fresh(fl, &sc);
                if nd.setup(f, t, p[0], p[1]) != 0 {
                    break;
                }
                nd.s[0] = s0;
                if let Err(super::Stop::Undef(w)) = enc::run_checked(&mut nd) {
                    if neighbour_class(w) && v.iter().filter(|c| c.1 == w).count() < 6 {
                        v.push((format!("{f} {t} {} {} {} {} {fl}", p[0], p[1], hex(&s0), hex(&sc.b)), w.to_string()));
                    }
                    break;
                }
            }
        }
    }
    v
}

fn neighbour_child(spec: &str, guard: usize) -> u32 {
    std::process::Command::new(std::env::current_exe().unwrap())
        .args(["dcc_codec::oracle_enc::oracle_xgx_dcc_enc_neighbour_memory_classes", "--exact", "--test-threads=1"])
        .env("XGX_DCC_ENC_NB", format!("{spec} {guard}"))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap()
        .code()
        .unwrap_or(0) as u32
}

#[test]
fn oracle_xgx_dcc_enc_neighbour_memory_classes() {
    if let Ok(spec) = std::env::var("XGX_DCC_ENC_NB") {
        let Some((_orc, ref or)) = console() else { std::process::exit(3) };
        let w: Vec<&str> = spec.split(' ').collect();
        let (f, t, p) = (w[0].parse().unwrap(), w[1].parse().unwrap(), [w[2].parse().unwrap(), w[3].parse().unwrap()]);
        let s0: [u8; 256] = unhex(w[4]).try_into().unwrap();
        let sc = super::run::Scratch { b: unhex(w[5]).try_into().unwrap(), unknown: 0 };
        let fl: usize = w[6].parse().unwrap();
        let g: usize = w[7].parse().unwrap();
        let gb = |i: usize| -> u8 { super::run::fill(g, 0x9000 + i) };
        let drv: St = unsafe { core::mem::transmute(or.rva(0x189b0)) };
        let (c, _) = Con::new(or, f, t, p);
        let mut d = fresh(fl, &sc);
        d.setup(f, t, p[0], p[1]);
        prefill(&c, &d);
        c.set_surf(0, &s0);
        for i in 0x420..c.buf.len() * 8 {
            unsafe { (c.ptr() as *mut u8).add(i).write(gb(i)) };
        }
        let mut s1 = vec![0xa5u8; 0x40000];
        let mut s2: Vec<u8> = (0..0x40000).map(|i| gb(0x10000 + i)).collect();
        s2[..256].copy_from_slice(&c.get_surf(2));
        unsafe {
            (c.ptr() as *mut u8).add(0x98).cast::<*mut u8>().write_unaligned(s1.as_mut_ptr());
            (c.ptr() as *mut u8).add(0xd8).cast::<*mut u8>().write_unaligned(s2.as_mut_ptr());
        }
        drv(c.ptr());
        let mut h = 0x811c_9dc5u32;
        for b in s1[..256].iter().chain(core::iter::once(&c.image()[0x34])) {
            h = (h ^ *b as u32).wrapping_mul(0x0100_0193);
        }
        let past = s1[256..].iter().any(|&b| b != 0xa5);
        std::process::exit((h & 0xffff) as i32 + 0x10 + if past { 0x10_0000 } else { 0 });
    }
    let Some((_orc, ref or)) = console() else { return };
    let nc = neighbour_cases(or);
    let mut by: std::collections::BTreeMap<String, [usize; 4]> = Default::default();
    for (spec, w) in &nc {
        let codes: Vec<u32> = (0..6).map(|g| neighbour_child(spec, g)).collect();
        let e = by.entry(w.clone()).or_default();
        if codes.iter().any(|&c| c >= 0xc000_0000) {
            e[0] += 1;
        } else if codes.iter().any(|&c| c & 0xffff != codes[0] & 0xffff) {
            e[1] += 1;
        } else if codes.iter().all(|&c| c & 0x10_0000 != 0) {
            e[3] += 1;
        } else {
            e[2] += 1;
        }
    }
    for (w, [crash, differ, same_out, past]) in &by {
        println!("xg_x dcc enc neighbour-memory class: {} cases, console driver 0x189b0 faulted under at least one of 6 guard fills (00, ff, 4 hashed patterns) in {crash}, output changed with the bytes past its object and surface 2 in {differ}, overran its 256-byte output surface under every fill in {past}, none of these in {same_out}: {w}", crash + differ + same_out + past);
    }
    for (w, k) in &by {
        assert_eq!(k[2], 0, "{w}");
    }
}
