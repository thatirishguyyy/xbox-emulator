use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};

use crate::gcdcom::Guid;
use crate::wasapi;

pub const ENVELOPE: usize = 0x810;
pub const PAYLOAD: usize = 0x10;
pub const CREATE_OBJECT: u32 = 0x01;
pub const GET_MIX_FORMAT: u32 = 0x02;
pub const GET_DEVICE_PERIOD: u32 = 0x03;
pub const INITIALIZE: u32 = 0x04;
pub const IS_FORMAT_SUPPORTED: u32 = 0x05;
pub const GET_STREAM_LATENCY: u32 = 0x06;
pub const START: u32 = 0x07;
pub const STOP: u32 = 0x08;
pub const RESET: u32 = 0x09;
pub const GET_BUFFER_SIZE_LIMITS: u32 = 0x0a;
pub const IS_OFFLOAD_CAPABLE: u32 = 0x0b;
pub const SET_CLIENT_PROPERTIES: u32 = 0x0c;
pub const GET_SHARED_MODE_ENGINE_PERIOD: u32 = 0x0d;
pub const GET_CURRENT_SHARED_MODE_ENGINE_PERIOD: u32 = 0x0e;
pub const STREAM_GET_CHANNEL_COUNT: u32 = 0x10;
pub const STREAM_SET_CHANNEL_VOLUME: u32 = 0x11;
pub const STREAM_GET_CHANNEL_VOLUME: u32 = 0x12;
pub const STREAM_SET_ALL_VOLUMES: u32 = 0x13;
pub const STREAM_GET_ALL_VOLUMES: u32 = 0x14;
pub const GET_SIMPLE_VOLUME: u32 = 0x15;
pub const SET_SIMPLE_VOLUME: u32 = 0x16;
pub const GET_SIMPLE_MUTE: u32 = 0x17;
pub const SET_SIMPLE_MUTE: u32 = 0x18;
pub const SESSION_GET_STATE: u32 = 0x19;
pub const SESSION_GET_DISPLAY_NAME: u32 = 0x1a;
pub const SESSION_SET_DISPLAY_NAME: u32 = 0x1b;
pub const SESSION_GET_GROUPING: u32 = 0x1c;
pub const SESSION_SET_GROUPING: u32 = 0x1d;
pub const SESSION_REGISTER_NOTIFICATION: u32 = 0x1e;
pub const ENUMERATOR_RESET: u32 = 0x23;
pub const DEVICE_GET_ID: u32 = 0x27;
pub const DEVICE_GET_STATE: u32 = 0x28;
pub const PROPERTY_GET_COUNT: u32 = 0x2b;
pub const PROPERTY_GET_AT: u32 = 0x2c;
pub const PROPERTY_GET_VALUE: u32 = 0x2d;
pub const RELEASE_OBJECT: u32 = 0x2e;
pub const SPATIAL_NATIVE_MASK: u32 = 0x31;
pub const SPATIAL_MAX_DYNAMIC: u32 = 0x32;
pub const SPATIAL_MAX_FRAMES: u32 = 0x33;
pub const SPATIAL_FORMAT_SUPPORTED: u32 = 0x34;
pub const SPATIAL_STREAM_AVAILABLE: u32 = 0x35;
pub const ENDPOINT_REGISTER: u32 = 0x4f;
pub const ENDPOINT_UNREGISTER: u32 = 0x50;
pub const ENDPOINT_CHANNEL_COUNT: u32 = 0x51;
pub const SET_ENDPOINT_VOLUME_LEVEL: u32 = 0x52;
pub const SET_ENDPOINT_VOLUME_SCALAR: u32 = 0x53;
pub const GET_ENDPOINT_VOLUME_LEVEL: u32 = 0x54;
pub const GET_ENDPOINT_VOLUME_SCALAR: u32 = 0x55;
pub const SET_ENDPOINT_CHANNEL_LEVEL: u32 = 0x56;
pub const SET_ENDPOINT_CHANNEL_SCALAR: u32 = 0x57;
pub const GET_ENDPOINT_CHANNEL_LEVEL: u32 = 0x58;
pub const GET_ENDPOINT_CHANNEL_SCALAR: u32 = 0x59;
pub const SET_ENDPOINT_MUTE: u32 = 0x5a;
pub const GET_ENDPOINT_MUTE: u32 = 0x5b;
pub const ENDPOINT_STEP_INFO: u32 = 0x5c;
pub const ENDPOINT_STEP_UP: u32 = 0x5d;
pub const ENDPOINT_STEP_DOWN: u32 = 0x5e;
pub const ENDPOINT_HARDWARE_SUPPORT: u32 = 0x5f;
pub const ENDPOINT_VOLUME_RANGE: u32 = 0x60;
pub const ENDPOINT_LIST: u32 = 0x70;
pub const DEFAULT_ENDPOINT_TABLE: u32 = 0x71;
pub const SESSION_CHANNEL_COUNT: u32 = 0x72;
pub const SESSION_SET_CHANNEL_VOLUME: u32 = 0x73;
pub const SESSION_GET_CHANNEL_VOLUME: u32 = 0x74;
pub const SESSION_SET_ALL_VOLUMES: u32 = 0x75;
pub const SESSION_GET_ALL_VOLUMES: u32 = 0x76;
pub const SESSION_MANAGER_REGISTER: u32 = 0x79;
pub const SESSION_MANAGER_UNREGISTER: u32 = 0x7a;
pub const SESSION_GET_IDENTIFIER: u32 = 0x7c;
pub const SESSION_GET_INSTANCE_IDENTIFIER: u32 = 0x7d;
pub const SESSION_IS_SYSTEM_SOUNDS: u32 = 0x7e;
pub const SESSION_SET_DUCKING: u32 = 0x7f;
pub const SPATIAL_OFFLOAD_CAPABLE: u32 = 0x85;

pub const METHOD_DEFAULT_ENDPOINT_19041: u32 = 0x22;
pub const METHOD_ENDPOINT_19041: u32 = 0x25;
pub const METHOD_DEFAULT_ENDPOINT: u32 = 0x80;
pub const METHOD_ENDPOINT: u32 = 0x81;
pub const BLOB_ID: usize = 0x1c;
pub const BLOB_ID_CHARS: usize = 0x37;

pub const DEVICE_ID_CHARS: usize = 0x82;
pub const DEVICE_ID_COUNT: usize = 0x104;
pub const EVENT_KIND_GAMECORE: u32 = 2;
pub const INIT_STRUCT: usize = 0x534;
pub const MAX_OBJECT_TYPE_19041: u32 = 0x17;
pub const MAX_OBJECT_TYPE: u32 = 0x1a;
pub const MAX_VOLUMES: usize = 0x32;
pub const LIST_PAGE: usize = 8;
pub const LIST_ENTRY: usize = 0x74;
pub const TABLE_ENTRY: usize = 0x78;
pub const TABLE_SLOTS: u32 = 9;
pub const PC_ROLES: u32 = 3;
pub const SESSION_STRING_CHARS: usize = 0x200;
pub const DISPLAY_NAME_CHARS: usize = 0x3e8;
pub const PROP_DATA: usize = 0x104;

pub const CB_MAGIC: u32 = 0x4550_4344;
pub const CB_WRITE: usize = 0x10;
pub const CB_READ: usize = 0x18;
pub const CB_FLAGS: usize = 0xa4;
pub const CB_META: usize = 0xb4;
pub const CB_MAGIC_AT: usize = 0xc0;
pub const CB_CONFIG: usize = 0xc8;
pub const FLAG_POSITION_IS_READ: u32 = 4;
pub const CFG_FIXED: u32 = 0xc8;
pub const CFG_ENTRIES: usize = 0x84;
pub const CFG_DURATION: usize = 0x88;
pub const CFG_LOOPBACK: usize = 0x98;
pub const CFG_QUEUE: usize = 0x9c;
pub const CFG_DATA_START: usize = 0xa0;
pub const CFG_DATA_END: usize = 0xa4;
pub const CFG_TOTAL: usize = 0xa8;
pub const CFG_FORMAT: usize = 0xb4;
pub const QUEUE_ENTRY: u32 = 0x40;
pub const MIN_QUEUE_ENTRIES: u32 = 0x19;

pub const INIT_SHARE_MODE: usize = 0;
pub const INIT_FORMAT: usize = 0x18;
pub const INIT_SESSION: usize = 0x60;
pub const INIT_ENDPOINT_CLSID: usize = 0x70;
pub const INIT_EVENT1: usize = 0x94;
pub const INIT_EVENT2: usize = 0x1b8;
pub const INIT_EVENT3: usize = 0x2dc;
pub const INIT_MEMORY: usize = 0x404;
pub const KIND_NONE: u32 = 0;
pub const KIND_CROSS_VM_EVENT: u32 = 2;
pub const KIND_CROSS_VM_MEMORY: u32 = 3;

pub const CLSID_CLIENT_OUTPUT_ENDPOINT: Guid = Guid { data1: 0xCD773740, data2: 0xB187, data3: 0x4974, data4: [0xA1, 0xD5, 0xE0, 0xFF, 0x91, 0x37, 0x22, 0x77] };
pub const CLSID_XVM_EVENT: Guid = Guid { data1: 0x148EC3BF, data2: 0x349C, data3: 0x4EAC, data4: [0xB3, 0x73, 0x9F, 0xD5, 0x7C, 0xB4, 0x6D, 0x88] };
pub const GUID_XVM_EVENT: Guid = Guid { data1: 0xA963C7E3, data2: 0x8A0A, data3: 0x45FA, data4: [0xBD, 0x23, 0xAA, 0x51, 0xE4, 0xCF, 0x85, 0x1E] };
pub const CLSID_XVM_BUFFER: Guid = Guid { data1: 0xE782CED1, data2: 0x3B5D, data3: 0x43AA, data4: [0xA4, 0x5D, 0x82, 0x85, 0x2B, 0x57, 0xD8, 0x4C] };
pub const GUID_XVM_BUFFER: Guid = Guid { data1: 0x00AA2A8A, data2: 0xACB6, data3: 0x4F3D, data4: [0x9A, 0x7E, 0xBD, 0xE7, 0x12, 0x7B, 0x43, 0x87] };
const GUID_NULL: Guid = Guid { data1: 0, data2: 0, data3: 0, data4: [0; 8] };

const S_OK: i32 = 0;
const S_FALSE: i32 = 1;
const E_NOTIMPL: i32 = 0x8000_4001u32 as i32;
const E_INVALIDARG: i32 = 0x8007_0057u32 as i32;
const E_OUTOFMEMORY: i32 = 0x8007_000Eu32 as i32;
const AUDCLNT_E_NOT_INITIALIZED: i32 = 0x8889_0001u32 as i32;
const AUDCLNT_E_ALREADY_INITIALIZED: i32 = 0x8889_0002u32 as i32;
const AUDCLNT_E_NOT_STOPPED: i32 = 0x8889_0005u32 as i32;
const HNS: i64 = 10_000_000;

static HELLO: AtomicU32 = AtomicU32::new(0);

pub fn set_hello(generation: u16) {
    HELLO.store(generation as u32, Ordering::SeqCst);
}

pub fn hello_seen() -> u32 {
    HELLO.load(Ordering::SeqCst)
}

pub fn generation() -> u32 {
    match HELLO.load(Ordering::SeqCst) {
        0 => match compat_common::title_os::title_os_build() {
            Some(b) if b < 22000 => 1,
            Some(_) => 2,
            None => 1,
        },
        g => g,
    }
}

pub fn max_object_type() -> u32 {
    if generation() >= 2 { MAX_OBJECT_TYPE } else { MAX_OBJECT_TYPE_19041 }
}

pub struct Stream {
    pub pool_index: u16,
    pub base: usize,
    pub format: Vec<u8>,
    pub frames: u32,
    pub event: isize,
    pub event_name: String,
    pub renderer: wasapi::Renderer,
}

struct Object {
    kind: u32,
    parent: u32,
    device: Option<String>,
    stream: Option<Stream>,
    gain: f32,
    mute: bool,
    volumes: Option<Vec<f32>>,
    properties: Option<[u32; 4]>,
    notify: bool,
    list_cursor: usize,
}

impl Object {
    fn new(kind: u32, parent: u32, device: Option<String>) -> Self {
        Object { kind, parent, device, stream: None, gain: 1.0, mute: false, volumes: None, properties: None, notify: false, list_cursor: 0 }
    }
}

struct Endpoint {
    scalar: f32,
    mute: bool,
    channels: Vec<f32>,
}

#[derive(Default)]
struct State {
    objects: HashMap<u32, Object>,
    endpoints: HashMap<String, Endpoint>,
    next: u32,
    next_event: u32,
    session: Option<Guid>,
    session_channels: Option<Vec<f32>>,
}

fn state() -> MutexGuard<'static, State> {
    static S: OnceLock<Mutex<State>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(State { next: 1, ..Default::default() })).lock().unwrap_or_else(|e| e.into_inner())
}

fn guid_bytes(g: &Guid) -> [u8; 16] {
    let mut b = [0u8; 16];
    b[0..4].copy_from_slice(&g.data1.to_le_bytes());
    b[4..6].copy_from_slice(&g.data2.to_le_bytes());
    b[6..8].copy_from_slice(&g.data3.to_le_bytes());
    b[8..16].copy_from_slice(&g.data4);
    b
}

fn guid_from(b: &[u8]) -> Guid {
    Guid { data1: u32::from_le_bytes(b[0..4].try_into().unwrap()), data2: u16::from_le_bytes([b[4], b[5]]), data3: u16::from_le_bytes([b[6], b[7]]), data4: b[8..16].try_into().unwrap() }
}

fn align_up(v: u32, a: u32) -> u32 {
    (v + a - 1) & !(a - 1)
}

pub fn event_name(index: u16) -> String {
    format!("{}{index}", crate::vab::GAMECORE_BASE)
}

pub struct Layout {
    pub frames: u32,
    pub bytes: u32,
    pub entries: u32,
    pub queue: u32,
    pub start: u32,
    pub end: u32,
    pub total: u32,
}

pub fn layout(format: &[u8], frames: u32, period_frames: u32) -> Layout {
    let cb = (wasapi::format_len(format) - 0x12) as u32;
    let bytes = frames * wasapi::block_align(format);
    let entries = (frames / period_frames.max(1) + 3).max(MIN_QUEUE_ENTRIES);
    let size = CFG_FIXED + cb;
    let queue = align_up(CB_CONFIG as u32 + size, 0x80);
    let start = align_up(queue + entries * QUEUE_ENTRY, 0x400);
    let end = start + bytes;
    Layout { frames, bytes, entries, queue, start, end, total: end }
}

pub unsafe fn write_control_block(base: usize, format: &[u8], duration: i64, l: &Layout) {
    let p = base as *mut u8;
    core::ptr::write_bytes(p, 0, l.start as usize);
    let w32 = |o: usize, v: u32| (p.add(o) as *mut u32).write_unaligned(v);
    w32(CB_FLAGS, FLAG_POSITION_IS_READ);
    w32(CB_META, u32::MAX);
    w32(CB_MAGIC_AT, CB_MAGIC);
    let c = CB_CONFIG;
    let flen = wasapi::format_len(format);
    w32(c, CFG_FIXED + (flen as u32 - 0x12));
    w32(c + CFG_ENTRIES, l.entries);
    (p.add(c + CFG_DURATION) as *mut i64).write_unaligned(duration);
    w32(c + CFG_LOOPBACK, u32::MAX);
    w32(c + CFG_QUEUE, l.queue);
    w32(c + CFG_DATA_START, l.start);
    w32(c + CFG_DATA_END, l.end);
    w32(c + CFG_TOTAL, l.total);
    core::ptr::copy_nonoverlapping(format.as_ptr(), p.add(c + CFG_FORMAT), flen);
}

unsafe fn rd<T: Copy>(p: *const u8, o: usize) -> T {
    (p.add(o) as *const T).read_unaligned()
}

unsafe fn wr<T: Copy>(p: *mut u8, o: usize, v: T) {
    (p.add(o) as *mut T).write_unaligned(v)
}

unsafe fn wide_at(p: *const u8, chars: usize) -> Option<String> {
    let s = core::slice::from_raw_parts(p as *const u16, chars);
    let n = s.iter().position(|&c| c == 0)?;
    Some(String::from_utf16_lossy(&s[..n]))
}

unsafe fn put_wide(p: *mut u8, s: &str, chars: usize) -> bool {
    let w: Vec<u16> = s.encode_utf16().collect();
    if w.len() >= chars {
        return false;
    }
    core::ptr::write_bytes(p, 0, chars * 2);
    core::ptr::copy_nonoverlapping(w.as_ptr() as *const u8, p, w.len() * 2);
    true
}

pub unsafe fn call(cb: i32, env: *mut u8) -> i32 {
    if env.is_null() || (cb as usize) < PAYLOAD {
        return E_INVALIDARG;
    }
    let id: u32 = rd(env, 8);
    let object: u32 = rd(env, 0xc);
    let a = env.add(PAYLOAD);
    let room = cb as usize - PAYLOAD;
    let hr = dispatch(id, object, a, room);
    wr(env, 4, hr);
    wr(env, 8, id);
    compat_common::record_raw("crossvm-audio", "call", &format!("id {id:#x} object {object:#x} -> {:#010x}", hr as u32));
    S_OK
}

fn device_of(s: &State, mut object: u32) -> Option<String> {
    for _ in 0..16 {
        let o = s.objects.get(&object)?;
        if o.device.is_some() {
            return o.device.clone();
        }
        if o.parent == 0 || o.parent == object {
            return None;
        }
        object = o.parent;
    }
    None
}

fn stream_owner(s: &State, mut object: u32) -> Option<u32> {
    for _ in 0..16 {
        let o = s.objects.get(&object)?;
        if o.stream.is_some() {
            return Some(object);
        }
        if o.parent == 0 || o.parent == object {
            return None;
        }
        object = o.parent;
    }
    None
}

fn device_for(object: u32) -> Option<String> {
    device_of(&state(), object)
}

fn session_guid() -> Guid {
    state().session.unwrap_or(GUID_NULL)
}

fn endpoint_key(dev: &Option<String>) -> String {
    dev.clone().or_else(|| wasapi::device_id().ok()).unwrap_or_default()
}

fn endpoint_channels(dev: &Option<String>) -> u32 {
    wasapi::endpoint_caps(dev.as_deref()).map(|c| c.channels).unwrap_or(2).max(1)
}

fn with_endpoint<R>(dev: &Option<String>, f: impl FnOnce(&mut Endpoint) -> R) -> R {
    let n = endpoint_channels(dev) as usize;
    let key = endpoint_key(dev);
    let mut s = state();
    let e = s.endpoints.entry(key).or_insert_with(|| Endpoint { scalar: 1.0, mute: false, channels: vec![1.0; n] });
    let r = f(e);
    apply_all(&mut s);
    r
}

fn apply_all(s: &mut State) {
    let keys: Vec<u32> = s.objects.iter().filter(|(_, o)| o.stream.is_some()).map(|(&k, _)| k).collect();
    for k in keys {
        let dev = device_of(s, k);
        let key = endpoint_key(&dev);
        let (es, em, ech) = s.endpoints.get(&key).map(|e| (e.scalar, e.mute, e.channels.clone())).unwrap_or((1.0, false, Vec::new()));
        let o = s.objects.get(&k).unwrap();
        let st = o.stream.as_ref().unwrap();
        let gain = if o.mute || em { 0.0 } else { o.gain * es };
        *st.renderer.shared.gain.lock().unwrap() = Some(gain.clamp(0.0, 1.0));
        let n = wasapi::channels(&st.format) as usize;
        let mut v = o.volumes.clone().unwrap_or_else(|| vec![1.0; n]);
        for (i, c) in v.iter_mut().enumerate() {
            *c *= ech.get(i).copied().unwrap_or(1.0) * s.session_channels.as_ref().and_then(|sc| sc.get(i).copied()).unwrap_or(1.0);
        }
        *st.renderer.shared.channels.lock().unwrap() = Some(v);
    }
}

fn level_to_scalar(db: f32, caps: &wasapi::EndpointCaps) -> f32 {
    if db <= caps.min_db {
        0.0
    } else {
        10f32.powf(db.min(caps.max_db).min(0.0) / 20.0)
    }
}

fn scalar_to_level(v: f32, caps: &wasapi::EndpointCaps) -> f32 {
    if v <= 0.0 {
        caps.min_db
    } else {
        (20.0 * v.log10()).clamp(caps.min_db, caps.max_db)
    }
}

unsafe fn dispatch(id: u32, object: u32, a: *mut u8, room: usize) -> i32 {
    match id {
        CREATE_OBJECT => create_object(a, room),
        RELEASE_OBJECT => {
            let o = state().objects.remove(&object);
            if let Some(Object { stream: Some(st), .. }) = o {
                drop(st.renderer);
                crate::logan::pool_release(st.pool_index as usize);
            }
            S_OK
        }
        GET_MIX_FORMAT => match wasapi::device_info_for(device_for(object).as_deref()) {
            Ok(d) => {
                core::ptr::write_bytes(a, 0, wasapi::MAX_FORMAT.min(room));
                core::ptr::copy_nonoverlapping(d.mix.as_ptr(), a, d.mix.len());
                S_OK
            }
            Err(hr) => hr,
        },
        GET_DEVICE_PERIOD => match wasapi::device_info_for(device_for(object).as_deref()) {
            Ok(d) => {
                wr(a, 2, d.default_period);
                wr(a, 0xa, d.min_period);
                S_OK
            }
            Err(hr) => hr,
        },
        GET_STREAM_LATENCY => {
            let s = state();
            match stream_owner(&s, object).and_then(|k| s.objects.get(&k)).and_then(|o| o.stream.as_ref()) {
                Some(st) => {
                    wr(a, 0, st.renderer.stream_latency());
                    S_OK
                }
                None => AUDCLNT_E_NOT_INITIALIZED,
            }
        }
        IS_FORMAT_SUPPORTED => is_format_supported(object, a),
        INITIALIZE => initialize(object, a, room),
        START | STOP | RESET => transport(id, object),
        GET_BUFFER_SIZE_LIMITS => {
            let f = format_at(a);
            let event: u8 = rd(a, wasapi::MAX_FORMAT);
            match wasapi::buffer_size_limits(device_for(object).as_deref(), &f, event != 0) {
                Ok((lo, hi)) => {
                    wr(a, 0x48, hi);
                    wr(a, 0x50, lo);
                    S_OK
                }
                Err(hr) => hr,
            }
        }
        IS_OFFLOAD_CAPABLE => {
            let cat: u32 = rd(a, 0);
            match wasapi::offload_capable(device_for(object).as_deref(), cat) {
                Ok(b) => {
                    wr(a, 0, b);
                    S_OK
                }
                Err(hr) => hr,
            }
        }
        SET_CLIENT_PROPERTIES => {
            let offload: u8 = rd(a, 0);
            let cat: u32 = rd(a, 1);
            let opts: u32 = rd(a, 5);
            let mut s = state();
            match s.objects.get_mut(&object) {
                Some(o) if o.stream.is_some() => AUDCLNT_E_ALREADY_INITIALIZED,
                Some(o) => {
                    o.properties = Some([offload as u32, cat, opts, 0]);
                    S_OK
                }
                None => E_INVALIDARG,
            }
        }
        GET_SHARED_MODE_ENGINE_PERIOD => {
            let f = format_at(a);
            match wasapi::shared_mode_engine_period(device_for(object).as_deref(), &f) {
                Ok(v) => {
                    for (i, x) in v.iter().enumerate() {
                        wr(a, 0x48 + 4 * i, *x);
                    }
                    S_OK
                }
                Err(hr) => hr,
            }
        }
        GET_CURRENT_SHARED_MODE_ENGINE_PERIOD => match wasapi::current_shared_mode_engine_period(device_for(object).as_deref()) {
            Ok((f, p)) => {
                core::ptr::write_bytes(a, 0, wasapi::MAX_FORMAT);
                core::ptr::copy_nonoverlapping(f.as_ptr(), a, f.len());
                wr(a, 0x48, p);
                S_OK
            }
            Err(hr) => hr,
        },
        STREAM_GET_CHANNEL_COUNT | STREAM_SET_CHANNEL_VOLUME | STREAM_GET_CHANNEL_VOLUME | STREAM_SET_ALL_VOLUMES | STREAM_GET_ALL_VOLUMES => stream_volume(id, object, a),
        SET_SIMPLE_VOLUME => {
            let v: f32 = rd(a, 0);
            if !(0.0..=1.0).contains(&v) {
                return E_INVALIDARG;
            }
            simple(object, |o| o.gain = v).map(|_| S_OK).unwrap_or(E_INVALIDARG)
        }
        SET_SIMPLE_MUTE => {
            let m: u8 = rd(a, 0);
            simple(object, |o| o.mute = m != 0).map(|_| S_OK).unwrap_or(E_INVALIDARG)
        }
        GET_SIMPLE_VOLUME => match simple(object, |o| o.gain) {
            Some(g) => {
                wr(a, 0, g);
                S_OK
            }
            None => E_INVALIDARG,
        },
        GET_SIMPLE_MUTE => match simple(object, |o| o.mute) {
            Some(m) => {
                wr(a, 0, m as u8);
                S_OK
            }
            None => E_INVALIDARG,
        },
        SESSION_GET_STATE => match wasapi::session_state(device_for(object).as_deref(), &session_guid()) {
            Ok(st) => {
                wr(a, 0, st);
                S_OK
            }
            Err(hr) => hr,
        },
        SESSION_GET_DISPLAY_NAME | SESSION_GET_IDENTIFIER | SESSION_GET_INSTANCE_IDENTIFIER => {
            let (slot, chars) = match id {
                SESSION_GET_DISPLAY_NAME => (4, DISPLAY_NAME_CHARS),
                SESSION_GET_IDENTIFIER => (12, SESSION_STRING_CHARS),
                _ => (13, SESSION_STRING_CHARS),
            };
            match wasapi::session_string(device_for(object).as_deref(), &session_guid(), slot) {
                Ok(t) => {
                    if put_wide(a, &t, chars) { S_OK } else { E_OUTOFMEMORY }
                }
                Err(hr) => hr,
            }
        }
        SESSION_SET_DISPLAY_NAME => {
            let Some(name) = wide_at(a, DISPLAY_NAME_CHARS) else { return E_INVALIDARG };
            let ctx = guid_from(core::slice::from_raw_parts(a.add(DISPLAY_NAME_CHARS * 2), 16));
            let w: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
            wasapi::session_set_display_name(device_for(object).as_deref(), &session_guid(), &w, &ctx)
        }
        SESSION_GET_GROUPING => match wasapi::session_grouping(device_for(object).as_deref(), &session_guid()) {
            Ok(g) => {
                core::ptr::copy_nonoverlapping(guid_bytes(&g).as_ptr(), a, 16);
                S_OK
            }
            Err(hr) => hr,
        },
        SESSION_SET_GROUPING => {
            let g = guid_from(core::slice::from_raw_parts(a, 16));
            let ctx = guid_from(core::slice::from_raw_parts(a.add(16), 16));
            wasapi::session_set_grouping(device_for(object).as_deref(), &session_guid(), &g, &ctx)
        }
        SESSION_IS_SYSTEM_SOUNDS => {
            let hr = wasapi::session_is_system_sounds(device_for(object).as_deref(), &session_guid());
            if hr < 0 {
                return hr;
            }
            wr(a, 0, hr as u32);
            S_OK
        }
        SESSION_SET_DUCKING => {
            let b: u32 = rd(a, 0);
            wasapi::session_set_ducking(device_for(object).as_deref(), &session_guid(), b != 0)
        }
        SESSION_REGISTER_NOTIFICATION | SESSION_MANAGER_REGISTER | ENDPOINT_REGISTER => register(object, true),
        SESSION_MANAGER_UNREGISTER | ENDPOINT_UNREGISTER => register(object, false),
        ENUMERATOR_RESET => register(object, false),
        DEVICE_GET_ID => match device_for(object).map(Ok).unwrap_or_else(wasapi::device_id) {
            Ok(id) => {
                let w: Vec<u16> = id.encode_utf16().take(DEVICE_ID_CHARS - 1).chain(Some(0)).collect();
                core::ptr::write_bytes(a, 0, DEVICE_ID_COUNT + 8);
                core::ptr::copy_nonoverlapping(w.as_ptr() as *const u8, a, w.len() * 2);
                wr(a, DEVICE_ID_COUNT, (w.len() * 2) as u64);
                S_OK
            }
            Err(hr) => hr,
        },
        DEVICE_GET_STATE => match wasapi::device_state(device_for(object).as_deref()) {
            Ok(st) => {
                wr(a, 0, st);
                S_OK
            }
            Err(hr) => hr,
        },
        PROPERTY_GET_COUNT => match wasapi::property_count(device_for(object).as_deref()) {
            Ok(n) => {
                wr(a, 0, n);
                S_OK
            }
            Err(hr) => hr,
        },
        PROPERTY_GET_AT => {
            let i: u32 = rd(a, 0);
            match wasapi::property_at(device_for(object).as_deref(), i) {
                Ok(k) => {
                    core::ptr::copy_nonoverlapping(k.fmtid.as_ptr(), a.add(4), 16);
                    wr(a, 0x14, k.pid);
                    S_OK
                }
                Err(hr) => hr,
            }
        }
        PROPERTY_GET_VALUE => property_value(object, a),
        SPATIAL_NATIVE_MASK | SPATIAL_MAX_DYNAMIC | SPATIAL_MAX_FRAMES | SPATIAL_FORMAT_SUPPORTED | SPATIAL_STREAM_AVAILABLE | SPATIAL_OFFLOAD_CAPABLE => spatial(id, object, a),
        ENDPOINT_CHANNEL_COUNT => {
            wr(a, 0, endpoint_channels(&device_for(object)));
            S_OK
        }
        SET_ENDPOINT_VOLUME_LEVEL | SET_ENDPOINT_VOLUME_SCALAR | GET_ENDPOINT_VOLUME_LEVEL | GET_ENDPOINT_VOLUME_SCALAR | SET_ENDPOINT_CHANNEL_LEVEL | SET_ENDPOINT_CHANNEL_SCALAR | GET_ENDPOINT_CHANNEL_LEVEL | GET_ENDPOINT_CHANNEL_SCALAR | SET_ENDPOINT_MUTE | GET_ENDPOINT_MUTE | ENDPOINT_STEP_INFO | ENDPOINT_STEP_UP | ENDPOINT_STEP_DOWN | ENDPOINT_HARDWARE_SUPPORT | ENDPOINT_VOLUME_RANGE => endpoint_volume(id, object, a),
        ENDPOINT_LIST => endpoint_list(object, a),
        DEFAULT_ENDPOINT_TABLE => default_table(a),
        SESSION_CHANNEL_COUNT | SESSION_SET_CHANNEL_VOLUME | SESSION_GET_CHANNEL_VOLUME | SESSION_SET_ALL_VOLUMES | SESSION_GET_ALL_VOLUMES => session_volume(id, object, a),
        _ => {
            compat_common::record_raw("crossvm-audio", "unserved", &format!("call id {id:#x} has no decoded argument layout; E_NOTIMPL"));
            E_NOTIMPL
        }
    }
}

unsafe fn format_at(a: *const u8) -> Vec<u8> {
    let head = core::slice::from_raw_parts(a, 0x12);
    core::slice::from_raw_parts(a, wasapi::format_len(head).max(0x12)).to_vec()
}

unsafe fn create_object(a: *mut u8, room: usize) -> i32 {
    if room < 0x14 {
        return E_INVALIDARG;
    }
    let kind: u32 = rd(a, 0);
    let flag: u32 = rd(a, 4);
    let parent: u32 = rd(a, 8);
    let method: u32 = rd(a, 0x10);
    if kind >= max_object_type() || flag > 1 {
        return E_INVALIDARG;
    }
    let device = match method {
        METHOD_DEFAULT_ENDPOINT | METHOD_DEFAULT_ENDPOINT_19041 => {
            let flow: u32 = rd(a, 0x14);
            let role: u32 = rd(a, 0x18);
            if flow > 1 || role >= PC_ROLES {
                return E_INVALIDARG;
            }
            match wasapi::default_id(flow, role) {
                Ok(id) => {
                    if method == METHOD_DEFAULT_ENDPOINT && !put_wide(a.add(BLOB_ID), &id, BLOB_ID_CHARS + 1) {
                        compat_common::record_raw("crossvm-audio", "CreateObject", &format!("PC endpoint id {id} is longer than the {BLOB_ID_CHARS}-char guest field"));
                        return E_INVALIDARG;
                    }
                    Some(id)
                }
                Err(hr) => return hr,
            }
        }
        METHOD_ENDPOINT | METHOD_ENDPOINT_19041 => match wide_at(a.add(BLOB_ID), BLOB_ID_CHARS + 1) {
            Some(id) if !id.is_empty() => match wasapi::device_state(Some(&id)) {
                Ok(_) => Some(id),
                Err(hr) => return hr,
            },
            _ if method == METHOD_ENDPOINT => return E_INVALIDARG,
            _ => None,
        },
        _ => None,
    };
    let mut s = state();
    let n = s.next;
    s.next += 1;
    compat_common::record_raw("crossvm-audio", "CreateObject", &format!("type {kind:#x} flag {flag} parent {parent:#x} method {method:#x} device {device:?} -> object {n:#x}"));
    s.objects.insert(n, Object::new(kind, if flag == 1 { 0 } else { parent }, device));
    wr(a, 0xc, n);
    S_OK
}

unsafe fn is_format_supported(object: u32, a: *mut u8) -> i32 {
    let want: u8 = rd(a, 0);
    let mode: u32 = rd(a, 2);
    let f = format_at(a.add(6));
    if f.len() < 0x12 || mode > 1 {
        return E_INVALIDARG;
    }
    let (hr, near) = wasapi::is_format_supported_on(device_for(object).as_deref(), mode, &f);
    wr(a, 1, 0u8);
    if let (Some(c), true) = (near, want != 0) {
        core::ptr::write_bytes(a.add(0x4e), 0, wasapi::MAX_FORMAT);
        core::ptr::copy_nonoverlapping(c.as_ptr(), a.add(0x4e), c.len());
        wr(a, 1, 1u8);
    }
    hr
}

unsafe fn transport(id: u32, object: u32) -> i32 {
    let s = state();
    let Some(st) = stream_owner(&s, object).and_then(|k| s.objects.get(&k)).and_then(|o| o.stream.as_ref()) else {
        return AUDCLNT_E_NOT_INITIALIZED;
    };
    let sh = &st.renderer.shared;
    let running = sh.running.load(Ordering::SeqCst);
    match id {
        START => {
            if running {
                return AUDCLNT_E_NOT_STOPPED;
            }
            let mut sig = sh.signal.lock().unwrap();
            sig.clear();
            sig.push(st.event);
            sig.extend(crate::vab::sra_events());
            drop(sig);
            sh.running.store(true, Ordering::SeqCst);
            S_OK
        }
        STOP => {
            if !running {
                return S_FALSE;
            }
            sh.running.store(false, Ordering::SeqCst);
            S_OK
        }
        _ => {
            if running {
                return AUDCLNT_E_NOT_STOPPED;
            }
            sh.reset.store(true, Ordering::SeqCst);
            S_OK
        }
    }
}

fn simple<R>(object: u32, f: impl FnOnce(&mut Object) -> R) -> Option<R> {
    let mut s = state();
    let k = stream_owner(&s, object).unwrap_or(object);
    let r = f(s.objects.get_mut(&k)?);
    apply_all(&mut s);
    Some(r)
}

unsafe fn stream_volume(id: u32, object: u32, a: *mut u8) -> i32 {
    let mut s = state();
    let Some(k) = stream_owner(&s, object) else { return AUDCLNT_E_NOT_INITIALIZED };
    let o = s.objects.get_mut(&k).unwrap();
    let n = wasapi::channels(&o.stream.as_ref().unwrap().format) as usize;
    let v = o.volumes.get_or_insert_with(|| vec![1.0; n]);
    let r = channel_op(id - STREAM_GET_CHANNEL_COUNT, v, a);
    apply_all(&mut s);
    r
}

unsafe fn channel_op(op: u32, v: &mut Vec<f32>, a: *mut u8) -> i32 {
    let n = v.len();
    match op {
        0 => {
            wr(a, 0, n as u32);
            S_OK
        }
        1 => {
            let i: u32 = rd(a, 0);
            let x: f32 = rd(a, 4);
            if i as usize >= n || !(0.0..=1.0).contains(&x) {
                return E_INVALIDARG;
            }
            v[i as usize] = x;
            S_OK
        }
        2 => {
            let i: u32 = rd(a, 0);
            if i as usize >= n {
                return E_INVALIDARG;
            }
            wr(a, 0, v[i as usize]);
            S_OK
        }
        3 => {
            let c: u32 = rd(a, 0);
            if c as usize != n || n > MAX_VOLUMES {
                return E_INVALIDARG;
            }
            let mut nv = Vec::with_capacity(n);
            for i in 0..n {
                let x: f32 = rd(a, 4 + 4 * i);
                if !(0.0..=1.0).contains(&x) {
                    return E_INVALIDARG;
                }
                nv.push(x);
            }
            *v = nv;
            S_OK
        }
        _ => {
            let c: u32 = rd(a, 0);
            if c as usize != n || n > MAX_VOLUMES {
                return E_INVALIDARG;
            }
            wr(a, 0, n as u32);
            for (i, x) in v.iter().enumerate() {
                wr(a, 4 + 4 * i, *x);
            }
            S_OK
        }
    }
}

unsafe fn session_volume(id: u32, object: u32, a: *mut u8) -> i32 {
    let dev = device_for(object);
    let n = wasapi::device_info_for(dev.as_deref()).map(|d| wasapi::channels(&d.mix) as usize).unwrap_or(2);
    let mut s = state();
    let mut v = s.session_channels.clone().unwrap_or_else(|| vec![1.0; n]);
    let r = channel_op(id - SESSION_CHANNEL_COUNT, &mut v, a);
    if r >= 0 && (id == SESSION_SET_CHANNEL_VOLUME || id == SESSION_SET_ALL_VOLUMES) {
        s.session_channels = Some(v);
        apply_all(&mut s);
    }
    r
}
fn register(object: u32, on: bool) -> i32 {
    let mut s = state();
    match s.objects.get_mut(&object) {
        Some(o) => {
            o.notify = on;
            compat_common::record_raw("crossvm-audio", "notify", &format!("object {object:#x} notifications {}", if on { "registered" } else { "unregistered" }));
            S_OK
        }
        None => E_INVALIDARG,
    }
}

pub fn notifications_registered(object: u32) -> bool {
    state().objects.get(&object).map(|o| o.notify).unwrap_or(false)
}

unsafe fn property_value(object: u32, a: *mut u8) -> i32 {
    let mut key = wasapi::PropertyKey::default();
    core::ptr::copy_nonoverlapping(a, key.fmtid.as_mut_ptr(), 16);
    key.pid = rd(a, 0x10);
    let v = match wasapi::property_value(device_for(object).as_deref(), &key) {
        Ok(v) => v,
        Err(hr) => return hr,
    };
    core::ptr::write_bytes(a.add(0x14), 0, 4 + PROP_DATA + 8);
    let (tag, data): (u32, Vec<u8>) = match v {
        Some(wasapi::Prop::Empty) => (0, Vec::new()),
        Some(wasapi::Prop::Text(w)) => (1, w.iter().flat_map(|c| c.to_le_bytes()).collect()),
        Some(wasapi::Prop::Dword(d)) => (4, d.to_le_bytes().to_vec()),
        Some(wasapi::Prop::Serialized(vt, bytes)) => {
            let mut d = vec![0u8; 8];
            d[0..2].copy_from_slice(&vt.to_le_bytes());
            d.extend_from_slice(&bytes);
            (3, d)
        }
        None => {
            compat_common::record_raw("crossvm-audio", "property", "PC property type has no decoded guest serialisation; E_NOTIMPL");
            return E_NOTIMPL;
        }
    };
    if data.len() > PROP_DATA {
        return E_OUTOFMEMORY;
    }
    wr(a, 0x14, tag);
    core::ptr::copy_nonoverlapping(data.as_ptr(), a.add(0x18), data.len());
    wr(a, 0x18 + PROP_DATA, data.len() as u64);
    S_OK
}

unsafe fn spatial(id: u32, object: u32, a: *mut u8) -> i32 {
    let sp = match wasapi::spatial(device_for(object).as_deref()) {
        Ok(s) => s,
        Err(hr) => return hr,
    };
    let put = |r: Result<u32, i32>, at: usize| match r {
        Ok(v) => {
            wr(a, at, v);
            S_OK
        }
        Err(hr) => hr,
    };
    match id {
        SPATIAL_NATIVE_MASK => put(sp.native_mask(), 0),
        SPATIAL_MAX_DYNAMIC => put(sp.max_dynamic(), 0),
        SPATIAL_MAX_FRAMES => {
            let f = format_at(a);
            put(sp.max_frames(&f), 0x48)
        }
        SPATIAL_FORMAT_SUPPORTED => sp.format_supported(&format_at(a)),
        SPATIAL_STREAM_AVAILABLE => sp.stream_available(&guid_from(core::slice::from_raw_parts(a, 16))),
        _ => {
            let cat: u32 = rd(a, 0);
            put(sp.offload_capable(cat), 4)
        }
    }
}

unsafe fn endpoint_volume(id: u32, object: u32, a: *mut u8) -> i32 {
    let dev = device_for(object);
    let caps = match wasapi::endpoint_caps(dev.as_deref()) {
        Ok(c) => c,
        Err(hr) => return hr,
    };
    let steps = caps.steps.max(2);
    match id {
        SET_ENDPOINT_VOLUME_LEVEL | SET_ENDPOINT_VOLUME_SCALAR => {
            let x: f32 = rd(a, 0);
            let v = if id == SET_ENDPOINT_VOLUME_LEVEL {
                if x < caps.min_db || x > caps.max_db {
                    return E_INVALIDARG;
                }
                level_to_scalar(x, &caps)
            } else {
                if !(0.0..=1.0).contains(&x) {
                    return E_INVALIDARG;
                }
                x
            };
            with_endpoint(&dev, |e| e.scalar = v);
            S_OK
        }
        GET_ENDPOINT_VOLUME_LEVEL | GET_ENDPOINT_VOLUME_SCALAR => {
            let v = with_endpoint(&dev, |e| e.scalar);
            wr(a, 0, if id == GET_ENDPOINT_VOLUME_LEVEL { scalar_to_level(v, &caps) } else { v });
            S_OK
        }
        SET_ENDPOINT_CHANNEL_LEVEL | SET_ENDPOINT_CHANNEL_SCALAR => {
            let ch: u32 = rd(a, 0);
            let x: f32 = rd(a, 4);
            let v = if id == SET_ENDPOINT_CHANNEL_LEVEL {
                if x < caps.min_db || x > caps.max_db {
                    return E_INVALIDARG;
                }
                level_to_scalar(x, &caps)
            } else {
                if !(0.0..=1.0).contains(&x) {
                    return E_INVALIDARG;
                }
                x
            };
            with_endpoint(&dev, |e| match e.channels.get_mut(ch as usize) {
                Some(c) => {
                    *c = v;
                    S_OK
                }
                None => E_INVALIDARG,
            })
        }
        GET_ENDPOINT_CHANNEL_LEVEL | GET_ENDPOINT_CHANNEL_SCALAR => {
            let ch: u32 = rd(a, 0);
            match with_endpoint(&dev, |e| e.channels.get(ch as usize).copied()) {
                Some(v) => {
                    wr(a, 0, if id == GET_ENDPOINT_CHANNEL_LEVEL { scalar_to_level(v, &caps) } else { v });
                    S_OK
                }
                None => E_INVALIDARG,
            }
        }
        SET_ENDPOINT_MUTE => {
            let m: u8 = rd(a, 0);
            let was = with_endpoint(&dev, |e| core::mem::replace(&mut e.mute, m != 0));
            if was == (m != 0) { S_FALSE } else { S_OK }
        }
        GET_ENDPOINT_MUTE => {
            let m = with_endpoint(&dev, |e| e.mute);
            wr(a, 0, m as u8);
            S_OK
        }
        ENDPOINT_STEP_INFO => {
            let v = with_endpoint(&dev, |e| e.scalar);
            wr(a, 0, (v * (steps - 1) as f32).round() as u32);
            wr(a, 4, steps);
            S_OK
        }
        ENDPOINT_STEP_UP | ENDPOINT_STEP_DOWN => {
            let d = 1.0 / (steps - 1) as f32;
            with_endpoint(&dev, |e| {
                let cur = (e.scalar * (steps - 1) as f32).round();
                let next = if id == ENDPOINT_STEP_UP { cur + 1.0 } else { cur - 1.0 };
                e.scalar = (next * d).clamp(0.0, 1.0);
            });
            S_OK
        }
        ENDPOINT_HARDWARE_SUPPORT => {
            wr(a, 0, 0u32);
            S_OK
        }
        _ => {
            wr(a, 0, caps.min_db);
            wr(a, 4, caps.max_db);
            wr(a, 8, caps.inc_db);
            S_OK
        }
    }
}

unsafe fn endpoint_list(object: u32, a: *mut u8) -> i32 {
    let flow: u32 = rd(a, 0);
    let mask: u32 = rd(a, 4);
    if flow > 2 || mask > 0xf {
        return E_INVALIDARG;
    }
    let list = match wasapi::endpoints(flow, mask) {
        Ok(l) => l,
        Err(hr) => return hr,
    };
    let mut s = state();
    let Some(o) = s.objects.get_mut(&object) else { return E_INVALIDARG };
    let at = o.list_cursor.min(list.len());
    let page: Vec<&(String, u32)> = list[at..].iter().take(LIST_PAGE).collect();
    let last = at + page.len() >= list.len();
    o.list_cursor = if last { 0 } else { at + page.len() };
    drop(s);
    core::ptr::write_bytes(a.add(8), 0, 8 + LIST_PAGE * LIST_ENTRY);
    wr(a, 8, page.len() as u32);
    wr(a, 0xc, last as u32);
    for (i, (id, st)) in page.iter().enumerate() {
        let e = a.add(0x10 + i * LIST_ENTRY);
        if !put_wide(e, id, BLOB_ID_CHARS + 1) {
            return E_INVALIDARG;
        }
        wr(e, 0x70, *st);
    }
    S_OK
}

unsafe fn default_table(a: *mut u8) -> i32 {
    let mut entries = Vec::new();
    for flow in 0..2u32 {
        for role in 0..PC_ROLES {
            match wasapi::default_id(flow, role) {
                Ok(id) => entries.push((flow, role, id)),
                Err(wasapi::E_NOTFOUND) => {}
                Err(hr) => return hr,
            }
        }
    }
    if entries.len() > LIST_PAGE {
        entries.truncate(LIST_PAGE);
    }
    core::ptr::write_bytes(a, 0, 8 + LIST_PAGE * TABLE_ENTRY);
    wr(a, 0, entries.len() as u32);
    wr(a, 4, 1u32);
    for (i, (flow, role, id)) in entries.iter().enumerate() {
        let e = a.add(8 + i * TABLE_ENTRY);
        wr(e, 0, *flow);
        wr(e, 4, *role);
        if !put_wide(e.add(8), id, BLOB_ID_CHARS + 1) {
            return E_INVALIDARG;
        }
    }
    S_OK
}

unsafe fn initialize(object: u32, a: *mut u8, room: usize) -> i32 {
    if room < INIT_STRUCT {
        return E_INVALIDARG;
    }
    let (existing, props) = {
        let s = state();
        match s.objects.get(&object) {
            Some(o) => (o.stream.is_some(), o.properties),
            None => return E_INVALIDARG,
        }
    };
    if existing {
        return AUDCLNT_E_ALREADY_INITIALIZED;
    }
    let mode: u32 = rd(a, INIT_SHARE_MODE);
    if mode > 1 {
        return E_INVALIDARG;
    }
    let duration: i64 = rd(a, 8);
    let format = format_at(a.add(INIT_FORMAT));
    let flen = format.len();
    let align = wasapi::block_align(&format);
    let rate = wasapi::sample_rate(&format);
    if align == 0 || rate == 0 {
        return E_INVALIDARG;
    }
    let session = guid_from(core::slice::from_raw_parts(a.add(INIT_SESSION), 16));
    let dev = device_for(object);
    let info = match wasapi::device_info_for(dev.as_deref()) {
        Ok(d) => d,
        Err(hr) => return hr,
    };
    let period_frames = (info.default_period * rate as i64 / HNS).max(1) as u32;
    let frames = ((duration.max(info.default_period * 2) * rate as i64 + HNS - 1) / HNS) as u32;
    let l = layout(&format, frames, period_frames);
    let pages = (l.total as usize).div_ceil(crate::logan::PAGE);
    let Some(index) = crate::logan::pool_alloc(pages) else {
        return E_OUTOFMEMORY;
    };
    let base = crate::logan::pool() + index as usize * crate::logan::PAGE;
    let hns = frames as i64 * HNS / rate as i64;
    write_control_block(base, &format, hns, &l);
    let ring = wasapi::Ring { base, start: l.start, end: l.end, align };
    let open = wasapi::Open { device: dev.clone(), format: format.clone(), duration: hns, session, properties: props };
    let (renderer, device_frames) = match wasapi::open(open, ring) {
        Ok(r) => r,
        Err(hr) => {
            crate::logan::pool_release(index as usize);
            return hr;
        }
    };
    let event_id = {
        let mut s = state();
        s.next_event += 1;
        s.next_event
    };
    let name = event_name(event_id as u16);
    let event = match crate::vab::create_event(&name) {
        Ok(h) => h,
        Err(e) => {
            crate::logan::pool_release(index as usize);
            return (0x8007_0000 | (e & 0xffff)) as i32;
        }
    };
    core::ptr::write_bytes(a.add(INIT_ENDPOINT_CLSID), 0, INIT_STRUCT - INIT_ENDPOINT_CLSID);
    core::ptr::copy_nonoverlapping(format.as_ptr(), a.add(INIT_FORMAT), flen);
    core::ptr::copy_nonoverlapping(guid_bytes(&CLSID_CLIENT_OUTPUT_ENDPOINT).as_ptr(), a.add(INIT_ENDPOINT_CLSID), 16);
    wr(a, INIT_EVENT1, KIND_NONE);
    wr(a, INIT_EVENT2, KIND_NONE);
    write_event_descriptor(a.add(INIT_EVENT3), EVENT_KIND_GAMECORE, event_id as u16);
    wr(a, INIT_MEMORY, KIND_CROSS_VM_MEMORY);
    core::ptr::copy_nonoverlapping(guid_bytes(&CLSID_XVM_BUFFER).as_ptr(), a.add(INIT_MEMORY + 4), 16);
    core::ptr::copy_nonoverlapping(guid_bytes(&GUID_XVM_BUFFER).as_ptr(), a.add(INIT_MEMORY + 0x14), 16);
    wr(a, INIT_MEMORY + 0x24, index as u32);
    compat_common::record_raw("crossvm-audio", "Initialize", &format!("object {object:#x}: {frames} frames ({} bytes) at pool page {index}, WASAPI buffer {device_frames} frames, period event {name}", l.bytes));
    let mut s = state();
    s.session.get_or_insert(session);
    if let Some(o) = s.objects.get_mut(&object) {
        o.stream = Some(Stream { pool_index: index, base, format, frames, event, event_name: name, renderer });
    }
    apply_all(&mut s);
    S_OK
}

pub unsafe fn write_event_descriptor(p: *mut u8, kind: u32, index: u16) {
    wr(p, 0, KIND_CROSS_VM_EVENT);
    core::ptr::copy_nonoverlapping(guid_bytes(&CLSID_XVM_EVENT).as_ptr(), p.add(4), 16);
    core::ptr::copy_nonoverlapping(guid_bytes(&GUID_XVM_EVENT).as_ptr(), p.add(0x14), 16);
    wr(p, 0x24, kind);
    wr(p, 0x28, index);
}

pub fn stream_info(object: u32) -> Option<(usize, u16, u32, String, i64)> {
    let s = state();
    let st = s.objects.get(&object)?.stream.as_ref()?;
    Some((st.base, st.pool_index, st.frames, st.event_name.clone(), st.renderer.shared.rendered.load(Ordering::SeqCst)))
}

pub fn stream_signals(object: u32) -> Option<i64> {
    let s = state();
    Some(s.objects.get(&object)?.stream.as_ref()?.renderer.shared.signals.load(Ordering::SeqCst))
}

pub fn stream_gain(object: u32) -> Option<(Option<f32>, Option<Vec<f32>>)> {
    let s = state();
    let sh = &s.objects.get(&object)?.stream.as_ref()?.renderer.shared;
    let g = *sh.gain.lock().unwrap();
    let c = sh.channels.lock().unwrap().clone();
    Some((g, c))
}

pub fn object_kind(object: u32) -> Option<u32> {
    state().objects.get(&object).map(|o| o.kind)
}

pub fn object_device(object: u32) -> Option<String> {
    device_for(object)
}

pub fn take_capture(object: u32) -> Option<Vec<u8>> {
    let s = state();
    let st = s.objects.get(&object)?.stream.as_ref()?;
    let mut c = st.renderer.shared.capture.lock().unwrap();
    c.as_mut().map(std::mem::take)
}

pub fn objects() -> usize {
    state().objects.len()
}
