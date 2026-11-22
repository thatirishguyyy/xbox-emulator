use core::ffi::c_void;
use core::sync::atomic::{AtomicI32, AtomicUsize, Ordering};
use std::collections::HashMap;
use std::sync::Mutex;
use compat_common::record;

use crate::device_wrap::Guid;

pub const IID_XBOX_GRAPHICS_COMMAND_LIST5: Guid =
    Guid { d1: 0x5505_0859, d2: 0x4024, d3: 0x474c, d4: [0x98, 0xcf, 0x64, 0x5b, 0x51, 0xdc, 0x48, 0x74] };
const IID_PC_GRAPHICS_COMMAND_LIST5: Guid =
    Guid { d1: 0x5505_0859, d2: 0x4024, d3: 0x474c, d4: [0x87, 0xf5, 0x64, 0x72, 0xea, 0xee, 0x44, 0xea] };
const IID_DEVICE: Guid =
    Guid { d1: 0x1898_19F1, d2: 0x1DB6, d3: 0x4B57, d4: [0xBE, 0x54, 0x18, 0x21, 0x33, 0x9B, 0x85, 0xF7] };
const PC_GRAPHICS_LIST_IIDS: [Guid; 5] = [
    Guid { d1: 0x5b16_0d0f, d2: 0xac1b, d3: 0x4185, d4: [0x8b, 0xa8, 0xb3, 0xae, 0x42, 0xa5, 0xa4, 0x55] },
    Guid { d1: 0x5531_03fb, d2: 0x1fe7, d3: 0x4557, d4: [0xbb, 0x38, 0x94, 0x6d, 0x7d, 0x0e, 0x7c, 0xa7] },
    Guid { d1: 0x38c3_e585, d2: 0xff17, d3: 0x412c, d4: [0x91, 0x50, 0x4f, 0xc6, 0xf9, 0xd7, 0x2a, 0x28] },
    Guid { d1: 0x6fda_83a7, d2: 0xb84c, d3: 0x4e38, d4: [0x9a, 0xc8, 0xc7, 0xbd, 0x22, 0x01, 0x6b, 0x3d] },
    Guid { d1: 0x8754_318e, d2: 0xd3a9, d3: 0x4541, d4: [0x98, 0xcf, 0x64, 0x5b, 0x50, 0xdc, 0x48, 0x74] },
];
const E_NOINTERFACE: i32 = 0x8000_4002u32 as i32;
const FEATURE_OPTIONS6: u32 = 30;

pub const XBOX_SLOT_RS_SET_SHADING_RATE: usize = 124;
pub const XBOX_SLOT_RS_SET_SHADING_RATE_IMAGE: usize = 125;
pub const PC_SLOT_RS_SET_SHADING_RATE: usize = 77;
pub const PC_SLOT_RS_SET_SHADING_RATE_IMAGE: usize = 78;

static REAL_QI: AtomicUsize = AtomicUsize::new(0);
static REAL_RATE: AtomicUsize = AtomicUsize::new(0);
static REAL_RATE_IMAGE: AtomicUsize = AtomicUsize::new(0);
static VRS_TIER: AtomicI32 = AtomicI32::new(-1);
static SHIMS: Mutex<Option<HashMap<usize, usize>>> = Mutex::new(None);

pub unsafe fn install(xbox_vt: &[AtomicUsize], real_vt: *const usize) {
    REAL_QI.store(*real_vt, Ordering::Relaxed);
    REAL_RATE.store(*real_vt.add(PC_SLOT_RS_SET_SHADING_RATE), Ordering::Relaxed);
    REAL_RATE_IMAGE.store(*real_vt.add(PC_SLOT_RS_SET_SHADING_RATE_IMAGE), Ordering::Relaxed);
    xbox_vt[0].store(query_interface_x as usize, Ordering::Relaxed);
    xbox_vt[XBOX_SLOT_RS_SET_SHADING_RATE].store(rs_set_shading_rate_x as usize, Ordering::Relaxed);
    xbox_vt[XBOX_SLOT_RS_SET_SHADING_RATE_IMAGE].store(rs_set_shading_rate_image_x as usize, Ordering::Relaxed);
    REAL_BARRIER.store(*real_vt.add(26), Ordering::Relaxed);
    xbox_vt[26].store(resource_barrier_x as usize, Ordering::Relaxed);
    crate::cl_ext::install(xbox_vt);
    crate::cl_fw::install_null_slots(xbox_vt);
    crate::gds::install(xbox_vt);
    crate::centroid_bind::install(xbox_vt);
    let prev_close = xbox_vt[9].load(Ordering::Relaxed);
    REAL_CLOSE.store(if prev_close != 0 { prev_close } else { *real_vt.add(9) }, Ordering::Relaxed);
    xbox_vt[9].store(close_x as usize, Ordering::Relaxed);
}

static REAL_CLOSE: AtomicUsize = AtomicUsize::new(0);

const IID_INFO_QUEUE: Guid =
    Guid { d1: 0x0742_a90b, d2: 0xc387, d3: 0x483f, d4: [0xb9, 0x46, 0x30, 0xa7, 0xe4, 0xe6, 0x14, 0x58] };

pub unsafe fn dump_debug_messages(list: *mut c_void, tag: &str) {
    let vt = *(list as *const *const usize);
    let get_device: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32 =
        core::mem::transmute(*vt.add(7));
    let mut dev: *mut c_void = core::ptr::null_mut();
    if get_device(list, &IID_DEVICE, &mut dev) < 0 || dev.is_null() {
        return;
    }
    dump_device_messages(dev, tag);
    let dvt = *(dev as *const *const usize);
    let release_dev: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(*dvt.add(2));
    release_dev(dev);
}

pub unsafe fn dump_device_messages(dev: *mut c_void, tag: &str) {
    let dvt = *(dev as *const *const usize);
    let qi: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32 = core::mem::transmute(*dvt);
    let mut iq: *mut c_void = core::ptr::null_mut();
    let have = qi(dev, &IID_INFO_QUEUE, &mut iq) >= 0 && !iq.is_null();
    if !have {
        record("d3d12_xs", tag, "debug layer not active; set TOMBOX_D3D12_DEBUG or enable the layer to name the failing command");
        return;
    }
    let ivt = *(iq as *const *const usize);
    let count: unsafe extern "system" fn(*mut c_void) -> u64 = core::mem::transmute(*ivt.add(8));
    let get: unsafe extern "system" fn(*mut c_void, u64, *mut u8, *mut usize) -> i32 = core::mem::transmute(*ivt.add(5));
    let n = count(iq);
    for i in n.saturating_sub(12)..n {
        let mut len = 0usize;
        if get(iq, i, core::ptr::null_mut(), &mut len) < 0 || len < 32 || len > 1 << 16 {
            continue;
        }
        let mut buf = vec![0u8; len];
        if get(iq, i, buf.as_mut_ptr(), &mut len) < 0 {
            continue;
        }
        let sev = u32::from_le_bytes(buf[4..8].try_into().unwrap());
        let id = u32::from_le_bytes(buf[8..12].try_into().unwrap());
        let p = u64::from_le_bytes(buf[16..24].try_into().unwrap()) as *const u8;
        let dl = u64::from_le_bytes(buf[24..32].try_into().unwrap()) as usize;
        let text = if p.is_null() || dl == 0 { String::new() } else {
            String::from_utf8_lossy(core::slice::from_raw_parts(p, dl.saturating_sub(1))).into_owned()
        };
        record("d3d12_xs", tag, &format!("debug msg #{i} sev={sev} id={id}: {text}"));
    }
    let release: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(*ivt.add(2));
    release(iq);
}

unsafe extern "system" fn close_x(this: *mut c_void) -> i32 {
    let f: unsafe extern "system" fn(*mut c_void) -> i32 = core::mem::transmute(REAL_CLOSE.load(Ordering::Relaxed));
    crate::gds::on_close(this);
    crate::centroid_bind::on_close(this);
    let hr = f(this);
    crate::ecl_probe::on_close(this, hr);
    crate::gpu_wait::on_close(this);
    if hr < 0 {
        static N: AtomicUsize = AtomicUsize::new(0);
        if N.fetch_add(1, Ordering::Relaxed) < 4 {
            record("d3d12_xs", "cmdlist.close", &format!("Close FAILED hr={hr:#010x} list={this:p}"));
            dump_debug_messages(this, "cmdlist.close");
            crate::cmd_spy::dump("cmdlist.close");
        }
    }
    hr
}

static REAL_BARRIER: AtomicUsize = AtomicUsize::new(0);

unsafe extern "system" fn resource_barrier_x(this: *mut c_void, n: u32, barriers: *const u8) {
    let fixed = crate::xone_barrier::sanitize(n, barriers);
    let (n, barriers) = fixed.as_ref().map_or((n, barriers), |v| ((v.len() / 32) as u32, v.as_ptr()));
    if n == 0 {
        return;
    }
    crate::gpu_va::note_barriers(n, barriers);
    if crate::rtcap::enabled() {
        crate::rtcap::note_barriers(n, barriers);
    }
    let f: unsafe extern "system" fn(*mut c_void, u32, *const u8) =
        core::mem::transmute(REAL_BARRIER.load(Ordering::Relaxed));
    f(this, n, barriers)
}

pub fn note_shim(real: *mut c_void, shim: *mut c_void) {
    let mut g = SHIMS.lock().unwrap_or_else(|e| e.into_inner());
    g.get_or_insert_with(HashMap::new).insert(real as usize, shim as usize);
}

pub(crate) fn shim_of(real: *mut c_void) -> Option<usize> {
    let g = SHIMS.lock().unwrap_or_else(|e| e.into_inner());
    g.as_ref().and_then(|m| m.get(&(real as usize)).copied())
}

unsafe fn vrs_tier(list: *mut c_void) -> i32 {
    let t = VRS_TIER.load(Ordering::Relaxed);
    if t >= 0 {
        return t;
    }
    let vt = *(list as *const *const usize);
    let get_device: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32 =
        core::mem::transmute(*vt.add(7));
    let mut dev: *mut c_void = core::ptr::null_mut();
    let mut tier = 0;
    if get_device(list, &IID_DEVICE, &mut dev) >= 0 && !dev.is_null() {
        let dvt = *(dev as *const *const usize);
        let check: unsafe extern "system" fn(*mut c_void, u32, *mut c_void, u32) -> i32 =
            core::mem::transmute(*dvt.add(13));
        let mut opt6 = [0u32; 5];
        if check(dev, FEATURE_OPTIONS6, opt6.as_mut_ptr() as *mut c_void, 20) >= 0 {
            tier = opt6[2] as i32;
        }
        let release: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(*dvt.add(2));
        release(dev);
    }
    VRS_TIER.store(tier, Ordering::Relaxed);
    record("d3d12_xs", "cmdlist.vrs", &format!("PC variable shading rate tier {tier}"));
    tier
}

pub fn xbox_list5_answer(is_list5_iid: bool, tier: i32, pc_qi_ok: bool) -> bool {
    is_list5_iid && tier >= 1 && pc_qi_ok
}

unsafe extern "system" fn query_interface_x(this: *mut c_void, riid: *const Guid, ppv: *mut *mut c_void) -> i32 {
    let qi: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32 =
        core::mem::transmute(REAL_QI.load(Ordering::Relaxed));
    if riid.is_null() || ppv.is_null() {
        return qi(this, riid, ppv);
    }
    if PC_GRAPHICS_LIST_IIDS.iter().any(|g| *g == *riid) {
        let hr = qi(this, riid, ppv);
        if hr >= 0 && !(*ppv).is_null() {
            if let Some(shim) = shim_of(this) {
                *ppv = shim as *mut c_void;
            }
        }
        return hr;
    }
    if *riid != IID_XBOX_GRAPHICS_COMMAND_LIST5 {
        return qi(this, riid, ppv);
    }
    *ppv = core::ptr::null_mut();
    let tier = vrs_tier(this);
    let mut pc5: *mut c_void = core::ptr::null_mut();
    let ok = tier >= 1 && qi(this, &IID_PC_GRAPHICS_COMMAND_LIST5, &mut pc5) >= 0 && !pc5.is_null();
    let shim = shim_of(this);
    if xbox_list5_answer(true, tier, ok) && shim.is_some() {
        *ppv = shim.unwrap() as *mut c_void;
        return 0;
    }
    if !pc5.is_null() {
        let vt = *(pc5 as *const *const usize);
        let release: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(*vt.add(2));
        release(pc5);
    }
    E_NOINTERFACE
}

unsafe extern "system" fn rs_set_shading_rate_x(this: *mut c_void, rate: u32, combiners: *const u32) {
    let tier = VRS_TIER.load(Ordering::Relaxed);
    if tier < 1 {
        return;
    }
    let f: unsafe extern "system" fn(*mut c_void, u32, *const u32) =
        core::mem::transmute(REAL_RATE.load(Ordering::Relaxed));
    f(this, rate, if tier >= 2 { combiners } else { core::ptr::null() });
}

unsafe extern "system" fn rs_set_shading_rate_image_x(this: *mut c_void, image: *mut c_void) {
    if VRS_TIER.load(Ordering::Relaxed) < 2 {
        return;
    }
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void) =
        core::mem::transmute(REAL_RATE_IMAGE.load(Ordering::Relaxed));
    f(this, image);
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HiStencilCompare {
    pub func: u32,
    pub value: u32,
    pub mask: u32,
    pub enable: bool,
}

pub fn decode_hi_stencil(reg: u32) -> HiStencilCompare {
    HiStencilCompare {
        func: reg & 0x7,
        value: (reg >> 4) & 0xff,
        mask: (reg >> 12) & 0xff,
        enable: reg & (1 << 24) != 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hi_stencil_decodes_fortnite_clear_mrt_value() {
        let s = decode_hi_stencil(0x10ff002 | (0x80 << 4));
        assert_eq!(s, HiStencilCompare { func: 2, value: 0x80, mask: 0xff, enable: true });
        assert!(!decode_hi_stencil(0).enable);
    }

    #[test]
    fn xbox_list5_iid_differs_from_pc() {
        assert!(IID_XBOX_GRAPHICS_COMMAND_LIST5 != IID_PC_GRAPHICS_COMMAND_LIST5);
        assert_eq!(IID_XBOX_GRAPHICS_COMMAND_LIST5.d1, IID_PC_GRAPHICS_COMMAND_LIST5.d1);
    }

    #[test]
    fn pc_list_iids_distinct_from_xbox_list5() {
        for g in &PC_GRAPHICS_LIST_IIDS { assert!(*g != IID_XBOX_GRAPHICS_COMMAND_LIST5); }
        for (i, a) in PC_GRAPHICS_LIST_IIDS.iter().enumerate() {
            for b in &PC_GRAPHICS_LIST_IIDS[i + 1..] { assert!(*a != *b); }
        }
    }

    #[test]
    fn vrs_slot_mapping() {
        assert_eq!(XBOX_SLOT_RS_SET_SHADING_RATE - PC_SLOT_RS_SET_SHADING_RATE, 47);
        assert_eq!(XBOX_SLOT_RS_SET_SHADING_RATE_IMAGE, XBOX_SLOT_RS_SET_SHADING_RATE + 1);
        assert_eq!(PC_SLOT_RS_SET_SHADING_RATE_IMAGE, PC_SLOT_RS_SET_SHADING_RATE + 1);
    }

    #[test]
    fn list5_answer_rules() {
        assert!(xbox_list5_answer(true, 1, true));
        assert!(xbox_list5_answer(true, 2, true));
        assert!(!xbox_list5_answer(true, 0, true));
        assert!(!xbox_list5_answer(true, 2, false));
        assert!(!xbox_list5_answer(false, 2, true));
    }

    #[test]
    fn shim_registry() {
        note_shim(0x1000 as *mut c_void, 0x2000 as *mut c_void);
        assert_eq!(shim_of(0x1000 as *mut c_void), Some(0x2000));
        assert_eq!(shim_of(0x3000 as *mut c_void), None);
    }
}
