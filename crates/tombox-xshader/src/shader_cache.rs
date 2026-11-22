use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::fs::{File, OpenOptions};
use std::os::windows::fs::FileExt;
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock, RwLock};

pub const FORMAT: u32 = 1;
const MAGIC: &[u8; 8] = b"TBSHPACK";
const REC: u32 = 0x3152_4254;
const HDR: u64 = 16;
const RHDR: usize = 28;
const MAX_REC: u32 = 64 << 20;

extern "system" {
    fn LockFileEx(h: *mut c_void, flags: u32, reserved: u32, lo: u32, hi: u32, ov: *mut [usize; 4]) -> i32;
    fn UnlockFileEx(h: *mut c_void, reserved: u32, lo: u32, hi: u32, ov: *mut [usize; 4]) -> i32;
    fn GetModuleHandleExW(flags: u32, addr: *const c_void, out: *mut *mut c_void) -> i32;
    fn GetModuleFileNameW(h: *mut c_void, buf: *mut u16, n: u32) -> u32;
}

fn module_path() -> Option<PathBuf> {
    let mut h: *mut c_void = std::ptr::null_mut();
    let mut buf = [0u16; 1024];
    let n = unsafe {
        if GetModuleHandleExW(6, module_path as *const c_void, &mut h) == 0 {
            return None;
        }
        GetModuleFileNameW(h, buf.as_mut_ptr(), buf.len() as u32)
    };
    if n == 0 { None } else { Some(PathBuf::from(String::from_utf16_lossy(&buf[..n as usize]))) }
}

fn emulator_root_from(p: &Path) -> Option<PathBuf> {
    for a in p.ancestors() {
        if a.join("input").is_dir() && a.join("crates").is_dir() {
            return Some(a.to_path_buf());
        }
        let e = a.join("emulator");
        if e.join("input").is_dir() && e.join("crates").is_dir() {
            return Some(e);
        }
    }
    None
}

pub fn root() -> &'static PathBuf {
    static R: OnceLock<PathBuf> = OnceLock::new();
    R.get_or_init(|| {
        let pointer = std::env::temp_dir().join("tombox_shader_cache").join("pack_root.txt");
        let found = module_path().and_then(|m| emulator_root_from(&m)).or_else(|| std::env::current_exe().ok().and_then(|e| emulator_root_from(&e))).map(|e| e.join("input").join("shader_cache"));
        if let Some(f) = &found {
            if std::fs::read_to_string(&pointer).ok().as_deref() != Some(&*f.to_string_lossy()) {
                let _ = std::fs::create_dir_all(pointer.parent().unwrap_or(Path::new(".")));
                let _ = std::fs::write(&pointer, f.to_string_lossy().as_bytes());
            }
        }
        let r = std::env::var_os("TOMBOX_SHADER_CACHE_DIR").map(PathBuf::from).or(found).or_else(|| {
            std::fs::read_to_string(&pointer).ok().map(|s| PathBuf::from(s.trim())).filter(|p| p.is_dir())
        });
        let r = r.unwrap_or_else(|| std::env::temp_dir().join("tombox_shader_cache").join("pack"));
        let _ = std::fs::create_dir_all(&r);
        r
    })
}

pub fn title_dir(title: &str) -> PathBuf {
    root().join("titles").join(title)
}

pub fn src_dir(title: &str) -> PathBuf {
    title_dir(title).join("src")
}

fn fnv(seed: u64, parts: &[&[u8]]) -> u64 {
    let mut h = seed;
    for p in parts {
        for &x in *p {
            h ^= x as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        h ^= 0xff;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn ident_tail(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
        i += 1;
    }
    i
}

fn tb_tokens<'a>(s: &'a str, out: &mut Vec<&'a str>) {
    let b = s.as_bytes();
    let mut i = 0;
    while i + 3 <= b.len() {
        if &b[i..i + 3] == b"tb_" && (i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_')) {
            let e = ident_tail(b, i + 3);
            out.push(&s[i..e]);
            i = e;
        } else {
            i += 1;
        }
    }
}

fn helper_name(line: &str) -> Option<&str> {
    let t = line.trim_end();
    if !t.ends_with('}') || line.starts_with(|c: char| c.is_whitespace() || c == '#') {
        return None;
    }
    let brace = t.find('{')?;
    let paren = t.find('(')?;
    if paren > brace || t[..paren].contains('=') {
        return None;
    }
    let head = t[..paren].trim_end();
    let start = head.rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).map(|k| k + 1).unwrap_or(0);
    let name = &head[start..];
    if !name.starts_with("tb_") || start == 0 {
        return None;
    }
    let mut depth = 0i32;
    for c in t.bytes() {
        match c {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth < 0 {
                    return None;
                }
            }
            _ => {}
        }
    }
    if depth == 0 { Some(name) } else { None }
}

pub fn reduced(hlsl: &str) -> String {
    let lines: Vec<&str> = hlsl.split('\n').collect();
    let names: Vec<Option<&str>> = lines.iter().map(|l| helper_name(l)).collect();
    let mut defs: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, n) in names.iter().enumerate() {
        if let Some(n) = n {
            defs.entry(n).or_default().push(i);
        }
    }
    let mut keep = vec![false; lines.len()];
    let mut work: Vec<&str> = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        if names[i].is_none() {
            keep[i] = true;
            tb_tokens(l, &mut work);
        }
    }
    let mut seen: HashSet<&str> = HashSet::new();
    while let Some(n) = work.pop() {
        if !seen.insert(n) {
            continue;
        }
        if let Some(ix) = defs.get(n) {
            for &i in ix {
                if !keep[i] {
                    keep[i] = true;
                    let l = lines[i];
                    let body = &l[l.find('(').unwrap_or(0)..];
                    tb_tokens(body, &mut work);
                }
            }
        }
    }
    let mut out = String::with_capacity(hlsl.len());
    for (i, l) in lines.iter().enumerate() {
        if keep[i] {
            out.push_str(l);
            out.push('\n');
        }
    }
    out
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Fma {
    pub preserve: bool,
    pub any: bool,
    pub hybrid: bool,
    pub f2h: bool,
}

pub fn key(final_hlsl: &str, profile: &str, fma: Fma, extra: &str) -> u128 {
    let r = reduced(final_hlsl);
    let flags = [fma.preserve as u8, fma.any as u8, fma.hybrid as u8, fma.f2h as u8];
    let parts: [&[u8]; 7] = [&FORMAT.to_le_bytes(), profile.as_bytes(), crate::dxc::COMPILE_FLAVOR.as_bytes(), &flags, crate::dxc::identity().as_bytes(), extra.as_bytes(), r.as_bytes()];
    let a = fnv(0xcbf2_9ce4_8422_2325, &parts);
    let b = fnv(0x84222325_cbf29ce4 ^ 0x9e37_79b9_7f4a_7c15, &parts);
    ((a as u128) << 64) | b as u128
}

fn crc32(parts: &[&[u8]]) -> u32 {
    static T: OnceLock<[u32; 256]> = OnceLock::new();
    let t = T.get_or_init(|| {
        let mut t = [0u32; 256];
        for i in 0..256u32 {
            let mut c = i;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
            }
            t[i as usize] = c;
        }
        t
    });
    let mut c = 0xffff_ffffu32;
    for p in parts {
        for &b in *p {
            c = t[((c ^ b as u32) & 0xff) as usize] ^ (c >> 8);
        }
    }
    !c
}

struct Pack {
    file: File,
    lockf: File,
    mx: Mutex<()>,
    index: RwLock<HashMap<u128, (u64, u32)>>,
    scanned: Mutex<u64>,
    path: PathBuf,
}

pub struct Stats {
    pub records: usize,
    pub truncated: u64,
    pub hits: u64,
    pub misses: u64,
    pub bad_crc: u64,
    pub writes: u64,
}

static HITS: AtomicU64 = AtomicU64::new(0);
static MISSES: AtomicU64 = AtomicU64::new(0);
static BAD: AtomicU64 = AtomicU64::new(0);
static WRITES: AtomicU64 = AtomicU64::new(0);
static TRUNC: AtomicU64 = AtomicU64::new(0);

struct Lock<'a>(&'a File, [usize; 4], std::sync::MutexGuard<'a, ()>);

impl<'a> Lock<'a> {
    fn new(p: &'a Pack) -> Lock<'a> {
        let g = p.mx.lock().unwrap_or_else(|e| e.into_inner());
        let mut ov = [0usize, 0, 0xffff_0000usize | (0x7fff_ffffusize << 32), 0];
        unsafe { LockFileEx(p.lockf.as_raw_handle() as *mut c_void, 2, 0, 1, 0, &mut ov) };
        Lock(&p.lockf, ov, g)
    }
}

impl Drop for Lock<'_> {
    fn drop(&mut self) {
        unsafe { UnlockFileEx(self.0.as_raw_handle() as *mut c_void, 0, 1, 0, &mut self.1) };
    }
}

impl Pack {
    fn open(path: PathBuf) -> Result<Pack, String> {
        let file = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let lockf = OpenOptions::new().read(true).write(true).open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let p = Pack { file, lockf, mx: Mutex::new(()), index: RwLock::new(HashMap::new()), scanned: Mutex::new(0), path };
        {
            let _l = Lock::new(&p);
            let len = p.file.metadata().map(|m| m.len()).unwrap_or(0);
            let mut h = [0u8; 16];
            let ok = len >= HDR && p.file.seek_read(&mut h, 0).ok() == Some(16) && &h[..8] == MAGIC && u32::from_le_bytes(h[8..12].try_into().unwrap()) == FORMAT;
            if !ok {
                let mut hd = [0u8; 16];
                hd[..8].copy_from_slice(MAGIC);
                hd[8..12].copy_from_slice(&FORMAT.to_le_bytes());
                p.file.set_len(0).map_err(|e| e.to_string())?;
                p.file.seek_write(&hd, 0).map_err(|e| e.to_string())?;
                *p.scanned.lock().unwrap() = HDR;
            } else {
                *p.scanned.lock().unwrap() = HDR;
                p.scan_locked(true);
            }
        }
        Ok(p)
    }

    fn scan_locked(&self, truncate: bool) {
        let len = self.file.metadata().map(|m| m.len()).unwrap_or(0);
        let mut off = *self.scanned.lock().unwrap();
        let mut found: Vec<(u128, u64, u32)> = Vec::new();
        let mut hdr = [0u8; RHDR];
        while off + RHDR as u64 <= len {
            if self.file.seek_read(&mut hdr, off).ok() != Some(RHDR) {
                break;
            }
            let magic = u32::from_le_bytes(hdr[0..4].try_into().unwrap());
            let n = u32::from_le_bytes(hdr[4..8].try_into().unwrap());
            if magic != REC || n == 0 || n > MAX_REC || off + RHDR as u64 + n as u64 > len {
                break;
            }
            let k = u128::from_le_bytes(hdr[8..24].try_into().unwrap());
            found.push((k, off, n));
            off += RHDR as u64 + n as u64;
        }
        if off < len && truncate {
            TRUNC.fetch_add(len - off, Ordering::Relaxed);
            let _ = self.file.set_len(off);
        }
        *self.scanned.lock().unwrap() = off;
        let mut ix = self.index.write().unwrap();
        for (k, o, n) in found {
            ix.insert(k, (o, n));
        }
    }

    fn read(&self, k: u128) -> Option<Vec<u8>> {
        let (off, n) = *self.index.read().unwrap().get(&k)?;
        let mut buf = vec![0u8; RHDR + n as usize];
        let mut got = 0;
        while got < buf.len() {
            match self.file.seek_read(&mut buf[got..], off + got as u64) {
                Ok(0) | Err(_) => return None,
                Ok(m) => got += m,
            }
        }
        let crc = u32::from_le_bytes(buf[24..28].try_into().unwrap());
        if u128::from_le_bytes(buf[8..24].try_into().unwrap()) != k || crc32(&[&buf[8..24], &buf[RHDR..]]) != crc {
            BAD.fetch_add(1, Ordering::Relaxed);
            self.index.write().unwrap().remove(&k);
            return None;
        }
        buf.drain(..RHDR);
        Some(buf)
    }

    fn append(&self, k: u128, data: &[u8]) -> Result<(), String> {
        if data.is_empty() || data.len() > MAX_REC as usize {
            return Err(format!("record size {} out of range", data.len()));
        }
        let mut rec = Vec::with_capacity(RHDR + data.len());
        rec.extend_from_slice(&REC.to_le_bytes());
        rec.extend_from_slice(&(data.len() as u32).to_le_bytes());
        rec.extend_from_slice(&k.to_le_bytes());
        rec.extend_from_slice(&crc32(&[&k.to_le_bytes(), data]).to_le_bytes());
        rec.extend_from_slice(data);
        let _l = Lock::new(self);
        self.scan_locked(true);
        let off = *self.scanned.lock().unwrap();
        let mut put = 0;
        while put < rec.len() {
            match self.file.seek_write(&rec[put..], off + put as u64) {
                Ok(0) => return Err("short write".into()),
                Ok(m) => put += m,
                Err(e) => return Err(format!("{}: {e}", self.path.display())),
            }
        }
        *self.scanned.lock().unwrap() = off + rec.len() as u64;
        self.index.write().unwrap().insert(k, (off, data.len() as u32));
        WRITES.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

fn pack() -> Option<&'static Pack> {
    static P: OnceLock<Option<Pack>> = OnceLock::new();
    P.get_or_init(|| Pack::open(root().join(format!("dxil-v{FORMAT}.pack"))).ok()).as_ref()
}

pub fn pack_path() -> PathBuf {
    root().join(format!("dxil-v{FORMAT}.pack"))
}

pub fn get(k: u128) -> Option<Vec<u8>> {
    let p = pack()?;
    let mut r = p.read(k);
    if r.is_none() && p.file.metadata().map(|m| m.len()).unwrap_or(0) > *p.scanned.lock().unwrap() {
        let _l = Lock::new(p);
        p.scan_locked(false);
        r = p.read(k);
    }
    match &r {
        Some(_) => HITS.fetch_add(1, Ordering::Relaxed),
        None => MISSES.fetch_add(1, Ordering::Relaxed),
    };
    r
}

pub fn contains(k: u128) -> bool {
    pack().is_some_and(|p| p.index.read().unwrap().contains_key(&k))
}

pub fn put(k: u128, data: &[u8]) -> Result<(), String> {
    pack().ok_or("shader pack unavailable")?.append(k, data)
}

pub fn stats() -> Stats {
    Stats {
        records: pack().map(|p| p.index.read().unwrap().len()).unwrap_or(0),
        truncated: TRUNC.load(Ordering::Relaxed),
        hits: HITS.load(Ordering::Relaxed),
        misses: MISSES.load(Ordering::Relaxed),
        bad_crc: BAD.load(Ordering::Relaxed),
        writes: WRITES.load(Ordering::Relaxed),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Pack,
    Compiled,
}

pub struct Compiled {
    pub code: Vec<u8>,
    pub source: Source,
    pub dxc_ms: f64,
    pub lookup_ms: f64,
}

pub fn compile_cached(final_hlsl: &str, profile: &str, fma: Fma, extra: &[&str]) -> Result<Compiled, String> {
    let t0 = std::time::Instant::now();
    let k = key(final_hlsl, profile, fma, &extra.join(" "));
    if let Some(code) = get(k) {
        return Ok(Compiled { code, source: Source::Pack, dxc_ms: 0.0, lookup_ms: t0.elapsed().as_secs_f64() * 1000.0 });
    }
    let lookup_ms = t0.elapsed().as_secs_f64() * 1000.0;
    let t1 = std::time::Instant::now();
    let code = crate::dxc::compile_fma(final_hlsl, profile, fma, extra)?;
    let dxc_ms = t1.elapsed().as_secs_f64() * 1000.0;
    let _ = put(k, &code);
    Ok(Compiled { code, source: Source::Compiled, dxc_ms, lookup_ms })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unused_helpers_do_not_change_the_reduced_text() {
        let a = "float tb_a(float x) { return tb_b(x); }\nfloat tb_b(float x) { return x; }\nfloat tb_u(float x) { return x * 2.0; }\n#define M 1\nfloat4 main() : SV_Target { return tb_a(1.0); }";
        let b = a.replace("x * 2.0", "x * 3.0");
        assert_eq!(reduced(a), reduced(&b));
        assert!(reduced(a).contains("tb_b(float x)"));
        assert!(!reduced(a).contains("tb_u"));
        let c = a.replace("return x; }", "return -x; }");
        assert_ne!(reduced(a), reduced(&c));
    }

    #[test]
    fn multi_line_and_non_function_lines_are_kept() {
        let a = "static const uint tb_t[2] = { 1u, 2u };\nfloat tb_m(float x)\n{ return x; }\nuint tb_k(uint i) { return tb_t[i]; }\nvoid main() {}";
        let r = reduced(a);
        assert!(r.contains("tb_t[2]"));
        assert!(r.contains("float tb_m(float x)"));
        assert!(!r.contains("tb_k"));
    }

    #[test]
    fn concurrent_appends_and_reads_do_not_deadlock() {
        let dir = std::env::temp_dir().join(format!("tb_pack_mt_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("m.pack");
        let _ = std::fs::remove_file(&path);
        let p = Pack::open(path.clone()).unwrap();
        std::thread::scope(|s| {
            for t in 0..8u128 {
                let p = &p;
                s.spawn(move || {
                    for i in 0..200u128 {
                        let k = t * 1000 + i;
                        p.append(k, &vec![(i & 0xff) as u8; 3000]).unwrap();
                        assert_eq!(p.read(k).unwrap().len(), 3000);
                    }
                });
            }
        });
        let q = Pack::open(path.clone()).unwrap();
        assert_eq!(q.index.read().unwrap().len(), 1600);
        drop((p, q));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pack_round_trip_and_truncates_a_torn_tail() {
        let dir = std::env::temp_dir().join(format!("tb_pack_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("t.pack");
        let _ = std::fs::remove_file(&path);
        {
            let p = Pack::open(path.clone()).unwrap();
            p.append(7, b"hello").unwrap();
            p.append(9, b"world!").unwrap();
            assert_eq!(p.read(7).as_deref(), Some(&b"hello"[..]));
        }
        let len = std::fs::metadata(&path).unwrap().len();
        {
            let f = OpenOptions::new().write(true).open(&path).unwrap();
            f.seek_write(&[0x54, 0x42, 0x52, 0x31, 50, 0, 0, 0, 1, 2], len).unwrap();
        }
        {
            let p = Pack::open(path.clone()).unwrap();
            assert_eq!(std::fs::metadata(&path).unwrap().len(), len);
            assert_eq!(p.read(9).as_deref(), Some(&b"world!"[..]));
            let f = OpenOptions::new().write(true).open(&path).unwrap();
            f.seek_write(b"X", len - 1).unwrap();
            assert!(p.read(9).is_none());
            assert_eq!(p.read(7).as_deref(), Some(&b"hello"[..]));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
