use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tombox_xshader::{cbv_raw_hlsl, dxc, extract, fma_probe, gcn, hlsl, parse_root_signature, runtime_hlsl, shader_cache, Stage};

fn stage_name(s: &Stage) -> &'static str {
    match s {
        Stage::Vertex => "vs",
        Stage::Pixel => "ps",
        Stage::Compute => "cs",
        Stage::Geometry => "gs",
        Stage::Hull => "hs",
        Stage::Domain => "ds",
        Stage::Mesh => "ms",
        Stage::Amplification => "as",
        Stage::Unknown(_) => "??",
    }
}

fn parse_name(name: &str) -> Option<(u64, [u8; 8])> {
    let stem = name.strip_suffix(".xbox")?;
    let (h, f) = stem.split_once('_')?;
    if h.len() != 16 || f.len() != 16 {
        return None;
    }
    let hash = u64::from_str_radix(h, 16).ok()?;
    let mut rt = [0u8; 8];
    for (i, b) in rt.iter_mut().enumerate() {
        *b = u8::from_str_radix(&f[2 * i..2 * i + 2], 16).ok()?;
    }
    Some((hash, rt))
}

fn rt_hex(rt: &[u8; 8]) -> String {
    rt.iter().map(|b| format!("{b:02x}")).collect()
}

struct Item {
    path: PathBuf,
    hash: u64,
    rt: [u8; 8],
    root: Option<PathBuf>,
}

fn gather(dir: &Path, out: &mut Vec<Item>, seen: &mut HashSet<(u64, [u8; 8], String)>) -> usize {
    let Ok(rd) = std::fs::read_dir(dir) else { return 0 };
    let mut files = Vec::new();
    let mut roots: HashMap<String, Vec<PathBuf>> = HashMap::new();
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            continue;
        }
        let n = e.file_name().to_string_lossy().into_owned();
        if let Some(s) = n.strip_suffix(".root") {
            roots.entry(s.split('.').next().unwrap_or(s).to_string()).or_default().push(p);
        } else {
            files.push((n, p));
        }
    }
    let mut n = 0;
    for (name, p) in files {
        let Some((hash, rt)) = parse_name(&name) else { continue };
        let stem = name.trim_end_matches(".xbox").to_string();
        let rl: Vec<Option<PathBuf>> = match roots.get(&stem) {
            Some(v) if !v.is_empty() => v.iter().cloned().map(Some).collect(),
            _ => vec![None],
        };
        for r in rl {
            let rk = r.as_ref().map(|p| p.file_name().unwrap_or_default().to_string_lossy().into_owned()).unwrap_or_default();
            if seen.insert((hash, rt, rk)) {
                out.push(Item { path: p.clone(), hash, rt, root: r });
                n += 1;
            }
        }
    }
    n
}

enum Prep {
    Key { st: &'static str, k: u128, src: String, profile: String },
    Fail { phase: &'static str, st: &'static str, reason: String },
    RuntimeOnly { st: &'static str, reason: &'static str },
}

fn prepare(it: &Item, loop_grad: bool, fma: shader_cache::Fma) -> Prep {
    let Ok(blob) = std::fs::read(&it.path) else {
        return Prep::Fail { phase: "read", st: "??", reason: format!("cannot read {}", it.path.display()) };
    };
    let root = it.root.as_ref().and_then(|p| std::fs::read(p).ok());
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Prep {
        let mut ex = match extract(&blob) {
            Ok(e) => e,
            Err(e) => return Prep::Fail { phase: "extract", st: "??", reason: format!("extract: {e}") },
        };
        let st = stage_name(&ex.stage);
        if !ex.has_root_sig && root.is_none() && matches!(ex.stage, Stage::Mesh | Stage::Amplification) {
            return Prep::RuntimeOnly { st, reason: "mesh/amplification shader without a dumped PSO root signature" };
        }
        if let Some(root) = root.as_ref().filter(|_| !ex.has_root_sig) {
            match parse_root_signature(root) {
                Ok(rs) => ex.root_sig = Some(rs),
                Err(e) => return Prep::Fail { phase: "extract", st, reason: format!("pipeline root signature: {e}") },
            }
        }
        let mut t = match hlsl::translate(&ex) {
            Ok(t) => t,
            Err(e) => return Prep::Fail { phase: "translate", st, reason: format!("translate: {e}") },
        };
        if gcn::is_durango(&ex.fssm) {
            match cbv_raw_hlsl::rewrite_hlsl(&t.hlsl) {
                Ok(h) => t.hlsl = h,
                Err(e) => return Prep::Fail { phase: "translate", st, reason: format!("bind: {e}") },
            }
        }
        let src = runtime_hlsl::finalize(&t.hlsl, &it.rt, loop_grad);
        let profile = dxc::profile_for(t.profile);
        let k = shader_cache::key(&src, &profile, fma, "");
        Prep::Key { st, k, src, profile }
    }));
    r.unwrap_or_else(|p| {
        let msg = p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_else(|| "panic".into());
        Prep::Fail { phase: "panic", st: "??", reason: format!("translator panicked: {msg}") }
    })
}

fn normalize(e: &str) -> String {
    let first = e.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let mut s = String::new();
    let mut last_digit = false;
    for c in first.chars() {
        if c.is_ascii_digit() {
            if !last_digit {
                s.push('N');
            }
            last_digit = true;
        } else {
            last_digit = false;
            s.push(c);
        }
    }
    s.chars().take(140).collect()
}

struct Shared {
    phase: AtomicU8,
    done: AtomicUsize,
    total: AtomicUsize,
    failed: AtomicUsize,
    skip: AtomicBool,
    finished: AtomicBool,
    compile_start: Mutex<Option<Instant>>,
    name: String,
}

impl Shared {
    fn eta(&self) -> Option<f64> {
        let st = (*self.compile_start.lock().unwrap())?;
        let done = self.done.load(Ordering::Relaxed);
        let total = self.total.load(Ordering::Relaxed);
        let el = st.elapsed().as_secs_f64();
        if done == 0 || el < 1.5 {
            return None;
        }
        Some(el / done as f64 * total.saturating_sub(done) as f64)
    }
}

struct FailRow {
    phase: &'static str,
    st: &'static str,
    hash: u64,
    rt: [u8; 8],
    reason: String,
}

#[derive(Default)]
struct Report {
    known: usize,
    unique_keys: usize,
    cached: usize,
    converted: usize,
    runtime_only: usize,
    known_dxc_fail: usize,
    duplicate: usize,
    would_convert: usize,
    fails: Vec<FailRow>,
    key_s: f64,
    compile_s: f64,
    compile_ms_sum: f64,
    by_stage: BTreeMap<&'static str, [usize; 4]>,
}

fn neg_path(title: &str) -> PathBuf {
    shader_cache::title_dir(title).join("prelaunch_failed.tsv")
}

fn load_neg(title: &str) -> HashMap<u128, String> {
    let mut m = HashMap::new();
    if let Ok(s) = std::fs::read_to_string(neg_path(title)) {
        for l in s.lines() {
            if let Some((k, r)) = l.split_once('\t') {
                if let Ok(k) = u128::from_str_radix(k, 16) {
                    m.insert(k, r.to_string());
                }
            }
        }
    }
    m
}

fn append_neg(title: &str, rows: &[(u128, String)]) {
    if rows.is_empty() {
        return;
    }
    use std::io::Write;
    let p = neg_path(title);
    let _ = std::fs::create_dir_all(p.parent().unwrap());
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) {
        for (k, r) in rows {
            let _ = writeln!(f, "{k:032x}\t{}", r.lines().next().unwrap_or("").replace('\t', " "));
        }
    }
}

struct Opts {
    title: String,
    dirs: Vec<PathBuf>,
    jobs: usize,
    retry_failed: bool,
    check: bool,
}

fn run(o: &Opts, sh: &Shared, rep: &Mutex<Report>) {
    let t0 = Instant::now();
    let mut items = Vec::new();
    let mut seen = HashSet::new();
    let mut per_dir = Vec::new();
    for d in &o.dirs {
        let n = gather(d, &mut items, &mut seen);
        if n > 0 {
            per_dir.push(format!("{} {n}", d.display()));
        }
        if let Ok(rd) = std::fs::read_dir(d) {
            for e in rd.flatten() {
                if e.path().is_dir() && e.file_name().to_string_lossy().eq_ignore_ascii_case(&o.title) {
                    let n = gather(&e.path(), &mut items, &mut seen);
                    if n > 0 {
                        per_dir.push(format!("{} {n}", e.path().display()));
                    }
                }
            }
        }
    }
    items.sort_by_key(|it| (it.hash, it.rt));
    eprintln!("[shader prelaunch] {} known (shader, render-target format) variants: {}", items.len(), if per_dir.is_empty() { "none".to_string() } else { per_dir.join(", ") });
    let fma = fma_probe::probe().fma;
    let loop_grad = runtime_hlsl::loop_grad_from_env();
    sh.total.store(items.len(), Ordering::Relaxed);
    let next = AtomicUsize::new(0);
    let preps: Vec<Mutex<Option<Prep>>> = (0..items.len()).map(|_| Mutex::new(None)).collect();
    std::thread::scope(|s| {
        for _ in 0..o.jobs {
            s.spawn(|| loop {
                if sh.skip.load(Ordering::Relaxed) {
                    break;
                }
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(it) = items.get(i) else { break };
                *preps[i].lock().unwrap() = Some(prepare(it, loop_grad, fma));
                sh.done.fetch_add(1, Ordering::Relaxed);
            });
        }
    });
    let neg = if o.retry_failed { HashMap::new() } else { load_neg(&o.title) };
    let mut todo: Vec<(u128, String, String, &'static str, u64, [u8; 8])> = Vec::new();
    let mut keys: HashSet<u128> = HashSet::new();
    {
        let mut r = rep.lock().unwrap();
        r.known = items.len();
        r.key_s = t0.elapsed().as_secs_f64();
        for (i, p) in preps.into_iter().enumerate() {
            let it = &items[i];
            match p.into_inner().unwrap() {
                None => {}
                Some(Prep::RuntimeOnly { st, reason }) => {
                    r.runtime_only += 1;
                    r.by_stage.entry(st).or_default()[3] += 1;
                    let _ = reason;
                }
                Some(Prep::Fail { phase, st, reason }) => {
                    r.by_stage.entry(st).or_default()[2] += 1;
                    r.fails.push(FailRow { phase, st, hash: it.hash, rt: it.rt, reason });
                }
                Some(Prep::Key { st, k, src, profile }) => {
                    if !keys.insert(k) {
                        r.duplicate += 1;
                        continue;
                    }
                    if shader_cache::contains(k) {
                        r.cached += 1;
                        r.by_stage.entry(st).or_default()[0] += 1;
                    } else if let Some(why) = neg.get(&k) {
                        r.known_dxc_fail += 1;
                        r.by_stage.entry(st).or_default()[2] += 1;
                        r.fails.push(FailRow { phase: "dxc (known)", st, hash: it.hash, rt: it.rt, reason: why.clone() });
                    } else {
                        todo.push((k, src, profile, st, it.hash, it.rt));
                    }
                }
            }
        }
        r.unique_keys = keys.len();
    }
    if sh.skip.load(Ordering::Relaxed) {
        return;
    }
    if o.check {
        rep.lock().unwrap().would_convert = todo.len();
        return;
    }
    todo.sort_by(|a, b| b.1.len().cmp(&a.1.len()));
    sh.done.store(0, Ordering::Relaxed);
    sh.total.store(todo.len(), Ordering::Relaxed);
    *sh.compile_start.lock().unwrap() = Some(Instant::now());
    sh.phase.store(1, Ordering::Relaxed);
    let t1 = Instant::now();
    let next = AtomicUsize::new(0);
    let new_neg: Mutex<Vec<(u128, String)>> = Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for _ in 0..o.jobs {
            s.spawn(|| loop {
                if sh.skip.load(Ordering::Relaxed) {
                    break;
                }
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some((k, src, profile, st, hash, rt)) = todo.get(i) else { break };
                let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| shader_cache::compile_cached(src, profile, fma, &[]))).unwrap_or_else(|_| Err("dxc panicked".into()));
                if sh.skip.load(Ordering::Relaxed) && res.is_err() {
                    break;
                }
                let mut r = rep.lock().unwrap();
                match res {
                    Ok(c) => {
                        if c.source == shader_cache::Source::Pack {
                            r.cached += 1;
                            r.by_stage.entry(st).or_default()[0] += 1;
                        } else {
                            r.converted += 1;
                            r.compile_ms_sum += c.dxc_ms;
                            r.by_stage.entry(st).or_default()[1] += 1;
                        }
                    }
                    Err(e) => {
                        sh.failed.fetch_add(1, Ordering::Relaxed);
                        r.by_stage.entry(st).or_default()[2] += 1;
                        new_neg.lock().unwrap().push((*k, format!("dxc: {}", e.lines().next().unwrap_or(""))));
                        r.fails.push(FailRow { phase: "dxc", st, hash: *hash, rt: *rt, reason: format!("dxc: {e}") });
                    }
                }
                drop(r);
                sh.done.fetch_add(1, Ordering::Relaxed);
            });
        }
    });
    append_neg(&o.title, &new_neg.into_inner().unwrap());
    rep.lock().unwrap().compile_s = t1.elapsed().as_secs_f64();
}

fn summary(o: &Opts, r: &Report, total_s: f64, skipped: bool, todo_left: usize) -> String {
    let mut s = String::new();
    let failed = r.fails.len();
    s.push_str(&format!("[shader prelaunch] {} ({}){}\n", o.title, shader_cache::pack_path().display(), if skipped { " SKIPPED by user, remaining shaders left to runtime async PSO creation" } else { "" }));
    s.push_str(&format!("  known variants {}  unique compiles {}  already cached {}  converted {}  failed {}  runtime-only {}  same HLSL as another variant {}{}\n", r.known, r.unique_keys, r.cached, r.converted, failed, r.runtime_only, r.duplicate, if todo_left > 0 { format!("  not converted (skipped) {todo_left}") } else { String::new() }));
    s.push_str(&format!("  time total {:.1}s (key pass {:.1}s, compile {:.1}s, summed dxc {:.1}s) on {} jobs; {}\n", total_s, r.key_s, r.compile_s, r.compile_ms_sum / 1000.0, o.jobs, dxc::identity()));
    if o.check {
        s.push_str(&format!("  check only: {} shaders would be converted, nothing compiled\n", r.would_convert));
    }
    let st: Vec<String> = r.by_stage.iter().map(|(k, v)| format!("{k} cached {} converted {} failed {} runtime-only {}", v[0], v[1], v[2], v[3])).collect();
    if !st.is_empty() {
        s.push_str(&format!("  per stage: {}\n", st.join("; ")));
    }
    if r.runtime_only > 0 {
        s.push_str(&format!("  runtime-only: {} mesh/amplification shaders have no dumped PSO root signature; compiled when the title creates their PSO\n", r.runtime_only));
    }
    let mut groups: BTreeMap<(String, &'static str), Vec<&FailRow>> = BTreeMap::new();
    for f in &r.fails {
        groups.entry((normalize(&f.reason), f.phase)).or_default().push(f);
    }
    let mut g: Vec<_> = groups.into_iter().collect();
    g.sort_by(|a, b| b.1.len().cmp(&a.1.len()));
    if !g.is_empty() {
        s.push_str(&format!("  failed ({failed}, left to runtime, which retries them and logs its own result):\n"));
    }
    for ((reason, phase), v) in g.iter().take(20) {
        let ex: Vec<String> = v.iter().take(3).map(|f| format!("{}:{:016x}_{}", f.st, f.hash, rt_hex(&f.rt))).collect();
        s.push_str(&format!("    {:4} [{phase}] {reason}  e.g. {}\n", v.len(), ex.join(" ")));
    }
    s
}

#[allow(non_snake_case, clippy::upper_case_acronyms)]
mod ui {
    use super::Shared;
    use std::ffi::c_void;
    use std::sync::atomic::Ordering;

    type HWND = *mut c_void;
    type HDC = *mut c_void;
    type H = *mut c_void;

    #[repr(C)]
    struct WNDCLASSEXW {
        cbSize: u32,
        style: u32,
        lpfnWndProc: unsafe extern "system" fn(HWND, u32, usize, isize) -> isize,
        cbClsExtra: i32,
        cbWndExtra: i32,
        hInstance: H,
        hIcon: H,
        hCursor: H,
        hbrBackground: H,
        lpszMenuName: *const u16,
        lpszClassName: *const u16,
        hIconSm: H,
    }
    #[repr(C)]
    struct MSG {
        hwnd: HWND,
        message: u32,
        wParam: usize,
        lParam: isize,
        time: u32,
        pt: [i32; 2],
        lPrivate: u32,
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct RECT {
        l: i32,
        t: i32,
        r: i32,
        b: i32,
    }
    #[repr(C)]
    struct PAINTSTRUCT {
        hdc: HDC,
        fErase: i32,
        rc: RECT,
        fRestore: i32,
        fIncUpdate: i32,
        rgb: [u8; 32],
    }

    #[link(name = "user32")]
    extern "system" {
        fn RegisterClassExW(c: *const WNDCLASSEXW) -> u16;
        fn CreateWindowExW(ex: u32, cls: *const u16, name: *const u16, style: u32, x: i32, y: i32, w: i32, h: i32, parent: HWND, menu: H, inst: H, p: *mut c_void) -> HWND;
        fn DefWindowProcW(h: HWND, m: u32, w: usize, l: isize) -> isize;
        fn PeekMessageW(m: *mut MSG, h: HWND, a: u32, b: u32, r: u32) -> i32;
        fn TranslateMessage(m: *const MSG) -> i32;
        fn DispatchMessageW(m: *const MSG) -> isize;
        fn InvalidateRect(h: HWND, r: *const RECT, e: i32) -> i32;
        fn BeginPaint(h: HWND, p: *mut PAINTSTRUCT) -> HDC;
        fn EndPaint(h: HWND, p: *const PAINTSTRUCT) -> i32;
        fn FillRect(dc: HDC, r: *const RECT, b: H) -> i32;
        fn DrawTextW(dc: HDC, s: *const u16, n: i32, r: *mut RECT, f: u32) -> i32;
        fn GetClientRect(h: HWND, r: *mut RECT) -> i32;
        fn LoadCursorW(i: H, n: *const u16) -> H;
        fn ShowWindow(h: HWND, c: i32) -> i32;
        fn UpdateWindow(h: HWND) -> i32;
        fn DestroyWindow(h: HWND) -> i32;
        fn GetSystemMetrics(i: i32) -> i32;
        fn SendMessageW(h: HWND, m: u32, w: usize, l: isize) -> isize;
        fn GetDpiForSystem() -> u32;
        fn SetProcessDPIAware() -> i32;
    }
    #[link(name = "gdi32")]
    extern "system" {
        fn CreateSolidBrush(c: u32) -> H;
        fn DeleteObject(o: H) -> i32;
        fn SetBkMode(dc: HDC, m: i32) -> i32;
        fn SetTextColor(dc: HDC, c: u32) -> u32;
        fn SelectObject(dc: HDC, o: H) -> H;
        fn CreateFontW(h: i32, w: i32, e: i32, o: i32, weight: i32, i: u32, u: u32, s: u32, cs: u32, op: u32, cp: u32, q: u32, pf: u32, face: *const u16) -> H;
        fn CreateCompatibleDC(dc: HDC) -> HDC;
        fn CreateCompatibleBitmap(dc: HDC, w: i32, h: i32) -> H;
        fn BitBlt(d: HDC, x: i32, y: i32, w: i32, h: i32, s: HDC, sx: i32, sy: i32, rop: u32) -> i32;
        fn DeleteDC(dc: HDC) -> i32;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetModuleHandleW(n: *const u16) -> H;
        fn Sleep(ms: u32);
    }

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    static SH: std::sync::OnceLock<&'static Shared> = std::sync::OnceLock::new();
    static FONTS: std::sync::OnceLock<(usize, usize, usize, i32)> = std::sync::OnceLock::new();

    fn fmt_eta(s: f64) -> String {
        let s = s.ceil() as u64;
        if s >= 60 {
            format!("about {} min {:02} s left", s / 60, s % 60)
        } else {
            format!("about {s} s left")
        }
    }

    unsafe fn text(dc: HDC, font: usize, color: u32, s: &str, mut r: RECT, flags: u32) {
        SelectObject(dc, font as H);
        SetTextColor(dc, color);
        let v: Vec<u16> = s.encode_utf16().collect();
        DrawTextW(dc, v.as_ptr(), v.len() as i32, &mut r, flags);
    }

    unsafe fn paint(h: HWND) {
        let Some(sh) = SH.get() else { return };
        let (big, small, _btn, sc) = *FONTS.get().unwrap();
        let px = |v: i32| v * sc / 96;
        let mut ps: PAINTSTRUCT = std::mem::zeroed();
        let wdc = BeginPaint(h, &mut ps);
        let mut rc = RECT { l: 0, t: 0, r: 0, b: 0 };
        GetClientRect(h, &mut rc);
        let dc = CreateCompatibleDC(wdc);
        let bmp = CreateCompatibleBitmap(wdc, rc.r, rc.b);
        let old = SelectObject(dc, bmp);
        let bg = CreateSolidBrush(0x00201c1c);
        FillRect(dc, &rc, bg);
        SetBkMode(dc, 1);
        let m = px(18);
        text(dc, big, 0x00ffffff, &sh.name, RECT { l: m, t: px(14), r: rc.r - m, b: px(42) }, 0x20 | 0x8000);
        let phase = sh.phase.load(Ordering::Relaxed);
        let done = sh.done.load(Ordering::Relaxed);
        let total = sh.total.load(Ordering::Relaxed).max(1);
        let line = if phase == 0 { format!("Checking shaders {done} / {total}") } else { format!("Converting shaders {done} / {total}") };
        text(dc, small, 0x00e0e0e0, &line, RECT { l: m, t: px(46), r: rc.r - m, b: px(66) }, 0x20);
        let bar = RECT { l: m, t: px(72), r: rc.r - m, b: px(86) };
        let track = CreateSolidBrush(0x00403a3a);
        FillRect(dc, &bar, track);
        DeleteObject(track);
        let frac = (done as f64 / total as f64).clamp(0.0, 1.0);
        let fill = RECT { r: bar.l + ((bar.r - bar.l) as f64 * frac) as i32, ..bar };
        let fb = CreateSolidBrush(0x0050c010);
        FillRect(dc, &fill, fb);
        DeleteObject(fb);
        let failed = sh.failed.load(Ordering::Relaxed);
        let mut info = if phase == 0 { "Finding shaders that are not converted yet".to_string() } else { sh.eta().map(fmt_eta).unwrap_or_else(|| "estimating time left".into()) };
        if failed > 0 {
            info.push_str(&format!("   {failed} failed (left to the game)"));
        }
        text(dc, small, 0x00a8a8a8, &info, RECT { l: m, t: px(92), r: rc.r - m, b: px(112) }, 0x20 | 0x8000);
        text(dc, small, 0x00808080, "The game starts when this finishes. Skip starts it now.", RECT { l: m, t: px(120), r: rc.r - px(150), b: px(150) }, 0x10);
        BitBlt(wdc, 0, 0, rc.r, rc.b, dc, 0, 0, 0x00CC0020);
        SelectObject(dc, old);
        DeleteObject(bmp);
        DeleteDC(dc);
        DeleteObject(bg);
        EndPaint(h, &ps);
    }

    unsafe extern "system" fn wndproc(h: HWND, m: u32, wp: usize, lp: isize) -> isize {
        match m {
            0x0014 => 1,
            0x000F => {
                paint(h);
                0
            }
            0x0111 if wp & 0xffff == 1 => {
                if let Some(sh) = SH.get() {
                    sh.skip.store(true, Ordering::Relaxed);
                }
                0
            }
            0x0010 => {
                if let Some(sh) = SH.get() {
                    sh.skip.store(true, Ordering::Relaxed);
                }
                0
            }
            _ => DefWindowProcW(h, m, wp, lp),
        }
    }

    pub fn show(sh: &'static Shared, skip_after: Option<std::time::Duration>) {
        unsafe {
            let _ = SH.set(sh);
            SetProcessDPIAware();
            let sc = GetDpiForSystem().max(96) as i32;
            let px = |v: i32| v * sc / 96;
            let face = w("Segoe UI");
            let big = CreateFontW(-px(19), 0, 0, 0, 600, 0, 0, 0, 1, 0, 0, 5, 0, face.as_ptr()) as usize;
            let small = CreateFontW(-px(14), 0, 0, 0, 400, 0, 0, 0, 1, 0, 0, 5, 0, face.as_ptr()) as usize;
            let _ = FONTS.set((big, small, small, sc));
            let inst = GetModuleHandleW(std::ptr::null());
            let cls = w("TomBoxShaderPrelaunch");
            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                style: 3,
                lpfnWndProc: wndproc,
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: inst,
                hIcon: std::ptr::null_mut(),
                hCursor: LoadCursorW(std::ptr::null_mut(), 32512 as *const u16),
                hbrBackground: std::ptr::null_mut(),
                lpszMenuName: std::ptr::null(),
                lpszClassName: cls.as_ptr(),
                hIconSm: std::ptr::null_mut(),
            };
            RegisterClassExW(&wc);
            let (ww, wh) = (px(500), px(200));
            let x = (GetSystemMetrics(0) - ww) / 2;
            let y = (GetSystemMetrics(1) - wh) / 2;
            let title = w(&format!("TomBox - preparing {}", sh.name));
            let h = CreateWindowExW(0x0004_0000, cls.as_ptr(), title.as_ptr(), 0x00C0_0000 | 0x0008_0000 | 0x0002_0000, x, y, ww, wh, std::ptr::null_mut(), std::ptr::null_mut(), inst, std::ptr::null_mut());
            if h.is_null() {
                return;
            }
            let mut rc = RECT { l: 0, t: 0, r: 0, b: 0 };
            GetClientRect(h, &mut rc);
            let bcls = w("BUTTON");
            let btxt = w("Skip");
            let b = CreateWindowExW(0, bcls.as_ptr(), btxt.as_ptr(), 0x5000_0000, rc.r - px(18) - px(110), rc.b - px(18) - px(32), px(110), px(32), h, 1usize as H, inst, std::ptr::null_mut());
            SendMessageW(b, 0x0030, small, 1);
            ShowWindow(h, 4);
            ShowWindow(h, 4);
            UpdateWindow(h);
            let t0 = std::time::Instant::now();
            let mut msg: MSG = std::mem::zeroed();
            loop {
                while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, 1) != 0 {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                if sh.finished.load(Ordering::Relaxed) || sh.skip.load(Ordering::Relaxed) {
                    break;
                }
                if skip_after.is_some_and(|d| t0.elapsed() >= d) {
                    sh.skip.store(true, Ordering::Relaxed);
                    break;
                }
                InvalidateRect(h, std::ptr::null(), 0);
                Sleep(60);
            }
            DestroyWindow(h);
            while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, 1) != 0 {
                DispatchMessageW(&msg);
            }
        }
    }
}

fn usage() {
    eprintln!("usage: xshader_prelaunch --title <exe stem> [--name <display name>] [--dir <dump dir>]...");
    eprintln!("                         [--no-ui] [--yes] [--jobs n] [--retry-failed] [--summary <file>] [--skip-after <ms>] [--check]");
    eprintln!("translates and DXC-compiles every known shader of the title that is not in the pack yet, then exits;");
    eprintln!("exit 0 done (failures are reported and left to runtime), 3 skipped, 2 setup problem (launch anyway).");
    eprintln!("pack root: {}", shader_cache::root().display());
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut title = String::new();
    let mut name = String::new();
    let mut dirs: Vec<PathBuf> = Vec::new();
    let mut no_ui = false;
    let mut summary_file: Option<PathBuf> = None;
    let mut skip_after: Option<Duration> = None;
    let mut retry_failed = false;
    let mut check = false;
    let mut jobs = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).saturating_sub(2).max(1);
    let mut i = 0;
    let val = |i: usize| args.get(i + 1).cloned().unwrap_or_default();
    while i < args.len() {
        let mut step = 2;
        match args[i].as_str() {
            "--title" => title = val(i),
            "--name" => name = val(i),
            "--dir" => dirs.push(PathBuf::from(val(i))),
            "--jobs" => jobs = val(i).parse().unwrap_or(jobs).max(1),
            "--summary" => summary_file = Some(PathBuf::from(val(i))),
            "--skip-after" => skip_after = val(i).parse().ok().map(Duration::from_millis),
            "--no-ui" => {
                no_ui = true;
                step = 1;
            }
            "--yes" => step = 1,
            "--check" => {
                check = true;
                no_ui = true;
                step = 1;
            }
            "--retry-failed" => {
                retry_failed = true;
                step = 1;
            }
            "-h" | "--help" => {
                usage();
                return;
            }
            a => {
                eprintln!("unknown argument {a}");
                usage();
                std::process::exit(2);
            }
        }
        i += step;
    }
    if title.is_empty() {
        usage();
        std::process::exit(2);
    }
    if name.is_empty() {
        name = title.clone();
    }
    let started = Instant::now();
    if !dxc::available() {
        println!("[shader prelaunch] {title}: dxcompiler.dll + dxil.dll not found (TOMBOX_DXC_DIR); nothing converted, every shader is left to runtime");
        std::process::exit(2);
    }
    let mut all_dirs = vec![shader_cache::src_dir(&title), shader_cache::title_dir(&title).join("dump"), shader_cache::root().join("dump"), std::env::temp_dir().join("tombox_shader_dump")];
    all_dirs.extend(dirs);
    let o: &'static Opts = Box::leak(Box::new(Opts { title: title.clone(), dirs: all_dirs, jobs, retry_failed, check }));
    let sh: &'static Shared = Box::leak(Box::new(Shared {
        phase: AtomicU8::new(0),
        done: AtomicUsize::new(0),
        total: AtomicUsize::new(0),
        failed: AtomicUsize::new(0),
        skip: AtomicBool::new(false),
        finished: AtomicBool::new(false),
        compile_start: Mutex::new(None),
        name,
    }));
    static REP: OnceLock<Mutex<Report>> = OnceLock::new();
    let rep = REP.get_or_init(|| Mutex::new(Report::default()));
    let worker = std::thread::spawn(move || {
        run(o, sh, rep);
        sh.finished.store(true, Ordering::Relaxed);
    });
    let ui_wait = Instant::now();
    loop {
        if sh.finished.load(Ordering::Relaxed) {
            break;
        }
        let compiling = sh.phase.load(Ordering::Relaxed) == 1 && sh.total.load(Ordering::Relaxed) > 0;
        if !no_ui && compiling {
            ui::show(sh, skip_after);
            break;
        }
        if no_ui && skip_after.is_some_and(|d| ui_wait.elapsed() >= d) {
            sh.skip.store(true, Ordering::Relaxed);
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let skipped = sh.skip.load(Ordering::Relaxed) && !sh.finished.load(Ordering::Relaxed);
    if skipped {
        let t = Instant::now();
        while !sh.finished.load(Ordering::Relaxed) && t.elapsed() < Duration::from_millis(150) {
            std::thread::sleep(Duration::from_millis(10));
        }
    } else {
        let _ = worker.join();
    }
    let r = rep.lock().unwrap_or_else(|e| e.into_inner());
    let left = if skipped { sh.total.load(Ordering::Relaxed).saturating_sub(sh.done.load(Ordering::Relaxed)) } else { 0 };
    let s = summary(o, &r, started.elapsed().as_secs_f64(), skipped, left);
    print!("{s}");
    if let Some(f) = &summary_file {
        let _ = std::fs::write(f, &s);
    }
    {
        use std::io::Write;
        let p = shader_cache::title_dir(&title).join("prelaunch.log");
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
            let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            let _ = writeln!(f, "unix {ts}: known {} cached {} converted {} failed {} runtime-only {} skipped {} total {:.1}s key {:.1}s compile {:.1}s jobs {}", r.known, r.cached, r.converted, r.fails.len(), r.runtime_only, skipped, started.elapsed().as_secs_f64(), r.key_s, r.compile_s, o.jobs);
        }
    }
    std::process::exit(if skipped { 3 } else { 0 });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dump_names_parse() {
        assert_eq!(parse_name("180e3a845e230ed1_8000000000000000.xbox"), Some((0x180e3a845e230ed1, [0x80, 0, 0, 0, 0, 0, 0, 0])));
        assert_eq!(parse_name("180e3a845e230ed1_8000000000000000.xbox.tmp1"), None);
        assert_eq!(parse_name("short_8000000000000000.xbox"), None);
        assert_eq!(rt_hex(&[0x80, 1, 0, 0, 0, 0, 0, 0xff]), "80010000000000ff");
    }

    #[test]
    fn reasons_group_by_shape() {
        assert_eq!(normalize("dxc: error at line 12: x\nmore"), normalize("dxc: error at line 345: x"));
    }

    #[test]
    fn gather_pairs_roots_and_dedupes() {
        let d = std::env::temp_dir().join(format!("tbx_prelaunch_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        std::fs::write(d.join("00000000000000aa_0000000000000000.xbox"), b"x").unwrap();
        std::fs::write(d.join("00000000000000aa_0000000000000000.1234.root"), b"r").unwrap();
        std::fs::write(d.join("00000000000000bb_8000000000000000.xbox"), b"y").unwrap();
        std::fs::write(d.join("notes.txt"), b"z").unwrap();
        let mut v = Vec::new();
        let mut seen = HashSet::new();
        assert_eq!(gather(&d, &mut v, &mut seen), 2);
        assert_eq!(gather(&d, &mut v, &mut seen), 0);
        assert!(v.iter().any(|i| i.hash == 0xaa && i.root.is_some()));
        assert!(v.iter().any(|i| i.hash == 0xbb && i.root.is_none()));
        let _ = std::fs::remove_dir_all(&d);
    }
}
