use super::oracle_dec::copy;
use super::oracle_enc::reject_cause;
use super::{dec, Dcc};

pub(super) struct Rng(pub(super) u64);
impl Rng {
    pub(super) fn n(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 16) as u32
    }
}

pub(super) struct Case {
    pub(super) enc: bool,
    pub(super) fmt: u32,
    pub(super) sw: u32,
    pub(super) dim: u32,
    pub(super) p: [u32; 4],
    pub(super) n: usize,
    pub(super) src: Vec<u8>,
    pub(super) meta: Vec<u8>,
}

fn block(r: &mut Rng) -> [u8; 256] {
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
    let mut r = Rng(0x9e3779b97f4a7c15 ^ (i.wrapping_mul(0x2545F4914F6CDD1D)) | 1);
    for _ in 0..4 {
        r.n();
    }
    let enc = r.n() % 2 == 0;
    let fmt = 1 + r.n() % 0x78;
    let sw = r.n() % 32;
    let dim = r.n() % 5;
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
        src.extend_from_slice(&block(&mut r));
        meta.push([0x00u8, 0x10, 0x20, 0x30, 0x50, 0x11, 0x21, 0x33, 0x44, 0x55, 0x01, 0x02, 0x03, 0x04, 0x05, 0x13, 0x23][(r.n() % 17) as usize] ^ if r.n() % 4 == 0 { r.n() as u8 } else { 0 });
    }
    Case { enc, fmt, sw, dim, p, n, src, meta }
}

#[test]
#[ignore]
fn oracle_dcc_cases_bucket() {
    let Ok(path) = std::env::var("TOMBOX_DCC_IDS") else { return };
    let ids: Vec<u64> = std::fs::read_to_string(path).unwrap().split_whitespace().map(|x| x.parse().unwrap()).collect();
    let mut b: std::collections::BTreeMap<String, Vec<u64>> = std::collections::BTreeMap::new();
    for i in ids {
        let c = case(i);
        let mut d = Dcc::new();
        if d.init(c.fmt, c.sw, c.dim, c.p[0], c.p[1]).is_err() {
            b.entry("init err".into()).or_default().push(i);
            continue;
        }
        let mut key = String::from("no reject");
        for k in 0..c.n {
            let pr = if c.enc {
                let mut od = vec![0u8; 256 * (k + 1)];
                let mut om = vec![0u8; k + 1];
                crate::dcc::encode(k as u32 + 1, &c.src[..256 * (k + 1)], &mut od, &mut om, None, c.fmt, c.sw, c.dim, c.p[0], c.p[1])
            } else {
                let mut od = vec![0u8; 256 * (k + 1)];
                crate::dcc::decode(k as u32 + 1, &c.src[..256 * (k + 1)], &c.meta[..k + 1], &mut od, c.fmt, c.sw, c.dim, c.p[0], c.p[1])
            };
            if c.enc {
                let mut inp = [0u8; 256];
                inp.copy_from_slice(&c.src[k * 256..k * 256 + 256]);
                let mut t = copy(&d);
                t.b[0] = inp;
                if pr == 0 {
                    let _ = super::enc::run_with(&mut t, Some([0; 64]), Some([0; 64]));
                    d = t;
                    continue;
                }
                let mut hd = copy(&d);
                hd.w32(0xf0, 0);
                let mut hs: Vec<usize> = super::enc::relay_holes(&hd, 1).into_iter().map(|x| x % 64).collect();
                hs.sort();
                hs.dedup();
                let q: Vec<usize> = hs.iter().map(|x| x / 8).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
                key = format!("E blk{} {} holeqw {:?}", k, reject_cause(&d, &inp), q);
                break;
            } else {
                d.b[1].copy_from_slice(&c.src[k * 256..k * 256 + 256]);
                d.w32(0x58, c.meta[k] as u32);
                let ok = dec::run(&mut d).is_ok();
                if pr == 0 {
                    let _ = ok;
                    continue;
                }
                key = format!("D blk{} s0={}", k, d.r32(0));
                break;
            }
        }
        b.entry(key).or_default().push(i);
    }
    for (k, v) in &b {
        println!("bucket {} : {} {:?}", k, v.len(), &v[..v.len().min(6)]);
    }
}

extern "system" {
    fn VirtualAlloc(a: *mut core::ffi::c_void, n: usize, t: u32, p: u32) -> *mut core::ffi::c_void;
    fn VirtualProtect(a: *mut core::ffi::c_void, n: usize, p: u32, o: *mut u32) -> i32;
    fn FlushInstructionCache(h: isize, a: *const core::ffi::c_void, n: usize) -> i32;
}

pub(super) const SNAP: usize = 0x800;
pub(super) const SLOTS: usize = 32;

pub(super) struct Snap {
    pub(super) page: *mut u8,
    site: usize,
    old: [u8; 4],
}

impl Snap {
    pub(super) unsafe fn new(h: usize, site: usize, target: usize) -> Snap {
        let mut page: *mut u8 = core::ptr::null_mut();
        let mut a = (h + 0x0100_0000) & !0xffff;
        while page.is_null() {
            page = VirtualAlloc(a as *mut _, 0x20000, 0x3000, 0x40) as *mut u8;
            a += 0x10000;
        }
        let code = page;
        let cnt = page.add(0x100) as u64;
        let mut c: Vec<u8> = vec![0x49, 0xbb];
        c.extend_from_slice(&cnt.to_le_bytes());
        c.extend_from_slice(&[0x49, 0x8b, 0x03, 0x49, 0xff, 0x03, 0x83, 0xe0, 0x1f, 0x48, 0xc1, 0xe0, 0x0b, 0x4d, 0x8d, 0x5c, 0x03, 0x08]);
        c.extend_from_slice(&[0x49, 0xc7, 0xc2, 0x00, 0xf8, 0xff, 0xff]);
        c.extend_from_slice(&[0x4a, 0x8b, 0x04, 0x14, 0x4b, 0x89, 0x84, 0x13, 0x00, 0x08, 0x00, 0x00, 0x49, 0x83, 0xc2, 0x08, 0x75, 0xee]);
        c.extend_from_slice(&[0x49, 0xbb]);
        c.extend_from_slice(&(page.add(0xe8) as u64).to_le_bytes());
        c.extend_from_slice(&[0x49, 0x83, 0x3b, 0x00, 0x74, 0x15, 0x49, 0x8b, 0x43, 0x08, 0x49, 0xc7, 0xc2, 0x00, 0xf8, 0xff, 0xff, 0x4a, 0x89, 0x04, 0x14, 0x49, 0x83, 0xc2, 0x08, 0x75, 0xf6]);
        c.extend_from_slice(&[0x48, 0xb8]);
        c.extend_from_slice(&(target as u64).to_le_bytes());
        c.extend_from_slice(&[0xff, 0xe0]);
        core::ptr::copy_nonoverlapping(c.as_ptr(), code, c.len());
        let s = (h + site) as *mut u8;
        assert_eq!(*s, 0xe8);
        let mut o = 0u32;
        assert!(VirtualProtect(s as *mut _, 5, 0x40, &mut o) != 0);
        let mut old = [0u8; 4];
        core::ptr::copy_nonoverlapping(s.add(1), old.as_mut_ptr(), 4);
        let rel = (code as isize - (s as isize + 5)) as i32;
        core::ptr::copy_nonoverlapping(rel.to_le_bytes().as_ptr(), s.add(1), 4);
        FlushInstructionCache(-1, s as *const _, 5);
        Snap { page, site: h + site, old }
    }
    pub(super) unsafe fn reset(&self) {
        core::ptr::write_bytes(self.page.add(0x100), 0, 8 + SNAP * SLOTS);
    }
    pub(super) unsafe fn fill(&self, v: Option<u8>) {
        core::ptr::write_volatile(self.page.add(0xe8) as *mut u64, v.is_some() as u64);
        core::ptr::write_volatile(self.page.add(0xf0) as *mut u64, u64::from_ne_bytes([v.unwrap_or(0); 8]));
    }
    pub(super) unsafe fn count(&self) -> u64 {
        core::ptr::read_volatile(self.page.add(0x100) as *const u64)
    }
    pub(super) unsafe fn slot(&self, k: usize) -> &[u8] {
        core::slice::from_raw_parts(self.page.add(0x108 + SNAP * k), SNAP)
    }
}

impl Drop for Snap {
    fn drop(&mut self) {
        unsafe {
            let s = self.site as *mut u8;
            core::ptr::copy_nonoverlapping(self.old.as_ptr(), s.add(1), 4);
            FlushInstructionCache(-1, s as *const _, 5);
        }
    }
}

fn hx(b: &[u8]) -> String {
    b.iter().map(|x| format!("{:02x}", x)).collect::<Vec<_>>().join(" ")
}

#[test]
#[ignore]
fn oracle_dcc_cases_snap() {
    let Ok(ids) = std::env::var("TOMBOX_DCC_SNAP") else { return };
    let _g = crate::oracle::SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let o = crate::oracle::oracle().expect("console xg_xs.dll");
    let enc: super::oracle_dec::Enc = unsafe { core::mem::transmute(o.f("XGEncodeDCC")) };
    let site = std::env::var("TOMBOX_DCC_SITE").map(|s| usize::from_str_radix(&s, 16).unwrap()).unwrap_or(0xab3f);
    let tgt = unsafe {
        let s = (o.h + site) as *const u8;
        (s as isize + 5 + core::ptr::read_unaligned(s.add(1) as *const i32) as isize) as usize
    };
    println!("site {:#x} target {:#x}", site, tgt - o.h);
    let sp = unsafe { Snap::new(o.h, site, tgt) };
    for id in ids.split(',').map(|x| x.parse::<u64>().unwrap()) {
        let c = case(id);
        for sm in [0x00u8, 0x77] {
            unsafe { sp.reset() };
            let mut out = vec![0u8; 256 * c.n];
            let mut meta = vec![0u8; c.n];
            let mut sz = vec![0u32; c.n];
            let _ = super::oracle_dec::smear(sm);
            let r = unsafe { enc(c.n as u32, c.src.as_ptr(), out.as_mut_ptr(), meta.as_mut_ptr(), sz.as_mut_ptr(), c.fmt, c.sw, c.dim, c.p.as_ptr()) };
            let n = unsafe { sp.count() } as usize;
            println!("case {} fmt={} sw={} dim={} p={:x?} n={} smear {:02x} hr {:#x} calls {} meta {:02x?}", id, c.fmt, c.sw, c.dim, c.p, c.n, sm, r as u32, n, meta);
            for k in 0..n.min(SLOTS) {
                if std::env::var("TOMBOX_DCC_X88").is_ok() {
                    let s = unsafe { sp.slot(k) };
                    println!("  call{} x88 {}", k, hx(&s[SNAP - 0x88..SNAP - 0x48]));
                    continue;
                }
                let s = unsafe { sp.slot(k) };
                println!("  blk{} relay {}", k, hx(&s[SNAP - 0x208..SNAP - 0x1c8]));
                println!("  blk{} conv  {}", k, hx(&s[SNAP - 0x578..SNAP - 0x538]));
            }
        }
    }
    drop(sp);
}

#[test]
#[ignore]
fn oracle_dcc_cases_fill() {
    let Ok(path) = std::env::var("TOMBOX_DCC_IDS") else { return };
    let ids: Vec<u64> = std::fs::read_to_string(path).unwrap().split_whitespace().map(|x| x.parse().unwrap()).collect();
    let _g = crate::oracle::SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let o = crate::oracle::oracle().expect("console xg_xs.dll");
    let enc: super::oracle_dec::Enc = unsafe { core::mem::transmute(o.f("XGEncodeDCC")) };
    let dec: super::oracle_dec::DecF = unsafe { core::mem::transmute(o.f("XGDecodeDCC")) };
    let esite = std::env::var("TOMBOX_DCC_ESITE").map(|s| usize::from_str_radix(&s, 16).unwrap()).unwrap_or(0xab3f);
    let dsite = std::env::var("TOMBOX_DCC_DSITE").map(|s| usize::from_str_radix(&s, 16).unwrap()).unwrap_or(0);
    let tg = |site: usize| unsafe {
        let s = (o.h + site) as *const u8;
        (s as isize + 5 + core::ptr::read_unaligned(s.add(1) as *const i32) as isize) as usize
    };
    let se = unsafe { Snap::new(o.h, esite, tg(esite)) };
    let sd = if dsite != 0 { Some(unsafe { Snap::new(o.h, dsite, tg(dsite)) }) } else { None };
    let mut tally: std::collections::BTreeMap<String, u64> = std::collections::BTreeMap::new();
    for id in ids {
        let c = case(id);
        if !c.enc && sd.is_none() {
            continue;
        }
        let mut outs = Vec::new();
        for f in [None, Some(0x00u8), Some(0xff), Some(0x5a), Some(0x13)] {
            unsafe {
                se.fill(f);
                if let Some(s) = &sd {
                    s.fill(f);
                }
            }
            let mut out = vec![0x3cu8; 256 * c.n];
            let mut meta = vec![0x3cu8; c.n];
            let mut sz = vec![0u32; c.n];
            let r = unsafe {
                if c.enc {
                    enc(c.n as u32, c.src.as_ptr(), out.as_mut_ptr(), meta.as_mut_ptr(), sz.as_mut_ptr(), c.fmt, c.sw, c.dim, c.p.as_ptr())
                } else {
                    dec(c.n as u32, c.src.as_ptr(), c.meta.as_ptr(), out.as_mut_ptr(), c.fmt, c.sw, c.dim, c.p.as_ptr())
                }
            };
            outs.push((r, out, meta, sz));
        }
        unsafe {
            se.fill(None);
            if let Some(s) = &sd {
                s.fill(None);
            }
        }
        let ch = outs.iter().any(|x| *x != outs[0]);
        *tally.entry(format!("{} changed-by-predriver-fill {}", if c.enc { "E" } else { "D" }, ch)).or_insert(0) += 1;
        if !ch {
            println!("insensitive {} {}", if c.enc { "E" } else { "D" }, id);
        }
    }
    for (k, v) in &tally {
        println!("tally {} {}", k, v);
    }
}

#[test]
#[ignore]
fn oracle_dcc_cases_relay_holes() {
    let Ok(ids) = std::env::var("TOMBOX_DCC_RH") else { return };
    let _g = crate::oracle::SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let o = crate::oracle::oracle().expect("console xg_xs.dll");
    for id in ids.split(',').map(|x| x.parse::<u64>().unwrap()) {
        let c = case(id);
        let con = crate::dcc_oracle::Con::new(&o, c.fmt, c.sw, c.dim, c.p[0], c.p[1]).unwrap();
        let mut s = super::oracle_dec::defined(con.pull());
        s.b[0].copy_from_slice(&c.src[..256]);
        s.w64(0x13c0, 0);
        s.w32(0xf0, 0);
        let mut outs = Vec::new();
        for sm in [0x00u8, 0xff, 0x5a] {
            let _ = super::oracle_dec::smear(sm);
            outs.push(super::oracle_enc::console_stage(&o, &con, &s, super::oracle_enc::St::Lay(0, 1)).b[2]);
        }
        let ch: Vec<usize> = (0..256).filter(|&j| outs.iter().any(|x| x[j] != outs[0][j])).collect();
        let nh = super::enc::relay_holes(&s, 1);
        let mut n = copy(&s);
        let r = super::enc::relay_in(&mut n, 2, 0, 1, &mut super::enc::Tmp::with(Some([0; 64])));
        let nd: Vec<usize> = (0..256).filter(|&j| !nh.contains(&j) && n.b[2][j] != outs[0][j]).collect();
        let mut t = copy(&s);
        for j in 0..256 {
            t.b[0][j] = j as u8;
        }
        let cp = super::oracle_enc::console_stage(&o, &con, &t, super::oracle_enc::St::Lay(0, 1)).b[2];
        let mut np = [0xffffu32; 64];
        for a in 0..64u32 {
            if let Ok(g) = super::px::get(&t, a, 0) {
                if let Ok(b) = super::px::set(&t, &g, 1) {
                    np[(b & 63) as usize] = a;
                }
            }
        }
        let gc: Vec<String> = (0..8u32).map(|a| format!("{:?}", super::px::get(&t, a, 0))).collect();
        println!("case {} fmt={} sw={} dim={} p={:x?} s0={} bpe={} seg {:x} varying {} nholes {} nmis {} {:?}
  console perm {:?}
  native  perm {:?}
  get0 {:?}", id, c.fmt, c.sw, c.dim, c.p, s.r32(0), s.r32(4), s.r32(0x5c), ch.len(), nh.len(), nd.len(), r.is_ok(), &cp[..64], &np[..], gc);
    }
}

#[test]
#[ignore]
fn oracle_dcc_cases_feed() {
    let Ok(ids) = std::env::var("TOMBOX_DCC_FEED") else { return };
    let _g = crate::oracle::SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let o = crate::oracle::oracle().expect("console xg_xs.dll");
    let enc: super::oracle_dec::Enc = unsafe { core::mem::transmute(o.f("XGEncodeDCC")) };
    let pre = unsafe { Snap::new(o.h, 0xab3f, o.h + 0x26220) };
    let rl = unsafe { Snap::new(o.h, 0x262e8, o.h + 0x4ac90) };
    let cv = unsafe { Snap::new(o.h, 0x28c58, o.h + 0x4adc0) };
    for id in ids.split(',').map(|x| x.parse::<u64>().unwrap()) {
        let c = case(id);
        for f in [None, Some(0x00u8), Some(0xff), Some(0x5a)] {
            unsafe {
                pre.fill(f);
                rl.reset();
                cv.reset();
            }
            let mut out = vec![0x3cu8; 256 * c.n];
            let mut meta = vec![0x3cu8; c.n];
            let mut sz = vec![0u32; c.n];
            let r = unsafe { enc(c.n as u32, c.src.as_ptr(), out.as_mut_ptr(), meta.as_mut_ptr(), sz.as_mut_ptr(), c.fmt, c.sw, c.dim, c.p.as_ptr()) };
            let mut d = Dcc::new();
            d.init(c.fmt, c.sw, c.dim, c.p[0], c.p[1]).unwrap();
            let mut nout = Vec::new();
            let mut nmeta = Vec::new();
            let mut ok = true;
            let ncv = unsafe { cv.count() } as usize;
            for k in 0..c.n {
                let mut rf = [0u8; 64];
                rf.copy_from_slice(unsafe { &rl.slot(k)[SNAP - 0x88..SNAP - 0x48] });
                let mut cf = [0u8; 64];
                if ncv > 0 {
                    cf.copy_from_slice(unsafe { &cv.slot(k.min(ncv - 1))[SNAP - 0x88..SNAP - 0x48] });
                }
                d.b[0].copy_from_slice(&c.src[k * 256..k * 256 + 256]);
                if super::enc::run_with(&mut d, Some(rf), Some(cf)).is_err() {
                    ok = false;
                    break;
                }
                nout.extend_from_slice(&d.b[1]);
                nmeta.push(d.r8(0x58));
            }
            let same = ok && nout == out && nmeta == meta;
            println!("feed {} fill {:?} console hr {:#x} meta {:02x?} native ok {} meta {:02x?} same {} relay0 {}", id, f, r as u32, meta, ok, nmeta, same, hx(unsafe { &rl.slot(0)[SNAP - 0x88..SNAP - 0x48] }));
        }
    }
    unsafe { pre.fill(None) };
}

#[test]
#[ignore]
fn oracle_dcc_cases_relay_chain() {
    let Ok(ids) = std::env::var("TOMBOX_DCC_CHAIN") else { return };
    let _g = crate::oracle::SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let o = crate::oracle::oracle().expect("console xg_xs.dll");
    let enc: super::oracle_dec::Enc = unsafe { core::mem::transmute(o.f("XGEncodeDCC")) };
    let pre = unsafe { Snap::new(o.h, 0xab3f, o.h + 0x26220) };
    let sites = [0x262e8usize, 0x26366, 0x263d0];
    let rs: Vec<Snap> = sites.iter().map(|&s| unsafe { Snap::new(o.h, s, o.h + 0x4ac90) }).collect();
    for id in ids.split(',').map(|x| x.parse::<u64>().unwrap()) {
        let c = case(id);
        for f in [Some(0x00u8), Some(0xff)] {
            unsafe {
                pre.fill(f);
                for r in &rs {
                    r.reset();
                }
            }
            let mut out = vec![0x3cu8; 256 * c.n];
            let mut meta = vec![0x3cu8; c.n];
            let mut sz = vec![0u32; c.n];
            unsafe { enc(1, c.src.as_ptr(), out.as_mut_ptr(), meta.as_mut_ptr(), sz.as_mut_ptr(), c.fmt, c.sw, c.dim, c.p.as_ptr()) };
            for (i, r) in rs.iter().enumerate() {
                println!("chain {} fill {:?} relay{} {}", id, f, i + 1, hx(unsafe { &r.slot(0)[SNAP - 0x88..SNAP - 0x48] }));
            }
        }
    }
    unsafe { pre.fill(None) };
}

#[test]
#[ignore]
fn oracle_dcc_cases_feedall() {
    let Ok(path) = std::env::var("TOMBOX_DCC_FEEDALL") else { return };
    let ids: Vec<u64> = std::fs::read_to_string(&path).unwrap().split(|c: char| !c.is_ascii_digit()).filter(|s| !s.is_empty()).map(|s| s.parse().unwrap()).collect();
    let _g = crate::oracle::SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let o = crate::oracle::oracle().expect("console xg_xs.dll");
    let enc: super::oracle_dec::Enc = unsafe { core::mem::transmute(o.f("XGEncodeDCC")) };
    let pre = unsafe { Snap::new(o.h, 0xab3f, o.h + 0x26220) };
    let sites = [0x262e8usize, 0x26366, 0x263d0];
    let rs: Vec<Snap> = sites.iter().map(|&s| unsafe { Snap::new(o.h, s, o.h + 0x4ac90) }).collect();
    let cv = unsafe { Snap::new(o.h, 0x28c58, o.h + 0x4adc0) };
    let mask: u32 = std::env::var("TOMBOX_DCC_RMASK").ok().map(|s| s.parse().unwrap()).unwrap_or(0);
    let mut tot = 0;
    let mut bad = 0;
    for &id in &ids {
        super::enc::RAND.with(|r| r.set((0x9e3779b97f4a7c15 ^ id, mask)));
        let c = case(id);
        if !c.enc {
            continue;
        }
        for f in [None, Some(0x00u8), Some(0xff)] {
            unsafe {
                pre.fill(f);
                for r in &rs {
                    r.reset();
                }
                cv.reset();
            }
            let mut out = vec![0x3cu8; 256 * c.n];
            let mut meta = vec![0x3cu8; c.n];
            let mut sz = vec![0u32; c.n];
            let r = unsafe { enc(c.n as u32, c.src.as_ptr(), out.as_mut_ptr(), meta.as_mut_ptr(), sz.as_mut_ptr(), c.fmt, c.sw, c.dim, c.p.as_ptr()) };
            let cnt: Vec<u64> = rs.iter().map(|r| unsafe { r.count() }).collect();
            let ncv = unsafe { cv.count() } as usize;
            let mut d = Dcc::new();
            d.init(c.fmt, c.sw, c.dim, c.p[0], c.p[1]).unwrap();
            let mut nout = Vec::new();
            let mut nmeta = Vec::new();
            let mut ok = true;
            let mut ci = 0usize;
            let mut used = 0usize;
            for k in 0..c.n {
                let mut rel = [[0u8; 64]; 3];
                for (i, r) in rs.iter().enumerate() {
                    if (k as u64) < cnt[i] {
                        rel[i].copy_from_slice(unsafe { &r.slot(k)[SNAP - 0x88..SNAP - 0x48] });
                    }
                }
                d.b[0].copy_from_slice(&c.src[k * 256..k * 256 + 256]);
                let mut conv = || {
                    let mut b = [0u8; 64];
                    if ci < ncv {
                        b.copy_from_slice(unsafe { &cv.slot(ci)[SNAP - 0x88..SNAP - 0x48] });
                    }
                    ci += 1;
                    b
                };
                if super::enc::run_feed(&mut d, &rel, &mut conv).is_err() {
                    ok = false;
                    break;
                }
                used = ci;
                nout.extend_from_slice(&d.b[1]);
                nmeta.push(d.r8(0x58));
            }
            let same = ok && nout == out && nmeta == meta;
            tot += 1;
            if !same {
                bad += 1;
                println!("feedall MISMATCH {} fmt={} sw={} dim={} p={:x?} n={} fill {:?} hr {:#x} cnt {:?} cv {} used {} ok {} meta {:02x?} nmeta {:02x?} outeq {}", id, c.fmt, c.sw, c.dim, c.p, c.n, f, r as u32, cnt, ncv, used, ok, meta, nmeta, nout == out);
            }
        }
    }
    unsafe { pre.fill(None) };
    println!("feedall total {} mismatch {}", tot, bad);
}

#[test]
#[ignore]
fn oracle_dcc_cases_decfeed() {
    let Ok(path) = std::env::var("TOMBOX_DCC_DECFEED") else { return };
    let ids: Vec<u64> = std::fs::read_to_string(&path).unwrap().split(|c: char| !c.is_ascii_digit()).filter(|s| !s.is_empty()).map(|s| s.parse().unwrap()).collect();
    let _g = crate::oracle::SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let o = crate::oracle::oracle().expect("console xg_xs.dll");
    let cdec: super::oracle_dec::DecF = unsafe { core::mem::transmute(o.f("XGDecodeDCC")) };
    let pre = unsafe { Snap::new(o.h, 0xaece, o.h + 0x23210) };
    let sites = [(0x232beusize, 0x4adc0usize), (0x2334e, 0x4ac90), (0x233cd, 0x4ac90), (0x2351d, 0x4ac90)];
    let ss: Vec<Snap> = sites.iter().map(|&(s, t)| unsafe { Snap::new(o.h, s, o.h + t) }).collect();
    let mut tot = 0;
    let mut bad = 0;
    let mut nerr = 0;
    for &id in &ids {
        let c = case(id);
        if c.enc {
            continue;
        }
        for f in [None, Some(0x00u8), Some(0xff)] {
            unsafe {
                pre.fill(f);
                for s in &ss {
                    s.reset();
                }
            }
            let mut out = vec![0x3cu8; 256 * c.n];
            let r = unsafe { cdec(c.n as u32, c.src.as_ptr(), c.meta.as_ptr(), out.as_mut_ptr(), c.fmt, c.sw, c.dim, c.p.as_ptr()) };
            let cnt: Vec<u64> = ss.iter().map(|s| unsafe { s.count() }).collect();
            let q: [std::collections::VecDeque<[u8; 64]>; 4] = core::array::from_fn(|i| {
                (0..(cnt[i] as usize).min(SLOTS)).map(|k| {
                    let mut b = [0u8; 64];
                    b.copy_from_slice(unsafe { &ss[i].slot(k)[SNAP - 0x88..SNAP - 0x48] });
                    b
                }).collect()
            });
            super::dec::FEED.with(|x| *x.borrow_mut() = Some(q));
            let mut nout = vec![0x3cu8; 256 * c.n];
            let nr = crate::dcc::decode(c.n as u32, &c.src, &c.meta, &mut nout, c.fmt, c.sw, c.dim, c.p[0], c.p[1]);
            super::dec::FEED.with(|x| *x.borrow_mut() = None);
            tot += 1;
            let same = nr == r && nout == out;
            if nr != 0 {
                nerr += 1;
            }
            if !same {
                bad += 1;
                println!("decfeed MISMATCH {} fmt={} sw={} dim={} p={:x?} n={} meta {:02x?} fill {:?} hr {:#x} nhr {:#x} cnt {:?} diff {:?}", id, c.fmt, c.sw, c.dim, c.p, c.n, c.meta, f, r as u32, nr as u32, cnt, (0..out.len()).filter(|&j| out[j] != nout[j]).take(6).collect::<Vec<_>>());
                if std::env::var("TOMBOX_DCC_DUMP").is_ok() {
                    for k in 0..c.n {
                        println!("  blk{} console {}", k, hx(&out[k * 256..k * 256 + 256]));
                        println!("  blk{} native  {}", k, hx(&nout[k * 256..k * 256 + 256]));
                    }
                }
            }
        }
    }
    unsafe { pre.fill(None) };
    println!("decfeed total {} mismatch {} native_err {}", tot, bad, nerr);
}

#[test]
#[ignore]
fn oracle_dcc_cases_holes() {
    let Ok(path) = std::env::var("TOMBOX_DCC_HOLES") else { return };
    let ids: Vec<u64> = std::fs::read_to_string(&path).unwrap().split(|c: char| !c.is_ascii_digit()).filter(|s| !s.is_empty()).map(|s| s.parse().unwrap()).collect();
    let _g = crate::oracle::SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let o = crate::oracle::oracle().expect("console xg_xs.dll");
    let enc: super::oracle_dec::Enc = unsafe { core::mem::transmute(o.f("XGEncodeDCC")) };
    let pre = unsafe { Snap::new(o.h, 0xab3f, o.h + 0x26220) };
    let sites = [0x262e8usize, 0x26366, 0x263d0];
    let rs: Vec<Snap> = sites.iter().map(|&s| unsafe { Snap::new(o.h, s, o.h + 0x4ac90) }).collect();
    let cv = unsafe { Snap::new(o.h, 0x28c58, o.h + 0x4adc0) };
    let mut agg: std::collections::BTreeMap<(usize, usize, usize), std::collections::BTreeMap<String, u32>> = std::collections::BTreeMap::new();
    for &id in &ids {
        let c = case(id);
        if !c.enc {
            continue;
        }
        let mut per: Vec<Vec<(usize, usize, usize, u8)>> = Vec::new();
        for f in [None, Some(0x00u8), Some(0xff)] {
            unsafe {
                pre.fill(f);
                for r in &rs {
                    r.reset();
                }
                cv.reset();
            }
            let mut out = vec![0x3cu8; 256 * c.n];
            let mut meta = vec![0x3cu8; c.n];
            let mut sz = vec![0u32; c.n];
            unsafe { enc(c.n as u32, c.src.as_ptr(), out.as_mut_ptr(), meta.as_mut_ptr(), sz.as_mut_ptr(), c.fmt, c.sw, c.dim, c.p.as_ptr()) };
            let cnt: Vec<u64> = rs.iter().map(|r| unsafe { r.count() }).collect();
            let ncv = unsafe { cv.count() } as usize;
            let mut d = Dcc::new();
            d.init(c.fmt, c.sw, c.dim, c.p[0], c.p[1]).unwrap();
            let mut ci = 0usize;
            let mut obs = Vec::new();
            for k in 0..c.n {
                let mut rel = [[0u8; 64]; 3];
                for (i, r) in rs.iter().enumerate() {
                    if (k as u64) < cnt[i] {
                        rel[i].copy_from_slice(unsafe { &r.slot(k)[SNAP - 0x88..SNAP - 0x48] });
                    }
                }
                d.b[0].copy_from_slice(&c.src[k * 256..k * 256 + 256]);
                let mut cvs: Vec<[u8; 64]> = Vec::new();
                let mut conv = || {
                    let mut b = [0u8; 64];
                    if ci < ncv {
                        b.copy_from_slice(unsafe { &cv.slot(ci)[SNAP - 0x88..SNAP - 0x48] });
                    }
                    ci += 1;
                    cvs.push(b);
                    b
                };
                let mut log = Vec::new();
                let r = super::enc::run_feed_log(&mut d, &rel, &mut conv, &mut log);
                let mut j = 0;
                for (site, hs) in &log {
                    let src = if *site == 3 {
                        j += 1;
                        cvs.get(j - 1).copied().unwrap_or([0; 64])
                    } else {
                        rel[*site]
                    };
                    for &p in hs {
                        obs.push((k, *site, p, src[p]));
                    }
                }
                if r.is_err() {
                    break;
                }
            }
            per.push(obs);
        }
        let mut line = String::new();
        for (i, &(k, site, p, v)) in per[0].iter().enumerate() {
            let stable = per.iter().all(|x| x.get(i).map(|y| y.3) == Some(v));
            let key = format!("{:02x}{}", v, if stable { "" } else { "*" });
            *agg.entry((k.min(1), site, p)).or_default().entry(key.clone()).or_insert(0) += 1;
            line.push_str(&format!(" b{}s{}p{}={}", k, site, p, key));
        }
        println!("holes {} fmt={} sw={} dim={} p={:x?} n={}:{}", id, c.fmt, c.sw, c.dim, c.p, c.n, line);
    }
    unsafe { pre.fill(None) };
    for ((k, site, p), m) in &agg {
        println!("agg blk{} site{} pos{} {:?}", if *k == 0 { "0" } else { "1+" }, site, p, m);
    }
}

#[test]
#[ignore]
fn oracle_dcc_cases_possens() {
    let Ok(path) = std::env::var("TOMBOX_DCC_POSSENS") else { return };
    let ids: Vec<u64> = std::fs::read_to_string(&path).unwrap().split(|c: char| !c.is_ascii_digit()).filter(|s| !s.is_empty()).map(|s| s.parse().unwrap()).collect();
    let _g = crate::oracle::SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let o = crate::oracle::oracle().expect("console xg_xs.dll");
    let enc: super::oracle_dec::Enc = unsafe { core::mem::transmute(o.f("XGEncodeDCC")) };
    let sites = [0x262e8usize, 0x26366, 0x263d0];
    let rs: Vec<Snap> = sites.iter().map(|&s| unsafe { Snap::new(o.h, s, o.h + 0x4ac90) }).collect();
    let cv = unsafe { Snap::new(o.h, 0x28c58, o.h + 0x4adc0) };
    let mut res: std::collections::BTreeMap<(usize, usize), Vec<u64>> = std::collections::BTreeMap::new();
    for &id in &ids {
        let c = case(id);
        if !c.enc {
            continue;
        }
        unsafe {
            for r in &rs {
                r.reset();
            }
            cv.reset();
        }
        let mut out = vec![0x3cu8; 256 * c.n];
        let mut meta = vec![0x3cu8; c.n];
        let mut sz = vec![0u32; c.n];
        unsafe { enc(c.n as u32, c.src.as_ptr(), out.as_mut_ptr(), meta.as_mut_ptr(), sz.as_mut_ptr(), c.fmt, c.sw, c.dim, c.p.as_ptr()) };
        let cnt: Vec<u64> = rs.iter().map(|r| unsafe { r.count() }).collect();
        let ncv = unsafe { cv.count() } as usize;
        let rels: Vec<[[u8; 64]; 3]> = (0..c.n).map(|k| {
            let mut rel = [[0u8; 64]; 3];
            for (i, r) in rs.iter().enumerate() {
                if (k as u64) < cnt[i] {
                    rel[i].copy_from_slice(unsafe { &r.slot(k)[SNAP - 0x88..SNAP - 0x48] });
                }
            }
            rel
        }).collect();
        let cvs: Vec<[u8; 64]> = (0..ncv.min(SLOTS)).map(|i| {
            let mut b = [0u8; 64];
            b.copy_from_slice(unsafe { &cv.slot(i)[SNAP - 0x88..SNAP - 0x48] });
            b
        }).collect();
        let run = || -> bool {
            let mut d = Dcc::new();
            d.init(c.fmt, c.sw, c.dim, c.p[0], c.p[1]).unwrap();
            let mut ci = 0usize;
            let mut nout = Vec::new();
            let mut nmeta = Vec::new();
            for k in 0..c.n {
                d.b[0].copy_from_slice(&c.src[k * 256..k * 256 + 256]);
                let mut conv = || {
                    let b = cvs.get(ci).copied().unwrap_or([0; 64]);
                    ci += 1;
                    b
                };
                if super::enc::run_feed(&mut d, &rels[k], &mut conv).is_err() {
                    return false;
                }
                nout.extend_from_slice(&d.b[1]);
                nmeta.push(d.r8(0x58));
            }
            nout == out && nmeta == meta
        };
        for site in 1..4usize {
            for p in 0..64usize {
                super::enc::POS.with(|q| q.set(Some(p)));
                let mut bad = false;
                for t in 0..4u64 {
                    super::enc::RAND.with(|r| r.set((0x9e3779b97f4a7c15 ^ (id * 977 + t * 31 + p as u64), 1 << site)));
                    if !run() {
                        bad = true;
                        break;
                    }
                }
                if bad {
                    res.entry((site, p)).or_default().push(id);
                }
            }
        }
        super::enc::POS.with(|q| q.set(None));
        super::enc::RAND.with(|r| r.set((0, 0)));
    }
    for ((site, p), v) in &res {
        println!("possens site{} pos{} {} ids {:?}", site, p, v.len(), &v[..v.len().min(8)]);
    }
}

#[test]
#[ignore]
fn oracle_dcc_cases_addr() {
    let Ok(ids) = std::env::var("TOMBOX_DCC_ADDR") else { return };
    let _g = crate::oracle::SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let o = crate::oracle::oracle().expect("console xg_xs.dll");
    let enc: super::oracle_dec::Enc = unsafe { core::mem::transmute(o.f("XGEncodeDCC")) };
    let sites = [0x262e8usize, 0x26366, 0x263d0];
    let rs: Vec<Snap> = sites.iter().map(|&s| unsafe { Snap::new(o.h, s, o.h + 0x4ac90) }).collect();
    let cv = unsafe { Snap::new(o.h, 0x28c58, o.h + 0x4adc0) };
    let pre = unsafe { Snap::new(o.h, 0xab3f, o.h + 0x26220) };
    for id in ids.split(',').map(|x| x.parse::<u64>().unwrap()) {
        let c = case(id);
        for shift in [0usize, 0x40] {
            unsafe {
                for r in &rs {
                    r.reset();
                }
                cv.reset();
                pre.reset();
            }
            let mut sbuf = vec![0u8; c.src.len() + 0x100];
            sbuf[shift..shift + c.src.len()].copy_from_slice(&c.src);
            let mut out = vec![0x3cu8; 256 * c.n + 0x100];
            let mut meta = vec![0x3cu8; c.n + 0x100];
            let mut sz = vec![0u32; c.n + 0x40];
            let marker = 0u64;
            unsafe { enc(c.n as u32, sbuf.as_ptr().add(shift), out.as_mut_ptr().add(shift), meta.as_mut_ptr().add(shift), sz.as_mut_ptr().add(shift / 4), c.fmt, c.sw, c.dim, c.p.as_ptr()) };
            println!("addr {} shift {:#x} h {:#x} src {:#x} out {:#x} meta {:#x} sz {:#x} p {:#x} stack {:#x}", id, shift, o.h, sbuf.as_ptr() as usize + shift, out.as_ptr() as usize + shift, meta.as_ptr() as usize + shift, sz.as_ptr() as usize + shift, c.p.as_ptr() as usize, &marker as *const u64 as usize);
            let s0 = unsafe { pre.slot(0) };
            println!("  pre-driver rsp-0x40.. {}", hx(&s0[SNAP - 0x40..SNAP]));
            for (i, r) in rs.iter().enumerate() {
                println!("  relay{} {}", i + 1, hx(unsafe { &r.slot(0)[SNAP - 0x88..SNAP - 0x48] }));
            }
            println!("  conv0 {}", hx(unsafe { &cv.slot(0)[SNAP - 0x88..SNAP - 0x48] }));
        }
    }
}

pub(super) fn console_case(h: usize, c: &Case) -> (i32, Vec<u8>, Vec<u8>) {
    let f = |n: &str| unsafe {
        let s = format!("{n}\0");
        GetProcAddress(h as *mut _, s.as_ptr())
    };
    let mut out = vec![0x3cu8; 256 * c.n];
    let mut meta = vec![0x3cu8; c.n];
    let mut sz = vec![0u32; c.n];
    let r = if c.enc {
        let enc: super::oracle_dec::Enc = unsafe { core::mem::transmute(f("XGEncodeDCC")) };
        unsafe { enc(c.n as u32, c.src.as_ptr(), out.as_mut_ptr(), meta.as_mut_ptr(), sz.as_mut_ptr(), c.fmt, c.sw, c.dim, c.p.as_ptr()) }
    } else {
        let dec: super::oracle_dec::DecF = unsafe { core::mem::transmute(f("XGDecodeDCC")) };
        meta.copy_from_slice(&c.meta);
        unsafe { dec(c.n as u32, c.src.as_ptr(), c.meta.as_ptr(), out.as_mut_ptr(), c.fmt, c.sw, c.dim, c.p.as_ptr()) }
    };
    (r, out, meta)
}

extern "system" {
    fn GetProcAddress(h: *mut core::ffi::c_void, n: *const u8) -> *mut core::ffi::c_void;
}

#[inline(never)]
fn console_deep(h: usize, c: &Case, depth: usize) -> (i32, Vec<u8>, Vec<u8>) {
    if depth == 0 {
        return console_case(h, c);
    }
    let pad = [0u8; 0x1000];
    let r = console_deep(h, c, depth - 1);
    unsafe { core::ptr::read_volatile(&pad[depth & 0xfff]) };
    r
}

pub(super) fn thread_variants(h: usize, id: u64) -> Vec<(i32, Vec<u8>, Vec<u8>)> {
    let mut v = Vec::new();
    v.push(console_case(h, &case(id)));
    for (depth, ss) in [(0usize, 1usize << 20), (0, 8 << 20), (3, 1 << 20), (16, 1 << 20)] {
        let r = std::thread::Builder::new().stack_size(ss).spawn(move || console_deep(h, &case(id), depth)).unwrap().join().unwrap();
        v.push(r);
    }
    v.push(console_deep(h, &case(id), 5));
    v
}

#[test]
#[ignore]
fn oracle_dcc_cases_thread() {
    let Ok(path) = std::env::var("TOMBOX_DCC_THREAD") else { return };
    let ids: Vec<u64> = std::fs::read_to_string(&path).unwrap().split(|c: char| !c.is_ascii_digit()).filter(|s| !s.is_empty()).map(|s| s.parse().unwrap()).collect();
    let _g = crate::oracle::SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let o = crate::oracle::oracle().expect("console xg_xs.dll");
    let mut moved = Vec::new();
    let mut fixed = Vec::new();
    for &id in &ids {
        let v = thread_variants(o.h, id);
        let k = (1..v.len()).find(|&k| v[k] != v[0]);
        let tag = if case(id).enc { "E" } else { "D" };
        match k {
            Some(k) => moved.push(format!("{}{}@{}", tag, id, k)),
            None => fixed.push(format!("{}{}", tag, id)),
        }
    }
    println!("thread moved {}: {}", moved.len(), moved.join(" "));
    println!("thread fixed {}: {}", fixed.len(), fixed.join(" "));
}

pub(super) struct RspProbe {
    page: *mut u8,
    site: usize,
    old: [u8; 4],
}

impl RspProbe {
    pub(super) unsafe fn new(h: usize, site: usize) -> RspProbe {
        let s = (h + site) as *mut u8;
        assert_eq!(*s, 0xe8);
        let target = (s as isize + 5 + core::ptr::read_unaligned(s.add(1) as *const i32) as isize) as u64;
        let mut page: *mut u8 = core::ptr::null_mut();
        let mut a = (h + 0x0180_0000) & !0xffff;
        while page.is_null() {
            page = VirtualAlloc(a as *mut _, 0x1000, 0x3000, 0x40) as *mut u8;
            a += 0x10000;
        }
        let mut c: Vec<u8> = vec![0x49, 0xbb];
        c.extend_from_slice(&(page.add(0x800) as u64).to_le_bytes());
        c.extend_from_slice(&[0x49, 0x89, 0x23]);
        c.extend_from_slice(&[0x49, 0xbb]);
        c.extend_from_slice(&target.to_le_bytes());
        c.extend_from_slice(&[0x41, 0xff, 0xe3]);
        core::ptr::copy_nonoverlapping(c.as_ptr(), page, c.len());
        let mut o = 0u32;
        assert!(VirtualProtect(s as *mut _, 5, 0x40, &mut o) != 0);
        let mut old = [0u8; 4];
        core::ptr::copy_nonoverlapping(s.add(1), old.as_mut_ptr(), 4);
        let rel = (page as isize - (s as isize + 5)) as i32;
        core::ptr::copy_nonoverlapping(rel.to_le_bytes().as_ptr(), s.add(1), 4);
        FlushInstructionCache(-1, s as *const _, 5);
        RspProbe { page, site: h + site, old }
    }
    pub(super) unsafe fn rsp(&self) -> usize {
        core::ptr::read_volatile(self.page.add(0x800) as *const usize)
    }
}

impl Drop for RspProbe {
    fn drop(&mut self) {
        unsafe {
            let s = self.site as *mut u8;
            core::ptr::copy_nonoverlapping(self.old.as_ptr(), s.add(1), 4);
            FlushInstructionCache(-1, s as *const _, 5);
        }
    }
}

fn find_ptrs(b: &[u8], lo: usize, hi: usize) -> Vec<(usize, usize)> {
    (0..b.len() / 8).filter_map(|i| {
        let v = u64::from_le_bytes(b[i * 8..i * 8 + 8].try_into().unwrap()) as usize;
        (v >= lo && v < hi).then_some((i * 8, v))
    }).collect()
}

#[test]
#[ignore]
fn oracle_dcc_cases_rsp() {
    let Ok(ids) = std::env::var("TOMBOX_DCC_RSP") else { return };
    let _g = crate::oracle::SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let o = crate::oracle::oracle().expect("console xg_xs.dll");
    let pe = unsafe { RspProbe::new(o.h, 0xa97a) };
    let pd = unsafe { RspProbe::new(o.h, 0xacfe) };
    let rs: Vec<Snap> = [0x262e8usize, 0x26366, 0x263d0].iter().map(|&s| unsafe { Snap::new(o.h, s, o.h + 0x4ac90) }).collect();
    let cv = unsafe { Snap::new(o.h, 0x28c58, o.h + 0x4adc0) };
    let dsites = [(0x232beusize, 0x4adc0usize), (0x2334e, 0x4ac90), (0x233cd, 0x4ac90), (0x2351d, 0x4ac90)];
    let ds: Vec<Snap> = dsites.iter().map(|&(s, t)| unsafe { Snap::new(o.h, s, o.h + t) }).collect();
    for id in ids.split(',').map(|x| x.parse::<u64>().unwrap()) {
        let c = case(id);
        for th in [false, true] {
            unsafe {
                for r in rs.iter().chain(ds.iter()).chain(std::iter::once(&cv)) {
                    r.reset();
                }
            }
            let h = o.h;
            let cc = case(id);
            let (r, out, _) = if th { std::thread::spawn(move || console_case(h, &cc)).join().unwrap() } else { console_case(h, &cc) };
            let e = unsafe { if c.enc { pe.rsp() + 0x48 } else { pd.rsp() + 0x30 } };
            println!("rsp {} enc {} thread {} hr {:#x} E {:#x}", id, c.enc, th, r as u32, e);
            let lo = e - 0x10000;
            let hi = e + 0x1000;
            let snaps: Vec<&Snap> = if c.enc { rs.iter().chain(std::iter::once(&cv)).collect() } else { ds.iter().collect() };
            for (i, s) in snaps.iter().enumerate() {
                let n = unsafe { s.count() } as usize;
                for k in 0..n.min(3) {
                    let b = unsafe { &s.slot(k)[SNAP - 0x88..SNAP - 0x48] };
                    let p: Vec<String> = find_ptrs(b, lo, hi).iter().map(|&(o2, v)| format!("@{}=E-{:#x}", o2, e - v)).collect();
                    println!("  site{} call{} stackptrs {:?}", i, k, p);
                }
            }
            for j in 0..out.len() / 8 {
                let v = u64::from_le_bytes(out[j * 8..j * 8 + 8].try_into().unwrap()) as usize;
                if v >= lo && v < hi {
                    println!("  out@{} = E-{:#x}", j * 8, e - v);
                }
            }
        }
    }
}

fn native_case(c: &Case, e: Option<usize>) -> (i32, Vec<u8>, Vec<u8>) {
    crate::dcc::ENTRY.with(|x| x.set(e));
    let mut out = vec![0x3cu8; 256 * c.n];
    let mut meta = vec![0x3cu8; c.n];
    let r = if c.enc {
        crate::dcc::encode(c.n as u32, &c.src, &mut out, &mut meta, None, c.fmt, c.sw, c.dim, c.p[0], c.p[1])
    } else {
        meta.copy_from_slice(&c.meta);
        crate::dcc::decode(c.n as u32, &c.src, &c.meta, &mut out, c.fmt, c.sw, c.dim, c.p[0], c.p[1])
    };
    crate::dcc::ENTRY.with(|x| x.set(None));
    (r, out, meta)
}

#[test]
#[ignore]
fn oracle_dcc_cases_model() {
    let Ok(path) = std::env::var("TOMBOX_DCC_MODEL") else { return };
    let ids: Vec<u64> = std::fs::read_to_string(&path).unwrap().split(|c: char| !c.is_ascii_digit()).filter(|s| !s.is_empty()).map(|s| s.parse().unwrap()).collect();
    let _g = crate::oracle::SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let o = crate::oracle::oracle().expect("console xg_xs.dll");
    let b = build_of(&o);
    println!("model build {}", b.name);
    let pe = unsafe { RspProbe::new(o.h, b.enc_chk) };
    let pd = unsafe { RspProbe::new(o.h, b.dec_chk) };
    let h = o.h;
    let vary: u32 = std::env::var("TOMBOX_DCC_VARY").ok().map(|s| s.parse().unwrap()).unwrap_or(!0);
    let mut stat: std::collections::BTreeMap<&'static str, Vec<String>> = std::collections::BTreeMap::new();
    for &id in &ids {
        let c = case(id);
        for th in [false, true] {
            let cc = case(id);
            let peu = &pe as *const RspProbe as usize;
            let pdu = &pd as *const RspProbe as usize;
            let job = move || {
                let con = console_case(h, &cc);
                let e = unsafe { if cc.enc { (*(peu as *const RspProbe)).rsp() + 0x48 } else { (*(pdu as *const RspProbe)).rsp() + 0x30 } };
                crate::dcc::VARY.with(|v| v.set(vary));
                let nat = native_case(&cc, Some(e));
                (con, nat)
            };
            let (con, nat) = if th { std::thread::spawn(job).join().unwrap() } else { job() };
            let k = if nat == con {
                "match"
            } else if nat.0 != 0 && con.0 == 0 {
                "native-reject"
            } else {
                "MISMATCH"
            };
            stat.entry(k).or_default().push(format!("{}{}{}", if c.enc { "E" } else { "D" }, id, if th { "t" } else { "" }));
        }
    }
    for (k, v) in &stat {
        println!("model {} {}: {}", k, v.len(), v.join(" "));
    }
}

#[test]
#[ignore]
fn oracle_dcc_cases_writer() {
    let Ok(ids) = std::env::var("TOMBOX_DCC_WRITER") else { return };
    let _g = crate::oracle::SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let o = crate::oracle::oracle().expect("console xg_xs.dll");
    let drv = unsafe { Snap::new(o.h, 0xab3f, o.h + 0x26220) };
    for site in [0xaa39usize, 0xaa54] {
        let tgt = unsafe {
            let s = (o.h + site) as *const u8;
            (s as isize + 5 + core::ptr::read_unaligned(s.add(1) as *const i32) as isize) as usize
        };
        let f = unsafe { Snap::new(o.h, site, tgt) };
        for id in ids.split(',').map(|x| x.parse::<u64>().unwrap()) {
            for fv in [None, Some(0x5au8)] {
                unsafe {
                    f.fill(fv);
                    f.reset();
                    drv.reset();
                }
                let _ = console_case(o.h, &case(id));
                println!("writer site {:#x} id {} fill {:?} relaytmp {}", site, id, fv, hx(unsafe { &drv.slot(0)[SNAP - 0x208..SNAP - 0x1c8] }));
            }
        }
        unsafe { f.fill(None) };
    }
}

pub(super) struct Up {
    page: *mut u8,
    site: usize,
    old: [u8; 4],
}

pub(super) const UP: usize = 0x1600;

impl Up {
    pub(super) unsafe fn new(h: usize, site: usize) -> Up {
        let s = (h + site) as *mut u8;
        assert_eq!(*s, 0xe8);
        let target = (s as isize + 5 + core::ptr::read_unaligned(s.add(1) as *const i32) as isize) as u64;
        let mut page: *mut u8 = core::ptr::null_mut();
        let mut a = (h + 0x0200_0000) & !0xffff;
        while page.is_null() {
            page = VirtualAlloc(a as *mut _, 0x4000, 0x3000, 0x40) as *mut u8;
            a += 0x10000;
        }
        let mut c: Vec<u8> = vec![0x56, 0x57, 0x51, 0x48, 0x8d, 0x74, 0x24, 0x20, 0x48, 0xbf];
        c.extend_from_slice(&(page.add(0x1000) as u64).to_le_bytes());
        c.extend_from_slice(&[0x48, 0x89, 0x77, 0xf8, 0xb9]);
        c.extend_from_slice(&(UP as u32).to_le_bytes());
        c.extend_from_slice(&[0xf3, 0xa4, 0x59, 0x5f, 0x5e, 0x48, 0xb8]);
        c.extend_from_slice(&target.to_le_bytes());
        c.extend_from_slice(&[0xff, 0xe0]);
        core::ptr::copy_nonoverlapping(c.as_ptr(), page, c.len());
        let mut o = 0u32;
        assert!(VirtualProtect(s as *mut _, 5, 0x40, &mut o) != 0);
        let mut old = [0u8; 4];
        core::ptr::copy_nonoverlapping(s.add(1), old.as_mut_ptr(), 4);
        let rel = (page as isize - (s as isize + 5)) as i32;
        core::ptr::copy_nonoverlapping(rel.to_le_bytes().as_ptr(), s.add(1), 4);
        FlushInstructionCache(-1, s as *const _, 5);
        Up { page, site: h + site, old }
    }
    pub(super) unsafe fn rsp(&self) -> usize {
        core::ptr::read_volatile(self.page.add(0x1000 - 8) as *const usize)
    }
    pub(super) unsafe fn mem(&self) -> &[u8] {
        core::slice::from_raw_parts(self.page.add(0x1000), UP)
    }
}

impl Drop for Up {
    fn drop(&mut self) {
        unsafe {
            let s = self.site as *mut u8;
            core::ptr::copy_nonoverlapping(self.old.as_ptr(), s.add(1), 4);
            FlushInstructionCache(-1, s as *const _, 5);
        }
    }
}

#[test]
#[ignore]
fn oracle_dcc_cases_decobj() {
    let Ok(ids) = std::env::var("TOMBOX_DCC_DECOBJ") else { return };
    let _g = crate::oracle::SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let o = crate::oracle::oracle().expect("console xg_xs.dll");
    let up = unsafe { Up::new(o.h, 0xaece) };
    let pd = unsafe { RspProbe::new(o.h, 0xacfe) };
    for id in ids.split(',').map(|x| x.parse::<u64>().unwrap()) {
        let c = case(id);
        let _ = console_case(o.h, &c);
        let e = unsafe { pd.rsp() } + 0x30;
        let r = unsafe { up.rsp() };
        let m = unsafe { up.mem() };
        let obj = &m[0x40..0x40 + crate::dcc::OBJ];
        let q = |o2: usize| u64::from_le_bytes(obj[o2..o2 + 8].try_into().unwrap()) as usize;
        println!("decobj {} E {:#x} rsp E-{:#x} obj E-{:#x} buf0 {:#x} buf1 {:#x} buf2 {:#x}", id, e, e - r, e - (r + 0x40), q(0x90), q(0xd0), q(0x118));
        for k in 0..3 {
            let a = q(crate::dcc::SURF[k] + 0x20);
            if a >= r && a + 256 <= r + UP {
                println!("  buf{} at E-{:#x} obj+{:#x}: {}", k, e - a, a - (r + 0x40), hx(&m[a - r..a - r + 256]));
            } else {
                println!("  buf{} at {:#x} outside", k, a);
            }
        }
    }
}

#[test]
#[ignore]
fn oracle_dcc_cases_decres() {
    let Ok(path) = std::env::var("TOMBOX_DCC_DECRES") else { return };
    let ids: Vec<u64> = std::fs::read_to_string(&path).unwrap().split(|c: char| !c.is_ascii_digit()).filter(|s| !s.is_empty()).map(|s| s.parse().unwrap()).collect();
    let _g = crate::oracle::SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let o = crate::oracle::oracle().expect("console xg_xs.dll");
    let pd = unsafe { RspProbe::new(o.h, 0xacfe) };
    let sites = [(0x232beusize, 0x4adc0usize), (0x2334e, 0x4ac90), (0x233cd, 0x4ac90), (0x2351d, 0x4ac90)];
    let ss: Vec<Snap> = sites.iter().map(|&(s, t)| unsafe { Snap::new(o.h, s, o.h + t) }).collect();
    let mut agg: std::collections::BTreeMap<String, u32> = std::collections::BTreeMap::new();
    for &id in &ids {
        let c = case(id);
        if c.enc {
            continue;
        }
        for th in [false, true] {
            unsafe {
                for s in &ss {
                    s.reset();
                }
            }
            let h = o.h;
            let cc = case(id);
            let _ = if th { std::thread::spawn(move || console_case(h, &cc)).join().unwrap() } else { console_case(h, &cc) };
            let e = unsafe { pd.rsp() } + 0x30;
            for (i, s) in ss.iter().enumerate() {
                let n = unsafe { s.count() } as usize;
                for k in 0..n.min(SLOTS) {
                    let b = unsafe { &s.slot(k)[SNAP - 0x88..SNAP - 0x48] };
                    let mut t = String::new();
                    for q in 0..8 {
                        let v = u64::from_le_bytes(b[q * 8..q * 8 + 8].try_into().unwrap()) as usize;
                        if v > e - 0x10000 && v < e + 0x1000 {
                            t.push_str(&format!("[E-{:#x}] ", e - v));
                        } else {
                            t.push_str(&format!("{} ", hx(&b[q * 8..q * 8 + 8]).replace(' ', "")));
                        }
                    }
                    *agg.entry(format!("site{} blk{} {}", i, k.min(1), t)).or_insert(0) += 1;
                }
            }
        }
    }
    let mut v: Vec<_> = agg.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1));
    for (k, n) in v.iter().take(60) {
        println!("decres {:4} {}", n, k);
    }
}

pub(super) struct Build {
    pub(super) name: &'static str,
    pub(super) enc: usize,
    pub(super) enc_chk: usize,
    pub(super) relays: [usize; 3],
    pub(super) relay: usize,
    pub(super) convs: usize,
    pub(super) conv: usize,
    pub(super) dec_chk: usize,
    pub(super) dconv: usize,
    pub(super) drelays: [usize; 3],
}

pub(super) const BUILDS: [Build; 3] = [
    Build { name: "22000 XDefiant/Terraria", enc: 0xa960, enc_chk: 0xa97a, relays: [0x262e8, 0x26366, 0x263d0], relay: 0x4ac90, convs: 0x28c58, conv: 0x4adc0, dec_chk: 0xacfe, dconv: 0x232be, drelays: [0x2334e, 0x233cd, 0x2351d] },
    Build { name: "22621 Trackmania", enc: 0xb7e0, enc_chk: 0xb7fa, relays: [0x29958, 0x299d6, 0x29a40], relay: 0x3e240, convs: 0x2c278, conv: 0x3e370, dec_chk: 0xbb7e, dconv: 0x2f9b6, drelays: [0x2fa46, 0x2fac4, 0x2fc13] },
    Build { name: "19041 Fortnite", enc: 0x70a0, enc_chk: 0x70ba, relays: [0x2bfa9, 0x2c011, 0x2c065], relay: 0x4b440, convs: 0x2e8ba, conv: 0x4b570, dec_chk: 0x740f, dconv: 0x28c9b, drelays: [0x28d15, 0x28d7d, 0x28ec4] },
];

pub(super) fn build_of(o: &crate::oracle::Oracle) -> &'static Build {
    let e = o.f("XGEncodeDCC") as usize - o.h;
    BUILDS.iter().find(|b| b.enc == e).expect("known xg_xs build")
}

fn erel(b: &[u8], e: usize) -> String {
    let mut t = String::new();
    for q in 0..b.len() / 8 {
        let v = u64::from_le_bytes(b[q * 8..q * 8 + 8].try_into().unwrap()) as usize;
        if v > e - 0x10000 && v < e + 0x1000 {
            t.push_str(&format!("[E-{:#x}] ", e - v));
        } else {
            t.push_str(&format!("{} ", hx(&b[q * 8..q * 8 + 8]).replace(' ', "")));
        }
    }
    t
}

#[test]
#[ignore]
fn oracle_dcc_cases_buildres() {
    let Ok(path) = std::env::var("TOMBOX_DCC_BUILDRES") else { return };
    let ids: Vec<u64> = std::fs::read_to_string(&path).unwrap().split(|c: char| !c.is_ascii_digit()).filter(|s| !s.is_empty()).map(|s| s.parse().unwrap()).collect();
    let _g = crate::oracle::SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let o = crate::oracle::oracle().expect("console xg_xs.dll");
    let b = build_of(&o);
    println!("buildres build {}", b.name);
    let pe = unsafe { RspProbe::new(o.h, b.enc_chk) };
    let cv = unsafe { Snap::new(o.h, b.convs, o.h + b.conv) };
    let rs: Vec<Snap> = b.relays.iter().map(|&s| unsafe { Snap::new(o.h, s, o.h + b.relay) }).collect();
    let mut agg: std::collections::BTreeMap<String, u32> = std::collections::BTreeMap::new();
    for &id in &ids {
        let c = case(id);
        if !c.enc {
            continue;
        }
        for th in [false, true] {
            unsafe {
                cv.reset();
                for r in &rs {
                    r.reset();
                }
            }
            let h = o.h;
            let cc = case(id);
            let _ = if th { std::thread::spawn(move || console_case(h, &cc)).join().unwrap() } else { console_case(h, &cc) };
            let e = unsafe { pe.rsp() } + 0x48;
            let n = unsafe { cv.count() } as usize;
            for k in 0..n.min(SLOTS) {
                *agg.entry(format!("conv {}", erel(unsafe { &cv.slot(k)[SNAP - 0x88..SNAP - 0x48] }, e))).or_insert(0) += 1;
            }
            for (i, r) in rs.iter().enumerate() {
                if unsafe { r.count() } > 0 {
                    *agg.entry(format!("relay{} blk0 {}", i + 1, erel(unsafe { &r.slot(0)[SNAP - 0x88..SNAP - 0x48] }, e))).or_insert(0) += 1;
                }
            }
        }
    }
    let mut v: Vec<_> = agg.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1));
    for (k, n) in v.iter().filter(|x| x.0.starts_with("conv")).take(8) {
        println!("buildres {:4} {}", n, k);
    }
}
