include!("../opus_build.rs");

fn public_api(root: &Path) -> Vec<String> {
    let mut names = Vec::new();
    for h in ["opus.h", "opus_multistream.h", "opus_projection.h", "opus_defines.h"] {
        let text = std::fs::read_to_string(root.join("include").join(h)).unwrap();
        let flat = text.replace('\n', " ");
        let mut rest = flat.as_str();
        while let Some(i) = rest.find("OPUS_EXPORT") {
            rest = &rest[i + 11..];
            let decl = &rest[..rest.find('(').unwrap_or(0)];
            if let Some(name) = decl.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).filter(|s| !s.is_empty()).last() {
                if name.starts_with("opus_") && !names.iter().any(|n| n == name) {
                    names.push(name.to_string());
                }
            }
        }
    }
    names
}

fn main() {
    let root = opus_root();
    rerun(&root);
    if !opus_available(&root) {
        println!("cargo:warning=Opus source tree not found at {}; skipping native build", root.display());
        let out_dir = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
        let _ = std::fs::write(out_dir.join("exports.txt"), "");
        return;
    }
    let mut b = opus_build(&root);
    b.files(opus_sources(&root, true));
    b.compile("tombox_opus");
    let names = public_api(&root);
    assert!(names.iter().any(|n| n == "opus_encoder_create") && names.iter().any(|n| n == "opus_decode"), "public API scan failed: {names:?}");
    export(&names);
    std::fs::write(std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("exports.txt"), names.join("\n")).unwrap();
}
