use core::ffi::c_void;
use core::sync::atomic::{AtomicUsize, Ordering};
use std::collections::HashMap;
use std::sync::Mutex;
use compat_common::record;

pub const XBOX_SLOT_RESET: usize = 10;
pub const XBOX_SLOT_BEGIN_QUERY: usize = 28;
pub const XBOX_SLOT_END_QUERY: usize = 29;
pub const XBOX_SLOT_RESOLVE_QUERY_DATA: usize = 30;
pub const XBOX_SLOT_SET_PREDICATION: usize = 31;
const PC_SLOT_RESET: usize = 10;
const PC_SLOT_BEGIN_QUERY: usize = 52;
const PC_SLOT_END_QUERY: usize = 53;
const PC_SLOT_RESOLVE_QUERY_DATA: usize = 54;
const PC_SLOT_SET_PREDICATION: usize = 55;

static REAL_RESET: AtomicUsize = AtomicUsize::new(0);
static REAL_BEGIN: AtomicUsize = AtomicUsize::new(0);
static REAL_END: AtomicUsize = AtomicUsize::new(0);
static REAL_RESOLVE: AtomicUsize = AtomicUsize::new(0);
static REAL_PRED: AtomicUsize = AtomicUsize::new(0);

static BEGINS: AtomicUsize = AtomicUsize::new(0);
static UNNESTED: AtomicUsize = AtomicUsize::new(0);
static SWALLOWED: AtomicUsize = AtomicUsize::new(0);
static RESOLVES: AtomicUsize = AtomicUsize::new(0);
static PREDICATIONS: AtomicUsize = AtomicUsize::new(0);
static PREV_DRAW: AtomicUsize = AtomicUsize::new(0);
static PREV_DRAW_INDEXED: AtomicUsize = AtomicUsize::new(0);
static QUERIES_WITHOUT_DRAWS: AtomicUsize = AtomicUsize::new(0);
static QUERIES_WITH_DRAWS: AtomicUsize = AtomicUsize::new(0);
static DRAWS_IN_QUERIES: AtomicUsize = AtomicUsize::new(0);
static RESULTS_ZERO: AtomicUsize = AtomicUsize::new(0);
static RESULTS_NONZERO: AtomicUsize = AtomicUsize::new(0);
static RESULTS_SUM: AtomicUsize = AtomicUsize::new(0);
static FORCED: AtomicUsize = AtomicUsize::new(0);
static ONES: AtomicUsize = AtomicUsize::new(0);
const XBOX_SLOT_DRAW: usize = 12;
const XBOX_SLOT_DRAW_INDEXED: usize = 13;
const PC_SLOT_COPY_BUFFER_REGION: usize = 15;
const ONES_SLOTS: u64 = 1 << 16;
const FORCED_SAMPLES: u64 = 0x10000;

pub type Query = (usize, u32);

#[derive(Default, Debug)]
pub struct Nest {
    active: Option<Query>,
    ended_early: Vec<Query>,
    draws: u32,
    pub last_draws: Option<u32>,
}

impl Nest {
    pub fn begin(&mut self, q: Query) -> Option<Query> {
        let prev = self.active.take();
        if let Some(p) = prev {
            self.ended_early.push(p);
        }
        self.active = Some(q);
        self.draws = 0;
        prev
    }

    pub fn carry_into(&mut self, to: &mut Nest) {
        to.ended_early.append(&mut self.ended_early);
    }

    pub fn draw(&mut self) {
        if self.active.is_some() {
            self.draws += 1;
        }
    }

    pub fn end(&mut self, q: Query) -> bool {
        self.last_draws = None;
        if self.active == Some(q) {
            self.active = None;
            self.last_draws = Some(self.draws);
            return true;
        }
        if let Some(i) = self.ended_early.iter().position(|e| *e == q) {
            self.ended_early.swap_remove(i);
            return false;
        }
        true
    }
}

static NESTS: Mutex<Option<HashMap<usize, Nest>>> = Mutex::new(None);

pub fn carry_nest(from: usize, to: usize) {
    let mut g = NESTS.lock().unwrap_or_else(|e| e.into_inner());
    let m = g.get_or_insert_with(HashMap::new);
    let Some(mut src) = m.remove(&from) else { return };
    src.carry_into(m.entry(to).or_default());
    m.insert(from, src);
}

fn is_occlusion(ty: u32) -> bool {
    ty <= 1
}

fn unnest_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("TOMBOX_QUERY_UNNEST").map(|v| v != "0").unwrap_or(true))
}

fn force_visible() -> bool {
    static SW: crate::live_sw::Sw = crate::live_sw::Sw::new("TOMBOX_QUERY_FORCE_VISIBLE", "query_force_visible", false);
    SW.on()
}

unsafe fn note_draw(list: *mut c_void) {
    let in_query = if unnest_enabled() {
        with_nest(list as usize, |n| {
            n.draw();
            n.active.is_some()
        })
    } else {
        false
    };
    if VP_SAMPLING.load(Ordering::Relaxed) {
        let st = with_list(list as usize, |l| *l);
        let key = ([st.vp[0] as i32, st.vp[1] as i32, st.vp[2] as i32, st.vp[3] as i32], st.sc, in_query, st.pso);
        let mut g = VP_HIST.lock().unwrap_or_else(|e| e.into_inner());
        let m = g.get_or_insert_with(HashMap::new);
        if m.len() < 4096 || m.contains_key(&key) {
            *m.entry(key).or_insert(0) += 1;
        }
    }
}

type VpKey = ([i32; 4], [i32; 4], bool, usize);
static VP_SAMPLING: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
static VP_HIST: Mutex<Option<HashMap<VpKey, u64>>> = Mutex::new(None);

fn pso_depth(pso: usize) -> Option<(u32, u32, u32)> {
    let (d, _keep) = crate::device_wrap::gfx_pso_desc(pso)?;
    if d.len() < 0x270 {
        return None;
    }
    let u = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
    Some((u(0x1f0), u(0x1f4), u(0x240)))
}

fn dump_viewport_histogram() {
    let Some(m) = VP_HIST.lock().unwrap_or_else(|e| e.into_inner()).take() else { return };
    let mut agg: HashMap<([i32; 4], [i32; 4], bool, String), u64> = HashMap::new();
    for ((vp, sc, q, pso), n) in m {
        let kind = match pso_depth(pso) {
            Some((en, wr, nrt)) => format!("depth-en={en} depth-write={wr} nRT={nrt}"),
            None => "pso unknown".to_string(),
        };
        *agg.entry((vp, sc, q, kind)).or_insert(0) += n;
    }
    let mut v: Vec<_> = agg.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1));
    for ((vp, sc, q, kind), n) in v.into_iter().take(24) {
        record("d3d12_xs", "query_x", &format!(
            "draw viewports during diag: {n} draws viewport=[{} {} {} {}] scissor=[{} {} {} {}] in-occlusion-query={q} {kind}",
            vp[0], vp[1], vp[2], vp[3], sc[0], sc[1], sc[2], sc[3]));
    }
}

unsafe extern "system" fn draw_x(this: *mut c_void, a: u32, b: u32, c: u32, d: u32) {
    note_draw(this);
    let f: unsafe extern "system" fn(*mut c_void, u32, u32, u32, u32) = core::mem::transmute(PREV_DRAW.load(Ordering::Relaxed));
    f(this, a, b, c, d)
}

unsafe extern "system" fn draw_indexed_x(this: *mut c_void, a: u32, b: u32, c: u32, d: i32, e: u32) {
    note_draw(this);
    let f: unsafe extern "system" fn(*mut c_void, u32, u32, u32, i32, u32) = core::mem::transmute(PREV_DRAW_INDEXED.load(Ordering::Relaxed));
    f(this, a, b, c, d, e)
}

#[repr(C)]
struct HeapProps {
    ty: u32,
    cpu_page: u32,
    pool: u32,
    creation_mask: u32,
    visible_mask: u32,
}

#[repr(C)]
struct BufferDesc {
    dimension: u32,
    alignment: u64,
    width: u64,
    height: u32,
    depth: u16,
    mips: u16,
    format: u32,
    sample_count: u32,
    sample_quality: u32,
    layout: u32,
    flags: u32,
}

#[repr(C)]
struct Iid {
    d1: u32,
    d2: u16,
    d3: u16,
    d4: [u8; 8],
}

const IID_DEVICE: Iid = Iid { d1: 0x1898_19F1, d2: 0x1DB6, d3: 0x4B57, d4: [0xBE, 0x54, 0x18, 0x21, 0x33, 0x9B, 0x85, 0xF7] };
const IID_QUERY_HEAP: Iid = Iid { d1: 0x0d96_58ae, d2: 0xed45, d3: 0x469e, d4: [0xa6, 0x1d, 0x97, 0x0e, 0xc5, 0x83, 0xca, 0xb4] };
const IID_RESOURCE: Iid = Iid { d1: 0x6964_42be, d2: 0xa72e, d3: 0x4059, d4: [0xbc, 0x79, 0x5b, 0x5c, 0x98, 0x04, 0x0f, 0xad] };

unsafe fn ones_buffer(list: *mut c_void) -> usize {
    let have = ONES.load(Ordering::Acquire);
    if have != 0 {
        return have;
    }
    let lvt = *(list as *const *const usize);
    let get_device: unsafe extern "system" fn(*mut c_void, *const Iid, *mut *mut c_void) -> i32 = core::mem::transmute(*lvt.add(7));
    let mut dev: *mut c_void = core::ptr::null_mut();
    if get_device(list, &IID_DEVICE, &mut dev) < 0 || dev.is_null() {
        return 0;
    }
    let dvt = *(dev as *const *const usize);
    let create: unsafe extern "system" fn(*mut c_void, *const HeapProps, u32, *const BufferDesc, u32, *const c_void, *const Iid, *mut *mut c_void) -> i32 =
        core::mem::transmute(*dvt.add(27));
    let props = HeapProps { ty: 2, cpu_page: 0, pool: 0, creation_mask: 1, visible_mask: 1 };
    let desc = BufferDesc { dimension: 1, alignment: 0, width: ONES_SLOTS * 8, height: 1, depth: 1, mips: 1, format: 0, sample_count: 1, sample_quality: 0, layout: 1, flags: 0 };
    let mut res: *mut c_void = core::ptr::null_mut();
    let hr = create(dev, &props, 0, &desc, 0xAC3, core::ptr::null(), &IID_RESOURCE, &mut res);
    let release: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(*dvt.add(2));
    release(dev);
    if hr < 0 || res.is_null() {
        record("d3d12_xs", "query_x", &format!("force-visible: CreateCommittedResource(upload) failed hr={hr:#010x}"));
        return 0;
    }
    let rvt = *(res as *const *const usize);
    let map: unsafe extern "system" fn(*mut c_void, u32, *const [usize; 2], *mut *mut c_void) -> i32 = core::mem::transmute(*rvt.add(8));
    let unmap: unsafe extern "system" fn(*mut c_void, u32, *const [usize; 2]) = core::mem::transmute(*rvt.add(9));
    let none = [0usize; 2];
    let mut p: *mut c_void = core::ptr::null_mut();
    if map(res, 0, &none, &mut p) < 0 || p.is_null() {
        record("d3d12_xs", "query_x", "force-visible: Map of the upload buffer failed");
        return 0;
    }
    core::slice::from_raw_parts_mut(p as *mut u64, ONES_SLOTS as usize).fill(FORCED_SAMPLES);
    unmap(res, 0, core::ptr::null());
    match ONES.compare_exchange(0, res as usize, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => {
            record("d3d12_xs", "query_x", &format!("force-visible: every resolved occlusion result is overwritten with {FORCED_SAMPLES} samples"));
            res as usize
        }
        Err(other) => {
            let rel: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(*rvt.add(2));
            rel(res);
            other
        }
    }
}

pub unsafe fn install(xbox_vt: &[AtomicUsize], real_vt: *const usize) {
    if std::env::var("TOMBOX_QUERY_HOOKS").map(|v| v == "0").unwrap_or(false) {
        record("d3d12_xs", "query_x", "query hooks disabled by TOMBOX_QUERY_HOOKS=0; Xbox slots 28-31 go straight to PC 52-55");
        crate::ts99::install_plain_resolve(xbox_vt, XBOX_SLOT_RESOLVE_QUERY_DATA, *real_vt.add(PC_SLOT_RESOLVE_QUERY_DATA));
        return;
    }
    REAL_RESET.store(*real_vt.add(PC_SLOT_RESET), Ordering::Relaxed);
    REAL_BEGIN.store(*real_vt.add(PC_SLOT_BEGIN_QUERY), Ordering::Relaxed);
    REAL_END.store(*real_vt.add(PC_SLOT_END_QUERY), Ordering::Relaxed);
    REAL_RESOLVE.store(*real_vt.add(PC_SLOT_RESOLVE_QUERY_DATA), Ordering::Relaxed);
    REAL_PRED.store(*real_vt.add(PC_SLOT_SET_PREDICATION), Ordering::Relaxed);
    xbox_vt[XBOX_SLOT_RESET].store(reset_x as usize, Ordering::Relaxed);
    xbox_vt[XBOX_SLOT_BEGIN_QUERY].store(begin_query_x as usize, Ordering::Relaxed);
    xbox_vt[XBOX_SLOT_END_QUERY].store(end_query_x as usize, Ordering::Relaxed);
    xbox_vt[XBOX_SLOT_RESOLVE_QUERY_DATA].store(resolve_query_data_x as usize, Ordering::Relaxed);
    xbox_vt[XBOX_SLOT_SET_PREDICATION].store(set_predication_x as usize, Ordering::Relaxed);
    let chain = |slot: usize| match xbox_vt[slot].load(Ordering::Relaxed) { 0 => *real_vt.add(slot), p => p };
    PREV_DRAW.store(chain(XBOX_SLOT_DRAW), Ordering::Relaxed);
    PREV_DRAW_INDEXED.store(chain(XBOX_SLOT_DRAW_INDEXED), Ordering::Relaxed);
    xbox_vt[XBOX_SLOT_DRAW].store(draw_x as usize, Ordering::Relaxed);
    PREV_VIEWPORTS.store(chain(XBOX_SLOT_SET_VIEWPORTS), Ordering::Relaxed);
    PREV_SCISSORS.store(chain(XBOX_SLOT_SET_SCISSORS), Ordering::Relaxed);
    PREV_PSO.store(chain(XBOX_SLOT_SET_PSO), Ordering::Relaxed);
    xbox_vt[XBOX_SLOT_SET_VIEWPORTS].store(set_viewports_x as usize, Ordering::Relaxed);
    xbox_vt[XBOX_SLOT_SET_SCISSORS].store(set_scissors_x as usize, Ordering::Relaxed);
    xbox_vt[XBOX_SLOT_SET_PSO].store(set_pso_x as usize, Ordering::Relaxed);
    xbox_vt[XBOX_SLOT_DRAW_INDEXED].store(draw_indexed_x as usize, Ordering::Relaxed);
    record("d3d12_xs", "query_x", &format!("query hooks installed; un-nesting of occlusion queries = {}, force-visible = {}", unnest_enabled(), force_visible()));
}

fn with_nest<R>(list: usize, f: impl FnOnce(&mut Nest) -> R) -> R {
    let mut g = NESTS.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(HashMap::new).entry(list).or_default())
}

unsafe extern "system" fn reset_x(this: *mut c_void, alloc: *mut c_void, pso: *mut c_void) -> i32 {
    if let Some(m) = NESTS.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        m.remove(&(this as usize));
    }
    with_list(this as usize, |s| *s = ListState { pso: pso as usize, ..ListState::default() });
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32 = core::mem::transmute(REAL_RESET.load(Ordering::Relaxed));
    crate::ecl_probe::on_reset(this);
    f(this, alloc, pso)
}

unsafe extern "system" fn begin_query_x(this: *mut c_void, heap: *mut c_void, ty: u32, index: u32) {
    let begin: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, u32) = core::mem::transmute(REAL_BEGIN.load(Ordering::Relaxed));
    if BEGINS.fetch_add(1, Ordering::Relaxed) < 16 {
        record("d3d12_xs", "query_x", &format!("BeginQuery list={this:p} heap={heap:p} type={ty} index={index}"));
    }
    if is_occlusion(ty) && unnest_enabled() {
        let prev = with_nest(this as usize, |n| n.begin((heap as usize, index)));
        if let Some((ph, pi)) = prev {
            let end: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, u32) = core::mem::transmute(REAL_END.load(Ordering::Relaxed));
            end(this, ph as *mut c_void, ty, pi);
            if UNNESTED.fetch_add(1, Ordering::Relaxed) == 0 {
                record("d3d12_xs", "query_x", &format!("nested occlusion query: ended outer index {pi} before inner index {index} (PC D3D12 allows one active occlusion query per list)"));
            }
        }
    }
    begin(this, heap, ty, index);
    if is_occlusion(ty) {
        diag_end(this, None);
        diag_begin(this, heap as usize, index);
    }
}

const XBOX_SLOT_SET_VIEWPORTS: usize = 21;
const XBOX_SLOT_SET_SCISSORS: usize = 22;
const XBOX_SLOT_SET_PSO: usize = 25;
const DIAG_SLOTS: u32 = 48;
const STATS_BYTES: u64 = 88;
static PREV_VIEWPORTS: AtomicUsize = AtomicUsize::new(0);
static PREV_SCISSORS: AtomicUsize = AtomicUsize::new(0);
static PREV_PSO: AtomicUsize = AtomicUsize::new(0);

#[derive(Default, Clone, Copy)]
struct ListState {
    pso: usize,
    vp: [f32; 6],
    sc: [i32; 4],
    stats_slot: Option<u32>,
}

#[derive(Clone)]
struct SlotInfo {
    heap: usize,
    index: u32,
    st: ListState,
    draws: u32,
}

struct Diag {
    heap: usize,
    readback: usize,
    next: u32,
    slots: Vec<SlotInfo>,
    finished_at: Option<usize>,
    dumped: bool,
    rounds: u32,
}

const DIAG_REARM_RESOLVES: usize = 6000;
const DIAG_ROUNDS: u32 = 8;

static LISTS: Mutex<Option<HashMap<usize, ListState>>> = Mutex::new(None);
static DIAG: Mutex<Option<Diag>> = Mutex::new(None);

fn with_list<R>(list: usize, f: impl FnOnce(&mut ListState) -> R) -> R {
    let mut g = LISTS.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(HashMap::new).entry(list).or_default())
}

fn diag_enabled() -> bool {
    static SW: crate::live_sw::Sw = crate::live_sw::Sw::new("TOMBOX_QUERY_DIAG", "query_diag", true);
    SW.on()
}

unsafe extern "system" fn set_viewports_x(this: *mut c_void, n: u32, vps: *const [f32; 6]) {
    if n > 0 && !vps.is_null() {
        let v = *vps;
        with_list(this as usize, |s| s.vp = v);
    }
    let f: unsafe extern "system" fn(*mut c_void, u32, *const [f32; 6]) = core::mem::transmute(PREV_VIEWPORTS.load(Ordering::Relaxed));
    f(this, n, vps)
}

unsafe extern "system" fn set_scissors_x(this: *mut c_void, n: u32, rects: *const [i32; 4]) {
    if n > 0 && !rects.is_null() {
        let r = *rects;
        with_list(this as usize, |s| s.sc = r);
    }
    let f: unsafe extern "system" fn(*mut c_void, u32, *const [i32; 4]) = core::mem::transmute(PREV_SCISSORS.load(Ordering::Relaxed));
    f(this, n, rects)
}

unsafe extern "system" fn set_pso_x(this: *mut c_void, pso: *mut c_void) {
    with_list(this as usize, |s| s.pso = pso as usize);
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void) = core::mem::transmute(PREV_PSO.load(Ordering::Relaxed));
    f(this, pso)
}

unsafe fn create_diag(list: *mut c_void) -> Option<Diag> {
    let lvt = *(list as *const *const usize);
    let get_device: unsafe extern "system" fn(*mut c_void, *const Iid, *mut *mut c_void) -> i32 = core::mem::transmute(*lvt.add(7));
    let mut dev: *mut c_void = core::ptr::null_mut();
    if get_device(list, &IID_DEVICE, &mut dev) < 0 || dev.is_null() {
        return None;
    }
    let dvt = *(dev as *const *const usize);
    let create_heap: unsafe extern "system" fn(*mut c_void, *const [u32; 3], *const Iid, *mut *mut c_void) -> i32 = core::mem::transmute(*dvt.add(39));
    let create_res: unsafe extern "system" fn(*mut c_void, *const HeapProps, u32, *const BufferDesc, u32, *const c_void, *const Iid, *mut *mut c_void) -> i32 =
        core::mem::transmute(*dvt.add(27));
    let mut heap: *mut c_void = core::ptr::null_mut();
    let hr1 = create_heap(dev, &[2, DIAG_SLOTS, 0], &IID_QUERY_HEAP, &mut heap);
    let props = HeapProps { ty: 3, cpu_page: 0, pool: 0, creation_mask: 1, visible_mask: 1 };
    let desc = BufferDesc { dimension: 1, alignment: 0, width: DIAG_SLOTS as u64 * STATS_BYTES, height: 1, depth: 1, mips: 1, format: 0, sample_count: 1, sample_quality: 0, layout: 1, flags: 0 };
    let mut rb: *mut c_void = core::ptr::null_mut();
    let hr2 = create_res(dev, &props, 0, &desc, 0x400, core::ptr::null(), &IID_RESOURCE, &mut rb);
    let release: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(*dvt.add(2));
    release(dev);
    if hr1 < 0 || hr2 < 0 || heap.is_null() || rb.is_null() {
        record("d3d12_xs", "query_x", &format!("occlusion diag: pipeline-statistics heap hr={hr1:#010x} readback hr={hr2:#010x}; diag off"));
        return None;
    }
    record("d3d12_xs", "query_x", &format!("occlusion diag: pipeline statistics are captured around the first {DIAG_SLOTS} inner occlusion queries"));
    VP_SAMPLING.store(true, Ordering::Relaxed);
    Some(Diag { heap: heap as usize, readback: rb as usize, next: 0, slots: Vec::new(), finished_at: None, dumped: false, rounds: 0 })
}

unsafe fn diag_begin(list: *mut c_void, heap: usize, index: u32) {
    if !diag_enabled() {
        return;
    }
    let slot = {
        let mut g = DIAG.lock().unwrap_or_else(|e| e.into_inner());
        if g.is_none() {
            match create_diag(list) {
                Some(d) => *g = Some(d),
                None => return,
            }
        }
        let d = g.as_mut().unwrap();
        if d.next >= DIAG_SLOTS || d.heap == 0 {
            return;
        }
        let s = d.next;
        d.next += 1;
        let st = with_list(list as usize, |l| { l.stats_slot = Some(s); *l });
        d.slots.push(SlotInfo { heap, index, st, draws: 0 });
        (d.heap, s)
    };
    let begin: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, u32) = core::mem::transmute(REAL_BEGIN.load(Ordering::Relaxed));
    begin(list, slot.0 as *mut c_void, 3, slot.1);
}

unsafe fn diag_end(list: *mut c_void, draws: Option<u32>) {
    let Some((slot, now)) = with_list(list as usize, |l| l.stats_slot.take().map(|s| (s, *l))) else { return };
    let (heap, rb) = {
        let mut g = DIAG.lock().unwrap_or_else(|e| e.into_inner());
        let Some(d) = g.as_mut() else { return };
        if let Some(s) = d.slots.get_mut(slot as usize) {
            s.draws = draws.unwrap_or(0);
            s.st = now;
        }
        if slot + 1 == DIAG_SLOTS {
            d.finished_at = Some(RESOLVES.load(Ordering::Relaxed));
        }
        (d.heap, d.readback)
    };
    let end: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, u32) = core::mem::transmute(REAL_END.load(Ordering::Relaxed));
    end(list, heap as *mut c_void, 3, slot);
    let resolve: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, u32, u32, *mut c_void, u64) = core::mem::transmute(REAL_RESOLVE.load(Ordering::Relaxed));
    resolve(list, heap as *mut c_void, 3, slot, 1, rb as *mut c_void, slot as u64 * STATS_BYTES);
}

fn pso_text(pso: usize) -> String {
    let Some((d, _keep)) = crate::device_wrap::gfx_pso_desc(pso) else { return format!("pso {pso:#x} not registered") };
    if d.len() < 0x270 {
        return format!("pso {pso:#x} desc too short ({} bytes)", d.len());
    }
    let u = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
    format!(
        "sampleMask={:#x} fill={} cull={} frontCCW={} depthClip={} conservative={} depth(en={} write={} func={}) stencil(en={} read={:#x} write={:#x} front=[{} {} {} {}] back=[{} {} {} {}]) topology={} nRT={} dsv={} samples={}",
        u(0x1c0), u(0x1c4), u(0x1c8), u(0x1cc), u(0x1dc), u(0x1ec), u(0x1f0), u(0x1f4), u(0x1f8), u(0x1fc), d[0x200], d[0x201],
        u(0x204), u(0x208), u(0x20c), u(0x210), u(0x214), u(0x218), u(0x21c), u(0x220), u(0x23c), u(0x240), u(0x264), u(0x268))
}

unsafe fn diag_maybe_dump() {
    let mut g = DIAG.lock().unwrap_or_else(|e| e.into_inner());
    let Some(d) = g.as_mut() else { return };
    let Some(at) = d.finished_at else { return };
    let now = RESOLVES.load(Ordering::Relaxed);
    if d.dumped {
        if now >= at + DIAG_REARM_RESOLVES && d.rounds < DIAG_ROUNDS {
            d.rounds += 1;
            d.next = 0;
            d.slots.clear();
            d.finished_at = None;
            d.dumped = false;
            record("d3d12_xs", "query_x", &format!("occlusion diag round {} armed at resolve {now}", d.rounds + 1));
            VP_SAMPLING.store(true, Ordering::Relaxed);
        }
        return;
    }
    if now < at + 1024 {
        return;
    }
    d.dumped = true;
    VP_SAMPLING.store(false, Ordering::Relaxed);
    dump_viewport_histogram();
    let rvt = *(d.readback as *const *const usize);
    let map: unsafe extern "system" fn(*mut c_void, u32, *const [usize; 2], *mut *mut c_void) -> i32 = core::mem::transmute(*rvt.add(8));
    let unmap: unsafe extern "system" fn(*mut c_void, u32, *const [usize; 2]) = core::mem::transmute(*rvt.add(9));
    let range = [0usize, (DIAG_SLOTS as u64 * STATS_BYTES) as usize];
    let mut p: *mut c_void = core::ptr::null_mut();
    if map(d.readback as *mut c_void, 0, &range, &mut p) < 0 || p.is_null() {
        record("d3d12_xs", "query_x", "occlusion diag: Map of the statistics readback failed");
        return;
    }
    let all = core::slice::from_raw_parts(p as *const u64, (DIAG_SLOTS as u64 * STATS_BYTES / 8) as usize).to_vec();
    let empty = [0usize; 2];
    unmap(d.readback as *mut c_void, 0, &empty);
    let mut seen_pso = std::collections::HashSet::new();
    for (i, s) in d.slots.iter().enumerate() {
        let v = &all[i * 11..i * 11 + 11];
        record("d3d12_xs", "query_x", &format!(
            "occlusion diag #{i}: query heap={:#x} index={} draws={} | IAVertices={} IAPrimitives={} VSInvocations={} CInvocations={} CPrimitives={} PSInvocations={} | viewport=[{} {} {} {} {} {}] scissor=[{} {} {} {}] pso={:#x}",
            s.heap, s.index, s.draws, v[0], v[1], v[2], v[5], v[6], v[7],
            s.st.vp[0], s.st.vp[1], s.st.vp[2], s.st.vp[3], s.st.vp[4], s.st.vp[5], s.st.sc[0], s.st.sc[1], s.st.sc[2], s.st.sc[3], s.st.pso));
        if seen_pso.insert(s.st.pso) {
            record("d3d12_xs", "query_x", &format!("occlusion diag pso {:#x}: {}", s.st.pso, pso_text(s.st.pso)));
        }
    }
}

unsafe extern "system" fn end_query_x(this: *mut c_void, heap: *mut c_void, ty: u32, index: u32) {
    if is_occlusion(ty) && unnest_enabled() {
        let (forward, draws) = with_nest(this as usize, |n| {
            let f = n.end((heap as usize, index));
            (f, n.last_draws)
        });
        match draws {
            Some(0) => {
                QUERIES_WITHOUT_DRAWS.fetch_add(1, Ordering::Relaxed);
            }
            Some(d) => {
                QUERIES_WITH_DRAWS.fetch_add(1, Ordering::Relaxed);
                DRAWS_IN_QUERIES.fetch_add(d as usize, Ordering::Relaxed);
            }
            None => {}
        }
        if !forward {
            SWALLOWED.fetch_add(1, Ordering::Relaxed);
            return;
        }
    }
    let end: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, u32) = core::mem::transmute(REAL_END.load(Ordering::Relaxed));
    end(this, heap, ty, index);
    if is_occlusion(ty) {
        let draws = if unnest_enabled() { with_nest(this as usize, |n| n.last_draws) } else { None };
        diag_end(this, draws);
    }
}

#[derive(Clone, Copy)]
struct Resolve {
    dest: usize,
    offset: u64,
    count: u32,
}
static RING: Mutex<Vec<Resolve>> = Mutex::new(Vec::new());
const RING_DELAY: usize = 512;
const RING_BATCH: usize = 256;

unsafe fn com_call(obj: usize, slot: usize) {
    let vt = *(obj as *const *const usize);
    let f: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(*vt.add(slot));
    f(obj as *mut c_void);
}

unsafe fn tally_results(batch: &[Resolve]) -> (u64, u64, u64, u32) {
    let (mut zero, mut nonzero, mut sum, mut failed) = (0u64, 0u64, 0u64, 0u32);
    for r in batch {
        if r.count == 0 || r.count > 65536 {
            continue;
        }
        let vt = *(r.dest as *const *const usize);
        let map: unsafe extern "system" fn(*mut c_void, u32, *const [usize; 2], *mut *mut c_void) -> i32 = core::mem::transmute(*vt.add(8));
        let unmap: unsafe extern "system" fn(*mut c_void, u32, *const [usize; 2]) = core::mem::transmute(*vt.add(9));
        let range = [r.offset as usize, r.offset as usize + r.count as usize * 8];
        let mut p: *mut c_void = core::ptr::null_mut();
        if map(r.dest as *mut c_void, 0, &range, &mut p) < 0 || p.is_null() {
            failed += 1;
            continue;
        }
        let vals = core::slice::from_raw_parts((p as *const u8).add(r.offset as usize) as *const u64, r.count as usize);
        for v in vals {
            if *v == 0 { zero += 1 } else { nonzero += 1; sum = sum.saturating_add(*v) }
        }
        let empty = [0usize; 2];
        unmap(r.dest as *mut c_void, 0, &empty);
    }
    (zero, nonzero, sum, failed)
}

unsafe fn force_results(list: *mut c_void, count: u32, dest: *mut c_void, offset: u64) {
    if count == 0 || count as u64 > ONES_SLOTS {
        return;
    }
    let ones = ones_buffer(list);
    if ones == 0 {
        return;
    }
    let lvt = *(list as *const *const usize);
    let copy: unsafe extern "system" fn(*mut c_void, *mut c_void, u64, *mut c_void, u64, u64) = core::mem::transmute(*lvt.add(PC_SLOT_COPY_BUFFER_REGION));
    copy(list, dest, offset, ones as *mut c_void, 0, count as u64 * 8);
    FORCED.fetch_add(count as usize, Ordering::Relaxed);
}

unsafe extern "system" fn resolve_query_data_x(this: *mut c_void, heap: *mut c_void, ty: u32, start: u32, count: u32, dest: *mut c_void, offset: u64) {
    let n = RESOLVES.fetch_add(1, Ordering::Relaxed);
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, u32, u32, *mut c_void, u64) = core::mem::transmute(REAL_RESOLVE.load(Ordering::Relaxed));
    f(this, heap, ty, start, count, dest, offset);
    crate::ts99::after_resolve(this, heap, ty, start, count, dest, offset);
    if !is_occlusion(ty) || dest.is_null() {
        return;
    }
    if n < 12 {
        record("d3d12_xs", "query_x", &format!("ResolveQueryData type={ty} heap={heap:p} start={start} count={count} dest={dest:p} offset={offset:#x}"));
    }
    if force_visible() {
        force_results(this, count, dest, offset);
    }
    com_call(dest as usize, 1);
    let batch: Vec<Resolve> = {
        let mut ring = RING.lock().unwrap_or_else(|e| e.into_inner());
        ring.push(Resolve { dest: dest as usize, offset, count });
        if ring.len() >= RING_DELAY + RING_BATCH { ring.drain(..RING_BATCH).collect() } else { Vec::new() }
    };
    diag_maybe_dump();
    if batch.is_empty() {
        return;
    }
    let (zero, nonzero, sum, failed) = tally_results(&batch);
    for r in &batch {
        com_call(r.dest, 2);
    }
    RESULTS_ZERO.fetch_add(zero as usize, Ordering::Relaxed);
    RESULTS_NONZERO.fetch_add(nonzero as usize, Ordering::Relaxed);
    RESULTS_SUM.fetch_add(sum.min(usize::MAX as u64) as usize, Ordering::Relaxed);
    let with = QUERIES_WITH_DRAWS.load(Ordering::Relaxed);
    record("d3d12_xs", "query_x", &format!(
        "resolves={} begins={} unnested={} swallowed-outer-ends={} predications={} | last {} resolves: {} results zero={zero} nonzero={nonzero} mean-nonzero={} map-failed={failed} | totals zero={} nonzero={} | queries ended with draws={with} without={} draws-in-queries={} forced={}",
        n + 1, BEGINS.load(Ordering::Relaxed), UNNESTED.load(Ordering::Relaxed), SWALLOWED.load(Ordering::Relaxed),
        PREDICATIONS.load(Ordering::Relaxed), batch.len(), zero + nonzero,
        if nonzero > 0 { sum / nonzero } else { 0 },
        RESULTS_ZERO.load(Ordering::Relaxed), RESULTS_NONZERO.load(Ordering::Relaxed),
        QUERIES_WITHOUT_DRAWS.load(Ordering::Relaxed), DRAWS_IN_QUERIES.load(Ordering::Relaxed), FORCED.load(Ordering::Relaxed)));
}
unsafe extern "system" fn set_predication_x(this: *mut c_void, buf: *mut c_void, offset: u64, op: u32) {
    if PREDICATIONS.fetch_add(1, Ordering::Relaxed) < 4 {
        record("d3d12_xs", "query_x", &format!("SetPredication(buf={buf:p} offset={offset:#x} op={op})"));
    }
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void, u64, u32) = core::mem::transmute(REAL_PRED.load(Ordering::Relaxed));
    f(this, buf, offset, op)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outer_query_is_ended_before_inner_and_its_end_swallowed() {
        let mut n = Nest::default();
        let outer = (1, 0);
        assert_eq!(n.begin(outer), None);
        assert_eq!(n.begin((1, 1)), Some(outer));
        assert!(n.end((1, 1)));
        assert_eq!(n.begin((1, 2)), None);
        assert!(n.end((1, 2)));
        assert!(!n.end(outer));
        assert!(n.active.is_none() && n.ended_early.is_empty());
    }

    #[test]
    fn draws_are_counted_for_the_active_query_only() {
        let mut n = Nest::default();
        n.draw();
        n.begin((1, 0));
        n.begin((1, 1));
        n.draw();
        n.draw();
        assert!(n.end((1, 1)));
        assert_eq!(n.last_draws, Some(2));
        n.draw();
        assert!(!n.end((1, 0)));
        assert_eq!(n.last_draws, None);
        n.begin((1, 2));
        assert!(n.end((1, 2)));
        assert_eq!(n.last_draws, Some(0));
    }

    #[test]
    fn plain_sequence_passes_through() {
        let mut n = Nest::default();
        for i in 0..4 {
            assert_eq!(n.begin((7, i)), None);
            assert!(n.end((7, i)));
        }
        assert!(n.end((7, 99)));
    }

    #[test]
    fn query_slots() {
        assert_eq!(XBOX_SLOT_BEGIN_QUERY + 24, PC_SLOT_BEGIN_QUERY);
        assert_eq!(XBOX_SLOT_SET_PREDICATION + 24, PC_SLOT_SET_PREDICATION);
    }
}
