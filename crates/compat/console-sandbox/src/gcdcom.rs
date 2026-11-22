use core::ffi::c_void;
use std::sync::atomic::{AtomicI32, AtomicU32, Ordering};

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Guid {
    pub data1: u32,
    pub data2: u16,
    pub data3: u16,
    pub data4: [u8; 8],
}

pub const CLSID_API_SERVICE: Guid = Guid { data1: 0xCA495FA6, data2: 0x7C3D, data3: 0x4220, data4: [0xBB, 0x44, 0x9E, 0xC8, 0xB7, 0x6F, 0x5B, 0x35] };
pub const IID_API_SERVICE: Guid = Guid { data1: 0xB74D51FD, data2: 0x388B, data3: 0x40C5, data4: [0x84, 0x34, 0x00, 0x23, 0xC8, 0xAF, 0x23, 0xCD] };
pub const ID_PROXY_CONTROLLER2: Guid = Guid { data1: 0x8DF04845, data2: 0x52BC, data3: 0x4D25, data4: [0xA2, 0xAB, 0xD7, 0x2A, 0xB1, 0x58, 0xC1, 0x08] };
pub const ID_PROXY_CONTROLLER: Guid = Guid { data1: 0xAB82175E, data2: 0x3BD7, data3: 0x4465, data4: [0x8F, 0x1B, 0x94, 0x36, 0xFB, 0x37, 0x43, 0x8B] };
pub const CLSID_DISPLAY_MODE_SERVER: Guid = Guid { data1: 0x255DE1B5, data2: 0xD2BF, data3: 0x5303, data4: [0x88, 0xF1, 0x25, 0x80, 0xB4, 0xE6, 0x0A, 0xD9] };
pub const IID_DISPLAY_MODE_SERVER3: Guid = Guid { data1: 0xC66A6DE1, data2: 0x07CE, data3: 0x5028, data4: [0xA8, 0x97, 0xD9, 0x6A, 0x72, 0x2E, 0x0F, 0x89] };
pub const IID_IUNKNOWN: Guid = Guid { data1: 0, data2: 0, data3: 0, data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };
pub const IID_IINSPECTABLE: Guid = Guid { data1: 0xAF86E2E0, data2: 0xB12D, data3: 0x4C6A, data4: [0x9C, 0x5A, 0xD7, 0xAA, 0x65, 0x10, 0x1E, 0x90] };
pub const HOST_SERVER: &str = "C12A416B-F347-405C-B765-2DE027A6249E";
pub const CLSCTX_REMOTE_SERVER: u32 = 0x10;

pub const S_OK: i32 = 0;
pub const E_NOINTERFACE: i32 = 0x8000_4002u32 as i32;
pub const E_POINTER: i32 = 0x8000_4003u32 as i32;
pub const E_INVALIDARG: i32 = 0x8007_0057u32 as i32;
pub const E_OUTOFMEMORY: i32 = 0x8007_000Eu32 as i32;
pub const CLASS_E_NOAGGREGATION: i32 = 0x8004_0110u32 as i32;
pub const REGDB_E_CLASSNOTREG: i32 = 0x8004_0154u32 as i32;
pub const CO_S_NOTALLINTERFACES: i32 = 0x0008_0012;

#[repr(C)]
pub struct MultiQi {
    pub iid: *const Guid,
    pub itf: *mut c_void,
    pub hr: i32,
}

#[repr(C)]
pub struct ServerInfo {
    pub reserved1: u32,
    pub name: *const u16,
    pub auth: *mut c_void,
    pub reserved2: u32,
}

#[repr(C)]
struct Obj {
    vt: *const *const (),
    refs: &'static AtomicU32,
    iid: Guid,
}

struct Vt<const N: usize>([*const (); N]);
unsafe impl<const N: usize> Sync for Vt<N> {}

struct Holder(Obj);
unsafe impl Sync for Holder {}

static BROKER_REFS: AtomicU32 = AtomicU32::new(0);
static TARGET_REFS: AtomicU32 = AtomicU32::new(0);
static CONTROLLER_REFS: AtomicU32 = AtomicU32::new(0);
static DISPLAY_REFS: AtomicU32 = AtomicU32::new(0);
static LAST_RESULT: AtomicI32 = AtomicI32::new(0);

static BROKER_VT: Vt<4> = Vt([qi as *const (), add_ref as *const (), release as *const (), broker_get as *const ()]);
static TARGET_VT: Vt<4> = Vt([qi as *const (), add_ref as *const (), release as *const (), target_register as *const ()]);
static CONTROLLER_VT: Vt<5> = Vt([qi as *const (), add_ref as *const (), release as *const (), controller_open as *const (), controller_close as *const ()]);
static DISPLAY_VT: Vt<7> = Vt([qi as *const (), add_ref as *const (), release as *const (), get_iids as *const (), get_runtime_class_name as *const (), get_trust_level as *const (), display_caps as *const ()]);

static BROKER: Holder = Holder(Obj { vt: BROKER_VT.0.as_ptr(), refs: &BROKER_REFS, iid: IID_API_SERVICE });
static TARGET: Holder = Holder(Obj { vt: TARGET_VT.0.as_ptr(), refs: &TARGET_REFS, iid: ID_PROXY_CONTROLLER2 });
static CONTROLLER: Holder = Holder(Obj { vt: CONTROLLER_VT.0.as_ptr(), refs: &CONTROLLER_REFS, iid: ID_PROXY_CONTROLLER });
static DISPLAY: Holder = Holder(Obj { vt: DISPLAY_VT.0.as_ptr(), refs: &DISPLAY_REFS, iid: IID_DISPLAY_MODE_SERVER3 });

pub fn live_refs() -> (u32, u32) {
    (BROKER_REFS.load(Ordering::SeqCst), TARGET_REFS.load(Ordering::SeqCst))
}

pub fn all_refs() -> [u32; 4] {
    [&BROKER_REFS, &TARGET_REFS, &CONTROLLER_REFS, &DISPLAY_REFS].map(|r| r.load(Ordering::SeqCst))
}

pub fn result_value() -> u32 {
    LAST_RESULT.load(Ordering::SeqCst) as u32
}

fn obj(h: &'static Holder) -> *mut c_void {
    h.0.refs.fetch_add(1, Ordering::SeqCst);
    &h.0 as *const Obj as *mut c_void
}

unsafe extern "system" fn add_ref(this: *mut c_void) -> u32 {
    (*(this as *const Obj)).refs.fetch_add(1, Ordering::SeqCst) + 1
}

unsafe extern "system" fn release(this: *mut c_void) -> u32 {
    let r = (*(this as *const Obj)).refs;
    let mut cur = r.load(Ordering::SeqCst);
    loop {
        if cur == 0 {
            return 0;
        }
        match r.compare_exchange(cur, cur - 1, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => return cur - 1,
            Err(v) => cur = v,
        }
    }
}

unsafe extern "system" fn qi(this: *mut c_void, iid: *const Guid, out: *mut *mut c_void) -> i32 {
    if out.is_null() {
        return E_POINTER;
    }
    *out = core::ptr::null_mut();
    if iid.is_null() {
        return E_INVALIDARG;
    }
    let own = (*(this as *const Obj)).iid;
    let inspectable = (own == IID_DISPLAY_MODE_SERVER3 || own == IID_NET_SECURITY || own == IID_NET_SECURITY_RESULT) && *iid == IID_IINSPECTABLE;
    if *iid == IID_IUNKNOWN || *iid == own || inspectable || *iid == IID_IAGILE_OBJECT {
        add_ref(this);
        *out = this;
        return S_OK;
    }
    E_NOINTERFACE
}

#[link(name = "ole32")]
extern "system" {
    fn CoTaskMemAlloc(n: usize) -> *mut c_void;
    fn CoCreateInstanceEx(clsid: *const Guid, outer: *mut c_void, ctx: u32, si: *const ServerInfo, n: u32, mqi: *mut MultiQi) -> i32;
}

unsafe extern "system" fn get_iids(this: *mut c_void, count: *mut u32, iids: *mut *mut Guid) -> i32 {
    if count.is_null() || iids.is_null() {
        return E_POINTER;
    }
    let p = CoTaskMemAlloc(core::mem::size_of::<Guid>()) as *mut Guid;
    if p.is_null() {
        *count = 0;
        *iids = core::ptr::null_mut();
        return E_OUTOFMEMORY;
    }
    *p = (*(this as *const Obj)).iid;
    *count = 1;
    *iids = p;
    S_OK
}

unsafe extern "system" fn get_runtime_class_name(_this: *mut c_void, name: *mut *mut c_void) -> i32 {
    if name.is_null() {
        return E_POINTER;
    }
    *name = core::ptr::null_mut();
    S_OK
}

unsafe extern "system" fn get_trust_level(_this: *mut c_void, level: *mut u32) -> i32 {
    if level.is_null() {
        return E_POINTER;
    }
    *level = 0;
    S_OK
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DisplayCaps {
    pub a: f32,
    pub b: f32,
    pub flags: u32,
}

pub fn display_caps_value() -> DisplayCaps {
    let d = crate::gcreply::display();
    DisplayCaps { a: 0.0, b: 0.0, flags: crate::gcreply::display_mode_1_flags(&d) }
}

unsafe extern "system" fn display_caps(_this: *mut c_void, out: *mut DisplayCaps) -> i32 {
    if out.is_null() {
        return E_POINTER;
    }
    let v = display_caps_value();
    *out = v;
    compat_common::record_raw("gamecore-host", "DisplayMode.IServer3", &format!("slot 6 caps flags {:#x}", v.flags));
    S_OK
}

unsafe extern "system" fn controller_open(_this: *mut c_void, id: *const Guid, c: u16, d: u64, out: *mut u32) -> i32 {
    if id.is_null() || out.is_null() {
        return E_POINTER;
    }
    *out = 0;
    compat_common::record_raw("gamecore-host", "IGameCoreSipcProxyController", &format!("slot 3 id {} c={c} d={d:#x} -> S_OK, 0", guid_text(&*id)));
    S_OK
}

unsafe extern "system" fn controller_close(_this: *mut c_void, id: *const Guid, c: u16) -> i32 {
    if id.is_null() {
        return E_POINTER;
    }
    compat_common::record_raw("gamecore-host", "IGameCoreSipcProxyController", &format!("slot 4 id {} c={c} -> S_OK", guid_text(&*id)));
    S_OK
}

fn served(clsid: &Guid, iid: &Guid) -> Option<&'static Holder> {
    if *clsid == ID_PROXY_CONTROLLER2 {
        Some(&TARGET)
    } else if *clsid == ID_PROXY_CONTROLLER {
        Some(&CONTROLLER)
    } else if *clsid == CLSID_DISPLAY_MODE_SERVER && *iid == IID_DISPLAY_MODE_SERVER4 {
        Some(&DISPLAY4)
    } else if *clsid == CLSID_DISPLAY_MODE_SERVER {
        Some(&DISPLAY)
    } else if *clsid == ID_CROSS_VM_AUDIO {
        Some(&AUDIO)
    } else if *clsid == ID_CURSOR_CONTROLLER {
        Some(&CURSOR)
    } else if *clsid == CLSID_PACKAGE_V7 && *iid == IID_PACKAGE_V7 {
        Some(&PACKAGE_V7)
    } else if *clsid == CLSID_NET_SECURITY {
        Some(&NET_SECURITY)
    } else {
        None
    }
}

pub const IID_DISPLAY_MODE_SERVER4: Guid = Guid { data1: 0xAE4E1013, data2: 0xA08C, data3: 0x53BF, data4: [0xAA, 0xBD, 0x6E, 0x1B, 0x33, 0x45, 0x8D, 0x2F] };
pub const ID_CROSS_VM_AUDIO: Guid = Guid { data1: 0x7267FD98, data2: 0xB90D, data3: 0x400A, data4: [0xB1, 0x2A, 0xBF, 0xAE, 0x79, 0x0F, 0x8F, 0xAC] };
pub const ID_CURSOR_CONTROLLER: Guid = Guid { data1: 0x939C387A, data2: 0x5057, data3: 0x4CF1, data4: [0x88, 0xE8, 0x4D, 0xA0, 0x3E, 0xE2, 0x83, 0x97] };
pub const E_NOTIMPL: i32 = 0x8000_4001u32 as i32;
pub const E_FAIL: i32 = 0x8000_4005u32 as i32;
pub const AUDIO_ENVELOPE: i32 = 0x810;

static DISPLAY4_REFS: AtomicU32 = AtomicU32::new(0);
static AUDIO_REFS: AtomicU32 = AtomicU32::new(0);
static CURSOR_REFS: AtomicU32 = AtomicU32::new(0);
static DISPLAY4_VT: Vt<7> = Vt([qi as *const (), add_ref as *const (), release as *const (), get_iids as *const (), get_runtime_class_name as *const (), get_trust_level as *const (), display_hdr as *const ()]);
static AUDIO_VT: Vt<5> = Vt([qi as *const (), add_ref as *const (), release as *const (), audio_call as *const (), audio_ack as *const ()]);
static CURSOR_VT: Vt<5> = Vt([qi as *const (), add_ref as *const (), release as *const (), cursor_type as *const (), cursor_image as *const ()]);
static DISPLAY4: Holder = Holder(Obj { vt: DISPLAY4_VT.0.as_ptr(), refs: &DISPLAY4_REFS, iid: IID_DISPLAY_MODE_SERVER4 });
static AUDIO: Holder = Holder(Obj { vt: AUDIO_VT.0.as_ptr(), refs: &AUDIO_REFS, iid: ID_CROSS_VM_AUDIO });
static CURSOR: Holder = Holder(Obj { vt: CURSOR_VT.0.as_ptr(), refs: &CURSOR_REFS, iid: ID_CURSOR_CONTROLLER });

pub fn more_refs() -> [u32; 3] {
    [&DISPLAY4_REFS, &AUDIO_REFS, &CURSOR_REFS].map(|r| r.load(Ordering::SeqCst))
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct HdrInfo {
    pub min: f32,
    pub max: f32,
    pub max_full_frame: f32,
    pub enabled: u8,
    pub pad: [u8; 3],
}

unsafe extern "system" fn display_hdr(_this: *mut c_void, mode: u32, out: *mut HdrInfo) -> i32 {
    if out.is_null() {
        return E_POINTER;
    }
    let mut f = [0f32; 3];
    let r = compat_common::gdk::basic::try_enable_hdr(mode, f.as_mut_ptr());
    *out = HdrInfo { min: f[0], max: f[1], max_full_frame: f[2], enabled: (r == compat_common::gdk::basic::HDR_ENABLED) as u8, pad: [0; 3] };
    compat_common::record_raw("gamecore-host", "DisplayMode.IServer4", &format!("slot 6 mode {mode} -> {r}"));
    if r == compat_common::gdk::basic::HDR_UNKNOWN {
        return if mode > 1 { E_INVALIDARG } else { E_FAIL };
    }
    S_OK
}

unsafe extern "system" fn audio_call(_this: *mut c_void, cb: i32, buf: *mut u8) -> i32 {
    if buf.is_null() {
        return E_POINTER;
    }
    crate::crossvm::call(cb, buf)
}

unsafe extern "system" fn audio_ack(_this: *mut c_void, v: i16) -> i32 {
    compat_common::record_raw("gamecore-host", "ICrossVmAudio", &format!("slot 4 {v}"));
    S_OK
}

#[repr(C)]
struct IconInfo {
    icon: i32,
    x: u32,
    y: u32,
    mask: usize,
    color: usize,
}

#[repr(C)]
struct BitmapV5 {
    size: u32,
    width: i32,
    height: i32,
    planes: u16,
    bits: u16,
    compression: u32,
    image_size: u32,
    xppm: i32,
    yppm: i32,
    used: u32,
    important: u32,
    masks: [u32; 4],
    rest: [u32; 17],
}

#[link(name = "user32")]
extern "system" {
    fn LoadCursorW(inst: usize, name: usize) -> usize;
    fn SetCursor(c: usize) -> usize;
    fn CreateIconIndirect(info: *const IconInfo) -> usize;
    fn DestroyCursor(c: usize) -> i32;
}

#[link(name = "gdi32")]
extern "system" {
    fn CreateDIBSection(dc: usize, bmi: *const BitmapV5, usage: u32, bits: *mut *mut u8, section: usize, offset: u32) -> usize;
    fn CreateBitmap(w: i32, h: i32, planes: u32, bits: u32, data: *const u8) -> usize;
    fn DeleteObject(o: usize) -> i32;
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct CursorState {
    pub kind: i32,
    pub handle: usize,
    pub custom: usize,
}

static CURSOR_STATE: std::sync::Mutex<CursorState> = std::sync::Mutex::new(CursorState { kind: 0, handle: 0, custom: 0 });

pub fn cursor_state() -> CursorState {
    *CURSOR_STATE.lock().unwrap_or_else(|e| e.into_inner())
}

fn system_cursor(kind: i32) -> Option<usize> {
    let id = match kind {
        0 => 32512,
        1 => 32515,
        3 => 32649,
        4 => 32651,
        5 => 32513,
        6 => 32646,
        7 => 32643,
        8 => 32645,
        9 => 32642,
        10 => 32644,
        11 => 32648,
        12 => 32516,
        13 => 32514,
        _ => return None,
    };
    Some(unsafe { LoadCursorW(0, id) })
}

unsafe extern "system" fn cursor_type(_this: *mut c_void, kind: i16) -> i32 {
    let kind = kind as i32;
    let mut s = CURSOR_STATE.lock().unwrap_or_else(|e| e.into_inner());
    let h = match kind {
        -1 => 0,
        2 => s.custom,
        k => match system_cursor(k) {
            Some(h) => h,
            None => {
                compat_common::record_raw("gamecore-host", "IGameCoreCursorController", &format!("cursor type {k} has no PC cursor; E_INVALIDARG"));
                return E_INVALIDARG;
            }
        },
    };
    s.kind = kind;
    s.handle = h;
    drop(s);
    SetCursor(h);
    S_OK
}

unsafe extern "system" fn cursor_image(_this: *mut c_void, w: i32, h: i32, hot_x: i32, hot_y: i32, pixels: *const u8, bytes: i32) -> i32 {
    if pixels.is_null() {
        return E_POINTER;
    }
    if w <= 0 || h <= 0 || w > 256 || h > 256 || hot_x < 0 || hot_y < 0 || hot_x >= w || hot_y >= h || bytes as i64 != w as i64 * h as i64 * 4 {
        return E_INVALIDARG;
    }
    let mut bmi: BitmapV5 = core::mem::zeroed();
    bmi.size = core::mem::size_of::<BitmapV5>() as u32;
    bmi.width = w;
    bmi.height = -h;
    bmi.planes = 1;
    bmi.bits = 32;
    bmi.compression = 3;
    bmi.masks = [0x00FF_0000, 0x0000_FF00, 0x0000_00FF, 0xFF00_0000];
    let mut bits = core::ptr::null_mut();
    let color = CreateDIBSection(0, &bmi, 0, &mut bits, 0, 0);
    if color == 0 || bits.is_null() {
        return E_OUTOFMEMORY;
    }
    core::ptr::copy_nonoverlapping(pixels, bits, bytes as usize);
    let mask = CreateBitmap(w, h, 1, 1, core::ptr::null());
    let info = IconInfo { icon: 0, x: hot_x as u32, y: hot_y as u32, mask, color };
    let c = CreateIconIndirect(&info);
    DeleteObject(color);
    DeleteObject(mask);
    if c == 0 {
        return E_FAIL;
    }
    let mut s = CURSOR_STATE.lock().unwrap_or_else(|e| e.into_inner());
    let old = s.custom;
    s.custom = c;
    let apply = s.kind == 2;
    if apply {
        s.handle = c;
    }
    drop(s);
    if apply {
        SetCursor(c);
    }
    if old != 0 {
        DestroyCursor(old);
    }
    S_OK
}

fn guid_text(g: &Guid) -> String {
    format!("{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{}", g.data1, g.data2, g.data3, g.data4[0], g.data4[1], g.data4[2..].iter().map(|b| format!("{b:02x}")).collect::<String>())
}

unsafe extern "system" fn broker_get(_this: *mut c_void, launch: *const Guid, clsid: *const Guid, iid: *const Guid, out: *mut *mut c_void) -> i32 {
    if out.is_null() {
        return E_POINTER;
    }
    *out = core::ptr::null_mut();
    if launch.is_null() || clsid.is_null() || iid.is_null() {
        return E_INVALIDARG;
    }
    let mine = core::slice::from_raw_parts(launch as *const u8, 16) == crate::claims::launch_instance();
    let Some(h) = served(&*clsid, &*iid) else {
        compat_common::record_raw("gamecore-host", "IGameCoreApiService", &format!("GetObject clsid {} iid {} is not served by the emulated host; REGDB_E_CLASSNOTREG", guid_text(&*clsid), guid_text(&*iid)));
        return REGDB_E_CLASSNOTREG;
    };
    let o = obj(h);
    let hr = qi(o, iid, out);
    release(o);
    compat_common::record_raw("gamecore-host", "IGameCoreApiService", &format!("GetObject clsid {} iid {} -> {hr:#010x} (launch instance matches emulated XBOX://LI: {mine})", guid_text(&*clsid), guid_text(&*iid)));
    hr
}

unsafe extern "system" fn target_register(_this: *mut c_void, id: *const Guid, a: u8, b: u8, c: u16, d: u64, out: *mut u32) -> i32 {
    if id.is_null() || out.is_null() {
        return E_POINTER;
    }
    let value = 0u32;
    *out = value;
    LAST_RESULT.store(value as i32, Ordering::SeqCst);
    compat_common::record_raw("gamecore-host", "IGameCoreSipcProxyController2", &format!("slot 3 id {} a={a} b={b} c={c} d={d:#x} -> S_OK, {value}", guid_text(&*id)));
    S_OK
}

unsafe fn server_is_host(si: *const ServerInfo) -> bool {
    if si.is_null() || (*si).name.is_null() {
        return false;
    }
    let mut n = 0;
    while n < 256 && *(*si).name.add(n) != 0 {
        n += 1;
    }
    String::from_utf16_lossy(core::slice::from_raw_parts((*si).name, n)).eq_ignore_ascii_case(HOST_SERVER)
}

pub unsafe fn host_activate(clsid: *const Guid, outer: *mut c_void, n: u32, mqi: *mut MultiQi) -> i32 {
    if clsid.is_null() || mqi.is_null() || n == 0 {
        return E_INVALIDARG;
    }
    if !outer.is_null() {
        return CLASS_E_NOAGGREGATION;
    }
    if *clsid != CLSID_API_SERVICE {
        compat_common::record_raw("gamecore-host", "CoCreateInstanceEx", &format!("host class {} is not served; REGDB_E_CLASSNOTREG", guid_text(&*clsid)));
        return REGDB_E_CLASSNOTREG;
    }
    let mut got = 0;
    for i in 0..n as usize {
        let m = &mut *mqi.add(i);
        m.itf = core::ptr::null_mut();
        if m.iid.is_null() {
            m.hr = E_INVALIDARG;
            continue;
        }
        if *m.iid == IID_API_SERVICE || *m.iid == IID_IUNKNOWN {
            m.itf = obj(&BROKER);
            m.hr = S_OK;
            got += 1;
        } else {
            m.hr = E_NOINTERFACE;
        }
    }
    compat_common::record_raw("gamecore-host", "CoCreateInstanceEx", &format!("IGameCoreApiService on the host VM: {got}/{n} interfaces"));
    if got == n as usize {
        S_OK
    } else if got > 0 {
        CO_S_NOTALLINTERFACES
    } else {
        E_NOINTERFACE
    }
}

pub unsafe extern "system" fn co_create_instance_ex(clsid: *const Guid, outer: *mut c_void, ctx: u32, si: *const ServerInfo, n: u32, mqi: *mut MultiQi) -> i32 {
    if ctx & CLSCTX_REMOTE_SERVER != 0 && server_is_host(si) {
        return host_activate(clsid, outer, n, mqi);
    }
    CoCreateInstanceEx(clsid, outer, ctx, si, n, mqi)
}

pub const IID_IAGILE_OBJECT: Guid = Guid { data1: 0x94EA2B94, data2: 0xE9CC, data3: 0x49E0, data4: [0xC0, 0xFF, 0xEE, 0x64, 0xCA, 0x8F, 0x5B, 0x90] };
pub const CLSID_PACKAGE_V7: Guid = Guid { data1: 0xE97EABA8, data2: 0x9BCD, data3: 0x4930, data4: [0x99, 0x2E, 0x2A, 0xDC, 0x66, 0x17, 0x68, 0x17] };
pub const IID_PACKAGE_V7: Guid = Guid { data1: 0x01958D59, data2: 0x8ABB, data3: 0x4DB9, data4: [0xB8, 0xB7, 0x17, 0x34, 0x5B, 0xE2, 0xE9, 0xFF] };
pub const CLSID_PACKAGE_AUX: Guid = Guid { data1: 0x1CD4BEF6, data2: 0xAEB3, data3: 0x41D7, data4: [0xAB, 0xBC, 0x61, 0xC3, 0x5C, 0xCB, 0xD4, 0xAD] };
pub const IID_PACKAGE_AUX: Guid = Guid { data1: 0xD657678E, data2: 0x9088, data3: 0x4EDD, data4: [0xA3, 0x9F, 0x23, 0x4A, 0xAF, 0x6B, 0xEB, 0xFF] };
pub const CLSID_NET_SECURITY: Guid = Guid { data1: 0x006DCD63, data2: 0x380F, data3: 0x41BF, data4: [0x9F, 0x85, 0xCF, 0x21, 0xBC, 0x47, 0x69, 0x55] };
pub const IID_NET_SECURITY: Guid = Guid { data1: 0x06C45D2E, data2: 0xECC6, data3: 0x4589, data4: [0xAD, 0x95, 0x01, 0x94, 0x37, 0x35, 0x9C, 0xFD] };
pub const IID_NET_SECURITY_RESULT: Guid = Guid { data1: 0x7DC51C93, data2: 0xE1B7, data3: 0x4A12, data4: [0x94, 0x77, 0x77, 0x4F, 0x33, 0xB2, 0x8D, 0xB0] };
pub const TLS_FLAGS: u32 = 0x0000_0800 | 0x0000_2000;
pub const PACKAGE_V7_SLOTS: usize = 32;
pub const E_ACCESSDENIED: i32 = 0x8007_0005u32 as i32;

#[link(name = "runtimeobject")]
extern "system" {
    fn WindowsCreateString(s: *const u16, len: u32, out: *mut *mut c_void) -> i32;
    fn WindowsGetStringRawBuffer(h: *mut c_void, len: *mut u32) -> *const u16;
}

static PACKAGE_V7_REFS: AtomicU32 = AtomicU32::new(0);
static NET_SECURITY_REFS: AtomicU32 = AtomicU32::new(0);
static NET_RESULT_REFS: AtomicU32 = AtomicU32::new(0);

macro_rules! unserved {
    ($name:ident, $iface:literal, $slot:literal) => {
        unsafe extern "system" fn $name(_this: *mut c_void) -> i32 {
            compat_common::record_raw("gamecore-host", $iface, concat!("slot ", $slot, " has no decoded meaning; E_NOTIMPL"));
            E_NOTIMPL
        }
    };
}

unserved!(v7_3, "PackageV7", 3);
unserved!(v7_4, "PackageV7", 4);
unserved!(v7_5, "PackageV7", 5);
unserved!(v7_6, "PackageV7", 6);
unserved!(v7_7, "PackageV7", 7);
unserved!(v7_9, "PackageV7", 9);
unserved!(v7_10, "PackageV7", 10);
unserved!(v7_12, "PackageV7", 12);
unserved!(v7_13, "PackageV7", 13);
unserved!(v7_14, "PackageV7", 14);
unserved!(v7_16, "PackageV7", 16);
unserved!(v7_17, "PackageV7", 17);
unserved!(v7_18, "PackageV7", 18);
unserved!(v7_19, "PackageV7", 19);
unserved!(v7_20, "PackageV7", 20);
unserved!(v7_21, "PackageV7", 21);
unserved!(v7_22, "PackageV7", 22);
unserved!(v7_23, "PackageV7", 23);
unserved!(v7_24, "PackageV7", 24);
unserved!(v7_25, "PackageV7", 25);
unserved!(v7_30, "PackageV7", 30);
unserved!(v7_31, "PackageV7", 31);

unsafe fn wide_text(p: *const u16) -> Option<String> {
    if p.is_null() {
        return None;
    }
    let mut n = 0;
    while *p.add(n) != 0 {
        n += 1;
    }
    Some(String::from_utf16_lossy(core::slice::from_raw_parts(p, n)))
}

unsafe fn hstring(s: &str) -> Result<*mut c_void, i32> {
    let w: Vec<u16> = s.encode_utf16().collect();
    let mut h = core::ptr::null_mut();
    let hr = WindowsCreateString(w.as_ptr(), w.len() as u32, &mut h);
    if hr < 0 { Err(hr) } else { Ok(h) }
}

pub fn package_known(pkg: Option<String>) -> bool {
    pkg.map(|p| p.eq_ignore_ascii_case(&compat_common::gdk::package::package_id())).unwrap_or(false)
}

#[repr(C)]
pub struct DownloadEstimate {
    pub bytes: u64,
    pub confirm: u32,
}

unsafe extern "system" fn v7_estimate(_this: *mut c_void, pkg: *const u16, _n: u32, _sel: *const c_void, out: *mut DownloadEstimate) -> i32 {
    if out.is_null() {
        return E_POINTER;
    }
    if !package_known(wide_text(pkg)) {
        return compat_common::gdk::E_NOT_FOUND;
    }
    *out = DownloadEstimate { bytes: 0, confirm: 0 };
    compat_common::record_raw("gamecore-host", "PackageV7", "slot 11 estimate download size: the package is fully installed, 0 bytes");
    S_OK
}

unsafe fn put_hstring(out: *mut *mut c_void, text: &[u16]) -> i32 {
    if text.is_empty() {
        *out = core::ptr::null_mut();
        return S_OK;
    }
    let mut h = core::ptr::null_mut();
    let hr = WindowsCreateString(text.as_ptr(), text.len() as u32, &mut h);
    if hr >= 0 {
        *out = h;
    }
    hr
}

pub unsafe fn hstring_units(h: *mut c_void) -> Vec<u16> {
    if h.is_null() {
        return Vec::new();
    }
    let mut n = 0u32;
    let p = WindowsGetStringRawBuffer(h, &mut n);
    if p.is_null() { Vec::new() } else { core::slice::from_raw_parts(p, n as usize).to_vec() }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

unsafe extern "system" fn v7_path(_this: *mut c_void, pkg: *const u16, _kind: i32, out: *mut *mut c_void) -> i32 {
    if out.is_null() {
        return E_POINTER;
    }
    *out = core::ptr::null_mut();
    if !package_known(wide_text(pkg)) {
        return compat_common::gdk::E_NOT_FOUND;
    }
    let path = compat_common::gdk::package::mount_path();
    compat_common::record_raw("gamecore-host", "PackageV7", &format!("slot 8 package path -> {path}"));
    put_hstring(out, &wide(&path))
}

unsafe extern "system" fn v7_locale(_this: *mut c_void, _pkg: *const u16, out: *mut *mut c_void) -> i32 {
    if out.is_null() {
        return E_POINTER;
    }
    *out = core::ptr::null_mut();
    put_hstring(out, &wide(&compat_common::gdk::package::user_locale()))
}

pub fn features_record(f: &[compat_common::gdk::package::Feature]) -> Vec<u16> {
    let mut v = Vec::new();
    for x in f {
        for field in [x.id.as_str(), x.display.as_str(), if x.hidden { "T" } else { "" }, x.tags.as_str(), x.store_id.as_str()] {
            v.extend(field.encode_utf16());
            v.push(0);
        }
    }
    v
}

unsafe extern "system" fn v7_features(_this: *mut c_void, pkg: *const u16, _kind: i32, out: *mut *mut c_void) -> i32 {
    if out.is_null() {
        return E_POINTER;
    }
    *out = core::ptr::null_mut();
    if !package_known(wide_text(pkg)) {
        return compat_common::gdk::E_NOT_FOUND;
    }
    let f = compat_common::gdk::package::features();
    compat_common::record_raw("gamecore-host", "PackageV7", &format!("slot 26 features -> {} records", f.len()));
    put_hstring(out, &features_record(f))
}

unsafe extern "system" fn v7_uninstall(_this: *mut c_void, pkg: *const u16) -> i32 {
    if !package_known(wide_text(pkg)) {
        return compat_common::gdk::E_NOT_FOUND;
    }
    compat_common::record_raw("gamecore-host", "PackageV7", "slot 27 uninstall of the running package refused: the installed package is the read-only game dump; E_ACCESSDENIED");
    E_ACCESSDENIED
}

#[repr(C)]
pub struct PkgDetails {
    pub id: *mut c_void,
    pub alias: *mut c_void,
    pub version: [u16; 4],
    pub kind: i32,
    pub flags: u32,
    pub display: *mut c_void,
    pub description: *mut c_void,
    pub publisher: *mut c_void,
    pub store_id: *mut c_void,
    pub alias4: *mut c_void,
}

unsafe fn fill_details(d: *mut PkgDetails) -> i32 {
    let t = compat_common::gdk::title();
    core::ptr::write_bytes(d as *mut u8, 0, core::mem::size_of::<PkgDetails>());
    let d = &mut *d;
    d.version = [t.version[3], t.version[2], t.version[1], t.version[0]];
    for (slot, text) in [(&mut d.id, compat_common::gdk::package::package_id()), (&mut d.display, t.display.clone()), (&mut d.publisher, t.publisher.clone())] {
        let hr = put_hstring(slot, &wide(&text));
        if hr < 0 {
            return hr;
        }
    }
    S_OK
}

unsafe extern "system" fn v7_details(_this: *mut c_void, pkg: *const u16, out: *mut PkgDetails) -> i32 {
    if out.is_null() {
        return E_POINTER;
    }
    if !package_known(wide_text(pkg)) {
        return compat_common::gdk::E_NOT_FOUND;
    }
    fill_details(out)
}

unsafe extern "system" fn v7_enumerate(_this: *mut c_void, _filter: *const u16, kind: i32, scope: i32, n: *mut u32, arr: *mut *mut PkgDetails) -> i32 {
    if n.is_null() || arr.is_null() {
        return E_POINTER;
    }
    *n = 0;
    *arr = core::ptr::null_mut();
    compat_common::record_raw("gamecore-host", "PackageV7", &format!("slot 28 enumerate kind={kind} scope={scope} -> {}", if kind == 0 { "this title" } else { "none" }));
    if kind != 0 {
        return S_OK;
    }
    let p = CoTaskMemAlloc(core::mem::size_of::<PkgDetails>()) as *mut PkgDetails;
    if p.is_null() {
        return 0x8007_000Eu32 as i32;
    }
    let hr = fill_details(p);
    if hr < 0 {
        return hr;
    }
    *arr = p;
    *n = 1;
    S_OK
}

static PACKAGE_V7_VT: Vt<PACKAGE_V7_SLOTS> = Vt([
    qi as *const (), add_ref as *const (), release as *const (),
    v7_3 as *const (), v7_4 as *const (), v7_5 as *const (), v7_6 as *const (), v7_7 as *const (), v7_path as *const (), v7_9 as *const (), v7_10 as *const (),
    v7_estimate as *const (), v7_12 as *const (), v7_13 as *const (), v7_14 as *const (), v7_locale as *const (), v7_16 as *const (), v7_17 as *const (), v7_18 as *const (),
    v7_19 as *const (), v7_20 as *const (), v7_21 as *const (), v7_22 as *const (), v7_23 as *const (), v7_24 as *const (), v7_25 as *const (),
    v7_features as *const (), v7_uninstall as *const (), v7_enumerate as *const (), v7_details as *const (), v7_30 as *const (), v7_31 as *const (),
]);
static PACKAGE_V7: Holder = Holder(Obj { vt: PACKAGE_V7_VT.0.as_ptr(), refs: &PACKAGE_V7_REFS, iid: IID_PACKAGE_V7 });

unsafe extern "system" fn net_query(_this: *mut c_void, url: *mut c_void, _a: u64, _b: u64, out: *mut *mut c_void) -> i32 {
    if out.is_null() {
        return E_POINTER;
    }
    let mut n = 0u32;
    let text = if url.is_null() { String::new() } else { String::from_utf16_lossy(core::slice::from_raw_parts(WindowsGetStringRawBuffer(url, &mut n), n as usize)) };
    compat_common::record_raw("gamecore-host", "NetSecurity", &format!("slot 6 '{text}' -> TLS1.2/1.3, no pinning (offline, same values as gdk net.rs)"));
    *out = obj(&NET_RESULT);
    S_OK
}

unsafe extern "system" fn net_result_flags(_this: *mut c_void, out: *mut [u32; 4]) -> i32 {
    if out.is_null() {
        return E_POINTER;
    }
    *out = [TLS_FLAGS, 0, 0, 0];
    S_OK
}

unsafe extern "system" fn net_result_list(_this: *mut c_void, count: *mut usize, items: *mut *mut c_void) -> i32 {
    if count.is_null() || items.is_null() {
        return E_POINTER;
    }
    *count = 0;
    *items = core::ptr::null_mut();
    S_OK
}

unsafe extern "system" fn net_result_bool(_this: *mut c_void, out: *mut u8) -> i32 {
    if out.is_null() {
        return E_POINTER;
    }
    *out = 0;
    S_OK
}

static NET_SECURITY_VT: Vt<7> = Vt([qi as *const (), add_ref as *const (), release as *const (), get_iids as *const (), get_runtime_class_name as *const (), get_trust_level as *const (), net_query as *const ()]);
static NET_SECURITY: Holder = Holder(Obj { vt: NET_SECURITY_VT.0.as_ptr(), refs: &NET_SECURITY_REFS, iid: IID_NET_SECURITY });
static NET_RESULT_VT: Vt<13> = Vt([
    qi as *const (), add_ref as *const (), release as *const (), get_iids as *const (), get_runtime_class_name as *const (), get_trust_level as *const (),
    net_result_flags as *const (), net_result_list as *const (), net_result_bool as *const (), net_result_list as *const (), net_result_list as *const (), net_result_bool as *const (), net_result_list as *const (),
]);
static NET_RESULT: Holder = Holder(Obj { vt: NET_RESULT_VT.0.as_ptr(), refs: &NET_RESULT_REFS, iid: IID_NET_SECURITY_RESULT });

pub fn server_refs() -> [u32; 3] {
    [&PACKAGE_V7_REFS, &NET_SECURITY_REFS, &NET_RESULT_REFS].map(|r| r.load(Ordering::SeqCst))
}
