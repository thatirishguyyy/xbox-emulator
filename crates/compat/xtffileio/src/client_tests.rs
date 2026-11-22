use crate::client::*;
use crate::com::*;
use crate::tests::{fresh, guard, host};
use core::ffi::c_void;
use core::ptr::null_mut;
use std::sync::Mutex;

pub(crate) const IID_XBTP_DISPATCHER: GUID = GUID::new(0xe6358d7e, 0x3b86, 0x425b, [0xb3, 0xd7, 0x3d, 0x21, 0x82, 0xb9, 0x76, 0x6a]);

#[repr(C)]
struct DispatcherVtbl {
    query_interface: usize,
    add_ref: usize,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    shutdown: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    add_extension: unsafe extern "system" fn(*mut c_void, *const GUID, *mut c_void) -> HRESULT,
}

type StartDispatcherFn = unsafe extern "C" fn(u32, i32, *const GUID, *mut *mut c_void) -> HRESULT;

pub(crate) struct Dispatcher(*mut c_void);

impl Dispatcher {
    pub(crate) unsafe fn start(port: u32) -> Dispatcher {
        let f: StartDispatcherFn = core::mem::transmute(xbtp_export(b"XbtpStartDispatcherEx\0"));
        let mut out = null_mut();
        let hr = f(port, 0, &IID_XBTP_DISPATCHER, &mut out);
        assert_eq!(hr, S_OK, "XbtpStartDispatcherEx({port}) failed: 0x{:08X}", hr as u32);
        Dispatcher(out)
    }

    unsafe fn vt(&self) -> &DispatcherVtbl {
        &**(self.0 as *const *const DispatcherVtbl)
    }

    pub(crate) unsafe fn with_metadata(port: u32, md: *mut c_void) -> Dispatcher {
        let d = Dispatcher::start(port);
        assert_eq!((d.vt().add_extension)(d.0, &FILEIO_EXTENSION_ID, md), S_OK);
        release(md);
        d
    }

    pub(crate) unsafe fn fileio(port: u32) -> Dispatcher {
        let mut md = null_mut();
        assert_eq!(crate::XtfOrdinal_4(&FILEIO_EXTENSION_ID, &IID_XBTP_EXTENSION_METADATA_IMPL, &mut md), S_OK);
        Dispatcher::with_metadata(port, md)
    }

    pub(crate) unsafe fn custom(port: u32, ops: &[(u32, OperationFn)]) -> Dispatcher {
        let mut md = null_mut();
        assert_eq!(XbtpCreateExtensionMetadata(&IID_XBTP_EXTENSION_METADATA_IMPL, &mut md), S_OK);
        for &(id, f) in ops {
            assert_eq!((vt::<MetadataVtbl>(md).add_function)(md, id, Some(f)), S_OK);
        }
        Dispatcher::with_metadata(port, md)
    }
}

impl Drop for Dispatcher {
    fn drop(&mut self) {
        unsafe {
            (self.vt().shutdown)(self.0);
            (self.vt().release)(self.0);
        }
    }
}

pub(crate) struct Client(pub *mut c_void);

impl Client {
    pub(crate) unsafe fn vt(&self) -> &ClientVtbl {
        &**(self.0 as *const *const ClientVtbl)
    }
    pub(crate) fn inner(&self) -> &FileIOClient {
        unsafe { client_from_main(self.0) }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        unsafe { release(self.0) };
    }
}

pub(crate) fn open_client(port: u32, address: &str) -> Result<Client, HRESULT> {
    TEST_PORT.with(|p| p.set(port));
    let w = wide(address);
    let mut out = null_mut();
    let hr = unsafe { create_client(w.as_ptr(), &IID_XTF_FILEIO_CLIENT, &mut out) };
    TEST_PORT.with(|p| p.set(0));
    if hr < 0 {
        assert!(out.is_null());
        Err(hr)
    } else {
        Ok(Client(out))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Seen {
    pub root: String,
    pub pattern: String,
    pub name: String,
    pub attributes: u32,
    pub size: u64,
}

#[repr(C)]
pub(crate) struct Callback {
    vtbl: *const FindCallbackVtbl,
    pub seen: Mutex<Vec<Seen>>,
    pub fail_at: Option<usize>,
}

unsafe extern "system" fn cb_qi(_: *mut c_void, _: *const GUID, _: *mut *mut c_void) -> HRESULT {
    E_NOINTERFACE
}

unsafe extern "system" fn cb_ref(_: *mut c_void) -> u32 {
    1
}

pub(crate) const CALLBACK_ABORT: HRESULT = 0x8000_4004u32 as i32;

unsafe extern "system" fn cb_on_file(this: *mut c_void, root: *const u16, pattern: *const u16, info: *const XtfFileInfo) -> HRESULT {
    let me = &*(this as *const Callback);
    let mut seen = me.seen.lock().unwrap_or_else(|e| e.into_inner());
    if me.fail_at == Some(seen.len()) {
        return CALLBACK_ABORT;
    }
    let i = &*info;
    seen.push(Seen { root: from_wide(root), pattern: from_wide(pattern), name: from_wide(i.file_name), attributes: i.attributes, size: i.size });
    S_OK
}

static CALLBACK_VTBL: FindCallbackVtbl = FindCallbackVtbl { base: UnknownVtbl { query_interface: cb_qi, add_ref: cb_ref, release: cb_ref }, on_file: cb_on_file };

impl Callback {
    pub(crate) fn new(fail_at: Option<usize>) -> Box<Callback> {
        Box::new(Callback { vtbl: &CALLBACK_VTBL, seen: Mutex::new(Vec::new()), fail_at })
    }
    pub(crate) fn ptr(&self) -> *mut c_void {
        self as *const Callback as *mut c_void
    }
    pub(crate) fn names(&self) -> Vec<String> {
        self.seen.lock().unwrap_or_else(|e| e.into_inner()).iter().map(|s| s.name.clone()).collect()
    }
}

fn w(s: &str) -> Vec<u16> {
    wide(s)
}

#[test]
fn factory_rejects_addresses_like_console() {
    let mut out = 1usize as *mut c_void;
    unsafe {
        assert_eq!(create_client(core::ptr::null(), &IID_XTF_FILEIO_CLIENT, &mut out), E_NO_IPADDRESS_IN_CONNSTR);
        assert!(out.is_null());
        assert_eq!(create_client(w("").as_ptr(), &IID_XTF_FILEIO_CLIENT, &mut out), E_NO_IPADDRESS_IN_CONNSTR);
        assert_eq!(create_client(w("/title").as_ptr(), &IID_XTF_FILEIO_CLIENT, &mut out), E_NO_IPADDRESS_IN_CONNSTR);
        assert_eq!(create_client(w("127.0.0.1/bogus").as_ptr(), &IID_XTF_FILEIO_CLIENT, &mut out), E_INVALID_VM_TARGET);
        let long: String = "a".repeat(MAX_ADDRESS_CHARS);
        assert_eq!(create_client(w(&long).as_ptr(), &IID_XTF_FILEIO_CLIENT, &mut out), E_NO_IPADDRESS_IN_CONNSTR);
        assert_eq!(create_client(w("127.0.0.1").as_ptr(), &IID_XTF_FILEIO_CLIENT, null_mut()), E_POINTER);
    }
}

#[test]
fn split_console_address_maps_targets_and_keys() {
    unsafe {
        let mut ip = Bstr::null();
        let mut key = Bstr::null();
        let mut port = 9u32;
        assert_eq!(split_console_address(w("10.0.0.5/TITLE").as_ptr(), 0, &mut ip, None, &mut port), S_OK);
        assert_eq!((ip.to_string(), port), ("10.0.0.5".to_string(), XBTP_PORT_TITLE));
        assert_eq!(split_console_address(w("10.0.0.5\\host").as_ptr(), 0, &mut ip, None, &mut port), S_OK);
        assert_eq!(port, XBTP_PORT_SYSTEM);
        assert_eq!(split_console_address(w("10.0.0.5+abc/host").as_ptr(), 7, &mut ip, Some(&mut key), &mut port), S_OK);
        assert_eq!((ip.to_string(), key.to_string(), port), ("10.0.0.5".to_string(), "abc".to_string(), XBTP_PORT_HOST));
        assert_eq!(split_console_address(w("box").as_ptr(), 7, &mut ip, None, &mut port), S_OK);
        assert_eq!((ip.to_string(), port), ("box".to_string(), 7));
    }
}

#[test]
fn file_list_sorts_directories_first_then_names() {
    let mk = |n: &str, a: u32| FoundEntry { name: Bstr::from_str(n).unwrap(), info: XtfFileInfo { attributes: a, size: 5000, ..Default::default() } };
    let mut l = FileList::default();
    l.add(mk("b", 0));
    l.add(mk("A", 0));
    l.add(mk("z", 0x10));
    l.add(mk("a", 0));
    l.add(mk("C", 0x10));
    l.sort();
    let names: Vec<String> = l.entries.iter().map(|e| e.name.to_string()).collect();
    assert_eq!(names, vec!["C", "z", "A", "a", "b"]);
    assert_eq!((l.files, l.dirs, l.total_size, l.total_rounded), (3, 2, 15000, 3 * 8192));
    assert!(unsafe { is_application_path(w("{pkg}:\\x").as_ptr()) });
    assert!(!unsafe { is_application_path(w("D:\\{pkg}:\\x").as_ptr()) });
    assert!(!unsafe { is_application_path(w("{pkg}\\x").as_ptr()) });
}

#[test]
fn client_query_interface_and_refcount() {
    let _g = guard();
    let port = 47940;
    let _d = unsafe { Dispatcher::fileio(port) };
    let c = open_client(port, "127.0.0.1").unwrap();
    unsafe {
        let mut p = null_mut();
        assert_eq!((c.vt().query_interface)(c.0, &IID_IUNKNOWN, &mut p), S_OK);
        assert_eq!(p, c.0);
        release(p);
        assert_eq!((c.vt().query_interface)(c.0, &IID_XTF_FILEIO_CLIENT_BASE, &mut p), S_OK);
        assert_eq!(p, c.0);
        release(p);
        assert_eq!((c.vt().query_interface)(c.0, &IID_XTF_FILEIO_CLIENT2, &mut p), S_OK);
        assert_eq!(p as usize, c.0 as usize + 8);
        let mut q = null_mut();
        assert_eq!((vt::<UnknownVtbl>(p).query_interface)(p, &IID_XTF_FILEIO_CLIENT3, &mut q), S_OK);
        assert_eq!(q as usize, c.0 as usize + 0x10);
        release(q);
        release(p);
        let mut z = 7usize as *mut c_void;
        assert_eq!((c.vt().query_interface)(c.0, &IID_XBTP_DISPATCHER, &mut z), E_NOINTERFACE);
        assert_eq!(z as usize, 7);
        assert_eq!((c.vt().add_ref)(c.0), 2);
        assert_eq!((c.vt().release)(c.0), 1);
    }
    assert_eq!(c.inner().port(), XBTP_PORT_SYSTEM);
}

#[test]
fn client_single_file_operations_round_trip() {
    let _g = guard();
    let port = 47941;
    let _d = unsafe { Dispatcher::fileio(port) };
    let base = fresh("cli_ops");
    std::fs::write(host(&format!("{base}\\f.bin")), vec![7u8; 1234]).unwrap();
    let c = open_client(port, "127.0.0.1").unwrap();
    unsafe {
        let f = w(&format!("X{base}\\f.bin"));
        let mut size = 0u64;
        assert_eq!((c.vt().get_file_size)(c.0, f.as_ptr(), &mut size), S_OK);
        assert_eq!(size, 1234);
        let mut attrs = 0u32;
        assert_eq!((c.vt().get_file_attributes)(c.0, f.as_ptr(), &mut attrs), S_OK);
        assert_eq!(attrs & 0x10, 0);
        let mut info = XtfFileInfo::default();
        assert_eq!((c.vt().get_file_info)(c.0, f.as_ptr(), &mut info), S_OK);
        assert_eq!(info.size, 1234);
        assert!(info.file_name.is_null());
        assert!(info.write != 0);
        assert_eq!((c.vt().set_file_attributes)(c.0, f.as_ptr(), 0, 0, 0, 1, 0, null_mut()), S_OK);
        assert_eq!((c.vt().get_file_attributes)(c.0, f.as_ptr(), &mut attrs), S_OK);
        assert_eq!(attrs & 1, 1);
        assert_eq!((c.vt().set_file_attributes)(c.0, f.as_ptr(), 0, 0, 0, 0, 1, null_mut()), S_OK);
        assert_eq!((c.vt().get_file_attributes)(c.0, f.as_ptr(), &mut attrs), S_OK);
        assert_eq!(attrs & 1, 0);
        let missing = w(&format!("X{base}\\nope.bin"));
        size = 99;
        assert_eq!((c.vt().get_file_size)(c.0, missing.as_ptr(), &mut size), HR_FILE_NOT_FOUND);
        assert_eq!(size, 99);
        assert_eq!((c.vt().get_file_attributes)(c.0, missing.as_ptr(), &mut attrs), HR_FILE_NOT_FOUND);
        let mut info2 = XtfFileInfo { size: 5, ..Default::default() };
        assert_eq!((c.vt().get_file_info)(c.0, missing.as_ptr(), &mut info2), HR_FILE_NOT_FOUND);
        assert_eq!(info2.size, 5);
        let d = w(&format!("X{base}\\newdir\\deep"));
        assert_eq!((c.vt().create_directory)(c.0, d.as_ptr()), S_OK);
        assert!(host(&format!("{base}\\newdir\\deep")).is_dir());
        assert_eq!((c.vt().remove_directory)(c.0, d.as_ptr(), 0, null_mut()), S_OK);
        assert!(!host(&format!("{base}\\newdir\\deep")).exists());
        assert_eq!((c.vt().remove_directory)(c.0, d.as_ptr(), 0, null_mut()), HR_FILE_NOT_FOUND);
        assert_eq!((c.vt().start_application_operations)(c.0, w("{pkg}:\\a").as_ptr()), S_OK);
        assert_eq!((c.vt().end_application_operations)(c.0, w("{pkg}:\\a").as_ptr()), S_OK);
        assert_eq!((c.vt().create_directory)(c.0, w("{pkg}:\\a").as_ptr()), HR_FILE_NOT_FOUND);
        let mut i3 = null_mut();
        assert_eq!((c.vt().query_interface)(c.0, &IID_XTF_FILEIO_CLIENT3, &mut i3), S_OK);
        let v3 = &**(i3 as *const *const Client3Vtbl);
        let (mut total, mut free) = (5u64, 6u64);
        assert_eq!((v3.get_storage_device_free_space)(i3, w("D:").as_ptr(), &mut total, &mut free), S_OK);
        assert_eq!((total, free), (0, 0));
        assert_eq!((v3.get_storage_device_free_space)(i3, core::ptr::null(), &mut total, &mut free), E_INVALIDARG);
        assert_eq!((v3.local_application_operations_start)(i3, core::ptr::null()), S_OK);
        assert_eq!((v3.local_application_operations_end)(i3, core::ptr::null()), S_OK);
        release(i3);
        assert_eq!((c.vt().open_session)(c.0), S_OK);
        assert_eq!((c.vt().get_file_size)(c.0, f.as_ptr(), &mut size), S_OK);
    }
}

#[test]
fn client_find_delete_and_recursive_remove() {
    let _g = guard();
    let port = 47942;
    let _d = unsafe { Dispatcher::fileio(port) };
    let base = fresh("cli_find");
    std::fs::write(host(&format!("{base}\\a.txt")), b"aa").unwrap();
    std::fs::write(host(&format!("{base}\\B.txt")), b"b").unwrap();
    std::fs::write(host(&format!("{base}\\c.bin")), b"c").unwrap();
    std::fs::create_dir_all(host(&format!("{base}\\sub\\inner"))).unwrap();
    std::fs::write(host(&format!("{base}\\sub\\d.txt")), b"dddd").unwrap();
    std::fs::write(host(&format!("{base}\\sub\\inner\\e.txt")), b"e").unwrap();
    let c = open_client(port, "127.0.0.1").unwrap();
    unsafe {
        let cb = Callback::new(None);
        let pat = w(&format!("X{base}\\*.txt"));
        assert_eq!((c.vt().find_files)(c.0, pat.as_ptr(), 0, 0, 1, 0, cb.ptr()), S_OK);
        let seen = cb.seen.lock().unwrap().clone();
        let mut names: Vec<String> = seen.iter().map(|s| s.name.clone()).collect();
        names.sort();
        assert_eq!(names, vec![format!("X{base}\\B.txt"), format!("X{base}\\a.txt"), format!("X{base}\\sub\\d.txt")]);
        assert!(seen.iter().all(|s| s.root == format!("X{base}") && s.pattern == "*.txt"));
        assert_eq!(seen.iter().find(|s| s.name.ends_with("d.txt")).unwrap().size, 4);
        let none = Callback::new(None);
        assert_eq!((c.vt().find_files)(c.0, w(&format!("X{base}\\zz\\*")).as_ptr(), 0, 0, 0, 0, none.ptr()), HR_FILE_NOT_FOUND);
        let del = Callback::new(None);
        assert_eq!((c.vt().delete_files)(c.0, pat.as_ptr(), 0, 0, 0, 0, del.ptr()), S_OK);
        assert_eq!(del.names(), vec![format!("X{base}\\a.txt"), format!("X{base}\\B.txt")]);
        assert!(!host(&format!("{base}\\a.txt")).exists());
        assert!(host(&format!("{base}\\c.bin")).exists());
        assert!(host(&format!("{base}\\sub\\d.txt")).exists());
        assert_eq!((c.vt().delete_files)(c.0, w(&format!("X{base}\\zz\\*")).as_ptr(), 0, 0, 0, 0, null_mut()), HR_FILE_NOT_FOUND);
        let sa = Callback::new(None);
        assert_eq!((c.vt().set_file_attributes)(c.0, w(&format!("X{base}\\sub\\*")).as_ptr(), 0, 0x10, 0, 1, 0, sa.ptr()), S_OK);
        assert_eq!(sa.names(), vec![format!("X{base}\\sub\\d.txt")]);
        let mut attrs = 0u32;
        assert_eq!((c.vt().get_file_attributes)(c.0, w(&format!("X{base}\\sub\\d.txt")).as_ptr(), &mut attrs), S_OK);
        assert_eq!(attrs & 1, 1);
        let sub = w(&format!("X{base}\\sub"));
        assert_ne!((c.vt().remove_directory)(c.0, sub.as_ptr(), 0, null_mut()), S_OK);
        let rd = Callback::new(None);
        assert_eq!((c.vt().remove_directory)(c.0, sub.as_ptr(), 1, rd.ptr()), S_OK);
        let mut rn = rd.names();
        rn.sort();
        assert_eq!(rn, vec![format!("X{base}\\sub\\d.txt"), format!("X{base}\\sub\\inner\\e.txt")]);
        assert!(!host(&format!("{base}\\sub")).exists());
        assert!(host(&format!("{base}\\c.bin")).exists());
    }
    let c2 = open_client(port, "127.0.0.1").unwrap();
    unsafe {
        let abort = Callback::new(Some(0));
        assert_eq!((c2.vt().find_files)(c2.0, w(&format!("X{base}\\*")).as_ptr(), 0, 0, 0, 0, abort.ptr()), CALLBACK_ABORT);
        assert!(abort.names().is_empty());
    }
}

#[test]
fn client_open_fails_without_dispatcher() {
    let port = 47943;
    let start = std::time::Instant::now();
    assert_eq!(open_client(port, "127.0.0.1").err(), Some(E_COULD_NOT_CONNECT));
    assert!(start.elapsed() < std::time::Duration::from_secs(120));
}

unsafe extern "system" fn supported_without_level(_session: usize, _request: *mut c_void, response: *mut c_void) -> HRESULT {
    let w = wide("{\"Result\":0}");
    (vt::<MessageVtbl>(response).set_json_body_w)(response, w.as_ptr())
}

unsafe extern "system" fn supported_failing(_session: usize, _request: *mut c_void, response: *mut c_void) -> HRESULT {
    let w = wide("{\"Result\":-2147467259,\"ProtocolLevelSupported\":2}");
    (vt::<MessageVtbl>(response).set_json_body_w)(response, w.as_ptr())
}

#[test]
fn client_direct_protocol_level_follows_server() {
    let _g = guard();
    let full = crate::operation_table();
    let without: Vec<(u32, OperationFn)> = full.iter().copied().filter(|(id, _)| *id != OP_TRANSFER_FILES_SUPPORTED).collect();
    let mut no_level = without.clone();
    no_level.push((OP_TRANSFER_FILES_SUPPORTED, supported_without_level as OperationFn));
    let mut failing = without.clone();
    failing.push((OP_TRANSFER_FILES_SUPPORTED, supported_failing as OperationFn));
    let expect_full = if full.iter().any(|(id, _)| *id == OP_TRANSFER_FILES_SUPPORTED) { 2 } else { 0 };
    for (port, ops, want) in [(47944u32, full.clone(), expect_full), (47945, without, 0), (47946, no_level, 1), (47947, failing, 0)] {
        let _d = unsafe { Dispatcher::custom(port, &ops) };
        let c = open_client(port, "127.0.0.1").unwrap();
        unsafe {
            assert_eq!(c.inner().direct_protocol_level(), want, "port {port}");
            assert_eq!(c.inner().direct_protocol_level(), want, "cached {port}");
        }
    }
}
