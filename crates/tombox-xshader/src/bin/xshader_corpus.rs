use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use tombox_xshader::{dxc, extract, hlsl, parse_root_signature, runtime_hlsl, shader_cache, Stage};

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
    let stem = name.strip_suffix(".xbox").or_else(|| name.strip_suffix(".dxbc"))?;
    let stem = stem.strip_prefix("inflight_").or_else(|| stem.strip_prefix("slow_")).unwrap_or(stem);
    let (h, f) = match stem.split_once('_') {
        Some((h, f)) => (h, f),
        None => (stem, "0000000000000000"),
    };
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

fn roots_in(dir: &Path) -> HashMap<String, Vec<PathBuf>> {
    let mut m: HashMap<String, Vec<PathBuf>> = HashMap::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if let Some(s) = n.strip_suffix(".root") {
                let base = s.split('.').next().unwrap_or(s).to_string();
                m.entry(base).or_default().push(e.path());
            }
        }
    }
    m
}

fn gather(dir: &Path, out: &mut Vec<Item>, seen: &mut HashSet<(u64, [u8; 8], String)>, depth: u32) -> usize {
    let Ok(rd) = std::fs::read_dir(dir) else {
        eprintln!("cannot read {}", dir.display());
        return 0;
    };
    let roots = roots_in(dir);
    let mut n = 0;
    let mut subdirs = Vec::new();
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            subdirs.push(p);
            continue;
        }
        let name = e.file_name().to_string_lossy().into_owned();
        let Some((hash, rt)) = parse_name(&name) else { continue };
        let stem = name.rsplit_once('.').map(|(s, _)| s.to_string()).unwrap_or_default();
        let rlist: Vec<Option<PathBuf>> = match roots.get(&stem) {
            Some(v) if !v.is_empty() => v.iter().cloned().map(Some).collect(),
            _ => vec![None],
        };
        for r in rlist {
            let rk = r.as_ref().map(|p| p.file_name().unwrap_or_default().to_string_lossy().into_owned()).unwrap_or_default();
            if !seen.insert((hash, rt, rk)) {
                continue;
            }
            out.push(Item { path: p.clone(), hash, rt, root: r });
            n += 1;
        }
    }
    if n == 0 && depth < 2 {
        subdirs.sort();
        for s in subdirs {
            n += gather(&s, out, seen, depth + 1);
        }
    }
    n
}

fn normalize(e: &str) -> String {
    let first = e.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let c: Vec<char> = first.chars().collect();
    let mut s = String::with_capacity(first.len());
    let mut i = 0;
    while i < c.len() {
        let boundary = i == 0 || !c[i - 1].is_ascii_alphanumeric();
        if boundary && c[i] == '0' && i + 2 < c.len() && (c[i + 1] == 'x' || c[i + 1] == 'X') && c[i + 2].is_ascii_hexdigit() {
            let mut j = i + 2;
            while j < c.len() && c[j].is_ascii_hexdigit() {
                j += 1;
            }
            s.push_str("0x#");
            i = j;
            continue;
        }
        if boundary && c[i].is_ascii_hexdigit() {
            let mut j = i;
            while j < c.len() && c[j].is_ascii_hexdigit() {
                j += 1;
            }
            if j - i >= 8 && (j == c.len() || !c[j].is_ascii_alphanumeric()) {
                s.push('#');
                i = j;
                continue;
            }
        }
        if c[i].is_ascii_digit() {
            let mut j = i;
            while j < c.len() && c[j].is_ascii_digit() {
                j += 1;
            }
            s.push('N');
            i = j;
            continue;
        }
        s.push(c[i]);
        i += 1;
    }
    s.chars().take(160).collect()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    TranslateOnly,
    ReadOnly,
    Fill,
}

#[derive(Default)]
struct StageRow {
    total: usize,
    ok: usize,
    pack: usize,
    compiled: usize,
    fail_extract: usize,
    fail_translate: usize,
    fail_dxc: usize,
    panicked: usize,
}

struct Fail {
    phase: &'static str,
    hash: u64,
    rt: [u8; 8],
    stage: String,
    reason: String,
}

fn usage() {
    eprintln!("usage: xshader_corpus [<dir>...] [--title <name>]... [--all-titles] [--failures]");
    eprintln!("                      [--hash h1,h2] [--stage ps,cs] [--no-dxc] [--fill-pack] [--recompile]");
    eprintln!("                      [--out <dir>] [--jobs n] [--limit n] [--examples n] [--list] [--time n]");
    eprintln!("dirs hold <hash>_<rtfmt>.xbox (TOMBOX_SHADER_DUMP / title src caches) or <hash>.dxbc (failure dumps);");
    eprintln!("a dir with no shaders is searched one level down (per-title subfolders).");
    eprintln!("default: translate + finalize + DXC, read-only (pack hits count as ok, the pack is never written).");
    eprintln!("pack root: {}", shader_cache::root().display());
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut dirs: Vec<PathBuf> = Vec::new();
    let mut hashes: Vec<String> = Vec::new();
    let mut stages: Vec<String> = Vec::new();
    let mut mode = Mode::ReadOnly;
    let mut recompile = false;
    let mut out: Option<PathBuf> = None;
    let mut jobs = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).saturating_sub(2).max(1);
    let mut limit = usize::MAX;
    let mut examples = 3usize;
    let mut list = false;
    let mut time_top: Option<usize> = None;
    let mut i = 0;
    let val = |i: usize| args.get(i + 1).cloned().unwrap_or_default();
    while i < args.len() {
        let a = args[i].as_str();
        let mut step = 2;
        match a {
            "--title" => dirs.push(shader_cache::src_dir(&val(i))),
            "--all-titles" => {
                let t = shader_cache::root().join("titles");
                if let Ok(rd) = std::fs::read_dir(&t) {
                    let mut v: Vec<PathBuf> = rd.flatten().map(|e| e.path().join("src")).filter(|p| p.is_dir()).collect();
                    v.sort();
                    dirs.extend(v);
                }
                step = 1;
            }
            "--failures" => {
                dirs.push(std::env::temp_dir().join("tombox_shaders"));
                step = 1;
            }
            "--hash" => hashes.extend(val(i).split(',').map(|s| s.trim().trim_start_matches("0x").to_ascii_lowercase()).filter(|s| !s.is_empty())),
            "--stage" => stages.extend(val(i).split(',').map(|s| s.trim().to_ascii_lowercase()).filter(|s| !s.is_empty())),
            "--no-dxc" => {
                mode = Mode::TranslateOnly;
                step = 1;
            }
            "--fill-pack" => {
                mode = Mode::Fill;
                step = 1;
            }
            "--recompile" => {
                recompile = true;
                step = 1;
            }
            "--out" => out = Some(PathBuf::from(val(i))),
            "--jobs" => jobs = val(i).parse().unwrap_or(jobs).max(1),
            "--limit" => limit = val(i).parse().unwrap_or(limit),
            "--examples" => examples = val(i).parse().unwrap_or(examples),
            "--time" => time_top = Some(val(i).parse().unwrap_or(20)),
            "--list" => {
                list = true;
                step = 1;
            }
            "-h" | "--help" => {
                usage();
                return ExitCode::SUCCESS;
            }
            _ if a.starts_with("--") => {
                eprintln!("unknown option {a}");
                usage();
                return ExitCode::from(2);
            }
            _ => {
                dirs.push(PathBuf::from(a));
                step = 1;
            }
        }
        i += step;
    }
    if dirs.is_empty() {
        usage();
        return ExitCode::from(2);
    }
    if mode != Mode::TranslateOnly && !dxc::available() {
        eprintln!("dxcompiler.dll + dxil.dll not found; use --no-dxc for translate-only");
        return ExitCode::from(2);
    }
    let t_gather = std::time::Instant::now();
    let mut items = Vec::new();
    let mut seen = HashSet::new();
    for d in &dirs {
        let n = gather(d, &mut items, &mut seen, 0);
        eprintln!("{}: {n} shaders", d.display());
    }
    if !hashes.is_empty() {
        items.retain(|it| {
            let h = format!("{:016x}", it.hash);
            hashes.iter().any(|w| h.starts_with(w.as_str()))
        });
    }
    items.sort_by_key(|it| (it.hash, it.rt));
    items.truncate(limit);
    let fma = (mode != Mode::TranslateOnly).then(|| {
        let p = tombox_xshader::fma_probe::probe();
        eprintln!("fma probe on {}: preserve {} any {} [{}]{}", p.adapter, p.fma.preserve, p.fma.any, p.detail, if p.cached { " (cached)" } else { "" });
        p.fma
    });
    let loop_grad = runtime_hlsl::loop_grad_from_env();
    eprintln!("{} shaders gathered in {:.1}s; {} jobs; mode {}", items.len(), t_gather.elapsed().as_secs_f64(), jobs, match mode {
        Mode::TranslateOnly => "translate only (no DXC)",
        Mode::ReadOnly => if recompile { "translate + DXC (pack ignored, not written)" } else { "translate + DXC (pack read-only)" },
        Mode::Fill => "translate + DXC, misses written to the pack",
    });
    if let Some(o) = &out {
        let _ = std::fs::create_dir_all(o);
    }
    let rows: Mutex<BTreeMap<String, StageRow>> = Mutex::new(BTreeMap::new());
    let fails: Mutex<Vec<Fail>> = Mutex::new(Vec::new());
    let times: Mutex<Vec<(u64, [u8; 8], String, f64, usize)>> = Mutex::new(Vec::new());
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let started = std::time::Instant::now();
    std::thread::scope(|s| {
        for _ in 0..jobs {
            s.spawn(|| loop {
                let idx = next.fetch_add(1, Ordering::Relaxed);
                let Some(it) = items.get(idx) else { break };
                let Ok(blob) = std::fs::read(&it.path) else { continue };
                let root = it.root.as_ref().and_then(|p| std::fs::read(p).ok());
                let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<(String, u8, Option<String>), (&'static str, String, String, Option<String>)> {
                    let mut ex = extract(&blob).map_err(|e| ("extract", "??".to_string(), format!("extract: {e}"), None))?;
                    let st = stage_name(&ex.stage).to_string();
                    if !stages.is_empty() && !stages.contains(&st) {
                        return Ok((st, 3, None));
                    }
                    if let Some(root) = root.as_ref().filter(|_| !ex.has_root_sig) {
                        ex.root_sig = Some(parse_root_signature(root).map_err(|e| ("extract", st.clone(), format!("pipeline root signature: {e}"), None))?);
                    }
                    let mut t = hlsl::translate(&ex).map_err(|e| ("translate", st.clone(), format!("translate: {e}"), None))?;
                    if tombox_xshader::gcn::is_durango(&ex.fssm) {
                        t.hlsl = tombox_xshader::cbv_raw_hlsl::rewrite_hlsl(&t.hlsl).map_err(|e| ("translate", st.clone(), format!("bind: {e}"), None))?;
                    }
                    let src = runtime_hlsl::finalize(&t.hlsl, &it.rt, loop_grad);
                    if mode == Mode::TranslateOnly {
                        return Ok((st, 0, Some(src)));
                    }
                    let Some(fma) = fma else { return Ok((st, 0, Some(src))) };
                    let profile = dxc::profile_for(t.profile);
                    let k = shader_cache::key(&src, &profile, fma, "");
                    if !recompile && shader_cache::contains(k) {
                        return Ok((st, 1, Some(src)));
                    }
                    let t_dxc = std::time::Instant::now();
                    let res = if mode == Mode::Fill && !recompile {
                        shader_cache::compile_cached(&src, &profile, fma, &[]).map(|_| ())
                    } else {
                        dxc::compile_fma(&src, &profile, fma, &[]).map(|_| ())
                    };
                    let ms = t_dxc.elapsed().as_secs_f64() * 1000.0;
                    res.map_err(|e| ("dxc", st.clone(), e, Some(src.clone())))?;
                    if time_top.is_some() {
                        times.lock().unwrap().push((it.hash, it.rt, st.clone(), ms, src.lines().count()));
                    }
                    Ok((st, 2, Some(src)))
                }));
                match r {
                    Ok(Ok((st, kind, src))) => {
                        if let (Some(o), Some(h), Mode::TranslateOnly) = (&out, &src, mode) {
                            let _ = std::fs::write(o.join(format!("{:016x}_{}.hlsl", it.hash, rt_hex(&it.rt))), h);
                        }
                        if kind != 3 {
                            let mut g = rows.lock().unwrap();
                            let row = g.entry(st).or_default();
                            row.total += 1;
                            row.ok += 1;
                            match kind {
                                1 => row.pack += 1,
                                2 => row.compiled += 1,
                                _ => {}
                            }
                        }
                    }
                    Ok(Err((phase, st, reason, src))) => {
                        if stages.is_empty() || stages.contains(&st) || st == "??" {
                            let mut g = rows.lock().unwrap();
                            let row = g.entry(st.clone()).or_default();
                            row.total += 1;
                            match phase {
                                "extract" => row.fail_extract += 1,
                                "translate" => row.fail_translate += 1,
                                _ => row.fail_dxc += 1,
                            }
                            drop(g);
                            if let Some(o) = &out {
                                let stem = format!("{:016x}_{}", it.hash, rt_hex(&it.rt));
                                let _ = std::fs::write(o.join(format!("{stem}.err.txt")), &reason);
                                if let Some(h) = &src {
                                    let _ = std::fs::write(o.join(format!("{stem}.hlsl")), h);
                                }
                            }
                            fails.lock().unwrap().push(Fail { phase, hash: it.hash, rt: it.rt, stage: st, reason });
                        }
                    }
                    Err(p) => {
                        let msg = p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_else(|| "panic".into());
                        rows.lock().unwrap().entry("??".into()).or_default().panicked += 1;
                        fails.lock().unwrap().push(Fail { phase: "panic", hash: it.hash, rt: it.rt, stage: "??".into(), reason: format!("translator panicked: {msg}") });
                    }
                }
                let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                if n % 500 == 0 {
                    eprintln!("{n}/{} in {:.0}s", items.len(), started.elapsed().as_secs_f64());
                }
            });
        }
    });
    let rows = rows.into_inner().unwrap();
    let fails = fails.into_inner().unwrap();
    println!("stage  total      ok  pack-hit  compiled  fail:extract  translate  dxc  panicked");
    let mut tot = StageRow::default();
    for (st, r) in &rows {
        println!("{st:5} {:6} {:7} {:9} {:9} {:13} {:10} {:4} {:9}", r.total, r.ok, r.pack, r.compiled, r.fail_extract, r.fail_translate, r.fail_dxc, r.panicked);
        tot.total += r.total;
        tot.ok += r.ok;
        tot.pack += r.pack;
        tot.compiled += r.compiled;
        tot.fail_extract += r.fail_extract;
        tot.fail_translate += r.fail_translate;
        tot.fail_dxc += r.fail_dxc;
        tot.panicked += r.panicked;
    }
    println!("all   {:6} {:7} {:9} {:9} {:13} {:10} {:4} {:9}", tot.total, tot.ok, tot.pack, tot.compiled, tot.fail_extract, tot.fail_translate, tot.fail_dxc, tot.panicked);
    println!("wall {:.1}s", started.elapsed().as_secs_f64());
    if let Some(top) = time_top {
        let mut t = times.into_inner().unwrap();
        t.sort_by(|a, b| b.3.total_cmp(&a.3));
        let mut per: BTreeMap<&str, (usize, f64)> = BTreeMap::new();
        for r in &t {
            let e = per.entry(r.2.as_str()).or_default();
            e.0 += 1;
            e.1 += r.3;
        }
        println!();
        println!("dxc time: {:.1}s over {} compiled shaders", t.iter().map(|r| r.3).sum::<f64>() / 1000.0, t.len());
        for (st, (n, ms)) in &per {
            println!("  {st:5} {n:6} shaders {:9.1}s  mean {:7.1} ms", ms / 1000.0, ms / *n as f64);
        }
        for r in t.iter().take(top) {
            println!("  {:016x}_{} {:5} {:8.1} ms  {} lines", r.0, rt_hex(&r.1), r.2, r.3, r.4);
        }
        if let Some(o) = &out {
            let body: String = t.iter().map(|r| format!("{:016x}_{}	{}	{:.1}	{}
", r.0, rt_hex(&r.1), r.2, r.3, r.4)).collect();
            let _ = std::fs::write(o.join("dxc_times.tsv"), body);
        }
    }
    let mut groups: BTreeMap<(String, &'static str), Vec<&Fail>> = BTreeMap::new();
    for f in &fails {
        groups.entry((normalize(&f.reason), f.phase)).or_default().push(f);
    }
    let mut g: Vec<_> = groups.into_iter().collect();
    g.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.0.cmp(&b.0)));
    if !g.is_empty() {
        println!();
        println!("failures by first reason ({} shaders, {} distinct reasons):", fails.len(), g.len());
    }
    for ((reason, phase), v) in &g {
        let mut st: BTreeMap<&str, usize> = BTreeMap::new();
        for f in v {
            *st.entry(f.stage.as_str()).or_default() += 1;
        }
        let sts: Vec<String> = st.iter().map(|(k, n)| format!("{k}={n}")).collect();
        println!("{:5}  [{phase}] {reason}  ({})", v.len(), sts.join(" "));
        let take = if list { v.len() } else { examples.min(v.len()) };
        for f in v.iter().take(take) {
            println!("         {:016x}_{} {}", f.hash, rt_hex(&f.rt), f.reason.lines().next().unwrap_or("").chars().take(200).collect::<String>());
        }
    }
    if fails.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}
