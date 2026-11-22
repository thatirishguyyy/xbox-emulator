use super::{log, query_api_impl};
use core::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};

extern "system" {
    fn GetCurrentProcess() -> *mut c_void;
    fn K32EnumProcessModules(p: *mut c_void, mods: *mut *mut c_void, cb: u32, needed: *mut u32) -> i32;
    fn GetModuleFileNameW(m: *mut c_void, buf: *mut u16, n: u32) -> u32;
    fn GetModuleHandleW(name: *const u16) -> *mut u8;
    fn GetSystemWindowsDirectoryW(buf: *mut u16, n: u32) -> u32;
    fn VirtualProtect(addr: *mut c_void, size: usize, new_protect: u32, old: *mut u32) -> i32;
}

static ORIG_GPA: AtomicUsize = AtomicUsize::new(0);
static ORIG_REG_OPEN: AtomicUsize = AtomicUsize::new(0);
static ORIG_REG_OPEN_EX: AtomicUsize = AtomicUsize::new(0);
static ORIG_REG_QUERY: AtomicUsize = AtomicUsize::new(0);
static ORIG_REG_CLOSE: AtomicUsize = AtomicUsize::new(0);
static CONSOLE_MODE: AtomicUsize = AtomicUsize::new(0);

pub const OEM_KEY: &str = "Software\\Microsoft\\Windows NT\\CurrentVersion\\OEM";
const HKLM: usize = 0xffff_ffff_8000_0002;
const SENTINEL: usize = 0x5842_4F58_4F45_4D11;
pub const PASSWORDLESS_DEVICE_KEY: &str = "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\PasswordLess\\Device";
const EMULATED_TAG: usize = 0x5842_4F58_5200_0000;
const EMULATED_MASK: usize = 0xffff_ffff_ff00_0000;

use super::emureg::{self, Hive};

static EMULATED_KEYS: std::sync::Mutex<Vec<(usize, Hive, String)>> = std::sync::Mutex::new(Vec::new());
static NEXT_KEY: AtomicUsize = AtomicUsize::new(1);

pub fn emulated_hklm_key(root: &std::path::Path, sub: &str) -> Option<std::path::PathBuf> {
    if !emureg::is_emulated(Hive::Hklm, sub) {
        return None;
    }
    emureg::key_dir(root, Hive::Hklm, sub)
}

pub fn emulated_hklm_open(root: &std::path::Path, sub: &str) -> Option<i32> {
    let dir = emulated_hklm_key(root, sub)?;
    Some(if dir.is_dir() { 0 } else { 2 })
}

pub fn emulated_hklm_dword(root: &std::path::Path, value: &str) -> Option<u32> {
    match emureg::get_value(root, Hive::Hklm, PASSWORDLESS_DEVICE_KEY, value).ok()? {
        (emureg::REG_DWORD, b) => Some(u32::from_le_bytes(b.try_into().ok()?)),
        _ => None,
    }
}

fn predefined(hkey: usize) -> Option<u32> {
    let hi = hkey >> 32;
    (hi == 0 || hi == 0xffff_ffff).then_some(hkey as u32)
}

fn is_emulated_handle(hkey: usize) -> bool {
    hkey & EMULATED_MASK == EMULATED_TAG
}

fn emulated_key(hkey: usize) -> Option<(Hive, String)> {
    EMULATED_KEYS.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|(h, _, _)| *h == hkey).map(|(_, hv, p)| (*hv, p.clone()))
}

pub fn emulated_target(hkey: usize, sub: &str, current_user: Option<&str>) -> Option<(Hive, String)> {
    let (hive, base) = if is_emulated_handle(hkey) {
        emulated_key(hkey)?
    } else {
        match predefined(hkey)? {
            0x8000_0002 => (Hive::Hklm, String::new()),
            0x8000_0003 => (Hive::Hku, String::new()),
            0x8000_0001 => (Hive::Hku, current_user?.to_string()),
            _ => return None,
        }
    };
    let path = if base.is_empty() { sub.to_string() } else if sub.is_empty() { base } else { format!("{base}\\{sub}") };
    emureg::is_emulated(hive, &path).then_some((hive, path))
}

pub fn emulated_open_at(root: &std::path::Path, hkey: usize, sub: &str, current_user: Option<&str>) -> Option<Result<usize, i32>> {
    let (hive, path) = emulated_target(hkey, sub, current_user)?;
    if !emureg::key_exists(root, hive, &path) {
        return Some(Err(2));
    }
    let h = EMULATED_TAG | (NEXT_KEY.fetch_add(1, Ordering::Relaxed) & !EMULATED_MASK);
    EMULATED_KEYS.lock().unwrap_or_else(|e| e.into_inner()).push((h, hive, path));
    Some(Ok(h))
}

pub unsafe fn emulated_query_at(root: &std::path::Path, hkey: usize, name: *const u16, kind: *mut u32, data: *mut u8, cb: *mut u32) -> Option<i32> {
    if !is_emulated_handle(hkey) {
        return None;
    }
    let Some((hive, path)) = emulated_key(hkey) else { return Some(6) };
    let n = super::wstr(name).unwrap_or_default();
    Some(emureg::query_into(emureg::get_value(root, hive, &path, &n), kind, data, cb))
}

pub fn emulated_close(hkey: usize) -> Option<i32> {
    if !is_emulated_handle(hkey) {
        return None;
    }
    let mut keys = EMULATED_KEYS.lock().unwrap_or_else(|e| e.into_inner());
    let before = keys.len();
    keys.retain(|(h, _, _)| *h != hkey);
    Some(if keys.len() < before { 0 } else { 6 })
}

unsafe fn emulated_open(hkey: usize, sub: *const u16, out: *mut usize) -> Option<i32> {
    if CONSOLE_MODE.load(Ordering::Relaxed) == 0 || out.is_null() {
        return None;
    }
    let s = if sub.is_null() { String::new() } else { super::wstr(sub)? };
    let me = if predefined(hkey) == Some(0x8000_0001) { emureg::effective_user() } else { None };
    match emulated_open_at(&super::data_root(), hkey, &s, me.as_deref())? {
        Ok(h) => {
            *out = h;
            Some(0)
        }
        Err(e) => Some(e),
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

unsafe fn module_path(m: *mut c_void) -> String {
    let mut buf = [0u16; 1024];
    let n = GetModuleFileNameW(m, buf.as_mut_ptr(), buf.len() as u32) as usize;
    String::from_utf16_lossy(&buf[..n.min(buf.len())])
}

unsafe fn is_xgameruntime(h: *mut c_void) -> bool {
    !h.is_null() && module_path(h).to_ascii_lowercase().ends_with("\\xgameruntime.dll")
}

unsafe fn is_query_name(name: *const u8) -> bool {
    if (name as usize) < 0x10000 {
        return name as usize == 3;
    }
    super::cstr(name).as_deref() == Some("QueryApiImpl")
}

pub unsafe extern "system" fn get_proc_address_hook(h: *mut c_void, name: *const u8) -> *mut c_void {
    let p = crate::os_host::routes_rt::resolve_proc(h, name);
    if !p.is_null() {
        return p;
    }
    let o = ORIG_GPA.load(Ordering::Acquire);
    if o == 0 {
        return core::ptr::null_mut();
    }
    let real: unsafe extern "system" fn(*mut c_void, *const u8) -> *mut c_void = core::mem::transmute(o);
    let r = real(h, name);
    if !r.is_null() && is_query_name(name) && is_xgameruntime(h) {
        super::set_real_query(r as usize);
        log("GetProcAddress", &format!("xgameruntime!QueryApiImpl {:#x} -> emulated runtime", r as usize));
        return query_api_impl as *mut c_void;
    }
    super::mediacontrol::route(h, name, r)
}

pub fn console_mode(device: &str) -> Option<u32> {
    match device.to_ascii_lowercase().as_str() {
        "anaconda" | "scarlett" | "seriesx" | "xsx" => Some(0x64),
        "lockhart" | "seriess" | "xss" => Some(0x60),
        "devkit" => Some(0x68),
        "pc" | "off" | "" => None,
        x => u32::from_str_radix(x.trim_start_matches("0x"), 16).ok(),
    }
}

unsafe fn is_oem(hkey: usize, sub: *const u16) -> bool {
    hkey == HKLM && CONSOLE_MODE.load(Ordering::Relaxed) != 0 && super::wstr(sub).map(|s| s.eq_ignore_ascii_case(OEM_KEY)).unwrap_or(false)
}

unsafe extern "system" fn reg_open_hook(hkey: usize, sub: *const u16, out: *mut usize) -> i32 {
    if is_oem(hkey, sub) && !out.is_null() {
        *out = SENTINEL;
        return 0;
    }
    if let Some(r) = emulated_open(hkey, sub, out) {
        return r;
    }
    let f: unsafe extern "system" fn(usize, *const u16, *mut usize) -> i32 = core::mem::transmute(ORIG_REG_OPEN.load(Ordering::Acquire));
    f(hkey, sub, out)
}

unsafe extern "system" fn reg_open_ex_hook(hkey: usize, sub: *const u16, opts: u32, sam: u32, out: *mut usize) -> i32 {
    if is_oem(hkey, sub) && !out.is_null() {
        *out = SENTINEL;
        return 0;
    }
    if let Some(r) = emulated_open(hkey, sub, out) {
        return r;
    }
    let f: unsafe extern "system" fn(usize, *const u16, u32, u32, *mut usize) -> i32 = core::mem::transmute(ORIG_REG_OPEN_EX.load(Ordering::Acquire));
    f(hkey, sub, opts, sam, out)
}

pub unsafe fn query_console_mode(name: *const u16, kind: *mut u32, data: *mut u8, cb: *mut u32) -> i32 {
    if super::wstr(name).map(|s| s.eq_ignore_ascii_case("ConsoleMode")) != Some(true) {
        return 2;
    }
    if !kind.is_null() {
        *kind = 4;
    }
    let v = CONSOLE_MODE.load(Ordering::Relaxed) as u32;
    if !data.is_null() {
        if cb.is_null() || *cb < 4 {
            if !cb.is_null() {
                *cb = 4;
            }
            return 234;
        }
        core::ptr::write_unaligned(data as *mut u32, v);
    }
    if !cb.is_null() {
        *cb = 4;
    }
    0
}

unsafe extern "system" fn reg_query_hook(hkey: usize, name: *const u16, reserved: *mut u32, kind: *mut u32, data: *mut u8, cb: *mut u32) -> i32 {
    if hkey == SENTINEL {
        return query_console_mode(name, kind, data, cb);
    }
    if let Some(r) = emulated_query_at(&super::data_root(), hkey, name, kind, data, cb) {
        return r;
    }
    let f: unsafe extern "system" fn(usize, *const u16, *mut u32, *mut u32, *mut u8, *mut u32) -> i32 = core::mem::transmute(ORIG_REG_QUERY.load(Ordering::Acquire));
    f(hkey, name, reserved, kind, data, cb)
}

unsafe extern "system" fn reg_close_hook(hkey: usize) -> i32 {
    if hkey == SENTINEL {
        return 0;
    }
    if let Some(r) = emulated_close(hkey) {
        return r;
    }
    let f: unsafe extern "system" fn(usize) -> i32 = core::mem::transmute(ORIG_REG_CLOSE.load(Ordering::Acquire));
    f(hkey)
}

struct Target {
    name: &'static str,
    hook: usize,
    orig: &'static AtomicUsize,
}

unsafe fn rd32(p: *const u8) -> u32 {
    core::ptr::read_unaligned(p as *const u32)
}

pub unsafe fn patch_module(base: *mut u8, targets: &[(&str, usize, &AtomicUsize)]) -> usize {
    if base.is_null() || core::ptr::read_unaligned(base as *const u16) != 0x5a4d {
        return 0;
    }
    let pe = base.add(rd32(base.add(0x3c)) as usize);
    if rd32(pe) != 0x4550 || core::ptr::read_unaligned(pe.add(24) as *const u16) != 0x20b {
        return 0;
    }
    let dir = pe.add(24 + 0x70 + 8);
    let (rva, size) = (rd32(dir) as usize, rd32(dir.add(4)) as usize);
    if rva == 0 || size == 0 {
        return 0;
    }
    let mut patched = 0;
    let mut desc = base.add(rva);
    loop {
        let int_rva = rd32(desc) as usize;
        let name_rva = rd32(desc.add(12)) as usize;
        let iat_rva = rd32(desc.add(16)) as usize;
        if name_rva == 0 && iat_rva == 0 {
            break;
        }
        if int_rva != 0 && iat_rva != 0 {
            let mut i = 0usize;
            loop {
                let thunk = core::ptr::read_unaligned(base.add(int_rva + i * 8) as *const u64);
                if thunk == 0 {
                    break;
                }
                if thunk & (1 << 63) == 0 {
                    let fname = super::cstr(base.add(thunk as usize + 2)).unwrap_or_default();
                    for (tname, hook, orig) in targets {
                        if fname == *tname {
                            let slot = base.add(iat_rva + i * 8) as *mut usize;
                            let cur = core::ptr::read_volatile(slot);
                            if cur != *hook {
                                let _ = orig.compare_exchange(0, cur, Ordering::AcqRel, Ordering::Acquire);
                                let mut old = 0u32;
                                if VirtualProtect(slot as *mut c_void, 8, 0x40, &mut old) != 0 {
                                    core::ptr::write_volatile(slot, *hook);
                                    let mut tmp = 0u32;
                                    VirtualProtect(slot as *mut c_void, 8, old, &mut tmp);
                                    patched += 1;
                                }
                            }
                        }
                    }
                }
                i += 1;
            }
        }
        desc = desc.add(20);
    }
    patched
}

fn own_module() -> *mut c_void {
    extern "system" {
        fn GetModuleHandleExW(flags: u32, name: *const u16, out: *mut *mut c_void) -> i32;
    }
    let mut h: *mut c_void = core::ptr::null_mut();
    unsafe { GetModuleHandleExW(0x6, own_module as *const u16, &mut h) };
    h
}

unsafe fn modules() -> Vec<*mut c_void> {
    let mut v: Vec<*mut c_void> = vec![core::ptr::null_mut(); 1024];
    let mut needed = 0u32;
    if K32EnumProcessModules(GetCurrentProcess(), v.as_mut_ptr(), (v.len() * 8) as u32, &mut needed) == 0 {
        return Vec::new();
    }
    v.truncate((needed as usize / 8).min(1024));
    v
}

unsafe fn windows_dir() -> String {
    let mut buf = [0u16; 512];
    let n = GetSystemWindowsDirectoryW(buf.as_mut_ptr(), buf.len() as u32) as usize;
    let mut s = String::from_utf16_lossy(&buf[..n.min(buf.len())]).to_ascii_lowercase();
    if !s.ends_with('\\') {
        s.push('\\');
    }
    s
}

fn targets(registry: bool) -> Vec<Target> {
    let mut t = vec![Target { name: "GetProcAddress", hook: get_proc_address_hook as usize, orig: &ORIG_GPA }];
    if registry {
        t.push(Target { name: "RegOpenKeyW", hook: reg_open_hook as usize, orig: &ORIG_REG_OPEN });
        t.push(Target { name: "RegOpenKeyExW", hook: reg_open_ex_hook as usize, orig: &ORIG_REG_OPEN_EX });
        t.push(Target { name: "RegQueryValueExW", hook: reg_query_hook as usize, orig: &ORIG_REG_QUERY });
        t.push(Target { name: "RegCloseKey", hook: reg_close_hook as usize, orig: &ORIG_REG_CLOSE });
    }
    for (name, hook, orig) in super::fs::targets() {
        t.push(Target { name, hook, orig });
    }
    t
}

fn lists() -> (Vec<(&'static str, usize, &'static AtomicUsize)>, Vec<(&'static str, usize, &'static AtomicUsize)>) {
    let registry = CONSOLE_MODE.load(Ordering::Relaxed) != 0;
    let base: Vec<(&'static str, usize, &'static AtomicUsize)> = targets(registry).iter().map(|x| (x.name, x.hook, x.orig)).collect();
    let mut title = base.clone();
    title.extend(super::identity::targets());
    title.extend(super::memstatus::targets());
    title.extend(crate::kmd_bridge::targets());
    (base, title)
}

unsafe fn list_for<'a>(m: *mut c_void, base: &'a [(&'static str, usize, &'static AtomicUsize)], title: &'a [(&'static str, usize, &'static AtomicUsize)]) -> &'a [(&'static str, usize, &'static AtomicUsize)] {
    if super::identity::is_tombox_module(m) { base } else { title }
}

pub fn patch_all() -> (usize, usize) {
    let (base, title) = lists();
    unsafe {
        let win = windows_dir();
        let (mut mods, mut slots) = (0, 0);
        let me = own_module();
        let me = if me == GetModuleHandleW(core::ptr::null()) as *mut c_void { core::ptr::null_mut() } else { me };
        for m in modules() {
            if m == me || module_path(m).to_ascii_lowercase().starts_with(&win) {
                continue;
            }
            let n = patch_module(m as *mut u8, list_for(m, &base, &title));
            if n > 0 {
                mods += 1;
                slots += n;
            }
        }
        (mods, slots)
    }
}

pub fn patch_late(module: *mut u8) -> usize {
    let (base, title) = lists();
    unsafe { patch_module(module, list_for(module as *mut c_void, &base, &title)) }
}

pub fn target_family() -> Option<String> {
    static F: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    F.get_or_init(|| {
        let dir = super::title().dir.as_ref()?;
        dir.ancestors().take(5).find_map(|d| std::fs::read_to_string(d.join("MicrosoftGame.config")).ok().and_then(|t| family_of(&t)))
    }).clone()
}

pub fn family_of(t: &str) -> Option<String> {
    let mut rest = t;
    while let Some(i) = rest.find("TargetDeviceFamily=\"") {
        let v = &rest[i + 20..];
        let end = v.find('"')?;
        let val = v[..end].trim().to_ascii_lowercase();
        let before = &rest[..i];
        let in_comment = before.rfind("<!--").map(|c| before[c..].find("-->").is_none()).unwrap_or(false);
        if !in_comment {
            return Some(val);
        }
        rest = &v[end..];
    }
    None
}

pub fn default_device(family: Option<&str>) -> &'static str {
    match family {
        Some("xboxone") | Some("scarlett") => "anaconda",
        _ => "",
    }
}

pub fn install() -> String {
    let device = super::identity::device();
    let mode = console_mode(&device);
    CONSOLE_MODE.store(mode.unwrap_or(0) as usize, Ordering::Relaxed);
    let preloaded = unsafe { !GetModuleHandleW(wide("xgameruntime.dll").as_ptr()).is_null() };
    let late = super::late::watch();
    let (mods, slots) = patch_all();
    let mut s = format!("import hooks: {slots} slots in {mods} modules, late loads {}; {}", if late { "watched" } else { "NOT watched" }, super::fs::describe());
    let cpu = super::identity::cpu();
    s.push_str(&format!("; identity {} (XSystemGetDeviceType {}), title CPU view {} cores mask {:#x}{}", super::identity::device_form(), super::identity::device_type_value(), cpu.count, cpu.mask, if super::identity::targets().is_empty() { " (GetSystemInfo hooks off)" } else { "" }));
    match mode {
        Some(m) => s.push_str(&format!("; XSystemGetDeviceType registry wrap ON (ConsoleMode={m:#x}, device_type={device})")),
        None => s.push_str(&format!("; device type registry wrap off (device_type '{device}', family {:?})", target_family())),
    }
    if preloaded {
        s.push_str("; WARNING xgameruntime.dll was already loaded - a title that resolved QueryApiImpl earlier keeps the real runtime");
    }
    s
}

#[cfg(test)]
mod emulated_hklm_tests {
    use super::*;

    #[test]
    fn passwordless_device_key_reads_the_emulated_hklm_store() {
        let root = std::env::temp_dir().join(format!("tombox_cc_hklm_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(emulated_hklm_open(&root, "Software\\Microsoft\\Windows NT\\CurrentVersion\\OEM"), None);
        assert_eq!(emulated_hklm_open(&root, PASSWORDLESS_DEVICE_KEY), Some(2));
        assert_eq!(emulated_hklm_dword(&root, "DevicePasswordLessBuildVersion"), None);
        let dir = emulated_hklm_key(&root, &PASSWORDLESS_DEVICE_KEY.to_ascii_lowercase()).unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(emulated_hklm_open(&root, PASSWORDLESS_DEVICE_KEY), Some(0));
        std::fs::write(dir.join("DevicePasswordLessBuildVersion.dword"), "2\n").unwrap();
        assert_eq!(emulated_hklm_dword(&root, "DevicePasswordLessBuildVersion"), Some(2));
        assert_eq!(emulated_hklm_dword(&root, "Other"), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    #[test]
    fn hku_families_and_user_config_keys_are_served_through_emulated_handles() {
        let root = std::env::temp_dir().join(format!("tombox_cc_hku_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let sid = "S-1-5-21-11-22-33-1001";
        let pkg = format!("{sid}\\{}\\Fam_8wekyb3d8bbwe\\Fam_1.0.0.0_x64__8wekyb3d8bbwe", emureg::FAMILIES);
        assert_eq!(emulated_open_at(&root, 0x8000_0003, &pkg, None), Some(Err(2)));
        assert_eq!(emulated_open_at(&root, 0x8000_0003, &format!("{sid}\\Software\\Microsoft"), None), None);
        emureg::create_key(&root, Hive::Hku, &pkg).unwrap();
        emureg::set_value(&root, Hive::Hku, &pkg, "InstallTime", emureg::REG_QWORD, &0x0102_0304_0506_0708u64.to_le_bytes()).unwrap();
        emureg::set_value(&root, Hive::Hku, &pkg, "Flags", emureg::REG_DWORD, &3u32.to_le_bytes()).unwrap();
        let rel = format!("{}\\Fam_8wekyb3d8bbwe\\Fam_1.0.0.0_x64__8wekyb3d8bbwe", emureg::FAMILIES);
        let h = emulated_open_at(&root, 0xffff_ffff_8000_0001, &rel, Some(sid)).unwrap().unwrap();
        unsafe {
            let (mut kind, mut cb) = (0u32, 0u32);
            assert_eq!(emulated_query_at(&root, h, w("InstallTime").as_ptr(), &mut kind, core::ptr::null_mut(), &mut cb), Some(0));
            assert_eq!((kind, cb), (emureg::REG_QWORD, 8));
            let mut v = 0u64;
            assert_eq!(emulated_query_at(&root, h, w("InstallTime").as_ptr(), &mut kind, &mut v as *mut u64 as *mut u8, &mut cb), Some(0));
            assert_eq!(v, 0x0102_0304_0506_0708);
            let mut d = 0u32;
            cb = 4;
            assert_eq!(emulated_query_at(&root, h, w("Flags").as_ptr(), &mut kind, &mut d as *mut u32 as *mut u8, &mut cb), Some(0));
            assert_eq!((kind, cb, d), (emureg::REG_DWORD, 4, 3));
            assert_eq!(emulated_query_at(&root, h, w("Gone").as_ptr(), &mut kind, core::ptr::null_mut(), &mut cb), Some(2));
        }
        assert_eq!(emulated_close(h), Some(0));
        assert_eq!(emulated_close(h), Some(6));
        let uc = format!("{}\\UserConfig\\{sid}", emureg::PASSWORDLESS_ROOT);
        emureg::create_key(&root, Hive::Hklm, &uc).unwrap();
        emureg::set_value(&root, Hive::Hklm, &uc, "PasswordLess", emureg::REG_DWORD, &1u32.to_le_bytes()).unwrap();
        let h = emulated_open_at(&root, 0xffff_ffff_8000_0002, &uc, None).unwrap().unwrap();
        unsafe {
            let (mut kind, mut cb, mut d) = (0u32, 2u32, 0u32);
            assert_eq!(emulated_query_at(&root, h, w("PasswordLess").as_ptr(), &mut kind, &mut d as *mut u32 as *mut u8, &mut cb), Some(234));
            assert_eq!(cb, 4);
            assert_eq!(emulated_query_at(&root, h, w("PasswordLess").as_ptr(), &mut kind, &mut d as *mut u32 as *mut u8, &mut cb), Some(0));
            assert_eq!(d, 1);
        }
        assert_eq!(emulated_close(h), Some(0));
        assert!(emulated_open_at(&root, 0xffff_ffff_8000_0002, emureg::IMAGE_CUSTOMIZATION_KEY, None).is_some());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn reg_hooks_serve_hkcu_families_in_console_mode() {
        let root = std::env::temp_dir().join(format!("tombox_cc_hookroot_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::env::set_var("TOMBOX_GDK_ROOT", &root);
        let me = emureg::effective_user().unwrap();
        let rel = format!("{}\\Hook_8wekyb3d8bbwe\\Hook_1.0.0.0_x64__8wekyb3d8bbwe", emureg::FAMILIES);
        let pkg = format!("{me}\\{rel}");
        emureg::create_key(&root, Hive::Hku, &pkg).unwrap();
        emureg::set_value(&root, Hive::Hku, &pkg, "InstallTime", emureg::REG_QWORD, &0x1122_3344_5566_7788u64.to_le_bytes()).unwrap();
        let prev = CONSOLE_MODE.swap(0x64, Ordering::Relaxed);
        unsafe {
            let mut h = 0usize;
            assert_eq!(reg_open_ex_hook(0xffff_ffff_8000_0001, w(&rel).as_ptr(), 0, 0x20019, &mut h), 0);
            assert!(is_emulated_handle(h));
            let (mut kind, mut cb, mut v) = (0u32, 8u32, 0u64);
            assert_eq!(reg_query_hook(h, w("InstallTime").as_ptr(), core::ptr::null_mut(), &mut kind, &mut v as *mut u64 as *mut u8, &mut cb), 0);
            assert_eq!((kind, cb, v), (emureg::REG_QWORD, 8, 0x1122_3344_5566_7788));
            assert_eq!(reg_close_hook(h), 0);
            let mut h2 = 0usize;
            assert_eq!(reg_open_hook(0xffff_ffff_8000_0003, w(&format!("{me}\\{}\\Missing_8wekyb3d8bbwe", emureg::FAMILIES)).as_ptr(), &mut h2), 2);
            let mut fam = 0usize;
            assert_eq!(reg_open_ex_hook(0xffff_ffff_8000_0003, w(&format!("{me}\\{}", emureg::FAMILIES)).as_ptr(), 0, 0x20019, &mut fam), 0);
            let mut child = 0usize;
            assert_eq!(reg_open_ex_hook(fam, w("Hook_8wekyb3d8bbwe\\Hook_1.0.0.0_x64__8wekyb3d8bbwe").as_ptr(), 0, 0x20019, &mut child), 0);
            assert_eq!(reg_query_hook(child, w("InstallTime").as_ptr(), core::ptr::null_mut(), &mut kind, core::ptr::null_mut(), &mut cb), 0);
            assert_eq!(cb, 8);
            assert_eq!(reg_close_hook(child), 0);
            assert_eq!(reg_close_hook(fam), 0);
        }
        CONSOLE_MODE.store(prev, Ordering::Relaxed);
        let _ = std::fs::remove_dir_all(&root);
    }
}
