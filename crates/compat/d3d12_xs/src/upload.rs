//! Texel transfer for placed (unified-memory) textures.
//!
//! On console a texture created with `CreatePlacedResourceX` lives at a title-chosen
//! address and the title fills it by writing memory in the console's tiled layout; there is
//! no API call when texels change. This module mirrors that memory into the PC texture:
//!
//!   * layout: the *real* `xg_xs.dll` shipped with the title (it runs natively) is asked for
//!     a texture address computer for the title's own resource desc. Verified with
//!     `tools/dev/xgprobe.rs`: vtable slot 2 = GetResourceLayout, 3 = size, 6 =
//!     GetTexelElementOffsetBytes(plane, mip, x: u64, y, zOrSlice, sample, u32* bitPos),
//!     8 = CopyIntoSubresource; slot 6 offsets matched slot 8's output for every element of
//!     five texture shapes (BC1/BC3/BGRA8/R8, 2D + cube, layout 261). Slot 9
//!     (CopyFromSubresource) refuses to run off-console (needs an exclusive-lock object),
//!     so de-tiling is done here from the slot-6 element offsets.
//!   * change detection: each `PresentX` hashes the title memory of tracked subresources
//!     (never-uploaded ones first, the rest round-robin under a byte budget) and re-uploads
//!     the ones whose hash changed.
//!   * lifetime: the registry holds one reference per placed texture; when it is the only
//!     one left the title has released the texture and tracking stops.
//!
//! Assumptions / limits (see docs/TECHNICAL_DEBT.md):
//!   * the PC texture is assumed to be in its creation state when the upload list runs
//!     (the title's own barriers are not tracked);
//!   * memory that is not committed/readable is skipped and retried later;
//!   * a write racing the read is repaired on a later frame (hash changes again), but a
//!     decommit racing the read would fault;
//!   * Texture3D, multi-plane and render-target/depth placed textures are not mirrored.

use core::ffi::c_void;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use compat_common::record;
use crate::placed::Placed;

extern "system" {
    fn VirtualQuery(addr: *const c_void, mbi: *mut Mbi, len: usize) -> usize;
    fn GetModuleHandleA(n: *const u8) -> *mut c_void;
    fn LoadLibraryA(n: *const u8) -> *mut c_void;
    fn GetProcAddress(h: *mut c_void, n: *const u8) -> *mut c_void;
}

#[repr(C)]
struct Mbi {
    base: usize,
    alloc_base: usize,
    alloc_protect: u32,
    partition: u32,
    region_size: usize,
    state: u32,
    protect: u32,
    ty: u32,
    _pad: u32,
}

#[repr(C)]
struct Guid(u32, u16, u16, [u8; 8]);
const IID_DEVICE: Guid = Guid(0x189819f1, 0x1db6, 0x4b57, [0xbe, 0x54, 0x18, 0x21, 0x33, 0x9b, 0x85, 0xf7]);
const IID_ALLOCATOR: Guid = Guid(0x6102dee4, 0xaf59, 0x4b09, [0xb9, 0x99, 0xb4, 0x4d, 0x73, 0xf0, 0x9b, 0x24]);
const IID_GFX_LIST: Guid = Guid(0x5b160d0f, 0xac1b, 0x4185, [0x8b, 0xa8, 0xb3, 0xae, 0x42, 0xa5, 0xa4, 0x55]);
const IID_FENCE: Guid = Guid(0x0a753dcf, 0xc4d8, 0x4b91, [0xad, 0xf6, 0xbe, 0x5a, 0x60, 0xd9, 0x5a, 0x76]);
const IID_RESOURCE: Guid = Guid(0x696442be, 0xa72e, 0x4059, [0xbc, 0x79, 0x5b, 0x5c, 0x98, 0x04, 0x0f, 0xad]);

unsafe fn vt(obj: *mut c_void) -> *const usize {
    *(obj as *const *const usize)
}
macro_rules! vcall {
    ($obj:expr, $slot:expr, fn($($at:ty),*) -> $rt:ty $(, $arg:expr)*) => {{
        let f: unsafe extern "system" fn(*mut c_void $(, $at)*) -> $rt =
            core::mem::transmute(*vt($obj).add($slot));
        f($obj $(, $arg)*)
    }};
}

struct Scan {
    ok: Vec<(usize, usize)>,
    bad: Option<(usize, u32, u32, u32, usize, usize)>,
    bads: Vec<(usize, usize, u32, u32, u32, usize, usize)>,
}

fn scan(addr: usize, len: usize) -> Scan {
    let end = addr + len;
    let mut p = addr;
    let mut s = Scan { ok: Vec::new(), bad: None, bads: Vec::new() };
    while p < end {
        let mut m: Mbi = unsafe { core::mem::zeroed() };
        if unsafe { VirtualQuery(p as *const c_void, &mut m, core::mem::size_of::<Mbi>()) } == 0 {
            if s.bad.is_none() {
                s.bad = Some((p, 0, 0, 0, 0, 0));
            }
            s.bads.push((p, end, 0, 0, 0, 0, 0));
            break;
        }
        let next = (m.base + m.region_size).min(end).max(p + 1);
        let prot = m.protect;
        let ok = m.state == 0x1000 && prot & 0x100 == 0 && matches!(prot & 0xff, 0x02 | 0x04 | 0x08 | 0x20 | 0x40 | 0x80);
        if ok {
            match s.ok.last_mut() {
                Some(last) if last.1 == p => last.1 = next,
                _ => s.ok.push((p, next)),
            }
        } else {
            if s.bad.is_none() {
                s.bad = Some((p, m.state, m.protect, m.ty, m.base, m.region_size));
            }
            s.bads.push((p, next, m.state, m.protect, m.ty, m.base, m.region_size));
        }
        p = next;
    }
    s
}

impl Scan {
    fn clip(&self, lo: usize, len: usize) -> Scan {
        let hi = lo + len;
        let mut s = Scan { ok: Vec::new(), bad: None, bads: Vec::new() };
        let mut covered = lo;
        let mut gap: Option<usize> = None;
        for &(a, b) in &self.ok {
            let (a, b) = (a.max(lo), b.min(hi));
            if a >= b {
                continue;
            }
            if a > covered && gap.is_none() {
                gap = Some(covered);
            }
            s.ok.push((a, b));
            covered = covered.max(b);
        }
        if covered < hi && gap.is_none() {
            gap = Some(covered);
        }
        if let Some(g) = gap {
            s.bad = Some(self.bads.iter().find(|b| b.0 <= g && g < b.1)
                .map(|b| (g, b.2, b.3, b.4, b.5, b.6))
                .unwrap_or((g, 0, 0, 0, 0, 0)));
        }
        s
    }
}

fn sub_range(t: &Tracked, sub: usize) -> Option<(usize, usize)> {
    let fp = t.fps[sub];
    let bpe = t.layout.bpe as usize;
    let map = unsafe { t.layout.submap(sub as u32, (fp.row_size / bpe as u64) as u32, fp.rows) }?;
    Some((t.p.va as usize + map.lo, map.hi - map.lo))
}

fn scan_subs(t: &Tracked, subs: &[usize]) -> Option<Scan> {
    let mut lo = usize::MAX;
    let mut hi = 0usize;
    for &sub in subs {
        if let Some((a, n)) = sub_range(t, sub) {
            lo = lo.min(a);
            hi = hi.max(a + n);
        }
    }
    if lo >= hi {
        return None;
    }
    Some(scan(lo, hi - lo))
}

fn sparse_zero_enabled() -> bool {
    static SW: crate::live_sw::Sw = crate::live_sw::Sw::new("TOMBOX_SPARSE_ZERO", "sparse_zero", true);
    SW.on()
}

fn hash_bytes(b: &[u8]) -> u64 {
    let mut h: u64 = 0x9E37_79B9_7F4A_7C15 ^ b.len() as u64;
    let mut chunks = b.chunks_exact(8);
    for c in &mut chunks {
        let w = u64::from_le_bytes(c.try_into().unwrap());
        h = (h ^ w).wrapping_mul(0x0000_0100_0000_01B3).rotate_left(23);
    }
    for &x in chunks.remainder() {
        h = (h ^ x as u64).wrapping_mul(0x0000_0100_0000_01B3);
    }
    h
}

// ---- Xbox layout via the real xg_xs.dll ---------------------------------------------------

/// Element offsets of one subresource inside the Xbox resource memory.
struct SubMap {
    w_e: u32,
    h_e: u32,
    offs: Vec<u32>,
    lo: usize,
    hi: usize, // exclusive
}

struct TexLayout {
    comp: usize, // XGTextureAddressComputer*
    total: u64,
    bpe: u32,
    planes: u32,
    mips: u32,
    subs: Mutex<HashMap<u32, Option<Arc<SubMap>>>>,
}

impl TexLayout {
    unsafe fn create(desc: &[u8; 0x38]) -> Result<TexLayout, String> {
        let (h, bpe_off) = crate::xone_tex::xg_module();
        if h.is_null() {
            return Err("neither xg_xs.dll nor xg_x.dll is loadable".into());
        }
        let f = GetProcAddress(h, b"XGCreateTextureComputer\0".as_ptr());
        if f.is_null() {
            return Err("XGCreateTextureComputer export missing".into());
        }
        let create: unsafe extern "system" fn(*const u8, *mut *mut c_void) -> i32 = core::mem::transmute(f);
        let mut comp: *mut c_void = core::ptr::null_mut();
        let hr = create(desc.as_ptr(), &mut comp);
        if hr < 0 || comp.is_null() {
            return Err(format!("XGCreateTextureComputer hr={hr:#010x}"));
        }
        // XG_RESOURCE_LAYOUT: u64 size, u64 alignment, u32 mips, u32 planes, planes[] of 0x6C8
        // bytes each (title's FD3D12TextureLayout::FillFromDesc copies exactly that); the
        // plane's BytesPerElement is at plane+0x28 (probe).
        let mut lay = vec![0u8; 0x4000];
        let hr = vcall!(comp, 2, fn(*mut u8) -> i32, lay.as_mut_ptr());
        if hr < 0 {
            return Err(format!("GetResourceLayout hr={hr:#010x}"));
        }
        let u64at = |o: usize| u64::from_le_bytes(lay[o..o + 8].try_into().unwrap());
        let u32at = |o: usize| u32::from_le_bytes(lay[o..o + 4].try_into().unwrap());
        Ok(TexLayout {
            comp: comp as usize,
            total: u64at(0),
            mips: u32at(0x10),
            planes: u32at(0x14),
            bpe: u32at(bpe_off),
            subs: Mutex::new(HashMap::new()),
        })
    }

    /// Element map of subresource `sub` (mip + slice * mips), built on first use.
    unsafe fn submap(&self, sub: u32, w_e: u32, h_e: u32) -> Option<Arc<SubMap>> {
        let mut g = self.subs.lock().unwrap();
        if let Some(v) = g.get(&sub) {
            return v.clone();
        }
        let (mip, slice) = (sub % self.mips, sub / self.mips);
        let comp = self.comp as *mut c_void;
        let mut offs = Vec::with_capacity((w_e * h_e) as usize);
        let (mut lo, mut hi) = (usize::MAX, 0usize);
        let mut ok = true;
        'outer: for y in 0..h_e {
            for x in 0..w_e {
                let o = vcall!(comp, 6, fn(u32, u32, u64, u32, u32, u32, *mut u32) -> u64,
                    0, mip, x as u64, y, slice, 0, core::ptr::null_mut());
                if o >= self.total || o + self.bpe as u64 > self.total || o > u32::MAX as u64 {
                    ok = false;
                    break 'outer;
                }
                lo = lo.min(o as usize);
                hi = hi.max(o as usize + self.bpe as usize);
                offs.push(o as u32);
            }
        }
        let v = if ok { Some(Arc::new(SubMap { w_e, h_e, offs, lo, hi })) } else { None };
        g.insert(sub, v.clone());
        v
    }
}

fn layout_for(desc: &[u8; 0x38]) -> Result<Arc<TexLayout>, String> {
    static CACHE: Mutex<Option<HashMap<[u8; 0x38], Result<Arc<TexLayout>, String>>>> = Mutex::new(None);
    let mut key = *desc;
    key[8..16].fill(0); // alignment does not affect the layout request we make
    let mut g = CACHE.lock().unwrap();
    let m = g.get_or_insert_with(HashMap::new);
    if let Some(v) = m.get(&key) {
        return v.clone();
    }
    let v = unsafe { TexLayout::create(desc) }.map(Arc::new);
    m.insert(key, v.clone());
    v
}

// ---- tracking + upload -----------------------------------------------------------------------

#[derive(Clone, Copy)]
struct Footprint {
    raw: [u8; 32], // D3D12_PLACED_SUBRESOURCE_FOOTPRINT
    rows: u32,
    row_size: u64,
}
impl Footprint {
    fn offset(&self) -> u64 { u64::from_le_bytes(self.raw[0..8].try_into().unwrap()) }
    fn row_pitch(&self) -> u32 { u32::from_le_bytes(self.raw[0x18..0x1c].try_into().unwrap()) }
}

struct Tracked {
    res: usize,
    p: Placed,
    layout: Arc<TexLayout>,
    fps: Vec<Footprint>,
    total_bytes: u64,
    hashes: Vec<Option<u64>>,
    partial: Vec<usize>,
    zero: Vec<bool>,
    watch: Vec<u64>,
    pend: Vec<u64>,
    hot: u64,
}

struct Uploader {
    device: *mut c_void,
    alloc: *mut c_void,
    list: *mut c_void,
    fence: *mut c_void,
    fence_value: u64,
    tracked: Vec<Tracked>,
    retire: Vec<(u64, usize, u64)>,
    pool: Vec<(u64, usize)>,
    pool_bytes: u64,
    backlog: std::collections::VecDeque<(usize, Placed)>,
    release_cursor: usize,
    cursor: usize,
    partial_cursor: usize,
    pumps: u64,
    uploaded_subs: u64,
    uploaded_bytes: u64,
    unreadable: u64,
    rejected: u64,
    recommitted: u64,
    zero_cursor: usize,
    zero_filled: u64,
    watch_changed: u64,
    index: Vec<(usize, usize, usize)>,
    index_stale: bool,
    ev: EventStats,
    cost_sum_us: u64,
    cost_max_us: u64,
    cost_n: u64,
    reaper: crate::reaper::Reaper,
}
unsafe impl Send for Uploader {}

#[derive(Default)]
struct EventStats {
    io_notes: u64,
    io_bytes: u64,
    io_subs: u64,
    commit_notes: u64,
    commit_bytes: u64,
    commit_subs: u64,
    unmatched: u64,
}

const WATCH_PUMPS: u64 = 30;
const PENDING_PUMPS: u64 = 120;
const ZERO_SCAN_BUDGET: u64 = 64 << 20;
const EVENT_SCAN_BUDGET: u64 = 16 << 20;
pub const NOTE_WRITTEN: u32 = 0;
pub const NOTE_COMMITTED: u32 = 1;
static UP: Mutex<Option<Uploader>> = Mutex::new(None);
static NOTES: Mutex<Vec<(usize, usize, u32)>> = Mutex::new(Vec::new());

fn poll_mode() -> bool {
    static SW: crate::live_sw::Sw = crate::live_sw::Sw::new("TOMBOX_UPLOAD_POLL", "upload_poll", false);
    SW.on()
}

#[no_mangle]
pub extern "system" fn TomboxNoteTitleMemory(addr: usize, len: usize, kind: u32) {
    if addr == 0 || len == 0 {
        return;
    }
    crate::alias::note_title_memory(addr, len, kind);
    let mut g = NOTES.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(last) = g.last_mut() {
        if last.2 == kind && last.0 + last.1 == addr {
            last.1 += len;
            return;
        }
    }
    g.push((addr, len, kind));
}

fn take_notes() -> Vec<(usize, usize, u32)> {
    core::mem::take(&mut *NOTES.lock().unwrap_or_else(|e| e.into_inner()))
}

fn rebuild_index(up: &mut Uploader) {
    up.index.clear();
    for (i, t) in up.tracked.iter().enumerate() {
        let lo = t.p.va as usize;
        up.index.push((lo, lo + t.layout.total as usize, i));
    }
    up.index.sort_unstable();
    up.index_stale = false;
}

fn overlapping(index: &[(usize, usize, usize)], a: usize, b: usize) -> Vec<usize> {
    let end = index.partition_point(|e| e.0 < b);
    let mut out = Vec::new();
    let mut i = end;
    while i > 0 {
        i -= 1;
        let (_, hi, idx) = index[i];
        if hi > a {
            out.push(idx);
        }
        if end - i > 64 && hi <= a {
            break;
        }
    }
    out
}

fn apply_notes(up: &mut Uploader, notes: &[(usize, usize, u32)]) {
    if notes.is_empty() {
        return;
    }
    if up.index_stale {
        rebuild_index(up);
    }
    let now = up.pumps;
    for &(a, n, kind) in notes {
        let b = a + n;
        if kind == NOTE_WRITTEN {
            up.ev.io_notes += 1;
            up.ev.io_bytes += n as u64;
        } else {
            up.ev.commit_notes += 1;
            up.ev.commit_bytes += n as u64;
        }
        let hits = overlapping(&up.index, a, b);
        if hits.is_empty() {
            up.ev.unmatched += 1;
            continue;
        }
        for idx in hits {
            let t = &mut up.tracked[idx];
            for sub in 0..t.hashes.len() {
                let Some((lo, len)) = sub_range(t, sub) else { continue };
                if lo >= b || lo + len <= a {
                    continue;
                }
                t.hashes[sub] = None;
                t.zero[sub] = false;
                if kind == NOTE_WRITTEN {
                    up.ev.io_subs += 1;
                } else {
                    t.pend[sub] = now + PENDING_PUMPS;
                    t.hot = t.hot.max(now + PENDING_PUMPS);
                    up.ev.commit_subs += 1;
                }
            }
        }
    }
}

unsafe fn init(queue: *mut c_void) -> Result<Uploader, String> {
    let mut device: *mut c_void = core::ptr::null_mut();
    let hr = vcall!(queue, 7, fn(*const Guid, *mut *mut c_void) -> i32, &IID_DEVICE, &mut device);
    if hr < 0 || device.is_null() {
        return Err(format!("queue->GetDevice hr={hr:#010x}"));
    }
    let mut alloc: *mut c_void = core::ptr::null_mut();
    let hr = vcall!(device, 9, fn(u32, *const Guid, *mut *mut c_void) -> i32, 0, &IID_ALLOCATOR, &mut alloc);
    if hr < 0 {
        return Err(format!("CreateCommandAllocator hr={hr:#010x}"));
    }
    let mut list: *mut c_void = core::ptr::null_mut();
    let hr = vcall!(device, 12, fn(u32, u32, *mut c_void, *mut c_void, *const Guid, *mut *mut c_void) -> i32,
        0, 0, alloc, core::ptr::null_mut(), &IID_GFX_LIST, &mut list);
    if hr < 0 {
        return Err(format!("CreateCommandList hr={hr:#010x}"));
    }
    vcall!(list, 9, fn() -> i32);
    let mut fence: *mut c_void = core::ptr::null_mut();
    let hr = vcall!(device, 36, fn(u64, u32, *const Guid, *mut *mut c_void) -> i32, 0, 0, &IID_FENCE, &mut fence);
    if hr < 0 {
        return Err(format!("CreateFence hr={hr:#010x}"));
    }
    let fence_addr = fence as usize;
    let reaper = crate::reaper::Reaper::new(
        move || unsafe { vcall!(fence_addr as *mut c_void, 8, fn() -> u64) },
        |res| unsafe {
            vcall!(res as *mut c_void, 2, fn() -> u32);
        },
    );
    Ok(Uploader {
        reaper,
        device, alloc, list, fence, fence_value: 0, tracked: Vec::new(), retire: Vec::new(), pool: Vec::new(), pool_bytes: 0, backlog: std::collections::VecDeque::new(), release_cursor: 0, cursor: 0, partial_cursor: 0,
        pumps: 0, uploaded_subs: 0, uploaded_bytes: 0, unreadable: 0, rejected: 0, recommitted: 0,
        zero_cursor: 0, zero_filled: 0, watch_changed: 0,
        index: Vec::new(), index_stale: true, ev: EventStats::default(), cost_sum_us: 0, cost_max_us: 0, cost_n: 0,
    })
}

/// Why a placed resource is not mirrored (None = it is).
fn unsupported_reason(p: &Placed) -> Option<String> {
    if p.dim() != 3 {
        return Some(format!("dimension {} (only Texture2D/array/cube are mirrored)", p.dim()));
    }
    if p.flags() & 0x3 != 0 {
        return Some("render-target/depth-stencil texture (GPU-written, nothing to mirror)".into());
    }
    None
}

unsafe fn adopt(up: &mut Uploader, res: *mut c_void, p: Placed) -> Result<Tracked, String> {
    if let Some(r) = unsupported_reason(&p) {
        return Err(r);
    }
    let layout = layout_for(&p.desc)?;
    if layout.planes != 1 {
        return Err(format!("{} planes", layout.planes));
    }
    if layout.bpe == 0 || layout.total == 0 || layout.total > u32::MAX as u64 {
        return Err(format!("layout size {:#x} bpe {}", layout.total, layout.bpe));
    }
    let nsubs = p.mips() as u32 * p.depth_or_array() as u32;
    if layout.mips != p.mips() as u32 || nsubs == 0 || nsubs > 4096 {
        return Err(format!("mip mismatch xg={} desc={}", layout.mips, p.mips()));
    }
    let mut pc_desc = [0u8; 0x38];
    vcall!(res, 10, fn(*mut u8) -> *mut u8, pc_desc.as_mut_ptr());
    let mut raw = vec![[0u8; 32]; nsubs as usize];
    let mut rows = vec![0u32; nsubs as usize];
    let mut row_sizes = vec![0u64; nsubs as usize];
    let mut total = 0u64;
    vcall!(up.device, 38, fn(*const u8, u32, u32, u64, *mut [u8; 32], *mut u32, *mut u64, *mut u64) -> (),
        pc_desc.as_ptr(), 0, nsubs, 0, raw.as_mut_ptr(), rows.as_mut_ptr(), row_sizes.as_mut_ptr(), &mut total);
    if total == 0 || total == u64::MAX {
        return Err("GetCopyableFootprints failed".into());
    }
    let fps: Vec<Footprint> = (0..nsubs as usize).map(|i| Footprint { raw: raw[i], rows: rows[i], row_size: row_sizes[i] }).collect();
    if fps.iter().any(|f| f.row_size % layout.bpe as u64 != 0) {
        return Err(format!("row size not a multiple of bytes-per-element {}", layout.bpe));
    }
    Ok(Tracked { res: res as usize, p, layout, fps, total_bytes: total, hashes: vec![None; nsubs as usize], partial: vec![0; nsubs as usize], zero: vec![false; nsubs as usize], watch: vec![0; nsubs as usize], pend: vec![up.pumps + PENDING_PUMPS; nsubs as usize], hot: up.pumps + PENDING_PUMPS })
}

struct Staged {
    res: usize,
    state: u32,
    buf: *mut c_void,
    size: u64,
    subs: Vec<(u32, Footprint)>,
}

/// Check one tracked texture; stage an upload for every changed, readable subresource.
/// `fresh_only`: look only at subresources that have never been uploaded.
unsafe fn check(up: &mut Uploader, idx: usize, hashed: &mut u64, fresh_only: bool) -> Option<Staged> {
    let device = up.device;
    let now = up.pumps;
    let t = &mut up.tracked[idx];
    let bpe = t.layout.bpe as usize;
    let mut changed: Vec<(u32, Arc<SubMap>, u64, Option<Arc<Vec<u8>>>)> = Vec::new();
    let mut unreadable = 0;
    let want: Vec<usize> = (0..t.fps.len()).filter(|&s| fresh_only != t.hashes[s].is_some()).collect();
    if want.is_empty() {
        return None;
    }
    let gpu_owned = crate::placed::gpu_written(t.res);
    let ck = std::time::Instant::now();
    let whole = scan_subs(t, &want);
    stage_time(0, ck);
    let Some(whole) = whole else { return None };
    for sub in want.into_iter().map(|s| s as u32) {
        let fp = t.fps[sub as usize];
        let (w_e, h_e) = ((fp.row_size / bpe as u64) as u32, fp.rows);
        let ck = std::time::Instant::now();
        let map = t.layout.submap(sub, w_e, h_e);
        stage_time(1, ck);
        let Some(map) = map else {
            continue; // layout query failed for this subresource (counted at adoption time)
        };
        let base = t.p.va as usize;
        let (lo, len) = (base + map.lo, map.hi - map.lo);
        let sc = whole.clip(lo, len);
        let mut snap: Option<Arc<Vec<u8>>> = None;
        if let Some((bp, st, pr, ty, rb, rs)) = sc.bad {
            let good: usize = sc.ok.iter().map(|(a, b)| b - a).sum();
            static LOGGED: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
            let mark = good + 1;
            if t.partial[sub as usize] != mark && LOGGED.fetch_add(1, core::sync::atomic::Ordering::Relaxed) < 400 {
                record("d3d12_xs", "upload", &format!(
                    "unreadable placed texture res={:#x} va={:#x} {}x{}x{} mips={} fmt={} layout={} sub={sub} range={lo:#x}+{len:#x} readable={good:#x} first-bad={bp:#x} state={st:#x} protect={pr:#x} type={ty:#x} region={rb:#x}+{rs:#x} -> {}",
                    t.res, t.p.va, t.p.width(), t.p.height(), t.p.depth_or_array(), t.p.mips(), t.p.format(), t.p.layout(),
                    if sparse_zero_enabled() { "unmapped pages read as zero, re-checked every present" } else { "skipped" }));
            }
            if !sparse_zero_enabled() {
                t.partial[sub as usize] = mark;
                unreadable += 1;
                continue;
            }
            let mut v = vec![0u8; len];
            for (a, b) in &sc.ok {
                core::ptr::copy_nonoverlapping(*a as *const u8, v.as_mut_ptr().add(a - lo), b - a);
            }
            snap = Some(Arc::new(v));
            t.partial[sub as usize] = mark;
        } else {
            t.partial[sub as usize] = 0;
        }
        let mem: &[u8] = match &snap {
            Some(v) => v.as_slice(),
            None => core::slice::from_raw_parts(lo as *const u8, len),
        };
        let ck = std::time::Instant::now();
        let h = hash_bytes(mem);
        stage_time(2, ck);
        *hashed += mem.len() as u64;
        if t.hashes[sub as usize].is_none() && (gpu_owned || mem.iter().all(|&b| b == 0)) {
            // Never written by the title (fresh committed pages are zero). Do not upload:
            // the title may instead fill this subresource with a GPU copy
            // (CopyTextureRegion), which an upload of zeros could overwrite.
            t.hashes[sub as usize] = Some(h);
            t.zero[sub as usize] = !gpu_owned;
            continue;
        }
        t.zero[sub as usize] = false;
        if t.hashes[sub as usize] != Some(h) {
            changed.push((sub, map, h, snap));
        }
    }
    up.unreadable += unreadable;
    if changed.is_empty() {
        return None;
    }
    // One upload buffer for this texture's changed subresources.
    let heap: [u32; 5] = [2 /* UPLOAD */, 0, 0, 1, 1];
    let mut bd = [0u8; 0x38];
    bd[0..4].copy_from_slice(&1u32.to_le_bytes());
    let size = pool_bucket(t.total_bytes);
    bd[0x10..0x18].copy_from_slice(&size.to_le_bytes());
    bd[0x18..0x1c].copy_from_slice(&1u32.to_le_bytes());
    bd[0x1c..0x1e].copy_from_slice(&1u16.to_le_bytes());
    bd[0x1e..0x20].copy_from_slice(&1u16.to_le_bytes());
    bd[0x24..0x28].copy_from_slice(&1u32.to_le_bytes());
    bd[0x2c..0x30].copy_from_slice(&1u32.to_le_bytes());
    let mut buf: *mut c_void = core::ptr::null_mut();
    let ck = std::time::Instant::now();
    STAGE_US[5].fetch_add(t.total_bytes, core::sync::atomic::Ordering::Relaxed);
    let hr = match pool_take(&mut up.pool, &mut up.pool_bytes, size) {
        Some(b) => {
            buf = b as *mut c_void;
            0
        }
        None => vcall!(device, 27, fn(*const u32, u32, *const u8, u32, *const c_void, *const Guid, *mut *mut c_void) -> i32,
            heap.as_ptr(), 0, bd.as_ptr(), 0x2C3, core::ptr::null(), &IID_RESOURCE, &mut buf),
    };
    if hr < 0 || buf.is_null() {
        record("d3d12_xs", "upload", &format!("upload buffer ({} bytes) FAILED hr={hr:#010x}", t.total_bytes));
        return None;
    }
    let mut data: *mut u8 = core::ptr::null_mut();
    let hr = vcall!(buf, 8, fn(u32, *const c_void, *mut *mut u8) -> i32, 0, core::ptr::null(), &mut data);
    if hr < 0 || data.is_null() {
        vcall!(buf, 2, fn() -> u32);
        record("d3d12_xs", "upload", &format!("upload buffer Map FAILED hr={hr:#010x}"));
        return None;
    }
    stage_time(3, ck);
    let ck = std::time::Instant::now();
    let base = t.p.va as *const u8;
    let mut subs = Vec::with_capacity(changed.len());
    for (sub, map, h, snap) in changed {
        let fp = t.fps[sub as usize];
        let dst0 = data.add(fp.offset() as usize);
        let pitch = fp.row_pitch() as usize;
        let src0 = match &snap { Some(v) => v.as_ptr().wrapping_sub(map.lo), None => base };
        detile_rows(dst0, pitch, src0, &map.offs, map.w_e as usize, map.h_e as usize, bpe);
        t.hashes[sub as usize] = Some(h);
        t.watch[sub as usize] = now + WATCH_PUMPS;
        t.hot = t.hot.max(now + WATCH_PUMPS);
        up.uploaded_subs += 1;
        up.uploaded_bytes += (map.w_e as u64) * (map.h_e as u64) * bpe as u64;
        subs.push((sub, fp));
    }
    vcall!(buf, 9, fn(u32, *const c_void) -> (), 0, core::ptr::null());
    stage_time(4, ck);
    Some(Staged { res: t.res, state: t.p.init_state, buf, size, subs })
}

static STAGE_US: [core::sync::atomic::AtomicU64; 6] = [const { core::sync::atomic::AtomicU64::new(0) }; 6];

fn stage_time(i: usize, since: std::time::Instant) {
    STAGE_US[i].fetch_add(since.elapsed().as_micros() as u64, core::sync::atomic::Ordering::Relaxed);
}

fn take_stage_times() -> [u64; 6] {
    core::array::from_fn(|i| STAGE_US[i].swap(0, core::sync::atomic::Ordering::Relaxed))
}

const PARALLEL_DETILE_BYTES: usize = 16 << 20;
const ADOPT_BUDGET_US: u128 = 3_000;
const RETIRE_BUDGET_US: u128 = 2_000;
const RELEASE_BUDGET_US: u128 = 3_000;
const POOL_CAP_BYTES: u64 = 256 << 20;
const POOL_MIN_BUCKET: u64 = 64 << 10;

fn pool_bucket(bytes: u64) -> u64 {
    bytes.max(POOL_MIN_BUCKET).next_power_of_two()
}

fn pool_take(pool: &mut Vec<(u64, usize)>, pool_bytes: &mut u64, size: u64) -> Option<usize> {
    let i = pool.iter().rposition(|&(s, _)| s == size)?;
    let (s, b) = pool.swap_remove(i);
    *pool_bytes -= s;
    Some(b)
}

fn pool_give(pool: &mut Vec<(u64, usize)>, pool_bytes: &mut u64, size: u64, buf: usize, cap: u64) -> bool {
    if *pool_bytes + size > cap {
        return false;
    }
    pool.push((size, buf));
    *pool_bytes += size;
    true
}

unsafe fn detile_span(dst0: usize, pitch: usize, src0: usize, offs: &[u32], w_e: usize, rows: core::ops::Range<usize>, bpe: usize) {
    for y in rows {
        let row = (dst0 + y * pitch) as *mut u8;
        for (x, &o) in offs[y * w_e..(y + 1) * w_e].iter().enumerate() {
            core::ptr::copy_nonoverlapping((src0 as *const u8).wrapping_add(o as usize), row.add(x * bpe), bpe);
        }
    }
}

unsafe fn detile_rows(dst0: *mut u8, pitch: usize, src0: *const u8, offs: &[u32], w_e: usize, h_e: usize, bpe: usize) {
    let (d, s) = (dst0 as usize, src0 as usize);
    let workers = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(1, 8);
    if w_e * h_e * bpe < PARALLEL_DETILE_BYTES || workers == 1 || h_e < 2 {
        detile_span(d, pitch, s, offs, w_e, 0..h_e, bpe);
        return;
    }
    let chunk = h_e.div_ceil(workers);
    std::thread::scope(|sc| {
        for k in 1..workers {
            let r = (k * chunk).min(h_e)..((k + 1) * chunk).min(h_e);
            if !r.is_empty() {
                sc.spawn(move || unsafe { detile_span(d, pitch, s, offs, w_e, r, bpe) });
            }
        }
        detile_span(d, pitch, s, offs, w_e, 0..chunk.min(h_e), bpe);
    });
}

fn barrier(res: usize, before: u32, after: u32) -> [u64; 4] {
    [0, res as u64, 0xffff_ffffu64 | ((before as u64) << 32), after as u64]
}

fn first_nonzero(ranges: &[(usize, usize)]) -> bool {
    ranges.iter().any(|&(a, b)| unsafe { core::slice::from_raw_parts(a as *const u8, b - a) }.iter().any(|&x| x != 0))
}

fn watch_pending(up: &mut Uploader) -> (u64, u64) {
    let nt = up.tracked.len();
    let now = up.pumps;
    let poll = poll_mode();
    let budget = if poll { ZERO_SCAN_BUDGET } else { EVENT_SCAN_BUDGET };
    let (mut scanned, mut filled, mut changed, mut k) = (0u64, 0u64, 0u64, 0usize);
    while k < nt && scanned < budget {
        let idx = (up.zero_cursor + k) % nt;
        k += 1;
        let t = &mut up.tracked[idx];
        if !poll && t.hot <= now {
            continue;
        }
        let subs: Vec<usize> = (0..t.hashes.len()).filter(|&s| t.hashes[s].is_some() && ((t.zero[s] && (poll || t.pend[s] > now)) || t.watch[s] > now)).collect();
        if subs.is_empty() {
            continue;
        }
        let Some(whole) = scan_subs(t, &subs) else { continue };
        for sub in subs {
            let Some((a, n)) = sub_range(t, sub) else { continue };
            let sc = whole.clip(a, n);
            if t.zero[sub] {
                scanned += sc.ok.iter().map(|(a, b)| (b - a) as u64).sum::<u64>();
                if first_nonzero(&sc.ok) {
                    t.zero[sub] = false;
                    t.hashes[sub] = None;
                    filled += 1;
                }
            } else if sc.bad.is_none() {
                scanned += n as u64;
                let h = hash_bytes(unsafe { core::slice::from_raw_parts(a as *const u8, n) });
                if t.hashes[sub] != Some(h) {
                    t.hashes[sub] = None;
                    changed += 1;
                }
            }
        }
    }
    up.zero_cursor = if nt == 0 { 0 } else { (up.zero_cursor + k) % nt };
    (filled, changed)
}

struct PumpPhases {
    t0: std::time::Instant,
    last: std::time::Instant,
    parts: Vec<(&'static str, u64)>,
}

impl PumpPhases {
    fn start() -> Self {
        let now = std::time::Instant::now();
        PumpPhases { t0: now, last: now, parts: Vec::new() }
    }

    fn mark(&mut self, what: &'static str) {
        let now = std::time::Instant::now();
        self.parts.push((what, (now - self.last).as_micros() as u64));
        self.last = now;
    }

    fn report(&self, pump: u64, note: &str, fresh_visits: usize, staged: usize, bytes: u64) {
        static N: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
        let total = self.t0.elapsed().as_micros() as u64;
        let st = take_stage_times();
        if total < 20_000 || N.fetch_add(1, core::sync::atomic::Ordering::Relaxed) >= 60 {
            return;
        }
        let parts: Vec<String> = self.parts.iter().map(|(w, us)| format!("{w}={:.1}ms", *us as f64 / 1000.0)).collect();
        record("d3d12_xs", "upload", &format!("slow pump {pump}: {:.1} ms on the present thread: {}; fresh visits={fresh_visits} staged={staged} hashed={} MB; inside check: residency-scan={:.1}ms submap={:.1}ms hash={:.1}ms upload-buffer-create+map={:.1}ms ({} MB) detile+unmap={:.1}ms{}",
            total as f64 / 1000.0, parts.join(" "), bytes >> 20, st[0] as f64 / 1000.0, st[1] as f64 / 1000.0, st[2] as f64 / 1000.0, st[3] as f64 / 1000.0, st[5] >> 20, st[4] as f64 / 1000.0,
            if note.is_empty() { String::new() } else { format!(" ({note})") }));
    }
}

/// Called once per `PresentX`, on the presenting thread, before the frame is copied out.
pub unsafe fn pump(queue: *mut c_void) {
    let t0 = std::time::Instant::now();
    pump_inner(queue);
    let us = t0.elapsed().as_micros() as u64;
    if let Some(up) = UP.lock().unwrap().as_mut() {
        up.cost_sum_us += us;
        up.cost_n += 1;
        up.cost_max_us = up.cost_max_us.max(us);
    }
}

unsafe fn pump_inner(queue: *mut c_void) {
    static FAILED: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
    if FAILED.load(core::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let mut guard = UP.lock().unwrap();
    if guard.is_none() {
        match init(queue) {
            Ok(u) => *guard = Some(u),
            Err(e) => {
                FAILED.store(true, core::sync::atomic::Ordering::Relaxed);
                record("d3d12_xs", "upload", &format!("init FAILED: {e} - placed textures will stay empty"));
                return;
            }
        }
    }
    let up = guard.as_mut().unwrap();
    up.pumps += 1;
    let mut ph = PumpPhases::start();

    // Retire upload buffers whose copy has completed.
    let done = vcall!(up.fence, 8, fn() -> u64);
    let budget = std::time::Instant::now();
    let (pool, pool_bytes, reaper) = (&mut up.pool, &mut up.pool_bytes, &up.reaper);
    up.retire.retain(|&(v, b, size)| {
        if v <= done && budget.elapsed().as_micros() < RETIRE_BUDGET_US {
            if !pool_give(pool, pool_bytes, size, b, POOL_CAP_BYTES) && !reaper.push(b, 0) {
                vcall!(b as *mut c_void, 2, fn() -> u32);
            }
            false
        } else {
            true
        }
    });
    ph.mark("retire");

    up.backlog.extend(crate::placed::drain_new());
    let budget = std::time::Instant::now();
    while budget.elapsed().as_micros() < ADOPT_BUDGET_US {
        let Some((res, p)) = up.backlog.pop_front() else { break };
        match adopt(up, res as *mut c_void, p.clone()) {
            Ok(t) => {
                up.tracked.push(t);
                up.index_stale = true;
            }
            Err(e) => {
                up.rejected += 1;
                if up.rejected <= 12 {
                    record("d3d12_xs", "upload", &format!(
                        "NOT mirrored: va={:#x} {}x{}x{} mips={} fmt={} layout={} flags={:#x}: {e}",
                        p.va, p.width(), p.height(), p.depth_or_array(), p.mips(), p.format(), p.layout(), p.flags()));
                }
                crate::placed::forget(res as *mut c_void);
                vcall!(res as *mut c_void, 2, fn() -> u32);
            }
        }
    }
    ph.mark("adopt");

    // Drop textures the title has released (ours is the last reference).
    let budget = std::time::Instant::now();
    let n = up.tracked.len();
    let mut seen = 0;
    let mut freed = 0u32;
    while seen < n && !up.tracked.is_empty() {
        if freed > 0 && budget.elapsed().as_micros() >= RELEASE_BUDGET_US {
            break;
        }
        let i = up.release_cursor % up.tracked.len();
        let r = up.tracked[i].res as *mut c_void;
        seen += 1;
        vcall!(r, 1, fn() -> u32);
        if vcall!(r, 2, fn() -> u32) == 1 {
            crate::placed::forget(r);
            if !up.reaper.push(r as usize, up.fence_value) {
                vcall!(r, 2, fn() -> u32);
            }
            up.tracked.swap_remove(i);
            up.index_stale = true;
            freed += 1;
        } else {
            up.release_cursor = i + 1;
        }
    }

    ph.mark("release");
    let notes = take_notes();
    apply_notes(up, &notes);
    ph.mark("notes");

    // The single upload list/allocator can be reused only once the previous copy is done.
    if done < up.fence_value {
        ph.report(up.pumps, "previous upload copy still in flight", 0, 0, 0);
        return;
    }

    const FRESH_BUDGET: u64 = 96 << 20; // bytes hashed per frame for never-uploaded textures
    const RECHECK_BUDGET: u64 = 8 << 20; // bytes hashed per frame re-checking uploaded ones
    const PARTIAL_TEXTURES_PER_FRAME: usize = 64;
    const RECHECK_TEXTURES_PER_FRAME: usize = 256;
    const PUMP_TIME_BUDGET_US: u64 = 4000;
    let poll = poll_mode();
    let nt = up.tracked.len();
    let mut partial_visits = 0;
    let mut k = 0;
    while poll && k < nt && partial_visits < PARTIAL_TEXTURES_PER_FRAME {
        let idx = (up.partial_cursor + k) % nt;
        k += 1;
        let t = &mut up.tracked[idx];
        let subs: Vec<usize> = (0..t.partial.len()).filter(|&s| t.partial[s] != 0 && t.hashes[s].is_some()).collect();
        if subs.is_empty() {
            continue;
        }
        partial_visits += 1;
        let Some(whole) = scan_subs(t, &subs) else { continue };
        for sub in subs {
            let Some((a, n)) = sub_range(t, sub) else { continue };
            let sc = whole.clip(a, n);
            let good: usize = sc.ok.iter().map(|(a, b)| b - a).sum();
            if sc.bad.is_none() || good + 1 != t.partial[sub] {
                t.hashes[sub] = None;
                up.recommitted += 1;
            }
        }
    }
    if poll {
        up.partial_cursor = if nt == 0 { 0 } else { (up.partial_cursor + k) % nt };
    }
    ph.mark("partial-rescan");
    let (filled, changed) = watch_pending(up);
    up.zero_filled += filled;
    up.watch_changed += changed;
    ph.mark("zero-watch");
    let mut staged: Vec<Staged> = Vec::new();
    let mut hashed = 0u64;
    let n = up.tracked.len();
    let deadline = std::time::Instant::now() + std::time::Duration::from_micros(PUMP_TIME_BUDGET_US);
    let mut fresh_visits = 0usize;
    static FRESH_CURSOR: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
    let start = if n == 0 { 0 } else { FRESH_CURSOR.load(core::sync::atomic::Ordering::Relaxed) % n };
    let mut last = start;
    for idx in (start..n).chain(0..start) {
        if hashed >= FRESH_BUDGET || (fresh_visits > 0 && std::time::Instant::now() >= deadline) {
            break;
        }
        last = idx + 1;
        if up.tracked[idx].hashes.iter().any(|h| h.is_none()) {
            fresh_visits += 1;
            if let Some(s) = check(up, idx, &mut hashed, true) {
                staged.push(s);
            }
        }
    }
    FRESH_CURSOR.store(if last >= n { 0 } else { last }, core::sync::atomic::Ordering::Relaxed);
    ph.mark("fresh-hash+stage");
    let fresh_staged = staged.len();
    let mut rehashed = 0u64;
    let mut visited = 0;
    while poll && n > 0 && visited < n.min(RECHECK_TEXTURES_PER_FRAME) && rehashed < RECHECK_BUDGET && (visited == 0 || std::time::Instant::now() < deadline) {
        let idx = up.cursor % n;
        up.cursor = (up.cursor + 1) % n;
        visited += 1;
        if let Some(s) = check(up, idx, &mut rehashed, false) {
            staged.push(s);
        }
    }
    ph.mark("recheck");

    if !staged.is_empty() {
        const COPY_DEST: u32 = 0x400;
        vcall!(up.alloc, 8, fn() -> i32);
        vcall!(up.list, 10, fn(*mut c_void, *mut c_void) -> i32, up.alloc, core::ptr::null_mut());
        for s in &staged {
            if s.state != COPY_DEST {
                let b = [barrier(s.res, s.state, COPY_DEST)];
                vcall!(up.list, 26, fn(u32, *const [u64; 4]) -> (), 1, b.as_ptr());
            }
            for (sub, fp) in &s.subs {
                // D3D12_TEXTURE_COPY_LOCATION { pResource; Type; union { PlacedFootprint | SubresourceIndex } }
                let mut dst = [0u8; 48];
                dst[0..8].copy_from_slice(&(s.res as u64).to_le_bytes());
                dst[16..20].copy_from_slice(&sub.to_le_bytes()); // Type 0 = SUBRESOURCE_INDEX
                let mut src = [0u8; 48];
                src[0..8].copy_from_slice(&(s.buf as u64).to_le_bytes());
                src[8..12].copy_from_slice(&1u32.to_le_bytes()); // Type 1 = PLACED_FOOTPRINT
                src[16..48].copy_from_slice(&fp.raw);
                vcall!(up.list, 16, fn(*const u8, u32, u32, u32, *const u8, *const c_void) -> (),
                    dst.as_ptr(), 0, 0, 0, src.as_ptr(), core::ptr::null());
            }
            if s.state != COPY_DEST {
                let b = [barrier(s.res, COPY_DEST, s.state)];
                vcall!(up.list, 26, fn(u32, *const [u64; 4]) -> (), 1, b.as_ptr());
            }
        }
        let hr = vcall!(up.list, 9, fn() -> i32);
        if hr < 0 {
            record("d3d12_xs", "upload", &format!("upload list Close FAILED hr={hr:#010x}"));
        } else {
            let lists = [up.list];
            vcall!(queue, 10, fn(u32, *const *mut c_void) -> (), 1, lists.as_ptr());
        }
        up.fence_value += 1;
        vcall!(queue, 14, fn(*mut c_void, u64) -> i32, up.fence, up.fence_value);
        for s in &staged {
            up.retire.push((up.fence_value, s.buf as usize, s.size));
        }
    }
    ph.mark("record+submit");
    ph.report(up.pumps, "", fresh_visits, fresh_staged, hashed + rehashed);

    if (!staged.is_empty() && up.pumps < 40) || up.pumps % 600 == 0 {
        let (gh, gn) = crate::placed::gpu_write_stats();
        let (mut subs_total, mut zero_pending, mut partial_subs, mut uploaded_now, mut zero_bytes) = (0usize, 0usize, 0usize, 0usize, 0u64);
        for t in &up.tracked {
            for s in 0..t.hashes.len() {
                subs_total += 1;
                if t.zero[s] {
                    zero_pending += 1;
                    zero_bytes += sub_range(t, s).map(|(_, n)| n as u64).unwrap_or(0);
                } else if t.partial[s] != 0 {
                    partial_subs += 1;
                } else if t.hashes[s].is_some() {
                    uploaded_now += 1;
                }
            }
        }
        record("d3d12_xs", "upload", &format!(
            "residency pump {}: subresources={subs_total} with-data={uploaded_now} still-zero={zero_pending} ({} MB) partly-unmapped={partial_subs} | zero->data detected={} re-written after upload={}",
            up.pumps, zero_bytes >> 20, up.zero_filled, up.watch_changed));
        let avg = if up.cost_n == 0 { 0 } else { up.cost_sum_us / up.cost_n };
        let e = &up.ev;
        record("d3d12_xs", "upload", &format!(
            "events pump {} mode={}: io-writes={} ({} MB) -> subresources dirtied={} | commits/maps={} ({} MB) -> subresources dirtied={} | notes outside placed textures={} | pump cost avg={avg}us max={}us over {} presents",
            up.pumps, if poll { "poll" } else { "event" }, e.io_notes, e.io_bytes >> 20, e.io_subs, e.commit_notes, e.commit_bytes >> 20, e.commit_subs, e.unmatched, up.cost_max_us, up.cost_n));
        up.cost_sum_us = 0;
        up.cost_max_us = 0;
        up.cost_n = 0;
        record("d3d12_xs", "upload", &format!(
            "pump {}: staged {} textures; tracked={} total uploaded {} subresources / {} bytes; unreadable-skips={} not-mirrored={} gpu-copy-dest hits={gh} textures={gn} inserted-slot-calls={} texcache-flushes={} recommit-rechecks={}; reaper pending={} freed={} worst-release={:.1}ms",
            up.pumps, staged.len(), up.tracked.len(), up.uploaded_subs, up.uploaded_bytes, up.unreadable, up.rejected, crate::cmdlist_wrap::inserted_hits(), crate::cmdlist_wrap::flush_hits(), up.recommitted,
            up.reaper.pending(), up.reaper.freed(), up.reaper.worst_us() as f64 / 1000.0));
    }
}

#[cfg(test)]
mod zero_watch_tests {
    use super::*;

    #[test]
    fn nonzero_found_only_inside_readable_ranges() {
        let mut buf = vec![0u8; 4096];
        let a = buf.as_ptr() as usize;
        assert!(!first_nonzero(&[(a, a + 4096)]));
        buf[3000] = 7;
        assert!(first_nonzero(&[(a, a + 4096)]));
        assert!(!first_nonzero(&[(a, a + 2048)]));
        assert!(first_nonzero(&[(a, a + 16), (a + 2990, a + 3001)]));
    }

    #[test]
    fn notes_find_only_overlapping_textures() {
        let index = vec![(0x1000, 0x2000, 7), (0x2000, 0x3000, 8), (0x8000, 0x9000, 9)];
        assert_eq!(overlapping(&index, 0x1800, 0x1900), vec![7]);
        let mut both = overlapping(&index, 0x1f00, 0x2100);
        both.sort();
        assert_eq!(both, vec![7, 8]);
        assert!(overlapping(&index, 0x3000, 0x8000).is_empty());
        assert_eq!(overlapping(&index, 0x8fff, 0xa000), vec![9]);
    }

    #[test]
    fn parallel_detile_matches_serial() {
        let (w, h, bpe, pitch) = (2048usize, 2176usize, 4usize, 8192usize + 256);
        let src: Vec<u8> = (0..w * h * bpe).map(|i| (i * 7 + i / 3) as u8).collect();
        let offs: Vec<u32> = (0..w * h).map(|i| (((i * 2654435761usize) % (w * h)) * bpe) as u32).collect();
        let mut a = vec![0u8; pitch * h];
        let mut b = vec![0u8; pitch * h];
        unsafe {
            detile_rows(a.as_mut_ptr(), pitch, src.as_ptr(), &offs, w, h, bpe);
            detile_span(b.as_mut_ptr() as usize, pitch, src.as_ptr() as usize, &offs, w, 0..h, bpe);
        }
        assert!(w * h * bpe >= PARALLEL_DETILE_BYTES);
        assert_eq!(a, b);
        assert_eq!(&a[pitch * 3 + 8..pitch * 3 + 12], &src[offs[3 * w + 2] as usize..offs[3 * w + 2] as usize + 4]);
    }

    #[test]
    fn adjacent_notes_of_one_kind_coalesce() {
        take_notes();
        TomboxNoteTitleMemory(0x10000, 0x1000, NOTE_WRITTEN);
        TomboxNoteTitleMemory(0x11000, 0x1000, NOTE_WRITTEN);
        TomboxNoteTitleMemory(0x12000, 0x1000, NOTE_COMMITTED);
        TomboxNoteTitleMemory(0, 0x1000, NOTE_WRITTEN);
        assert_eq!(take_notes(), vec![(0x10000, 0x2000, NOTE_WRITTEN), (0x12000, 0x1000, NOTE_COMMITTED)]);
    }
}
#[cfg(test)]
mod pool_tests {
    use super::*;

    #[test]
    fn buckets_round_up_to_powers_of_two_with_a_floor() {
        assert_eq!(pool_bucket(1), 64 << 10);
        assert_eq!(pool_bucket(64 << 10), 64 << 10);
        assert_eq!(pool_bucket((64 << 10) + 1), 128 << 10);
        assert_eq!(pool_bucket(5 << 20), 8 << 20);
    }

    #[test]
    fn pool_reuses_exact_buckets_and_respects_the_cap() {
        let (mut pool, mut bytes) = (Vec::new(), 0u64);
        assert!(pool_give(&mut pool, &mut bytes, 1 << 20, 0xa, 3 << 20));
        assert!(pool_give(&mut pool, &mut bytes, 2 << 20, 0xb, 3 << 20));
        assert!(!pool_give(&mut pool, &mut bytes, 1 << 20, 0xc, 3 << 20));
        assert_eq!(bytes, 3 << 20);
        assert_eq!(pool_take(&mut pool, &mut bytes, 4 << 20), None);
        assert_eq!(pool_take(&mut pool, &mut bytes, 2 << 20), Some(0xb));
        assert_eq!(bytes, 1 << 20);
        assert_eq!(pool_take(&mut pool, &mut bytes, 1 << 20), Some(0xa));
        assert_eq!(pool_take(&mut pool, &mut bytes, 1 << 20), None);
        assert_eq!(bytes, 0);
    }
}
#[cfg(test)]
mod detile_speed_tests {
    use super::*;

    #[test]
    fn one_megabyte_detiles_on_one_thread_in_a_few_ms() {
        let (w, h, bpe) = (512usize, 512usize, 4usize);
        let pitch = w * bpe;
        let src: Vec<u8> = (0..w * h * bpe).map(|i| (i * 13) as u8).collect();
        let offs: Vec<u32> = (0..w * h).map(|i| (((i * 2654435761usize) % (w * h)) * bpe) as u32).collect();
        let mut dst = vec![0u8; pitch * h];
        let t = std::time::Instant::now();
        for _ in 0..10 {
            unsafe { detile_span(dst.as_mut_ptr() as usize, pitch, src.as_ptr() as usize, &offs, w, 0..h, bpe) };
        }
        let per = t.elapsed().as_secs_f64() * 100.0;
        eprintln!("single-thread detile of 1 MB: {per:.2} ms");
        assert!(per < 20.0, "{per} ms");
    }
}
