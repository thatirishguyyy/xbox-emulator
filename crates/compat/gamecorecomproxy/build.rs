fn main() {
    let p = std::path::Path::new("data/proxy_19041.bin");
    if !p.exists() {
        if let Some(parent) = p.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(p, [0u8; 0x25000]);
    }
    println!("cargo:rerun-if-changed=data/proxy_19041.bin");
    xone_exports::emit_link_args("exports.txt");
}
