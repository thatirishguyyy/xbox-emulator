#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Export {
    pub ordinal: u16,
    pub name: String,
    pub status: String,
    pub data: bool,
    pub alias: Option<String>,
    pub note: String,
}

pub const STATUSES: [&str; 5] = ["implemented", "forwarded", "stub", "missing", "unsupported"];

pub fn parse(text: &str) -> Vec<Export> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|line| {
            let (l, note) = line.split_once(" | ").map_or((line, ""), |(a, b)| (a, b.trim()));
            let f: Vec<&str> = l.split_whitespace().collect();
            assert!(f.len() >= 3, "bad export line: {line}");
            let mut data = false;
            let mut alias = None;
            for x in &f[3..] {
                if *x == "data" {
                    data = true;
                } else if let Some(a) = x.strip_prefix('=') {
                    alias = Some(a.to_string());
                } else {
                    panic!("bad export line: {line}");
                }
            }
            Export {
                ordinal: f[0].parse().unwrap_or_else(|_| panic!("bad ordinal: {line}")),
                name: f[1].to_string(),
                status: f[2].to_string(),
                data,
                alias,
                note: note.to_string(),
            }
        })
        .collect()
}

pub fn internal_symbol(e: &Export) -> String {
    if let Some(a) = &e.alias {
        return a.clone();
    }
    match e.name.strip_prefix('#') {
        Some(n) => format!("XtfOrdinal_{n}"),
        None => e.name.clone(),
    }
}

pub fn link_args(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for e in parse(text) {
        if e.status == "missing" {
            continue;
        }
        let data = if e.data { ",DATA" } else { "" };
        if e.name.starts_with('#') {
            out.push(format!("/EXPORT:{},@{},NONAME{}", internal_symbol(&e), e.ordinal, data));
        } else if let Some(a) = &e.alias {
            out.push(format!("/EXPORT:{}={},@{}{}", e.name, a, e.ordinal, data));
        } else {
            out.push(format!("/EXPORT:{},@{}{}", e.name, e.ordinal, data));
        }
    }
    out
}

pub fn emit_link_args(path: &str) {
    let mut full = std::path::PathBuf::from(path);
    if full.is_relative() {
        if let Ok(dir) = std::env::var("CARGO_MANIFEST_DIR") {
            full = std::path::Path::new(&dir).join(path);
        }
    }
    println!("cargo:rerun-if-changed={}", full.display());
    let text = std::fs::read_to_string(&full).unwrap_or_else(|e| panic!("exports.txt at {}: {e}", full.display()));
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").map(|e| e == "msvc").unwrap_or(false);
    if !msvc {
        return;
    }
    for a in link_args(&text) {
        println!("cargo:rustc-cdylib-link-arg={a}");
    }
}

pub fn table(ex: &[Export]) -> Vec<(u16, String)> {
    let mut v: Vec<(u16, String)> = ex.iter().map(|e| (e.ordinal, e.name.clone())).collect();
    v.sort();
    v
}

pub fn check_against_titles(dll: &str, text: &str) -> usize {
    let ex = parse(text);
    for e in &ex {
        assert!(STATUSES.contains(&e.status.as_str()), "{}: bad status {}", e.name, e.status);
        if e.status == "missing" {
            assert!(!e.note.is_empty(), "{}: missing row needs a reason", e.name);
        }
    }
    let mut ords: Vec<u16> = ex.iter().map(|e| e.ordinal).collect();
    ords.sort();
    ords.dedup();
    assert_eq!(ords.len(), ex.len(), "{dll}: duplicate ordinals");
    let want = table(&ex);
    let mut union = std::collections::BTreeSet::new();
    let refs = crate::pe::title_dlls(dll);
    for (title, p) in &refs {
        let got = crate::pe::export_table(&std::fs::read(p).unwrap()).expect("reference PE");
        for g in &got {
            assert!(want.contains(g), "{dll}: {title} exports {g:?} which exports.txt lacks");
            union.insert(g.clone());
        }
    }
    if !refs.is_empty() {
        for w in &want {
            assert!(union.contains(w), "{dll}: exports.txt row {w:?} is not exported by any title's DLL");
        }
    }
    refs.len()
}

pub fn source_defines(src_dir: &str, e: &Export) -> bool {
    let sym = internal_symbol(e);
    let a = format!("export_name = \"{}\"", sym);
    let b = format!("fn {}(", sym);
    let c = format!("static {}:", sym);
    let Ok(rd) = std::fs::read_dir(src_dir) else { return false };
    for f in rd.flatten() {
        if let Ok(t) = std::fs::read_to_string(f.path()) {
            if t.contains(&a) || t.contains(&b) || t.contains(&c) {
                return true;
            }
        }
    }
    false
}
