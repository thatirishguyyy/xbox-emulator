use compat_common::record;
use core::ffi::c_void;
use core::sync::atomic::{AtomicUsize, Ordering};
use std::collections::HashMap;
use std::sync::Mutex;

pub const PAGE: u64 = 0x1_0000;
const SRV: usize = 18;
const UAV: usize = 19;
const CL_COPY_BUFFER: usize = 15;
const CL_COPY_TEXTURE: usize = 16;
const CL_COPY_RESOURCE: usize = 17;
const CL_RESOLVE_QUERY: usize = 30;
const CL_PREDICATION: usize = 31;
const CL_DISCARD: usize = 55;
const CL_INDIRECT: usize = 59;
const CL_COPY_RESOURCE_X: usize = 84;
const CL_COPY_TEXTURE_X: usize = 83;
const RES_GET_DESC: usize = 10;
const DEV_PLACED: usize = 29;
const LOCATION: usize = 0x30;
const IID_RESOURCE: Guid = Guid(0x696442be, 0xa72e, 0x4059, [0xbc, 0x79, 0x5b, 0x5c, 0x98, 0x04, 0x0f, 0xad]);
const IID_UNKNOWN: Guid = Guid(0, 0, 0, [0xc0, 0, 0, 0, 0, 0, 0, 0x46]);
const WATCH_TAG: Guid = Guid(0x0ff5_e7a1, 0x7c3b, 0x4d52, [0x9e, 0x41, 0x2a, 0x6d, 0x0b, 0x93, 0xc4, 0x17]);

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Guid(u32, u16, u16, [u8; 8]);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Off {
    pub bytes: u64,
    pub width: u64,
    pub heap: usize,
    pub heap_off: u64,
    pub real: usize,
    pub flags: u32,
}

static TABLE: Mutex<Option<HashMap<usize, Off>>> = Mutex::new(None);
static LIVE: AtomicUsize = AtomicUsize::new(0);
static UNEVEN: AtomicUsize = AtomicUsize::new(0);
static PREV_SRV: AtomicUsize = AtomicUsize::new(0);
static PREV_UAV: AtomicUsize = AtomicUsize::new(0);
static PREV_CL: [AtomicUsize; 96] = [const { AtomicUsize::new(0) }; 96];
static HELPERS: Mutex<Option<HashMap<(usize, u64), usize>>> = Mutex::new(None);
static DESC_VT: [AtomicUsize; 16] = [const { AtomicUsize::new(0) }; 16];
static DESC_ORIG: [AtomicUsize; 16] = [const { AtomicUsize::new(0) }; 16];
static DESC_LOCK: Mutex<()> = Mutex::new(());

thread_local! {
    static RUNTIME_VIEW: core::cell::Cell<u32> = const { core::cell::Cell::new(0) };
}

pub fn with_true_desc<R>(f: impl FnOnce() -> R) -> R {
    RUNTIME_VIEW.with(|c| c.set(c.get() + 1));
    let r = f();
    RUNTIME_VIEW.with(|c| c.set(c.get() - 1));
    r
}

pub fn plan(va: u64, desc: &[u8; 0x38]) -> Option<(u64, u64, [u8; 0x38])> {
    let dim = u32::from_le_bytes(desc[0..4].try_into().unwrap());
    let o = va % PAGE;
    if dim != 1 || o == 0 || va == 0 {
        return None;
    }
    let width = u64::from_le_bytes(desc[0x10..0x18].try_into().unwrap());
    let mut w = *desc;
    w[0x10..0x18].copy_from_slice(&width.checked_add(o)?.to_le_bytes());
    Some((va - o, o, w))
}

pub fn set(res: usize, off: Off) {
    let mut g = TABLE.lock().unwrap_or_else(|e| e.into_inner());
    if g.get_or_insert_with(HashMap::new).insert(res, off).is_none() {
        LIVE.fetch_add(1, Ordering::AcqRel);
    }
}

pub fn of(res: usize) -> Option<Off> {
    if LIVE.load(Ordering::Acquire) == 0 {
        return None;
    }
    TABLE.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(&res).copied())
}

pub fn bytes(res: usize) -> u64 {
    of(res).map_or(0, |o| o.bytes)
}

pub fn forget(res: usize) {
    if LIVE.load(Ordering::Acquire) == 0 {
        return;
    }
    let gone = {
        let mut g = TABLE.lock().unwrap_or_else(|e| e.into_inner());
        g.as_mut().and_then(|m| m.remove(&res)).is_some()
    };
    if !gone {
        return;
    }
    LIVE.fetch_sub(1, Ordering::AcqRel);
    let helpers: Vec<usize> = {
        let mut g = HELPERS.lock().unwrap_or_else(|e| e.into_inner());
        match g.as_mut() {
            Some(m) => {
                let keys: Vec<(usize, u64)> = m.keys().filter(|k| k.0 == res).copied().collect();
                keys.iter().filter_map(|k| m.remove(k)).collect()
            }
            None => Vec::new(),
        }
    };
    for h in helpers {
        crate::title_shadow::helper_gone(h);
        unsafe { release(h as *mut c_void) };
    }
}

unsafe fn vt(o: *mut c_void) -> *const usize {
    unsafe { *(o as *const *const usize) }
}

unsafe fn release(o: *mut c_void) -> u32 {
    let f: unsafe extern "system" fn(*mut c_void) -> u32 = unsafe { core::mem::transmute(*vt(o).add(2)) };
    unsafe { f(o) }
}

#[repr(C)]
struct Watch {
    vtbl: *const usize,
    refs: AtomicUsize,
    res: usize,
}

unsafe extern "system" fn watch_qi(this: *mut c_void, riid: *const Guid, out: *mut *mut c_void) -> i32 {
    if out.is_null() {
        return 0x8000_4003u32 as i32;
    }
    if unsafe { *riid } == IID_UNKNOWN {
        unsafe { watch_addref(this) };
        unsafe { *out = this };
        return 0;
    }
    unsafe { *out = core::ptr::null_mut() };
    0x8000_4002u32 as i32
}

unsafe extern "system" fn watch_addref(this: *mut c_void) -> u32 {
    (unsafe { (*(this as *mut Watch)).refs.fetch_add(1, Ordering::AcqRel) } + 1) as u32
}

unsafe extern "system" fn watch_release(this: *mut c_void) -> u32 {
    let w = this as *mut Watch;
    let n = unsafe { (*w).refs.fetch_sub(1, Ordering::AcqRel) } - 1;
    if n == 0 {
        forget(unsafe { (*w).res });
        drop(unsafe { Box::from_raw(w) });
    }
    n as u32
}

#[repr(C)]
struct WatchVtbl {
    qi: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32,
    addref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
}

static WATCH_VTBL: WatchVtbl = WatchVtbl { qi: watch_qi, addref: watch_addref, release: watch_release };

unsafe fn attach_watch(res: *mut c_void) -> bool {
    let w = Box::into_raw(Box::new(Watch { vtbl: &WATCH_VTBL as *const WatchVtbl as *const usize, refs: AtomicUsize::new(1), res: res as usize }));
    let set: unsafe extern "system" fn(*mut c_void, *const Guid, *mut c_void) -> i32 = unsafe { core::mem::transmute(*vt(res).add(5)) };
    let hr = unsafe { set(res, &WATCH_TAG, w as *mut c_void) };
    unsafe { watch_release(w as *mut c_void) };
    hr >= 0
}

pub fn keep_or_abandon(res: usize, attached: bool, drop_res: impl FnOnce()) -> bool {
    if attached {
        return true;
    }
    forget(res);
    drop_res();
    false
}

unsafe extern "system" fn get_desc_x(this: *mut c_void, out: *mut u8) -> *mut u8 {
    let vtv = unsafe { *(this as *const usize) };
    let orig = DESC_VT.iter().position(|p| p.load(Ordering::Acquire) == vtv).map_or(0, |i| DESC_ORIG[i].load(Ordering::Acquire));
    if orig == 0 {
        return out;
    }
    let f: unsafe extern "system" fn(*mut c_void, *mut u8) -> *mut u8 = unsafe { core::mem::transmute(orig) };
    let r = unsafe { f(this, out) };
    if RUNTIME_VIEW.with(|c| c.get()) != 0 {
        return r;
    }
    if let Some(o) = of(this as usize) {
        if !r.is_null() {
            unsafe { core::ptr::write_unaligned(r.add(0x10) as *mut u64, o.width) };
        }
    }
    r
}

unsafe fn patch_get_desc(res: *mut c_void) -> bool {
    extern "system" {
        fn VirtualProtect(addr: *mut c_void, size: usize, new_protect: u32, old_protect: *mut u32) -> i32;
    }
    let vtv = unsafe { *(res as *const usize) };
    if DESC_VT.iter().any(|p| p.load(Ordering::Acquire) == vtv) {
        return true;
    }
    let _g = DESC_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if DESC_VT.iter().any(|p| p.load(Ordering::Acquire) == vtv) {
        return true;
    }
    let slot = (vtv as *mut usize).wrapping_add(RES_GET_DESC);
    let orig = unsafe { *slot };
    if orig == get_desc_x as usize {
        return true;
    }
    let Some(i) = DESC_VT.iter().position(|p| p.load(Ordering::Acquire) == 0) else { return false };
    DESC_ORIG[i].store(orig, Ordering::Release);
    DESC_VT[i].store(vtv, Ordering::Release);
    let mut old = 0u32;
    unsafe {
        if VirtualProtect(slot as *mut c_void, 8, 0x04, &mut old) == 0 {
            DESC_VT[i].store(0, Ordering::Release);
            return false;
        }
        *slot = get_desc_x as usize;
        VirtualProtect(slot as *mut c_void, 8, old, &mut old);
    }
    true
}

pub fn true_entry(e: crate::gpu_va::Entry, a: u64) -> (crate::gpu_va::Entry, u64) {
    let o = bytes(e.res);
    let start = e.start.saturating_sub(o);
    (crate::gpu_va::Entry { start, len: e.len + (e.start - start), res: e.res, buffer: e.buffer }, a - start)
}

pub fn map_pointer(res: usize, p: u64, title_va: Option<u64>) -> u64 {
    let o = bytes(res);
    if o == 0 || p == 0 || title_va == Some(p) { p } else { p + o }
}

pub unsafe fn create(real: *mut c_void, va: u64, d: &[u8; 0x38], xbox_size: u64, init_state: u32, pclear: *const c_void, ppv: *mut *mut c_void) -> Option<crate::alias::Made> {
    let (base, o, wide) = plan(va, d)?;
    let width = u64::from_le_bytes(d[0x10..0x18].try_into().unwrap());
    let made = unsafe { crate::alias::create_placed_x(real, base, &wide, xbox_size.max(width + o), init_state, pclear, ppv) }?;
    let res = unsafe { *ppv };
    let flags = u32::from_le_bytes(d[0x30..0x34].try_into().unwrap());
    set(res as usize, Off { bytes: o, width, heap: made.heap, heap_off: made.offset, real: real as usize, flags });
    let ok = unsafe { attach_watch(res) } && unsafe { patch_get_desc(res) };
    if !keep_or_abandon(res as usize, ok, || unsafe { release(res); *ppv = core::ptr::null_mut(); }) {
        record("d3d12_xs", "placed_offset", &format!("buffer va={va:#x}: release watch or GetDesc hook could not be attached, so the page-sharing placement was dropped and a committed resource is used"));
        return None;
    }
    static N: AtomicUsize = AtomicUsize::new(0);
    if N.fetch_add(1, Ordering::Relaxed) < 16 {
        record("d3d12_xs", "placed_offset", &format!("buffer va={va:#x} width={width:#x} placed at the 64 KB base {base:#x} of {:?} heap {:#x}+{:#x}, resource GPU VA offset {o:#x}", made.kind, made.heap, made.offset));
    }
    Some(made)
}

pub unsafe fn place_in_heap(real: *mut c_void, heap: *mut c_void, offset: u64, d: &[u8; 0x38], init_state: u32, pclear: *const c_void, ppv: *mut *mut c_void) -> Option<i32> {
    let (base, o, wide) = plan(offset, d)?;
    let width = u64::from_le_bytes(d[0x10..0x18].try_into().unwrap());
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void, u64, *const u8, u32, *const c_void, *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(*vt(real).add(DEV_PLACED)) };
    let hr = unsafe { f(real, heap, base, wide.as_ptr(), init_state, pclear, &IID_RESOURCE, ppv) };
    if hr < 0 || unsafe { *ppv }.is_null() {
        record("d3d12_xs", "placed_offset", &format!("heap {heap:p} offset {offset:#x}: buffer widened to {:#x} at the 64 KB base {base:#x} was refused hr={hr:#010x}", width + o));
        return Some(hr);
    }
    let res = unsafe { *ppv };
    let flags = u32::from_le_bytes(d[0x30..0x34].try_into().unwrap());
    set(res as usize, Off { bytes: o, width, heap: heap as usize, heap_off: base, real: real as usize, flags });
    let ok = unsafe { attach_watch(res) } && unsafe { patch_get_desc(res) };
    if !keep_or_abandon(res as usize, ok, || unsafe { release(res); *ppv = core::ptr::null_mut(); }) {
        record("d3d12_xs", "placed_offset", &format!("heap {heap:p} offset {offset:#x}: release watch or GetDesc hook could not be attached, so the placement was dropped"));
        return Some(0x8000_4005u32 as i32);
    }
    static N: AtomicUsize = AtomicUsize::new(0);
    if N.fetch_add(1, Ordering::Relaxed) < 16 {
        record("d3d12_xs", "placed_offset", &format!("CreatePlacedResource heap {heap:p} offset {offset:#x} width={width:#x}: console packing kept by placing at the 64 KB base {base:#x}, resource GPU VA offset {o:#x}"));
    }
    Some(hr)
}

pub fn helper_plan(pos: u64, stride: u64, heap_off: u64) -> Option<(u64, u64)> {
    if stride == 0 {
        return None;
    }
    if pos % stride == 0 {
        return Some((0, pos / stride));
    }
    let mut m = 1u64;
    while m <= stride && m * PAGE <= heap_off {
        let p = pos + m * PAGE;
        if p % stride == 0 {
            return Some((m, p / stride));
        }
        m += 1;
    }
    None
}

unsafe fn helper(res: usize, off: Off, m: u64) -> Option<*mut c_void> {
    if m == 0 {
        return Some(res as *mut c_void);
    }
    if let Some(h) = HELPERS.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|t| t.get(&(res, m)).copied()) {
        return Some(h as *mut c_void);
    }
    let mut d = [0u8; 0x38];
    d[0..4].copy_from_slice(&1u32.to_le_bytes());
    d[0x10..0x18].copy_from_slice(&(m * PAGE + off.bytes + off.width).to_le_bytes());
    d[0x18..0x1c].copy_from_slice(&1u32.to_le_bytes());
    d[0x1c..0x1e].copy_from_slice(&1u16.to_le_bytes());
    d[0x1e..0x20].copy_from_slice(&1u16.to_le_bytes());
    d[0x24..0x28].copy_from_slice(&1u32.to_le_bytes());
    d[0x2c..0x30].copy_from_slice(&1u32.to_le_bytes());
    d[0x30..0x34].copy_from_slice(&(off.flags & 0xff).to_le_bytes());
    let real = off.real as *mut c_void;
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void, u64, *const u8, u32, *const c_void, *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(*vt(real).add(DEV_PLACED)) };
    let mut h: *mut c_void = core::ptr::null_mut();
    let hr = if crate::title_shadow::is_shadow_heap(off.heap) {
        unsafe { crate::title_shadow::place_helper(real, off.heap, off.heap_off - m * PAGE, &d, &mut h) }
    } else {
        unsafe { f(real, off.heap as *mut c_void, off.heap_off - m * PAGE, d.as_ptr(), 0, core::ptr::null(), &IID_RESOURCE, &mut h) }
    };
    if hr < 0 || h.is_null() {
        record("d3d12_xs", "placed_offset", &format!("view helper buffer at heap {:#x}+{:#x} hr={hr:#010x}", off.heap, off.heap_off - m * PAGE));
        return None;
    }
    HELPERS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert((res, m), h as usize);
    Some(h)
}

pub unsafe fn exact_view(res: usize, view: &mut [u8], at: crate::console_slots_dev2::BufferView) -> Option<*mut c_void> {
    let off = of(res)?;
    let e = crate::console_slots_dev2::element_bytes(view, at)? as u64;
    let first = u64::from_le_bytes(view[at.first..at.first + 8].try_into().unwrap());
    let (m, f) = helper_plan(off.bytes + first * e, e, off.heap_off)?;
    let target = unsafe { helper(res, off, m) }?;
    view[at.first..at.first + 8].copy_from_slice(&f.to_le_bytes());
    Some(target)
}

pub fn rebase_view(view: &mut [u8], at: crate::console_slots_dev2::BufferView, o: u64) -> bool {
    if crate::console_slots_dev2::rebase_buffer_view(view, at, o) {
        return true;
    }
    let e = crate::console_slots_dev2::element_bytes(view, at).unwrap_or(1).max(1) as u64;
    let first = u64::from_le_bytes(view[at.first..at.first + 8].try_into().unwrap()) + o / e;
    view[at.first..at.first + 8].copy_from_slice(&first.to_le_bytes());
    false
}

fn uneven(what: &str, o: u64, view: &[u8], at: crate::console_slots_dev2::BufferView) {
    let n = UNEVEN.fetch_add(1, Ordering::Relaxed);
    if n < 16 || n.is_power_of_two() {
        let e = crate::console_slots_dev2::element_bytes(view, at);
        record("d3d12_xs", "placed_offset", &format!("{what} on a buffer {o:#x} bytes into its 64 KB page: element size {e:?} does not divide the offset and no 64 KB step below it in the heap lines up, so the PC view starts {} bytes early (#{})", e.map_or(0, |e| o % e as u64), n + 1));
    }
}

unsafe extern "system" fn srv(this: *mut c_void, res: *mut c_void, desc: *const u8, dest: usize) {
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void, *const u8, usize) = unsafe { core::mem::transmute(PREV_SRV.load(Ordering::Acquire)) };
    let o = bytes(res as usize);
    if o == 0 || desc.is_null() || unsafe { *(desc.add(4) as *const u32) } != 1 {
        return unsafe { f(this, res, desc, dest) };
    }
    let mut v = [0u8; 0x28];
    unsafe { core::ptr::copy_nonoverlapping(desc, v.as_mut_ptr(), 0x28) };
    let orig = v;
    if let Some(t) = unsafe { exact_view(res as usize, &mut v, crate::console_slots_dev2::SRV_BUFFER) } {
        return with_true_desc(|| unsafe { f(this, t, v.as_ptr(), dest) });
    }
    v = orig;
    rebase_view(&mut v, crate::console_slots_dev2::SRV_BUFFER, o);
    uneven("CreateShaderResourceView", o, &v, crate::console_slots_dev2::SRV_BUFFER);
    with_true_desc(|| unsafe { f(this, res, v.as_ptr(), dest) })
}

unsafe extern "system" fn uav(this: *mut c_void, res: *mut c_void, counter: *mut c_void, desc: *const u8, dest: usize) {
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, *const u8, usize) = unsafe { core::mem::transmute(PREV_UAV.load(Ordering::Acquire)) };
    let o = bytes(res as usize);
    let oc = bytes(counter as usize);
    if (o == 0 && oc == 0) || desc.is_null() || unsafe { *(desc.add(4) as *const u32) } != 1 {
        return unsafe { f(this, res, counter, desc, dest) };
    }
    let mut v = [0u8; 0x28];
    unsafe { core::ptr::copy_nonoverlapping(desc, v.as_mut_ptr(), 0x28) };
    let co = u64::from_le_bytes(v[0x18..0x20].try_into().unwrap()) + oc;
    v[0x18..0x20].copy_from_slice(&co.to_le_bytes());
    let mut target = res;
    if o != 0 {
        let orig = v;
        match unsafe { exact_view(res as usize, &mut v, crate::console_slots_dev2::UAV_BUFFER) } {
            Some(t) => target = t,
            None => {
                v = orig;
                rebase_view(&mut v, crate::console_slots_dev2::UAV_BUFFER, o);
                uneven("CreateUnorderedAccessView", o, &v, crate::console_slots_dev2::UAV_BUFFER);
            }
        }
    }
    with_true_desc(|| unsafe { f(this, target, counter, v.as_ptr(), dest) })
}

pub fn install(v: &mut [usize]) {
    PREV_SRV.store(v[SRV], Ordering::Release);
    PREV_UAV.store(v[UAV], Ordering::Release);
    v[SRV] = srv as usize;
    v[UAV] = uav as usize;
}

fn prev(slot: usize) -> usize {
    PREV_CL[slot].load(Ordering::Acquire)
}

unsafe extern "system" fn copy_buffer(this: *mut c_void, dst: *mut c_void, doff: u64, src: *mut c_void, soff: u64, n: u64) {
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void, u64, *mut c_void, u64, u64) = unsafe { core::mem::transmute(prev(CL_COPY_BUFFER)) };
    with_true_desc(|| unsafe { f(this, dst, doff + bytes(dst as usize), src, soff + bytes(src as usize), n) })
}

fn shift_location(loc: &mut [u8; LOCATION]) -> bool {
    let res = u64::from_le_bytes(loc[0..8].try_into().unwrap()) as usize;
    let o = bytes(res);
    if o == 0 || u32::from_le_bytes(loc[8..12].try_into().unwrap()) != 1 {
        return false;
    }
    let off = u64::from_le_bytes(loc[0x10..0x18].try_into().unwrap()) + o;
    loc[0x10..0x18].copy_from_slice(&off.to_le_bytes());
    true
}

unsafe extern "system" fn copy_texture(this: *mut c_void, dst: *const u8, x: u32, y: u32, z: u32, src: *const u8, src_box: *const c_void) {
    let f: unsafe extern "system" fn(*mut c_void, *const u8, u32, u32, u32, *const u8, *const c_void) = unsafe { core::mem::transmute(prev(CL_COPY_TEXTURE)) };
    if LIVE.load(Ordering::Acquire) == 0 || dst.is_null() || src.is_null() {
        return unsafe { f(this, dst, x, y, z, src, src_box) };
    }
    let (d, s) = unsafe { shifted(dst, src) };
    with_true_desc(|| unsafe { f(this, d.as_ref().map_or(dst, |b| b.as_ptr()), x, y, z, s.as_ref().map_or(src, |b| b.as_ptr()), src_box) })
}

unsafe fn shifted(dst: *const u8, src: *const u8) -> (Option<[u8; LOCATION]>, Option<[u8; LOCATION]>) {
    let mut d = [0u8; LOCATION];
    let mut s = [0u8; LOCATION];
    unsafe {
        core::ptr::copy_nonoverlapping(dst, d.as_mut_ptr(), LOCATION);
        core::ptr::copy_nonoverlapping(src, s.as_mut_ptr(), LOCATION);
    }
    (shift_location(&mut d).then_some(d), shift_location(&mut s).then_some(s))
}

unsafe extern "system" fn copy_texture_x(this: *mut c_void, dst: *const u8, x: u32, y: u32, z: u32, src: *const u8, src_box: *const c_void, flags: u32) {
    let f: unsafe extern "system" fn(*mut c_void, *const u8, u32, u32, u32, *const u8, *const c_void, u32) = unsafe { core::mem::transmute(prev(CL_COPY_TEXTURE_X)) };
    if LIVE.load(Ordering::Acquire) == 0 || dst.is_null() || src.is_null() {
        return unsafe { f(this, dst, x, y, z, src, src_box, flags) };
    }
    let (d, s) = unsafe { shifted(dst, src) };
    with_true_desc(|| unsafe { f(this, d.as_ref().map_or(dst, |b| b.as_ptr()), x, y, z, s.as_ref().map_or(src, |b| b.as_ptr()), src_box, flags) })
}


unsafe fn buffer_width(res: *mut c_void) -> Option<u64> {
    let mut d = [0u8; 0x38];
    unsafe {
        let g: unsafe extern "system" fn(*mut c_void, *mut u8) -> *mut u8 = core::mem::transmute(*(*(res as *const *const usize)).add(RES_GET_DESC));
        g(res, d.as_mut_ptr());
    }
    (u32::from_le_bytes(d[0..4].try_into().unwrap()) == 1).then(|| u64::from_le_bytes(d[0x10..0x18].try_into().unwrap()))
}

pub fn resource_copy_span(dst: Option<Off>, dst_width: Option<u64>, src: Option<Off>, src_width: Option<u64>) -> Option<(u64, u64, u64)> {
    if dst.is_none() && src.is_none() {
        return None;
    }
    let dw = dst.map(|o| o.width).or(dst_width)?;
    let sw = src.map(|o| o.width).or(src_width)?;
    Some((dst.map_or(0, |o| o.bytes), src.map_or(0, |o| o.bytes), dw.min(sw)))
}

unsafe extern "system" fn copy_resource(this: *mut c_void, dst: *mut c_void, src: *mut c_void) {
    let (od, os) = (of(dst as usize), of(src as usize));
    if od.is_some() || os.is_some() {
        let dw = if od.is_none() { unsafe { buffer_width(dst) } } else { None };
        let sw = if os.is_none() { unsafe { buffer_width(src) } } else { None };
        if let Some((a, b, n)) = resource_copy_span(od, dw, os, sw) {
            crate::placed::note_gpu_write(dst);
            let f: unsafe extern "system" fn(*mut c_void, *mut c_void, u64, *mut c_void, u64, u64) = unsafe { core::mem::transmute(prev(CL_COPY_BUFFER)) };
            return with_true_desc(|| unsafe { f(this, dst, a, src, b, n) });
        }
    }
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) = unsafe { core::mem::transmute(prev(CL_COPY_RESOURCE)) };
    unsafe { f(this, dst, src) }
}

unsafe extern "system" fn copy_resource_x(this: *mut c_void, dst: *mut c_void, src: *mut c_void, flags: u32) {
    let (od, os) = (of(dst as usize), of(src as usize));
    if od.is_some() || os.is_some() {
        let dw = if od.is_none() { unsafe { buffer_width(dst) } } else { None };
        let sw = if os.is_none() { unsafe { buffer_width(src) } } else { None };
        if let Some((a, b, n)) = resource_copy_span(od, dw, os, sw) {
            crate::placed::note_gpu_write(dst);
            let f: unsafe extern "system" fn(*mut c_void, *mut c_void, u64, *mut c_void, u64, u64) = unsafe { core::mem::transmute(prev(CL_COPY_BUFFER)) };
            return with_true_desc(|| unsafe { f(this, dst, a, src, b, n) });
        }
    }
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, u32) = unsafe { core::mem::transmute(prev(CL_COPY_RESOURCE_X)) };
    unsafe { f(this, dst, src, flags) }
}

unsafe extern "system" fn resolve_query(this: *mut c_void, heap: *mut c_void, ty: u32, start: u32, n: u32, dst: *mut c_void, off: u64) {
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, u32, u32, *mut c_void, u64) = unsafe { core::mem::transmute(prev(CL_RESOLVE_QUERY)) };
    with_true_desc(|| unsafe { f(this, heap, ty, start, n, dst, off + bytes(dst as usize)) })
}

unsafe extern "system" fn predication(this: *mut c_void, buf: *mut c_void, off: u64, op: u32) {
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void, u64, u32) = unsafe { core::mem::transmute(prev(CL_PREDICATION)) };
    with_true_desc(|| unsafe { f(this, buf, off + bytes(buf as usize), op) })
}

unsafe extern "system" fn indirect(this: *mut c_void, sig: *mut c_void, max: u32, arg: *mut c_void, arg_off: u64, cnt: *mut c_void, cnt_off: u64) {
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut c_void, u64, *mut c_void, u64) = unsafe { core::mem::transmute(prev(CL_INDIRECT)) };
    with_true_desc(|| unsafe { f(this, sig, max, arg, arg_off + bytes(arg as usize), cnt, cnt_off + bytes(cnt as usize)) })
}

unsafe extern "system" fn discard(this: *mut c_void, res: *mut c_void, region: *const c_void) {
    if bytes(res as usize) != 0 {
        static N: AtomicUsize = AtomicUsize::new(0);
        if N.fetch_add(1, Ordering::Relaxed) < 8 {
            record("d3d12_xs", "placed_offset", &format!("DiscardResource on buffer {res:p}: no GPU work, as on console (xgs12 0x38020 only discards resource kinds 3 and 4, the colour and depth targets; any other resource returns at 0x3822b)"));
        }
        return;
    }
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void, *const c_void) = unsafe { core::mem::transmute(prev(CL_DISCARD)) };
    unsafe { f(this, res, region) }
}

fn p_is_set(vt: &[AtomicUsize], slot: usize, f: usize) -> bool {
    let p = vt[slot].load(Ordering::Acquire);
    p == 0 || p == f
}

pub fn install_cmdlist(vt: &[AtomicUsize]) -> usize {
    let hooks: [(usize, usize); 9] = [
        (CL_COPY_BUFFER, copy_buffer as usize),
        (CL_COPY_TEXTURE, copy_texture as usize),
        (CL_COPY_RESOURCE, copy_resource as usize),
        (CL_RESOLVE_QUERY, resolve_query as usize),
        (CL_PREDICATION, predication as usize),
        (CL_DISCARD, discard as usize),
        (CL_INDIRECT, indirect as usize),
        (CL_COPY_RESOURCE_X, copy_resource_x as usize),
        (CL_COPY_TEXTURE_X, copy_texture_x as usize),
    ];
    let mut n = 0;
    for (slot, f) in hooks {
        if slot >= vt.len() || p_is_set(vt, slot, f) {
            continue;
        }
        PREV_CL[slot].store(vt[slot].load(Ordering::Acquire), Ordering::Release);
        vt[slot].store(f, Ordering::Release);
        n += 1;
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    const IID_DEVICE: Guid = Guid(0x189819f1, 0x1db6, 0x4b57, [0xbe, 0x54, 0x18, 0x21, 0x33, 0x9b, 0x85, 0xf7]);

    const IID_ALLOCATOR: Guid = Guid(0x6102dee4, 0xaf59, 0x4b09, [0xb9, 0x99, 0xb4, 0x4d, 0x73, 0xf0, 0x9b, 0x24]);
    const IID_GFX_LIST: Guid = Guid(0x5b160d0f, 0xac1b, 0x4185, [0x8b, 0xa8, 0xb3, 0xae, 0x42, 0xa5, 0xa4, 0x55]);
    const IID_FENCE: Guid = Guid(0x0a753dcf, 0xc4d8, 0x4b91, [0xad, 0xf6, 0xbe, 0x5a, 0x60, 0xd9, 0x5a, 0x76]);
    const IID_QUEUE: Guid = Guid(0x0ec870a6, 0x5d7e, 0x4c22, [0x8c, 0xfc, 0x5b, 0xaa, 0xe0, 0x76, 0x16, 0xed]);

    unsafe fn vt(o: *mut c_void) -> *const usize {
        unsafe { *(o as *const *const usize) }
    }

    macro_rules! vc {
        ($obj:expr, $slot:expr, fn($($at:ty),*) -> $rt:ty $(, $arg:expr)*) => {{
            let f: unsafe extern "system" fn(*mut c_void $(, $at)*) -> $rt = core::mem::transmute(*vt($obj).add($slot));
            f($obj $(, $arg)*)
        }};
    }

    fn buf_desc(width: u64) -> [u8; 0x38] {
        let mut d = [0u8; 0x38];
        d[0..4].copy_from_slice(&1u32.to_le_bytes());
        d[0x10..0x18].copy_from_slice(&width.to_le_bytes());
        d[0x18..0x1c].copy_from_slice(&1u32.to_le_bytes());
        d[0x1c..0x1e].copy_from_slice(&1u16.to_le_bytes());
        d[0x1e..0x20].copy_from_slice(&1u16.to_le_bytes());
        d[0x24..0x28].copy_from_slice(&1u32.to_le_bytes());
        d[0x2c..0x30].copy_from_slice(&1u32.to_le_bytes());
        d
    }

    #[test]
    fn plan_widens_only_unaligned_buffers() {
        let d = buf_desc(0x1000);
        let (base, o, w) = plan(0x7100_0001_2000, &d).unwrap();
        assert_eq!((base, o), (0x7100_0001_0000, 0x2000));
        assert_eq!(u64::from_le_bytes(w[0x10..0x18].try_into().unwrap()), 0x3000);
        assert!(plan(0x7100_0001_0000, &d).is_none());
        let mut t = d;
        t[0..4].copy_from_slice(&3u32.to_le_bytes());
        assert!(plan(0x7100_0001_2000, &t).is_none());
        assert_eq!(resource_copy_span(Some(Off { bytes: 0x2000, width: 0x1000, heap: 0, heap_off: 0, real: 0, flags: 0 }), None, None, Some(0x4000)), Some((0x2000, 0, 0x1000)));
        assert_eq!(resource_copy_span(None, Some(1), None, Some(1)), None);
        let mut v = [0u8; 0x28];
        v[0x1c..0x20].copy_from_slice(&12u32.to_le_bytes());
        assert!(!rebase_view(&mut v, crate::console_slots_dev2::SRV_BUFFER, 0x2000));
        assert_eq!(u64::from_le_bytes(v[0x10..0x18].try_into().unwrap()), 0x2aa);
    }

    #[test]
    fn sub_page_buffers_share_the_page_with_their_own_gpu_va_offset() {
        unsafe {
            let p = crate::host_proc(b"D3D12CreateDevice\0");
            assert!(p != 0);
            let mk: extern "system" fn(*mut c_void, u32, *const Guid, *mut *mut c_void) -> i32 = core::mem::transmute(p);
            let mut dev: *mut c_void = core::ptr::null_mut();
            assert!(mk(core::ptr::null_mut(), 0xc000, &IID_DEVICE, &mut dev) >= 0);
            let base = 0x7300_0000_0000u64;
            let da = buf_desc(0x2000);
            let db = buf_desc(0x1000);
            let mut a: *mut c_void = core::ptr::null_mut();
            let mut b: *mut c_void = core::ptr::null_mut();
            let mut c: *mut c_void = core::ptr::null_mut();
            let ma = crate::alias::create_placed_x(dev, base, &da, 0x2000, 0, core::ptr::null(), &mut a).expect("aligned placement");
            assert!(plan(base, &da).is_none());
            let mb = create(dev, base + 0x2000, &db, 0x1000, 0, core::ptr::null(), &mut b).expect("sub-page placement");
            let mc = create(dev, base + 0x3000, &db, 0x1000, 0, core::ptr::null(), &mut c).expect("second sub-page placement");
            assert_eq!((ma.kind, ma.heap, ma.offset), (mb.kind, mb.heap, mb.offset));
            assert_eq!((ma.heap, ma.offset), (mc.heap, mc.offset));
            assert_eq!(of(b as usize).map(|o| (o.bytes, o.width, o.heap, o.heap_off)), Some((0x2000, 0x1000, mb.heap, mb.offset)));
            let mut gd = [0u8; 0x38];
            vc!(b, 10, fn(*mut u8) -> *mut u8, gd.as_mut_ptr());
            assert_eq!(u64::from_le_bytes(gd[0x10..0x18].try_into().unwrap()), 0x1000, "GetDesc reports the title's width, not the widened one");
            vc!(a, 10, fn(*mut u8) -> *mut u8, gd.as_mut_ptr());
            assert_eq!(u64::from_le_bytes(gd[0x10..0x18].try_into().unwrap()), 0x2000);
            crate::gpu_va::note_resource(dev, a, &da, 0, Some(base));
            crate::gpu_va::note_resource(dev, b, &db, 0, Some(base + 0x2000));
            crate::gpu_va::note_resource(dev, c, &db, 0, Some(base + 0x3000));
            let (ga, gb, gc) = (vc!(a, 11, fn() -> u64), vc!(b, 11, fn() -> u64), vc!(c, 11, fn() -> u64));
            assert_eq!((gb, gc), (ga + 0x2000, ga + 0x3000));
            let (e, off) = crate::gpu_va::lookup(base + 0x2004).unwrap();
            assert_eq!((e.res, off), (b as usize, 0x2004));
            let (e, off) = crate::gpu_va::lookup(gc + 8).unwrap();
            assert_eq!((e.res, off), (c as usize, 0x3008));
            let (e, off) = crate::gpu_va::lookup(base + 0x1000).unwrap();
            assert_eq!((e.res, off), (a as usize, 0x1000));
            let mut page: *mut c_void = core::ptr::null_mut();
            let whole = buf_desc(0x1_0000);
            crate::alias::create_placed_x(dev, base, &whole, 0x1_0000, 0, core::ptr::null(), &mut page).expect("whole-page view");
            let qd = [0u32; 4];
            let (mut q, mut al, mut l, mut fe): (*mut c_void, *mut c_void, *mut c_void, *mut c_void) = (core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut());
            assert!(vc!(dev, 8, fn(*const u32, *const Guid, *mut *mut c_void) -> i32, qd.as_ptr(), &IID_QUEUE, &mut q) >= 0);
            assert!(vc!(dev, 9, fn(u32, *const Guid, *mut *mut c_void) -> i32, 0, &IID_ALLOCATOR, &mut al) >= 0);
            assert!(vc!(dev, 12, fn(u32, u32, *mut c_void, *mut c_void, *const Guid, *mut *mut c_void) -> i32, 0, 0, al, core::ptr::null_mut(), &IID_GFX_LIST, &mut l) >= 0);
            assert!(vc!(dev, 36, fn(u64, u32, *const Guid, *mut *mut c_void) -> i32, 0, 0, &IID_FENCE, &mut fe) >= 0);
            vc!(l, 9, fn() -> i32);
            let committed = |ty: u32, size: u64, state: u32| {
                let heap: [u32; 5] = [ty, 0, 0, 1, 1];
                let d = buf_desc(size);
                let mut r: *mut c_void = core::ptr::null_mut();
                assert!(vc!(dev, 27, fn(*const u32, u32, *const u8, u32, *const c_void, *const Guid, *mut *mut c_void) -> i32, heap.as_ptr(), 0, d.as_ptr(), state, core::ptr::null(), &IID_RESOURCE, &mut r) >= 0);
                r
            };
            let up = committed(2, 0x1_0000, 0xac3);
            let rb = committed(3, 0x1_0000, 0x400);
            let mut u: *mut u8 = core::ptr::null_mut();
            assert!(vc!(up, 8, fn(u32, *const c_void, *mut *mut u8) -> i32, 0, core::ptr::null(), &mut u) >= 0);
            core::ptr::write_bytes(u, 0xaa, 0x1_0000);
            core::ptr::write_bytes(u.add(0x8000), 0x5c, 0x1000);
            let mut value = 0u64;
            let mut run = |rec: &dyn Fn(*mut c_void)| {
                vc!(al, 8, fn() -> i32);
                vc!(l, 10, fn(*mut c_void, *mut c_void) -> i32, al, core::ptr::null_mut());
                rec(l);
                assert!(vc!(l, 9, fn() -> i32) >= 0);
                let ls = [l];
                vc!(q, 10, fn(u32, *const *mut c_void) -> (), 1, ls.as_ptr());
                value += 1;
                vc!(q, 14, fn(*mut c_void, u64) -> i32, fe, value);
                let mut n = 0;
                while vc!(fe, 8, fn() -> u64) < value {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                    n += 1;
                    assert!(n < 5000);
                }
            };
            let cl = [const { AtomicUsize::new(0) }; 64];
            let real_l = vt(l);
            for (i, s) in cl.iter().enumerate() {
                if i < 28 {
                    s.store(*real_l.add(i), Ordering::Relaxed);
                }
            }
            assert_eq!(install_cmdlist(&cl), 3);
            let copy: unsafe extern "system" fn(*mut c_void, *mut c_void, u64, *mut c_void, u64, u64) = core::mem::transmute(cl[CL_COPY_BUFFER].load(Ordering::Relaxed));
            let copy_res: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) = core::mem::transmute(cl[CL_COPY_RESOURCE].load(Ordering::Relaxed));
            run(&|l| { copy(l, page, 0, up, 0, 0x1_0000); });
            run(&|l| { copy(l, b, 0, up, 0x8000, 0x1000); });
            run(&|l| { copy(l, rb, 0, page, 0, 0x1_0000); });
            let mut m: *mut u8 = core::ptr::null_mut();
            assert!(vc!(rb, 8, fn(u32, *const c_void, *mut *mut u8) -> i32, 0, core::ptr::null(), &mut m) >= 0);
            assert!((0..0x2000).all(|i| *m.add(i) == 0xaa), "the neighbour below the sub-page buffer is untouched");
            assert!((0x2000..0x3000).all(|i| *m.add(i) == 0x5c), "the sub-page buffer's bytes land at its own place in the page");
            assert!((0x3000..0x1_0000).all(|i| *m.add(i) == if (0x8000..0x9000).contains(&i) { 0x5c } else { 0xaa }), "the neighbour above is untouched");
            vc!(rb, 9, fn(u32, *const c_void) -> (), 0, core::ptr::null());
            let small = committed(3, 0x1000, 0x400);
            run(&|l| { copy_res(l, small, b); });
            let mut s: *mut u8 = core::ptr::null_mut();
            assert!(vc!(small, 8, fn(u32, *const c_void, *mut *mut u8) -> i32, 0, core::ptr::null(), &mut s) >= 0);
            assert!((0..0x1000).all(|i| *s.add(i) == 0x5c), "CopyResource of a sub-page buffer copies only its own bytes");
            let pat: [u8; 16] = core::array::from_fn(|j| 0xc3 + j as u8);
            run(&|l| { crate::fill::fill_pattern(l, false, base + 0x2100, 0x100, &pat, 0); });
            run(&|l| { copy(l, rb, 0, page, 0, 0x1_0000); });
            assert!(vc!(rb, 8, fn(u32, *const c_void, *mut *mut u8) -> i32, 0, core::ptr::null(), &mut m) >= 0);
            assert!((0x2000..0x2100).all(|i| *m.add(i) == 0x5c));
            assert!((0x2100..0x2200).all(|i| *m.add(i) == pat[(i - 0x2100) % 16]), "a title VA inside the sub-page buffer reaches the right page byte through gpu_va::lookup");
            assert!((0x2200..0x3000).all(|i| *m.add(i) == 0x5c));
            assert!((0..0x2000).all(|i| *m.add(i) == 0xaa));
            vc!(rb, 9, fn(u32, *const c_void) -> (), 0, core::ptr::null());
            for r in [small, rb, up, l, al, fe, q] {
                vc!(r, 2, fn() -> u32);
            }
            vc!(b, 2, fn() -> u32);
            assert_eq!(of(b as usize), None, "the offset is dropped when the resource is released");
            for r in [a, c, page] {
                vc!(r, 2, fn() -> u32);
            }
        }
    }

    static SEEN_TEX: Mutex<Vec<(u64, u32, u32)>> = Mutex::new(Vec::new());
    static DISCARDS: AtomicUsize = AtomicUsize::new(0);

    unsafe extern "system" fn fake_copy_tex_x(_this: *mut c_void, dst: *const u8, _x: u32, _y: u32, _z: u32, src: *const u8, _b: *const c_void, flags: u32) {
        for p in [dst, src] {
            let off = unsafe { core::ptr::read_unaligned(p.add(0x10) as *const u64) };
            let pitch = unsafe { core::ptr::read_unaligned(p.add(0x28) as *const u32) };
            SEEN_TEX.lock().unwrap().push((off, pitch, flags));
        }
    }

    unsafe extern "system" fn fake_discard(_this: *mut c_void, _res: *mut c_void, _region: *const c_void) {
        DISCARDS.fetch_add(1, Ordering::SeqCst);
    }

    #[test]
    fn residual_hooks_texture_copy_discard_views_and_abandon() {
        let fake_res = 0x5eed_0000usize;
        set(fake_res, Off { bytes: 0x2000, width: 0x100, heap: 0, heap_off: 0, real: 0, flags: 0 });
        let vt_tab = [const { AtomicUsize::new(0) }; 96];
        vt_tab[CL_DISCARD].store(fake_discard as usize, Ordering::SeqCst);
        vt_tab[CL_COPY_TEXTURE_X].store(fake_copy_tex_x as usize, Ordering::SeqCst);
        assert_eq!(install_cmdlist(&vt_tab), 2);
        let mut dst = [0u8; LOCATION];
        dst[0..8].copy_from_slice(&(fake_res as u64).to_le_bytes());
        dst[8..12].copy_from_slice(&1u32.to_le_bytes());
        dst[0x10..0x18].copy_from_slice(&0x40u64.to_le_bytes());
        dst[0x28..0x2c].copy_from_slice(&0x100u32.to_le_bytes());
        let mut src = [0u8; LOCATION];
        src[0..8].copy_from_slice(&0x1234u64.to_le_bytes());
        src[0x10..0x18].copy_from_slice(&3u64.to_le_bytes());
        src[0x28..0x2c].copy_from_slice(&0x200u32.to_le_bytes());
        unsafe {
            let f: unsafe extern "system" fn(*mut c_void, *const u8, u32, u32, u32, *const u8, *const c_void, u32) = core::mem::transmute(vt_tab[CL_COPY_TEXTURE_X].load(Ordering::SeqCst));
            f(core::ptr::null_mut(), dst.as_ptr(), 0, 0, 0, src.as_ptr(), core::ptr::null(), 7);
            let d: unsafe extern "system" fn(*mut c_void, *mut c_void, *const c_void) = core::mem::transmute(vt_tab[CL_DISCARD].load(Ordering::SeqCst));
            d(core::ptr::null_mut(), fake_res as *mut c_void, core::ptr::null());
            assert_eq!(DISCARDS.load(Ordering::SeqCst), 0, "a buffer discard does no GPU work, as on console");
            d(core::ptr::null_mut(), 0x9999 as *mut c_void, core::ptr::null());
            assert_eq!(DISCARDS.load(Ordering::SeqCst), 1);
        }
        assert_eq!(*SEEN_TEX.lock().unwrap(), vec![(0x2040, 0x100, 7), (3, 0x200, 7)], "the footprint offset moves by the page offset and the row pitch survives");
        assert_eq!(helper_plan(0x2000, 12, 0x2_0000), Some((1, 0x1_2000 / 12)));
        assert_eq!(helper_plan(0x2000, 12, 0), None);
        assert_eq!(helper_plan(0x2004, 4, 0), Some((0, 0x801)));
        assert_eq!(helper_plan(0x2001, 12, 0x10_0000), None);
        let mut dropped = false;
        assert!(keep_or_abandon(fake_res, true, || dropped = true));
        assert!(!dropped && of(fake_res).is_some());
        assert!(!keep_or_abandon(fake_res, false, || dropped = true));
        assert!(dropped && of(fake_res).is_none(), "a failed watch attach drops the offset and the placement");
    }

    fn tex_desc(samples: u32) -> [u8; 0x38] {
        let mut d = [0u8; 0x38];
        d[0..4].copy_from_slice(&3u32.to_le_bytes());
        d[0x10..0x18].copy_from_slice(&256u64.to_le_bytes());
        d[0x18..0x1c].copy_from_slice(&256u32.to_le_bytes());
        d[0x1c..0x1e].copy_from_slice(&1u16.to_le_bytes());
        d[0x1e..0x20].copy_from_slice(&1u16.to_le_bytes());
        d[0x20..0x24].copy_from_slice(&28u32.to_le_bytes());
        d[0x24..0x28].copy_from_slice(&samples.to_le_bytes());
        d[0x30..0x34].copy_from_slice(&1u32.to_le_bytes());
        d
    }

    extern "system" {
        fn VirtualAlloc(addr: *mut c_void, size: usize, ty: u32, protect: u32) -> *mut c_void;
        fn VirtualFree(p: *mut c_void, n: usize, ty: u32) -> i32;
    }

    #[test]
    fn uneven_views_get_exact_helpers_and_msaa_shares_the_alias_heap() {
        unsafe {
            let p = crate::host_proc(b"D3D12CreateDevice\0");
            let mk: extern "system" fn(*mut c_void, u32, *const Guid, *mut *mut c_void) -> i32 = core::mem::transmute(p);
            let mut dev: *mut c_void = core::ptr::null_mut();
            assert!(mk(core::ptr::null_mut(), 0xc000, &IID_DEVICE, &mut dev) >= 0);
            let r = VirtualAlloc(core::ptr::null_mut(), 0x80_0000, 0x2000, 0x01) as u64;
            assert!(r != 0);
            let db = buf_desc(0x1000);
            let mut b: *mut c_void = core::ptr::null_mut();
            let mb = create(dev, r + 0x2_2000, &db, 0x1000, 0, core::ptr::null(), &mut b).expect("sub-page placement in a reservation");
            assert_eq!(mb.offset, 0x2_0000);
            crate::gpu_va::note_resource(dev, b, &db, 0, Some(r + 0x2_2000));
            let mut v = [0u8; 0x28];
            v[4..8].copy_from_slice(&1u32.to_le_bytes());
            v[0x18..0x1c].copy_from_slice(&10u32.to_le_bytes());
            v[0x1c..0x20].copy_from_slice(&12u32.to_le_bytes());
            let h = exact_view(b as usize, &mut v, crate::console_slots_dev2::SRV_BUFFER).expect("stride 12 at +0x2000 lines up one 64 KB step down");
            assert!(h != b);
            let first = u64::from_le_bytes(v[0x10..0x18].try_into().unwrap());
            assert_eq!(first, 0x1_2000 / 12);
            match crate::title_shadow::backing(b as usize) {
                Some((heap, at)) => assert_eq!(crate::title_shadow::backing(h as usize), Some((heap, at - 0x1_0000)), "the shadowed helper maps the heap tile one 64 KB step below the buffer, so its element {first} is the buffer's first byte"),
                None => assert_eq!(vc!(h, 11, fn() -> u64) + first * 12, vc!(b, 11, fn() -> u64), "the view's first element is the buffer's first byte"),
            }
            let mut v2 = v;
            v2[0x10..0x18].copy_from_slice(&0u64.to_le_bytes());
            assert_eq!(exact_view(b as usize, &mut v2, crate::console_slots_dev2::SRV_BUFFER), Some(h), "the helper is shared by later views");
            vc!(b, 2, fn() -> u32);
            assert!(of(b as usize).is_none());
            assert!(HELPERS.lock().unwrap().as_ref().map_or(true, |m| !m.keys().any(|k| k.0 == b as usize)), "helpers go with their buffer");

            let mut t1: *mut c_void = core::ptr::null_mut();
            let mut t2: *mut c_void = core::ptr::null_mut();
            let mut t3: *mut c_void = core::ptr::null_mut();
            let at = r + 0x40_0000;
            let m1 = crate::alias::create_placed_x(dev, at, &tex_desc(4), 0x40_0000, 4, core::ptr::null(), &mut t1).expect("4x MSAA target placed 4 MB into the reservation");
            let m2 = crate::alias::create_placed_x(dev, at, &tex_desc(1), 0x4_0000, 4, core::ptr::null(), &mut t2).expect("single-sample texture at the same VA");
            assert_eq!(m1.kind, crate::alias::Kind::Local);
            assert_eq!((m1.heap, m1.offset), (m2.heap, m2.offset));
            assert_eq!(m1.offset, 0x40_0000);
            assert!(crate::msaa_place::capable(m1.heap));
            let mut info = [0u64; 2];
            let md = tex_desc(4);
            vc!(dev, 25, fn(*mut u64, u32, u32, *const u8) -> *mut u64, info.as_mut_ptr(), 0, 1, md.as_ptr());
            let off_step = crate::alias::create_placed_x(dev, r + 0x31_0000, &md, 0x1_0000, 4, core::ptr::null(), &mut t3);
            if info[1] >= crate::msaa_place::MSAA_ALIGN {
                assert!(off_step.is_none(), "MSAA off a 4 MB step of its heap falls back to a committed resource");
            } else {
                assert!(off_step.is_some(), "this adapter places MSAA at {:#x}, so any 64 KB step works", info[1]);
                vc!(t3, 2, fn() -> u32);
            }
            for t in [t1, t2] {
                vc!(t, 2, fn() -> u32);
            }
            VirtualFree(r as *mut c_void, 0, 0x8000);
            vc!(dev, 2, fn() -> u32);
        }
    }
}

#[cfg(test)]
pub unsafe fn unpatched_width(res: *mut c_void) -> Option<u64> {
    let vtv = unsafe { *(res as *const usize) };
    let orig = DESC_VT.iter().position(|p| p.load(Ordering::Acquire) == vtv).map(|i| DESC_ORIG[i].load(Ordering::Acquire))?;
    let f: unsafe extern "system" fn(*mut c_void, *mut u8) -> *mut u8 = unsafe { core::mem::transmute(orig) };
    let mut d = [0u8; 0x38];
    let r = unsafe { f(res, d.as_mut_ptr()) };
    Some(u64::from_le_bytes(unsafe { core::slice::from_raw_parts(r.add(0x10), 8) }.try_into().unwrap()))
}
