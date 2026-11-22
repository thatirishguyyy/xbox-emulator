use core::ffi::c_void;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

pub mod access;
pub mod basic;
pub mod emureg;
pub mod capture;
pub mod events;
pub mod fs;
pub mod hook;
pub mod late;
pub mod launcher;
pub mod identity;
pub mod media;
pub mod memstatus;
pub mod mediacontrol;
pub mod net;
pub mod rt;
pub mod social;
pub mod webauth;
pub mod win;
pub mod package;
pub mod save;
pub mod store;
pub mod streaming;
pub mod system;
pub mod ui;
pub mod user;
pub mod xasync;
pub mod xsign;
pub mod xcrd_paths;

pub const S_OK: i32 = 0;
pub const E_NOTIMPL: i32 = 0x8000_4001u32 as i32;
pub const E_NOINTERFACE: i32 = 0x8000_4002u32 as i32;
pub const E_ABORT: i32 = 0x8000_4004u32 as i32;
pub const E_FAIL: i32 = 0x8000_4005u32 as i32;
pub const E_PENDING: i32 = 0x8000_000Au32 as i32;
pub const E_INVALIDARG: i32 = 0x8007_0057u32 as i32;
pub const E_NOT_SUFFICIENT_BUFFER: i32 = 0x8007_007Au32 as i32;
pub const E_NOT_FOUND: i32 = 0x8007_0490u32 as i32;

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct Guid {
    pub d1: u32,
    pub d2: u16,
    pub d3: u16,
    pub d4: [u8; 8],
}

const fn hexv(c: u8) -> u64 {
    match c {
        b'0'..=b'9' => (c - b'0') as u64,
        b'a'..=b'f' => (c - b'a' + 10) as u64,
        b'A'..=b'F' => (c - b'A' + 10) as u64,
        _ => panic!("bad guid digit"),
    }
}

const fn hexn(s: &[u8], at: usize, n: usize) -> u64 {
    let mut v = 0u64;
    let mut i = 0;
    while i < n {
        v = (v << 4) | hexv(s[at + i]);
        i += 1;
    }
    v
}

impl Guid {
    pub const fn parse(s: &str) -> Guid {
        let b = s.as_bytes();
        let mut d4 = [0u8; 8];
        d4[0] = hexn(b, 19, 2) as u8;
        d4[1] = hexn(b, 21, 2) as u8;
        let mut i = 0;
        while i < 6 {
            d4[2 + i] = hexn(b, 24 + i * 2, 2) as u8;
            i += 1;
        }
        Guid { d1: hexn(b, 0, 8) as u32, d2: hexn(b, 9, 4) as u16, d3: hexn(b, 14, 4) as u16, d4 }
    }

    pub fn text(&self) -> String {
        let d = &self.d4;
        format!(
            "{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            self.d1, self.d2, self.d3, d[0], d[1], d[2], d[3], d[4], d[5], d[6], d[7]
        )
    }
}

pub type This = *const Obj;

#[repr(C)]
pub struct Obj {
    vtbl: *const usize,
    name: &'static str,
}

pub struct Iface {
    pub clsid: Guid,
    pub iid: Guid,
    pub name: &'static str,
    pub slots: Vec<(usize, usize)>,
}

pub const FORWARDED: &[(&str, Guid)] = &[];

pub const VTABLE_LEN: usize = 128;

struct Entry {
    clsid: Guid,
    iid: Guid,
    name: &'static str,
    obj: usize,
    slots: Vec<usize>,
}

fn all_ifaces() -> Vec<Iface> {
    let mut v = Vec::new();
    v.extend(user::interfaces());
    v.extend(store::interfaces());
    v.extend(save::interfaces());
    v.extend(package::interfaces());
    v.extend(ui::interfaces());
    v.extend(net::interfaces());
    v.extend(system::interfaces());
    v.extend(capture::interfaces());
    v.extend(access::interfaces());
    v.extend(basic::interfaces());
    v.extend(streaming::interfaces());
    v
}

fn build(i: Iface, stubs: &[usize; VTABLE_LEN]) -> Entry {
    let mut vt: Vec<usize> = stubs.to_vec();
    vt[0] = qi as usize;
    vt[1] = add_ref as usize;
    vt[2] = release as usize;
    let mut slots = Vec::new();
    for &(slot, f) in &i.slots {
        vt[slot] = f;
        slots.push(slot);
    }
    slots.sort_unstable();
    let vt: &'static [usize] = Box::leak(vt.into_boxed_slice());
    let obj = Box::leak(Box::new(Obj { vtbl: vt.as_ptr(), name: i.name })) as *const Obj as usize;
    Entry { clsid: i.clsid, iid: i.iid, name: i.name, obj, slots }
}

fn registry() -> &'static Vec<Entry> {
    static R: OnceLock<Vec<Entry>> = OnceLock::new();
    R.get_or_init(|| {
        let stubs = stub_table();
        all_ifaces().into_iter().map(|i| build(i, &stubs)).collect()
    })
}

fn fallback_registry() -> &'static Vec<Entry> {
    static R: OnceLock<Vec<Entry>> = OnceLock::new();
    R.get_or_init(|| {
        let stubs = stub_table();
        rt::interfaces().into_iter().map(|i| build(i, &stubs)).collect()
    })
}

pub fn implemented() -> Vec<(Guid, Guid, &'static str, Vec<usize>)> {
    registry().iter().chain(fallback_registry().iter()).map(|e| (e.clsid, e.iid, e.name, e.slots.clone())).collect()
}

pub fn slot_functions() -> Vec<(Guid, Guid, &'static str, usize, usize)> {
    let mut out = Vec::new();
    for e in registry().iter().chain(fallback_registry().iter()) {
        let vt = unsafe { (*(e.obj as *const Obj)).vtbl };
        for &s in &e.slots {
            out.push((e.clsid, e.iid, e.name, s, unsafe { *vt.add(s) }));
        }
    }
    out
}

unsafe extern "system" fn qi(_t: This, _iid: *const Guid, out: *mut *mut c_void) -> i32 {
    if !out.is_null() {
        *out = core::ptr::null_mut();
    }
    E_NOINTERFACE
}

unsafe extern "system" fn add_ref(_t: This) -> u32 {
    1
}

unsafe extern "system" fn release(_t: This) -> u32 {
    1
}

unsafe extern "system" fn stub<const N: usize>(this: This) -> i32 {
    let name = if this.is_null() { "?" } else { (*this).name };
    log("unconfirmed-slot", &format!("{name} slot {N} called -> E_NOTIMPL"));
    E_NOTIMPL
}

macro_rules! stubs {
    ($($n:literal)*) => { [$(stub::<$n> as usize),*] };
}

fn stub_table() -> [usize; VTABLE_LEN] {
    stubs!(
        0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 27 28 29 30 31
        32 33 34 35 36 37 38 39 40 41 42 43 44 45 46 47 48 49 50 51 52 53 54 55 56 57 58 59 60 61 62 63
        64 65 66 67 68 69 70 71 72 73 74 75 76 77 78 79 80 81 82 83 84 85 86 87 88 89 90 91 92 93 94 95
        96 97 98 99 100 101 102 103 104 105 106 107 108 109 110 111 112 113 114 115 116 117 118 119 120 121 122 123 124 125 126 127
    )
}

pub type QueryFn = unsafe extern "system" fn(*const Guid, *const Guid, *mut *mut c_void) -> i32;

pub(crate) static REAL_QUERY: AtomicUsize = AtomicUsize::new(0);

pub fn set_real_query(f: usize) {
    REAL_QUERY.store(f, Ordering::SeqCst);
}

pub fn real_query() -> Option<QueryFn> {
    match REAL_QUERY.load(Ordering::SeqCst) {
        0 => None,
        f => Some(unsafe { core::mem::transmute::<usize, QueryFn>(f) }),
    }
}

fn first_time(c: Guid, i: Guid) -> bool {
    static SEEN: OnceLock<Mutex<HashSet<(Guid, Guid)>>> = OnceLock::new();
    SEEN.get_or_init(|| Mutex::new(HashSet::new())).lock().map(|mut s| s.insert((c, i))).unwrap_or(false)
}

pub unsafe extern "system" fn query_api_impl(clsid: *const Guid, iid: *const Guid, out: *mut *mut c_void) -> i32 {
    if clsid.is_null() || iid.is_null() || out.is_null() {
        return E_INVALIDARG;
    }
    let (c, i) = (*clsid, *iid);
    if let Some(e) = registry().iter().find(|e| e.clsid == c && e.iid == i) {
        *out = e.obj as *mut c_void;
        if first_time(c, i) {
            log("QueryApiImpl", &format!("{} {} -> emulator {}", c.text(), i.text(), e.name));
            if c == capture::CAPTURE {
                capture::start_history();
            }
        }
        return S_OK;
    }
    if c == xasync::XASYNC && rt::active() {
        if let Some(e) = fallback_registry().iter().find(|e| e.clsid == c && e.iid == i) {
            *out = e.obj as *mut c_void;
            if first_time(c, i) {
                log("QueryApiImpl", &format!("{} {} -> emulated {} (real XAsync unavailable or gdk_xasync=emulated)", c.text(), i.text(), e.name));
            }
            return S_OK;
        }
    }
    match real_query() {
        Some(f) => {
            let hr = f(clsid, iid, out);
            if first_time(c, i) {
                log("QueryApiImpl", &format!("{} {} -> real runtime hr={hr:#010x}", c.text(), i.text()));
            }
            hr
        }
        None => {
            *out = core::ptr::null_mut();
            if first_time(c, i) {
                log("QueryApiImpl", &format!("{} {} -> no real runtime, E_NOINTERFACE", c.text(), i.text()));
            }
            E_NOINTERFACE
        }
    }
}

pub fn log(func: &str, detail: &str) {
    static N: AtomicUsize = AtomicUsize::new(0);
    static LIMIT: OnceLock<usize> = OnceLock::new();
    let limit = *LIMIT.get_or_init(|| setting("gdk_log_limit").and_then(|v| v.parse().ok()).unwrap_or(400));
    let n = N.fetch_add(1, Ordering::Relaxed);
    if n < limit || n % 1000 == 0 {
        crate::record_raw("gdk", func, detail);
    }
}

pub fn setting(key: &str) -> Option<String> {
    let env = format!("TOMBOX_{}", key.to_ascii_uppercase());
    std::env::var(env).ok().or_else(|| crate::cfg(key)).map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

pub fn next_token() -> u64 {
    static T: AtomicU64 = AtomicU64::new(1);
    T.fetch_add(1, Ordering::Relaxed)
}

pub unsafe fn cstr(p: *const u8) -> Option<String> {
    if p.is_null() {
        return None;
    }
    let mut n = 0;
    while n < 65536 && *p.add(n) != 0 {
        n += 1;
    }
    Some(String::from_utf8_lossy(core::slice::from_raw_parts(p, n)).into_owned())
}

pub unsafe fn wstr(p: *const u16) -> Option<String> {
    if p.is_null() {
        return None;
    }
    let mut n = 0;
    while n < 65536 && *p.add(n) != 0 {
        n += 1;
    }
    Some(String::from_utf16_lossy(core::slice::from_raw_parts(p, n)))
}

pub unsafe fn put_str(s: &str, size: usize, buf: *mut u8, used: *mut usize) -> i32 {
    let need = s.len() + 1;
    if !used.is_null() {
        *used = need;
    }
    if buf.is_null() || size < need {
        return E_NOT_SUFFICIENT_BUFFER;
    }
    core::ptr::copy_nonoverlapping(s.as_ptr(), buf, s.len());
    *buf.add(s.len()) = 0;
    S_OK
}

pub unsafe fn put_wstr(s: &str, count: usize, buf: *mut u16, used: *mut usize) -> i32 {
    let w: Vec<u16> = s.encode_utf16().chain(Some(0)).collect();
    if !used.is_null() {
        *used = w.len();
    }
    if buf.is_null() || count < w.len() {
        return E_NOT_SUFFICIENT_BUFFER;
    }
    core::ptr::copy_nonoverlapping(w.as_ptr(), buf, w.len());
    S_OK
}

#[derive(Clone, Debug, Default)]
pub struct Title {
    pub title_id: Option<u32>,
    pub identity: String,
    pub publisher: String,
    pub display: String,
    pub version: [u16; 4],
    pub key: String,
    pub dir: Option<PathBuf>,
}

fn attr(text: &str, tag: &str, name: &str) -> Option<String> {
    let start = text.find(&format!("<{tag}"))?;
    let rest = &text[start..];
    let end = rest.find('>')?;
    let el = &rest[..end];
    let pat = format!(" {name}=\"");
    let a = el.find(&pat)? + pat.len();
    let b = el[a..].find('"')?;
    Some(el[a..a + b].to_string())
}

fn element(text: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let a = text.find(&open)? + open.len();
    let b = text[a..].find(&format!("</{tag}>"))?;
    Some(text[a..a + b].trim().to_string())
}

pub fn sanitize(s: &str) -> String {
    let v: String = s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' { c } else { '_' }).collect();
    let v = v.trim_end_matches('.').to_string();
    if v.is_empty() { "title".into() } else { v }
}

impl Title {
    pub fn from_dir(start: &Path, fallback: &str) -> Title {
        let mut t = Title { identity: fallback.to_string(), key: sanitize(fallback), display: fallback.to_string(), ..Default::default() };
        let mut d = Some(start.to_path_buf());
        for _ in 0..10 {
            let Some(cur) = d else { break };
            let cfgp = cur.join("MicrosoftGame.config");
            if let Ok(bytes) = std::fs::read(&cfgp) {
                let text = String::from_utf8_lossy(&bytes).trim_start_matches('\u{feff}').to_string();
                if let Some(n) = attr(&text, "Identity", "Name") {
                    t.identity = n;
                }
                t.publisher = attr(&text, "Identity", "Publisher").unwrap_or_default();
                if let Some(v) = attr(&text, "Identity", "Version") {
                    let p: Vec<u16> = v.split('.').map(|x| x.parse().unwrap_or(0)).collect();
                    for (i, x) in p.iter().take(4).enumerate() {
                        t.version[i] = *x;
                    }
                }
                t.title_id = element(&text, "TitleId").and_then(|x| u32::from_str_radix(x.trim_start_matches("0x"), 16).ok());
                t.display = attr(&text, "ShellVisuals", "DefaultDisplayName").unwrap_or_else(|| t.identity.clone());
                t.key = match t.title_id {
                    Some(id) => format!("{id:08X}"),
                    None => sanitize(&t.identity),
                };
                t.dir = Some(cur);
                break;
            }
            d = cur.parent().map(|p| p.to_path_buf());
        }
        t
    }
}

static TITLE: OnceLock<Title> = OnceLock::new();

pub fn title() -> &'static Title {
    TITLE.get_or_init(|| {
        let exe = std::env::current_exe().unwrap_or_default();
        let stem = exe.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "title".into());
        let dir = setting("title_dir").map(PathBuf::from).unwrap_or_else(|| exe.parent().map(|p| p.to_path_buf()).unwrap_or_default());
        Title::from_dir(&dir, &stem)
    })
}

pub fn init_title(dir: &Path, fallback: &str) -> bool {
    TITLE.set(Title::from_dir(dir, fallback)).is_ok()
}

pub fn data_root() -> PathBuf {
    if let Ok(p) = std::env::var("TOMBOX_GDK_ROOT") {
        return PathBuf::from(p);
    }
    match std::env::var_os("LOCALAPPDATA") {
        Some(p) => PathBuf::from(p).join("XboxEmulator"),
        None => std::env::temp_dir().join("XboxEmulator"),
    }
}

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleExW(flags: u32, name: *const u16, out: *mut *mut c_void) -> i32;
}

#[used]
#[link_section = ".CRT$XCU"]
pub static EARLY_INIT: extern "C" fn() = early_init;

extern "C" fn early_init() {
    let _ = std::panic::catch_unwind(|| {
        if setting("gdk_early").as_deref() != Some("0") {
            install();
        }
    });
}

fn claim_owner() -> bool {
    let marker = std::process::id().to_string();
    if std::env::var("TOMBOX_GDK_OWNER").ok().as_deref() == Some(marker.as_str()) {
        return false;
    }
    std::env::set_var("TOMBOX_GDK_OWNER", &marker);
    let mut h: *mut c_void = core::ptr::null_mut();
    unsafe { GetModuleHandleExW(0x5, claim_owner as *const u16, &mut h) };
    true
}

pub fn install() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        if setting("gdk_runtime").as_deref() == Some("0") {
            crate::record_raw("gdk", "install", "gdk_runtime=0 - emulated GDK runtime NOT installed");
            return;
        }
        if !claim_owner() {
            return;
        }
        let _ = user::claim_handle();
        let t = title();
        let n = registry().len();
        let report = hook::install();
        crate::record_raw("gdk", "install", &format!(
            "emulated GDK runtime: {n} interfaces, title key '{}' identity '{}' titleId {:?}; {report}",
            t.key, t.identity, t.title_id.map(|x| format!("{x:08X}"))));
    });
}
