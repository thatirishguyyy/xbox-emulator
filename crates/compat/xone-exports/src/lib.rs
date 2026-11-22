use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Export {
    pub ordinal: u16,
    pub name: String,
    pub status: String,
    pub data: bool,
    pub note: String,
}

pub fn parse(text: &str) -> Vec<Export> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|line| {
            let (l, note) = line.split_once(" | ").map_or((line, ""), |(a, b)| (a, b.trim()));
            let f: Vec<&str> = l.split_whitespace().collect();
            assert!(f.len() == 3 || (f.len() == 4 && f[3] == "data"), "bad export line: {line}");
            Export {
                ordinal: f[0].parse().unwrap_or_else(|_| panic!("bad ordinal: {line}")),
                name: f[1].to_string(),
                status: f[2].to_string(),
                data: f.len() == 4,
                note: note.to_string(),
            }
        })
        .collect()
}

pub fn emit_link_args(exports_txt: &str) {
    println!("cargo:rerun-if-changed={exports_txt}");
    let text = std::fs::read_to_string(exports_txt).expect("exports.txt");
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").map(|e| e == "msvc").unwrap_or(false);
    if !msvc {
        return;
    }
    for e in parse(&text) {
        let data = if e.data { ",DATA" } else { "" };
        println!("cargo:rustc-cdylib-link-arg=/EXPORT:{},@{}{}", e.name, e.ordinal, data);
    }
}

fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}

pub fn pe_exports(bytes: &[u8]) -> Option<Vec<(u16, String)>> {
    let pe = u32_at(bytes, 0x3c)? as usize;
    if bytes.get(pe..pe + 4)? != b"PE\0\0" {
        return None;
    }
    let nsec = u16_at(bytes, pe + 6)? as usize;
    let opt = pe + 24;
    let optsz = u16_at(bytes, pe + 20)? as usize;
    let magic = u16_at(bytes, opt)?;
    let dd = opt + if magic == 0x20b { 112 } else { 96 };
    let erva = u32_at(bytes, dd)?;
    if erva == 0 {
        return Some(Vec::new());
    }
    let secs = opt + optsz;
    let map = |rva: u32| -> Option<usize> {
        (0..nsec).find_map(|i| {
            let s = secs + i * 40;
            let va = u32_at(bytes, s + 12)?;
            let vs = u32_at(bytes, s + 8)?.max(u32_at(bytes, s + 16)?);
            let raw = u32_at(bytes, s + 20)?;
            (rva >= va && rva < va + vs).then(|| (rva - va + raw) as usize)
        })
    };
    let ed = map(erva)?;
    let base = u32_at(bytes, ed + 16)?;
    let nnames = u32_at(bytes, ed + 24)? as usize;
    let names = map(u32_at(bytes, ed + 32)?)?;
    let ords = map(u32_at(bytes, ed + 36)?)?;
    let mut out = Vec::with_capacity(nnames);
    for i in 0..nnames {
        let np = map(u32_at(bytes, names + i * 4)?)?;
        let end = bytes[np..].iter().position(|&c| c == 0)?;
        let name = String::from_utf8_lossy(&bytes[np..np + end]).into_owned();
        let ord = u16_at(bytes, ords + i * 2)? as u32 + base;
        out.push((ord as u16, name));
    }
    out.sort();
    Some(out)
}

pub fn table(exports: &[Export]) -> Vec<(u16, String)> {
    let mut v: Vec<(u16, String)> = exports.iter().map(|e| (e.ordinal, e.name.clone())).collect();
    v.sort();
    v
}

pub fn reference_dlls(file: &str) -> Vec<PathBuf> {
    let root = Path::new(r"D:\xbox-pc-port\games");
    let mut out = Vec::new();
    for title in ["Terraria", "Trackmania"] {
        let Ok(rd) = std::fs::read_dir(root.join(title).join("extracted")) else { continue };
        for e in rd.flatten() {
            let p = e.path().join("EmbeddedXvd").join("Windows").join("System32").join(file);
            if p.is_file() {
                out.push(p);
            }
        }
    }
    out
}

pub fn built_dll(crate_dir: &str, file: &str) -> Option<PathBuf> {
    let target = std::env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| Path::new(crate_dir).join("..").join("..").join("..").join("target"));
    ["debug", "release"]
        .iter()
        .map(|p| target.join(p).join(file))
        .filter(|p| p.is_file())
        .max_by_key(|p| p.metadata().and_then(|m| m.modified()).ok())
}

pub fn source_symbols(src_dir: &str) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(src_dir) else { return out };
    for e in rd.flatten() {
        let Ok(text) = std::fs::read_to_string(e.path()) else { continue };
        let mut armed = false;
        for line in text.lines() {
            let l = line.trim();
            if l == "#[no_mangle]" {
                armed = true;
                continue;
            }
            if armed {
                for kw in ["fn ", "static "] {
                    if let Some(i) = l.find(kw) {
                        let rest = &l[i + kw.len()..];
                        let n: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                        out.push(n);
                        break;
                    }
                }
                armed = false;
            }
        }
    }
    out.sort();
    out
}

pub const STATUSES: [&str; 4] = ["implemented", "forwarded", "stub", "missing"];

pub fn check_crate(crate_dir: &str, dll: &str, exports_txt: &str) {
    let ex = parse(exports_txt);
    let want = table(&ex);
    for e in &ex {
        assert!(STATUSES.contains(&e.status.as_str()), "{}: bad status {}", e.name, e.status);
    }
    let mut ords: Vec<u16> = ex.iter().map(|e| e.ordinal).collect();
    ords.sort();
    ords.dedup();
    assert_eq!(ords.len(), ex.len(), "duplicate ordinals in {dll} exports.txt");
    let mut syms = source_symbols(&format!("{crate_dir}/src"));
    syms.dedup();
    for e in &ex {
        assert!(syms.contains(&e.name), "{dll}: {} in exports.txt has no #[no_mangle] item", e.name);
    }
    let refs = reference_dlls(dll);
    for r in &refs {
        let got = pe_exports(&std::fs::read(r).unwrap()).expect("reference PE");
        assert_eq!(got, want, "{dll}: exports.txt differs from reference {}", r.display());
    }
    if refs.is_empty() {
        eprintln!("{dll}: no reference DLL in the title dumps; name/ordinal table checked against itself only");
    }
}

fn profile_dir_of(exe: &std::path::Path) -> Option<std::path::PathBuf> {
    exe.ancestors().skip(1).find(|a| matches!(a.file_name().and_then(|n| n.to_str()), Some("debug") | Some("release")) && a.parent().is_some()).map(|a| a.to_path_buf())
}

pub fn build_cdylib(cargo: &str, manifest_dir: &str, package: &str, dll: &str) -> std::path::PathBuf {
    static BUILT: std::sync::Mutex<Vec<(String, std::path::PathBuf)>> = std::sync::Mutex::new(Vec::new());
    let mut built = BUILT.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, p)) = built.iter().find(|(k, _)| k == package) {
        return p.clone();
    }
    let exe = std::env::current_exe().expect("test exe path");
    let profile = profile_dir_of(&exe).unwrap_or_else(|| panic!("no debug/release directory above {}", exe.display()));
    let target = profile.parent().unwrap().to_path_buf();
    let mut cmd = std::process::Command::new(cargo);
    cmd.current_dir(manifest_dir).args(["build", "--offline", "-p", package]).env("CARGO_TARGET_DIR", &target).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::piped());
    if profile.file_name().and_then(|n| n.to_str()) == Some("release") {
        cmd.arg("--release");
    }
    let mut child = cmd.spawn().unwrap_or_else(|e| panic!("spawn {cargo} build -p {package}: {e}"));
    let reader = child.stderr.take().map(|mut e| {
        std::thread::spawn(move || {
            use std::io::Read;
            let mut s = String::new();
            let _ = e.read_to_string(&mut s);
            s
        })
    });
    let start = std::time::Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait().expect("wait for cargo build") {
            break s;
        }
        if start.elapsed() > std::time::Duration::from_secs(900) {
            let _ = child.kill();
            let _ = child.wait();
            panic!("cargo build -p {package} did not finish within 900 s");
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    };
    let err = reader.and_then(|r| r.join().ok()).unwrap_or_default();
    assert!(status.success(), "cargo build -p {package} failed ({status}):\n{err}");
    let p = profile.join(dll);
    assert!(p.is_file(), "cargo build -p {package} succeeded but {} is missing", p.display());
    built.push((package.to_string(), p.clone()));
    p
}

pub fn check_built(crate_dir: &str, dll: &str, exports_txt: &str) {
    let want = table(&parse(exports_txt));
    let Some(p) = built_dll(crate_dir, dll) else {
        panic!("{dll} not built; run cargo build -p for this crate before cargo test");
    };
    let got = pe_exports(&std::fs::read(&p).unwrap()).expect("built PE");
    assert_eq!(got, want, "built {} exports differ from exports.txt", p.display());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_lines() {
        let v = parse("1 A implemented\n\n  28 IID_X implemented data | same GUID as PC\n");
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].note, "");
        assert_eq!(v[1], Export { ordinal: 28, name: "IID_X".into(), status: "implemented".into(), data: true, note: "same GUID as PC".into() });
    }

    #[test]
    #[should_panic]
    fn rejects_bad_line() {
        parse("1 A");
    }

    #[test]
    fn reads_own_test_binary_header() {
        let exe = std::env::current_exe().unwrap();
        assert!(pe_exports(&std::fs::read(exe).unwrap()).is_some());
    }
}
