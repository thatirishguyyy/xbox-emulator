use super::oracle_cases::{build_of, case, Case, RspProbe};
use crate::oracle::{oracle, SERIAL};

pub(super) type EncF = unsafe extern "system" fn(u32, *const u8, *mut u8, *mut u8, *mut u32, u32, u32, u32, *const u32) -> i32;
pub(super) type DecF = unsafe extern "system" fn(u32, *const u8, *const u8, *mut u8, u32, u32, u32, *const u32) -> i32;

pub(super) struct Out {
    pub(super) hr: i32,
    pub(super) dst: Vec<u8>,
    pub(super) meta: Vec<u8>,
    pub(super) sizes: Vec<u32>,
}

#[inline(never)]
pub(super) fn call_via(dec: usize, enc: usize, c: &Case, fill: u8) -> Out {
    let mut dst = vec![fill; c.n * 256];
    let mut meta = vec![fill; c.n];
    let mut sizes = vec![0xdead_beefu32; c.n];
    let hr = unsafe {
        if c.enc {
            let f: EncF = core::mem::transmute(enc);
            f(c.n as u32, c.src.as_ptr(), dst.as_mut_ptr(), meta.as_mut_ptr(), sizes.as_mut_ptr(), c.fmt, c.sw, c.dim, c.p.as_ptr())
        } else {
            let f: DecF = core::mem::transmute(dec);
            meta.copy_from_slice(&c.meta);
            f(c.n as u32, c.src.as_ptr(), c.meta.as_ptr(), dst.as_mut_ptr(), c.fmt, c.sw, c.dim, c.p.as_ptr())
        }
    };
    Out { hr, dst, meta, sizes }
}

pub(super) fn init_ok(c: &Case) -> bool {
    crate::dcc::args_ok(c.fmt, c.sw) && crate::dcc::Dcc::new().init(c.fmt, c.sw, c.dim, c.p[0], c.p[1]).is_ok()
}

pub(super) fn natives() -> (usize, usize) {
    (crate::XGDecodeDCC as *const () as usize, crate::XGEncodeDCC as *const () as usize)
}

extern "system" {
    fn ExitProcess(code: u32) -> !;
    fn AddVectoredExceptionHandler(first: u32, h: unsafe extern "system" fn(*mut *mut u32) -> i32) -> *mut core::ffi::c_void;
}

pub(super) static VEH_BASE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
pub(super) static CUR_ID: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(usize::MAX);

unsafe extern "system" fn veh(p: *mut *mut u32) -> i32 {
    let rec = *p;
    let code = *rec;
    let addr = *(rec.add(4) as *const usize);
    if code == 0x80000003 || code >= 0xc0000000 {
        let b = VEH_BASE.load(std::sync::atomic::Ordering::SeqCst);
        eprintln!("veh code {code:#x} at {addr:#x} console+{:#x} case {}", addr.wrapping_sub(b), CUR_ID.load(std::sync::atomic::Ordering::SeqCst) as isize);
    }
    0
}

unsafe extern "system" fn veh_exit(p: *mut *mut u32) -> i32 {
    let code = **p;
    if code == 0x80000003 || (code >= 0xc0000000 && code != 0xc0000027) {
        use std::io::Write;
        let _ = writeln!(std::io::stdout(), "@@F {code:08x}");
        let _ = std::io::stdout().flush();
        ExitProcess(code);
    }
    0
}

pub(super) fn arm_exit_on_fault() {
    unsafe { AddVectoredExceptionHandler(1, veh_exit) };
}

pub(super) fn arm_veh(h: usize) {
    VEH_BASE.store(h, std::sync::atomic::Ordering::SeqCst);
    unsafe { AddVectoredExceptionHandler(1, veh) };
}

#[test]
fn oracle_dcc_export_entry_matches_console() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let o = oracle().expect("console xg_xs.dll");
    let b = build_of(&o);
    arm_veh(o.h);
    let pe = unsafe { RspProbe::new(o.h, b.enc_chk) };
    let pd = unsafe { RspProbe::new(o.h, b.dec_chk) };
    let (nd, ne) = natives();
    let (cd, ce) = (o.f("XGDecodeDCC") as usize, o.f("XGEncodeDCC") as usize);
    let mut n = 0;
    for id in 0..200u64 {
        let c = case(id);
        if !init_ok(&c) {
            continue;
        }
        for th in [false, true] {
            let cc = case(id);
            let peu = &pe as *const RspProbe as usize;
            let pdu = &pd as *const RspProbe as usize;
            let job = move || {
                let r = call_via(nd, ne, &cc, 0x3c);
                let nat = crate::dcc::LAST_E.with(|x| x.get());
                if r.hr != 0 {
                    return None;
                }
                let _ = call_via(cd, ce, &cc, 0x3c);
                let con = unsafe { if cc.enc { (*(peu as *const RspProbe)).rsp() + 0x48 } else { (*(pdu as *const RspProbe)).rsp() + 0x30 } };
                Some((con, nat))
            };
            let Some((con, nat)) = (if th { std::thread::spawn(job).join().unwrap() } else { job() }) else { continue };
            assert_eq!(nat, con, "case {id} enc {} thread {th}: native export entry rsp {nat:#x} console {con:#x}", c.enc);
            n += 1;
        }
    }
    println!("dcc export entry ({}): native naked-stub entry rsp equals the console export's entry rsp in {n}/{n} calls (main and worker threads)", b.name);
}

pub(super) fn pfill(f: usize, i: usize) -> u8 {
    let mut x = (i as u64 + 1).wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ (f as u64).wrapping_mul(0xd6e8_feb8_6659_fd93);
    x ^= x >> 29;
    x = x.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    (x >> 32) as u8
}

#[inline(never)]
pub(super) fn smear(pattern: usize) -> u8 {
    let mut a = [0u8; 0x8000];
    for (i, x) in a.iter_mut().enumerate() {
        let v = match pattern {
            0 => 0,
            1 => 0xff,
            _ => pfill(pattern, 0x4000 + i),
        };
        unsafe { core::ptr::write_volatile(x, v) };
    }
    unsafe { core::ptr::read_volatile(&a[0x777]) }
}

extern "system" {
    fn GetProcessHeap() -> *mut core::ffi::c_void;
    fn HeapAlloc(h: *mut core::ffi::c_void, f: u32, n: usize) -> *mut u8;
    fn HeapFree(h: *mut core::ffi::c_void, f: u32, p: *mut u8) -> i32;
}

pub(super) fn spray(pattern: usize) {
    if pattern == 0 {
        return;
    }
    let h = unsafe { GetProcessHeap() };
    let sizes = [0x10usize, 0x100, 0x40, 0x1d0, 0x110, 0x14d0, 0x800];
    let n = 48 + 29 * pattern;
    let ps: Vec<*mut u8> = (0..n).map(|i| unsafe { HeapAlloc(h, 0, sizes[(i + pattern) % sizes.len()]) }).collect();
    for (i, &p) in ps.iter().enumerate() {
        for k in 0..sizes[(i + pattern) % sizes.len()] {
            unsafe { p.add(k).write(pfill(pattern + 7, i * 0x2000 + k)) };
        }
    }
    for (i, &p) in ps.iter().enumerate() {
        if (i * 7 + pattern) % 3 != 0 {
            unsafe { HeapFree(h, 0, p) };
        }
    }
    if pattern >= 3 {
        let mut keep = Vec::new();
        for r in 0..40 * (pattern - 2) {
            for sz in [0x100usize, 0x108, 0x110, 0x120, 0x140, 0x180, 0x200, 0x14d0, 0x1500, 0x800, 0x40, 0x48] {
                let p = unsafe { HeapAlloc(h, 0, sz + r % 3 * 8) };
                unsafe { core::ptr::write_bytes(p, pfill(pattern, r) , sz) };
                keep.push(p);
            }
        }
        for (i, &p) in keep.iter().enumerate() {
            if (i + pattern) % 2 == 0 {
                unsafe { HeapFree(h, 0, p) };
            }
        }
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

pub(super) fn out_hash(c: &Case, r: &Out) -> u64 {
    let mut all = r.dst.clone();
    if c.enc {
        all.extend_from_slice(&r.meta);
        for s in &r.sizes {
            all.extend_from_slice(&s.to_le_bytes());
        }
    }
    h64(&all)
}

#[inline(never)]
fn perturbed_call(dec: usize, enc: usize, c: &Case, p: usize) -> Out {
    spray(p);
    let _ = smear(p);
    call_via(dec, enc, c, 0x3c)
}

pub(super) const PATTERNS: usize = 11;
pub(super) const HOOKED: usize = 6;
pub(super) const HELD: usize = 8;
pub(super) const THREAD: usize = 10;

fn with_held_xmem<R>(n: usize, f: impl FnOnce() -> R) -> R {
    let w: Vec<u16> = "xmem.dll".encode_utf16().chain(Some(0)).collect();
    let h = unsafe { GetModuleHandleW(w.as_ptr()) };
    let g = |nm: &str| unsafe { GetProcAddress(h, format!("{nm}\0").as_ptr()) as usize };
    let a: XAlloc = unsafe { core::mem::transmute(g("XMemAllocDefault")) };
    let fr: XFree = unsafe { core::mem::transmute(g("XMemFreeDefault")) };
    let sizes = [0x4usize, 0x10, 0x100, 0x14d0, 0x10, 0x100, 0x4];
    let held: Vec<*mut u8> = (0..n).map(|i| unsafe { a(sizes[i % sizes.len()], 0x480009e) }).collect();
    for (i, &p) in held.iter().enumerate() {
        unsafe { core::ptr::write_bytes(p, pfill(n + 90, i), sizes[i % sizes.len()]) };
    }
    let r = f();
    for &p in &held {
        unsafe { fr(p, 0x480009e) };
    }
    r
}

fn on_fresh_thread(dec: usize, enc: usize, id: u64) -> Out {
    std::thread::spawn(move || perturbed_call(dec, enc, &case(id), 0)).join().unwrap()
}

pub(super) fn host_residue_sites(b: &super::oracle_cases::Build, c: &Case) -> Vec<(usize, usize, usize)> {
    let m = std::env::var("TOMBOX_DCC_POKE_MASK").ok().and_then(|x| usize::from_str_radix(&x, 16).ok()).unwrap_or(0xff & !super::enc::conv_known() as usize);
    if c.enc { vec![(b.convs, 0x90, m)] } else { vec![] }
}

fn poked_call(o: &crate::oracle::Oracle, c: &Case, p: usize) -> Out {
    let b = build_of(o);
    let sites = host_residue_sites(&b, c);
    let (cd, ce) = (o.f("XGDecodeDCC") as usize, o.f("XGEncodeDCC") as usize);
    let Some(&(site, off, mask)) = sites.first() else { return perturbed_call(cd, ce, c, 0) };
    use std::sync::atomic::Ordering::SeqCst;
    super::watch::POKE[0].store(off, SeqCst);
    super::watch::POKE[1].store(p, SeqCst);
    super::watch::POKE[2].store(mask, SeqCst);
    let w = super::watch::Watch::arm(&[], Some(o.h + site));
    let r = perturbed_call(cd, ce, c, 0);
    drop(w);
    super::watch::POKE[2].store(0, SeqCst);
    r
}

type XAlloc = unsafe extern "system" fn(usize, u64) -> *mut u8;
type XFree = unsafe extern "system" fn(*mut u8, u64);
static XDEF: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static XDEFF: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static HOOK_PAT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

pub(super) static ALOG: std::sync::Mutex<Vec<(bool, usize, usize, u64)>> = std::sync::Mutex::new(Vec::new());
pub(super) static SURFW_ON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub(super) static SURFW_N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
pub(super) static SURFW_OFF: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
pub(super) static SURFW: std::sync::Mutex<Vec<usize>> = std::sync::Mutex::new(Vec::new());
pub(super) static ALOG_ON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[inline(never)]
unsafe extern "system" fn hook_alloc(size: usize, attr: u64) -> *mut u8 {
    if ALOG_ON.load(std::sync::atomic::Ordering::SeqCst) {
        let f: XAlloc = core::mem::transmute(XDEF.load(std::sync::atomic::Ordering::SeqCst));
        let r = f(size, attr);
        if let Ok(mut l) = ALOG.try_lock() {
            l.push((true, r as usize, size, attr));
        }
        return r;
    }
    if SURFW_ON.load(std::sync::atomic::Ordering::SeqCst) {
        let f: XAlloc = core::mem::transmute(XDEF.load(std::sync::atomic::Ordering::SeqCst));
        let r = f(size, attr);
        if size == 0x100 {
            let n = SURFW_N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if n % 2 == 1 {
                let mut l = SURFW.lock().unwrap();
                l.push(r as usize + SURFW_OFF.load(std::sync::atomic::Ordering::SeqCst));
                super::watch::rearm(&l);
            }
        }
        return r;
    }
    let mut pad = [0u8; 0x900];
    let pat = HOOK_PAT.load(std::sync::atomic::Ordering::SeqCst);
    for (i, x) in pad.iter_mut().enumerate() {
        core::ptr::write_volatile(x, pfill(pat + 40, i));
    }
    let f: XAlloc = core::mem::transmute(XDEF.load(std::sync::atomic::Ordering::SeqCst));
    let r = f(size, attr);
    core::ptr::read_volatile(&pad[0x123]);
    r
}

unsafe extern "system" fn hook_free(p: *mut u8, attr: u64) {
    if ALOG_ON.load(std::sync::atomic::Ordering::SeqCst) {
        if let Ok(mut l) = ALOG.try_lock() {
            l.push((false, p as usize, 0, attr));
        }
    }
    let f: XFree = core::mem::transmute(XDEFF.load(std::sync::atomic::Ordering::SeqCst));
    f(p, attr)
}

extern "system" {
    fn GetModuleHandleW(name: *const u16) -> *mut core::ffi::c_void;
}

pub(super) fn with_xmem_hook<R>(pat: usize, f: impl FnOnce() -> R) -> R {
    let w: Vec<u16> = "xmem.dll".encode_utf16().chain(Some(0)).collect();
    let h = unsafe { GetModuleHandleW(w.as_ptr()) };
    assert!(!h.is_null());
    let g = |n: &str| unsafe { GetProcAddress(h, format!("{n}\0").as_ptr()) as usize };
    let (set, def, deff) = (g("XMemSetAllocationHooks"), g("XMemAllocDefault"), g("XMemFreeDefault"));
    assert!(set != 0 && def != 0 && deff != 0);
    XDEF.store(def, std::sync::atomic::Ordering::SeqCst);
    XDEFF.store(deff, std::sync::atomic::Ordering::SeqCst);
    HOOK_PAT.store(pat, std::sync::atomic::Ordering::SeqCst);
    let set: unsafe extern "system" fn(usize, usize) = unsafe { core::mem::transmute(set) };
    unsafe { set(hook_alloc as *const () as usize, hook_free as *const () as usize) };
    let r = f();
    unsafe { set(0, 0) };
    r
}
pub(super) const RELOC: usize = 1;

extern "system" {
    fn LoadLibraryW(name: *const u16) -> *mut core::ffi::c_void;
    fn GetProcAddress(h: *mut core::ffi::c_void, name: *const u8) -> *mut core::ffi::c_void;
}

pub(super) fn relocated_copy() -> (usize, usize, usize) {
    let src = std::env::var("TOMBOX_XG_ORACLE").expect("TOMBOX_XG_ORACLE");
    let len = std::fs::metadata(&src).unwrap().len();
    let dst = std::env::temp_dir().join(format!("tombox_xg_xs_reloc_{len:x}.dll"));
    if std::fs::metadata(&dst).map(|m| m.len()).unwrap_or(0) != len {
        std::fs::copy(&src, &dst).unwrap();
    }
    let w: Vec<u16> = dst.to_string_lossy().encode_utf16().chain(Some(0)).collect();
    let h = unsafe { LoadLibraryW(w.as_ptr()) } as usize;
    assert!(h != 0);
    let f = |n: &str| unsafe { GetProcAddress(h as *mut _, format!("{n}\0").as_ptr()) as usize };
    (h, f("XGDecodeDCC"), f("XGEncodeDCC"))
}

fn child(test: &str, var: &str, spec: String) -> String {
    let log = std::env::temp_dir().join(format!("tombox_xgxs_child_{}_{}.txt", std::process::id(), spec.replace([':', ' '], "_")));
    let mut ch = std::process::Command::new(std::env::current_exe().unwrap())
        .args([test, "--exact", "--test-threads=1", "--nocapture"])
        .env(var, spec)
        .stdout(std::fs::File::create(&log).unwrap())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let t0 = std::time::Instant::now();
    let mut code = None;
    while code.is_none() {
        if let Some(st) = ch.try_wait().unwrap() {
            code = Some(st.code().unwrap_or(0) as u32);
            break;
        }
        if t0.elapsed().as_secs() > 15 {
            let _ = ch.kill();
            let _ = ch.wait();
            code = Some(HANG);
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let mut s = std::fs::read_to_string(&log).unwrap_or_default();
    let _ = std::fs::remove_file(&log);
    s.push_str(&format!("\n@@EXIT {:08x}\n", code.unwrap()));
    s
}

pub(super) const HANG: u32 = 0x4841_4e47;

fn heap_child(spec: &str) {
    let (id, from) = spec.split_once(':').unwrap();
    let (id, from): (u64, usize) = (id.parse().unwrap(), from.parse().unwrap());
    let o = oracle().expect("console xg_xs.dll");
    arm_exit_on_fault();
    let (cd, ce) = (o.f("XGDecodeDCC") as usize, o.f("XGEncodeDCC") as usize);
    let c = case(id);
    use std::io::Write;
    let mut so = std::io::stdout();
    let (rh, rd, re) = relocated_copy();
    assert!(rh != o.h);
    for p in from..PATTERNS {
        writeln!(so, "@@P {p}").unwrap();
        so.flush().unwrap();
        let r = if p == RELOC { perturbed_call(rd, re, &c, 0) } else if p == THREAD { on_fresh_thread(cd, ce, id) } else if p >= HELD { with_held_xmem(if p == HELD { 7 } else { 23 }, || perturbed_call(cd, ce, &c, 0)) } else if p >= HOOKED { with_xmem_hook(p, || perturbed_call(cd, ce, &c, 0)) } else { perturbed_call(cd, ce, &c, p.saturating_sub(1)) };
        writeln!(so, "@@H {p} {:08x} {:016x}", r.hr as u32, out_hash(&c, &r)).unwrap();
        so.flush().unwrap();
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Res {
    Ok(u32, u64, u32),
    Fault(u32),
}

pub(super) fn perturb(id: u64) -> Vec<Res> {
    let mut v: Vec<Res> = Vec::new();
    let mut gen = 0u32;
    while v.len() < PATTERNS {
        let text = child("dcc::oracle_seq::oracle_dcc_rejected_cases_depend_on_process_state", "TOMBOX_DCC_HEAP_CHILD", format!("{id}:{}", v.len()));
        let mut pending = None;
        let mut code = 0u32;
        for l in text.lines().filter_map(|l| l.find("@@").map(|k| &l[k..])) {
            let t: Vec<&str> = l.split(' ').collect();
            match t[0] {
                "@@P" => pending = Some(()),
                "@@H" => {
                    pending = None;
                    v.push(Res::Ok(u32::from_str_radix(t[2], 16).unwrap(), u64::from_str_radix(t[3], 16).unwrap(), gen));
                }
                "@@EXIT" => code = u32::from_str_radix(t[1], 16).unwrap(),
                _ => {}
            }
        }
        if v.len() < PATTERNS {
            assert!(pending.is_some(), "case {id}: heap child made no progress (exit {code:#x})");
            v.push(Res::Fault(code));
            if v.len() == 2 && v[0] == v[1] {
                return v;
            }
        }
        gen += 1;
    }
    v
}

pub(super) fn proven(v: &[Res]) -> Option<&'static str> {
    let faults = v.iter().filter(|r| matches!(r, Res::Fault(_))).count();
    if faults > 0 && faults < v.len() {
        return Some("console faults under some heap/stack states and returns under others");
    }
    if faults == v.len() && v.len() == PATTERNS {
        return v.iter().any(|r| *r != v[0]).then_some("console fault code changes with the heap/stack state");
    }
    let differ = |a: &Res, b: &Res| matches!((a, b), (Res::Ok(ha, xa, ga), Res::Ok(hb, xb, gb)) if ga == gb && (ha != hb || xa != xb));
    let sh: Vec<&Res> = v.iter().enumerate().filter(|(i, _)| *i != RELOC && *i < HOOKED).map(|(_, r)| r).collect();
    for a in &sh {
        for b in &sh {
            if differ(a, b) {
                return Some("console output changes with the stack/heap contents before the call at a fixed entry rsp");
            }
        }
    }
    if v.len() > RELOC && differ(&v[0], &v[RELOC]) {
        return Some("console output changes when the same console DLL is loaded at another base (it writes its own code addresses)");
    }
    if v.len() > HELD && v[HELD..THREAD.min(v.len())].iter().any(|r| differ(&v[0], r)) {
        return Some("console output changes when the title holds other XMem blocks across the call (XMemAllocDefault, the console attribute 0x480009e); it embeds addresses or contents of the title allocator's heap");
    }
    if v.len() > HOOKED && v[HOOKED..HELD.min(v.len())].iter().any(|r| differ(&v[0], r)) {
        return Some("console output changes when the title installs a forwarding XMem allocation hook (XMEM_SET_ALLOCATION_HOOKS); it depends on the title's allocator path");
    }
    None
}

#[test]
fn oracle_dcc_rejected_cases_depend_on_process_state() {
    if let Ok(spec) = std::env::var("TOMBOX_DCC_HEAP_CHILD") {
        heap_child(&spec);
        std::process::exit(0);
    }
    let Ok(ids) = std::env::var("TOMBOX_DCC_PROOF_IDS") else { return };
    let ids: Vec<u64> = std::fs::read_to_string(&ids).unwrap_or(ids).split(|c: char| !c.is_ascii_digit()).filter(|s| !s.is_empty()).map(|s| s.parse().unwrap()).collect();
    let out = std::env::var("TOMBOX_DCC_PROOF_OUT").ok();
    let mut done: std::collections::BTreeMap<u64, String> = Default::default();
    if let Some(p) = &out {
        for l in std::fs::read_to_string(p).unwrap_or_default().lines() {
            if let Some((id, k)) = l.split_once(' ') {
                done.insert(id.parse().unwrap(), k.to_string());
            }
        }
    }
    let mut by: std::collections::BTreeMap<String, Vec<u64>> = Default::default();
    for &id in &ids {
        let k = match done.get(&id) {
            Some(k) => k.clone(),
            None => {
                let v = perturb(id);
                if std::env::var("TOMBOX_DCC_PROOF_RAW").is_ok() {
                    println!("raw {id} {v:x?}");
                }
                let k = match proven(&v) {
                    Some(w) => format!("proven: {w}"),
                    None => format!("stable: {}", match &v[0] { Res::Ok(h, _, _) => format!("hr {h:08x}"), Res::Fault(c) => format!("fault {c:08x}") }),
                };
                if let Some(p) = &out {
                    use std::io::Write;
                    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(p).unwrap();
                    writeln!(f, "{id} {k}").unwrap();
                }
                k
            }
        };
        by.entry(k).or_default().push(id);
    }
    for (k, v) in &by {
        println!("proof {} {k}: {v:?}", v.len());
    }
}

#[test]
#[ignore]
fn dcc_native_reject_ids() {
    let Ok(path) = std::env::var("TOMBOX_DCC_REJECT_OUT") else { return };
    let n: u64 = std::env::var("TOMBOX_DCC_REJECT_N").ok().and_then(|x| x.parse().ok()).unwrap_or(3000);
    let _o = oracle().expect("console xg_xs.dll");
    let (nd, ne) = natives();
    let mut v = Vec::new();
    let (mut init, mut acc) = (0, 0);
    for id in 0..n {
        let c = case(id);
        if !init_ok(&c) {
            init += 1;
            continue;
        }
        let r = call_via(nd, ne, &c, 0x3c);
        if r.hr != 0 {
            v.push(id.to_string());
        } else {
            acc += 1;
        }
    }
    println!("dcc native rejects: {} of {n} ({init} init-invalid skipped, {acc} accepted)", v.len());
    std::fs::write(path, v.join(" ")).unwrap();
}

pub(super) fn native_with(c: &Case, e: usize, vary: u32) -> i32 {
    crate::dcc::VARY.with(|v| v.set(vary));
    let mut out = vec![0u8; 256 * c.n];
    let mut meta = vec![0u8; c.n];
    let r = crate::dcc::with_entry(Some(e), || {
        if c.enc {
            crate::dcc::encode(c.n as u32, &c.src, &mut out, &mut meta, None, c.fmt, c.sw, c.dim, c.p[0], c.p[1])
        } else {
            crate::dcc::decode(c.n as u32, &c.src, &c.meta, &mut out, c.fmt, c.sw, c.dim, c.p[0], c.p[1])
        }
    });
    crate::dcc::VARY.with(|v| v.set(!0));
    r
}

#[test]
#[ignore]
fn dcc_reject_causes() {
    let Ok(ids) = std::env::var("TOMBOX_DCC_CAUSE_IDS") else { return };
    let ids: Vec<u64> = std::fs::read_to_string(&ids).unwrap_or(ids).split(|c: char| !c.is_ascii_digit()).filter(|s| !s.is_empty()).map(|s| s.parse().unwrap()).collect();
    let _o = oracle().expect("console xg_xs.dll");
    let e = 0x7ff0_0000_8000usize;
    let mut by: std::collections::BTreeMap<String, Vec<u64>> = Default::default();
    for &id in &ids {
        let c = case(id);
        let k = if !c.enc {
            format!("D {}", if native_with(&c, e, !0) == 0 { "accepts" } else { "rejects" })
        } else if native_with(&c, e, 0) != 0 {
            "E rejects with every unknown fixed".to_string()
        } else {
            let bits: Vec<String> = (0..4).filter(|&b| native_with(&c, e, 1 << b) != 0).map(|b| ["relay1-entry", "relay2-entry", "relay3-entry", "conv-host-residue"][b].to_string()).collect();
            { let qs: Vec<String> = (0..8).filter(|&q| native_with(&c, e, 1 << (8 + q)) != 0).map(|q| format!("q{q}")).collect(); format!("E needs {} conv quads [{}]", bits.join("+"), qs.join(",")) }
        };
        by.entry(k).or_default().push(id);
    }
    for (k, v) in &by {
        println!("cause {} {k}: {v:?}", v.len());
    }
}

#[test]
#[ignore]
fn dcc_relay_entry_profile() {
    let Ok(ids) = std::env::var("TOMBOX_DCC_PROFILE_IDS") else { return };
    let ids: Vec<u64> = std::fs::read_to_string(&ids).unwrap_or(ids).split(|c: char| !c.is_ascii_digit()).filter(|s| !s.is_empty()).map(|s| s.parse().unwrap()).collect();
    let which: usize = std::env::var("TOMBOX_DCC_PROFILE_RELAY").ok().and_then(|x| x.parse().ok()).unwrap_or(0);
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let o = oracle().expect("console xg_xs.dll");
    let b = build_of(&o);
    arm_veh(o.h);
    let (cd, ce) = (o.f("XGDecodeDCC") as usize, o.f("XGEncodeDCC") as usize);
    let dec = (3..=6).contains(&which);
    let pe = unsafe { RspProbe::new(o.h, if dec { b.dec_chk } else { b.enc_chk }) };
    let (site, tgt) = match which {
        0..=2 => (b.relays[which], b.relay),
        3 => (b.dconv, b.conv),
        4..=6 => (b.drelays[which - 4], b.relay),
        _ => (b.convs, b.conv),
    };
    let sn = unsafe { super::oracle_cases::Snap::new(o.h, site, o.h + tgt) };
    println!("prof h {:#x}", o.h);
    let mut tally: std::collections::BTreeMap<(usize, usize, String), usize> = Default::default();
    let mut stable_ids: std::collections::BTreeMap<usize, usize> = Default::default();
    for &id in &ids {
        let c = case(id);
        if c.enc == dec || !init_ok(&c) {
            continue;
        }
        let mut snaps: Vec<Vec<[u8; 64]>> = Vec::new();
        let mut es = Vec::new();
        for p in 0..4 {
            unsafe { sn.reset() };
            let _ = perturbed_call(cd, ce, &c, p);
            es.push(unsafe { pe.rsp() } + if dec { 0x30 } else { 0x48 });
            let n = (unsafe { sn.count() } as usize).min(super::oracle_cases::SLOTS);
            snaps.push((0..n).map(|k| unsafe { sn.slot(k)[super::oracle_cases::SNAP - 0x88..super::oracle_cases::SNAP - 0x48].try_into().unwrap() }).collect());
        }
        let e = es[0];
        assert!(es.iter().all(|&x| x == e));
        let nb = snaps.iter().map(|s| s.len()).min().unwrap();
        for k in 0..nb.min(3) {
            let mut all = true;
            for q in 0..8 {
                let vals: Vec<u64> = snaps.iter().map(|s| u64::from_le_bytes(s[k][q * 8..q * 8 + 8].try_into().unwrap())).collect();
                for bb in 0..8 {
                    let bv: Vec<u8> = snaps.iter().map(|s| s[k][q * 8 + bb]).collect();
                    let key = if bv.iter().all(|&x| x == bv[0]) {
                        let v = vals[0] as usize;
                        if v > e - 0x10000 && v < e + 0x1000 {
                            format!("E-{:#x}", e - v)
                        } else {
                            format!("{:02x}", bv[0])
                        }
                    } else {
                        all = false;
                        "vary".to_string()
                    };
                    *tally.entry((k, q * 8 + bb, key)).or_insert(0) += 1;
                }
            }
            if all {
                *stable_ids.entry(k).or_insert(0) += 1;
            }
        }
    }
    let mut last = (99, 99);
    let mut line = String::new();
    for ((k, pos, key), n) in &tally {
        if (*k, *pos) != last {
            if !line.is_empty() {
                println!("{line}");
            }
            line = format!("prof relay{} blk{k} byte{pos:2}:", which + 1, );
            last = (*k, *pos);
        }
        line.push_str(&format!(" {key}x{n}"));
    }
    println!("{line}");
    println!("prof fully-stable blocks {:?}", stable_ids);
}

fn model_out(c: &Case, e: usize, rel: &[Vec<[u8; 64]>], k_only: Option<(usize, usize, u8)>) -> Option<Vec<u8>> {
    let mut d = crate::dcc::Dcc::new();
    d.init(c.fmt, c.sw, c.dim, c.p[0], c.p[1]).ok()?;
    let conv = super::enc::conv_seen(Some(e.wrapping_sub(crate::dcc::CONV_FROM_ENTRY)));
    let mut out = Vec::new();
    for i in 0..c.n {
        d.b[0].copy_from_slice(&c.src[i * 256..i * 256 + 256]);
        let mut f = [rel[0][i], rel[1].get(i).copied().unwrap_or([0; 64]), rel[2].get(i).copied().unwrap_or([0; 64])];
        if let Some((blk, pos, x)) = k_only {
            if blk == i {
                f[0][pos] ^= x;
            }
        }
        super::enc::run_model(&mut d, &f, &conv).ok()?;
        out.extend_from_slice(&d.b[1]);
        out.push(d.r8(0x58));
    }
    Some(out)
}

#[test]
#[ignore]
fn dcc_needed_relay1_bytes() {
    let Ok(ids) = std::env::var("TOMBOX_DCC_NEED_IDS") else { return };
    let ids: Vec<u64> = std::fs::read_to_string(&ids).unwrap_or(ids).split(|c: char| !c.is_ascii_digit()).filter(|s| !s.is_empty()).map(|s| s.parse().unwrap()).collect();
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let o = oracle().expect("console xg_xs.dll");
    let b = build_of(&o);
    arm_veh(o.h);
    let (cd, ce) = (o.f("XGDecodeDCC") as usize, o.f("XGEncodeDCC") as usize);
    let pe = unsafe { RspProbe::new(o.h, b.enc_chk) };
    let sn: Vec<super::oracle_cases::Snap> = (0..3).map(|i| unsafe { super::oracle_cases::Snap::new(o.h, b.relays[i], o.h + b.relay) }).collect();
    println!("need h {:#x}", o.h);
    for &id in &ids {
        let c = case(id);
        if !c.enc || !init_ok(&c) {
            continue;
        }
        let mut per: Vec<Vec<Vec<[u8; 64]>>> = Vec::new();
        let mut e = 0;
        let mut con = Vec::new();
        for p in 0..3 {
            for s in &sn {
                unsafe { s.reset() };
            }
            let r = perturbed_call(cd, ce, &c, p);
            e = unsafe { pe.rsp() } + 0x48;
            let mut co = Vec::new();
            for i in 0..c.n {
                co.extend_from_slice(&r.dst[i * 256..i * 256 + 256]);
                co.push(r.meta[i]);
            }
            con.push(co);
            per.push(sn.iter().map(|s| (0..(unsafe { s.count() } as usize).min(super::oracle_cases::SLOTS)).map(|k| unsafe { s.slot(k)[super::oracle_cases::SNAP - 0x88..super::oracle_cases::SNAP - 0x48].try_into().unwrap() }).collect()).collect());
        }
        let base = model_out(&c, e, &per[0], None);
        println!("need {id} native-fed-equals-console {} consoles-equal {}", base.as_ref() == Some(&con[0]), con.iter().all(|x| *x == con[0]));
        for blk in 0..c.n {
            let mut need = Vec::new();
            for pos in 0..64 {
                let a = model_out(&c, e, &per[0], Some((blk, pos, 0x5a)));
                let bb = model_out(&c, e, &per[0], Some((blk, pos, 0xa5)));
                if a != base || bb != base {
                    let vals: Vec<String> = per.iter().map(|p| format!("{:02x}", p[0][blk][pos])).collect();
                    need.push(format!("{pos}={}", vals.join("/")));
                }
            }
            println!("  blk{blk} needs relay1 bytes: {}", need.join(" "));
        }
    }
}

#[test]
#[ignore]
fn dcc_watch_scratch() {
    let Ok(spec) = std::env::var("TOMBOX_DCC_WATCH") else { return };
    let (id, rest) = spec.split_once(':').unwrap();
    let id: u64 = id.parse().unwrap();
    let (site_k, quads) = rest.split_once(':').unwrap();
    let site_k: usize = site_k.parse().unwrap();
    let quads: Vec<usize> = quads.split(',').map(|x| x.parse().unwrap()).collect();
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let o = oracle().expect("console xg_xs.dll");
    let b = build_of(&o);
    let c = case(id);
    let dec = !c.enc;
    let (site, chk) = if dec { ([b.dconv, b.drelays[0], b.drelays[1], b.drelays[2]][site_k], b.dec_chk) } else { ([b.relays[0], b.relays[1], b.relays[2], b.convs][site_k], b.enc_chk) };
    let (cd, ce) = (o.f("XGDecodeDCC") as usize, o.f("XGEncodeDCC") as usize);
    let mut e = 0;
    let mut rs = 0;
    for pass in 0..1 + quads.len().div_ceil(3) {
        if pass == 0 {
            let pe = unsafe { RspProbe::new(o.h, chk) };
            let pr = unsafe { RspProbe::new(o.h, site) };
            let _ = perturbed_call(cd, ce, &c, 0);
            e = unsafe { pe.rsp() } + if dec { 0x30 } else { 0x48 };
            rs = unsafe { pr.rsp() };
            println!("watch {id} site {site:#x} h {:#x} E {e:#x} temp E-{:#x}", o.h, e - (rs - 0x90));
            continue;
        }
        let qs = &quads[(pass - 1) * 3..((pass - 1) * 3 + 3).min(quads.len())];
        let addrs: Vec<usize> = qs.iter().map(|q| rs - 0x90 + 8 * q).collect();
        let w = super::watch::Watch::arm(&addrs, Some(o.h + site));
        let _ = perturbed_call(cd, ce, &c, 0);
        let ev = w.events();
        let stks: Vec<[usize; 64]> = (0..ev.len()).map(|i| w.stack(i)).collect();
        drop(w);
        if std::env::var("TOMBOX_DCC_WATCH_STK").is_ok() {
            let hits: Vec<usize> = ev.iter().enumerate().filter(|(_, e)| e.1 & 8 != 0).map(|(i, _)| i).collect();
            let pick: Vec<usize> = hits.iter().take(2).flat_map(|&h| h.saturating_sub(3)..h).collect();
            for (i, (rip, m)) in ev.iter().enumerate().filter(|(i, _)| pick.contains(i)) {
                let st: Vec<String> = stks[i].iter().enumerate().filter(|(_, &v)| (v >= o.h && v < o.h + 0x100000) || (v > 0x10000_0000 && super::watch::where_is(v).contains('+'))).map(|(k, &v)| format!("[{:#x}]{}", k * 8, if v >= o.h && v < o.h + 0x100000 { format!("con+{:#x}", v - o.h) } else { super::watch::where_is(v) })).collect();
                let regs: Vec<String> = stks[i][..2].iter().map(|v| format!("{v:#x}")).collect();
                println!("watch {id} ev{i} top {}", regs.join(" "));
                println!("watch {id} ev{i} m{m} rip {} stk {}", super::watch::where_is(*rip), st.join(" "));
            }
        }
        let mut blk = 0usize;
        let mut last: Vec<Option<usize>> = vec![None; qs.len()];
        let mut line = Vec::new();
        for (rip, m) in ev {
            if m & 8 != 0 {
                let s: Vec<String> = qs.iter().zip(&last).map(|(q, l)| format!("q{q}<-{}", l.map(|r| if r >= o.h && r < o.h + 0x100000 { format!("con+{:#x}", r - o.h) } else { super::watch::where_is(r) }).unwrap_or("-".into()))).collect();
                line.push(format!("[site hit {blk}: {}]", s.join(" ")));
                blk += 1;
                continue;
            }
            for k in 0..qs.len() {
                if m >> k & 1 != 0 {
                    last[k] = Some(rip);
                }
            }
        }
        println!("watch {id} {}", line.join(" "));
    }
}

fn dec_fed(c: &Case, q: &[Vec<[u8; 64]>; 4], flip: Option<(usize, usize, usize)>) -> (i32, Vec<u8>) {
    let qq: [std::collections::VecDeque<[u8; 64]>; 4] = core::array::from_fn(|i| {
        q[i].iter().enumerate().map(|(k, b)| {
            let mut b = *b;
            if let Some((s, kk, j)) = flip {
                if s == i && kk == k {
                    b[j] ^= 0x5a;
                }
            }
            b
        }).collect()
    });
    super::dec::FEED.with(|x| *x.borrow_mut() = Some(qq));
    let mut nout = vec![0x3cu8; 256 * c.n];
    let nr = crate::dcc::decode(c.n as u32, &c.src, &c.meta, &mut nout, c.fmt, c.sw, c.dim, c.p[0], c.p[1]);
    super::dec::FEED.with(|x| *x.borrow_mut() = None);
    (nr, nout)
}

#[test]
#[ignore]
fn dcc_needed_dec_bytes() {
    let Ok(ids) = std::env::var("TOMBOX_DCC_DNEED_IDS") else { return };
    let ids: Vec<u64> = std::fs::read_to_string(&ids).unwrap_or(ids).split(|c: char| !c.is_ascii_digit()).filter(|s| !s.is_empty()).map(|s| s.parse().unwrap()).collect();
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let o = oracle().expect("console xg_xs.dll");
    let b = build_of(&o);
    arm_veh(o.h);
    let (cd, ce) = (o.f("XGDecodeDCC") as usize, o.f("XGEncodeDCC") as usize);
    let sites = [(b.dconv, b.conv), (b.drelays[0], b.relay), (b.drelays[1], b.relay), (b.drelays[2], b.relay)];
    let ss: Vec<super::oracle_cases::Snap> = sites.iter().map(|&(s, t)| unsafe { super::oracle_cases::Snap::new(o.h, s, o.h + t) }).collect();
    let pd = unsafe { RspProbe::new(o.h, b.dec_chk) };
    for &id in &ids {
        let c = case(id);
        if c.enc || !init_ok(&c) {
            continue;
        }
        let mut qs: Vec<[Vec<[u8; 64]>; 4]> = Vec::new();
        let mut con = Vec::new();
        let mut e = 0;
        for p in 0..3 {
            for s in &ss {
                unsafe { s.reset() };
            }
            let r = perturbed_call(cd, ce, &c, p);
            e = unsafe { pd.rsp() } + 0x30;
            con.push((r.hr, r.dst));
            qs.push(core::array::from_fn(|i| (0..(unsafe { ss[i].count() } as usize).min(super::oracle_cases::SLOTS)).map(|k| unsafe { ss[i].slot(k)[super::oracle_cases::SNAP - 0x88..super::oracle_cases::SNAP - 0x48].try_into().unwrap() }).collect()));
        }
        let base = dec_fed(&c, &qs[0], None);
        let diffs: Vec<usize> = (0..base.1.len()).filter(|&j| base.1[j] != con[0].1[j]).collect();
        println!("dneed {id} fmt {} sw {} dim {} p {:x?} meta {:02x?} native-fed hr {:#x} console hr {:#x} differing bytes {} first {:?} consoles-equal {} counts {:?}", c.fmt, c.sw, c.dim, c.p, c.meta, base.0 as u32, con[0].0 as u32, diffs.len(), &diffs[..diffs.len().min(16)], con.iter().all(|x| *x == con[0]), qs[0].iter().map(|v| v.len()).collect::<Vec<_>>());
        if std::env::var("TOMBOX_DCC_DNEED_DUMP").is_ok() {
            for blk in 0..c.n {
                println!("  blk{blk} console {:02x?}", &con[0].1[blk * 256..blk * 256 + 64]);
                println!("  blk{blk} native  {:02x?}", &base.1[blk * 256..blk * 256 + 64]);
            }
            continue;
        }
        for s in 0..4 {
            for k in 0..qs[0][s].len() {
                let mut need = Vec::new();
                for j in 0..64 {
                    if dec_fed(&c, &qs[0], Some((s, k, j))) != base {
                        let vals: Vec<String> = qs.iter().map(|q| q[s].get(k).map(|x| format!("{:02x}", x[j])).unwrap_or("--".into())).collect();
                        need.push(format!("{j}={}", vals.join("/")));
                    }
                }
                if !need.is_empty() {
                    let quads: Vec<String> = (0..8).map(|q| {
                        let v = u64::from_le_bytes(qs[0][s][k][q * 8..q * 8 + 8].try_into().unwrap()) as usize;
                        if v > e - 0x10000 && v < e + 0x1000 { format!("E-{:#x}", e - v) } else { format!("{v:016x}") }
                    }).collect();
                    println!("  site{s} call{k} needs {} | {}", need.join(" "), quads.join(" "));
                }
            }
        }
    }
}

#[test]
#[ignore]
fn dcc_surface_initial() {
    let Ok(ids) = std::env::var("TOMBOX_DCC_SURF_IDS") else { return };
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let o = oracle().expect("console xg_xs.dll");
    for id in ids.split_whitespace().map(|x| x.parse::<u64>().unwrap()) {
        let c = case(id);
        for p in 0..3 {
            spray(p);
            let k = crate::dcc_oracle::Con::new(&o, c.fmt, c.sw, c.dim, c.p[0], c.p[1]).unwrap();
            for s in 0..3 {
                let b = k.buf(s);
                let nz: Vec<usize> = (0..256).filter(|&i| b[i] != 0).collect();
                println!("surf {id} p{p} s{s} at {:#x} nonzero {} {:02x?}", b.as_ptr() as usize, nz.len(), &b[..32]);
            }
        }
    }
}

#[test]
#[ignore]
fn dcc_watch_surfaces() {
    let Ok(spec) = std::env::var("TOMBOX_DCC_SURFW") else { return };
    let (id, off) = spec.split_once(':').unwrap_or((&spec, "0"));
    let id: u64 = id.parse().unwrap();
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let o = oracle().expect("console xg_xs.dll");
    let (cd, ce) = (o.f("XGDecodeDCC") as usize, o.f("XGEncodeDCC") as usize);
    let c = case(id);
    let w = super::watch::Watch::arm(&[], None);
    SURFW.lock().unwrap().clear();
    SURFW_N.store(0, std::sync::atomic::Ordering::SeqCst);
    SURFW_OFF.store(off.parse().unwrap(), std::sync::atomic::Ordering::SeqCst);
    SURFW_ON.store(true, std::sync::atomic::Ordering::SeqCst);
    let r = with_xmem_hook(0, || call_via(cd, ce, &c, 0x3c));
    SURFW_ON.store(false, std::sync::atomic::Ordering::SeqCst);
    let ev = w.events();
    let w_st: Vec<[usize; 64]> = (0..ev.len()).map(|i| w.stack(i)).collect();
    drop(w);
    let surf = SURFW.lock().unwrap().clone();
    println!("surfw {id} surfaces {surf:x?} hr {:#x} out {:02x?}", r.hr, &r.dst[..r.dst.len().min(16)]);
    for (i, (rip, m)) in ev.iter().enumerate() {
        let v = super::watch::VAL[i].load(std::sync::atomic::Ordering::SeqCst);
        let at = if *rip >= o.h && *rip < o.h + 0x100000 { format!("con+{:#x}", rip - o.h) } else { super::watch::where_is(*rip) };
        let st: Vec<String> = w_st[i].iter().enumerate().filter(|(_, &x)| x >= o.h && x < o.h + 0x100000).map(|(k, &x)| format!("[{:#x}]con+{:#x}", k * 8, x - o.h)).collect();
        println!("surfw {id} ev{i} s{m} {at} val {v:016x} stk {}", st.join(" "));
    }
}

#[test]
#[ignore]
fn dcc_dec_residue_find() {
    let Ok(spec) = std::env::var("TOMBOX_DCC_RFIND") else { return };
    let (id, pat) = spec.split_once(':').unwrap();
    let id: u64 = id.parse().unwrap();
    let pat: Vec<u8> = pat.split(',').map(|x| u8::from_str_radix(x, 16).unwrap()).collect();
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let o = oracle().expect("console xg_xs.dll");
    let b = build_of(&o);
    let (cd, ce) = (o.f("XGDecodeDCC") as usize, o.f("XGEncodeDCC") as usize);
    let c = case(id);
    let sites = [(b.dconv, b.conv), (b.drelays[0], b.relay), (b.drelays[1], b.relay), (b.drelays[2], b.relay)];
    for &m in &c.meta {
        let sg = crate::dcc::bits::seg_info_checked(m as u32);
        println!("rfind {id} meta {m:#x} segs {sg:x?}");
    }
    for (si, &(s, t)) in sites.iter().enumerate() {
        let sn = unsafe { super::oracle_cases::Snap::new(o.h, s, o.h + t) };
        unsafe { sn.reset() };
        let r = perturbed_call(cd, ce, &c, 0);
        let n = (unsafe { sn.count() } as usize).min(super::oracle_cases::SLOTS);
        for k in 0..n {
            let sl = unsafe { sn.slot(k) };
            let hits: Vec<String> = (0..sl.len() - pat.len()).filter(|&j| sl[j..j + pat.len()] == pat[..]).map(|j| format!("rsp-{:#x}", super::oracle_cases::SNAP - j)).collect();
            let t = &sl[super::oracle_cases::SNAP - 0x88..super::oracle_cases::SNAP - 0x48];
            println!("rfind {id} site{si} call{k} hr {:#x} hits {hits:?} temp {:02x?}", r.hr, t);
        }
    }
}

#[test]
#[ignore]
fn dcc_alloc_trace() {
    let Ok(ids) = std::env::var("TOMBOX_DCC_ALLOC_IDS") else { return };
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let o = oracle().expect("console xg_xs.dll");
    let (cd, ce) = (o.f("XGDecodeDCC") as usize, o.f("XGEncodeDCC") as usize);
    for id in ids.split_whitespace().map(|x| x.parse::<u64>().unwrap()) {
        let c = case(id);
        ALOG.lock().unwrap().clear();
        ALOG_ON.store(true, std::sync::atomic::Ordering::SeqCst);
        let _ = with_xmem_hook(0, || call_via(cd, ce, &c, 0x3c));
        ALOG_ON.store(false, std::sync::atomic::Ordering::SeqCst);
        let l = ALOG.lock().unwrap().clone();
        let s: Vec<String> = l.iter().map(|(a, p, n, at)| if *a { format!("A{n:#x}@{p:#x}/{at:#x}") } else { format!("F@{p:#x}") }).collect();
        println!("alloc {id} {}", s.join(" "));
    }
}

fn same_out(c: &Case, a: &Out, b: &Out) -> bool {
    a.hr == b.hr && a.dst == b.dst && (!c.enc || (a.meta == b.meta && a.sizes == b.sizes))
}

#[test]
fn oracle_dcc_accepted_cases_invariant() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let o = oracle().expect("console xg_xs.dll");
    let b = build_of(&o);
    let n: u64 = std::env::var("TOMBOX_DCC_INV_N").ok().and_then(|x| x.parse().ok()).unwrap_or(3000);
    let (nd, ne) = natives();
    let (cd, ce) = (o.f("XGDecodeDCC") as usize, o.f("XGEncodeDCC") as usize);
    let (_rh, rd, re) = relocated_copy();
    arm_veh(o.h);
    let (mut acc, mut runs, mut bad) = (0usize, 0usize, Vec::new());
    for id in 0..n {
        let c = case(id);
        if !init_ok(&c) {
            continue;
        }
        let nat = perturbed_call(nd, ne, &c, 0);
        if nat.hr != 0 {
            continue;
        }
        acc += 1;
        CUR_ID.store(id as usize, std::sync::atomic::Ordering::SeqCst);
        for v in 0..10usize {
            let r = match v {
                5 => perturbed_call(rd, re, &c, 0),
                6 | 7 => with_xmem_hook(v, || perturbed_call(cd, ce, &c, 0)),
                8 | 9 => poked_call(&o, &c, v),
                _ => perturbed_call(cd, ce, &c, v),
            };
            runs += 1;
            if !same_out(&c, &nat, &r) {
                if v == 0 && std::env::var("TOMBOX_DCC_INV_DUMP").is_ok() {
                    for blk in 0..c.n {
                        let (a, b2) = (&nat.dst[blk * 256..blk * 256 + 256], &r.dst[blk * 256..blk * 256 + 256]);
                        let dj: Vec<usize> = (0..256).filter(|&j| a[j] != b2[j]).collect();
                        if !dj.is_empty() {
                            println!("invdump {id} fmt {} meta {:02x?} blk{blk} differ {:?}
  native  {:02x?}
  console {:02x?}", c.fmt, c.meta, &dj[..dj.len().min(24)], &a[dj[0] & !63..(dj[0] & !63) + 64], &b2[dj[0] & !63..(dj[0] & !63) + 64]);
                        }
                    }
                }
                bad.push(format!("{id}@{v}"));
            }
        }
    }
    println!("dcc accepted cases ({}): {acc} of {n} generator cases accepted natively; {runs} console runs under stack smears 00/ff/random, heap sprays, a relocated console DLL, two forwarding XMem allocation hooks and two rewrites of the allocator-dependent convert scratch quads; {} differ from native: {:?}", b.name, bad.len(), &bad[..bad.len().min(40)]);
    assert!(bad.is_empty());
}

fn seq_id(seed: u64, j: u64) -> u64 {
    if seed == 0 { j } else { seed.wrapping_mul(1_000_003).wrapping_add(j) % 1_000_000 }
}

fn seq_child(seed: u64, start: u64, end: u64) {
    let o = oracle().expect("console xg_xs.dll");
    arm_exit_on_fault();
    let (nd, ne) = natives();
    let (cd, ce) = (o.f("XGDecodeDCC") as usize, o.f("XGEncodeDCC") as usize);
    use std::io::Write;
    let mut so = std::io::stdout();
    for j in start..end {
        let c = case(seq_id(seed, j));
        let n = call_via(nd, ne, &c, 0x3c);
        writeln!(so, "@@C {j} {:08x}", n.hr as u32).unwrap();
        so.flush().unwrap();
        let k = call_via(cd, ce, &c, 0x3c);
        writeln!(so, "@@R {j} {:08x} {:016x} {:08x} {:016x}", n.hr as u32, out_hash(&c, &n), k.hr as u32, out_hash(&c, &k)).unwrap();
        so.flush().unwrap();
    }
    writeln!(so, "@@DONE").unwrap();
    so.flush().unwrap();
}

#[test]
fn oracle_dcc_call_sequences() {
    if let Ok(spec) = std::env::var("TOMBOX_DCC_SEQ_CHILD") {
        let v: Vec<u64> = spec.split('_').map(|x| x.parse().unwrap()).collect();
        seq_child(v[0], v[1], v[2]);
        std::process::exit(0);
    }
    let o = oracle().expect("console xg_xs.dll");
    let b = build_of(&o);
    let seqs: u64 = std::env::var("TOMBOX_DCC_SEQS").ok().and_then(|x| x.parse().ok()).unwrap_or(3);
    let len: u64 = std::env::var("TOMBOX_DCC_SEQ_LEN").ok().and_then(|x| x.parse().ok()).unwrap_or(400);
    let v0: u64 = std::env::var("TOMBOX_DCC_SEQ0").ok().and_then(|x| x.parse().ok()).unwrap_or(3000);
    let plan: Vec<(u64, u64)> = std::iter::once((0u64, v0)).chain((1..=seqs).map(|s| (s, len))).collect();
    let (mut calls, mut same, mut rejected, mut crashes, mut hangs, mut init_bad, mut init_crash) = (0usize, 0usize, 0usize, 0usize, 0usize, 0usize, 0usize);
    let mut bad = Vec::new();
    for &(seed, len) in &plan {
        let mut j = 0u64;
        while j < len {
            let text = child("dcc::oracle_seq::oracle_dcc_call_sequences", "TOMBOX_DCC_SEQ_CHILD", format!("{seed}_{j}_{len}"));
            let mut pending: Option<(u64, bool)> = None;
            let mut done = false;
            let mut code = 0u32;
            for l in text.lines().filter_map(|l| l.find("@@").map(|k| &l[k..])) {
                let t: Vec<&str> = l.split(' ').collect();
                match t[0] {
                    "@@C" => pending = Some((t[1].parse().unwrap(), t[2] == "00000000")),
                    "@@R" => {
                        pending = None;
                        calls += 1;
                        let id: u64 = t[1].parse().unwrap();
                        j = id + 1;
                        if !init_ok(&case(seq_id(seed, id))) {
                            init_bad += 1;
                            if t[2] == "00000000" {
                                bad.push(format!("seed {seed} call {id}: native accepted an input whose init native rejects"));
                            }
                        }
                        if t[2] == "00000000" {
                            if t[2] == t[4] && t[3] == t[5] {
                                same += 1;
                            } else {
                                bad.push(format!("seed {seed} call {id}: native {} {} console {} {}", t[2], t[3], t[4], t[5]));
                            }
                        } else {
                            rejected += 1;
                        }
                    }
                    "@@EXIT" => code = u32::from_str_radix(t[1], 16).unwrap(),
                    "@@DONE" => done = true,
                    _ => {}
                }
            }
            if done {
                break;
            }
            let (id, accepted) = pending.unwrap_or_else(|| panic!("seed {seed}: child made no progress from call {j} (exit {code:#x})"));
            calls += 1;
            if accepted {
                bad.push(format!("seed {seed} call {id}: native S_OK but the console {} ({code:#x})", if code == HANG { "hung" } else { "crashed" }));
            } else {
                rejected += 1;
            }
            if !init_ok(&case(seq_id(seed, id))) {
                init_bad += 1;
                init_crash += 1;
            }
            if code == HANG { hangs += 1 } else { crashes += 1 }
            j = id + 1;
        }
    }
    for x in &bad[..bad.len().min(20)] {
        println!("sequence divergence: {x}");
    }
    println!("xg_xs dcc call sequences ({}): case order 0..{v0} plus {seqs} random sequences of {len} mixed XGDecodeDCC/XGEncodeDCC calls in one process with no reset between calls; {calls} calls: {same} native S_OK equal to the console (hr, data, meta, sizes), {rejected} native E_INVALIDARG, of which {crashes} console crashes and {hangs} hangs (process restarted); {init_bad} calls with init-invalid arguments, {init_crash} of them console crashes; {} divergences", b.name, bad.len());
    assert!(bad.is_empty());
}
