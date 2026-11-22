use compat_common::record;
use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Mutex;

#[derive(Clone, Default)]
pub struct Snapshot {
    pub cpu_name: Option<String>,
    pub cpu_usage: Option<f32>,
    pub cpu_temp: Option<f32>,
    pub cpu_mhz: Option<f32>,
    pub gpu_name: Option<String>,
    pub gpu_usage: Option<f32>,
    pub gpu_temp: Option<f32>,
    pub gpu_mhz: Option<f32>,
    pub vram_mb: Option<f32>,
}

static SNAP: Mutex<Option<Snapshot>> = Mutex::new(None);
static STARTED: AtomicBool = AtomicBool::new(false);
static LAST_WANTED: AtomicI64 = AtomicI64::new(0);

extern "system" {
    fn LoadLibraryW(name: *const u16) -> *mut c_void;
    fn LoadLibraryExW(name: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
    fn GetProcAddress(h: *mut c_void, name: *const u8) -> *mut c_void;
    fn GetSystemTimes(idle: *mut u64, kernel: *mut u64, user: *mut u64) -> i32;
    fn GetTickCount64() -> u64;
    fn Sleep(ms: u32);
}

#[link(name = "advapi32")]
extern "system" {
    fn RegGetValueW(key: isize, sub: *const u16, value: *const u16, flags: u32, ty: *mut u32, data: *mut u16, size: *mut u32) -> i32;
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

pub fn latest() -> Option<Snapshot> {
    LAST_WANTED.store(unsafe { GetTickCount64() } as i64, Ordering::Relaxed);
    if !STARTED.swap(true, Ordering::Relaxed) {
        let _ = std::thread::Builder::new().name("tombox-overlay-sensors".into()).spawn(run);
    }
    SNAP.try_lock().ok().and_then(|g| g.clone())
}

fn short_cpu(s: &str) -> String {
    let s = s.replace("(R)", "").replace("(TM)", "").replace("(tm)", "");
    let mut out: Vec<&str> = Vec::new();
    for t in s.split_whitespace() {
        if t.starts_with('@') {
            break;
        }
        let lower = t.to_ascii_lowercase();
        let ordinal = lower.len() > 2 && lower[..lower.len() - 2].chars().all(|c| c.is_ascii_digit()) && ["st", "nd", "rd", "th"].contains(&&lower[lower.len() - 2..]);
        if ordinal || ["processor", "cpu", "core", "gen"].contains(&lower.as_str()) || lower.ends_with("-core") {
            continue;
        }
        out.push(t);
    }
    out.join(" ")
}

fn short_gpu(s: &str) -> String {
    s.split_whitespace().filter(|t| !["NVIDIA", "GeForce", "AMD", "(R)", "Intel(R)"].contains(t)).collect::<Vec<_>>().join(" ")
}

fn cpu_name() -> Option<String> {
    let sub = wide("HARDWARE\\DESCRIPTION\\System\\CentralProcessor\\0");
    let val = wide("ProcessorNameString");
    let mut buf = [0u16; 256];
    let mut size = (buf.len() * 2) as u32;
    let rc = unsafe { RegGetValueW(0x8000_0002u32 as i32 as isize, sub.as_ptr(), val.as_ptr(), 2, core::ptr::null_mut(), buf.as_mut_ptr(), &mut size) };
    if rc != 0 {
        return None;
    }
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    let s = short_cpu(String::from_utf16_lossy(&buf[..n]).trim());
    (!s.is_empty()).then_some(s)
}

type PdhOpen = unsafe extern "system" fn(*const u16, usize, *mut isize) -> i32;
type PdhAdd = unsafe extern "system" fn(isize, *const u16, usize, *mut isize) -> i32;
type PdhCollect = unsafe extern "system" fn(isize) -> i32;
type PdhValue = unsafe extern "system" fn(isize, u32, *mut u32, *mut [u64; 2]) -> i32;
type PdhArray = unsafe extern "system" fn(isize, u32, *mut u32, *mut u32, *mut u8) -> i32;

struct Pdh {
    collect: PdhCollect,
    value: PdhValue,
    array: PdhArray,
    query: isize,
    perf: isize,
    freq: isize,
    thermal: isize,
}

const PDH_FMT_DOUBLE: u32 = 0x200;

impl Pdh {
    fn open() -> Option<Pdh> {
        unsafe {
            let h = LoadLibraryExW(wide("pdh.dll").as_ptr(), core::ptr::null_mut(), 0x800);
            if h.is_null() {
                return None;
            }
            let p = |n: &[u8]| GetProcAddress(h, n.as_ptr());
            let (open, add, collect, value, array) = (p(b"PdhOpenQueryW\0"), p(b"PdhAddEnglishCounterW\0"), p(b"PdhCollectQueryData\0"), p(b"PdhGetFormattedCounterValue\0"), p(b"PdhGetFormattedCounterArrayW\0"));
            if [open, add, collect, value, array].iter().any(|f| f.is_null()) {
                return None;
            }
            let open: PdhOpen = core::mem::transmute(open);
            let add: PdhAdd = core::mem::transmute(add);
            let mut query = 0isize;
            if open(core::ptr::null(), 0, &mut query) != 0 {
                return None;
            }
            let counter = |path: &str| {
                let mut c = 0isize;
                if add(query, wide(path).as_ptr(), 0, &mut c) == 0 { c } else { 0 }
            };
            let perf = counter("\\Processor Information(_Total)\\% Processor Performance");
            let freq = counter("\\Processor Information(_Total)\\Processor Frequency");
            let thermal = counter("\\Thermal Zone Information(*)\\Temperature");
            let pdh = Pdh { collect: core::mem::transmute(collect), value: core::mem::transmute(value), array: core::mem::transmute(array), query, perf, freq, thermal };
            (pdh.collect)(query);
            Some(pdh)
        }
    }

    fn read(&self, c: isize) -> Option<f64> {
        if c == 0 {
            return None;
        }
        let mut v = [0u64; 2];
        let rc = unsafe { (self.value)(c, PDH_FMT_DOUBLE, core::ptr::null_mut(), &mut v) };
        let status = v[0] as u32;
        (rc == 0 && status <= 1).then(|| f64::from_bits(v[1]))
    }

    fn max_of_array(&self, c: isize) -> Option<f64> {
        if c == 0 {
            return None;
        }
        let (mut size, mut count) = (0u32, 0u32);
        unsafe { (self.array)(c, PDH_FMT_DOUBLE, &mut size, &mut count, core::ptr::null_mut()) };
        if size == 0 {
            return None;
        }
        let mut buf = vec![0u8; size as usize + 64];
        if unsafe { (self.array)(c, PDH_FMT_DOUBLE, &mut size, &mut count, buf.as_mut_ptr()) } != 0 {
            return None;
        }
        let mut best: Option<f64> = None;
        for i in 0..count as usize {
            let o = i * 24;
            let status = u32::from_le_bytes(buf[o + 8..o + 12].try_into().ok()?);
            let v = f64::from_le_bytes(buf[o + 16..o + 24].try_into().ok()?);
            if status <= 1 && v > 0.0 {
                best = Some(best.map_or(v, |b: f64| b.max(v)));
            }
        }
        best
    }
}

fn power_info_mhz() -> Option<(f32, f32)> {
    type Cnpi = unsafe extern "system" fn(u32, *const c_void, u32, *mut u8, u32) -> i32;
    unsafe {
        let h = LoadLibraryExW(wide("powrprof.dll").as_ptr(), core::ptr::null_mut(), 0x800);
        if h.is_null() {
            return None;
        }
        let f = GetProcAddress(h, b"CallNtPowerInformation\0".as_ptr());
        if f.is_null() {
            return None;
        }
        let f: Cnpi = core::mem::transmute(f);
        let n = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1).max(1);
        let mut buf = vec![0u8; n * 24];
        if f(11, core::ptr::null(), 0, buf.as_mut_ptr(), buf.len() as u32) != 0 {
            return None;
        }
        let mut max_mhz = 0u32;
        let mut cur = 0u32;
        for i in 0..n {
            let o = i * 24;
            max_mhz = max_mhz.max(u32::from_le_bytes(buf[o + 4..o + 8].try_into().ok()?));
            cur = cur.max(u32::from_le_bytes(buf[o + 8..o + 12].try_into().ok()?));
        }
        (max_mhz > 0).then_some((max_mhz as f32, cur as f32))
    }
}

type NvmlInit = unsafe extern "C" fn() -> i32;
type NvmlHandle = unsafe extern "C" fn(u32, *mut usize) -> i32;
type NvmlName = unsafe extern "C" fn(usize, *mut u8, u32) -> i32;
type NvmlUtil = unsafe extern "C" fn(usize, *mut [u32; 2]) -> i32;
type NvmlTemp = unsafe extern "C" fn(usize, u32, *mut u32) -> i32;
type NvmlClock = unsafe extern "C" fn(usize, u32, *mut u32) -> i32;
type NvmlMem = unsafe extern "C" fn(usize, *mut [u64; 3]) -> i32;

struct Nvml {
    dev: usize,
    name: String,
    util: Option<NvmlUtil>,
    temp: Option<NvmlTemp>,
    clock: Option<NvmlClock>,
    mem: Option<NvmlMem>,
}

impl Nvml {
    fn open() -> Option<Nvml> {
        unsafe {
            let mut h = LoadLibraryExW(wide("nvml.dll").as_ptr(), core::ptr::null_mut(), 0x800);
            if h.is_null() {
                h = LoadLibraryW(wide("C:\\Program Files\\NVIDIA Corporation\\NVSMI\\nvml.dll").as_ptr());
            }
            if h.is_null() {
                return None;
            }
            let p = |n: &[u8]| {
                let f = GetProcAddress(h, n.as_ptr());
                (!f.is_null()).then_some(f)
            };
            let init: NvmlInit = core::mem::transmute(p(b"nvmlInit_v2\0")?);
            if init() != 0 {
                return None;
            }
            let get: NvmlHandle = core::mem::transmute(p(b"nvmlDeviceGetHandleByIndex_v2\0")?);
            let mut dev = 0usize;
            if get(0, &mut dev) != 0 || dev == 0 {
                return None;
            }
            let mut name = String::new();
            if let Some(f) = p(b"nvmlDeviceGetName\0") {
                let f: NvmlName = core::mem::transmute(f);
                let mut b = [0u8; 96];
                if f(dev, b.as_mut_ptr(), b.len() as u32) == 0 {
                    let n = b.iter().position(|&c| c == 0).unwrap_or(b.len());
                    name = short_gpu(&String::from_utf8_lossy(&b[..n]));
                }
            }
            Some(Nvml {
                dev,
                name,
                util: p(b"nvmlDeviceGetUtilizationRates\0").map(|f| core::mem::transmute::<_, NvmlUtil>(f)),
                temp: p(b"nvmlDeviceGetTemperature\0").map(|f| core::mem::transmute::<_, NvmlTemp>(f)),
                clock: p(b"nvmlDeviceGetClockInfo\0").map(|f| core::mem::transmute::<_, NvmlClock>(f)),
                mem: p(b"nvmlDeviceGetMemoryInfo\0").map(|f| core::mem::transmute::<_, NvmlMem>(f)),
            })
        }
    }

    fn poll(&self, s: &mut Snapshot) {
        unsafe {
            s.gpu_name = (!self.name.is_empty()).then(|| self.name.clone());
            s.gpu_usage = self.util.and_then(|f| {
                let mut u = [0u32; 2];
                (f(self.dev, &mut u) == 0).then_some(u[0] as f32)
            });
            s.gpu_temp = self.temp.and_then(|f| {
                let mut t = 0u32;
                (f(self.dev, 0, &mut t) == 0).then_some(t as f32)
            });
            s.gpu_mhz = self.clock.and_then(|f| {
                let mut c = 0u32;
                (f(self.dev, 0, &mut c) == 0).then_some(c as f32)
            });
            s.vram_mb = self.mem.and_then(|f| {
                let mut m = [0u64; 3];
                (f(self.dev, &mut m) == 0).then_some(m[2] as f32 / (1024.0 * 1024.0))
            });
        }
    }
}

#[repr(C)]
struct Guid(u32, u16, u16, [u8; 8]);
const IID_FACTORY1: Guid = Guid(0x770aae78, 0xf26f, 0x4dba, [0xa8, 0x29, 0x25, 0x3c, 0x83, 0xd1, 0xb3, 0x87]);
const IID_ADAPTER3: Guid = Guid(0x645967a4, 0x1392, 0x4310, [0xa7, 0x98, 0x80, 0x53, 0xce, 0x3e, 0x93, 0xfd]);

unsafe fn vslot(o: *mut c_void, i: usize) -> usize {
    unsafe { *(*(o as *const *const usize)).add(i) }
}

struct Dxgi {
    adapter: *mut c_void,
    name: String,
}

impl Dxgi {
    fn open() -> Option<Dxgi> {
        unsafe {
            let h = LoadLibraryExW(wide("dxgi.dll").as_ptr(), core::ptr::null_mut(), 0x800);
            if h.is_null() {
                return None;
            }
            let f = GetProcAddress(h, b"CreateDXGIFactory1\0".as_ptr());
            if f.is_null() {
                return None;
            }
            let f: unsafe extern "system" fn(*const Guid, *mut *mut c_void) -> i32 = core::mem::transmute(f);
            let mut fac: *mut c_void = core::ptr::null_mut();
            if f(&IID_FACTORY1, &mut fac) < 0 || fac.is_null() {
                return None;
            }
            let enum1: unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> i32 = core::mem::transmute(vslot(fac, 12));
            let release = |o: *mut c_void| {
                let r: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(vslot(o, 2));
                r(o);
            };
            let mut found = None;
            for i in 0..8u32 {
                let mut a: *mut c_void = core::ptr::null_mut();
                if enum1(fac, i, &mut a) < 0 || a.is_null() {
                    break;
                }
                let desc1: unsafe extern "system" fn(*mut c_void, *mut [u8; 312]) -> i32 = core::mem::transmute(vslot(a, 10));
                let mut d = [0u8; 312];
                let ok = desc1(a, &mut d) >= 0;
                let flags = u32::from_le_bytes(d[304..308].try_into().unwrap());
                if !ok || flags & 2 != 0 {
                    release(a);
                    continue;
                }
                let qi: unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32 = core::mem::transmute(vslot(a, 0));
                let mut a3: *mut c_void = core::ptr::null_mut();
                let hr = qi(a, &IID_ADAPTER3, &mut a3);
                release(a);
                if hr < 0 || a3.is_null() {
                    continue;
                }
                let w: Vec<u16> = d[..256].chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect();
                found = Some(Dxgi { adapter: a3, name: short_gpu(&String::from_utf16_lossy(&w)) });
                break;
            }
            release(fac);
            found
        }
    }

    fn poll(&self, s: &mut Snapshot) {
        unsafe {
            s.gpu_name = (!self.name.is_empty()).then(|| self.name.clone());
            let q: unsafe extern "system" fn(*mut c_void, u32, u32, *mut [u64; 4]) -> i32 = core::mem::transmute(vslot(self.adapter, 14));
            let mut info = [0u64; 4];
            s.vram_mb = (q(self.adapter, 0, 0, &mut info) >= 0).then_some(info[1] as f32 / (1024.0 * 1024.0));
        }
    }
}

static USE_PDH: crate::live_sw::Sw = crate::live_sw::Sw::new("TOMBOX_OVERLAY_PDH", "overlay_pdh", true);
static USE_NVML: crate::live_sw::Sw = crate::live_sw::Sw::new("TOMBOX_OVERLAY_NVML", "overlay_nvml", true);

fn timed<T>(what: &str, f: impl FnOnce() -> Option<T>) -> Option<T> {
    let t0 = crate::frame::qpc();
    let r = f();
    let ms = (crate::frame::qpc() - t0) as f64 * 1000.0 / crate::frame::qpf() as f64;
    record("d3d12_xs", "[overlay]", &format!("sensor init {what}: {} in {ms:.1} ms", if r.is_some() { "ok" } else { "unavailable" }));
    r
}

fn run() {
    let cpu = timed("registry cpu name", cpu_name);
    let power = timed("CallNtPowerInformation", power_info_mhz);
    let mut pdh: Option<Option<Pdh>> = None;
    let mut nvml: Option<Option<Nvml>> = None;
    let mut dxgi: Option<Option<Dxgi>> = None;
    let mut prev: Option<(u64, u64, u64)> = None;
    let mut reported = false;
    loop {
        let now = unsafe { GetTickCount64() } as i64;
        if now - LAST_WANTED.load(Ordering::Relaxed) > 3000 {
            unsafe { Sleep(500) };
            continue;
        }
        let (use_pdh, use_nvml) = (USE_PDH.on(), USE_NVML.on());
        if use_pdh && pdh.is_none() {
            pdh = Some(timed("pdh", Pdh::open));
        }
        if use_nvml && nvml.is_none() {
            nvml = Some(timed("nvml", Nvml::open));
            if nvml.as_ref().is_some_and(|n| n.is_none()) {
                dxgi = Some(timed("dxgi", Dxgi::open));
            }
        }
        let pdh_now = if use_pdh { pdh.as_ref().and_then(|p| p.as_ref()) } else { None };
        let nvml_now = if use_nvml { nvml.as_ref().and_then(|n| n.as_ref()) } else { None };
        let dxgi_now = if use_nvml { dxgi.as_ref().and_then(|d| d.as_ref()) } else { None };
        let mut s = Snapshot { cpu_name: cpu.clone(), ..Default::default() };
        let (mut idle, mut kernel, mut user) = (0u64, 0u64, 0u64);
        if unsafe { GetSystemTimes(&mut idle, &mut kernel, &mut user) } != 0 {
            if let Some((pi, pk, pu)) = prev {
                let total = (kernel - pk) + (user - pu);
                if total > 0 {
                    s.cpu_usage = Some((1.0 - (idle - pi) as f64 / total as f64).clamp(0.0, 1.0) as f32 * 100.0);
                }
            }
            prev = Some((idle, kernel, user));
        }
        let mut clock_src = "--";
        let mut temp_src = "--";
        if let Some(p) = pdh_now {
            unsafe { (p.collect)(p.query) };
            let base = p.read(p.freq).filter(|&f| f > 0.0).or(power.map(|(m, _)| m as f64));
            if let (Some(pct), Some(base)) = (p.read(p.perf), base) {
                s.cpu_mhz = Some((pct * base / 100.0) as f32);
                clock_src = "pdh % Processor Performance x base";
            }
            if let Some(k) = p.max_of_array(p.thermal) {
                s.cpu_temp = Some((k - 273.15) as f32);
                temp_src = "pdh Thermal Zone Information (ACPI)";
            }
        }
        if s.cpu_mhz.is_none() {
            if let Some((_, cur)) = power.filter(|&(_, c)| c > 0.0).and_then(|_| power_info_mhz()) {
                s.cpu_mhz = Some(cur);
                clock_src = "CallNtPowerInformation CurrentMhz";
            }
        }
        let gpu_src = if let Some(n) = nvml_now {
            n.poll(&mut s);
            "nvml"
        } else if let Some(d) = dxgi_now {
            d.poll(&mut s);
            "dxgi QueryVideoMemoryInfo (process VRAM only)"
        } else {
            "--"
        };
        if !reported && prev.is_some() && s.cpu_usage.is_some() {
            reported = true;
            let f = |v: Option<f32>| v.map_or("--".to_string(), |v| format!("{v:.0}"));
            record("d3d12_xs", "[overlay]", &format!(
                "sensors: cpu='{}' usage={}% clock={}MHz ({clock_src}) temp={}C ({temp_src}) gpu='{}' via {gpu_src}: usage={}% temp={}C clock={}MHz vram={}MB",
                s.cpu_name.as_deref().unwrap_or("--"), f(s.cpu_usage), f(s.cpu_mhz), f(s.cpu_temp), s.gpu_name.as_deref().unwrap_or("--"),
                f(s.gpu_usage), f(s.gpu_temp), f(s.gpu_mhz), f(s.vram_mb)));
        }
        *SNAP.lock().unwrap() = Some(s);
        unsafe { Sleep(1000) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_shorten() {
        assert_eq!(short_cpu("12th Gen Intel(R) Core(TM) i7-12700KF"), "Intel i7-12700KF");
        assert_eq!(short_cpu("AMD Ryzen 7 9800X3D 8-Core Processor"), "AMD Ryzen 7 9800X3D");
        assert_eq!(short_gpu("NVIDIA GeForce RTX 5080"), "RTX 5080");
    }
}
