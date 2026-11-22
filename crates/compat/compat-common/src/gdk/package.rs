use super::xasync::{self, Payload};
use super::events::Registry;
use super::{log, title, Guid, Iface, This, E_INVALIDARG, E_NOT_SUFFICIENT_BUFFER, S_OK};
use core::ffi::c_void;
use std::ffi::CString;
use std::path::Path;
use std::sync::OnceLock;

pub const CLSID: Guid = Guid::parse("af406016-e850-4aa8-a88d-2f3dcb9dac7e");
pub const IID_V1: Guid = Guid::parse("be3f3e9f-c4b3-424d-9a95-d1af060559a5");
pub const IID_V2: Guid = Guid::parse("f92d8712-2b27-4d8a-bf01-11a6f8e3eb42");
pub const IID_3720: Guid = Guid::parse("3720de07-e8e4-44a3-ad32-b359e8adbe55");

#[derive(Default, Debug, Clone, PartialEq)]
pub struct ChunkInfo {
    pub id: u32,
    pub languages: Vec<String>,
    pub tags: Vec<String>,
    pub launch: bool,
    pub on_demand: bool,
    pub files: Vec<(String, String)>,
}

#[derive(Default, Debug, Clone, PartialEq)]
pub struct Layout {
    pub chunks: Vec<u32>,
    pub languages: Vec<String>,
    pub tags: Vec<String>,
    pub entries: Vec<ChunkInfo>,
}

pub const READY: u32 = 0;
pub const UNAVAILABLE: u32 = 3;
pub const E_GAMEPACKAGE_INVALID_SELECTOR: i32 = 0x8924_5203u32 as i32;
pub const FIND_SWALLOWED: [u32; 3] = [0x8007_0002, 0x8007_0570, 0x8007_01e7];

impl Layout {
    pub fn select(&self, kind: u32, value: &str, chunk: u32) -> Vec<u32> {
        self.entries.iter().filter(|c| match kind {
            0 => c.languages.iter().any(|l| l.eq_ignore_ascii_case(value)),
            1 => c.tags.iter().any(|t| t.eq_ignore_ascii_case(value)),
            2 => c.id == chunk,
            _ => false,
        }).map(|c| c.id).collect()
    }

    pub fn chunk_bytes(&self, root: &Path, id: u32) -> u64 {
        self.entries.iter().filter(|c| c.id == id).flat_map(|c| c.files.iter()).map(|(dir, inc)| glob_bytes(root, dir, inc)).sum()
    }
}

fn wild(p: &[u8], s: &[u8]) -> bool {
    match (p.first(), s.first()) {
        (None, None) => true,
        (Some(b'*'), _) => wild(&p[1..], s) || (!s.is_empty() && wild(p, &s[1..])),
        (Some(b'?'), Some(_)) => wild(&p[1..], &s[1..]),
        (Some(a), Some(b)) if a.eq_ignore_ascii_case(b) => wild(&p[1..], &s[1..]),
        _ => false,
    }
}

fn glob_bytes(root: &Path, dir: &str, include: &str) -> u64 {
    let rel = format!("{}\\{}", dir, include).replace('/', "\\");
    let parts: Vec<&str> = rel.split('\\').filter(|x| !x.is_empty() && *x != ".").collect();
    fn walk(cur: &Path, parts: &[&str]) -> u64 {
        let Some((first, rest)) = parts.split_first() else { return 0 };
        if *first == "**" {
            let mut n = walk(cur, rest);
            if let Ok(rd) = std::fs::read_dir(cur) {
                for e in rd.flatten() {
                    if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                        n += walk(&e.path(), parts);
                    }
                }
            }
            return n;
        }
        if !first.contains(['*', '?']) {
            let p = cur.join(first);
            return if rest.is_empty() { std::fs::metadata(&p).map(|m| if m.is_file() { m.len() } else { 0 }).unwrap_or(0) } else { walk(&p, rest) };
        }
        let mut n = 0;
        if let Ok(rd) = std::fs::read_dir(cur) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if !wild(first.as_bytes(), name.as_bytes()) {
                    continue;
                }
                let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
                if rest.is_empty() {
                    if !is_dir {
                        n += e.metadata().map(|m| m.len()).unwrap_or(0);
                    }
                } else if is_dir {
                    n += walk(&e.path(), rest);
                }
            }
        }
        n
    }
    walk(root, &parts)
}

fn attr(el: &str, name: &str) -> Option<String> {
    let pat = format!(" {name}=\"");
    let a = el.find(&pat)? + pat.len();
    let b = el[a..].find('"')?;
    Some(el[a..a + b].to_string())
}

pub fn parse_layout(text: &str) -> Layout {
    let mut l = Layout::default();
    let mut rest = text;
    while let Some(i) = rest.find("<Chunk ") {
        let el_end = rest[i..].find('>').map(|e| i + e).unwrap_or(rest.len());
        let el = &rest[i..el_end];
        let close = rest[el_end..].find("</Chunk>").map(|e| el_end + e).unwrap_or(rest.len());
        let body = if el.ends_with('/') { "" } else { &rest[el_end..close] };
        if let Some(id) = attr(el, "Id").and_then(|x| x.parse::<u32>().ok()) {
            if !l.chunks.contains(&id) {
                l.chunks.push(id);
            }
            let list = |k: &str| attr(el, k).map(|v| v.split([';', ',', ' ']).filter(|x| !x.is_empty()).map(String::from).collect::<Vec<_>>()).unwrap_or_default();
            let mut files = Vec::new();
            let mut b = body;
            while let Some(f) = b.find("<FileGroup ") {
                let fe = b[f..].find('>').map(|e| f + e).unwrap_or(b.len());
                let g = &b[f..fe];
                if let Some(inc) = attr(g, "Include") {
                    let dir = attr(g, "DestinationPath").or_else(|| attr(g, "SourcePath")).unwrap_or_default();
                    files.push((dir, inc));
                }
                b = &b[fe..];
            }
            l.entries.push(ChunkInfo {
                id,
                languages: list("Languages"),
                tags: list("Tags"),
                launch: attr(el, "Marker").map(|m| m.eq_ignore_ascii_case("Launch")).unwrap_or(false),
                on_demand: attr(el, "OnDemand").map(|m| m.eq_ignore_ascii_case("true")).unwrap_or(false),
                files,
            });
        }
        for (key, list) in [("Languages", &mut l.languages), ("Tags", &mut l.tags)] {
            if let Some(v) = attr(el, key) {
                for x in v.split([';', ',', ' ']).filter(|x| !x.is_empty()) {
                    if !list.iter().any(|y| y == x) {
                        list.push(x.to_string());
                    }
                }
            }
        }
        rest = &rest[el_end..];
    }
    l
}

pub fn load_layout(dir: &Path) -> Layout {
    let Ok(rd) = std::fs::read_dir(dir) else { return Layout::default() };
    for e in rd.flatten() {
        let n = e.file_name().to_string_lossy().to_ascii_lowercase();
        if n.starts_with("layout") && n.ends_with(".xml") {
            if let Ok(b) = std::fs::read(e.path()) {
                return parse_layout(&String::from_utf8_lossy(&b));
            }
        }
    }
    Layout::default()
}

fn layout() -> &'static Layout {
    static L: OnceLock<Layout> = OnceLock::new();
    L.get_or_init(|| title().dir.as_deref().map(load_layout).unwrap_or_default())
}

pub fn package_id() -> String {
    let s: String = title().identity.chars().take(32).collect();
    if s.is_empty() { "XboxEmulatorTitle".into() } else { s }
}

static OP_INSTALL: &[u8] = b"XPackageInstallChunksAsync\0";

const MMAGIC: u64 = 0x5850_4B47_4D4F_4E32;

#[repr(C)]
pub struct Monitor {
    magic: u64,
    pub chunks: Vec<u32>,
    pub total: u64,
    pub launch: u64,
}

fn root() -> std::path::PathBuf {
    title().dir.clone().unwrap_or_default()
}

pub fn chunk_size(id: u32) -> u64 {
    static C: OnceLock<std::sync::Mutex<std::collections::HashMap<u32, u64>>> = OnceLock::new();
    let m = C.get_or_init(Default::default);
    if let Some(v) = m.lock().unwrap_or_else(|e| e.into_inner()).get(&id) {
        return *v;
    }
    let v = layout().chunk_bytes(&root(), id);
    m.lock().unwrap_or_else(|e| e.into_inner()).insert(id, v);
    v
}

unsafe fn resolve(n: u32, sel: *const Selector) -> Result<Vec<u32>, i32> {
    let l = layout();
    if n == 0 {
        return Ok(l.chunks.clone());
    }
    if sel.is_null() {
        return Err(E_INVALIDARG);
    }
    let mut out: Vec<u32> = Vec::new();
    for i in 0..n as usize {
        let s = &*sel.add(i);
        let found = match s.kind {
            0 | 1 => l.select(s.kind, &super::cstr(s.value as *const u8).unwrap_or_default(), 0),
            2 => l.select(2, "", s.value as u32),
            3 => {
                let f = super::cstr(s.value as *const u8).unwrap_or_default();
                if features().iter().any(|x| x.id.eq_ignore_ascii_case(&f)) { l.chunks.clone() } else { Vec::new() }
            }
            _ => return Err(E_INVALIDARG),
        };
        if found.is_empty() {
            return Err(E_GAMEPACKAGE_INVALID_SELECTOR);
        }
        for c in found {
            if !out.contains(&c) {
                out.push(c);
            }
        }
    }
    Ok(out)
}

fn monitor_for(chunks: Vec<u32>) -> usize {
    let l = layout();
    let total = chunks.iter().map(|&c| chunk_size(c)).sum();
    let launch = chunks.iter().filter(|c| l.entries.iter().any(|e| e.id == **c && e.launch)).map(|&c| chunk_size(c)).sum::<u64>();
    let launch = if launch == 0 { total } else { launch };
    Box::into_raw(Box::new(Monitor { magic: MMAGIC, chunks, total, launch })) as usize
}

pub unsafe fn monitor<'a>(m: usize) -> Option<&'a Monitor> {
    let p = (m as *const Monitor).as_ref()?;
    (p.magic == MMAGIC).then_some(p)
}

fn known_package(id: *const u8) -> Result<(), i32> {
    if id.is_null() {
        return Err(E_INVALIDARG);
    }
    if unsafe { super::cstr(id) }.unwrap_or_default().eq_ignore_ascii_case(&package_id()) { Ok(()) } else { Err(super::E_NOT_FOUND) }
}

unsafe extern "system" fn get_package_id(_t: This, size: usize, buf: *mut u8) -> i32 {
    let id = package_id();
    if buf.is_null() || size < id.len() + 1 {
        return E_NOT_SUFFICIENT_BUFFER;
    }
    core::ptr::copy_nonoverlapping(id.as_ptr(), buf, id.len());
    *buf.add(id.len()) = 0;
    S_OK
}

unsafe extern "system" fn is_packaged(_t: This) -> bool {
    true
}

unsafe extern "system" fn create_monitor(_t: This, id: *const u8, n: u32, sel: *const Selector, _ms: u32, _queue: usize, out: *mut usize) -> i32 {
    if out.is_null() {
        return E_INVALIDARG;
    }
    if let Err(e) = known_package(id) {
        return e;
    }
    match resolve(n, sel) {
        Ok(c) => {
            *out = monitor_for(c);
            S_OK
        }
        Err(e) => e,
    }
}

unsafe extern "system" fn close_monitor(_t: This, m: usize) {
    if monitor(m).is_some() {
        let mut b = Box::from_raw(m as *mut Monitor);
        b.magic = 0;
        drop(b);
    }
}

unsafe extern "system" fn get_progress(_t: This, m: usize, out: *mut u8) {
    if out.is_null() {
        return;
    }
    let (total, launch) = monitor(m).map(|x| (x.total, x.launch)).unwrap_or((0, 0));
    core::ptr::write_unaligned(out as *mut u64, total);
    core::ptr::write_unaligned(out.add(8) as *mut u64, total);
    core::ptr::write_unaligned(out.add(16) as *mut u64, launch);
    *out.add(24) = 1;
    *out.add(25) = 1;
}

unsafe extern "system" fn find_chunk_availability(_t: This, id: *const u8, n: u32, sel: *const Selector, out: *mut u32) -> i32 {
    if out.is_null() || n == 0 {
        return E_INVALIDARG;
    }
    *out = UNAVAILABLE;
    let r = known_package(id).and_then(|_| resolve(n, sel));
    match r {
        Ok(_) => {
            *out = READY;
            S_OK
        }
        Err(e) if FIND_SWALLOWED.contains(&(e as u32)) => S_OK,
        Err(e) => e,
    }
}

#[repr(C)]
struct Selector {
    kind: u32,
    value: usize,
}

type ChunkCb = unsafe extern "system" fn(*mut c_void, *const Selector, u32) -> bool;
type PackageCb = unsafe extern "system" fn(*mut c_void, *const u8) -> bool;

unsafe extern "system" fn enumerate_chunks(_t: This, id: *const u8, kind: u32, ctx: *mut c_void, cb: Option<ChunkCb>) -> i32 {
    let Some(cb) = cb else { return E_INVALIDARG };
    if let Err(e) = known_package(id) {
        return e;
    }
    let l = layout();
    let strings: Vec<CString> = match kind {
        0 => l.languages.iter().filter_map(|s| CString::new(s.as_str()).ok()).collect(),
        1 => l.tags.iter().filter_map(|s| CString::new(s.as_str()).ok()).collect(),
        3 => features().iter().filter_map(|f| CString::new(f.id.as_str()).ok()).collect(),
        2 => Vec::new(),
        _ => return E_INVALIDARG,
    };
    let sels: Vec<Selector> = match kind {
        0 | 1 | 3 => strings.iter().map(|s| Selector { kind, value: s.as_ptr() as usize }).collect(),
        _ => l.chunks.iter().map(|&c| Selector { kind, value: c as usize }).collect(),
    };
    for s in &sels {
        if !cb(ctx, s, READY) {
            break;
        }
    }
    S_OK
}

unsafe extern "system" fn change_order(_t: This, id: *const u8, n: u32, sel: *const Selector) -> i32 {
    if let Err(e) = known_package(id) {
        return e;
    }
    if n == 0 {
        return E_INVALIDARG;
    }
    match resolve(n, sel) {
        Ok(_) => S_OK,
        Err(e) => e,
    }
}

#[allow(clippy::too_many_arguments)]
unsafe extern "system" fn install_chunks_async(_t: This, id: *const u8, n: u32, sel: *const Selector, _ms: u32, _suppress: bool, block: *mut c_void) -> i32 {
    if let Err(e) = known_package(id) {
        return e;
    }
    if n == 0 {
        return E_INVALIDARG;
    }
    let chunks = match resolve(n, sel) {
        Ok(c) => c,
        Err(e) => return e,
    };
    log("XPackageInstallChunksAsync", &format!("{n} selectors -> chunks {chunks:?} already installed"));
    xasync::run(block, OP_INSTALL, Payload::value(monitor_for(chunks)))
}

unsafe extern "system" fn install_chunks_result(_t: This, block: *mut c_void, out: *mut usize) -> i32 {
    if out.is_null() {
        return E_INVALIDARG;
    }
    xasync::value(block, OP_INSTALL, out)
}

unsafe extern "system" fn enumerate_packages(_t: This, kind: u32, scope: u32, ctx: *mut c_void, cb: Option<PackageCb>) -> i32 {
    let Some(cb) = cb else { return E_INVALIDARG };
    log("XPackageEnumeratePackages", &format!("kind={kind} scope={scope} -> {}", if kind == 0 { "this title" } else { "none" }));
    if kind != 0 {
        return S_OK;
    }
    let t = title();
    let id = CString::new(package_id()).unwrap_or_default();
    let disp = CString::new(t.display.clone()).unwrap_or_default();
    let publ = CString::new(t.publisher.clone()).unwrap_or_default();
    let empty = CString::new("").unwrap_or_default();
    let tid = CString::new(t.title_id.map(|x| format!("{x:08X}")).unwrap_or_default()).unwrap_or_default();
    let v = t.version;
    let version = xversion(v);
    let mut d = [0u8; 0x60];
    let p = d.as_mut_ptr();
    core::ptr::write_unaligned(p as *mut usize, id.as_ptr() as usize);
    core::ptr::write_unaligned(p.add(0x08) as *mut u64, version);
    core::ptr::write_unaligned(p.add(0x10) as *mut u32, 0);
    core::ptr::write_unaligned(p.add(0x18) as *mut usize, disp.as_ptr() as usize);
    core::ptr::write_unaligned(p.add(0x20) as *mut usize, empty.as_ptr() as usize);
    core::ptr::write_unaligned(p.add(0x28) as *mut usize, publ.as_ptr() as usize);
    core::ptr::write_unaligned(p.add(0x30) as *mut usize, empty.as_ptr() as usize);
    core::ptr::write_unaligned(p.add(0x40) as *mut u32, 1);
    core::ptr::write_unaligned(p.add(0x48) as *mut usize, tid.as_ptr() as usize);
    cb(ctx, p);
    S_OK
}

static INSTALLED: Registry = Registry::new();
static PROGRESS: Registry = Registry::new();

unsafe extern "system" fn update_monitor(_t: This, m: usize) {
    if m == 0 {
        return;
    }
    let m2 = m;
    PROGRESS.fire(Some(m), move |r| {
        let cb: unsafe extern "system" fn(*mut c_void, usize) = core::mem::transmute(r.cb);
        cb(r.ctx as *mut c_void, m2);
    });
}

unsafe extern "system" fn unregister_progress(_t: This, m: usize, token: u64, _wait: bool) -> bool {
    PROGRESS.unregister(Some(m), token)
}

unsafe extern "system" fn register_progress_v2(_t: This, m: usize, ctx: *mut c_void, cb: usize, token: *mut u64) -> i32 {
    if m == 0 {
        return E_INVALIDARG;
    }
    PROGRESS.register(m, 0, ctx, cb, token)
}

unsafe extern "system" fn register_installed(_t: This, queue: usize, ctx: *mut c_void, cb: usize, token: *mut u64) -> i32 {
    INSTALLED.register(0, queue, ctx, cb, token)
}

unsafe extern "system" fn unregister_installed(_t: This, token: u64, _wait: bool) -> bool {
    INSTALLED.unregister(None, token)
}

pub fn user_locale() -> String {
    extern "system" {
        fn GetUserDefaultLocaleName(buf: *mut u16, n: i32) -> i32;
    }
    let mut buf = [0u16; 85];
    let n = unsafe { GetUserDefaultLocaleName(buf.as_mut_ptr(), buf.len() as i32) };
    if n <= 1 {
        return "en-US".into();
    }
    String::from_utf16_lossy(&buf[..(n - 1) as usize])
}

unsafe extern "system" fn get_user_locale(_t: This, size: usize, buf: *mut u8) -> i32 {
    let l = user_locale();
    if buf.is_null() || size < l.len() + 1 {
        return E_NOT_SUFFICIENT_BUFFER;
    }
    core::ptr::copy_nonoverlapping(l.as_ptr(), buf, l.len());
    *buf.add(l.len()) = 0;
    S_OK
}

unsafe extern "system" fn estimate_download_size(_t: This, id: *const u8, _n: u32, _sel: *const c_void, size: *mut u64, confirm: *mut bool) -> i32 {
    if size.is_null() {
        return E_INVALIDARG;
    }
    let pid = super::cstr(id).unwrap_or_default();
    if pid != package_id() {
        return super::E_NOT_FOUND;
    }
    *size = 0;
    if !confirm.is_null() {
        *confirm = false;
    }
    S_OK
}

pub fn mount_path() -> String {
    let d = title().dir.clone().or_else(|| std::env::current_exe().ok().and_then(|p| p.parent().map(|x| x.to_path_buf()))).unwrap_or_default();
    d.to_string_lossy().into_owned()
}

unsafe extern "system" fn mount_path_size(_t: This, mount: usize, out: *mut usize) -> i32 {
    if mount == 0 || out.is_null() {
        return E_INVALIDARG;
    }
    *out = mount_path().len() + 1;
    S_OK
}

unsafe extern "system" fn get_mount_path(_t: This, mount: usize, size: usize, buf: *mut u8) -> i32 {
    if mount == 0 {
        return E_INVALIDARG;
    }
    let p = mount_path();
    if buf.is_null() || size < p.len() + 1 {
        return E_NOT_SUFFICIENT_BUFFER;
    }
    core::ptr::copy_nonoverlapping(p.as_ptr(), buf, p.len());
    *buf.add(p.len()) = 0;
    S_OK
}

unsafe extern "system" fn close_mount(_t: This, _mount: usize) {}

static WRITTEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn note_write(bytes: u64) {
    WRITTEN.fetch_add(bytes, std::sync::atomic::Ordering::Relaxed);
}

fn start_time() -> std::time::Instant {
    static S: OnceLock<std::time::Instant> = OnceLock::new();
    *S.get_or_init(std::time::Instant::now)
}

unsafe extern "system" fn get_write_stats(_t: This, out: *mut u64) -> i32 {
    if out.is_null() {
        return E_INVALIDARG;
    }
    let interval = 5u64 * 60 * 1000;
    let elapsed = start_time().elapsed().as_millis() as u64 % interval;
    *out = interval;
    *out.add(1) = super::setting("write_budget_mb").and_then(|v| v.parse::<u64>().ok()).unwrap_or(1024) << 20;
    *out.add(2) = elapsed;
    *out.add(3) = WRITTEN.load(std::sync::atomic::Ordering::Relaxed);
    S_OK
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Feature {
    pub id: String,
    pub display: String,
    pub tags: String,
    pub hidden: bool,
    pub store_id: String,
}

pub fn parse_features(text: &str) -> Vec<Feature> {
    let mut v = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find("<Feature ") {
        let end = rest[i..].find('>').map(|e| i + e).unwrap_or(rest.len());
        let el = &rest[i..end];
        if let Some(id) = attr(el, "Id") {
            v.push(Feature {
                display: attr(el, "DisplayName").unwrap_or_else(|| id.clone()),
                tags: attr(el, "Tags").unwrap_or_default(),
                hidden: attr(el, "Hidden").map(|h| h.eq_ignore_ascii_case("true")).unwrap_or(false),
                store_id: attr(el, "StoreId").unwrap_or_default(),
                id,
            });
        }
        rest = &rest[end..];
    }
    v
}

pub fn features() -> &'static Vec<Feature> {
    static F: OnceLock<Vec<Feature>> = OnceLock::new();
    F.get_or_init(|| {
        let Some(dir) = title().dir.as_deref() else { return Vec::new() };
        let mut v = Vec::new();
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().to_ascii_lowercase();
                if n.ends_with(".xml") || n == "microsoftgame.config" {
                    if let Ok(b) = std::fs::read(e.path()) {
                        v.extend(parse_features(&String::from_utf8_lossy(&b)));
                    }
                }
            }
        }
        v
    })
}

type FeatureCb = unsafe extern "system" fn(*mut c_void, *const u8) -> bool;

unsafe extern "system" fn enumerate_features(_t: This, id: *const u8, ctx: *mut c_void, cb: Option<FeatureCb>) -> i32 {
    let Some(cb) = cb else { return E_INVALIDARG };
    if super::cstr(id).unwrap_or_default() != package_id() {
        return super::E_NOT_FOUND;
    }
    for f in features() {
        let s: Vec<CString> = [&f.id, &f.display, &f.tags, &f.store_id].iter().map(|x| CString::new(x.as_str()).unwrap_or_default()).collect();
        let mut d = [0u8; 40];
        let p = d.as_mut_ptr();
        core::ptr::write_unaligned(p as *mut usize, s[0].as_ptr() as usize);
        core::ptr::write_unaligned(p.add(8) as *mut usize, s[1].as_ptr() as usize);
        core::ptr::write_unaligned(p.add(16) as *mut usize, s[2].as_ptr() as usize);
        *p.add(24) = f.hidden as u8;
        core::ptr::write_unaligned(p.add(32) as *mut usize, s[3].as_ptr() as usize);
        if !cb(ctx, p) {
            break;
        }
    }
    S_OK
}

fn base(v: &mut Vec<(usize, usize)>) {
    v.extend([
        (3, get_package_id as usize),
        (4, is_packaged as usize),
        (5, create_monitor as usize),
        (6, close_monitor as usize),
        (7, get_progress as usize),
        (8, update_monitor as usize),
        (9, register_progress_v2 as usize),
        (10, unregister_progress as usize),
        (11, get_user_locale as usize),
    ]);
}

pub fn interfaces() -> Vec<Iface> {
    let mut v1 = Vec::new();
    base(&mut v1);
    v1.extend([
        (12, find_chunk_availability as usize),
        (13, enumerate_chunks as usize),
        (14, change_order as usize),
        (16, install_chunks_async as usize),
        (17, install_chunks_result as usize),
        (28, enumerate_packages as usize),
    ]);
    let mut v3 = Vec::new();
    base(&mut v3);
    let mut v2 = Vec::new();
    base(&mut v2);
    v2.extend([
        (18, estimate_download_size as usize),
        (22, unregister_installed as usize),
        (24, mount_path_size as usize),
        (25, get_mount_path as usize),
        (26, close_mount as usize),
        (30, get_write_stats as usize),
        (33, enumerate_features as usize),
        (35, enumerate_packages as usize),
        (36, register_installed as usize),
    ]);
    vec![
        Iface { clsid: CLSID, iid: IID_V1, name: "XPackage(be3f3e9f)", slots: v1 },
        Iface { clsid: CLSID, iid: IID_V2, name: "XPackage(f92d8712)", slots: v2 },
        Iface { clsid: CLSID, iid: IID_3720, name: "XPackage(3720de07)", slots: v3 },
    ]
}

pub fn xversion(v: [u16; 4]) -> u64 {
    (v[0] as u64) | ((v[1] as u64) << 16) | ((v[2] as u64) << 32) | ((v[3] as u64) << 48)
}
