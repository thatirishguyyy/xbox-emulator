use crate::com::*;
use crate::file;
use crate::find::*;
use crate::handlers::*;
use crate::msg::*;
use crate::paths::*;
use crate::transfer::*;
use core::ffi::c_void;
use core::ptr::null_mut;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Mutex, Once};

static INIT: Once = Once::new();
static SERIAL: Mutex<()> = Mutex::new(());

pub(crate) fn root() -> PathBuf {
    std::env::temp_dir().join(format!("xtffileio-test-{}", std::process::id()))
}

pub(crate) fn guard() -> std::sync::MutexGuard<'static, ()> {
    INIT.call_once(|| {
        let r = root();
        let _ = std::fs::remove_dir_all(&r);
        std::fs::create_dir_all(r.join("console").join("fs").join("D")).unwrap();
        std::fs::create_dir_all(r.join("console").join("fs").join("C").join("Windows").join("System32")).unwrap();
        std::env::set_var("TOMBOX_GDK_ROOT", &r);
    });
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

pub(crate) fn host(console: &str) -> PathBuf {
    host_path(console).unwrap().path
}

pub(crate) fn fresh(dir: &str) -> String {
    let c = format!("D:\\{dir}");
    let h = host(&c);
    let _ = std::fs::remove_dir_all(&h);
    std::fs::create_dir_all(&h).unwrap();
    c
}

fn jstr(s: &str) -> String {
    s.replace('\\', "\\\\")
}

#[repr(C)]
struct MockResponse {
    vtbl: *const MessageVtbl,
    json: Option<String>,
}

unsafe extern "system" fn m_qi(_: *mut c_void, _: *const GUID, _: *mut *mut c_void) -> HRESULT {
    E_FAIL
}
unsafe extern "system" fn m_ref(_: *mut c_void) -> u32 {
    1
}
unsafe extern "system" fn m_get(_: *mut c_void, _: *mut u32, _: *mut *mut u8) -> HRESULT {
    E_FAIL
}
unsafe extern "system" fn m_set(_: *mut c_void, _: u32, _: *const u8) -> HRESULT {
    E_FAIL
}
unsafe extern "system" fn m_set_w(this: *mut c_void, w: *const u16) -> HRESULT {
    (*(this as *mut MockResponse)).json = Some(from_wide(w));
    S_OK
}

static RESPONSE_VTBL: MessageVtbl = MessageVtbl {
    base: UnknownVtbl { query_interface: m_qi, add_ref: m_ref, release: m_ref },
    get_json_body: m_get,
    get_binary_body: m_get,
    set_json_body: m_set,
    set_json_body_w: m_set_w,
    set_binary_body: m_set,
};

impl MockResponse {
    fn new() -> Box<Self> {
        Box::new(MockResponse { vtbl: &RESPONSE_VTBL, json: None })
    }
    fn ptr(&mut self) -> *mut c_void {
        self as *mut _ as *mut c_void
    }
}

#[derive(Clone, Debug)]
struct Sent {
    json: String,
    binary: Vec<u8>,
    is_async: bool,
}

#[repr(C)]
struct MockSession {
    vtbl: *const SessionVtbl,
    sent: Vec<Sent>,
    incoming: VecDeque<Com>,
    pending_first: u32,
}

unsafe fn capture(msg: *mut c_void, is_async: bool) -> Sent {
    let mut n = 0u32;
    let mut p: *mut u8 = null_mut();
    (vt::<MessageVtbl>(msg).get_json_body)(msg, &mut n, &mut p);
    let json = if p.is_null() { String::new() } else { String::from_utf8_lossy(core::slice::from_raw_parts(p, n as usize)).trim_end_matches('\0').to_string() };
    let mut bn = 0u32;
    let mut bp: *mut u8 = null_mut();
    (vt::<MessageVtbl>(msg).get_binary_body)(msg, &mut bn, &mut bp);
    let binary = if bp.is_null() { Vec::new() } else { core::slice::from_raw_parts(bp, bn as usize).to_vec() };
    Sent { json, binary, is_async }
}

unsafe extern "system" fn s_send(this: *mut c_void, msg: *mut c_void) -> HRESULT {
    let s = &mut *(this as *mut MockSession);
    s.sent.push(capture(msg, false));
    S_OK
}
unsafe extern "system" fn s_send_async(this: *mut c_void, msg: *mut c_void, cb: Option<MessageSentCallback>, ctx: *mut c_void) -> HRESULT {
    let s = &mut *(this as *mut MockSession);
    s.sent.push(capture(msg, true));
    if let Some(cb) = cb {
        cb(0, msg as usize, 0, ctx);
    }
    S_OK
}
unsafe extern "system" fn s_recv(this: *mut c_void, timeout: u32, out: *mut *mut c_void) -> HRESULT {
    let s = &mut *(this as *mut MockSession);
    assert_eq!(timeout, XTF_RECEIVE_RECOMMENDED_TIMEOUT_MS);
    if s.pending_first > 0 {
        s.pending_first -= 1;
        return E_PENDING;
    }
    match s.incoming.pop_front() {
        Some(mut m) => {
            *out = core::mem::replace(&mut m.0, null_mut());
            S_OK
        }
        None => HR_WSAECONNRESET,
    }
}

static SESSION_VTBL: SessionVtbl = SessionVtbl {
    base: UnknownVtbl { query_interface: m_qi, add_ref: m_ref, release: m_ref },
    send_more_message: s_send,
    send_more_message_async: s_send_async,
    receive_more_message: s_recv,
};

impl MockSession {
    fn new() -> Box<Self> {
        Box::new(MockSession { vtbl: &SESSION_VTBL, sent: Vec::new(), incoming: VecDeque::new(), pending_first: 0 })
    }
    fn id(&mut self) -> usize {
        self as *mut _ as usize
    }
}

unsafe fn message(json: &str, binary: &[u8]) -> Com {
    let mut m = Com::null();
    assert_eq!(XbtpCreateMessage(&IID_XBTP_MESSAGE_IMPL, m.out()), S_OK);
    let mut b = json.as_bytes().to_vec();
    b.push(0);
    assert_eq!((vt::<MessageImplVtbl>(m.0).set_json_body_copy)(m.0, b.len() as u32, b.as_ptr()), S_OK);
    if !binary.is_empty() {
        assert_eq!((vt::<MessageImplVtbl>(m.0).set_binary_body_copy)(m.0, binary.len() as u32, binary.as_ptr()), S_OK);
    }
    m
}

unsafe fn parse(json: &str) -> Com {
    let mut b = json.as_bytes().to_vec();
    b.push(0);
    let mut o = Com::null();
    assert_eq!(XbtpGetJsonObjectFromUtf8(b.as_ptr(), o.out()), S_OK, "{json}");
    o
}

fn num_i32(json: &str, key: &str) -> i32 {
    unsafe {
        let o = parse(json);
        let mut v = 0;
        assert_eq!(get_i32(o.0, key, &mut v), S_OK, "{key} in {json}");
        v
    }
}

fn num_u32(json: &str, key: &str) -> u32 {
    unsafe {
        let o = parse(json);
        let mut v = 0;
        assert_eq!(get_u32(o.0, key, &mut v), S_OK, "{key} in {json}");
        v
    }
}

fn num_u64(json: &str, key: &str) -> u64 {
    unsafe {
        let o = parse(json);
        let mut v = 0;
        assert_eq!(get_u64(o.0, key, &mut v), S_OK, "{key} in {json}");
        v
    }
}

fn str_of(json: &str, key: &str) -> Option<String> {
    unsafe {
        let o = parse(json);
        let mut b = Bstr::null();
        if get_string(o.0, key, &mut b) < 0 {
            return None;
        }
        Some(b.to_string())
    }
}

fn call(f: OperationFn, session: usize, json: &str) -> String {
    unsafe {
        let req = message(json, &[]);
        let mut resp = MockResponse::new();
        assert_eq!(f(session, req.0, resp.ptr()), S_OK);
        resp.json.clone().unwrap()
    }
}

fn result_of(f: OperationFn, json: &str) -> i32 {
    num_i32(&call(f, 0, json), "Result")
}

#[test]
fn xbtp_dll_loads_for_json_objects() {
    unsafe {
        let mut o = Com::null();
        assert_eq!(XbtpCreateJsonObject(o.out()), S_OK);
        assert!(!o.0.is_null());
    }
}

#[test]
fn path_type_matches_console_rules() {
    let w = |s: &str| s.encode_utf16().collect::<Vec<u16>>();
    assert_eq!(path_type(&w("")), Err(E_INVALIDARG));
    assert_eq!(path_type(&w("XD:\\a")), Ok((PATH_CONSOLE, 1)));
    assert_eq!(path_type(&w("xg:\\a")), Ok((PATH_CONSOLE, 1)));
    assert_eq!(path_type(&w("X1:\\a")), Ok((PATH_NORMAL, 0)));
    assert_eq!(path_type(&w("{pkg_1}:\\dir")), Ok((PATH_APPLICATION, 9)));
    assert_eq!(path_type(&w("{pkg}x")), Ok((PATH_NORMAL, 0)));
    assert_eq!(path_type(&w("D:\\a")), Ok((PATH_NORMAL, 0)));
}

#[test]
fn full_path_name_normalises_like_getfullpathname() {
    assert_eq!(full_path_name("d:\\a\\.\\b\\..\\c").unwrap(), "D:\\a\\c");
    assert_eq!(full_path_name("D:").unwrap(), "D:\\");
    assert_eq!(full_path_name("D:\\dir\\").unwrap(), "D:\\dir\\");
    assert_eq!(full_path_name("foo.txt").unwrap(), "C:\\Windows\\System32\\foo.txt");
    assert_eq!(full_path_name("\\x").unwrap(), "C:\\x");
    assert_eq!(full_path_name("D:\\a\\b. ").unwrap(), "D:\\a\\b");
    assert_eq!(full_path_name("").unwrap_err(), hr_from_win32(123));
}

#[test]
fn entry_point_checks_guids_and_registers_handlers() {
    let _g = guard();
    unsafe {
        let mut p: *mut c_void = null_mut();
        let bad = GUID::new(1, 2, 3, [0; 8]);
        assert_eq!(crate::XtfOrdinal_4(&bad, &IID_XBTP_EXTENSION_METADATA_IMPL, &mut p), E_INVALIDARG);
        assert_eq!(crate::XtfOrdinal_4(&FILEIO_EXTENSION_ID, &bad, &mut p), E_INVALIDARG);
        assert_eq!(crate::XtfOrdinal_4(&FILEIO_EXTENSION_ID, &IID_XBTP_EXTENSION_METADATA_IMPL, &mut p), S_OK);
        assert!(!p.is_null());
        let count: unsafe extern "system" fn(*mut c_void, *mut u32) -> HRESULT = core::mem::transmute(vt::<MetadataVtbl>(p).get_handler_count);
        let mut n = 0u32;
        assert_eq!(count(p, &mut n), S_OK);
        assert_eq!(n as usize, crate::operation_table().len());
        release(p);
    }
    let ids: Vec<u32> = crate::operation_table().iter().map(|e| e.0).collect();
    assert_eq!(&ids[..15], &[6, 5, 2, 1, 4, 7, 3, 0xa, 0xb, 9, 0xc, 0xd, 0xe, 0xf, 0x10]);
}

#[test]
fn create_directory_handler_creates_parents_and_reports_conflicts() {
    let _g = guard();
    let base = fresh("mk");
    let p = format!("{base}\\a\\b\\c");
    assert_eq!(result_of(create_directory_handler, &format!("{{\"FileName\":\"{}\"}}", jstr(&p))), S_OK);
    assert!(host(&p).is_dir());
    assert_eq!(result_of(create_directory_handler, &format!("{{\"FileName\":\"{}\"}}", jstr(&p))), S_OK);
    std::fs::write(host(&format!("{base}\\f")), b"x").unwrap();
    assert_eq!(result_of(create_directory_handler, &format!("{{\"FileName\":\"{}\"}}", jstr(&format!("{base}\\f")))), E_ACCESSDENIED);
    assert_eq!(result_of(create_directory_handler, "{\"FileName\":\"Q:\\\\nodrive\\\\x\"}"), HR_INVALID_DRIVE);
    assert_eq!(result_of(create_directory_handler, "{\"FileName\":\"{pkg}:\\\\x\"}"), HR_FILE_NOT_FOUND);
    assert_eq!(result_of(create_directory_handler, "{}"), E_JSON_NOT_FOUND);
    assert_eq!(result_of(create_directory_handler, &format!("{{\"FileName\":\"X{}\\\\xc\"}}", jstr(&base))), S_OK);
    assert!(host(&format!("{base}\\xc")).is_dir());
}

#[test]
fn delete_file_handler_clears_readonly_and_defers() {
    let _g = guard();
    let base = fresh("del");
    let f = format!("{base}\\ro.txt");
    std::fs::write(host(&f), b"x").unwrap();
    assert_eq!(file::set_file_attributes(&f, 1), S_OK);
    assert_eq!(result_of(delete_file_handler, &format!("{{\"FileName\":\"{}\",\"Flags\":0}}", jstr(&f))), S_OK);
    assert!(!host(&f).exists());
    assert_eq!(result_of(delete_file_handler, &format!("{{\"FileName\":\"{}\",\"Flags\":0}}", jstr(&f))), HR_FILE_NOT_FOUND);
    let d = format!("{base}\\later.txt");
    std::fs::write(host(&d), b"x").unwrap();
    let _ = std::fs::remove_file(file::pending_moves_file());
    assert_eq!(result_of(delete_file_handler, &format!("{{\"FileName\":\"{}\",\"Flags\":2147483648}}", jstr(&d))), S_OK);
    assert!(host(&d).exists());
    assert!(std::fs::read_to_string(file::pending_moves_file()).unwrap().contains("later.txt\t\n"));
    assert_eq!(file::apply_pending_moves(), 1);
    assert!(!host(&d).exists());
}

#[test]
fn get_file_attributes_handler_reports_attributes() {
    let _g = guard();
    let base = fresh("ga");
    let r = call(get_file_attributes_handler, 0, &format!("{{\"FileName\":\"{}\"}}", jstr(&base)));
    assert_eq!(num_i32(&r, "Result"), S_OK);
    assert_eq!(num_u32(&r, "FileAttributes") & 0x10, 0x10);
    let r = call(get_file_attributes_handler, 0, &format!("{{\"FileName\":\"{}\\\\none\"}}", jstr(&base)));
    assert_eq!(num_i32(&r, "Result"), HR_FILE_NOT_FOUND);
}

#[test]
fn get_file_size_handler_reports_64bit_size() {
    let _g = guard();
    let base = fresh("gs");
    let f = format!("{base}\\s.bin");
    std::fs::write(host(&f), vec![7u8; 1234]).unwrap();
    let r = call(get_file_size_handler, 0, &format!("{{\"FileName\":\"{}\"}}", jstr(&f)));
    assert_eq!(num_i32(&r, "Result"), S_OK);
    assert_eq!(num_u64(&r, "FileSize"), 1234);
}

#[test]
fn modify_file_attributes_handler_adds_and_rejects_overlap() {
    let _g = guard();
    let base = fresh("ma");
    let f = format!("{base}\\m.txt");
    std::fs::write(host(&f), b"x").unwrap();
    assert_eq!(result_of(modify_file_attributes_handler, &format!("{{\"FileName\":\"{}\",\"AddFileAttributes\":3,\"RemoveFileAttributes\":0}}", jstr(&f))), S_OK);
    assert_eq!(file::get_file_attributes(&f).unwrap() & 3, 3);
    assert_eq!(result_of(modify_file_attributes_handler, &format!("{{\"FileName\":\"{}\",\"AddFileAttributes\":1,\"RemoveFileAttributes\":1}}", jstr(&f))), E_INVALIDARG);
    assert_eq!(result_of(modify_file_attributes_handler, &format!("{{\"FileName\":\"{}\",\"AddFileAttributes\":0,\"RemoveFileAttributes\":3}}", jstr(&f))), S_OK);
    assert_eq!(file::get_file_attributes(&f).unwrap() & 3, 0);
}

#[test]
fn remove_directory_handler_removes_empty_dir() {
    let _g = guard();
    let base = fresh("rd");
    let d = format!("{base}\\sub");
    std::fs::create_dir(host(&d)).unwrap();
    std::fs::write(host(&format!("{d}\\f")), b"x").unwrap();
    assert_eq!(result_of(remove_directory_handler, &format!("{{\"FileName\":\"{}\",\"Flags\":0}}", jstr(&d))), hr_from_win32(145));
    std::fs::remove_file(host(&format!("{d}\\f"))).unwrap();
    assert_eq!(result_of(remove_directory_handler, &format!("{{\"FileName\":\"{}\",\"Flags\":0}}", jstr(&d))), S_OK);
    assert!(!host(&d).exists());
}

#[test]
fn set_file_attributes_handler_sets_exact_value() {
    let _g = guard();
    let base = fresh("sa");
    let f = format!("{base}\\s.txt");
    std::fs::write(host(&f), b"x").unwrap();
    assert_eq!(result_of(set_file_attributes_handler, &format!("{{\"FileName\":\"{}\",\"FileAttributes\":2}}", jstr(&f))), S_OK);
    assert_eq!(file::get_file_attributes(&f).unwrap(), 2);
    assert_eq!(result_of(set_file_attributes_handler, &format!("{{\"FileName\":\"{}\",\"FileAttributes\":128}}", jstr(&f))), S_OK);
}

#[test]
fn get_file_info_handler_writes_console_json() {
    let _g = guard();
    let base = fresh("gi");
    let f = format!("{base}\\i.bin");
    std::fs::write(host(&f), vec![1u8; 77]).unwrap();
    let r = call(get_file_info_handler, 0, &format!("{{\"FileName\":\"{}\"}}", jstr(&f)));
    assert!(r.starts_with("{\"Result\":0,\"FileAttributes\":"), "{r}");
    assert_eq!(num_u64(&r, "FileSize"), 77);
    assert!(num_u64(&r, "LastWriteTime") > 0);
    let r = call(get_file_info_handler, 0, &format!("{{\"FileName\":\"{}\\\\none\"}}", jstr(&base)));
    assert_eq!(num_i32(&r, "Result"), HR_FILE_NOT_FOUND);
    let max = file_info_json(i32::MIN, &file::FileInfo { attributes: u32::MAX, creation: u64::MAX, access: u64::MAX, write: u64::MAX, size: u64::MAX });
    assert!(max.len() < 0xf4);
}

#[test]
fn application_operation_handlers_follow_console_mount_failure() {
    let _g = guard();
    assert_eq!(result_of(application_operations_start_handler, "{\"FileName\":\"{pkg}:\\\\a\"}"), HR_FILE_NOT_FOUND);
    assert_eq!(result_of(application_operations_end_handler, "{\"FileName\":\"{pkg}:\\\\a\"}"), HR_FILE_NOT_FOUND);
    assert_eq!(result_of(application_operations_start_handler, "{\"FileName\":\"D:\\\\a\"}"), E_INVALIDARG);
    assert_eq!(result_of(application_operations_end_handler, "{\"FileName\":\"\"}"), E_INVALIDARG);
    let mut s = MockSession::new();
    let r = call(package_mount_hold_handler, s.id(), "{\"FileName\":\"{pkg}:\\\\a\"}");
    assert_eq!(num_i32(&r, "Result"), S_OK);
    assert_eq!(s.sent.len(), 1);
    assert_eq!(num_i32(&s.sent[0].json, "Result"), HR_FILE_NOT_FOUND);
    let mut s = MockSession::new();
    let r = call(package_mount_hold_handler, s.id(), "{\"FileName\":\"D:\\\\a\"}");
    assert_eq!(num_i32(&r, "Result"), E_INVALIDARG);
    assert!(s.sent.is_empty());
}

#[test]
fn storage_device_free_space_handler_reports_deserialize_result() {
    let _g = guard();
    let r = call(get_storage_device_free_space_handler, 0, "{\"StorageDevice\":\"dev\"}");
    assert_eq!(num_i32(&r, "Result"), S_OK);
    assert_eq!(num_u64(&r, "TotalNumberOfBytes"), 0);
    assert_eq!(num_u64(&r, "NumberOfFreeBytes"), 0);
    assert_eq!(num_i32(&call(get_storage_device_free_space_handler, 0, "{}"), "Result"), E_JSON_NOT_FOUND);
}

#[test]
fn create_path_rejects_traversal_and_prefixes_console_paths() {
    assert_eq!(create_path("d:\\a\\b", PATH_NORMAL).unwrap(), "D:\\a\\b");
    assert_eq!(create_path("D:\\a", PATH_CONSOLE).unwrap(), "XD:\\a");
    assert_eq!(create_path("D:\\a\\..\\b", PATH_NORMAL), Err(E_INVALIDARG));
    assert_eq!(create_path("D:\\a\\\\b", PATH_NORMAL), Err(E_INVALIDARG));
    assert_eq!(create_path("", PATH_NORMAL), Err(E_INVALIDARG));
    assert_eq!(create_path("D:\\a", PATH_APPLICATION), Err(E_INVALIDARG));
    assert_eq!(append_path("D:\\", Some("\\x")), "D:\\x");
    assert_eq!(append_path("D:\\a", None), "D:\\a");
}

#[test]
fn find_files_handler_streams_info_and_entries() {
    let _g = guard();
    let base = fresh("ff");
    std::fs::write(host(&format!("{base}\\a.txt")), b"aa").unwrap();
    std::fs::write(host(&format!("{base}\\b.bin")), b"b").unwrap();
    std::fs::create_dir(host(&format!("{base}\\sub"))).unwrap();
    std::fs::write(host(&format!("{base}\\sub\\c.txt")), b"ccc").unwrap();
    let mut s = MockSession::new();
    let req = format!("{{\"FileName\":\"X{}\\\\*.txt\",\"Flags\":0,\"IncludeFileAttributes\":0,\"ExcludeFileAttributes\":0,\"RecursionLevels\":1}}", jstr(&base));
    let r = call(find_files_handler, s.id(), &req);
    assert_eq!(num_i32(&r, "Result"), S_OK);
    assert_eq!(s.sent.len(), 3);
    assert_eq!(str_of(&s.sent[0].json, "RootDirectory").unwrap(), format!("X{base}"));
    assert_eq!(str_of(&s.sent[0].json, "SearchPattern").unwrap(), "*.txt");
    let mut names: Vec<String> = s.sent[1..].iter().map(|e| str_of(&e.json, "FileName").unwrap()).collect();
    names.sort();
    assert_eq!(names, vec![format!("X{base}\\a.txt"), format!("X{base}\\sub\\c.txt")]);
    let a = s.sent.iter().find(|e| str_of(&e.json, "FileName").unwrap_or_default().ends_with("a.txt")).unwrap();
    assert_eq!(num_u64(&a.json, "FileSize"), 2);
    let mut s = MockSession::new();
    let req = format!("{{\"FileName\":\"{}\",\"Flags\":0,\"IncludeFileAttributes\":0,\"ExcludeFileAttributes\":16,\"RecursionLevels\":0}}", jstr(&base));
    assert_eq!(num_i32(&call(find_files_handler, s.id(), &req), "Result"), S_OK);
    assert_eq!(str_of(&s.sent[0].json, "SearchPattern").unwrap(), "*");
    assert_eq!(s.sent.len(), 3);
    let mut s = MockSession::new();
    let req = format!("{{\"FileName\":\"{}\\\\none\\\\x\",\"Flags\":0,\"IncludeFileAttributes\":0,\"ExcludeFileAttributes\":0,\"RecursionLevels\":0}}", jstr(&base));
    assert_eq!(num_i32(&call(find_files_handler, s.id(), &req), "Result"), HR_FILE_NOT_FOUND);
    assert!(s.sent.is_empty());
    let req = "{\"FileName\":\"{pkg}:\\\\x\",\"Flags\":0,\"IncludeFileAttributes\":0,\"ExcludeFileAttributes\":0,\"RecursionLevels\":0}";
    assert_eq!(num_i32(&call(find_files_handler, s.id(), req), "Result"), HR_FILE_NOT_FOUND);
}

#[test]
fn optimal_chunks_match_console_table() {
    assert_eq!(optimal_chunks(0), (1, 0x1000));
    assert_eq!(optimal_chunks(10), (1, 0x1000));
    assert_eq!(optimal_chunks(5000), (1, 5000));
    assert_eq!(optimal_chunks(0xa0000 * 2 + 1), (3, 0xa0000));
    assert_eq!(optimal_chunks(0xa0000 * 10), (4, 0xa0000));
}

#[test]
fn temp_file_name_creates_unique_file() {
    let _g = guard();
    let base = fresh("tmp");
    let a = get_temp_file_name(&base, "XTF").unwrap();
    let b = get_temp_file_name(&base, "XTF").unwrap();
    assert_ne!(a, b);
    assert!(a.starts_with(&format!("{base}\\XTF")) && a.ends_with(".tmp"));
    assert!(host(&a).is_file());
    assert_eq!(get_temp_file_name(&base, "XT"), Err(E_INVALIDARG));
}

fn send_request(dst: &str, size: u64) -> String {
    format!(
        "{{\"SrcFileName\":\"host.bin\",\"DstFileName\":\"{}\",\"Flags\":0,\"FileAttributes\":32,\"CreationTime\":{{\"L\":1000000,\"H\":30000000}},\"LastAccessTime\":{{\"L\":2000000,\"H\":30000000}},\"LastWriteTime\":{{\"L\":3000000,\"H\":30000000}},\"FileSize\":{{\"L\":{},\"H\":0}},\"FileOffset\":{{\"L\":0,\"H\":0}}}}",
        jstr(dst),
        size
    )
}

#[test]
fn send_file_handler_writes_chunks_and_metadata() {
    let _g = guard();
    let base = fresh("send");
    let dst = format!("{base}\\deep\\out.bin");
    let mut s = MockSession::new();
    unsafe {
        s.incoming.push_back(message("{\"FileOffset\":{\"L\":6,\"H\":0}}", b"6789"));
        s.pending_first = 2;
        let req = message(&send_request(&dst, 10), b"012345");
        let mut resp = MockResponse::new();
        assert_eq!(send_file_handler(s.id(), req.0, resp.ptr()), S_OK);
        assert_eq!(num_i32(resp.json.as_ref().unwrap(), "Result"), S_OK);
    }
    assert_eq!(std::fs::read(host(&dst)).unwrap(), b"0123456789");
    assert_eq!(file::get_file_attributes(&dst).unwrap(), 32);
    let info = file::get_file_info(&dst).unwrap();
    assert_eq!(info.write, (30000000u64 << 32) | 3000000);
    assert_eq!(info.creation, (30000000u64 << 32) | 1000000);
}

#[test]
fn send_file_handler_rejects_bad_offset_and_cancel() {
    let _g = guard();
    let base = fresh("send2");
    let dst = format!("{base}\\o.bin");
    let mut s = MockSession::new();
    unsafe {
        s.incoming.push_back(message("{\"FileOffset\":{\"L\":9,\"H\":0}}", b"zz"));
        let req = message(&send_request(&dst, 10), b"0123");
        let mut resp = MockResponse::new();
        send_file_handler(s.id(), req.0, resp.ptr());
        assert_eq!(num_i32(resp.json.as_ref().unwrap(), "Result"), E_UNEXPECTED);
        assert_eq!(std::fs::metadata(host(&dst)).unwrap().len(), 0);
        let mut s = MockSession::new();
        s.incoming.push_back(message("{\"CancelTransfer\":1,\"FileOffset\":{\"L\":4,\"H\":0}}", b""));
        let req = message(&send_request(&dst, 10), b"0123");
        let mut resp = MockResponse::new();
        send_file_handler(s.id(), req.0, resp.ptr());
        assert_eq!(num_i32(resp.json.as_ref().unwrap(), "Result"), HR_CANCELLED);
        assert!(!host(&dst).exists());
        let req = message(&send_request(&dst, 2), b"0123");
        let mut resp = MockResponse::new();
        send_file_handler(s.id(), req.0, resp.ptr());
        assert_eq!(num_i32(resp.json.as_ref().unwrap(), "Result"), E_UNEXPECTED);
        let req = message("{\"SrcFileName\":\"a\"}", b"");
        let mut resp = MockResponse::new();
        send_file_handler(s.id(), req.0, resp.ptr());
        assert_eq!(num_i32(resp.json.as_ref().unwrap(), "Result"), E_JSON_NOT_FOUND);
        let req = message(&send_request("{pkg}:\\x", 0), b"");
        let mut resp = MockResponse::new();
        send_file_handler(s.id(), req.0, resp.ptr());
        assert_eq!(num_i32(resp.json.as_ref().unwrap(), "Result"), HR_FILE_NOT_FOUND);
    }
}

#[test]
fn send_file_handler_writes_empty_file() {
    let _g = guard();
    let base = fresh("send3");
    let dst = format!("{base}\\e.bin");
    let mut s = MockSession::new();
    unsafe {
        let req = message(&send_request(&dst, 0), b"");
        let mut resp = MockResponse::new();
        send_file_handler(s.id(), req.0, resp.ptr());
        assert_eq!(num_i32(resp.json.as_ref().unwrap(), "Result"), S_OK);
    }
    assert_eq!(std::fs::read(host(&dst)).unwrap().len(), 0);
}

#[test]
fn recv_file_handler_streams_file_in_console_chunks() {
    let _g = guard();
    let base = fresh("recv");
    let src = format!("{base}\\big.bin");
    let data: Vec<u8> = (0..(0xa0000 + 10)).map(|i| (i % 251) as u8).collect();
    std::fs::write(host(&src), &data).unwrap();
    let mut s = MockSession::new();
    let r = call(recv_file_handler, s.id(), &format!("{{\"SrcFileName\":\"{}\",\"DstFileName\":\"pc.bin\",\"Flags\":0}}", jstr(&src)));
    assert_eq!(num_i32(&r, "Result"), S_OK);
    assert_eq!(s.sent.len(), 2);
    assert!(s.sent.iter().all(|m| m.is_async));
    let first = &s.sent[0].json;
    assert_eq!(str_of(first, "SrcFileName").unwrap(), src);
    assert_eq!(str_of(first, "DstFileName").unwrap(), "pc.bin");
    assert_eq!(num_u64(first, "FileSize"), data.len() as u64);
    assert_eq!(num_u64(first, "FileOffset"), 0);
    assert_eq!(num_u32(first, "FileAttributes") & 0x10, 0);
    assert_eq!(s.sent[0].binary.len(), 0xa0000);
    assert!(str_of(&s.sent[1].json, "SrcFileName").is_none());
    assert_eq!(num_u64(&s.sent[1].json, "FileOffset"), 0xa0000);
    let mut all = s.sent[0].binary.clone();
    all.extend_from_slice(&s.sent[1].binary);
    assert_eq!(all, data);
    let mut s = MockSession::new();
    let r = call(recv_file_handler, s.id(), &format!("{{\"SrcFileName\":\"{}\\\\none\",\"DstFileName\":\"x\",\"Flags\":0}}", jstr(&base)));
    assert_eq!(num_i32(&r, "Result"), HR_FILE_NOT_FOUND);
    assert!(s.sent.is_empty());
    let e = format!("{base}\\empty.bin");
    std::fs::write(host(&e), b"").unwrap();
    let r = call(recv_file_handler, s.id(), &format!("{{\"SrcFileName\":\"{}\",\"DstFileName\":\"x\",\"Flags\":0}}", jstr(&e)));
    assert_eq!(num_i32(&r, "Result"), S_OK);
    assert_eq!(s.sent.len(), 1);
    assert!(s.sent[0].binary.is_empty());
}

#[test]
fn export_table_matches_console_dll() {
    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/exports.txt")).unwrap();
    xtfcommon::exports::check_against_titles("xtffileio.dll", &text);
    let ex = xtfcommon::exports::parse(&text);
    let e4 = ex.iter().find(|e| e.ordinal == 4).unwrap();
    assert!(xtfcommon::exports::source_defines(concat!(env!("CARGO_MANIFEST_DIR"), "/src"), e4));
    let e5 = ex.iter().find(|e| e.ordinal == 5).unwrap();
    assert_eq!((e5.name.as_str(), e5.status.as_str()), ("XtfCreateFileIOClient", "missing"));
    assert!(!xtfcommon::exports::source_defines(concat!(env!("CARGO_MANIFEST_DIR"), "/src"), e5));
}
