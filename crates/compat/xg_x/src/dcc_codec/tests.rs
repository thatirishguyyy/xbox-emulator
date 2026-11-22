use super::addr::{channels_1b4d0, coord_1b540, index_1c960, mode_1b440, reorder_1dd10};
use super::bits::{insert_bits, key_decode, key_overflows, remove_bits, sector_compressed};
use super::Dcc;
use crate::oracle::{oracle, Oracle};
use core::ffi::c_void;

pub(super) struct Rng(pub(super) u64);

impl Rng {
    pub(super) fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 16) as u32
    }
}

extern "system" {
    fn LoadLibraryW(p: *const u16) -> *mut c_void;
    fn GetProcAddress(h: *mut c_void, n: *const u8) -> *mut c_void;
}

#[derive(Clone, Copy)]
pub(super) struct Tm(pub(super) usize, pub(super) usize, pub(super) &'static str);

impl Tm {
    pub(super) fn rva(&self, rva: usize) -> usize {
        self.0 + rva
    }

    pub(super) fn sc(&self) -> usize {
        self.0 + self.1
    }

    pub(super) fn f(&self, name: &str) -> usize {
        let n = format!("{name}\0");
        let p = unsafe { GetProcAddress(self.0 as *mut c_void, n.as_ptr()) } as usize;
        assert!(p != 0, "{name}");
        p
    }

    pub(super) fn hardware_version(&self, or: &Oracle, v: i32) {
        or.hardware_version(v);
        let f: extern "system" fn(i32) = unsafe { core::mem::transmute(self.f("XGSetHardwareVersion")) };
        f(v);
    }
}

pub(super) const BUILDS: [(&str, usize); 3] = [("Trackmania", 0x2d4d0), ("XDefiant", 0x2c4d0), ("Fortnite", 0x2c4c0)];

pub(super) fn build_path(title: &str) -> std::path::PathBuf {
    let ex = std::path::Path::new(r"D:\xbox-pc-port\games").join(title).join("extracted");
    let tail = ["EmbeddedXvd", "Windows", "System32", "xg_x.dll"];
    let mut cands = vec![tail.iter().fold(ex.clone(), |p, s| p.join(s))];
    if let Ok(rd) = std::fs::read_dir(&ex) {
        for e in rd.flatten() {
            cands.push(tail.iter().fold(e.path(), |p, s| p.join(s)));
        }
    }
    cands.into_iter().find(|p| p.is_file()).unwrap_or_else(|| panic!("console xg_x.dll of {title} not found under {}; the DCC oracles need it", ex.display()))
}

pub(super) fn console() -> Option<(Oracle, Tm)> {
    let or = oracle().expect("console oracle xg_x.dll or xmem.dll missing; the DCC oracles need them");
    let title = std::env::var("XGX_DCC_BUILD").unwrap_or_else(|_| "Trackmania".into());
    let &(name, sc) = BUILDS.iter().find(|b| b.0 == title).unwrap_or_else(|| panic!("unknown XGX_DCC_BUILD {title}"));
    let dll = build_path(name);
    let w: Vec<u16> = dll.to_string_lossy().encode_utf16().chain(Some(0)).collect();
    let h = unsafe { LoadLibraryW(w.as_ptr()) } as usize;
    assert!(h != 0, "LoadLibrary failed for {}", dll.display());
    assert!(h != or.0 as usize, "{} resolved to the oracle module already loaded; the DCC oracles need the {name} build itself", dll.display());
    let tm = Tm(h, sc, name);
    let f: extern "system" fn(i32) = unsafe { core::mem::transmute(tm.f("XGSetHardwareVersion")) };
    f(0);
    Some((or, tm))
}

pub(super) struct Con<'a> {
    pub(super) or: &'a Tm,
    pub(super) buf: Vec<u64>,
}

impl<'a> Con<'a> {
    pub(super) fn new(or: &'a Tm, format: i32, tile: i32, params: [u32; 2]) -> (Con<'a>, i32) {
        let c = Con { or, buf: vec![0u64; 0x140] };
        let ctor: extern "system" fn(*mut c_void) -> *mut c_void = unsafe { core::mem::transmute(or.rva(0x1aff0)) };
        let setup: extern "system" fn(*mut c_void, i32, i32, *const u32) -> i32 = unsafe { core::mem::transmute(or.rva(0x5890)) };
        ctor(c.ptr());
        let hr = setup(c.ptr(), format, tile, params.as_ptr());
        (c, hr)
    }

    pub(super) fn ptr(&self) -> *mut c_void {
        self.buf.as_ptr() as *mut c_void
    }

    pub(super) fn image(&self) -> [u8; 0x420] {
        let mut o = [0u8; 0x420];
        unsafe { core::ptr::copy_nonoverlapping(self.ptr() as *const u8, o.as_mut_ptr(), 0x420) };
        o
    }

    pub(super) fn put(&self, off: usize, v: u32) {
        unsafe { (self.ptr() as *mut u8).add(off).cast::<u32>().write_unaligned(v) };
    }

    pub(super) fn surf(&self, i: usize) -> *mut u8 {
        let off = [0x60, 0x98, 0xd8][i];
        unsafe { (self.ptr() as *const u8).add(off).cast::<*mut u8>().read_unaligned() }
    }

    pub(super) fn get_surf(&self, i: usize) -> [u8; 256] {
        let mut s = [0u8; 256];
        unsafe { core::ptr::copy_nonoverlapping(self.surf(i), s.as_mut_ptr(), 256) };
        s
    }

    pub(super) fn set_surf(&self, i: usize, s: &[u8; 256]) {
        unsafe { core::ptr::copy_nonoverlapping(s.as_ptr(), self.surf(i), 256) };
    }

    pub(super) fn load(&self, d: &Dcc) {
        for off in (0..0x40).chain(0xb0..0xb4).chain(0xf0..0x418).step_by(4) {
            self.put(off, d.g(off));
        }
        for i in 0..3 {
            self.set_surf(i, &d.s[i]);
        }
        let s = d.sc;
        unsafe { core::ptr::copy_nonoverlapping(s.as_ptr(), self.or.sc() as *mut u8, 64) };
    }

    pub(super) fn f(&self, rva: usize) -> usize {
        self.or.rva(rva)
    }
}

pub(super) fn same(d: &Dcc, c: &Con, what: &str) {
    same_object(d, c, what);
    for i in 0..3 {
        assert_eq!(d.s[i], c.get_surf(i), "{what}: surface {i}");
    }
    let mut cs = [0u8; 64];
    unsafe { core::ptr::copy_nonoverlapping(c.or.sc() as *const u8, cs.as_mut_ptr(), 64) };
    assert_eq!(d.sc, cs, "{what}: scratch");
}

pub(super) fn same_object(d: &Dcc, c: &Con, what: &str) {
    let img = c.image();
    for off in (0..0x40).chain(0xb0..0xb4).chain(0xf0..0x418) {
        assert_eq!(d.o[off], img[off], "{what}: object byte {off:#x}");
    }
    let cp = u64::from_le_bytes(img[0x418..0x420].try_into().unwrap());
    let dp = u64::from_le_bytes(d.o[0x418..0x420].try_into().unwrap());
    assert_eq!(cp != 0, dp != 0, "{what}: error pointer");
}

pub(crate) fn ours(format: i32, tile: i32, params: [u32; 2]) -> (Dcc, i32) {
    let mut d = Dcc::new();
    let hr = d.setup(format, tile, params[0], params[1]);
    (d, hr)
}

pub(super) fn configs() -> Vec<(i32, i32, [u32; 2])> {
    let mut v = Vec::new();
    for format in 1..=0x78 {
        for tile in 0..=0x1a {
            if ours(format, tile, [0, 0]).1 == 0 {
                v.push((format, tile, [0, 0]));
            }
        }
    }
    v
}

#[test]
fn setup_matches_console_for_every_format_and_tile_mode() {
    let Some((orc, ref or)) = console() else { return };
    let mut r = Rng(0x1234_5678_9abc);
    for v in [0, 2] {
        or.hardware_version(&orc, v);
        for format in 1..=0x78 {
            for tile in -1..=0x1b {
                let params = [r.next() % 4, r.next() % 3];
                let (c, chr) = Con::new(or, format, tile, params);
                let (d, dhr) = ours(format, tile, params);
                assert_eq!(chr, dhr, "setup hr format {format} tile {tile}");
                same_object(&d, &c, &format!("setup {format} {tile}"));
            }
        }
    }
}

#[test]
fn bit_helpers_match_console() {
    let Some((_orc, ref or)) = console() else { return };
    let rm: extern "system" fn(u32, u32, u32) -> u32 = unsafe { core::mem::transmute(or.rva(0x16950)) };
    let ins: extern "system" fn(u32, u32, u32, u32) -> u32 = unsafe { core::mem::transmute(or.rva(0x188f0)) };
    let mk: extern "system" fn(u32) -> u32 = unsafe { core::mem::transmute(or.rva(0x16930)) };
    let mut r = Rng(77);
    for n in 0..64 {
        assert_eq!(mk(n), super::bits::mask(n), "mask {n}");
    }
    for _ in 0..200_000 {
        let (v, i) = (r.next(), r.next());
        let pos = (r.next() % 40).wrapping_sub(3);
        let w = (r.next() % 40).wrapping_sub(3);
        assert_eq!(rm(v, pos, w), remove_bits(v, pos, w), "16950 {v:#x} {pos} {w}");
        assert_eq!(ins(v, i, pos, w), insert_bits(v, i, pos, w), "188f0 {v:#x} {i:#x} {pos} {w}");
    }
}

#[test]
fn key_decode_matches_console() {
    let Some((_orc, ref or)) = console() else { return };
    let kd: extern "system" fn(*const u32, *mut u32) -> *mut u32 = unsafe { core::mem::transmute(or.rva(0x1de40)) };
    let sc: extern "system" fn(*const u32, u32) -> u8 = unsafe { core::mem::transmute(or.rva(0x1e020)) };
    let mut r = Rng(99);
    let mut keys: Vec<u32> = (0..256).collect();
    keys.extend((0..20000).map(|_| r.next()));
    let mut tested = 0;
    for key in keys {
        if key_overflows(key) {
            continue;
        }
        tested += 1;
        let mut out = [0x5555_5555u32; 8];
        kd(&key, out.as_mut_ptr());
        assert_eq!(out[..], key_decode(key)[..8], "1de40 key {key:#x}");
        for off in 0..256 {
            assert_eq!(sc(&key, off) != 0, sector_compressed(key, off), "1e020 key {key:#x} off {off}");
        }
    }
    assert!(tested > 10000);
}

#[test]
fn channel_and_mode_helpers_match_console() {
    let Some((_orc, ref or)) = console() else { return };
    let (c, _) = Con::new(or, 28, 14, [0, 0]);
    let (mut d, _) = ours(28, 14, [0, 0]);
    let m4d0: extern "system" fn(*mut c_void, *mut c_void) -> u32 = unsafe { core::mem::transmute(or.rva(0x1b4d0)) };
    let m440: extern "system" fn(*mut c_void, *mut c_void) -> u32 = unsafe { core::mem::transmute(or.rva(0x1b440)) };
    let mut r = Rng(5);
    for _ in 0..50_000 {
        for off in [0x08, 0x10, 0x24, 0x28] {
            let v = match r.next() % 3 {
                0 => r.next() % 16,
                1 => r.next() % 4,
                _ => r.next(),
            };
            d.p(off, v);
            c.put(off, v);
        }
        let p = unsafe { (c.ptr() as *mut u8).add(0x14) } as *mut c_void;
        assert_eq!(m4d0(c.ptr(), p), channels_1b4d0(&d, 0x14));
        assert_eq!(m440(c.ptr(), p), mode_1b440(&d, 0x14));
    }
}

fn unique_configs(or: &Tm) -> Vec<(i32, i32, [u32; 2])> {
    let mut seen = std::collections::HashSet::new();
    let mut v = Vec::new();
    for (f, t, _) in configs() {
        for bs in [0u32, 1] {
            let (c, _) = Con::new(or, f, t, [bs, 0]);
            let img = c.image();
            let key: Vec<u8> = (0..0x40).chain(0xb0..0xb4).chain(0xf0..0x418).map(|i| img[i]).collect();
            if seen.insert(key) {
                v.push((f, t, [bs, 0]));
            }
        }
    }
    v
}

#[test]
fn coordinate_mappers_match_console_exhaustively() {
    let Some((_orc, ref or)) = console() else { return };
    let co: extern "system" fn(*mut c_void, *mut u32, u32, u32) -> *mut u32 = unsafe { core::mem::transmute(or.rva(0x1b540)) };
    let ix: extern "system" fn(*mut c_void, *const u32, u32) -> u32 = unsafe { core::mem::transmute(or.rva(0x1c960)) };
    let mut r = Rng(31337);
    let cfgs = unique_configs(&or);
    assert!(cfgs.len() > 10);
    for (f, t, p) in cfgs {
        let (c, _) = Con::new(or, f, t, p);
        let (mut d, _) = ours(f, t, p);
        for mode in [None, Some(3u32)] {
            if let Some(m) = mode {
                d.p(0, m);
                c.put(0, m);
            }
            for ord in 0..7u32 {
                for idx in 0..256u32 {
                    let mut out = [0xaaaa_aaaau32; 5];
                    co(c.ptr(), out.as_mut_ptr(), idx, ord);
                    let mine = coord_1b540(&d, idx, ord);
                    assert_eq!(out, mine, "1b540 fmt {f} tile {t} ord {ord} idx {idx}");
                    for o2 in 0..7u32 {
                        assert_eq!(ix(c.ptr(), out.as_ptr(), o2), index_1c960(&d, &mine, o2), "1c960 fmt {f} tile {t} ord {ord}->{o2} idx {idx}");
                    }
                    let rc = [r.next() % 64, r.next() % 64, r.next() % 8, r.next() % 32, 0];
                    assert_eq!(ix(c.ptr(), rc.as_ptr(), ord), index_1c960(&d, &rc, ord), "1c960 random fmt {f} tile {t} ord {ord} {rc:?}");
                }
            }
        }
    }
}

#[test]
fn reorder_matches_console_including_shared_scratch() {
    let Some((_orc, ref or)) = console() else { return };
    let ro: extern "system" fn(*mut c_void, *mut u8, *const u8, u32) = unsafe { core::mem::transmute(or.rva(0x1dd10)) };
    let mut r = Rng(4242);
    for (f, t, p) in unique_configs(&or) {
        let (c, _) = Con::new(or, f, t, p);
        let (mut d, _) = ours(f, t, p);
        for from in 0..6u32 {
            for to in 0..6u32 {
                d.p(0xb0, from);
                let mask = r.next() % 16;
                d.p(0x38, mask);
                for i in 0..3 {
                    for b in d.s[i].iter_mut() {
                        *b = r.next() as u8;
                    }
                }
                c.load(&d);
                let (dst, src) = ((r.next() % 3) as usize, (r.next() % 3) as usize);
                ro(c.ptr(), c.surf(dst), c.surf(src), to);
                reorder_1dd10(&mut d, dst, src, to);
                same(&d, &c, &format!("1dd10 fmt {f} tile {t} {from}->{to}"));
            }
        }
    }
}
