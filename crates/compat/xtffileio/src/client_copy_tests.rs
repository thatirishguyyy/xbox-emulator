use crate::client::*;
use crate::client_copy::{CopyCallbackVtbl, PERF_READ_PENDING, PERF_SIZE, PERF_WRITE_PENDING};
use crate::client_tests::{open_client, Client, Dispatcher};
use crate::com::*;
use crate::tests::{fresh, guard, host};
use core::ffi::c_void;
use core::ptr::null_mut;
use std::sync::Mutex;

#[derive(Clone, Debug, PartialEq)]
enum Ev {
    Start { root: String, name: String, dest: String, size: u64 },
    Progress { src: String, dst: String, size: u64, done: u64 },
    End { dest: String, hr: HRESULT },
}

#[repr(C)]
struct CopyCb {
    vtbl: *const CopyCallbackVtbl,
    events: Mutex<Vec<Ev>>,
    fail_progress_after: Option<u64>,
}

unsafe fn s(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut n = 0;
    while *p.add(n) != 0 {
        n += 1;
    }
    String::from_utf16_lossy(core::slice::from_raw_parts(p, n))
}

unsafe extern "system" fn qi(_: *mut c_void, _: *const GUID, _: *mut *mut c_void) -> HRESULT {
    E_NOINTERFACE
}

unsafe extern "system" fn rf(_: *mut c_void) -> u32 {
    1
}

unsafe extern "system" fn on_start(this: *mut c_void, root: *const u16, _pattern: *const u16, info: *const XtfFileInfo, dest: *const u16) -> HRESULT {
    let me = &*(this as *const CopyCb);
    me.events.lock().unwrap().push(Ev::Start { root: s(root), name: s((*info).file_name), dest: s(dest), size: (*info).size });
    S_OK
}

unsafe extern "system" fn on_progress(this: *mut c_void, src: *const u16, dst: *const u16, size: u64, done: u64) -> HRESULT {
    let me = &*(this as *const CopyCb);
    me.events.lock().unwrap().push(Ev::Progress { src: s(src), dst: s(dst), size, done });
    match me.fail_progress_after {
        Some(n) if done >= n => E_ABORT,
        _ => S_OK,
    }
}

unsafe extern "system" fn on_end(this: *mut c_void, _root: *const u16, _pattern: *const u16, _info: *const XtfFileInfo, dest: *const u16, hr: HRESULT) -> HRESULT {
    let me = &*(this as *const CopyCb);
    me.events.lock().unwrap().push(Ev::End { dest: s(dest), hr });
    S_OK
}

const E_ABORT: HRESULT = 0x8000_4004u32 as i32;

static CB_VTBL: CopyCallbackVtbl = CopyCallbackVtbl {
    base: UnknownVtbl { query_interface: qi, add_ref: rf, release: rf },
    on_start_file_copy: on_start,
    on_file_copy_progress: on_progress,
    on_end_file_copy: on_end,
};

impl CopyCb {
    fn new(fail_progress_after: Option<u64>) -> Box<CopyCb> {
        Box::new(CopyCb { vtbl: &CB_VTBL, events: Mutex::new(Vec::new()), fail_progress_after })
    }
    fn ptr(&self) -> *mut c_void {
        self as *const CopyCb as *mut c_void
    }
    fn events(&self) -> Vec<Ev> {
        self.events.lock().unwrap().clone()
    }
}

fn w(s: &str) -> Vec<u16> {
    wide(s)
}

fn pattern_bytes(n: usize, seed: u8) -> Vec<u8> {
    (0..n).map(|i| (i as u32).wrapping_mul(2654435761).wrapping_add(seed as u32).to_le_bytes()[1]).collect()
}

const SIZES: [usize; 4] = [0, 0x1234, 0x80005, 0x1a0033];

fn populate(base: &str) {
    std::fs::create_dir_all(host(&format!("{base}\\sub"))).unwrap();
    for (i, n) in SIZES.iter().enumerate() {
        std::fs::write(host(&format!("{base}\\f{i}.bin")), pattern_bytes(*n, i as u8)).unwrap();
    }
    std::fs::write(host(&format!("{base}\\sub\\deep.txt")), b"nested").unwrap();
    let ro = host(&format!("{base}\\f1.bin"));
    let mut p = std::fs::metadata(&ro).unwrap().permissions();
    p.set_readonly(true);
    std::fs::set_permissions(&ro, p).unwrap();
}

fn clear_readonly(base: &str) {
    for f in ["f1.bin"] {
        let h = host(&format!("{base}\\{f}"));
        if let Ok(m) = std::fs::metadata(&h) {
            let mut p = m.permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            p.set_readonly(false);
            let _ = std::fs::set_permissions(&h, p);
        }
    }
}

fn check_copy(src: &str, dst: &str) {
    for (i, n) in SIZES.iter().enumerate() {
        let a = host(&format!("{src}\\f{i}.bin"));
        let b = host(&format!("{dst}\\f{i}.bin"));
        let got = std::fs::read(&b).unwrap_or_else(|e| panic!("{b:?}: {e}"));
        assert_eq!(got.len(), *n, "{b:?}");
        assert!(got == pattern_bytes(*n, i as u8), "content {b:?}");
        let ma = std::fs::metadata(&a).unwrap();
        let mb = std::fs::metadata(&b).unwrap();
        assert_eq!(ma.modified().unwrap(), mb.modified().unwrap(), "write time {b:?}");
        assert_eq!(mb.permissions().readonly(), i == 1, "readonly {b:?}");
    }
    assert_eq!(std::fs::read(host(&format!("{dst}\\sub\\deep.txt"))).unwrap(), b"nested");
}

unsafe fn copy(c: &Client, pattern: &str, recursion: u32, dst: &str, cb: &CopyCb) -> HRESULT {
    (c.vt().copy_files)(c.0, w(pattern).as_ptr(), 0, 0, recursion, w(dst).as_ptr(), 0, cb.ptr())
}

unsafe fn round_trip(c: &Client, tag: &str) {
    let src = fresh(&format!("{tag}_src"));
    let mid = fresh(&format!("{tag}_mid"));
    let back = fresh(&format!("{tag}_back"));
    populate(&src);
    let cb = CopyCb::new(None);
    assert_eq!(copy(c, &format!("{src}\\*"), u32::MAX, &format!("X{mid}\\"), &cb), S_OK, "{tag} to console");
    check_copy(&src, &mid);
    let ev = cb.events();
    let starts = ev.iter().filter(|e| matches!(e, Ev::Start { .. })).count();
    let ends: Vec<&Ev> = ev.iter().filter(|e| matches!(e, Ev::End { .. })).collect();
    assert_eq!(starts, SIZES.len() + 1, "{tag} {ev:?}");
    assert_eq!(ends.len(), SIZES.len() + 1, "{tag}");
    assert!(ends.iter().all(|e| matches!(e, Ev::End { hr: 0, .. })));
    let big = format!("{mid}\\f3.bin");
    let last = ev.iter().filter_map(|e| match e {
        Ev::Progress { dst, size, done, .. } if *dst == big => Some((*size, *done)),
        _ => None,
    });
    let v: Vec<(u64, u64)> = last.collect();
    assert!(v.len() >= 3, "{tag} progress {v:?}");
    assert_eq!(v.first().unwrap().1, 0);
    assert_eq!(*v.last().unwrap(), (SIZES[3] as u64, SIZES[3] as u64));
    assert!(v.windows(2).all(|p| p[0].1 <= p[1].1));
    let cb2 = CopyCb::new(None);
    assert_eq!(copy(c, &format!("X{mid}\\*"), u32::MAX, &format!("{back}\\"), &cb2), S_OK, "{tag} from console");
    check_copy(&src, &back);
    assert_eq!(cb2.events().iter().filter(|e| matches!(e, Ev::End { hr: 0, .. })).count(), SIZES.len() + 1);
    let single = format!("{back}\\renamed.bin");
    assert_eq!(copy(c, &format!("X{mid}\\f2.bin"), 0, &single, &CopyCb::new(None)), S_OK, "{tag} single from console");
    assert!(std::fs::read(host(&single)).unwrap() == pattern_bytes(SIZES[2], 2));
    let up = format!("X{mid}\\up.bin");
    assert_eq!(copy(c, &format!("{src}\\f0.bin"), 0, &up, &CopyCb::new(None)), S_OK, "{tag} single to console");
    assert_eq!(std::fs::read(host(&format!("{mid}\\up.bin"))).unwrap().len(), 0);
    for b in [&src, &mid, &back] {
        clear_readonly(b);
    }
}

fn legacy_ops() -> Vec<(u32, OperationFn)> {
    crate::operation_table().into_iter().filter(|(id, _)| *id != OP_TRANSFER_FILES_SUPPORTED).collect()
}

#[test]
fn copy_files_legacy_protocol_both_directions() {
    let _g = guard();
    let port = 47948;
    let _d = unsafe { Dispatcher::custom(port, &legacy_ops()) };
    let c = open_client(port, "127.0.0.1").unwrap();
    unsafe {
        assert_eq!(c.inner().direct_protocol_level(), 0);
        round_trip(&c, "cpl");
    }
}

#[cfg(fileio_b)]
unsafe extern "system" fn level_one(_session: usize, _request: *mut c_void, response: *mut c_void) -> HRESULT {
    let w = wide("{\"Result\":0}");
    (vt::<MessageVtbl>(response).set_json_body_w)(response, w.as_ptr())
}

#[cfg(fileio_b)]
#[test]
fn copy_files_direct_protocol_level_one_both_directions() {
    let _g = guard();
    let port = 47949;
    let mut ops = legacy_ops();
    ops.push((OP_TRANSFER_FILES_SUPPORTED, level_one as OperationFn));
    let _d = unsafe { Dispatcher::custom(port, &ops) };
    let c = open_client(port, "127.0.0.1").unwrap();
    unsafe {
        assert_eq!(c.inner().direct_protocol_level(), 1);
        round_trip(&c, "cd1");
    }
}

#[cfg(fileio_b)]
#[test]
fn copy_files_direct_protocol_level_two_both_directions_with_perf() {
    let _g = guard();
    let port = 47950;
    let _d = unsafe { Dispatcher::fileio(port) };
    let c = open_client(port, "127.0.0.1").unwrap();
    unsafe {
        assert_eq!(c.inner().direct_protocol_level(), 2);
        round_trip(&c, "cd2");
        let src = fresh("cd2p_src");
        let dst = fresh("cd2p_dst");
        std::fs::write(host(&format!("{src}\\p.bin")), pattern_bytes(0x90000, 9)).unwrap();
        let mut i3 = null_mut();
        assert_eq!((c.vt().query_interface)(c.0, &IID_XTF_FILEIO_CLIENT3, &mut i3), S_OK);
        let v3 = &**(i3 as *const *const Client3Vtbl);
        let mut perf = [0xAAu8; PERF_SIZE];
        let hr = (v3.copy_files_perf)(i3, w(&format!("{src}\\p.bin")).as_ptr(), 0, 0, 0, w(&format!("X{dst}\\p.bin")).as_ptr(), 0, perf.as_mut_ptr() as *mut c_void, null_mut());
        assert_eq!(hr, S_OK);
        assert_eq!(perf[PERF_READ_PENDING], 1);
        assert_eq!(perf[PERF_WRITE_PENDING], 0);
        let f = |o: usize| f64::from_le_bytes(perf[o..o + 8].try_into().unwrap());
        assert!(f(0x78) >= 0.0 && f(0xa8) >= 0.0);
        let pct: f64 = (0..5).map(|i| f(0x20 + i * 0x10)).sum();
        assert!((pct - 100.0).abs() < 1e-6 || pct == 0.0, "{pct}");
        assert_eq!(perf[0], 0);
        let mut perf2 = [0xAAu8; PERF_SIZE];
        let back = fresh("cd2p_back");
        let hr = (v3.copy_files_perf)(i3, w(&format!("X{dst}\\p.bin")).as_ptr(), 0, 0, 0, w(&format!("{back}\\p.bin")).as_ptr(), 0, perf2.as_mut_ptr() as *mut c_void, null_mut());
        assert_eq!(hr, S_OK);
        assert_eq!(perf2[PERF_READ_PENDING], 0);
        assert_eq!(perf2[PERF_WRITE_PENDING], 1);
        assert!(std::fs::read(host(&format!("{back}\\p.bin"))).unwrap() == pattern_bytes(0x90000, 9));
        assert_eq!((v3.copy_files_direct_list)(i3, null_mut(), 0, null_mut(), null_mut(), 0, null_mut(), null_mut()), E_INVALIDARG);
        assert_eq!((v3.copy_files_direct_list)(i3, perf.as_mut_ptr() as *mut c_void, 1, null_mut(), null_mut(), 0, null_mut(), null_mut()), E_INVALIDARG);
        release(i3);
    }
}

#[test]
fn copy_files_client2_copies_each_named_file() {
    let _g = guard();
    let port = 47951;
    let _d = unsafe { Dispatcher::fileio(port) };
    let c = open_client(port, "127.0.0.1").unwrap();
    let src = fresh("cc2_src");
    let dst = fresh("cc2_dst");
    std::fs::write(host(&format!("{src}\\a.txt")), b"alpha").unwrap();
    std::fs::write(host(&format!("{src}\\b.txt")), b"beta").unwrap();
    unsafe {
        let mut i2 = null_mut();
        assert_eq!((c.vt().query_interface)(c.0, &IID_XTF_FILEIO_CLIENT2, &mut i2), S_OK);
        let v2 = &**(i2 as *const *const Client2Vtbl);
        let a = w("a.txt");
        let b = w("b.txt");
        let names = [a.as_ptr(), b.as_ptr()];
        let cb = CopyCb::new(None);
        assert_eq!((v2.copy_files)(i2, w(&src).as_ptr(), names.as_ptr(), 2, w(&format!("X{dst}\\")).as_ptr(), 0, cb.ptr()), S_OK);
        assert_eq!(std::fs::read(host(&format!("{dst}\\a.txt"))).unwrap(), b"alpha");
        assert_eq!(std::fs::read(host(&format!("{dst}\\b.txt"))).unwrap(), b"beta");
        assert_eq!(cb.events().iter().filter(|e| matches!(e, Ev::End { hr: 0, .. })).count(), 2);
        let missing = w("zzz.txt");
        let names = [a.as_ptr(), missing.as_ptr()];
        assert_eq!((v2.copy_files)(i2, w(&src).as_ptr(), names.as_ptr(), 2, w(&format!("X{dst}\\")).as_ptr(), 0, null_mut()), HR_FILE_NOT_FOUND);
        assert_eq!((v2.copy_files)(i2, w(&src).as_ptr(), core::ptr::null(), 1, w(&format!("X{dst}\\")).as_ptr(), 0, null_mut()), E_INVALIDARG);
        release(i2);
    }
}

#[test]
fn copy_files_rejects_bad_arguments_and_honours_callback_abort() {
    let _g = guard();
    let port = 47952;
    let _d = unsafe { Dispatcher::custom(port, &legacy_ops()) };
    let c = open_client(port, "127.0.0.1").unwrap();
    let src = fresh("cce_src");
    let dst = fresh("cce_dst");
    std::fs::write(host(&format!("{src}\\big.bin")), pattern_bytes(0x300000, 3)).unwrap();
    std::fs::write(host(&format!("{src}\\two.bin")), b"2").unwrap();
    unsafe {
        let none = CopyCb::new(None);
        assert_eq!(copy(&c, &format!("{src}\\big.bin"), 0, &format!("{dst}\\x.bin"), &none), E_INVALIDARG);
        assert_eq!(copy(&c, &format!("X{src}\\big.bin"), 0, &format!("X{dst}\\x.bin"), &none), E_INVALIDARG);
        assert_eq!(copy(&c, &format!("{src}\\nothing*.bin"), 0, &format!("X{dst}\\"), &none), HR_FILE_NOT_FOUND);
        std::fs::write(host(&format!("{dst}\\file")), b"f").unwrap();
        assert_eq!(copy(&c, &format!("{src}\\*.bin"), 0, &format!("X{dst}\\file"), &none), E_INVALIDARG);
        assert_eq!((c.vt().copy_files)(c.0, core::ptr::null(), 0, 0, 0, w("XD:\\a").as_ptr(), 0, null_mut()), E_INVALIDARG);
        let ok = CopyCb::new(None);
        assert_eq!(copy(&c, &format!("{src}\\two.bin"), 0, &format!("X{dst}\\two.bin"), &ok), S_OK);
        assert_eq!(std::fs::read(host(&format!("{dst}\\two.bin"))).unwrap(), b"2");
        match &ok.events()[0] {
            Ev::Start { root, name, dest, size } => {
                assert_eq!(root, &src);
                assert_eq!(name, &format!("{src}\\two.bin"));
                assert_eq!(dest, &format!("{dst}\\two.bin"));
                assert_eq!(*size, 1);
            }
            e => panic!("{e:?}"),
        }
        let abort = CopyCb::new(Some(1));
        assert_eq!(copy(&c, &format!("{src}\\big.bin"), 0, &format!("X{dst}\\big.bin"), &abort), E_ABORT);
        assert!(!abort.events().iter().any(|e| matches!(e, Ev::End { .. })));
        assert!(!host(&format!("{dst}\\big.bin")).exists());
        drop(c);
        let c = open_client(port, "127.0.0.1").unwrap();
        let abort2 = CopyCb::new(Some(1));
        assert_eq!(copy(&c, &format!("X{src}\\big.bin"), 0, &format!("{dst}\\down.bin"), &abort2), E_ABORT);
    }
}
