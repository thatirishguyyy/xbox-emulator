use super::run::{self, Scratch};

pub(super) struct VRng(pub(super) u64);

impl VRng {
    pub(super) fn n(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 16) as u32
    }
}

pub(super) struct Case {
    pub(super) enc: bool,
    pub(super) fmt: i32,
    pub(super) sw: i32,
    pub(super) p: [u32; 4],
    pub(super) n: usize,
    pub(super) src: Vec<u8>,
    pub(super) meta: Vec<u8>,
}

fn vblock(r: &mut VRng) -> [u8; 256] {
    let mut b = [0u8; 256];
    match r.n() % 6 {
        0 => b.iter_mut().for_each(|x| *x = r.n() as u8),
        1 => b.fill(r.n() as u8),
        2 => {
            let per = [1usize, 2, 4, 8, 16][(r.n() % 5) as usize];
            let e: Vec<u8> = (0..per).map(|_| r.n() as u8).collect();
            for i in 0..256 {
                b[i] = e[i % per];
            }
        }
        3 => {
            let s = (r.n() % 5) as u8;
            let base = r.n() as u8;
            for i in 0..256 {
                b[i] = base.wrapping_add(s.wrapping_mul(i as u8));
            }
        }
        4 => {
            let base = r.n() as u8;
            b.iter_mut().for_each(|x| *x = base.wrapping_add((r.n() % 3) as u8));
        }
        _ => {
            for _ in 0..(r.n() % 12) {
                b[(r.n() % 256) as usize] = r.n() as u8;
            }
        }
    }
    b
}

pub(super) fn case(i: u64) -> Case {
    let mut r = VRng(0x9e37_79b9_7f4a_7c15 ^ (i.wrapping_mul(0x2545_F491_4F6C_DD1D)) | 1);
    for _ in 0..4 {
        r.n();
    }
    let enc = r.n() % 2 == 0;
    let fmt = 1 + r.n() % 0x78;
    let sw = r.n() % 0x1b;
    let _dim = r.n() % 5;
    let mut p = [r.n() % 3, [0u32, 0, 1, 2, 3][(r.n() % 5) as usize], 0, 0];
    if r.n() % 8 == 0 {
        p[2] = r.n();
        p[3] = r.n();
    }
    if r.n() % 16 == 0 {
        p[0] = r.n() % 40;
    }
    let n = 1 + (r.n() % 3) as usize;
    let mut src = Vec::new();
    let mut meta = Vec::new();
    for _ in 0..n {
        src.extend_from_slice(&vblock(&mut r));
        meta.push([0x00u8, 0x10, 0x20, 0x30, 0x50, 0x11, 0x21, 0x33, 0x44, 0x55, 0x01, 0x02, 0x03, 0x04, 0x05, 0x13, 0x23][(r.n() % 17) as usize] ^ if r.n() % 4 == 0 { r.n() as u8 } else { 0 });
    }
    Case { enc, fmt: fmt as i32, sw: sw as i32, p, n, src, meta }
}

pub(super) fn reason(c: &Case, sc: &Scratch) -> Option<String> {
    let mut runs = run::setup_runs(c.fmt, c.sw, [c.p[0], c.p[1]], sc).ok()?;
    let out = if c.enc {
        let (mut d, mut m) = (vec![0u8; c.n * 256], vec![0u8; c.n]);
        run::encode_blocks(&mut runs, &c.src, &mut d, &mut m, None)
    } else {
        let (mut d, mut notes) = (vec![0u8; c.n * 256], Vec::new());
        run::decode_blocks(&mut runs, &c.src, &c.meta, &mut d, &mut notes)
    };
    match out {
        run::Outcome::Done => None,
        run::Outcome::Stopped { block, why } => Some(format!("block {block}: {why}")),
    }
}

#[test]
fn dccx_stable_reasons() {
    let Ok(ids) = std::env::var("XGX_DCCX_IDS") else { return };
    let mut by: std::collections::BTreeMap<String, Vec<u64>> = Default::default();
    for id in ids.split_whitespace().map(|x| x.parse::<u64>().unwrap()) {
        let c = case(id);
        let w = reason(&c, &Scratch { b: [0; 64], unknown: 0 }).unwrap_or_else(|| "native accepts from a zero scratch".into());
        by.entry(format!("{} {}", if c.enc { "E" } else { "D" }, w)).or_default().push(id);
    }
    for (w, v) in &by {
        println!("{} {w} {:?}", v.len(), &v[..v.len().min(10)]);
    }
}

#[test]
fn dccx_region_probe() {
    let Ok(ids) = std::env::var("XGX_DCCX_PROBE") else { return };
    for id in ids.split_whitespace().map(|x| x.parse::<u64>().unwrap()) {
        let c = case(id);
        println!("case {id} enc {} fmt {} sw {} p {:x?} n {} meta {:02x?}", c.enc, c.fmt, c.sw, c.p, c.n, c.meta);
        let sc = Scratch { b: [0; 64], unknown: 0 };
        let mut names: Vec<(String, Vec<usize>)> = run::UNSET.iter().map(|&(a, b)| (format!("obj {a:#x}..{b:#x}"), (a..b).collect())).collect();
        for s in 0..3 {
            names.push((format!("surface {s}"), (0x1000 + 0x100 * s..0x1100 + 0x100 * s).collect()));
        }
        for (name, idx) in names {
            let mut outs = Vec::new();
            for f in 0..run::FILLS {
                let mut d = run::fresh(0, &sc);
                let fd = run::fresh(f, &sc);
                for &i in &idx {
                    if i < 0x1000 {
                        d.o[i] = fd.o[i];
                    } else {
                        let s = (i - 0x1000) / 0x100;
                        d.s[s][i & 0xff] = fd.s[s][i & 0xff];
                    }
                }
                if d.setup(c.fmt, c.sw, c.p[0], c.p[1]) != 0 {
                    break;
                }
                let mut o = Vec::new();
                let mut err = None;
                for b in 0..c.n {
                    if c.enc {
                        d.s[0].copy_from_slice(&c.src[b * 256..b * 256 + 256]);
                        if let Err(e) = super::enc::run_checked(&mut d) {
                            err = Some(e);
                            break;
                        }
                        o.extend_from_slice(&d.s[1]);
                    } else {
                        d.s[1].copy_from_slice(&c.src[b * 256..b * 256 + 256]);
                        d.p(0x34, c.meta[b] as u32);
                        if let Err(e) = super::dec::run(&mut d) {
                            err = Some(e);
                            break;
                        }
                        o.extend_from_slice(&d.s[0]);
                    }
                }
                outs.push((o, err));
            }
            let diff: Vec<usize> = if outs.len() > 1 { (0..outs[0].0.len().min(outs[1].0.len())).filter(|&i| outs.iter().any(|x| x.0.get(i) != outs[0].0.get(i))).collect() } else { vec![] };
            println!("  {name}: differing output bytes {} {:?} errs {:?}", diff.len(), &diff[..diff.len().min(12)], outs.iter().map(|x| x.1).collect::<Vec<_>>());
        }
    }
}

#[test]
fn dccx_surface_initial_probe() {
    if std::env::var("XGX_DCCX_SURF").is_err() {
        return;
    }
    let Some((_orc, ref or)) = super::tests::console() else { return };
    let dec: super::oracle_dec::DecF = unsafe { core::mem::transmute(or.f("XGDecodeDCC")) };
    for k in 0..6 {
        let (c, _) = super::tests::Con::new(or, 28, 14, [0, 0]);
        let s: Vec<[u8; 256]> = (0..3).map(|i| c.get_surf(i)).collect();
        println!("con {k}: s0 {:02x?} s1 {:02x?} s2 {:02x?} ptrs {:p} {:p} {:p}", &s[0][..12], &s[1][..12], &s[2][..12], c.surf(0), c.surf(1), c.surf(2));
        c.set_surf(0, &[0x11 + k as u8; 256]);
        let src = [0x5au8; 256];
        let mut out = [0u8; 256];
        unsafe { dec(1, src.as_ptr(), [0xffu8].as_ptr(), out.as_mut_ptr(), 28, 14, [0u32, 0].as_ptr()) };
    }
}

pub(super) fn h64(b: &[u8]) -> u64 {
    let mut x = 0xcbf2_9ce4_8422_2325u64;
    for &c in b {
        x ^= c as u64;
        x = x.wrapping_mul(0x100_0000_01b3);
    }
    x
}

pub(super) unsafe fn vcall(dec: usize, enc: usize, c: &Case) -> (i32, u64) {
    vcall_smeared(dec, enc, c, None)
}

pub(super) unsafe fn vcall_smeared(dec: usize, enc: usize, c: &Case, pat: Option<usize>) -> (i32, u64) {
    let mut sbuf = vec![0u8; c.src.len() + 64];
    sbuf[..c.src.len()].copy_from_slice(&c.src);
    let mut mbuf = vec![0u8; c.n + 64];
    mbuf[..c.n].copy_from_slice(&c.meta);
    let mut dst = vec![0x3cu8; c.n * 256 + 64];
    let mut mo = vec![0x3cu8; c.n + 64];
    let mut sz = vec![0xdead_beefu32; c.n + 16];
    if let Some(p) = pat {
        let _ = smear(p);
    }
    let hr = if c.enc {
        let f: super::oracle_dec::EncF = core::mem::transmute(enc);
        f(c.n as u32, sbuf.as_ptr(), dst.as_mut_ptr(), mo.as_mut_ptr(), sz.as_mut_ptr(), c.fmt, c.sw, c.p.as_ptr())
    } else {
        let f: super::oracle_dec::DecF = core::mem::transmute(dec);
        f(c.n as u32, sbuf.as_ptr(), mbuf.as_ptr(), dst.as_mut_ptr(), c.fmt, c.sw, c.p.as_ptr())
    };
    let mut all = dst[..c.n * 256].to_vec();
    if c.enc {
        all.extend_from_slice(&mo[..c.n]);
        for s in &sz[..c.n] {
            all.extend_from_slice(&s.to_le_bytes());
        }
    }
    (hr, h64(&all))
}

#[test]
fn dccx_hash_probe() {
    let Ok(ids) = std::env::var("XGX_DCCX_HASH") else { return };
    let Some((_orc, ref or)) = super::tests::console() else { return };
    for id in ids.split_whitespace().map(|x| x.parse::<u64>().unwrap()) {
        let c = case(id);
        let (hr, h) = unsafe { vcall(or.f("XGDecodeDCC"), or.f("XGEncodeDCC"), &c) };
        println!("hash {} {id} {:08x} {h:016x}", or.2, hr as u32);
    }
}

extern "system" {
    fn GetProcessHeap() -> *mut core::ffi::c_void;
    fn HeapAlloc(h: *mut core::ffi::c_void, f: u32, n: usize) -> *mut u8;
    fn HeapFree(h: *mut core::ffi::c_void, f: u32, p: *mut u8) -> i32;
}

#[inline(never)]
pub(super) fn smear(pattern: usize) -> u8 {
    let mut a = [0u8; 0x8000];
    for (i, x) in a.iter_mut().enumerate() {
        unsafe { core::ptr::write_volatile(x, run::fill(pattern + 1, 0x4000 + i)) };
    }
    unsafe { core::ptr::read_volatile(&a[0x777]) }
}

pub(super) fn spray(pattern: usize) {
    if pattern == 0 {
        return;
    }
    let h = unsafe { GetProcessHeap() };
    let n = 64 + 37 * pattern;
    let ps: Vec<*mut u8> = (0..n).map(|i| unsafe { HeapAlloc(h, 0, [0x10usize, 0x100, 4, 0x100, 0x110][i % 5]) }).collect();
    for (i, &p) in ps.iter().enumerate() {
        let len = [0x10usize, 0x100, 4, 0x100, 0x110][i % 5];
        for k in 0..len {
            unsafe { p.add(k).write(run::fill(pattern + 1, i * 0x200 + k)) };
        }
    }
    for (i, &p) in ps.iter().enumerate() {
        if (i * 7 + pattern) % 3 != 0 {
            unsafe { HeapFree(h, 0, p) };
        }
    }
}

pub(super) fn dccx_child(test: &str, var: &str, spec: String) -> u32 {
    std::process::Command::new(std::env::current_exe().unwrap())
        .args([test, "--exact", "--test-threads=1"])
        .env(var, spec)
        .env("XGX_DCC_BUILD", std::env::var("XGX_DCC_BUILD").unwrap_or_else(|_| "Trackmania".into()))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap()
        .code()
        .unwrap_or(0) as u32
}

pub(super) const VERIFIER_STABLE: [u64; 49] = [81, 103, 126, 175, 321, 325, 416, 428, 467, 580, 594, 674, 733, 741, 756, 816, 826, 829, 835, 1051, 1074, 1103, 1196, 1217, 1301, 1322, 1360, 1396, 1457, 1465, 1466, 1731, 1756, 1796, 1882, 1959, 2280, 2314, 2352, 2356, 2416, 2609, 2640, 2659, 2742, 2792, 2793, 2805, 2810];

#[test]
fn oracle_xgx_dcc_rejected_cases_depend_on_process_heap() {
    if let Ok(spec) = std::env::var("XGX_DCC_HEAP_CHILD") {
        let (id, pat) = spec.split_once(':').unwrap();
        spray(pat.parse().unwrap());
        let Some((_orc, ref or)) = super::tests::console() else { std::process::exit(3) };
        let c = case(id.parse().unwrap());
        let (hr, h) = unsafe { vcall_smeared(or.f("XGDecodeDCC"), or.f("XGEncodeDCC"), &c, Some(pat.parse().unwrap())) };
        std::process::exit(if hr != 0 { 1 } else { (h & 0xffff) as i32 + 0x10 });
    }
    let (mut varied, mut faulted) = (0usize, 0usize);
    let mut by: std::collections::BTreeMap<String, (usize, usize)> = Default::default();
    for &id in &VERIFIER_STABLE {
        let c = case(id);
        let why = reason(&c, &Scratch { b: [0; 64], unknown: 0 }).expect("native accepts a verifier-stable case");
        let codes: Vec<u32> = (0..5).map(|p| dccx_child("dcc_codec::verify::oracle_xgx_dcc_rejected_cases_depend_on_process_heap", "XGX_DCC_HEAP_CHILD", format!("{id}:{p}"))).collect();
        let f = codes.iter().any(|&x| x >= 0xc000_0000);
        let v = codes.iter().any(|&x| x != codes[0]);
        let e = by.entry(why.split_once(": ").map(|x| x.1.to_string()).unwrap_or(why.clone())).or_default();
        e.0 += 1;
        if f || v {
            e.1 += 1;
        }
        faulted += f as usize;
        varied += (v && !f) as usize;
        assert!(f || v, "case {id}: console result {codes:x?} did not change with the process heap; native rejects it ({why})");
    }
    for (w, (n, k)) in &by {
        println!("xg_x dcc verifier-stable class: {k} of {n} cases change with the process heap and stack state before the call: {w}");
    }
    println!("xg_x dcc verifier-stable cases: {} cases run in child processes with 5 different heap and stack states before one console {} export call: {faulted} faulted in at least one state, {varied} returned different bytes, 0 unchanged", VERIFIER_STABLE.len(), std::env::var("XGX_DCC_BUILD").unwrap_or_else(|_| "Trackmania".into()));
}

pub(super) fn native_verifier_hash(c: &Case) -> (String, u64, usize) {
    let mut d = run::fresh(0, &Scratch { b: [0; 64], unknown: 0 });
    if d.setup(c.fmt, c.sw, c.p[0], c.p[1]) != 0 {
        return ("setup".into(), 0, 0);
    }
    let (mut dst, mut meta, mut sizes) = (Vec::new(), Vec::new(), Vec::new());
    let mut spill = 0;
    for b in 0..c.n {
        d.s[0].copy_from_slice(&c.src[b * 256..b * 256 + 256]);
        if let Err(e) = super::enc::run(&mut d) {
            return (format!("{e:?}"), 0, spill);
        }
        spill = spill.max(d.spill.len());
        dst.extend_from_slice(&d.s[1]);
        meta.push(d.o[0x34]);
        match run::block_size(&d) {
            Ok(s) => sizes.push(s),
            Err(e) => return (format!("size {e:?}"), 0, spill),
        }
    }
    let mut all = dst;
    all.extend_from_slice(&meta);
    for s in &sizes {
        all.extend_from_slice(&s.to_le_bytes());
    }
    ("ok".into(), h64(&all), spill)
}

#[test]
fn dccx_spill_compare() {
    if std::env::var("XGX_DCCX_SPILL").is_err() {
        return;
    }
    let Some((_orc, ref or)) = super::tests::console() else { return };
    for &id in &VERIFIER_STABLE {
        let c = case(id);
        if !c.enc {
            continue;
        }
        let (st, h, sp) = native_verifier_hash(&c);
        let (hr, ch) = unsafe { vcall(or.f("XGDecodeDCC"), or.f("XGEncodeDCC"), &c) };
        println!("spill {id} native {st} {h:016x} spill {sp} console {:08x} {ch:016x} {}", hr as u32, if h == ch { "EQ" } else { "ne" });
    }
}

fn seq_case(seed: u64, j: u64) -> Case {
    case(seed.wrapping_mul(1_000_003).wrapping_add(j))
}

fn scratch_copy() -> Scratch {
    let g = run::SCRATCH.lock().unwrap();
    Scratch { b: g.b, unknown: g.unknown }
}

fn seq_child(seed: u64, start: u64, end: u64) {
    let Some((_orc, ref or)) = super::tests::console() else { std::process::exit(3) };
    let init = unsafe { core::slice::from_raw_parts(or.sc() as *const u8, 64).to_vec() };
    assert!(init.iter().all(|&b| b == 0), "console scratch at load is not zero: {init:02x?}");
    *run::SCRATCH.lock().unwrap() = Scratch { b: [0; 64], unknown: 0 };
    let (nd, ne) = (crate::dcc::XGDecodeDCC as usize, crate::dcc::XGEncodeDCC as usize);
    let (cd, ce) = (or.f("XGDecodeDCC"), or.f("XGEncodeDCC"));
    let out = std::io::stdout();
    let mut o = out.lock();
    use std::io::Write;
    for j in start..end {
        let c = seq_case(seed, j);
        let why = reason(&c, &scratch_copy()).unwrap_or_default();
        let (nh, nx) = unsafe { vcall(nd, ne, &c) };
        if why.contains("faults here") {
            unsafe { core::ptr::write_bytes(or.sc() as *mut u8, 0, 64) };
            *run::SCRATCH.lock().unwrap() = Scratch { b: [0; 64], unknown: 0 };
            writeln!(o, "@@S {j} {:08x} fault", nh as u32).unwrap();
            o.flush().unwrap();
            continue;
        }
        writeln!(o, "@@C {j} {:08x}", nh as u32).unwrap();
        o.flush().unwrap();
        let (ch, cx) = unsafe { vcall(cd, ce, &c) };
        writeln!(o, "@@R {j} {:08x} {nx:016x} {:08x} {cx:016x} {}", nh as u32, ch as u32, why.replace(' ', "_")).unwrap();
        o.flush().unwrap();
    }
    writeln!(o, "@@DONE").unwrap();
}

#[test]
fn oracle_xgx_dcc_call_sequences() {
    if let Ok(spec) = std::env::var("XGX_DCC_SEQ_CHILD") {
        let v: Vec<u64> = spec.split(' ').map(|x| x.parse().unwrap()).collect();
        seq_child(v[0], v[1], v[2]);
        std::process::exit(0);
    }
    let build = std::env::var("XGX_DCC_BUILD").unwrap_or_else(|_| "Trackmania".into());
    let seqs: u64 = std::env::var("XGX_DCC_SEQS").ok().and_then(|x| x.parse().ok()).unwrap_or(3);
    let len: u64 = std::env::var("XGX_DCC_SEQ_LEN").ok().and_then(|x| x.parse().ok()).unwrap_or(400);
    let (mut calls, mut same, mut rejected, mut faults, mut crashes, mut restarts, mut hangs) = (0usize, 0usize, 0usize, 0usize, 0usize, 0usize, 0usize);
    let mut bad = Vec::new();
    let v0: u64 = std::env::var("XGX_DCC_SEQ0").ok().and_then(|x| x.parse().ok()).unwrap_or(3000);
    let plan: Vec<(u64, u64)> = std::iter::once((0u64, v0)).chain((1..=seqs).map(|s| (s, len))).collect();
    for &(seed, len) in &plan {
        let mut j = 0u64;
        while j < len {
            let log = std::env::temp_dir().join(format!("xgx_dcc_seq_{}_{seed}_{j}.txt", std::process::id()));
            let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["dcc_codec::verify::oracle_xgx_dcc_call_sequences", "--exact", "--test-threads=1", "--nocapture"])
                .env("XGX_DCC_SEQ_CHILD", format!("{seed} {j} {len}"))
                .env("XGX_DCC_BUILD", &build)
                .stdout(std::fs::File::create(&log).unwrap())
                .stderr(std::fs::File::create(log.with_extension("err")).unwrap())
                .spawn()
                .unwrap();
            let t0 = std::time::Instant::now();
            let mut hung = false;
            while child.try_wait().unwrap().is_none() {
                if t0.elapsed().as_secs() > 10 {
                    let _ = child.kill();
                    let _ = child.wait();
                    hung = true;
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            let st = child.try_wait().ok().flatten();
            if std::env::var("XGX_DCC_SEQ_TRACE").is_ok() {
                println!("child from {j}: {:?} after {} ms hung {hung}", st, t0.elapsed().as_millis());
            }
            let text = std::fs::read_to_string(&log).unwrap_or_default();
            let _ = std::fs::remove_file(&log);
            let err = std::fs::read_to_string(log.with_extension("err")).unwrap_or_default();
            let _ = std::fs::remove_file(log.with_extension("err"));
            let mut pending = None;
            let mut done = false;
            for l in text.lines().filter_map(|l| l.find("@@").map(|k| &l[k..])) {
                let t: Vec<&str> = l.split(' ').collect();
                match t[0] {
                    "@@C" => pending = Some((t[1].parse::<u64>().unwrap(), t[2] == "00000000")),
                    "@@S" => {
                        faults += 1;
                        calls += 1;
                        j = t[1].parse::<u64>().unwrap() + 1;
                    }
                    "@@R" => {
                        pending = None;
                        calls += 1;
                        let id: u64 = t[1].parse().unwrap();
                        j = id + 1;
                        if t[2] == "00000000" {
                            if t[2] == t[4] && t[3] == t[5] {
                                same += 1;
                            } else if bad.len() < 12 {
                                bad.push(format!("seed {seed} call {id}: native {} {} console {} {}", t[2], t[3], t[4], t[5]));
                            } else {
                                bad.push(String::new());
                            }
                        } else {
                            rejected += 1;
                        }
                    }
                    "@@DONE" => done = true,
                    _ => {}
                }
            }
            if done {
                break;
            }
            match pending {
                Some((id, accepted)) => {
                    if accepted {
                        bad.push(format!("seed {seed} call {id}: native S_OK but the console {} on it", if hung { "hung" } else { "crashed" }));
                    }
                    println!("restart seed {seed} call {id} hung {hung} after {} ms", t0.elapsed().as_millis());
                    if hung {
                        hangs += 1;
                    } else {
                        crashes += 1;
                    }
                    restarts += 1;
                    j = id + 1;
                }
                None => {
                    println!("restart seed {seed} from {j}: no progress, hung {hung}, status {st:?}, stdout {:?}, stderr {:?}", &text[text.len().saturating_sub(300)..], &err[err.len().saturating_sub(600)..]);
                    restarts += 1;
                    assert!(false, "child made no progress from call {j} (hung {hung}, status {st:?})");
                }
            }
        }
    }
    for b in bad.iter().filter(|b| !b.is_empty()) {
        println!("sequence divergence: {b}");
    }
    println!("xg_x dcc call sequences ({build}): the verifier's own case order 0..{v0} plus {seqs} random sequences of {len} mixed XGDecodeDCC/XGEncodeDCC calls sharing one console and one native state with no reset between calls; {calls} calls: {same} native S_OK equal to the console (hr, data, meta, sizes), {rejected} native E_INVALIDARG (console called anyway so its global scratch advances), {faults} console-fault inputs (both states reset, as a process restart), {crashes} console crashes and {hangs} console hangs (killed after 10 s) on native-rejected inputs (restart), {} divergences", bad.len());
    assert!(bad.is_empty());
    assert!(same as u64 > (v0 + seqs * len) / 3);
}

#[test]
fn xgx_dcc_export_overrun_rejects_in_bounds() {
    let _g = crate::oracle::serial();
    const G: usize = 4096;
    let (mut over, mut before, mut cases) = (0usize, 0usize, 0usize);
    for i in 0..20_000u64 {
        let c = case(i);
        if !c.enc {
            continue;
        }
        cases += 1;
        *run::SCRATCH.lock().unwrap() = Scratch { b: [0; 64], unknown: 0 };
        let Some(why) = reason(&c, &Scratch { b: [0; 64], unknown: 0 }) else { continue };
        if !why.contains("overruns") {
            continue;
        }
        let stop: usize = why["block ".len()..why.find(':').unwrap()].parse().unwrap();
        over += 1;
        before += stop;
        let (mut d, mut m, mut z) = (vec![0x3cu8; c.n * 256 + G], vec![0x5au8; c.n + G], vec![0xdead_beefu32; c.n + G]);
        *run::SCRATCH.lock().unwrap() = Scratch { b: [0; 64], unknown: 0 };
        let hr = unsafe { crate::dcc::XGEncodeDCC(c.n as u32, c.src.as_ptr(), d.as_mut_ptr(), m.as_mut_ptr(), z.as_mut_ptr(), c.fmt, c.sw, c.p.as_ptr() as *const crate::dcc::DccParams) };
        assert_eq!(hr, super::E_INVALIDARG, "case {i}: {why}");
        assert!(d[stop * 256..].iter().all(|&b| b == 0x3c), "case {i}: dst written at or past the overrunning block {stop}");
        assert!(m[stop..].iter().all(|&b| b == 0x5a), "case {i}: meta written at or past block {stop}");
        assert!(z[stop..].iter().all(|&v| v == 0xdead_beef), "case {i}: sizes written at or past block {stop}");
    }
    *run::SCRATCH.lock().unwrap() = Scratch { b: [0; 64], unknown: 0 };
    println!("xg_x dcc export overrun rule: {cases} XGEncodeDCC calls from the verifier generator, {over} where a block's packed stream overruns 256 bytes: every one returned E_INVALIDARG, wrote nothing at or past the overrunning block (dst, meta, sizes and a {G}-element guard tail untouched), {before} earlier blocks written in bounds");
    assert!(over > 100);
}
