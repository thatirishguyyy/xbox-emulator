use super::{os_build, state, tool_alloc, wide, E_NOTIMPL, E_OUTOFMEMORY, S_OK};
use core::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};

pub const S_FALSE: i32 = 1;
pub const E_POINTER: i32 = 0x8000_4003u32 as i32;
pub const E_FAIL: i32 = 0x8000_4005u32 as i32;
pub const E_UNEXPECTED: i32 = 0x8000_FFFFu32 as i32;
pub const E_PENDING: i32 = 0x8000_000Au32 as i32;
pub const E_ACCESSDENIED: i32 = 0x8007_0005u32 as i32;
pub const HR_CANCELLED: i32 = 0x8007_04C7u32 as i32;
pub const HR_BUSY: i32 = 0x8007_00AAu32 as i32;
pub const HR_WAIT_TIMEOUT: i32 = 0x8007_0102u32 as i32;

pub const SERVICE: u16 = 0x1080;
pub const TRANSPORT: [u32; 13] = [3, 0x1080, 0, 0x0400_0000, 2, 4, 0x8_0000, 4, 0x8_0000, 2, 4, 4, 0x20_0000];
pub const LISTEN_ARG: usize = 1;
pub const ACCEPT_MS: u32 = 0x32;
pub const CONTEXT_BYTES: usize = 0x678;
pub const SLOTS: usize = 15;
pub const TABLE_BYTES: usize = SLOTS * 8;
pub const PIX_DIR: &str = "D:\\PIX\\";
pub const THREAD_NAMES: [&str; 3] = [
    "PIX::InteractiveRuntime(device engine request)",
    "PIX::InteractiveRuntime(capture engine request)",
    "PIX::InteractiveRuntime(notification)",
];
pub const SCHEDULER_EVENTS: [&str; 3] = [
    "XBOX-PEVT-SCHEDULER-MONITOR-START-EVENT",
    "XBOX-PEVT-SCHEDULER-MONITOR-STOP-EVENT",
    "XBOX-PEVT-SCHEDULER-WORK-COMPLETED-EVENT",
];
pub const PROVIDER: [u8; 16] = [0xef, 0x69, 0x57, 0x60, 0x12, 0x24, 0xee, 0x42, 0xa5, 0x64, 0x72, 0x07, 0x25, 0x39, 0x44, 0x29];
pub const REPORT_COUNTER_EVENT: [u8; 16] = [0x64, 0, 0, 0, 0, 0x0a, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0];
pub const EVENT_LEVELS: [u8; 1] = [0];
pub const EVENT_KEYWORDS: [u64; 1] = [0];
pub const XMONDRV: &str = "\\\\.\\xmondrv";
pub const PMC_EVENT: &str = "Global\\XmonLibUserPMCConfigEvent";
pub const IOCTL_DF_PMC: u32 = 0x8fff_2098;
pub const IOCTL_L2I_PMC: u32 = 0x8fff_209c;
pub const IOCTL_PMC_APPLY: u32 = 0x8fff_2090;
pub const MSG_DF_PMC: &[u8] = b"Error installing DF PMC configuration.\n\0";
pub const MSG_L2I_PMC: &[u8] = b"Error installing L2I PMC configuration.\n\0";
pub const AFFINITY_WIDE: usize = 0x3000;
pub const AFFINITY_NARROW: usize = 0x60;
pub const AFFINITY_SPLIT: usize = 0x7f;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub build: &'static str,
    pub from_title: u32,
    pub singleton_bytes: usize,
    pub runtime_bytes: usize,
    pub runtime_ctor: u32,
    pub slots: [u32; SLOTS],
}

pub const LAYOUT_22000: Layout = Layout {
    build: "22000 (XDefiant, Terraria)",
    from_title: 0x234a0,
    singleton_bytes: 0x95b30,
    runtime_bytes: 0x1098,
    runtime_ctor: 0x18234,
    slots: [0x22d40, 0x22d60, 0x22540, 0x22e90, 0x22ed0, 0x22f20, 0x22ec0, 0x22570, 0x22f70, 0x230c0, 0x23250, 0x233e0, 0x23410, 0x23440, 0x23470],
};

pub const LAYOUT_22621: Layout = Layout {
    build: "22621 (Trackmania)",
    from_title: 0x283e0,
    singleton_bytes: 0x95ba8,
    runtime_bytes: 0x1130,
    runtime_ctor: 0x1c254,
    slots: [0x27c50, 0x27c70, 0x27440, 0x27dd0, 0x27e10, 0x27e60, 0x27e00, 0x27470, 0x27eb0, 0x28000, 0x28190, 0x28320, 0x28350, 0x28380, 0x283b0],
};

pub fn layout(os_build: u32) -> Option<Layout> {
    match os_build {
        22621.. => Some(LAYOUT_22621),
        22000.. => Some(LAYOUT_22000),
        _ => None,
    }
}

type Handle = usize;

#[repr(C)]
struct DataDesc {
    ptr: u64,
    size: u32,
    reserved: u32,
}

type EnableCallback = unsafe extern "system" fn(*const [u8; 16], u32, u8, u64, u64, *const c_void, *mut c_void);
type ThreadProc = unsafe extern "system" fn(*mut c_void) -> u32;

#[link(name = "kernel32")]
extern "system" {
    fn CreateEventW(sa: *mut c_void, manual: i32, initial: i32, name: *const u16) -> Handle;
    fn OpenEventW(access: u32, inherit: i32, name: *const u16) -> Handle;
    fn SetEvent(h: Handle) -> i32;
    fn WaitForSingleObject(h: Handle, ms: u32) -> u32;
    fn WaitForMultipleObjects(n: u32, h: *const Handle, all: i32, ms: u32) -> u32;
    fn CreateThread(sa: *mut c_void, stack: usize, f: ThreadProc, p: *mut c_void, flags: u32, id: *mut u32) -> Handle;
    fn SetThreadDescription(h: Handle, name: *const u16) -> i32;
    fn GetCurrentThread() -> Handle;
    fn GetCurrentProcess() -> Handle;
    fn GetProcessAffinityMask(p: Handle, proc_mask: *mut usize, sys_mask: *mut usize) -> i32;
    fn SetThreadAffinityMask(t: Handle, mask: usize) -> usize;
    fn GetExitCodeThread(t: Handle, code: *mut u32) -> i32;
    fn GetLastError() -> u32;
    fn QueryPerformanceCounter(c: *mut i64) -> i32;
    fn CreateFileW(n: *const u16, a: u32, s: u32, sa: *mut c_void, d: u32, f: u32, t: Handle) -> Handle;
    fn DeviceIoControl(h: Handle, code: u32, i: *mut c_void, il: u32, o: *mut c_void, ol: u32, ret: *mut u32, ov: *mut c_void) -> i32;
    fn OutputDebugStringW(s: *const u16);
    fn OutputDebugStringA(s: *const u8);
    fn GetModuleHandleW(n: *const u16) -> usize;
    fn LoadLibraryW(n: *const u16) -> usize;
    fn GetProcAddress(m: usize, n: *const u8) -> usize;
}

#[link(name = "ntdll")]
extern "system" {
    fn NtClose(h: Handle) -> i32;
}

#[link(name = "advapi32")]
extern "system" {
    fn EventRegister(id: *const [u8; 16], cb: EnableCallback, ctx: *mut c_void, reg: *mut u64) -> u32;
    fn EventWriteTransfer(reg: u64, desc: *const [u8; 16], activity: *const c_void, related: *const c_void, n: u32, data: *const DataDesc) -> u32;
}

fn hr_from_win32(e: u32) -> i32 {
    if e as i32 <= 0 {
        e as i32
    } else {
        ((e & 0xffff) | 0x8007_0000) as i32
    }
}

fn record(what: &str, detail: &str) {
    compat_common::record(env!("CARGO_CRATE_NAME"), what, detail);
}

#[derive(Clone, Copy)]
pub struct XbtpFns {
    pub init_system: unsafe extern "C" fn() -> i32,
    pub create: unsafe extern "C" fn() -> *mut u8,
    pub listen: unsafe extern "C" fn(*mut u8, *mut c_void, u16, *mut Handle) -> i32,
    pub accept: unsafe extern "C" fn(*mut u8, *mut *mut u8, u32) -> i32,
    pub close: unsafe extern "C" fn(*mut u8),
}

static XBTP: OnceLock<Option<XbtpFns>> = OnceLock::new();

pub fn bind_xbtp(f: XbtpFns) -> bool {
    XBTP.get_or_init(|| Some(f)).is_some_and(|g| g.create as usize == f.create as usize)
}

fn load_xbtp() -> Option<XbtpFns> {
    unsafe {
        let n = wide("xbtp.dll");
        let mut m = GetModuleHandleW(n.as_ptr());
        if m == 0 {
            m = LoadLibraryW(n.as_ptr());
        }
        if m == 0 {
            return None;
        }
        let p = |s: &[u8]| GetProcAddress(m, s.as_ptr());
        let (i, c, l, a, x) = (p(b"XbtpLinkInitializeSystem\0"), p(b"XbtpLinkCreateHandle\0"), p(b"XbtpLinkListen\0"), p(b"XbtpLinkAccept\0"), p(b"XbtpLinkCloseHandle\0"));
        if i == 0 || c == 0 || l == 0 || a == 0 || x == 0 {
            return None;
        }
        Some(XbtpFns {
            init_system: core::mem::transmute::<usize, unsafe extern "C" fn() -> i32>(i),
            create: core::mem::transmute::<usize, unsafe extern "C" fn() -> *mut u8>(c),
            listen: core::mem::transmute::<usize, unsafe extern "C" fn(*mut u8, *mut c_void, u16, *mut Handle) -> i32>(l),
            accept: core::mem::transmute::<usize, unsafe extern "C" fn(*mut u8, *mut *mut u8, u32) -> i32>(a),
            close: core::mem::transmute::<usize, unsafe extern "C" fn(*mut u8)>(x),
        })
    }
}

pub fn xbtp() -> Option<XbtpFns> {
    *XBTP.get_or_init(load_xbtp)
}

pub struct EnableState {
    pub enabled: AtomicU32,
    pub level: AtomicU32,
    pub any: AtomicU64,
    pub all: AtomicU64,
    pub bits: AtomicU32,
}

pub static PROVIDER_STATE: EnableState = EnableState { enabled: AtomicU32::new(0), level: AtomicU32::new(0), any: AtomicU64::new(0), all: AtomicU64::new(0), bits: AtomicU32::new(0) };

pub fn apply_enable(is_enabled: u32, level: u8, any: u64, all: u64) {
    let s = &PROVIDER_STATE;
    match is_enabled {
        0 => {
            s.enabled.store(0, Ordering::SeqCst);
            s.level.store(0, Ordering::SeqCst);
            s.any.store(0, Ordering::SeqCst);
            s.all.store(0, Ordering::SeqCst);
            s.bits.store(0, Ordering::SeqCst);
        }
        1 => {
            s.any.store(any, Ordering::SeqCst);
            s.all.store(all, Ordering::SeqCst);
            s.level.store(level as u32, Ordering::SeqCst);
            s.enabled.store(1, Ordering::SeqCst);
            for (i, (&lv, &kw)) in EVENT_LEVELS.iter().zip(EVENT_KEYWORDS.iter()).enumerate() {
                let on = !(lv > level && level != 0) && (kw == 0 || (any & kw != 0 && all & kw == all));
                if on {
                    s.bits.fetch_or(1 << i, Ordering::SeqCst);
                } else {
                    s.bits.fetch_and(!(1 << i), Ordering::SeqCst);
                }
            }
        }
        _ => {}
    }
}

unsafe extern "system" fn enable_callback(_id: *const [u8; 16], is_enabled: u32, level: u8, any: u64, all: u64, _f: *const c_void, ctx: *mut c_void) {
    if ctx.is_null() {
        return;
    }
    apply_enable(is_enabled, level, any, all);
}

pub struct Context {
    pub block: usize,
    pub provider: u64,
    pub register_status: u32,
    pub scheduler_events: [Handle; 3],
    pub pix_dir: Mutex<String>,
    pub magic: AtomicU32,
    pub version: AtomicU32,
    pub capture_if: AtomicUsize,
    pub capture_arg: AtomicUsize,
}

static CONTEXT: OnceLock<Context> = OnceLock::new();

pub fn context() -> &'static Context {
    CONTEXT.get_or_init(|| unsafe {
        let mut reg = 0u64;
        let st = EventRegister(&PROVIDER, enable_callback, &PROVIDER_STATE as *const EnableState as *mut c_void, &mut reg);
        let block = tool_alloc(CONTEXT_BYTES) as usize;
        let ev = SCHEDULER_EVENTS.map(|n| CreateEventW(core::ptr::null_mut(), 0, 0, wide(n).as_ptr()));
        Context {
            block,
            provider: reg,
            register_status: st,
            scheduler_events: ev,
            pix_dir: Mutex::new(String::new()),
            magic: AtomicU32::new(0),
            version: AtomicU32::new(0),
            capture_if: AtomicUsize::new(0),
            capture_arg: AtomicUsize::new(0),
        }
    })
}

pub fn set_capture_interface(p: usize, arg: usize) -> i32 {
    if p == 0 {
        return E_POINTER;
    }
    let c = context();
    c.capture_if.store(p, Ordering::SeqCst);
    c.capture_arg.store(arg, Ordering::SeqCst);
    S_OK
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn pix_host_dir() -> Option<std::path::PathBuf> {
    console_sandbox::fs::try_resolve(PIX_DIR).ok().map(|(_, p)| p)
}

fn create_pix_dir() -> bool {
    let _ = std::fs::create_dir_all(console_sandbox::fs::drive_dir('D'));
    match pix_host_dir() {
        Some(p) => std::fs::create_dir(&p).is_ok(),
        None => false,
    }
}

pub fn affinity_mask(process_mask: usize) -> usize {
    if process_mask > AFFINITY_SPLIT {
        AFFINITY_WIDE
    } else {
        AFFINITY_NARROW
    }
}

unsafe fn pin_thread() -> usize {
    let (mut p, mut s) = (0usize, 0usize);
    let ok = GetProcessAffinityMask(GetCurrentProcess(), &mut p, &mut s) != 0;
    if !ok {
        record("thread affinity", "PixRT::GetMinimalTitleImpactProcessorAffinityMask - GetProcessAffinityMask call failed");
    }
    let m = affinity_mask(p);
    SetThreadAffinityMask(GetCurrentThread(), m);
    m
}

struct Listener {
    service: u16,
    handle: usize,
    ready: Handle,
    refs: u32,
}

static LISTENERS: Mutex<Vec<Listener>> = Mutex::new(Vec::new());
static SYSTEM_REFS: Mutex<u32> = Mutex::new(0);

static REF_MIRROR: Mutex<Vec<(u16, u32)>> = Mutex::new(Vec::new());

fn mirror_refs(ls: &[Listener]) {
    let mut m = lock(&REF_MIRROR);
    m.clear();
    m.extend(ls.iter().map(|l| (l.service, l.refs)));
}

pub fn listener_refs(service: u16) -> u32 {
    lock(&REF_MIRROR).iter().filter(|l| l.0 == service).map(|l| l.1).sum()
}

struct XbtpLink {
    cancel: Handle,
    listener: usize,
    ready: Handle,
    accepted: usize,
}

pub struct Transport {
    cs: Mutex<()>,
    cancelled: AtomicU32,
    connected: AtomicU32,
    mode: AtomicU32,
    kind: AtomicU32,
    link: Mutex<Option<XbtpLink>>,
    pub accepted: AtomicUsize,
}

impl Transport {
    const fn new() -> Transport {
        Transport { cs: Mutex::new(()), cancelled: AtomicU32::new(0), connected: AtomicU32::new(0), mode: AtomicU32::new(0), kind: AtomicU32::new(0), link: Mutex::new(None), accepted: AtomicUsize::new(0) }
    }

    pub fn cancel_event(&self) -> Handle {
        lock(&self.link).as_ref().map(|l| l.cancel).unwrap_or(0)
    }
}

unsafe fn xbtp_init_system(x: &XbtpFns) -> i32 {
    let mut n = lock(&SYSTEM_REFS);
    if *n == 0 {
        let hr = (x.init_system)();
        if hr < 0 {
            return hr;
        }
    }
    *n += 1;
    S_OK
}

unsafe fn xbtp_listen_accept(x: &XbtpFns, link: &mut XbtpLink, service: u16, timeout: u32) -> i32 {
    let mut ls = lock(&LISTENERS);
    if let Some(l) = ls.iter_mut().find(|l| l.service == service) {
        l.refs += 1;
        link.listener = l.handle;
        link.ready = l.ready;
    }
    if link.listener == 0 {
        let h = (x.create)();
        if h.is_null() {
            return E_FAIL;
        }
        let mut ready: Handle = 0;
        let hr = (x.listen)(h, LISTEN_ARG as *mut c_void, service, &mut ready);
        if hr < 0 {
            (x.close)(h);
            return hr;
        }
        link.listener = h as usize;
        link.ready = ready;
        ls.push(Listener { service, handle: h as usize, ready, refs: 1 });
    }
    mirror_refs(&ls);
    let w = [link.ready, link.cancel];
    let ms = if timeout == 0 { u32::MAX } else { timeout };
    match WaitForMultipleObjects(2, w.as_ptr(), 0, ms) {
        0 => {
            let mut acc: *mut u8 = core::ptr::null_mut();
            let hr = (x.accept)(link.listener as *mut u8, &mut acc, ACCEPT_MS);
            if hr >= 0 {
                link.accepted = acc as usize;
            }
            hr
        }
        1 => HR_CANCELLED,
        0x102 => HR_WAIT_TIMEOUT,
        _ => E_FAIL,
    }
}

unsafe fn connect(t: &Transport) -> i32 {
    let g = lock(&t.cs);
    if t.cancelled.load(Ordering::SeqCst) != 0 {
        return HR_CANCELLED;
    }
    if t.connected.load(Ordering::SeqCst) != 0 || lock(&t.link).is_some() {
        return HR_BUSY;
    }
    t.mode.store(1, Ordering::SeqCst);
    t.kind.store(TRANSPORT[0], Ordering::SeqCst);
    match TRANSPORT[0] {
        1 => return E_NOTIMPL,
        3 => {}
        _ => {
            record("device engine", "transport type other than Xbtp (3) is not in the console config 0x51c88");
            return E_NOTIMPL;
        }
    }
    let Some(x) = xbtp() else {
        record("device engine", "xbtp.dll (XbtpLinkInitializeSystem/CreateHandle/Listen/Accept/CloseHandle) could not be resolved");
        return E_FAIL;
    };
    let mut q = 0i64;
    QueryPerformanceCounter(&mut q);
    let hr = xbtp_init_system(&x);
    if hr < 0 {
        return hr;
    }
    let cancel = CreateEventW(core::ptr::null_mut(), 0, 0, core::ptr::null());
    *lock(&t.link) = Some(XbtpLink { cancel, listener: 0, ready: 0, accepted: 0 });
    drop(g);
    QueryPerformanceCounter(&mut q);
    let mut link = XbtpLink { cancel, listener: 0, ready: 0, accepted: 0 };
    let hr = xbtp_listen_accept(&x, &mut link, TRANSPORT[1] as u16, TRANSPORT[2]);
    let _g = lock(&t.cs);
    if hr < 0 {
        *lock(&t.link) = None;
        return hr;
    }
    t.accepted.store(link.accepted, Ordering::SeqCst);
    record(
        "device engine",
        "a PIX tool connected on xbtp service 0x1080; the session setup 0x3a790 and the device-engine request dispatch 0x1b4e7 are not ported, so the connection is closed",
    );
    (x.close)(link.accepted as *mut u8);
    *lock(&t.link) = None;
    E_NOTIMPL
}

pub struct Runtime {
    pub block: usize,
    pub magic: u32,
    pub version: u32,
    pub flag: bool,
    pub connected: Handle,
    pub shutdown: Handle,
    pub notify: Handle,
    pub threads: [AtomicUsize; 3],
    pub affinity: [AtomicUsize; 3],
    pub started: [AtomicBool; 3],
    pub device: Transport,
    pub capture: Transport,
    pub device_state: AtomicI32,
    pub capture_state: AtomicI32,
    pub pix_dir_created: bool,
}

unsafe fn park(rt: &Runtime) {
    WaitForSingleObject(rt.shutdown, u32::MAX);
}

unsafe extern "system" fn device_thread(p: *mut c_void) -> u32 {
    let rt = &*(p as *const Runtime);
    rt.affinity[0].store(pin_thread(), Ordering::SeqCst);
    rt.started[0].store(true, Ordering::SeqCst);
    loop {
        if WaitForSingleObject(rt.shutdown, 0) == 0 {
            return 0;
        }
        let hr = connect(&rt.device);
        if hr >= 0 {
            SetEvent(rt.connected);
            continue;
        }
        if WaitForSingleObject(rt.shutdown, 0) == 0 {
            return 0;
        }
        record("device engine", &format!("connect failed {hr:#x}; the console's xbtplink makes this transient and retries at 0x1b405, the emulator parks the thread on the shutdown event instead of spinning"));
        park(rt);
        return 0;
    }
}

unsafe extern "system" fn capture_thread(p: *mut c_void) -> u32 {
    let rt = &*(p as *const Runtime);
    rt.affinity[1].store(pin_thread(), Ordering::SeqCst);
    rt.started[1].store(true, Ordering::SeqCst);
    let w = [rt.shutdown, rt.connected];
    if WaitForMultipleObjects(2, w.as_ptr(), 0, u32::MAX) == 0 {
        return 0;
    }
    record("capture engine", "device engine connected; the capture-engine transport and request dispatch (0x1b9bd onward) are not ported");
    park(rt);
    0
}

unsafe extern "system" fn notification_thread(p: *mut c_void) -> u32 {
    let rt = &*(p as *const Runtime);
    rt.affinity[2].store(pin_thread(), Ordering::SeqCst);
    rt.started[2].store(true, Ordering::SeqCst);
    let w = [rt.notify, rt.shutdown];
    while WaitForMultipleObjects(2, w.as_ptr(), 0, u32::MAX) == 0 {
        record("notification", "notification event signalled; the notification send 0x1ad0c is not ported");
    }
    0
}

unsafe fn runtime_new(layout: &Layout, magic: u32, version: u32, flags: u32) -> Option<&'static Runtime> {
    let block = tool_alloc(layout.runtime_bytes) as usize;
    if block == 0 {
        return None;
    }
    let connected = CreateEventW(core::ptr::null_mut(), 1, 0, core::ptr::null());
    let pix_dir_created = create_pix_dir();
    let c = context();
    *lock(&c.pix_dir) = PIX_DIR.to_string();
    c.magic.store(magic, Ordering::SeqCst);
    c.version.store(version, Ordering::SeqCst);
    let shutdown = CreateEventW(core::ptr::null_mut(), 1, 0, core::ptr::null());
    let notify = CreateEventW(core::ptr::null_mut(), 0, 0, core::ptr::null());
    let rt: &'static Runtime = Box::leak(Box::new(Runtime {
        block,
        magic,
        version,
        flag: flags & 1 != 0,
        connected,
        shutdown,
        notify,
        threads: [const { AtomicUsize::new(0) }; 3],
        affinity: [const { AtomicUsize::new(0) }; 3],
        started: [const { AtomicBool::new(false) }; 3],
        device: Transport::new(),
        capture: Transport::new(),
        device_state: AtomicI32::new(0),
        capture_state: AtomicI32::new(0),
        pix_dir_created,
    }));
    set_capture_interface(rt as *const Runtime as usize + 8, 0);
    let procs: [ThreadProc; 3] = [device_thread, capture_thread, notification_thread];
    for (i, f) in procs.into_iter().enumerate() {
        let h = CreateThread(core::ptr::null_mut(), 0, f, rt as *const Runtime as *mut c_void, 0, core::ptr::null_mut());
        rt.threads[i].store(h, Ordering::SeqCst);
        if h != 0 {
            SetThreadDescription(h, wide(THREAD_NAMES[i]).as_ptr());
        }
    }
    Some(rt)
}

pub struct Singleton {
    pub block: usize,
    pub layout: Layout,
    runtime: Mutex<Option<&'static Runtime>>,
    constructing: AtomicBool,
    pub capturing: AtomicBool,
}

static INIT: Mutex<Option<&'static Singleton>> = Mutex::new(None);

pub fn singleton() -> Option<&'static Singleton> {
    *lock(&INIT)
}

pub fn runtime() -> Option<&'static Runtime> {
    singleton().and_then(|s| *lock(&s.runtime))
}

unsafe fn ensure_runtime(s: &Singleton, magic: u32, version: u32, flags: u32) -> i32 {
    let mut r = lock(&s.runtime);
    if r.is_none() && !s.constructing.load(Ordering::SeqCst) {
        s.constructing.store(true, Ordering::SeqCst);
        *r = runtime_new(&s.layout, magic, version, flags);
        s.constructing.store(false, Ordering::SeqCst);
    }
    S_OK
}

pub unsafe extern "system" fn from_title(magic: u32, version: u32, flags: u32, table: *mut c_void, bytes: u32) -> i32 {
    let Some(layout) = layout(os_build()) else {
        record("PixDllInitializeFromTitle", "this console build's pixrt10 (19041: 0xae8-byte singleton, two-thread runtime, d:\\PIX_ForceTCP) is not ported");
        return E_NOTIMPL;
    };
    context();
    let hr = {
        let mut g = lock(&INIT);
        if g.is_none() {
            let block = tool_alloc(layout.singleton_bytes) as usize;
            if block == 0 {
                return E_OUTOFMEMORY;
            }
            *g = Some(Box::leak(Box::new(Singleton { block, layout, runtime: Mutex::new(None), constructing: AtomicBool::new(false), capturing: AtomicBool::new(false) })));
        }
        ensure_runtime(g.unwrap(), magic, version, flags)
    };
    if hr < 0 {
        return hr;
    }
    copy_slots(table, bytes);
    hr
}

pub fn title_table() -> [usize; SLOTS] {
    [
        begin_capture as *const () as usize,
        end_capture as *const () as usize,
        capture_flags as *const () as usize,
        report_counter as *const () as usize,
        set_capture_handler as *const () as usize,
        clear_capture_handler as *const () as usize,
        capture_state as *const () as usize,
        thread_event_buffer as *const () as usize,
        slot8 as *const () as usize,
        install_df_pmc as *const () as usize,
        install_l2i_pmc as *const () as usize,
        set_word_14 as *const () as usize,
        set_word_12 as *const () as usize,
        set_word_10 as *const () as usize,
        set_word_16 as *const () as usize,
    ]
}

unsafe fn copy_slots(dst: *mut c_void, bytes: u32) {
    if dst.is_null() {
        return;
    }
    let t = title_table();
    let n = (bytes as usize).min(TABLE_BYTES);
    core::ptr::copy_nonoverlapping(t.as_ptr() as *const u8, dst as *mut u8, n);
}

#[repr(C)]
pub struct CaptureHandler {
    pub kind: u32,
    pub reserved: [u32; 3],
    pub ctx: usize,
    pub begin: unsafe extern "system" fn(usize, *const c_void) -> i32,
    pub end: unsafe extern "system" fn(usize) -> i32,
}

pub struct CaptureGlobals {
    pub state: AtomicU32,
    pub end_arg: AtomicU32,
    pub handler: AtomicUsize,
    pub pending_if: AtomicUsize,
}

pub static CAPTURE: CaptureGlobals = CaptureGlobals { state: AtomicU32::new(0), end_arg: AtomicU32::new(0), handler: AtomicUsize::new(0), pending_if: AtomicUsize::new(0) };
static CAPTURE_CS: Mutex<()> = Mutex::new(());
pub static BACKEND_STATUS: AtomicU32 = AtomicU32::new(0);

pub fn flags() -> u32 {
    let w = &state().words;
    let b = |i: usize| (w[i].load(Ordering::SeqCst) & 1) as u32;
    let mut r = b(5);
    r = (r << 1) | b(4);
    r = (r << 1) | b(3);
    r = (r << 1) | b(2);
    r = (r << 1) | b(1);
    (r << 1) | (w[0].load(Ordering::SeqCst) & 0x8001) as u32
}

pub unsafe extern "system" fn begin_capture(kind: u32, desc: *const c_void) -> i32 {
    if singleton().is_none() {
        return E_UNEXPECTED;
    }
    let _g = lock(&CAPTURE_CS);
    if desc.is_null() {
        return E_POINTER;
    }
    if CAPTURE.state.load(Ordering::SeqCst) != 0 || flags() != 0 {
        return E_PENDING;
    }
    if kind == 1 {
        record("begin capture", "timing capture (type 1: 0x32601-0x32c8b, default backend 0x5f1d0 slot 3 0x27500, ETW .pevt writer) is not ported");
        return E_NOTIMPL;
    }
    let h = CAPTURE.handler.load(Ordering::SeqCst) as *const CaptureHandler;
    let hr = if !h.is_null() {
        if (*h).kind != kind {
            CAPTURE.state.store(1, Ordering::SeqCst);
            return S_OK;
        }
        ((*h).begin)((*h).ctx, desc);
        S_OK
    } else {
        S_FALSE
    };
    CAPTURE.state.store(1, Ordering::SeqCst);
    hr
}

pub unsafe extern "system" fn end_capture(arg: u32) -> i32 {
    CAPTURE.end_arg.store(arg, Ordering::SeqCst);
    CAPTURE.state.store(2, Ordering::SeqCst);
    if flags() & 1 != 0 {
        record("end capture", "timing capture active (word +0xc bit 0): the end-capture worker 0x176c0 is not ported");
        return E_NOTIMPL;
    }
    let mut code = 0u32;
    let hr = if GetExitCodeThread(0, &mut code) != 0 { code as i32 } else { hr_from_win32(GetLastError()) };
    let _g = lock(&CAPTURE_CS);
    CAPTURE.state.store(0, Ordering::SeqCst);
    let h = CAPTURE.handler.load(Ordering::SeqCst) as *const CaptureHandler;
    if !h.is_null() {
        ((*h).end)((*h).ctx);
    }
    if CAPTURE.pending_if.swap(0, Ordering::SeqCst) != 0 {
        set_capture_interface(0, 0);
    }
    if hr >= 0 {
        return BACKEND_STATUS.load(Ordering::SeqCst) as i32;
    }
    hr
}

pub unsafe extern "system" fn capture_flags() -> u32 {
    flags()
}

pub unsafe extern "system" fn report_counter(name: *const u16, value: f32) {
    if PROVIDER_STATE.bits.load(Ordering::SeqCst) & 1 == 0 {
        return;
    }
    let c = context();
    let null: Vec<u16> = wide("NULL");
    let (p, n) = if name.is_null() {
        (null.as_ptr(), 10u32)
    } else {
        let mut k = 0usize;
        while *name.add(k) != 0 {
            k += 1;
        }
        (name, (k * 2 + 2) as u32)
    };
    let v = value;
    let data = [
        DataDesc { ptr: 0, size: 0, reserved: 0 },
        DataDesc { ptr: &v as *const f32 as u64, size: 4, reserved: 0 },
        DataDesc { ptr: p as u64, size: n, reserved: 0 },
    ];
    EventWriteTransfer(c.provider, &REPORT_COUNTER_EVENT, core::ptr::null(), core::ptr::null(), 3, data.as_ptr());
}

pub unsafe extern "system" fn set_capture_handler(h: *const CaptureHandler) {
    let _g = lock(&CAPTURE_CS);
    if CAPTURE.handler.load(Ordering::SeqCst) == 0 {
        CAPTURE.handler.store(h as usize, Ordering::SeqCst);
    }
}

pub unsafe extern "system" fn clear_capture_handler(kind: u32) {
    let _g = lock(&CAPTURE_CS);
    let h = CAPTURE.handler.load(Ordering::SeqCst) as *const CaptureHandler;
    if !h.is_null() && (*h).kind == kind {
        CAPTURE.handler.store(0, Ordering::SeqCst);
    }
}

pub unsafe extern "system" fn capture_state() -> u32 {
    CAPTURE.state.load(Ordering::SeqCst)
}

pub unsafe extern "system" fn thread_event_buffer(_rdtsc_once: u8) -> u64 {
    record("thread event buffer", "slot 7 (0x22570 -> 0x2cf30: per-thread CPU event buffer flush to context+0x468 and refill via 0x2c9dc) is not ported");
    0
}

pub unsafe extern "system" fn slot8(_a: u32, _b: u32, _c: u32, _d: u32) -> i32 {
    record("slot 8", "slot 8 (0x22f70: limit check on 0x5f994, 0x44dd0/0x45764/0x453a4) is not ported");
    E_NOTIMPL
}

static PMC_EVENT_HANDLE: AtomicUsize = AtomicUsize::new(0);

unsafe fn pmc_event() -> Handle {
    let n = wide(PMC_EVENT);
    let mut h = OpenEventW(0x0010_0002, 0, n.as_ptr());
    if h == 0 {
        h = CreateEventW(core::ptr::null_mut(), 1, 0, n.as_ptr());
    }
    PMC_EVENT_HANDLE.store(h, Ordering::SeqCst);
    h
}

unsafe fn open_xmon() -> Handle {
    let h = CreateFileW(wide(XMONDRV).as_ptr(), 0xC000_0000, 0, core::ptr::null_mut(), 3, 0, 0);
    if h == usize::MAX {
        let e = GetLastError();
        if e != 2 {
            OutputDebugStringW(wide(&format!("XMon DLL: !!!Error 0x{e:x} when opening XMon driver\n")).as_ptr());
        }
    }
    h
}

unsafe fn install_pmc(code: u32, msg: &[u8], a: u32, b: u32, c: u32, d: u32) -> i32 {
    let h = open_xmon();
    if h == 0 {
        OutputDebugStringA(msg.as_ptr());
        return E_ACCESSDENIED;
    }
    let mut inb: [u32; 9] = [1, 0, 0, 0, 0, a & 0xffff, b & 0xffff, c & 0xffff, d & 0xffff];
    let mut out = 0u64;
    let mut ret = 0u32;
    let ok = DeviceIoControl(h, code, inb.as_mut_ptr() as *mut c_void, 0x24, &mut out as *mut u64 as *mut c_void, 8, &mut ret, core::ptr::null_mut()) != 0;
    let hr = if ok { S_OK } else { E_ACCESSDENIED };
    let mut ev = PMC_EVENT_HANDLE.load(Ordering::SeqCst);
    if ev == 0 {
        ev = pmc_event();
    }
    if ev != 0 {
        SetEvent(ev);
    }
    let ok2 = DeviceIoControl(h, IOCTL_PMC_APPLY, core::ptr::null_mut(), 0, &mut out as *mut u64 as *mut c_void, 8, &mut ret, core::ptr::null_mut()) != 0;
    if h != usize::MAX {
        NtClose(h);
    }
    if ok2 {
        hr
    } else {
        E_ACCESSDENIED
    }
}

pub unsafe extern "system" fn install_df_pmc(a: u32, b: u32, c: u32, d: u32) -> i32 {
    install_pmc(IOCTL_DF_PMC, MSG_DF_PMC, a, b, c, d)
}

pub unsafe extern "system" fn install_l2i_pmc(a: u32, b: u32, c: u32, d: u32) -> i32 {
    install_pmc(IOCTL_L2I_PMC, MSG_L2I_PMC, a, b, c, d)
}

fn set_word(i: usize, v: u16) -> i32 {
    state().words[i].swap(v, Ordering::SeqCst);
    S_OK
}

pub unsafe extern "system" fn set_word_14(v: u16) -> i32 {
    set_word(4, v)
}

pub unsafe extern "system" fn set_word_12(v: u16) -> i32 {
    set_word(3, v)
}

pub unsafe extern "system" fn set_word_10(v: u16) -> i32 {
    set_word(2, v)
}

pub unsafe extern "system" fn set_word_16(v: u16) -> i32 {
    set_word(5, v)
}
