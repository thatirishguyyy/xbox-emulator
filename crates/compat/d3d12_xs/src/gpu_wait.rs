use core::ffi::c_void;
use core::sync::atomic::{AtomicUsize, Ordering};
use std::collections::HashMap;
use std::sync::Mutex;
use compat_common::record;

use crate::device_wrap::Guid;

pub const FUNC_NEVER: u32 = 1;
pub const FUNC_LESS: u32 = 2;
pub const FUNC_EQUAL: u32 = 3;
pub const FUNC_LESS_EQUAL: u32 = 4;
pub const FUNC_GREATER: u32 = 5;
pub const FUNC_NOT_EQUAL: u32 = 6;
pub const FUNC_GREATER_EQUAL: u32 = 7;
pub const FUNC_ALWAYS: u32 = 8;
pub const WAIT_FLAG_PFP: u32 = 1;
pub const WAIT_FLAGS_RESERVED: u32 = 0xfffe;
pub const DEFAULT_POLL_INTERVAL: u32 = 8;

const IID_DEVICE: Guid =
    Guid { d1: 0x1898_19F1, d2: 0x1DB6, d3: 0x4B57, d4: [0xBE, 0x54, 0x18, 0x21, 0x33, 0x9B, 0x85, 0xF7] };
const IID_FENCE: Guid =
    Guid { d1: 0x0a75_3dcf, d2: 0xc4d8, d3: 0x4b91, d4: [0xad, 0xf6, 0xbe, 0x5a, 0x60, 0xd9, 0x5a, 0x76] };
const DEVICE_SLOT_CREATE_FENCE: usize = 36;
const CHILD_SLOT_GET_DEVICE: usize = 7;

pub fn wait_reg_mem_function(func: u32) -> u32 {
    match func {
        FUNC_LESS => 1,
        FUNC_EQUAL => 3,
        FUNC_LESS_EQUAL => 2,
        FUNC_GREATER => 6,
        FUNC_GREATER_EQUAL => 5,
        FUNC_ALWAYS => 0,
        FUNC_NOT_EQUAL => 4,
        _ => 4,
    }
}

pub fn compare(func: u32, mem: u64, mask: u64, reference: u64) -> bool {
    let m = mem & mask;
    match wait_reg_mem_function(func) {
        0 => true,
        1 => m < reference,
        2 => m <= reference,
        3 => m == reference,
        4 => m != reference,
        5 => m >= reference,
        _ => m > reference,
    }
}

pub fn poll_interval(flags: u32) -> u32 {
    match flags >> 16 {
        0 => DEFAULT_POLL_INTERVAL,
        n => n,
    }
}

pub fn wait_args_error(va: u64, func: u32, flags: u32, align: u64) -> Option<&'static str> {
    if flags & WAIT_FLAGS_RESERVED != 0 {
        return Some("invalid flags");
    }
    if poll_interval(flags) < 4 {
        return Some("poll interval must be at least 4 (or 0 for the default)");
    }
    if func == FUNC_NEVER || func as i32 > FUNC_ALWAYS as i32 {
        return Some("invalid comparison function");
    }
    if flags & WAIT_FLAG_PFP != 0 && func != FUNC_GREATER_EQUAL {
        return Some("waits using the PFP engine support only D3D12_COMPARISON_FUNC_GREATER_EQUAL");
    }
    if va & crate::console_slots::GRAPHICS_VA_HIGH_MASK != 0 {
        return Some("address must be in graphics memory");
    }
    if va % align != 0 {
        return Some(if align == 8 { "address must be 8 byte aligned" } else { "address must be 4 byte aligned" });
    }
    None
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Op {
    Write { addr: u64, value: u64 },
    Wait { addr: u64, func: u32, value: u64, mask: u64 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Step {
    WaitAddr { addr: u64, seq: u64 },
    WaitCond { id: u64 },
    Exec { index: usize },
    SignalAddr { addr: u64, seq: u64 },
    SignalCond { id: u64 },
    Unresolved { addr: u64, func: u32, value: u64, mask: u64 },
    WaitPoll { id: u64, addr: u64, func: u32, value: u64, mask: u64 },
    WaitProbe { id: u64, addr: u64, func: u32, value: u64, mask: u64 },
}

struct Pending {
    id: u64,
    func: u32,
    value: u64,
    mask: u64,
}

#[derive(Default)]
struct AddrBook {
    seq: u64,
    value: u64,
    pending: Vec<Pending>,
}

#[derive(Default)]
pub struct Book {
    addrs: HashMap<u64, AddrBook>,
    next_cond: u64,
}

#[cfg(test)]
pub fn plan(book: &mut Book, lists: &[&[Op]]) -> Vec<Step> {
    plan_with(book, lists, &|_| Watch::None)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Watch {
    Poll,
    Probe,
    None,
}

pub fn watch_of(addr: u64) -> Watch {
    if cpu_address(addr).is_some() {
        Watch::Poll
    } else if crate::wait_probe::probe_target(addr, false).is_some() {
        Watch::Probe
    } else {
        Watch::None
    }
}

pub fn plan_with(book: &mut Book, lists: &[&[Op]], watch: &dyn Fn(u64) -> Watch) -> Vec<Step> {
    let mut steps = Vec::new();
    for (index, ops) in lists.iter().enumerate() {
        let mut local: HashMap<u64, u64> = HashMap::new();
        for op in ops.iter() {
            match *op {
                Op::Write { addr, value } => {
                    local.insert(addr, value);
                }
                Op::Wait { addr, func, value, mask } => {
                    if wait_reg_mem_function(func) == 0 {
                        continue;
                    }
                    if let Some(&v) = local.get(&addr) {
                        if compare(func, v, mask, value) {
                            continue;
                        }
                    }
                    let a = book.addrs.entry(addr).or_default();
                    if a.seq != 0 && compare(func, a.value, mask, value) {
                        steps.push(Step::WaitAddr { addr, seq: a.seq });
                    } else {
                        let w = watch(addr);
                        if a.seq == 0 && !local.contains_key(&addr) && w == Watch::None {
                            steps.push(Step::Unresolved { addr, func, value, mask });
                        } else {
                            book.next_cond += 1;
                            let id = book.next_cond;
                            a.pending.push(Pending { id, func, value, mask });
                            steps.push(match w {
                                Watch::Poll => Step::WaitPoll { id, addr, func, value, mask },
                                Watch::Probe => Step::WaitProbe { id, addr, func, value, mask },
                                Watch::None => Step::WaitCond { id },
                            });
                        }
                    }
                }
            }
        }
        steps.push(Step::Exec { index });
        let mut signalled: Vec<u64> = Vec::new();
        for op in ops.iter() {
            if let Op::Write { addr, value } = *op {
                let a = book.addrs.entry(addr).or_default();
                a.seq += 1;
                a.value = value;
                a.pending.retain(|p| {
                    if compare(p.func, value, p.mask, p.value) {
                        steps.push(Step::SignalCond { id: p.id });
                        false
                    } else {
                        true
                    }
                });
                if !signalled.contains(&addr) {
                    signalled.push(addr);
                }
            }
        }
        for addr in signalled {
            let seq = book.addrs[&addr].seq;
            steps.push(Step::SignalAddr { addr, seq });
        }
    }
    steps
}

#[cfg(test)]
pub fn exec_runs(steps: &[Step]) -> Vec<(usize, usize)> {
    let mut runs: Vec<(usize, usize)> = Vec::new();
    let mut prev_exec = false;
    for s in steps {
        match *s {
            Step::Exec { index } => {
                match runs.last_mut() {
                    Some(r) if prev_exec && r.1 == index => r.1 = index + 1,
                    _ => runs.push((index, index + 1)),
                }
                prev_exec = true;
            }
            _ => prev_exec = false,
        }
    }
    runs
}

static RECORDING: Mutex<Option<HashMap<usize, Vec<Op>>>> = Mutex::new(None);
static CLOSED: Mutex<Option<HashMap<usize, Vec<Op>>>> = Mutex::new(None);
static BOOK: Mutex<Option<Book>> = Mutex::new(None);
static FENCES: Mutex<Option<HashMap<(bool, u64), usize>>> = Mutex::new(None);
static DEVICE: AtomicUsize = AtomicUsize::new(0);
static LOG: AtomicUsize = AtomicUsize::new(0);

fn once(n: usize) -> bool {
    LOG.fetch_add(1, Ordering::Relaxed) < n
}

pub fn note(list: *mut c_void, op: Op) {
    RECORDING.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).entry(list as usize).or_default().push(op);
}

pub fn locally_satisfied(list: *mut c_void, op: Op) -> bool {
    let Op::Wait { addr, func, value, mask } = op else { return true };
    let g = RECORDING.lock().unwrap_or_else(|e| e.into_inner());
    let last = g.as_ref().and_then(|m| m.get(&(list as usize))).and_then(|ops| {
        ops.iter().rev().find_map(|o| match *o { Op::Write { addr: a, value: v } if a == addr => Some(v), _ => None })
    });
    matches!(last, Some(v) if compare(func, v, mask, value))
}

pub fn on_close(list: *mut c_void) {
    let ops = RECORDING.lock().unwrap_or_else(|e| e.into_inner()).as_mut().and_then(|m| m.remove(&(list as usize))).unwrap_or_default();
    let mut g = CLOSED.lock().unwrap_or_else(|e| e.into_inner());
    let m = g.get_or_insert_with(HashMap::new);
    if ops.is_empty() {
        m.remove(&(list as usize));
    } else {
        m.insert(list as usize, ops);
    }
}

#[cfg(test)]
pub fn closed_ops(list: usize) -> Vec<Op> {
    CLOSED.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(&list).cloned()).unwrap_or_default()
}

unsafe fn vt(obj: *mut c_void) -> *const usize {
    unsafe { *(obj as *const *const usize) }
}

unsafe fn device_of_queue(queue: *mut c_void) -> *mut c_void {
    let d = DEVICE.load(Ordering::Relaxed);
    if d != 0 {
        return d as *mut c_void;
    }
    let get: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32 =
        unsafe { core::mem::transmute(*vt(queue).add(CHILD_SLOT_GET_DEVICE)) };
    let mut dev: *mut c_void = core::ptr::null_mut();
    if unsafe { get(queue, &IID_DEVICE, &mut dev) } < 0 || dev.is_null() {
        return core::ptr::null_mut();
    }
    DEVICE.store(dev as usize, Ordering::Relaxed);
    dev
}

unsafe fn fence(queue: *mut c_void, cond: bool, key: u64) -> *mut c_void {
    let mut g = FENCES.lock().unwrap_or_else(|e| e.into_inner());
    let m = g.get_or_insert_with(HashMap::new);
    if let Some(&f) = m.get(&(cond, key)) {
        return f as *mut c_void;
    }
    let dev = unsafe { device_of_queue(queue) };
    if dev.is_null() {
        return core::ptr::null_mut();
    }
    let create: unsafe extern "system" fn(*mut c_void, u64, u32, *const Guid, *mut *mut c_void) -> i32 =
        unsafe { core::mem::transmute(*vt(dev).add(DEVICE_SLOT_CREATE_FENCE)) };
    let mut f: *mut c_void = core::ptr::null_mut();
    let hr = unsafe { create(dev, 0, 0, &IID_FENCE, &mut f) };
    if hr < 0 || f.is_null() {
        record("d3d12_xs", "gpu_wait", &format!("CreateFence for {} {key:#x} FAILED hr={hr:#010x}", if cond { "condition" } else { "address" }));
        return core::ptr::null_mut();
    }
    m.insert((cond, key), f as usize);
    f
}

pub type ExecFn = unsafe extern "system" fn(*mut c_void, u32, *const *mut c_void);
type FenceOp = unsafe extern "system" fn(*mut c_void, *mut c_void, u64) -> i32;

static SIGNAL_FN: AtomicUsize = AtomicUsize::new(0);
static WAIT_FN: AtomicUsize = AtomicUsize::new(0);
static COND_FREE: Mutex<Vec<(usize, u64)>> = Mutex::new(Vec::new());
static COND_USED: Mutex<Option<HashMap<u64, (usize, u64)>>> = Mutex::new(None);

pub fn cond_take(id: u64, create: impl FnOnce() -> usize) -> Option<(usize, u64)> {
    let got = COND_FREE.lock().unwrap_or_else(|e| e.into_inner()).pop();
    let (f, last) = match got {
        Some(x) => x,
        None => {
            let f = create();
            if f == 0 {
                return None;
            }
            (f, 0)
        }
    };
    let v = last + 1;
    COND_USED.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(id, (f, v));
    Some((f, v))
}

pub fn cond_release(id: u64) -> Option<(usize, u64)> {
    let e = COND_USED.lock().unwrap_or_else(|e| e.into_inner()).as_mut().and_then(|m| m.remove(&id))?;
    COND_FREE.lock().unwrap_or_else(|e| e.into_inner()).push(e);
    Some(e)
}

const FENCE_SLOT_SIGNAL: usize = 10;
const MEM_COMMIT: u32 = 0x1000;
const PAGE_READABLE: u32 = 0x02 | 0x04 | 0x08 | 0x20 | 0x40 | 0x80;
static POLLS: Mutex<Vec<Poll>> = Mutex::new(Vec::new());
static POLLER: std::sync::Once = std::sync::Once::new();

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Poll {
    pub ptr: usize,
    pub wide: bool,
    pub func: u32,
    pub value: u64,
    pub mask: u64,
    pub id: u64,
    pub fence: usize,
    pub target: u64,
}

#[repr(C)]
struct Mbi {
    base: usize,
    alloc_base: usize,
    alloc_protect: u32,
    partition: u16,
    size: usize,
    state: u32,
    protect: u32,
    ty: u32,
}

extern "system" {
    fn VirtualQuery(addr: *const c_void, info: *mut Mbi, len: usize) -> usize;
    fn GetCurrentProcess() -> *mut c_void;
    fn ReadProcessMemory(process: *mut c_void, base: *const c_void, buf: *mut c_void, len: usize, read: *mut usize) -> i32;
}

fn readable(p: usize, len: usize) -> bool {
    let mut m: Mbi = unsafe { core::mem::zeroed() };
    if unsafe { VirtualQuery(p as *const c_void, &mut m, core::mem::size_of::<Mbi>()) } == 0 {
        return false;
    }
    m.state == MEM_COMMIT && m.protect & PAGE_READABLE != 0 && m.protect & 0x100 == 0 && p + len <= m.base + m.size
}

fn cpu_address(addr: u64) -> Option<usize> {
    let (e, off) = crate::gpu_va::lookup(addr)?;
    if !e.buffer {
        return None;
    }
    let p = crate::placed::lookup(e.res as *mut c_void)?;
    let cpu = (p.va + off) as usize;
    readable(cpu, 8).then_some(cpu)
}

fn read_title(ptr: usize, wide: bool) -> Option<u64> {
    let mut v = 0u64;
    let n = if wide { 8 } else { 4 };
    if let Some(b) = crate::title_shadow::read(ptr as u64, n) {
        let mut w = [0u8; 8];
        w[..n].copy_from_slice(&b);
        return Some(u64::from_le_bytes(w));
    }
    let ok = unsafe { ReadProcessMemory(GetCurrentProcess(), ptr as *const c_void, &mut v as *mut u64 as *mut c_void, n, core::ptr::null_mut()) };
    (ok != 0).then_some(v)
}

pub fn cond_live(id: u64) -> bool {
    COND_USED.lock().unwrap_or_else(|e| e.into_inner()).as_ref().is_some_and(|m| m.contains_key(&id))
}

pub fn poll_ready(p: &Poll, mem: u64) -> bool {
    compare(p.func, mem, p.mask, p.value)
}

fn poll_add(p: Poll) {
    POLLS.lock().unwrap_or_else(|e| e.into_inner()).push(p);
    POLLER.call_once(|| {
        std::thread::spawn(|| loop {
            let ready: Vec<Poll> = {
                let mut g = POLLS.lock().unwrap_or_else(|e| e.into_inner());
                let mut r = Vec::new();
                g.retain(|p| {
                    if !cond_live(p.id) {
                        return false;
                    }
                    let Some(mem) = read_title(p.ptr, p.wide) else { return true };
                    if poll_ready(p, mem) { r.push(*p); false } else { true }
                });
                r
            };
            for p in &ready {
                let Some((f, v)) = cond_release(p.id) else { continue };
                let s: unsafe extern "system" fn(*mut c_void, u64) -> i32 = unsafe { core::mem::transmute(*vt(f as *mut c_void).add(FENCE_SLOT_SIGNAL)) };
                unsafe { s(f as *mut c_void, v) };
            }
            let idle = POLLS.lock().unwrap_or_else(|e| e.into_inner()).is_empty();
            std::thread::sleep(std::time::Duration::from_micros(if idle { 2000 } else { 100 }));
        });
    });
}

pub(crate) fn signal_fn() -> usize {
    SIGNAL_FN.load(Ordering::Relaxed)
}

pub(crate) unsafe fn new_fence(queue: *mut c_void) -> usize {
    let dev = unsafe { device_of_queue(queue) };
    if dev.is_null() {
        return 0;
    }
    let create: unsafe extern "system" fn(*mut c_void, u64, u32, *const Guid, *mut *mut c_void) -> i32 =
        unsafe { core::mem::transmute(*vt(dev).add(DEVICE_SLOT_CREATE_FENCE)) };
    let mut f: *mut c_void = core::ptr::null_mut();
    let hr = unsafe { create(dev, 0, 0, &IID_FENCE, &mut f) };
    if hr < 0 || f.is_null() {
        record("d3d12_xs", "gpu_wait", &format!("CreateFence for a wait condition FAILED hr={hr:#010x}"));
        return 0;
    }
    f as usize
}

pub(crate) unsafe fn device_of(queue: *mut c_void) -> *mut c_void {
    unsafe { device_of_queue(queue) }
}

pub(crate) fn wait_fn() -> usize {
    WAIT_FN.load(Ordering::Relaxed)
}

pub fn has_queue_fns() -> bool {
    SIGNAL_FN.load(Ordering::Relaxed) != 0 && WAIT_FN.load(Ordering::Relaxed) != 0
}

pub unsafe fn wait_now(queue: *mut c_void, op: Op, ty: u32) -> bool {
    let (signal, wait) = (SIGNAL_FN.load(Ordering::Relaxed), WAIT_FN.load(Ordering::Relaxed));
    if signal == 0 || wait == 0 {
        return false;
    }
    let mut book = BOOK.lock().unwrap_or_else(|e| e.into_inner());
    let steps = plan_with(book.get_or_insert_with(Book::default), &[&[op][..]], &watch_of);
    let steps: Vec<Step> = steps.into_iter().filter(|s| !matches!(s, Step::Exec { .. })).collect();
    unsafe { run_steps(queue, &steps, &[], None, signal, wait, ty) };
    true
}

pub unsafe fn submit(queue: *mut c_void, lists: &[*mut c_void], exec: ExecFn, signal: usize, wait: usize) {
    SIGNAL_FN.store(signal, Ordering::Relaxed);
    WAIT_FN.store(wait, Ordering::Relaxed);
    crate::wait_probe::set_exec(exec as usize);
    let ty = lists.first().map(|&l| unsafe { crate::fill::list_type(l) }).unwrap_or(0);
    let ops: Vec<Vec<Op>> = {
        let g = CLOSED.lock().unwrap_or_else(|e| e.into_inner());
        match g.as_ref() {
            Some(m) if !m.is_empty() => lists.iter().map(|l| m.get(&(*l as usize)).cloned().unwrap_or_default()).collect(),
            _ => Vec::new(),
        }
    };
    if ops.iter().all(|o| o.is_empty()) || signal == 0 || wait == 0 {
        unsafe { exec(queue, lists.len() as u32, lists.as_ptr()) };
    } else {
        let refs: Vec<&[Op]> = ops.iter().map(|v| v.as_slice()).collect();
        let mut book = BOOK.lock().unwrap_or_else(|e| e.into_inner());
        let steps = plan_with(book.get_or_insert_with(Book::default), &refs, &watch_of);
        unsafe { run_steps(queue, &steps, lists, Some(exec), signal, wait, ty) };
    }
    unsafe { crate::wait_probe::after_submit(queue, ty) };
}

unsafe fn run_steps(queue: *mut c_void, steps: &[Step], lists: &[*mut c_void], exec: Option<ExecFn>, signal: usize, wait: usize, ty: u32) {
    let signal: FenceOp = unsafe { core::mem::transmute(signal) };
    let wait: FenceOp = unsafe { core::mem::transmute(wait) };
    let mut i = 0;
    while i < steps.len() {
        match steps[i] {
            Step::Exec { index } => {
                let mut end = index + 1;
                while let Some(Step::Exec { index: n }) = steps.get(i + 1) {
                    if *n != end {
                        break;
                    }
                    end += 1;
                    i += 1;
                }
                if let Some(exec) = exec {
                    unsafe { exec(queue, (end - index) as u32, lists[index..end].as_ptr()) };
                }
            }
            Step::WaitAddr { addr, seq } => {
                crate::qwait::step(crate::qwait::STEP_ADDR);
                let f = unsafe { fence(queue, false, addr) };
                if !f.is_null() {
                    unsafe { wait(queue, f, seq) };
                }
            }
            Step::SignalAddr { addr, seq } => {
                let f = unsafe { fence(queue, false, addr) };
                if !f.is_null() {
                    unsafe { signal(queue, f, seq) };
                }
            }
            Step::WaitCond { id } => {
                crate::qwait::step(crate::qwait::STEP_COND);
                if let Some((f, v)) = cond_take(id, || unsafe { new_fence(queue) }) {
                    unsafe { wait(queue, f as *mut c_void, v) };
                }
                if once(16) {
                    record("d3d12_xs", "gpu_wait", &format!("queue {queue:p} waits for a value not yet written by any submitted GPU work (condition {id})"));
                }
            }
            Step::SignalCond { id } => {
                if let Some((f, v)) = cond_release(id) {
                    unsafe { signal(queue, f as *mut c_void, v) };
                }
            }
            Step::WaitProbe { id, addr, func, value, mask } => {
                crate::qwait::step(crate::qwait::STEP_PROBE);
                let armed = crate::wait_probe::probe_target(addr, mask > u32::MAX as u64).and_then(|(res, off)| {
                    let (f, v) = cond_take(id, || unsafe { new_fence(queue) })?;
                    unsafe { crate::wait_probe::arm(queue, ty, crate::wait_probe::Probe { id, res, off, wide: mask > u32::MAX as u64, func, value, mask }) };
                    unsafe { wait(queue, f as *mut c_void, v) };
                    Some(())
                });
                if armed.is_none() && once(16) {
                    record("d3d12_xs", "gpu_wait", &format!("Wait*BitValueX addr={addr:#x}: GPU buffer no longer tracked before the wait was armed; executed without the wait"));
                }
            }
            Step::WaitPoll { id, addr, func, value, mask } => {
                crate::qwait::step(crate::qwait::STEP_POLL);
                let armed = cpu_address(addr).and_then(|p| {
                    let (f, v) = cond_take(id, || unsafe { new_fence(queue) })?;
                    unsafe { wait(queue, f as *mut c_void, v) };
                    poll_add(Poll { ptr: p, wide: mask > u32::MAX as u64, func, value, mask, id, fence: f, target: v });
                    Some(())
                });
                if armed.is_none() && once(16) {
                    record("d3d12_xs", "gpu_wait", &format!("Wait*BitValueX addr={addr:#x}: title memory stopped being CPU-visible before the wait was armed; executed without the wait"));
                }
            }
            Step::Unresolved { addr, func, value, .. } => {
                if once(16) {
                    record("d3d12_xs", "gpu_wait", &format!("Wait*BitValueX addr={addr:#x} func={func} value={value:#x}: no submitted GPU write and the memory is not CPU-visible title memory (shader-written default-heap memory has no PC fence); executed without the wait"));
                }
            }
        }
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comparison_functions_map_like_the_debug_umd() {
        assert_eq!(wait_reg_mem_function(FUNC_LESS), 1);
        assert_eq!(wait_reg_mem_function(FUNC_LESS_EQUAL), 2);
        assert_eq!(wait_reg_mem_function(FUNC_EQUAL), 3);
        assert_eq!(wait_reg_mem_function(FUNC_NOT_EQUAL), 4);
        assert_eq!(wait_reg_mem_function(0), 4);
        assert_eq!(wait_reg_mem_function(FUNC_GREATER_EQUAL), 5);
        assert_eq!(wait_reg_mem_function(FUNC_GREATER), 6);
        assert_eq!(wait_reg_mem_function(FUNC_ALWAYS), 0);
        assert!(compare(FUNC_GREATER_EQUAL, 5, u64::MAX, 5));
        assert!(!compare(FUNC_GREATER_EQUAL, 4, u64::MAX, 5));
        assert!(compare(FUNC_EQUAL, 0x1_0005, 0xff, 5));
        assert!(compare(FUNC_LESS, 4, u64::MAX, 5));
        assert!(compare(FUNC_GREATER, 6, u64::MAX, 5));
        assert!(compare(FUNC_ALWAYS, 0, 0, 9));
    }

    #[test]
    fn wait_validation_matches_debug_umd() {
        assert_eq!(wait_args_error(0x1000, FUNC_GREATER_EQUAL, 0, 4), None);
        assert_eq!(wait_args_error(0x1000, FUNC_GREATER_EQUAL, 1, 4), None);
        assert!(wait_args_error(0x1000, FUNC_EQUAL, 1, 4).is_some());
        assert!(wait_args_error(0x1000, FUNC_EQUAL, 2, 4).is_some());
        assert!(wait_args_error(0x1000, FUNC_EQUAL, 3 << 16, 4).is_some());
        assert_eq!(wait_args_error(0x1000, FUNC_EQUAL, 4 << 16, 4), None);
        assert!(wait_args_error(0x1000, FUNC_NEVER, 0, 4).is_some());
        assert!(wait_args_error(0x1000, 9, 0, 4).is_some());
        assert!(wait_args_error(0x1002, FUNC_EQUAL, 0, 4).is_some());
        assert!(wait_args_error(0x1004, FUNC_EQUAL, 0, 8).is_some());
        assert!(wait_args_error(0x100_0000_0000, FUNC_EQUAL, 0, 4).is_some());
        assert_eq!(poll_interval(0), 8);
        assert_eq!(poll_interval(0x20_0000), 0x20);
    }

    #[test]
    fn cross_queue_fence_pattern_becomes_fence_wait() {
        let mut b = Book::default();
        let producer = [Op::Write { addr: 0x100, value: 7 }];
        let s = plan(&mut b, &[&producer]);
        assert_eq!(s, vec![Step::Exec { index: 0 }, Step::SignalAddr { addr: 0x100, seq: 1 }]);
        let consumer = [Op::Wait { addr: 0x100, func: FUNC_GREATER_EQUAL, value: 7, mask: u64::MAX }];
        let s = plan(&mut b, &[&consumer]);
        assert_eq!(s, vec![Step::WaitAddr { addr: 0x100, seq: 1 }, Step::Exec { index: 0 }]);
    }

    #[test]
    fn wait_before_its_producer_is_submitted_uses_a_condition_fence() {
        let mut b = Book::default();
        plan(&mut b, &[&[Op::Write { addr: 0x200, value: 1 }][..]]);
        let consumer = [Op::Wait { addr: 0x200, func: FUNC_EQUAL, value: 2, mask: u64::MAX }];
        let s = plan(&mut b, &[&consumer]);
        assert_eq!(s, vec![Step::WaitCond { id: 1 }, Step::Exec { index: 0 }]);
        let s = plan(&mut b, &[&[Op::Write { addr: 0x200, value: 3 }][..]]);
        assert_eq!(s, vec![Step::Exec { index: 0 }, Step::SignalAddr { addr: 0x200, seq: 2 }]);
        let s = plan(&mut b, &[&[Op::Write { addr: 0x200, value: 2 }, Op::Write { addr: 0x200, value: 5 }][..]]);
        assert_eq!(s, vec![Step::Exec { index: 0 }, Step::SignalCond { id: 1 }, Step::SignalAddr { addr: 0x200, seq: 4 }]);
    }

    #[test]
    fn same_list_and_same_batch_producers_need_no_queue_wait() {
        let mut b = Book::default();
        let l0 = [Op::Write { addr: 0x300, value: 4 }, Op::Wait { addr: 0x300, func: FUNC_GREATER_EQUAL, value: 4, mask: u64::MAX }];
        let l1 = [Op::Wait { addr: 0x300, func: FUNC_EQUAL, value: 4, mask: u64::MAX }];
        let s = plan(&mut b, &[&l0, &l1]);
        assert_eq!(s, vec![Step::Exec { index: 0 }, Step::SignalAddr { addr: 0x300, seq: 1 }, Step::WaitAddr { addr: 0x300, seq: 1 }, Step::Exec { index: 1 }]);
        assert_eq!(exec_runs(&s), vec![(0, 1), (1, 2)]);
        let s = plan(&mut b, &[&[][..], &[][..], &l1]);
        assert_eq!(exec_runs(&s), vec![(0, 2), (2, 3)]);
    }

    #[test]
    fn untracked_memory_is_reported_and_always_needs_no_wait() {
        let mut b = Book::default();
        let s = plan(&mut b, &[&[Op::Wait { addr: 0x400, func: FUNC_EQUAL, value: 1, mask: u64::MAX }][..]]);
        assert_eq!(s, vec![Step::Unresolved { addr: 0x400, func: FUNC_EQUAL, value: 1, mask: u64::MAX }, Step::Exec { index: 0 }]);
        let s = plan(&mut b, &[&[Op::Wait { addr: 0x400, func: FUNC_ALWAYS, value: 1, mask: 0 }][..]]);
        assert_eq!(s, vec![Step::Exec { index: 0 }]);
    }

    static COND_POOL_TEST: Mutex<()> = Mutex::new(());

    #[test]
    fn condition_fences_are_pooled_with_rising_values() {
        let _pool = COND_POOL_TEST.lock().unwrap_or_else(|e| e.into_inner());
        COND_FREE.lock().unwrap_or_else(|e| e.into_inner()).clear();
        let mut made = 0;
        let a = cond_take(9001, || { made += 1; 0xf00 }).unwrap();
        assert_eq!(a, (0xf00, 1));
        assert_eq!(cond_release(9001), Some((0xf00, 1)));
        assert_eq!(cond_release(9001), None);
        let b = cond_take(9002, || { made += 1; 0xf10 }).unwrap();
        assert_eq!(b, (0xf00, 2));
        assert_eq!(made, 1);
        assert_eq!(cond_take(9003, || 0), None);
        cond_release(9002);
    }

    #[test]
    fn waits_on_an_earlier_write_in_the_same_list_do_not_split() {
        let l = 0x6160usize as *mut c_void;
        let w = |v| Op::Wait { addr: 0x40, func: FUNC_GREATER_EQUAL, value: v, mask: u64::MAX };
        assert!(!locally_satisfied(l, w(1)));
        note(l, Op::Write { addr: 0x40, value: 3 });
        assert!(locally_satisfied(l, w(3)));
        assert!(!locally_satisfied(l, w(4)));
        note(l, Op::Write { addr: 0x40, value: 9 });
        assert!(locally_satisfied(l, w(4)));
        on_close(l);
        assert!(!locally_satisfied(l, w(4)));
    }

    #[test]
    fn cpu_written_waits_poll_title_memory() {
        let p = Poll { ptr: 0, wide: false, func: FUNC_GREATER_EQUAL, value: 5, mask: 0xffff_ffff, id: 1, fence: 0, target: 1 };
        assert!(!poll_ready(&p, 4));
        assert!(poll_ready(&p, 5));
        assert!(poll_ready(&Poll { func: FUNC_EQUAL, mask: 0xff, ..p }, 0x1105));
        let v = Box::new(7u64);
        assert!(readable(&*v as *const u64 as usize, 8));
        assert!(!readable(0, 8));
        assert!(!readable(0x10, 8));
        assert_eq!(read_title(&*v as *const u64 as usize, true), Some(7));
        assert_eq!(read_title(0x10, false), None);
        let mut b = Book::default();
        let w = [Op::Wait { addr: 0x700, func: FUNC_EQUAL, value: 2, mask: u64::MAX }];
        let classify = |a: u64| match a { 0x700 => Watch::Poll, 0x800 => Watch::Probe, _ => Watch::None };
        let s = plan_with(&mut b, &[&w], &classify);
        assert_eq!(s, vec![Step::WaitPoll { id: 1, addr: 0x700, func: FUNC_EQUAL, value: 2, mask: u64::MAX }, Step::Exec { index: 0 }]);
        let s = plan_with(&mut b, &[&[Op::Write { addr: 0x700, value: 2 }][..]], &classify);
        assert_eq!(s, vec![Step::Exec { index: 0 }, Step::SignalCond { id: 1 }, Step::SignalAddr { addr: 0x700, seq: 1 }]);
        let shader = [Op::Wait { addr: 0x800, func: FUNC_GREATER_EQUAL, value: 9, mask: 0xffff_ffff }];
        let s = plan_with(&mut b, &[&shader], &classify);
        assert_eq!(s, vec![Step::WaitProbe { id: 2, addr: 0x800, func: FUNC_GREATER_EQUAL, value: 9, mask: 0xffff_ffff }, Step::Exec { index: 0 }]);
        let s = plan_with(&mut b, &[&[Op::Write { addr: 0x800, value: 9 }][..]], &classify);
        assert_eq!(s, vec![Step::Exec { index: 0 }, Step::SignalCond { id: 2 }, Step::SignalAddr { addr: 0x800, seq: 1 }]);
        let s = plan_with(&mut b, &[&[Op::Wait { addr: 0x900, func: FUNC_EQUAL, value: 1, mask: u64::MAX }][..]], &classify);
        assert_eq!(s, vec![Step::Unresolved { addr: 0x900, func: FUNC_EQUAL, value: 1, mask: u64::MAX }, Step::Exec { index: 0 }]);
        let _pool = COND_POOL_TEST.lock().unwrap_or_else(|e| e.into_inner());
        assert!(!cond_live(77_001));
        cond_take(77_001, || 0xabc).unwrap();
        assert!(cond_live(77_001));
        cond_release(77_001);
        assert!(!cond_live(77_001));
    }

    #[test]
    fn ops_are_frozen_at_close() {
        let l = 0x5150usize as *mut c_void;
        note(l, Op::Write { addr: 8, value: 1 });
        assert!(closed_ops(l as usize).is_empty());
        on_close(l);
        assert_eq!(closed_ops(l as usize), vec![Op::Write { addr: 8, value: 1 }]);
        on_close(l);
        assert!(closed_ops(l as usize).is_empty());
    }
}
