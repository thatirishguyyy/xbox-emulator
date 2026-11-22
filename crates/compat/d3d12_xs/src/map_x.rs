use compat_common::record;
use core::ffi::c_void;
use core::sync::atomic::{AtomicUsize, Ordering};
use std::collections::HashMap;
use std::sync::Mutex;

extern "system" {
    fn VirtualProtect(addr: *mut c_void, size: usize, new_protect: u32, old_protect: *mut u32) -> i32;
    fn VirtualQuery(addr: *const c_void, info: *mut [u64; 6], len: usize) -> usize;
    fn Sleep(ms: u32);
}

#[repr(C)]
struct Guid(u32, u16, u16, [u8; 8]);
const IID_DEVICE: Guid = Guid(0x189819f1, 0x1db6, 0x4b57, [0xbe, 0x54, 0x18, 0x21, 0x33, 0x9b, 0x85, 0xf7]);
const IID_ALLOCATOR: Guid = Guid(0x6102dee4, 0xaf59, 0x4b09, [0xb9, 0x99, 0xb4, 0x4d, 0x73, 0xf0, 0x9b, 0x24]);
const IID_GFX_LIST: Guid = Guid(0x5b160d0f, 0xac1b, 0x4185, [0x8b, 0xa8, 0xb3, 0xae, 0x42, 0xa5, 0xa4, 0x55]);
const IID_FENCE: Guid = Guid(0x0a753dcf, 0xc4d8, 0x4b91, [0xad, 0xf6, 0xbe, 0x5a, 0x60, 0xd9, 0x5a, 0x76]);
const IID_RESOURCE: Guid = Guid(0x696442be, 0xa72e, 0x4059, [0xbc, 0x79, 0x5b, 0x5c, 0x98, 0x04, 0x0f, 0xad]);
const IID_QUEUE: Guid = Guid(0x0ec870a6, 0x5d7e, 0x4c22, [0x8c, 0xfc, 0x5b, 0xaa, 0xe0, 0x76, 0x16, 0xed]);
const COPY_DEST: u32 = 0x400;
const COPY_SOURCE: u32 = 0x800;
const GENERIC_READ: u32 = 0xac3;
const E_INVALIDARG: i32 = 0x8007_0057u32 as i32;
const SLOTS: usize = 16;

static VT: [AtomicUsize; SLOTS] = [const { AtomicUsize::new(0) }; SLOTS];
static ORIG_MAP: [AtomicUsize; SLOTS] = [const { AtomicUsize::new(0) }; SLOTS];
static ORIG_UNMAP: [AtomicUsize; SLOTS] = [const { AtomicUsize::new(0) }; SLOTS];
static PATCH_LOCK: Mutex<()> = Mutex::new(());
static QUEUE: AtomicUsize = AtomicUsize::new(0);
static SYNC: Mutex<Option<Sync>> = Mutex::new(None);
static MAPPED: Mutex<Option<HashMap<usize, Mapping>>> = Mutex::new(None);

#[derive(Clone, Copy)]
struct Mapping {
    cpu: usize,
    size: u64,
    count: u32,
    owned: bool,
}

unsafe fn vt(obj: *mut c_void) -> *const usize {
    unsafe { *(obj as *const *const usize) }
}

macro_rules! vcall {
    ($obj:expr, $slot:expr, fn($($at:ty),*) -> $rt:ty $(, $arg:expr)*) => {{
        let f: unsafe extern "system" fn(*mut c_void $(, $at)*) -> $rt = core::mem::transmute(*vt($obj).add($slot));
        f($obj $(, $arg)*)
    }};
}

pub fn slot_of(table: &[usize], vtable: usize) -> Option<usize> {
    table.iter().position(|&v| v == vtable && v != 0)
}

pub fn read_span(range: Option<(u64, u64)>, size: u64) -> Option<(u64, u64)> {
    let (b, e) = range.unwrap_or((0, size));
    let e = e.min(size);
    if b >= e { None } else { Some((b, e)) }
}

pub fn clamp_read_range(r: (u64, u64), width: u64) -> Option<(u64, u64)> {
    if r.0 <= width && r.1 <= width {
        return None;
    }
    Some((r.0.min(width), r.1.min(width)))
}

fn table(t: &[AtomicUsize; SLOTS]) -> [usize; SLOTS] {
    core::array::from_fn(|i| t[i].load(Ordering::Acquire))
}

pub fn set_queue(queue: *mut c_void) {
    if !queue.is_null() {
        let _ = QUEUE.compare_exchange(0, queue as usize, Ordering::AcqRel, Ordering::Relaxed);
    }
}

pub fn ensure_patched(res: *mut c_void) {
    if res.is_null() {
        return;
    }
    let vtbl = unsafe { vt(res) } as usize;
    if slot_of(&table(&VT), vtbl).is_some() {
        return;
    }
    let _g = PATCH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if slot_of(&table(&VT), vtbl).is_some() {
        return;
    }
    let Some(i) = (0..SLOTS).find(|&i| VT[i].load(Ordering::Acquire) == 0) else {
        record("d3d12_xs", "map_x", &format!("resource vtable {vtbl:#x} not hooked: table full"));
        return;
    };
    unsafe {
        let p = vtbl as *mut usize;
        let (m, u) = (*p.add(8), *p.add(9));
        if m == map_hook as usize {
            return;
        }
        ORIG_MAP[i].store(m, Ordering::Release);
        ORIG_UNMAP[i].store(u, Ordering::Release);
        VT[i].store(vtbl, Ordering::Release);
        let mut old = 0u32;
        if VirtualProtect(p.add(8) as *mut c_void, 16, 0x04, &mut old) != 0 {
            *p.add(8) = map_hook as usize;
            *p.add(9) = unmap_hook as usize;
            let mut tmp = 0u32;
            VirtualProtect(p.add(8) as *mut c_void, 16, old, &mut tmp);
            record("d3d12_xs", "map_x", &format!("resource vtable {vtbl:#x} Map/Unmap hooked (#{i})"));
        } else {
            VT[i].store(0, Ordering::Release);
        }
    }
}

fn orig(res: *mut c_void, unmap: bool) -> usize {
    let vtbl = unsafe { vt(res) } as usize;
    match slot_of(&table(&VT), vtbl) {
        Some(i) => if unmap { ORIG_UNMAP[i].load(Ordering::Acquire) } else { ORIG_MAP[i].load(Ordering::Acquire) },
        None => 0,
    }
}

unsafe fn describe(res: *mut c_void) -> String {
    let mut d = [0u8; 0x38];
    unsafe { vcall!(res, 10, fn(*mut u8) -> *mut u8, d.as_mut_ptr()) };
    let mut hp = [0u32; 5];
    let mut hf = 0u32;
    let hr = unsafe { vcall!(res, 14, fn(*mut u32, *mut u32) -> i32, hp.as_mut_ptr(), &mut hf) };
    let u32at = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
    let u64at = |o: usize| u64::from_le_bytes(d[o..o + 8].try_into().unwrap());
    let placed = crate::placed::lookup(res).map(|p| format!(" placedX va={:#x}", p.va)).unwrap_or_default();
    format!("dim={} width={} height={} depth={} mips={} fmt={} layout={} flags={:#x} | heap hr={hr:#010x} type={} cpuPage={} pool={} heapFlags={hf:#x}{placed} trackedState={:#x}",
        u32at(0), u64at(0x10), u32at(0x18), u16::from_le_bytes([d[0x1c], d[0x1d]]), u16::from_le_bytes([d[0x1e], d[0x1f]]), u32at(0x20), u32at(0x2c), u32at(0x30),
        hp[0], hp[1], hp[2], crate::gpu_va::state(res as usize))
}

struct Sync {
    queue: *mut c_void,
    device: *mut c_void,
    alloc: *mut c_void,
    list: *mut c_void,
    fence: *mut c_void,
    value: u64,
}
unsafe impl Send for Sync {}

impl Sync {
    unsafe fn new(queue: *mut c_void) -> Option<Sync> {
        unsafe {
            let mut device: *mut c_void = core::ptr::null_mut();
            if vcall!(queue, 7, fn(*const Guid, *mut *mut c_void) -> i32, &IID_DEVICE, &mut device) < 0 || device.is_null() {
                return None;
            }
            let mut alloc: *mut c_void = core::ptr::null_mut();
            let mut list: *mut c_void = core::ptr::null_mut();
            let mut fence: *mut c_void = core::ptr::null_mut();
            if vcall!(device, 9, fn(u32, *const Guid, *mut *mut c_void) -> i32, 0, &IID_ALLOCATOR, &mut alloc) < 0
                || vcall!(device, 12, fn(u32, u32, *mut c_void, *mut c_void, *const Guid, *mut *mut c_void) -> i32, 0, 0, alloc, core::ptr::null_mut(), &IID_GFX_LIST, &mut list) < 0
                || vcall!(device, 36, fn(u64, u32, *const Guid, *mut *mut c_void) -> i32, 0, 0, &IID_FENCE, &mut fence) < 0
            {
                return None;
            }
            vcall!(list, 9, fn() -> i32);
            let qdesc = [0u32; 4];
            let mut own: *mut c_void = core::ptr::null_mut();
            if vcall!(device, 8, fn(*const u32, *const Guid, *mut *mut c_void) -> i32, qdesc.as_ptr(), &IID_QUEUE, &mut own) < 0 || own.is_null() {
                own = queue;
            }
            Some(Sync { queue: own, device, alloc, list, fence, value: 0 })
        }
    }

    unsafe fn buffer(&self, heap_type: u32, size: u64, state: u32) -> *mut c_void {
        let heap: [u32; 5] = [heap_type, 0, 0, 1, 1];
        let mut bd = [0u8; 0x38];
        bd[0..4].copy_from_slice(&1u32.to_le_bytes());
        bd[0x10..0x18].copy_from_slice(&size.to_le_bytes());
        bd[0x18..0x1c].copy_from_slice(&1u32.to_le_bytes());
        bd[0x1c..0x1e].copy_from_slice(&1u16.to_le_bytes());
        bd[0x1e..0x20].copy_from_slice(&1u16.to_le_bytes());
        bd[0x24..0x28].copy_from_slice(&1u32.to_le_bytes());
        bd[0x2c..0x30].copy_from_slice(&1u32.to_le_bytes());
        let mut buf: *mut c_void = core::ptr::null_mut();
        let hr = unsafe {
            vcall!(self.device, 27, fn(*const u32, u32, *const u8, u32, *const c_void, *const Guid, *mut *mut c_void) -> i32,
                heap.as_ptr(), 0, bd.as_ptr(), state, core::ptr::null(), &IID_RESOURCE, &mut buf)
        };
        if hr < 0 { core::ptr::null_mut() } else { buf }
    }

    unsafe fn copy(&mut self, dst: *mut c_void, dst_off: u64, src: *mut c_void, src_off: u64, len: u64, res: *mut c_void, res_state: u32, need: u32) -> bool {
        unsafe {
            vcall!(self.alloc, 8, fn() -> i32);
            vcall!(self.list, 10, fn(*mut c_void, *mut c_void) -> i32, self.alloc, core::ptr::null_mut());
            let trans = res_state != need && res_state & need == 0;
            if trans {
                let b = [0u64, res as u64, 0xffff_ffffu64 | ((res_state as u64) << 32), need as u64];
                vcall!(self.list, 26, fn(u32, *const u64) -> (), 1, b.as_ptr());
            }
            vcall!(self.list, 15, fn(*mut c_void, u64, *mut c_void, u64, u64) -> (), dst, dst_off, src, src_off, len);
            if trans {
                let b = [0u64, res as u64, 0xffff_ffffu64 | ((need as u64) << 32), res_state as u64];
                vcall!(self.list, 26, fn(u32, *const u64) -> (), 1, b.as_ptr());
            }
            if vcall!(self.list, 9, fn() -> i32) < 0 {
                return false;
            }
            let lists = [self.list];
            vcall!(self.queue, 10, fn(u32, *const *mut c_void) -> (), 1, lists.as_ptr());
            self.value += 1;
            vcall!(self.queue, 14, fn(*mut c_void, u64) -> i32, self.fence, self.value);
            let mut spins = 0;
            while vcall!(self.fence, 8, fn() -> u64) < self.value && spins < 5_000 {
                Sleep(1);
                spins += 1;
            }
            spins < 5_000
        }
    }
}

fn writable(p: usize, len: usize) -> bool {
    let mut info = [0u64; 6];
    let mut a = p;
    while a < p + len {
        if unsafe { VirtualQuery(a as *const c_void, &mut info, 48) } == 0 {
            return false;
        }
        let base = info[0] as usize;
        let size = info[3] as usize;
        let state = (info[4] & 0xffff_ffff) as u32;
        let protect = (info[4] >> 32) as u32;
        if state != 0x1000 || protect & 0xcc == 0 || protect & 0x100 != 0 || size == 0 {
            return false;
        }
        a = base + size;
    }
    true
}

unsafe fn with_sync<R>(f: impl FnOnce(&mut Sync) -> R) -> Option<R> {
    let q = QUEUE.load(Ordering::Acquire) as *mut c_void;
    if q.is_null() {
        return None;
    }
    let mut g = SYNC.lock().unwrap_or_else(|e| e.into_inner());
    if g.is_none() {
        *g = unsafe { Sync::new(q) };
    }
    g.as_mut().map(f)
}

unsafe fn console_map(res: *mut c_void, range: Option<(u64, u64)>) -> Option<usize> {
    let p = crate::placed::lookup(res)?;
    if p.dim() != 1 {
        return None;
    }
    let size = p.width();
    let existing = MAPPED.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(&(res as usize)).copied());
    let (cpu, owned) = match existing {
        Some(m) => (m.cpu, m.owned),
        None if p.va != 0 && writable(p.va as usize, size as usize) => (p.va as usize, false),
        None => (Box::into_raw(vec![0u8; size as usize].into_boxed_slice()) as *mut u8 as usize, true),
    };
    if let Some((b, e)) = read_span(range, size) {
        let state = crate::gpu_va::state(res as usize);
        let ok = unsafe {
            with_sync(|s| {
                let rb = s.buffer(3, e - b, COPY_DEST);
                if rb.is_null() {
                    return false;
                }
                let ok = s.copy(rb, 0, res, b + crate::placed_offset::bytes(res as usize), e - b, res, state, COPY_SOURCE);
                let mut data: *mut u8 = core::ptr::null_mut();
                let r = (0u64, e - b);
                let mapped = ok && vcall!(rb, 8, fn(u32, *const (u64, u64), *mut *mut u8) -> i32, 0, &r, &mut data) >= 0 && !data.is_null();
                if mapped {
                    core::ptr::copy_nonoverlapping(data, (cpu + b as usize) as *mut u8, (e - b) as usize);
                    vcall!(rb, 9, fn(u32, *const c_void) -> (), 0, core::ptr::null());
                }
                vcall!(rb, 2, fn() -> u32);
                mapped
            })
        };
        if ok != Some(true) {
            if owned && existing.is_none() {
                drop(unsafe { Box::from_raw(core::ptr::slice_from_raw_parts_mut(cpu as *mut u8, size as usize)) });
            }
            return None;
        }
    }
    let mut g = MAPPED.lock().unwrap_or_else(|e| e.into_inner());
    let m = g.get_or_insert_with(HashMap::new).entry(res as usize).or_insert(Mapping { cpu, size, count: 0, owned });
    m.count += 1;
    Some(cpu)
}

unsafe fn console_unmap(res: *mut c_void, written: Option<(u64, u64)>) -> bool {
    let m = {
        let mut g = MAPPED.lock().unwrap_or_else(|e| e.into_inner());
        let Some(map) = g.as_mut() else { return false };
        let Some(m) = map.get_mut(&(res as usize)) else { return false };
        m.count = m.count.saturating_sub(1);
        let snapshot = *m;
        if m.count == 0 {
            map.remove(&(res as usize));
        }
        snapshot
    };
    if let Some((b, e)) = read_span(written, m.size) {
        let state = crate::gpu_va::state(res as usize);
        let _ = unsafe {
            with_sync(|s| {
                let ub = s.buffer(2, e - b, GENERIC_READ);
                if ub.is_null() {
                    return;
                }
                let mut data: *mut u8 = core::ptr::null_mut();
                if vcall!(ub, 8, fn(u32, *const c_void, *mut *mut u8) -> i32, 0, core::ptr::null(), &mut data) >= 0 && !data.is_null() {
                    core::ptr::copy_nonoverlapping((m.cpu + b as usize) as *const u8, data, (e - b) as usize);
                    vcall!(ub, 9, fn(u32, *const c_void) -> (), 0, core::ptr::null());
                    s.copy(res, b + crate::placed_offset::bytes(res as usize), ub, 0, e - b, res, state, COPY_DEST);
                }
                vcall!(ub, 2, fn() -> u32);
            })
        };
    }
    if m.count == 0 && m.owned {
        drop(unsafe { Box::from_raw(core::ptr::slice_from_raw_parts_mut(m.cpu as *mut u8, m.size as usize)) });
    }
    true
}

static TEX_MAPPED: Mutex<Option<HashMap<usize, u32>>> = Mutex::new(None);
static TEX_BACK_LOCK: Mutex<()> = Mutex::new(());

unsafe fn heap_type(res: *mut c_void) -> Option<u32> {
    let mut hp = [0u32; 5];
    let mut hf = 0u32;
    let hr = unsafe { vcall!(res, 14, fn(*mut u32, *mut u32) -> i32, hp.as_mut_ptr(), &mut hf) };
    if hr >= 0 { Some(hp[0]) } else { None }
}

unsafe fn texture_map(res: *mut c_void) -> Option<u64> {
    let mut d = [0u8; 0x38];
    unsafe { vcall!(res, 10, fn(*mut u8) -> *mut u8, d.as_mut_ptr()) };
    if !crate::gpu_va::is_texture_desc(&d) {
        return None;
    }
    let va = {
        let _g = TEX_BACK_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        match crate::placed::lookup(res) {
            Some(p) if p.dim() >= 2 && p.va != 0 && writable(p.va as usize, 1) => p.va,
            Some(_) => return None,
            None => {
                if unsafe { heap_type(res) } != Some(1) {
                    return None;
                }
                let x = crate::gpu_va::committed_texture_desc(res).filter(|x| crate::gpu_va::same_texture(x, &d))?;
                let va = unsafe { crate::gpu_va::alloc_texture_backing(&x) }?;
                unsafe { crate::gpu_va::adopt_backing(res, va, x, crate::gpu_va::state(res as usize)) };
                static N: AtomicUsize = AtomicUsize::new(0);
                if N.fetch_add(1, Ordering::Relaxed) < 16 {
                    record("d3d12_xs", "map_x", &format!("Map res={res:p}: committed default-heap texture given console CPU memory {va:#x}+{:#x} (GetResourceAllocationInfo size); title writes reach the GPU through the placed-texture uploader | {}", crate::gpu_va::console_texture_size(&x), unsafe { describe(res) }));
                }
                va
            }
        }
    };
    *TEX_MAPPED.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).entry(res as usize).or_insert(0) += 1;
    Some(va)
}

fn texture_unmap(res: *mut c_void) -> bool {
    let mut g = TEX_MAPPED.lock().unwrap_or_else(|e| e.into_inner());
    let Some(m) = g.as_mut() else { return false };
    let Some(c) = m.get_mut(&(res as usize)) else { return false };
    *c -= 1;
    if *c == 0 {
        m.remove(&(res as usize));
    }
    true
}

unsafe fn opt_range(r: *const (u64, u64)) -> Option<(u64, u64)> {
    if r.is_null() { None } else { Some(unsafe { *r }) }
}

unsafe extern "system" fn map_hook(res: *mut c_void, sub: u32, range: *const (u64, u64), out: *mut *mut c_void) -> i32 {
    if let Some(va) = crate::title_shadow::map_va(res) {
        if sub != 0 {
            return E_INVALIDARG;
        }
        if !out.is_null() {
            unsafe { *out = va as *mut c_void };
        }
        return 0;
    }
    let o = orig(res, false);
    if o == 0 {
        return E_INVALIDARG;
    }
    let f: unsafe extern "system" fn(*mut c_void, u32, *const (u64, u64), *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(o) };
    let rr = unsafe { opt_range(range) };
    let clamped = rr.and_then(|r| {
        let mut d = [0u8; 0x38];
        unsafe { vcall!(res, 10, fn(*mut u8) -> *mut u8, d.as_mut_ptr()) };
        let dim = u32::from_le_bytes(d[0..4].try_into().unwrap());
        let width = u64::from_le_bytes(d[0x10..0x18].try_into().unwrap());
        if sub == 0 && dim == 1 { clamp_read_range(r, width) } else { None }
    });
    let hr = unsafe { f(res, sub, clamped.as_ref().map_or(range, |c| c as *const (u64, u64)), out) };
    if let Some(c) = clamped {
        static C: AtomicUsize = AtomicUsize::new(0);
        if C.fetch_add(1, Ordering::Relaxed) < 8 {
            record("d3d12_xs", "map_x", &format!("Map res={res:p} ReadRange {rr:x?} runs past the buffer; clamped to {c:x?} (console treats ReadRange as a hint) -> hr={hr:#010x}"));
        }
    }
    if hr >= 0 {
        return hr;
    }
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    if rr.is_some() {
        let empty = (0u64, 0u64);
        let hr2 = unsafe { f(res, sub, &empty, out) };
        if hr2 >= 0 {
            if n < 8 {
                record("d3d12_xs", "map_x", &format!("Map res={res:p} sub={sub} range={rr:x?} hr={hr:#010x}; mapped with an empty ReadRange instead (it is only a hint) | {}", unsafe { describe(res) }));
            }
            return hr2;
        }
    }
    if sub == 0 {
        if let Some(cpu) = unsafe { console_map(res, rr) } {
            if !out.is_null() {
                unsafe { *out = cpu as *mut c_void };
            }
            if n < 8 {
                record("d3d12_xs", "map_x", &format!("Map res={res:p} sub={sub} range={rr:x?} host hr={hr:#010x}: console placement served from title memory {cpu:#x} after GPU readback | {}", unsafe { describe(res) }));
            }
            return 0;
        }
        if let Some(va) = unsafe { texture_map(res) } {
            if !out.is_null() {
                unsafe { *out = va as *mut c_void };
            }
            return 0;
        }
    }
    if n < 32 {
        record("d3d12_xs", "map_x", &format!("Map FAILED res={res:p} sub={sub} range={rr:x?} out={out:p} hr={hr:#010x} | {}", unsafe { describe(res) }));
    }
    hr
}

unsafe extern "system" fn unmap_hook(res: *mut c_void, sub: u32, written: *const (u64, u64)) {
    if crate::title_shadow::map_va(res).is_some() {
        return;
    }
    if sub == 0 && texture_unmap(res) {
        return;
    }
    if sub == 0 && unsafe { console_unmap(res, opt_range(written)) } {
        return;
    }
    let o = orig(res, true);
    if o == 0 {
        return;
    }
    let f: unsafe extern "system" fn(*mut c_void, u32, *const (u64, u64)) = unsafe { core::mem::transmute(o) };
    unsafe { f(res, sub, written) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_span_clamps_and_rejects_empty() {
        assert_eq!(read_span(None, 64), Some((0, 64)));
        assert_eq!(read_span(Some((8, 24)), 64), Some((8, 24)));
        assert_eq!(read_span(Some((8, 200)), 64), Some((8, 64)));
        assert_eq!(read_span(Some((0, 0)), 64), None);
        assert_eq!(read_span(Some((32, 16)), 64), None);
        assert_eq!(read_span(Some((64, 80)), 64), None);
    }

    #[test]
    fn read_range_past_end_is_clamped() {
        assert_eq!(clamp_read_range((0x7f0, 0x810), 0x800), Some((0x7f0, 0x800)));
        assert_eq!(clamp_read_range((0x900, 0x910), 0x800), Some((0x800, 0x800)));
        assert_eq!(clamp_read_range((0, 0x800), 0x800), None);
        assert_eq!(clamp_read_range((8, 0), 0x800), None);
    }

    #[test]
    fn map_with_range_past_end_succeeds() {
        unsafe {
            let p = crate::host_proc(b"D3D12CreateDevice\0");
            let create: extern "system" fn(*mut c_void, u32, *const Guid, *mut *mut c_void) -> i32 = core::mem::transmute(p);
            let mut dev: *mut c_void = core::ptr::null_mut();
            assert!(create(core::ptr::null_mut(), 0xc000, &IID_DEVICE, &mut dev) >= 0);
            let heap: [u32; 5] = [3, 0, 0, 1, 1];
            let mut bd = [0u8; 0x38];
            bd[0..4].copy_from_slice(&1u32.to_le_bytes());
            bd[0x10..0x18].copy_from_slice(&0x800u64.to_le_bytes());
            bd[0x18..0x1c].copy_from_slice(&1u32.to_le_bytes());
            bd[0x1c..0x1e].copy_from_slice(&1u16.to_le_bytes());
            bd[0x1e..0x20].copy_from_slice(&1u16.to_le_bytes());
            bd[0x24..0x28].copy_from_slice(&1u32.to_le_bytes());
            bd[0x2c..0x30].copy_from_slice(&1u32.to_le_bytes());
            let mut rb: *mut c_void = core::ptr::null_mut();
            assert!(vcall!(dev, 27, fn(*const u32, u32, *const u8, u32, *const c_void, *const Guid, *mut *mut c_void) -> i32,
                heap.as_ptr(), 0, bd.as_ptr(), COPY_DEST, core::ptr::null(), &IID_RESOURCE, &mut rb) >= 0);
            let past = (0x7f0u64, 0x810u64);
            let mut raw: *mut u8 = core::ptr::null_mut();
            let direct = vcall!(rb, 8, fn(u32, *const (u64, u64), *mut *mut u8) -> i32, 0, &past, &mut raw);
            ensure_patched(rb);
            let mut out: *mut u8 = core::ptr::null_mut();
            let hr = vcall!(rb, 8, fn(u32, *const (u64, u64), *mut *mut u8) -> i32, 0, &past, &mut out);
            assert!(hr >= 0, "hooked Map hr={hr:#010x} (unhooked hr={direct:#010x})");
            assert!(!out.is_null());
            let mut info = [0u64; 6];
            assert!(VirtualQuery(out.add(0x80f) as *const c_void, &mut info, 48) != 0);
            assert_eq!(info[4] & 0xffff_ffff, 0x1000);
            let _ = core::ptr::read_volatile(out.add(0x80f));
            vcall!(rb, 9, fn(u32, *const c_void) -> (), 0, core::ptr::null());
            vcall!(rb, 2, fn() -> u32);
        }
    }

    #[test]
    fn vtable_slots_match_exactly() {
        let t = [0x10usize, 0x20, 0, 0];
        assert_eq!(slot_of(&t, 0x20), Some(1));
        assert_eq!(slot_of(&t, 0x30), None);
        assert_eq!(slot_of(&t, 0), None);
    }
}
