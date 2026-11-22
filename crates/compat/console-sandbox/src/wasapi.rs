use core::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use crate::gcdcom::Guid;

const CLSID_MM_DEVICE_ENUMERATOR: Guid = Guid { data1: 0xBCDE0395, data2: 0xE52F, data3: 0x467C, data4: [0x8E, 0x3D, 0xC4, 0x57, 0x92, 0x91, 0x69, 0x2E] };
const IID_IMM_DEVICE_ENUMERATOR: Guid = Guid { data1: 0xA95664D2, data2: 0x9614, data3: 0x4F35, data4: [0xA7, 0x46, 0xDE, 0x8D, 0xB6, 0x36, 0x17, 0xE6] };
const IID_IAUDIO_CLIENT3: Guid = Guid { data1: 0x7ED4EE07, data2: 0x8E67, data3: 0x4CD4, data4: [0x8C, 0x1A, 0x2B, 0x7A, 0x59, 0x87, 0xAD, 0x42] };
const IID_IAUDIO_CLIENT: Guid = Guid { data1: 0x1CB9AD4C, data2: 0xDBFA, data3: 0x4C32, data4: [0xB1, 0x78, 0xC2, 0xF5, 0x68, 0xA7, 0x03, 0xB2] };
const IID_IAUDIO_RENDER_CLIENT: Guid = Guid { data1: 0xF294ACFC, data2: 0x3146, data3: 0x4483, data4: [0xA7, 0xBF, 0xAD, 0xDC, 0xA7, 0xC2, 0x60, 0xE2] };
const IID_ISIMPLE_AUDIO_VOLUME: Guid = Guid { data1: 0x87CE5498, data2: 0x68D6, data3: 0x44E5, data4: [0x92, 0x15, 0x6D, 0xA4, 0x7E, 0xF8, 0x83, 0xD8] };
const IID_IAUDIO_STREAM_VOLUME: Guid = Guid { data1: 0x93014887, data2: 0x242D, data3: 0x4068, data4: [0x8A, 0x15, 0xCF, 0x5E, 0x93, 0xB9, 0x0F, 0xE3] };
const IID_IAUDIO_ENDPOINT_VOLUME: Guid = Guid { data1: 0x5CDF2C82, data2: 0x841E, data3: 0x4546, data4: [0x97, 0x22, 0x0C, 0xF7, 0x40, 0x78, 0x22, 0x9A] };
const IID_IAUDIO_SESSION_MANAGER: Guid = Guid { data1: 0xBFA971F1, data2: 0x4D5E, data3: 0x40BB, data4: [0x93, 0x5E, 0x96, 0x70, 0x39, 0xBF, 0xBE, 0xE4] };
const IID_IAUDIO_SESSION_CONTROL2: Guid = Guid { data1: 0xBFB7FF88, data2: 0x7239, data3: 0x4FC9, data4: [0x8F, 0xA2, 0x07, 0xC9, 0x50, 0xBE, 0x9C, 0x6D] };
const IID_ISPATIAL_AUDIO_CLIENT: Guid = Guid { data1: 0xBBF8E066, data2: 0xAAAA, data3: 0x49BE, data4: [0x9A, 0x4D, 0xFD, 0x2A, 0x85, 0x8E, 0xA2, 0x7F] };
const IID_ISPATIAL_AUDIO_CLIENT2: Guid = Guid { data1: 0xCAABE452, data2: 0xA66A, data3: 0x4BEE, data4: [0xA9, 0x3E, 0xE3, 0x20, 0x46, 0x3F, 0x6A, 0x53] };
const CLSCTX_ALL: u32 = 0x17;
pub const SHARED: u32 = 0;
const AUTOCONVERT: u32 = 0x8000_0000 | 0x0800_0000;
pub const MAX_FORMAT: usize = 0x48;
pub const ID_CHARS: usize = 0x37;
pub const E_NOINTERFACE: i32 = 0x8000_4002u32 as i32;
pub const E_INVALIDARG: i32 = 0x8007_0057u32 as i32;
pub const E_NOTFOUND: i32 = 0x8007_0490u32 as i32;
pub const AUDCLNT_E_NOT_STOPPED: i32 = 0x8889_0005u32 as i32;

#[link(name = "ole32")]
extern "system" {
    fn CoInitializeEx(r: *mut c_void, f: u32) -> i32;
    fn CoCreateInstance(clsid: *const Guid, outer: *mut c_void, ctx: u32, iid: *const Guid, out: *mut *mut c_void) -> i32;
    fn CoTaskMemFree(p: *mut c_void);
    fn PropVariantClear(p: *mut PropVariant) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn SetEvent(h: usize) -> i32;
    fn Sleep(ms: u32);
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PropVariant {
    pub vt: u16,
    pub r1: u16,
    pub r2: u16,
    pub r3: u16,
    pub a: u64,
    pub b: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct PropertyKey {
    pub fmtid: [u8; 16],
    pub pid: u32,
}

unsafe fn slot<T: Copy>(obj: *mut c_void, n: usize) -> T {
    let vt = *(obj as *const *const usize);
    core::mem::transmute_copy(&*vt.add(n))
}

unsafe fn release(obj: *mut c_void) {
    if !obj.is_null() {
        slot::<unsafe extern "system" fn(*mut c_void) -> u32>(obj, 2)(obj);
    }
}

struct Com(*mut c_void);

impl Drop for Com {
    fn drop(&mut self) {
        unsafe { release(self.0) }
    }
}

pub fn format_len(f: &[u8]) -> usize {
    if f.len() < 0x12 {
        return 0;
    }
    let cb = u16::from_le_bytes([f[0x10], f[0x11]]) as usize;
    (0x12 + cb).min(MAX_FORMAT)
}

pub fn block_align(f: &[u8]) -> u32 {
    u16::from_le_bytes([f[0xc], f[0xd]]) as u32
}

pub fn sample_rate(f: &[u8]) -> u32 {
    u32::from_le_bytes(f[4..8].try_into().unwrap())
}

pub fn channels(f: &[u8]) -> u32 {
    u16::from_le_bytes([f[2], f[3]]) as u32
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

unsafe fn take_wide(p: *mut u16) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut n = 0;
    while *p.add(n) != 0 {
        n += 1;
    }
    let s = String::from_utf16_lossy(core::slice::from_raw_parts(p, n));
    CoTaskMemFree(p as *mut c_void);
    s
}

unsafe fn take_format(f: *mut u8) -> Vec<u8> {
    let head = core::slice::from_raw_parts(f, 0x12);
    let v = core::slice::from_raw_parts(f, format_len(head)).to_vec();
    CoTaskMemFree(f as *mut c_void);
    v
}

unsafe fn enumerator() -> Result<Com, i32> {
    CoInitializeEx(core::ptr::null_mut(), 0);
    let mut en = core::ptr::null_mut();
    let hr = CoCreateInstance(&CLSID_MM_DEVICE_ENUMERATOR, core::ptr::null_mut(), CLSCTX_ALL, &IID_IMM_DEVICE_ENUMERATOR, &mut en);
    if hr < 0 { Err(hr) } else { Ok(Com(en)) }
}

unsafe fn device(id: Option<&str>) -> Result<Com, i32> {
    let en = enumerator()?;
    let mut dev = core::ptr::null_mut();
    let hr = match id {
        Some(s) => {
            let w = wide(s);
            slot::<unsafe extern "system" fn(*mut c_void, *const u16, *mut *mut c_void) -> i32>(en.0, 5)(en.0, w.as_ptr(), &mut dev)
        }
        None => slot::<unsafe extern "system" fn(*mut c_void, u32, u32, *mut *mut c_void) -> i32>(en.0, 4)(en.0, 0, 0, &mut dev),
    };
    if hr < 0 { Err(hr) } else { Ok(Com(dev)) }
}

unsafe fn activate(dev: &Com, iid: &Guid) -> Result<Com, i32> {
    let mut o = core::ptr::null_mut();
    let hr = slot::<unsafe extern "system" fn(*mut c_void, *const Guid, u32, *const c_void, *mut *mut c_void) -> i32>(dev.0, 3)(dev.0, iid, CLSCTX_ALL, core::ptr::null(), &mut o);
    if hr < 0 { Err(hr) } else { Ok(Com(o)) }
}

unsafe fn client(id: Option<&str>) -> Result<Com, i32> {
    let dev = device(id)?;
    activate(&dev, &IID_IAUDIO_CLIENT3).or_else(|_| activate(&dev, &IID_IAUDIO_CLIENT))
}

unsafe fn device_id_of(dev: &Com) -> Result<String, i32> {
    let mut id: *mut u16 = core::ptr::null_mut();
    let hr = slot::<unsafe extern "system" fn(*mut c_void, *mut *mut u16) -> i32>(dev.0, 5)(dev.0, &mut id);
    if hr < 0 || id.is_null() {
        return Err(hr);
    }
    Ok(take_wide(id))
}

pub fn device_id() -> Result<String, i32> {
    unsafe { device_id_of(&device(None)?) }
}

pub fn default_id(flow: u32, role: u32) -> Result<String, i32> {
    unsafe {
        let en = enumerator()?;
        let mut dev = core::ptr::null_mut();
        let hr = slot::<unsafe extern "system" fn(*mut c_void, u32, u32, *mut *mut c_void) -> i32>(en.0, 4)(en.0, flow, role, &mut dev);
        if hr < 0 {
            return Err(hr);
        }
        device_id_of(&Com(dev))
    }
}

pub fn device_state(id: Option<&str>) -> Result<u32, i32> {
    unsafe {
        let dev = device(id)?;
        let mut st = 0u32;
        let hr = slot::<unsafe extern "system" fn(*mut c_void, *mut u32) -> i32>(dev.0, 6)(dev.0, &mut st);
        if hr < 0 { Err(hr) } else { Ok(st) }
    }
}

pub fn endpoints(flow: u32, mask: u32) -> Result<Vec<(String, u32)>, i32> {
    unsafe {
        let en = enumerator()?;
        let mut coll = core::ptr::null_mut();
        let hr = slot::<unsafe extern "system" fn(*mut c_void, u32, u32, *mut *mut c_void) -> i32>(en.0, 3)(en.0, flow, mask, &mut coll);
        if hr < 0 {
            return Err(hr);
        }
        let coll = Com(coll);
        let mut n = 0u32;
        let hr = slot::<unsafe extern "system" fn(*mut c_void, *mut u32) -> i32>(coll.0, 3)(coll.0, &mut n);
        if hr < 0 {
            return Err(hr);
        }
        let mut out = Vec::new();
        for i in 0..n {
            let mut d = core::ptr::null_mut();
            if slot::<unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> i32>(coll.0, 4)(coll.0, i, &mut d) < 0 {
                continue;
            }
            let d = Com(d);
            let mut st = 0u32;
            slot::<unsafe extern "system" fn(*mut c_void, *mut u32) -> i32>(d.0, 6)(d.0, &mut st);
            if let Ok(id) = device_id_of(&d) {
                out.push((id, st));
            }
        }
        Ok(out)
    }
}

pub struct DeviceInfo {
    pub mix: Vec<u8>,
    pub default_period: i64,
    pub min_period: i64,
}

pub fn device_info_for(id: Option<&str>) -> Result<DeviceInfo, i32> {
    unsafe {
        let c = client(id)?;
        let mut f: *mut u8 = core::ptr::null_mut();
        let hr = slot::<unsafe extern "system" fn(*mut c_void, *mut *mut u8) -> i32>(c.0, 8)(c.0, &mut f);
        if hr < 0 {
            return Err(hr);
        }
        let mix = take_format(f);
        let (mut d, mut m) = (0i64, 0i64);
        let hr = slot::<unsafe extern "system" fn(*mut c_void, *mut i64, *mut i64) -> i32>(c.0, 9)(c.0, &mut d, &mut m);
        if hr < 0 {
            return Err(hr);
        }
        Ok(DeviceInfo { mix, default_period: d, min_period: m })
    }
}

pub fn device_info() -> Result<DeviceInfo, i32> {
    device_info_for(None)
}

pub fn is_format_supported_on(id: Option<&str>, mode: u32, fmt: &[u8]) -> (i32, Option<Vec<u8>>) {
    unsafe {
        let c = match client(id) {
            Ok(c) => c,
            Err(hr) => return (hr, None),
        };
        let mut closest: *mut u8 = core::ptr::null_mut();
        let want = if mode == SHARED { &mut closest as *mut *mut u8 } else { core::ptr::null_mut() };
        let hr = slot::<unsafe extern "system" fn(*mut c_void, u32, *const u8, *mut *mut u8) -> i32>(c.0, 7)(c.0, mode, fmt.as_ptr(), want);
        let near = if closest.is_null() { None } else { Some(take_format(closest)) };
        (hr, near)
    }
}

pub fn is_format_supported(fmt: &[u8]) -> i32 {
    is_format_supported_on(None, SHARED, fmt).0
}

pub fn offload_capable(id: Option<&str>, category: u32) -> Result<u32, i32> {
    unsafe {
        let c = client(id)?;
        let mut b = 0i32;
        let hr = slot::<unsafe extern "system" fn(*mut c_void, u32, *mut i32) -> i32>(c.0, 15)(c.0, category, &mut b);
        if hr < 0 { Err(hr) } else { Ok(b as u32) }
    }
}

pub fn buffer_size_limits(id: Option<&str>, fmt: &[u8], event: bool) -> Result<(i64, i64), i32> {
    unsafe {
        let c = client(id)?;
        let (mut lo, mut hi) = (0i64, 0i64);
        let hr = slot::<unsafe extern "system" fn(*mut c_void, *const u8, i32, *mut i64, *mut i64) -> i32>(c.0, 17)(c.0, fmt.as_ptr(), event as i32, &mut lo, &mut hi);
        if hr < 0 { Err(hr) } else { Ok((lo, hi)) }
    }
}

pub fn shared_mode_engine_period(id: Option<&str>, fmt: &[u8]) -> Result<[u32; 4], i32> {
    unsafe {
        let c = client(id)?;
        let mut v = [0u32; 4];
        let hr = slot::<unsafe extern "system" fn(*mut c_void, *const u8, *mut u32, *mut u32, *mut u32, *mut u32) -> i32>(c.0, 18)(c.0, fmt.as_ptr(), &mut v[0], &mut v[1], &mut v[2], &mut v[3]);
        if hr < 0 { Err(hr) } else { Ok(v) }
    }
}

pub fn current_shared_mode_engine_period(id: Option<&str>) -> Result<(Vec<u8>, u32), i32> {
    unsafe {
        let c = client(id)?;
        let mut f: *mut u8 = core::ptr::null_mut();
        let mut p = 0u32;
        let hr = slot::<unsafe extern "system" fn(*mut c_void, *mut *mut u8, *mut u32) -> i32>(c.0, 19)(c.0, &mut f, &mut p);
        if hr < 0 || f.is_null() {
            return Err(hr);
        }
        Ok((take_format(f), p))
    }
}

pub struct EndpointCaps {
    pub channels: u32,
    pub steps: u32,
    pub min_db: f32,
    pub max_db: f32,
    pub inc_db: f32,
}

pub fn endpoint_caps(id: Option<&str>) -> Result<EndpointCaps, i32> {
    unsafe {
        let dev = device(id)?;
        let v = activate(&dev, &IID_IAUDIO_ENDPOINT_VOLUME)?;
        let mut ch = 0u32;
        let hr = slot::<unsafe extern "system" fn(*mut c_void, *mut u32) -> i32>(v.0, 5)(v.0, &mut ch);
        if hr < 0 {
            return Err(hr);
        }
        let (mut step, mut count) = (0u32, 0u32);
        let hr = slot::<unsafe extern "system" fn(*mut c_void, *mut u32, *mut u32) -> i32>(v.0, 16)(v.0, &mut step, &mut count);
        if hr < 0 {
            return Err(hr);
        }
        let (mut lo, mut hi, mut inc) = (0f32, 0f32, 0f32);
        let hr = slot::<unsafe extern "system" fn(*mut c_void, *mut f32, *mut f32, *mut f32) -> i32>(v.0, 20)(v.0, &mut lo, &mut hi, &mut inc);
        if hr < 0 {
            return Err(hr);
        }
        Ok(EndpointCaps { channels: ch, steps: count, min_db: lo, max_db: hi, inc_db: inc })
    }
}

pub enum Prop {
    Empty,
    Text(Vec<u16>),
    Dword(u32),
    Serialized(u16, Vec<u8>),
}

unsafe fn property_store(id: Option<&str>) -> Result<Com, i32> {
    let dev = device(id)?;
    let mut ps = core::ptr::null_mut();
    let hr = slot::<unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> i32>(dev.0, 4)(dev.0, 0, &mut ps);
    if hr < 0 { Err(hr) } else { Ok(Com(ps)) }
}

pub fn property_count(id: Option<&str>) -> Result<u32, i32> {
    unsafe {
        let ps = property_store(id)?;
        let mut n = 0u32;
        let hr = slot::<unsafe extern "system" fn(*mut c_void, *mut u32) -> i32>(ps.0, 3)(ps.0, &mut n);
        if hr < 0 { Err(hr) } else { Ok(n) }
    }
}

pub fn property_at(id: Option<&str>, index: u32) -> Result<PropertyKey, i32> {
    unsafe {
        let ps = property_store(id)?;
        let mut k = PropertyKey::default();
        let hr = slot::<unsafe extern "system" fn(*mut c_void, u32, *mut PropertyKey) -> i32>(ps.0, 4)(ps.0, index, &mut k);
        if hr < 0 { Err(hr) } else { Ok(k) }
    }
}

pub fn scalar_size(vt: u16) -> Option<usize> {
    match vt {
        2 | 18 | 11 => Some(2),
        3 | 19 | 4 | 10 | 22 | 23 => Some(4),
        5 | 6 | 7 | 20 | 21 | 64 => Some(8),
        16 | 17 => Some(1),
        _ => None,
    }
}

pub fn property_value(id: Option<&str>, key: &PropertyKey) -> Result<Option<Prop>, i32> {
    unsafe {
        let ps = property_store(id)?;
        let mut v: PropVariant = core::mem::zeroed();
        let hr = slot::<unsafe extern "system" fn(*mut c_void, *const PropertyKey, *mut PropVariant) -> i32>(ps.0, 5)(ps.0, key, &mut v);
        if hr < 0 {
            return Err(hr);
        }
        let out = match v.vt {
            0 => Some(Prop::Empty),
            31 => {
                let p = v.a as *const u16;
                let mut n = 0;
                while !p.is_null() && *p.add(n) != 0 {
                    n += 1;
                }
                let mut w = if p.is_null() { Vec::new() } else { core::slice::from_raw_parts(p, n).to_vec() };
                w.push(0);
                Some(Prop::Text(w))
            }
            19 => Some(Prop::Dword(v.a as u32)),
            65 => {
                let cb = v.a as u32 as usize;
                let p = v.b as *const u8;
                Some(Prop::Serialized(65, if p.is_null() { Vec::new() } else { core::slice::from_raw_parts(p, cb).to_vec() }))
            }
            t => scalar_size(t).map(|n| Prop::Serialized(t, v.a.to_le_bytes()[..n].to_vec())),
        };
        PropVariantClear(&mut v);
        Ok(out)
    }
}

unsafe fn session_control(id: Option<&str>, session: &Guid) -> Result<Com, i32> {
    let dev = device(id)?;
    let mgr = activate(&dev, &IID_IAUDIO_SESSION_MANAGER)?;
    let mut ctl = core::ptr::null_mut();
    let hr = slot::<unsafe extern "system" fn(*mut c_void, *const Guid, u32, *mut *mut c_void) -> i32>(mgr.0, 3)(mgr.0, session, 0, &mut ctl);
    if hr < 0 {
        return Err(hr);
    }
    let ctl = Com(ctl);
    let mut c2 = core::ptr::null_mut();
    let hr = slot::<unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32>(ctl.0, 0)(ctl.0, &IID_IAUDIO_SESSION_CONTROL2, &mut c2);
    if hr < 0 { Err(hr) } else { Ok(Com(c2)) }
}

pub fn session_state(id: Option<&str>, session: &Guid) -> Result<u32, i32> {
    unsafe {
        let c = session_control(id, session)?;
        let mut st = 0u32;
        let hr = slot::<unsafe extern "system" fn(*mut c_void, *mut u32) -> i32>(c.0, 3)(c.0, &mut st);
        if hr < 0 { Err(hr) } else { Ok(st) }
    }
}

pub fn session_string(id: Option<&str>, session: &Guid, n: usize) -> Result<String, i32> {
    unsafe {
        let c = session_control(id, session)?;
        let mut p: *mut u16 = core::ptr::null_mut();
        let hr = slot::<unsafe extern "system" fn(*mut c_void, *mut *mut u16) -> i32>(c.0, n)(c.0, &mut p);
        if hr < 0 {
            return Err(hr);
        }
        Ok(take_wide(p))
    }
}

pub fn session_set_display_name(id: Option<&str>, session: &Guid, name: &[u16], ctx: &Guid) -> i32 {
    unsafe {
        match session_control(id, session) {
            Ok(c) => slot::<unsafe extern "system" fn(*mut c_void, *const u16, *const Guid) -> i32>(c.0, 5)(c.0, name.as_ptr(), ctx),
            Err(hr) => hr,
        }
    }
}

pub fn session_grouping(id: Option<&str>, session: &Guid) -> Result<Guid, i32> {
    unsafe {
        let c = session_control(id, session)?;
        let mut g = Guid { data1: 0, data2: 0, data3: 0, data4: [0; 8] };
        let hr = slot::<unsafe extern "system" fn(*mut c_void, *mut Guid) -> i32>(c.0, 8)(c.0, &mut g);
        if hr < 0 { Err(hr) } else { Ok(g) }
    }
}

pub fn session_set_grouping(id: Option<&str>, session: &Guid, g: &Guid, ctx: &Guid) -> i32 {
    unsafe {
        match session_control(id, session) {
            Ok(c) => slot::<unsafe extern "system" fn(*mut c_void, *const Guid, *const Guid) -> i32>(c.0, 9)(c.0, g, ctx),
            Err(hr) => hr,
        }
    }
}

pub fn session_is_system_sounds(id: Option<&str>, session: &Guid) -> i32 {
    unsafe {
        match session_control(id, session) {
            Ok(c) => slot::<unsafe extern "system" fn(*mut c_void) -> i32>(c.0, 15)(c.0),
            Err(hr) => hr,
        }
    }
}

pub fn session_set_ducking(id: Option<&str>, session: &Guid, opt_out: bool) -> i32 {
    unsafe {
        match session_control(id, session) {
            Ok(c) => slot::<unsafe extern "system" fn(*mut c_void, i32) -> i32>(c.0, 16)(c.0, opt_out as i32),
            Err(hr) => hr,
        }
    }
}

pub struct Spatial(Com);

pub fn spatial(id: Option<&str>) -> Result<Spatial, i32> {
    unsafe {
        let dev = device(id)?;
        Ok(Spatial(activate(&dev, &IID_ISPATIAL_AUDIO_CLIENT)?))
    }
}

impl Spatial {
    pub fn native_mask(&self) -> Result<u32, i32> {
        unsafe {
            let mut m = 0u32;
            let hr = slot::<unsafe extern "system" fn(*mut c_void, *mut u32) -> i32>(self.0 .0, 4)(self.0 .0, &mut m);
            if hr < 0 { Err(hr) } else { Ok(m) }
        }
    }
    pub fn max_dynamic(&self) -> Result<u32, i32> {
        unsafe {
            let mut m = 0u32;
            let hr = slot::<unsafe extern "system" fn(*mut c_void, *mut u32) -> i32>(self.0 .0, 5)(self.0 .0, &mut m);
            if hr < 0 { Err(hr) } else { Ok(m) }
        }
    }
    pub fn max_frames(&self, fmt: &[u8]) -> Result<u32, i32> {
        unsafe {
            let mut m = 0u32;
            let hr = slot::<unsafe extern "system" fn(*mut c_void, *const u8, *mut u32) -> i32>(self.0 .0, 7)(self.0 .0, fmt.as_ptr(), &mut m);
            if hr < 0 { Err(hr) } else { Ok(m) }
        }
    }
    pub fn format_supported(&self, fmt: &[u8]) -> i32 {
        unsafe { slot::<unsafe extern "system" fn(*mut c_void, *const u8) -> i32>(self.0 .0, 8)(self.0 .0, fmt.as_ptr()) }
    }
    pub fn stream_available(&self, iid: &Guid) -> i32 {
        unsafe { slot::<unsafe extern "system" fn(*mut c_void, *const Guid, *const c_void) -> i32>(self.0 .0, 9)(self.0 .0, iid, core::ptr::null()) }
    }
    pub fn offload_capable(&self, category: u32) -> Result<u32, i32> {
        unsafe {
            let mut c2 = core::ptr::null_mut();
            let hr = slot::<unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32>(self.0 .0, 0)(self.0 .0, &IID_ISPATIAL_AUDIO_CLIENT2, &mut c2);
            if hr < 0 {
                return Err(hr);
            }
            let c2 = Com(c2);
            let mut b = 0i32;
            let hr = slot::<unsafe extern "system" fn(*mut c_void, u32, *mut i32) -> i32>(c2.0, 11)(c2.0, category, &mut b);
            if hr < 0 { Err(hr) } else { Ok(b as u32) }
        }
    }
}

pub struct Ring {
    pub base: usize,
    pub start: u32,
    pub end: u32,
    pub align: u32,
}

impl Ring {
    pub fn write_pos(&self) -> &AtomicI64 {
        unsafe { &*((self.base + 0x10) as *const AtomicI64) }
    }
    pub fn read_pos(&self) -> &AtomicI64 {
        unsafe { &*((self.base + 0x18) as *const AtomicI64) }
    }
    pub fn bytes(&self) -> u32 {
        self.end - self.start
    }
}

#[derive(Default)]
pub struct Shared {
    pub running: AtomicBool,
    pub quit: AtomicBool,
    pub reset: AtomicBool,
    pub gain: Mutex<Option<f32>>,
    pub channels: Mutex<Option<Vec<f32>>>,
    pub capture: Mutex<Option<Vec<u8>>>,
    pub rendered: AtomicI64,
    pub signals: AtomicI64,
    pub signal: Mutex<Vec<isize>>,
    pub client: AtomicUsize,
    pub latency: AtomicI64,
    pub period: AtomicI64,
}

pub struct Renderer {
    pub shared: Arc<Shared>,
    pub thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Renderer {
    fn drop(&mut self) {
        self.shared.quit.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

pub fn muted() -> bool {
    std::env::var("TOMBOX_AUDIO_MUTE").map(|v| v == "1").unwrap_or(false)
}

pub fn capturing() -> bool {
    std::env::var("TOMBOX_AUDIO_CAPTURE").map(|v| v == "1").unwrap_or(false)
}

pub struct Open {
    pub device: Option<String>,
    pub format: Vec<u8>,
    pub duration: i64,
    pub session: Guid,
    pub properties: Option<[u32; 4]>,
}

pub fn open(o: Open, ring: Ring) -> Result<(Renderer, u32), i32> {
    let shared = Arc::new(Shared::default());
    if capturing() {
        *shared.capture.lock().unwrap() = Some(Vec::new());
    }
    let (tx, rx) = std::sync::mpsc::channel::<Result<u32, i32>>();
    let s2 = shared.clone();
    let t = std::thread::spawn(move || unsafe { run(o, ring, s2, tx) });
    match rx.recv() {
        Ok(Ok(frames)) => Ok((Renderer { shared, thread: Some(t) }, frames)),
        Ok(Err(hr)) => {
            let _ = t.join();
            Err(hr)
        }
        Err(_) => Err(-2147467259),
    }
}

impl Renderer {
    pub fn stream_latency(&self) -> i64 {
        self.shared.latency.load(Ordering::SeqCst)
    }
}

unsafe fn run(o: Open, ring: Ring, s: Arc<Shared>, tx: std::sync::mpsc::Sender<Result<u32, i32>>) {
    let c = match client(o.device.as_deref()) {
        Ok(c) => c,
        Err(hr) => {
            let _ = tx.send(Err(hr));
            return;
        }
    };
    if let Some(p) = o.properties {
        let props = [16u32, p[0], p[1], p[2]];
        slot::<unsafe extern "system" fn(*mut c_void, *const u32) -> i32>(c.0, 16)(c.0, props.as_ptr());
    }
    let hr = slot::<unsafe extern "system" fn(*mut c_void, u32, u32, i64, i64, *const u8, *const Guid) -> i32>(c.0, 3)(c.0, SHARED, AUTOCONVERT, o.duration, 0, o.format.as_ptr(), &o.session);
    if hr < 0 {
        let _ = tx.send(Err(hr));
        return;
    }
    let mut frames = 0u32;
    slot::<unsafe extern "system" fn(*mut c_void, *mut u32) -> i32>(c.0, 4)(c.0, &mut frames);
    let mut lat = 0i64;
    slot::<unsafe extern "system" fn(*mut c_void, *mut i64) -> i32>(c.0, 5)(c.0, &mut lat);
    s.latency.store(lat, Ordering::SeqCst);
    let (mut dp, mut mp) = (0i64, 0i64);
    slot::<unsafe extern "system" fn(*mut c_void, *mut i64, *mut i64) -> i32>(c.0, 9)(c.0, &mut dp, &mut mp);
    s.period.store(dp.max(10_000), Ordering::SeqCst);
    let mut rc = core::ptr::null_mut();
    let hr = slot::<unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32>(c.0, 14)(c.0, &IID_IAUDIO_RENDER_CLIENT, &mut rc);
    if hr < 0 {
        let _ = tx.send(Err(hr));
        return;
    }
    let rc = Com(rc);
    let mut vol = core::ptr::null_mut();
    slot::<unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32>(c.0, 14)(c.0, &IID_ISIMPLE_AUDIO_VOLUME, &mut vol);
    let vol = Com(vol);
    let mut sv = core::ptr::null_mut();
    slot::<unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32>(c.0, 14)(c.0, &IID_IAUDIO_STREAM_VOLUME, &mut sv);
    let sv = Com(sv);
    if !vol.0.is_null() && muted() {
        slot::<unsafe extern "system" fn(*mut c_void, i32, *const Guid) -> i32>(vol.0, 5)(vol.0, 1, core::ptr::null());
    }
    s.client.store(c.0 as usize, Ordering::SeqCst);
    let _ = tx.send(Ok(frames));
    let align = ring.align.max(1) as i64;
    let rate = sample_rate(&o.format).max(1) as i64;
    let tick = ((frames as i64 * 1000 / rate) / 4).clamp(1, 10) as u32;
    let period = std::time::Duration::from_nanos(s.period.load(Ordering::SeqCst) as u64 * 100);
    let mut next_signal = std::time::Instant::now();
    let mut playing = false;
    let mut applied_gain: Option<f32> = None;
    let mut applied_channels: Option<Vec<f32>> = None;
    while !s.quit.load(Ordering::SeqCst) {
        let want = s.running.load(Ordering::SeqCst);
        if want != playing {
            slot::<unsafe extern "system" fn(*mut c_void) -> i32>(c.0, if want { 10 } else { 11 })(c.0);
            playing = want;
            next_signal = std::time::Instant::now();
        }
        if !playing && s.reset.swap(false, Ordering::SeqCst) {
            slot::<unsafe extern "system" fn(*mut c_void) -> i32>(c.0, 12)(c.0);
        }
        let g = *s.gain.lock().unwrap();
        if g != applied_gain && !vol.0.is_null() {
            if let Some(v) = g {
                slot::<unsafe extern "system" fn(*mut c_void, f32, *const Guid) -> i32>(vol.0, 3)(vol.0, v.clamp(0.0, 1.0), core::ptr::null());
            }
            applied_gain = g;
        }
        let ch = s.channels.lock().unwrap().clone();
        if ch != applied_channels && !sv.0.is_null() {
            if let Some(v) = &ch {
                slot::<unsafe extern "system" fn(*mut c_void, u32, *const f32) -> i32>(sv.0, 6)(sv.0, v.len() as u32, v.as_ptr());
            }
            applied_channels = ch;
        }
        if playing {
            pump(c.0, rc.0, frames, &ring, align, &s);
            let now = std::time::Instant::now();
            if now >= next_signal {
                for &e in s.signal.lock().unwrap().iter() {
                    SetEvent(e as usize);
                }
                s.signals.fetch_add(1, Ordering::SeqCst);
                next_signal = (next_signal + period).max(now);
            }
        }
        Sleep(tick);
    }
    s.client.store(0, Ordering::SeqCst);
    if playing {
        slot::<unsafe extern "system" fn(*mut c_void) -> i32>(c.0, 11)(c.0);
    }
}

unsafe fn pump(c: *mut c_void, rc: *mut c_void, frames: u32, ring: &Ring, align: i64, s: &Shared) {
    let mut pad = 0u32;
    if slot::<unsafe extern "system" fn(*mut c_void, *mut u32) -> i32>(c, 6)(c, &mut pad) < 0 {
        return;
    }
    let w = ring.write_pos().load(Ordering::Acquire);
    let r = ring.read_pos().load(Ordering::Acquire);
    if w < r || r < 0 {
        return;
    }
    let avail = (w - r) / align;
    let n = avail.min(frames.saturating_sub(pad) as i64);
    if n <= 0 {
        return;
    }
    let mut dst: *mut u8 = core::ptr::null_mut();
    if slot::<unsafe extern "system" fn(*mut c_void, u32, *mut *mut u8) -> i32>(rc, 3)(rc, n as u32, &mut dst) < 0 || dst.is_null() {
        return;
    }
    let total = (n * align) as usize;
    let size = ring.bytes() as usize;
    let mut done = 0usize;
    while done < total {
        let off = (r as usize + done) % size;
        let chunk = (size - off).min(total - done);
        core::ptr::copy_nonoverlapping((ring.base + ring.start as usize + off) as *const u8, dst.add(done), chunk);
        done += chunk;
    }
    if let Some(cap) = s.capture.lock().unwrap().as_mut() {
        cap.extend_from_slice(core::slice::from_raw_parts(dst, total));
    }
    slot::<unsafe extern "system" fn(*mut c_void, u32, u32) -> i32>(rc, 4)(rc, n as u32, 0);
    ring.read_pos().store(r + total as i64, Ordering::Release);
    s.rendered.fetch_add(total as i64, Ordering::SeqCst);
}
