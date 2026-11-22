use std::path::PathBuf;

fn built(file: &str) -> PathBuf {
    xone_exports::build_cdylib(env!("CARGO"), env!("CARGO_MANIFEST_DIR"), "hix_xs", file)
}

#[test]
fn export_table_matches_exports_txt() {
    let rows = xone_exports::parse(&std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/exports.txt")).unwrap());
    let mut want: Vec<(u16, String)> = rows.iter().filter(|e| e.status != "missing").map(|e| (e.ordinal, e.name.clone())).collect();
    want.sort();
    let got = xone_exports::pe_exports(&std::fs::read(built("hix_xs.dll")).unwrap()).unwrap();
    assert_eq!(got, want);
    assert!(!rows.iter().any(|e| e.status == "missing"));
    assert!(got.iter().any(|(o, n)| *o == 4 && n == "DebugLive"));
    assert!(got.iter().any(|(o, n)| *o == 8 && n == "WriteHangDump"));
    assert!(got.iter().any(|(o, n)| *o == 3 && n == "CollectHangSummary"));
}
