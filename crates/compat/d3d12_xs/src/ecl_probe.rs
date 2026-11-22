use compat_common::record;
use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::collections::HashMap;
use std::sync::Mutex;

static STATE: Mutex<Option<HashMap<usize, i32>>> = Mutex::new(None);
static SEEN: AtomicUsize = AtomicUsize::new(0);
static GONE: AtomicBool = AtomicBool::new(false);

pub fn on_close(list: *mut c_void, hr: i32) {
    STATE.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(list as usize, hr);
}

pub fn on_reset(list: *mut c_void) {
    STATE.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(list as usize, 1);
}

pub fn failed_close(list: usize) -> Option<i32> {
    STATE.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(&list).copied()).filter(|&hr| hr < 0)
}

pub fn split_failed(real: Vec<*mut c_void>, failed: impl Fn(usize) -> Option<i32>) -> (Vec<*mut c_void>, Vec<(*mut c_void, i32)>) {
    let mut keep = Vec::with_capacity(real.len());
    let mut refused = Vec::new();
    for l in real {
        match failed(l as usize) {
            Some(hr) => refused.push((l, hr)),
            None => keep.push(l),
        }
    }
    (keep, refused)
}

pub fn refuse_failed(real: Vec<*mut c_void>) -> Vec<*mut c_void> {
    let (keep, refused) = split_failed(real, failed_close);
    if !refused.is_empty() {
        static N: AtomicUsize = AtomicUsize::new(0);
        let n = N.fetch_add(refused.len(), Ordering::Relaxed);
        if n < 32 || n.is_power_of_two() {
            record("d3d12_xs", "ecl.refuse", &format!("ERROR: {} command list(s) whose real Close failed were not submitted (the PC runtime removes the device for them; their recorded GPU work is lost; the title already got the failing hr from Close): {:?}; {} list(s) of this ExecuteCommandLists still run", refused.len(), refused, keep.len()));
        }
    }
    keep
}

fn close_state(list: usize) -> String {
    match STATE.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(&list).copied()) {
        None => "no title Reset/Close seen".into(),
        Some(1) => "open (Reset, no Close)".into(),
        Some(hr) if hr < 0 => format!("Close failed {hr:#010x}"),
        Some(_) => "closed".into(),
    }
}

unsafe fn vslot(obj: *mut c_void, i: usize) -> usize {
    unsafe { *(*(obj as *const *const usize)).add(i) }
}

pub unsafe fn removed_reason(obj: *mut c_void) -> Option<i32> {
    if obj.is_null() {
        return None;
    }
    let iid = crate::device_wrap::Guid { d1: 0x189819f1, d2: 0x1db6, d3: 0x4b57, d4: [0xbe, 0x54, 0x18, 0x21, 0x33, 0x9b, 0x85, 0xf7] };
    let get_device: unsafe extern "system" fn(*mut c_void, *const crate::device_wrap::Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(vslot(obj, 7)) };
    let mut dev: *mut c_void = core::ptr::null_mut();
    if unsafe { get_device(obj, &iid, &mut dev) } < 0 || dev.is_null() {
        return None;
    }
    let reason: unsafe extern "system" fn(*mut c_void) -> i32 = unsafe { core::mem::transmute(vslot(dev, 37)) };
    let hr = unsafe { reason(dev) };
    let release: unsafe extern "system" fn(*mut c_void) -> u32 = unsafe { core::mem::transmute(vslot(dev, 2)) };
    unsafe { release(dev) };
    Some(hr)
}

pub unsafe fn queue_desc(queue: *mut c_void, get_desc: usize) -> [u32; 4] {
    let mut d = [0u32; 4];
    if get_desc != 0 {
        let f: unsafe extern "system" fn(*mut c_void, *mut [u32; 4]) -> *mut [u32; 4] = unsafe { core::mem::transmute(get_desc) };
        unsafe { f(queue, &mut d) };
    }
    d
}

pub unsafe fn check(tag: &str, queue: *mut c_void, get_desc: usize, title: &[*mut c_void], real: &[*mut c_void], before: Option<i32>) {
    if GONE.load(Ordering::Relaxed) {
        return;
    }
    let after = unsafe { removed_reason(queue) };
    let n = SEEN.fetch_add(1, Ordering::Relaxed);
    let removed_here = before == Some(0) && after.is_some_and(|h| h != 0);
    if n >= 6 && !removed_here && after == Some(0) {
        return;
    }
    let d = unsafe { queue_desc(queue, get_desc) };
    let lists: Vec<String> = real
        .iter()
        .map(|&l| format!("{l:p} type={} {}", unsafe { crate::fill::list_type(l) }, close_state(l as usize)))
        .collect();
    record("d3d12_xs", "ecl.probe", &format!(
        "{tag} #{n} queue={queue:p} type={} prio={} flags={:#x} node={:#x} title lists={title:?} real lists=[{}] removed before={before:?} after={after:?}",
        d[0], d[1], d[2], d[3], lists.join("; ")
    ));
    if after.is_some_and(|h| h != 0) {
        GONE.store(true, Ordering::Relaxed);
        record("d3d12_xs", "ecl.probe", &format!("device removed {} this submission", if removed_here { "by" } else { "before" }));
    }
}

const PROBE_SLOTS: usize = 64;
static PREV: [AtomicUsize; PROBE_SLOTS] = [const { AtomicUsize::new(0) }; PROBE_SLOTS];
static LAST: Mutex<Option<&'static str>> = Mutex::new(None);

pub const PROBED: [(usize, &str, usize, usize); 19] = [
    (14, "CreateDescriptorHeap", 1, 0x10),
    (17, "CreateConstantBufferView", 1, 0x10),
    (18, "CreateShaderResourceView", 2, 0x28),
    (19, "CreateUnorderedAccessView", 3, 0x28),
    (20, "CreateRenderTargetView", 2, 0x20),
    (21, "CreateDepthStencilView", 2, 0x20),
    (22, "CreateSampler", 1, 0x34),
    (23, "CopyDescriptors", 0, 0),
    (24, "CopyDescriptorsSimple", 0, 0),
    (27, "CreateCommittedResource", 3, 0x38),
    (28, "CreateHeap", 1, 0x28),
    (29, "CreatePlacedResource", 3, 0x38),
    (30, "CreateReservedResource", 1, 0x38),
    (10, "CreateGraphicsPipelineState", 0, 0),
    (11, "CreateComputePipelineState", 0, 0),
    (16, "CreateRootSignature", 0, 0),
    (44, "CreatePlacedResourceX", 2, 0x38),
    (60, "CreateGraphicsPipelineStateX", 0, 0),
    (62, "CreateComputePipelineStateX", 0, 0),
];

pub fn note_last(tag: &'static str) {
    *LAST.lock().unwrap_or_else(|e| e.into_inner()) = Some(tag);
}

pub fn last() -> Option<&'static str> {
    *LAST.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn removed_now(was: Option<i32>, now: Option<i32>) -> bool {
    was == Some(0) && now.is_some_and(|h| h != 0)
}

pub fn describe_args(args: &[usize; 8], desc: Option<&[u8]>) -> String {
    let a: Vec<String> = args.iter().map(|v| format!("{v:#x}")).collect();
    match desc {
        Some(d) => format!("args [{}] desc {:02x?}", a.join(", "), d),
        None => format!("args [{}]", a.join(", ")),
    }
}

unsafe fn reason_of(dev: *mut c_void) -> Option<i32> {
    if dev.is_null() {
        return None;
    }
    let reason: unsafe extern "system" fn(*mut c_void) -> i32 = unsafe { core::mem::transmute(vslot(dev, 37)) };
    Some(unsafe { reason(dev) })
}

unsafe fn probed_call(i: usize, args: [usize; 8]) -> usize {
    let (slot, tag, di, dl) = PROBED[i];
    let real = unsafe { *((args[0] + 8) as *const *mut c_void) };
    let watch = !GONE.load(Ordering::Relaxed);
    let was = if watch { unsafe { reason_of(real) } } else { None };
    if watch && was.is_some_and(|h| h != 0) && !GONE.swap(true, Ordering::Relaxed) {
        record("d3d12_xs", "ecl.probe", &format!("device already removed (GetDeviceRemovedReason={:#010x}) when the title called {tag}; the removing call ran below this layer after the last probed call {:?}", was.unwrap_or(0), last()));
    }
    let f: unsafe extern "system" fn(usize, usize, usize, usize, usize, usize, usize, usize) -> usize = unsafe { core::mem::transmute(PREV[slot].load(Ordering::Acquire)) };
    let r = unsafe { f(args[0], args[1], args[2], args[3], args[4], args[5], args[6], args[7]) };
    if watch && !GONE.load(Ordering::Relaxed) {
        let now = unsafe { reason_of(real) };
        if removed_now(was, now) && !GONE.swap(true, Ordering::Relaxed) {
            let d = if di != 0 && args[di] != 0 { Some(unsafe { core::slice::from_raw_parts(args[di] as *const u8, dl) }) } else { None };
            record("d3d12_xs", "ecl.probe", &format!("device removed by {tag} (slot {slot}, GetDeviceRemovedReason={:#010x}, returned {r:#x}): {}", now.unwrap_or(0), describe_args(&args, d)));
            if (18..=21).contains(&slot) {
                let res_args: &[usize] = if slot == 19 { &args[1..3] } else { &args[1..2] };
                for &res in res_args {
                    if res != 0 {
                        record("d3d12_xs", "ecl.probe", &format!("  view resource {res:#x}: {}", unsafe { describe_resource(res as *mut c_void) }));
                    }
                }
            }
        }
        note_last(tag);
    }
    r
}

unsafe fn describe_resource(res: *mut c_void) -> String {
    let mut d = [0u8; 0x38];
    let get_desc: unsafe extern "system" fn(*mut c_void, *mut u8) -> *mut u8 = unsafe { core::mem::transmute(vslot(res, 10)) };
    unsafe { get_desc(res, d.as_mut_ptr()) };
    let real_va: unsafe extern "system" fn(*mut c_void) -> u64 = unsafe { core::mem::transmute(vslot(res, 11)) };
    let va = unsafe { real_va(res) };
    let mut hp = [0u32; 5];
    let mut hf = 0u32;
    let get_heap: unsafe extern "system" fn(*mut c_void, *mut u32, *mut u32) -> i32 = unsafe { core::mem::transmute(vslot(res, 14)) };
    let hhr = unsafe { get_heap(res, hp.as_mut_ptr(), &mut hf) };
    format!(
        "desc {:02x?} gpuVA={va:#x} heap hr={hhr:#x} props={hp:?} flags={hf:#x} placed_offset={:?} placed={:?} alias={}",
        d,
        crate::placed_offset::of(res as usize),
        crate::placed::lookup(res).map(|p| p.va),
        crate::alias::is_aliased(res)
    )
}

macro_rules! probes {
    ($(($name:ident, $i:literal)),* $(,)?) => {
        $(
            unsafe extern "system" fn $name(a0: usize, a1: usize, a2: usize, a3: usize, a4: usize, a5: usize, a6: usize, a7: usize) -> usize {
                unsafe { probed_call($i, [a0, a1, a2, a3, a4, a5, a6, a7]) }
            }
        )*
        const PROBE_FNS: [unsafe extern "system" fn(usize, usize, usize, usize, usize, usize, usize, usize) -> usize; PROBED.len()] = [$($name),*];
    };
}

probes!(
    (p0, 0), (p1, 1), (p2, 2), (p3, 3), (p4, 4), (p5, 5), (p6, 6), (p7, 7), (p8, 8), (p9, 9),
    (p10, 10), (p11, 11), (p12, 12), (p13, 13), (p14, 14), (p15, 15), (p16, 16), (p17, 17), (p18, 18),
);

pub fn device_probe_enabled() -> bool {
    static E: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *E.get_or_init(|| compat_common::live::flag("TOMBOX_DEVICE_PROBE", "device_probe", true))
}

pub fn install_device(v: &mut [usize]) {
    if !device_probe_enabled() {
        return;
    }
    for (i, &(slot, _, _, _)) in PROBED.iter().enumerate() {
        if slot < v.len() && slot < PROBE_SLOTS && v[slot] != 0 {
            PREV[slot].store(v[slot], Ordering::Release);
            v[slot] = PROBE_FNS[i] as usize;
        }
    }
}

pub unsafe fn after_device_call(dev: *mut c_void, tag: &str, detail: impl FnOnce() -> String) {
    if GONE.load(Ordering::Relaxed) || dev.is_null() {
        return;
    }
    let reason: unsafe extern "system" fn(*mut c_void) -> i32 = unsafe { core::mem::transmute(vslot(dev, 37)) };
    let hr = unsafe { reason(dev) };
    if hr != 0 && !GONE.swap(true, Ordering::Relaxed) {
        record("d3d12_xs", "ecl.probe", &format!("device removed by {tag} (GetDeviceRemovedReason={hr:#010x}): {}", detail()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_whose_close_failed_are_kept_out_of_the_submit() {
        let a = 0x1000 as *mut c_void;
        let b = 0x2000 as *mut c_void;
        let c = 0x3000 as *mut c_void;
        let (keep, refused) = split_failed(vec![a, b, c], |l| if l == 0x2000 { Some(0x80070057u32 as i32) } else { None });
        assert_eq!(keep, vec![a, c]);
        assert_eq!(refused, vec![(b, 0x80070057u32 as i32)]);
        let (keep, refused) = split_failed(vec![a, c], |_| None);
        assert_eq!(keep, vec![a, c]);
        assert!(refused.is_empty());
    }

    #[test]
    fn failed_close_ignores_successful_and_reset_lists() {
        let l = 0x7777_0000usize as *mut c_void;
        on_close(l, 0x80070057u32 as i32);
        assert_eq!(failed_close(l as usize), Some(0x80070057u32 as i32));
        on_reset(l);
        assert_eq!(failed_close(l as usize), None);
        on_close(l, 0);
        assert_eq!(failed_close(l as usize), None);
    }

    #[test]
    fn removal_is_blamed_only_on_the_transition() {
        assert!(removed_now(Some(0), Some(0x887a0001u32 as i32)));
        assert!(!removed_now(Some(0), Some(0)));
        assert!(!removed_now(Some(0x887a0001u32 as i32), Some(0x887a0001u32 as i32)));
        assert!(!removed_now(None, Some(0x887a0001u32 as i32)));
    }

    #[test]
    fn probed_slots_are_unique_and_in_range() {
        let mut seen = std::collections::HashSet::new();
        for &(slot, tag, di, dl) in PROBED.iter() {
            assert!(slot < PROBE_SLOTS, "{tag}");
            assert!(seen.insert(slot), "{tag}");
            assert!(di < 8 && (di == 0) == (dl == 0), "{tag}");
        }
        assert_eq!(PROBE_FNS.len(), PROBED.len());
    }

    #[test]
    fn install_wraps_and_forwards() {
        unsafe extern "system" fn fake(_: usize, a: usize, b: usize, _: usize, _: usize, _: usize, _: usize, _: usize) -> usize {
            a + b
        }
        let mut v = vec![0usize; 64];
        v[23] = fake as usize;
        install_device(&mut v);
        assert_ne!(v[23], fake as usize);
        assert_eq!(PREV[23].load(Ordering::Acquire), fake as usize);
        GONE.store(true, Ordering::Relaxed);
        let mut dev = [0usize; 2];
        let f: unsafe extern "system" fn(usize, usize, usize, usize, usize, usize, usize, usize) -> usize = unsafe { core::mem::transmute(v[23]) };
        assert_eq!(unsafe { f(dev.as_mut_ptr() as usize, 2, 3, 0, 0, 0, 0, 0) }, 5);
        assert!(describe_args(&[1, 2, 0, 0, 0, 0, 0, 0], Some(&[0xab])).contains("ab"));
    }
}
