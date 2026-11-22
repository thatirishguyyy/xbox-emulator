use std::collections::BTreeSet;
use std::path::PathBuf;

const CONSOLE_DLLS: [&str; 2] = [r"D:\xbox-pc-port\games\XDefiant\extracted\EmbeddedXvd\Windows\System32\xtfdebugmonitor.dll", r"D:\xbox-pc-port\games\Fortnite\extracted\EmbeddedXvd\Windows\System32\xtfdebugmonitor.dll"];

fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes(b[o..o + 2].try_into().unwrap())
}

fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

fn u64_at(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}

fn cstr(b: &[u8], o: usize) -> String {
    let n = b[o..].iter().position(|&c| c == 0).unwrap();
    String::from_utf8_lossy(&b[o..o + n]).into_owned()
}

fn imports(bytes: &[u8]) -> Vec<(String, BTreeSet<String>, bool)> {
    let pe = u32_at(bytes, 0x3c) as usize;
    assert_eq!(&bytes[pe..pe + 4], b"PE\0\0");
    let nsec = u16_at(bytes, pe + 6) as usize;
    let opt = pe + 24;
    assert_eq!(u16_at(bytes, opt), 0x20b);
    let secs = opt + u16_at(bytes, pe + 20) as usize;
    let map = |rva: u32| -> usize {
        (0..nsec)
            .find_map(|i| {
                let s = secs + i * 40;
                let va = u32_at(bytes, s + 12);
                let vs = u32_at(bytes, s + 8).max(u32_at(bytes, s + 16));
                (rva >= va && rva < va + vs).then(|| (rva - va + u32_at(bytes, s + 20)) as usize)
            })
            .unwrap_or_else(|| panic!("rva {rva:#x} outside every section"))
    };
    let mut out = Vec::new();
    let dir = u32_at(bytes, opt + 112 + 8);
    if dir != 0 {
        let mut d = map(dir);
        while u32_at(bytes, d + 12) != 0 {
            let name = cstr(bytes, map(u32_at(bytes, d + 12)));
            let ilt = if u32_at(bytes, d) != 0 { u32_at(bytes, d) } else { u32_at(bytes, d + 16) };
            let mut t = map(ilt);
            let mut names = BTreeSet::new();
            loop {
                let e = u64_at(bytes, t);
                if e == 0 {
                    break;
                }
                names.insert(if e >> 63 != 0 { format!("#{}", e & 0xffff) } else { cstr(bytes, map(e as u32) + 2) });
                t += 8;
            }
            out.push((name, names, false));
            d += 20;
        }
    }
    let delay = u32_at(bytes, opt + 112 + 13 * 8);
    if delay != 0 {
        let mut d = map(delay);
        while u32_at(bytes, d + 4) != 0 {
            out.push((cstr(bytes, map(u32_at(bytes, d + 4))), BTreeSet::new(), true));
            d += 32;
        }
    }
    out
}

fn xbtp_imports(path: &PathBuf) -> (BTreeSet<String>, usize) {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let all = imports(&bytes);
    let x: Vec<_> = all.iter().filter(|(n, _, _)| n.eq_ignore_ascii_case("xbtp.dll")).collect();
    assert!(x.iter().all(|(_, _, delay)| !delay), "{}: xbtp.dll must be a static import, not delay-loaded", path.display());
    (x.iter().flat_map(|(_, s, _)| s.iter().cloned()).collect(), x.len())
}

fn built_dll() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    let mut found: Vec<PathBuf> = Vec::new();
    for a in exe.ancestors().skip(1).take(5) {
        found.extend([a.join("xtfdebugmonitor.dll"), a.join("deps").join("xtfdebugmonitor.dll")].into_iter().filter(|p| p.is_file()));
        if let Ok(rd) = std::fs::read_dir(a.join("build").join("xtfdebugmonitor")) {
            found.extend(rd.flatten().map(|e| e.path().join("out").join("xtfdebugmonitor.dll")).filter(|p| p.is_file()));
        }
    }
    found.sort_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());
    found.pop().unwrap_or_else(|| panic!("xtfdebugmonitor.dll not built next to {}", exe.display()))
}

#[test]
fn cdylib_imports_xbtp_statically_like_the_console() {
    let ours = built_dll();
    let (got, blocks) = xbtp_imports(&ours);
    assert_eq!(blocks, 1, "{}", ours.display());
    for c in CONSOLE_DLLS {
        let p = PathBuf::from(c);
        assert!(p.is_file(), "{c} missing");
        let (want, n) = xbtp_imports(&p);
        assert_eq!(n, 1);
        assert_eq!(got, want, "{} vs {c}", ours.display());
    }
}

#[test]
fn test_binary_itself_has_no_xbtp_import() {
    let exe = std::env::current_exe().unwrap();
    let (got, _) = xbtp_imports(&exe);
    assert!(got.is_empty(), "{}: {got:?}", exe.display());
}
