use core::ffi::c_void;
use std::path::PathBuf;

#[link(name = "kernel32")]
extern "system" {
    fn LoadLibraryW(name: *const u16) -> *mut c_void;
    fn GetProcAddress(h: *mut c_void, name: *const u8) -> *mut c_void;
}

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

pub fn built(file: &str) -> PathBuf {
    let package = file.strip_suffix(".dll").unwrap_or(file);
    xone_exports::build_cdylib(env!("CARGO"), env!("CARGO_MANIFEST_DIR"), package, file)
}

pub fn load_built(file: &str, alias: &str) -> *mut c_void {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let src = built(file);
    let dir = std::env::temp_dir().join(format!("tombox-a32l-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let dst = dir.join(alias);
    if !dst.is_file() {
        std::fs::copy(&src, &dst).unwrap();
    }
    let h = unsafe { LoadLibraryW(wide(dst.to_str().unwrap()).as_ptr()) };
    assert!(!h.is_null(), "LoadLibrary {}", dst.display());
    h
}

pub fn proc(h: *mut c_void, name: &str) -> *mut c_void {
    unsafe { GetProcAddress(h, format!("{name}\0").as_ptr()) }
}

pub fn system(dll: &str) -> *mut c_void {
    let h = unsafe { LoadLibraryW(wide(dll).as_ptr()) };
    assert!(!h.is_null(), "{dll}");
    h
}

pub fn rows() -> Vec<xone_exports::Export> {
    xone_exports::parse(&std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/exports.txt")).unwrap())
}

pub fn forward_target(e: &xone_exports::Export) -> Option<(String, String)> {
    let t = e.note.strip_prefix("PE forwarder to ")?.split(';').next()?.trim();
    let (dll, f) = t.split_once('.')?;
    Some((format!("{dll}.dll"), f.to_string()))
}

pub fn check_table(file: &str) {
    let built = xone_exports::pe_exports(&std::fs::read(built(file)).unwrap()).unwrap();
    let want: Vec<(u16, String)> = {
        let mut v: Vec<(u16, String)> = rows().into_iter().filter(|e| e.status != "missing").map(|e| (e.ordinal, e.name)).collect();
        v.sort();
        v
    };
    assert_eq!(built, want, "built {file} export table differs from exports.txt");
}

pub fn check_forwarders(h: *mut c_void, preload: &[(&str, *mut c_void)]) -> usize {
    let mut n = 0;
    for e in rows() {
        if e.status == "missing" {
            assert!(proc(h, &e.name).is_null(), "{} is missing but exported", e.name);
            continue;
        }
        let got = proc(h, &e.name);
        assert!(!got.is_null(), "{} does not resolve", e.name);
        if let Some((dll, f)) = forward_target(&e) {
            let host = preload.iter().find(|(d, _)| d.eq_ignore_ascii_case(&dll)).map(|(_, h)| *h).unwrap_or_else(|| system(&dll));
            let want = proc(host, &f);
            assert!(!want.is_null(), "{dll}!{f} missing on this PC");
            assert_eq!(got, want, "{} does not forward to {dll}!{f}", e.name);
            n += 1;
        }
    }
    n
}
