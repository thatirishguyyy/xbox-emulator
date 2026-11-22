use std::fmt::Write as _;
use std::path::{Path, PathBuf};

const ROUTED: &[&str] = &["implemented", "forwarded", "hooked"];

fn lib_name(toml: &str) -> Option<(String, bool)> {
    let mut in_lib = false;
    let mut name = None;
    let mut cdylib = false;
    for l in toml.lines() {
        let t = l.trim();
        if t.starts_with('[') {
            in_lib = t == "[lib]";
            continue;
        }
        if in_lib {
            if let Some(v) = t.strip_prefix("name").map(str::trim).and_then(|r| r.strip_prefix('=')) {
                name = Some(v.trim().trim_matches('"').to_string());
            }
            if t.starts_with("crate-type") && t.contains("cdylib") {
                cdylib = true;
            }
        }
    }
    name.map(|n| (n, cdylib))
}

struct Row {
    ordinal: u16,
    name: String,
    status: String,
    forward: String,
}

fn rows(text: &str) -> Vec<Row> {
    let mut v = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let (l, note) = line.split_once(" | ").map_or((line, ""), |(a, b)| (a, b.trim()));
        let f: Vec<&str> = l.split_whitespace().collect();
        if !(f.len() == 3 || (f.len() == 4 && f[3] == "data")) {
            continue;
        }
        let Ok(ordinal) = f[0].parse::<u16>() else { continue };
        let forward = note
            .strip_prefix("PE forwarder to ")
            .and_then(|r| r.split(';').next())
            .and_then(|t| t.trim().split_once('.'))
            .map(|(d, _)| format!("{}.dll", d.to_ascii_lowercase()))
            .unwrap_or_default();
        v.push(Row { ordinal, name: f[1].to_string(), status: f[2].to_string(), forward });
    }
    v
}

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn main() {
    let here = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let backend_data = here.join("backend_data");
    println!("cargo:rustc-check-cfg=cfg(tombox_backend_data)");
    let watch = if backend_data.is_dir() {
        println!("cargo:rustc-cfg=tombox_backend_data");
        &backend_data
    } else {
        &here
    };
    println!("cargo:rerun-if-changed={}", watch.display());
    let analysis_gdk = here.join("../../../analysis/gdk");
    println!("cargo:rustc-check-cfg=cfg(tombox_analysis_gdk)");
    println!("cargo:rerun-if-changed={}", analysis_gdk.display());
    if analysis_gdk.is_dir() {
        println!("cargo:rustc-cfg=tombox_analysis_gdk");
    }
    let compat = here.parent().unwrap().to_path_buf();
    let hosts = here.join("data").join("console_apiset_hosts.txt");
    println!("cargo:rerun-if-changed={}", hosts.display());
    println!("cargo:rerun-if-changed={}", here.join("../../../Cargo.toml").display());
    let mut libs: Vec<(PathBuf, String, bool)> = Vec::new();
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&compat).map(|r| r.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.is_dir()).collect()).unwrap_or_default();
    dirs.sort();
    for d in &dirs {
        let toml = d.join("Cargo.toml");
        if !toml.is_file() {
            continue;
        }
        println!("cargo:rerun-if-changed={}", toml.display());
        if let Some((n, c)) = std::fs::read_to_string(&toml).ok().as_deref().and_then(lib_name) {
            libs.push((d.clone(), n, c));
        }
    }
    let built: std::collections::BTreeSet<String> = libs.iter().filter(|l| l.2).map(|l| l.1.to_ascii_lowercase()).collect();
    let mut out = String::new();
    out.push_str("pub static ROUTES: &[Route] = &[\n");
    for (dir, lib, cdylib) in &libs {
        if !cdylib {
            continue;
        }
        let mut files: Vec<PathBuf> = std::fs::read_dir(dir).map(|r| r.filter_map(|e| e.ok().map(|e| e.path())).collect()).unwrap_or_default();
        files.sort();
        for f in files {
            let Some(fname) = f.file_name().and_then(|s| s.to_str()).map(str::to_string) else { continue };
            let (build, target) = if fname == "exports.txt" {
                (0u32, lib.clone())
            } else if let Some(b) = fname.strip_prefix("exports_").and_then(|r| r.strip_suffix(".txt")).and_then(|b| b.parse::<u32>().ok()) {
                (b, format!("{lib}_{b}"))
            } else {
                continue;
            };
            println!("cargo:rerun-if-changed={}", f.display());
            if !built.contains(&target.to_ascii_lowercase()) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&f) else { continue };
            let console = format!("{}.dll", lib.to_ascii_lowercase());
            for r in rows(&text) {
                if !ROUTED.contains(&r.status.as_str()) {
                    continue;
                }
                let _ = writeln!(
                    out,
                    "    Route {{ console: \"{}\", func: \"{}\", ordinal: {}, lib: \"{}\", build: {}, status: \"{}\", forward: \"{}\" }},",
                    esc(&console),
                    esc(&r.name),
                    r.ordinal,
                    esc(&target),
                    build,
                    esc(&r.status),
                    esc(&r.forward)
                );
            }
        }
    }
    out.push_str("];\n\npub static APISET_HOSTS: &[ApiSetHost] = &[\n");
    if let Ok(text) = std::fs::read_to_string(&hosts) {
        for l in text.lines() {
            let f: Vec<&str> = l.split_whitespace().collect();
            if f.len() != 4 {
                continue;
            }
            let Ok(b) = f[0].parse::<u32>() else { continue };
            let _ = writeln!(out, "    ApiSetHost {{ build: {b}, apiset: \"{}\", func: \"{}\", host: \"{}\" }},", esc(f[1]), esc(f[2]), esc(f[3]));
        }
    }
    out.push_str("];\n");
    let dst = Path::new(&std::env::var("OUT_DIR").unwrap()).join("compat_routes.rs");
    std::fs::write(dst, out).unwrap();
}
