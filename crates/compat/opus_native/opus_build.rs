use std::path::{Path, PathBuf};

fn opus_root() -> PathBuf {
    if let Ok(p) = std::env::var("TOMBOX_OPUS_SRC") {
        return PathBuf::from(p);
    }
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../third_party/opus")
}

fn opus_available(root: &Path) -> bool {
    root.join("celt_sources.mk").is_file()
}

fn mk_list(root: &Path, file: &str, var: &str) -> Vec<PathBuf> {
    let text = std::fs::read_to_string(root.join(file)).unwrap_or_else(|e| panic!("{file}: {e}"));
    let mut out = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        let t = line.trim();
        if !inside {
            if t.starts_with(var) && t[var.len()..].trim_start().starts_with('=') {
                inside = true;
            }
            continue;
        }
        let item = t.trim_end_matches('\\').trim();
        if !item.is_empty() {
            out.push(root.join(item));
        }
        if !t.ends_with('\\') {
            break;
        }
    }
    assert!(!out.is_empty(), "{var} empty in {file}");
    out
}

fn opus_sources(root: &Path, opus_api: bool) -> Vec<PathBuf> {
    let mut v = mk_list(root, "celt_sources.mk", "CELT_SOURCES");
    v.extend(mk_list(root, "silk_sources.mk", "SILK_SOURCES"));
    v.extend(mk_list(root, "silk_sources.mk", "SILK_SOURCES_FIXED"));
    if opus_api {
        v.extend(mk_list(root, "opus_sources.mk", "OPUS_SOURCES"));
        v.extend(mk_list(root, "opus_sources.mk", "OPUS_SOURCES_FLOAT"));
    }
    v
}

fn opus_build(root: &Path) -> cc::Build {
    let mut b = cc::Build::new();
    for d in ["include", "celt", "silk", "silk/fixed", "silk/float", "dnn", "src"] {
        b.include(root.join(d));
    }
    b.define("OPUS_BUILD", None)
        .define("FIXED_POINT", "1")
        .define("USE_ALLOCA", None)
        .define("HAVE_LRINTF", None)
        .define("PACKAGE_VERSION", "\"316c675fa1c9-tombox\"")
        .define("_CRT_SECURE_NO_WARNINGS", None)
        .warnings(false)
        .static_crt(false);
    b
}

fn rerun(root: &Path) {
    println!("cargo:rerun-if-env-changed=TOMBOX_OPUS_SRC");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../opus_build.rs");
    for f in ["celt_sources.mk", "silk_sources.mk", "opus_sources.mk", "include/opus.h"] {
        println!("cargo:rerun-if-changed={}", root.join(f).display());
    }
}

fn export(names: &[String]) {
    for n in names {
        println!("cargo:rustc-cdylib-link-arg=/EXPORT:{n}");
    }
}
