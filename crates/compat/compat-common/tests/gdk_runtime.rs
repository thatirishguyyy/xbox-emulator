use compat_common::gdk::{self, package, save, Guid};
use core::ffi::c_void;
use std::path::PathBuf;

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("tombox_gdk_test_{}_{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

unsafe fn method(clsid: Guid, iid: Guid, slot: usize) -> (*mut c_void, usize) {
    let mut obj: *mut c_void = core::ptr::null_mut();
    let hr = gdk::query_api_impl(&clsid, &iid, &mut obj);
    assert_eq!(hr, 0, "QueryApiImpl {} {}", clsid.text(), iid.text());
    let vt = *(obj as *const *const usize);
    (obj, *vt.add(slot))
}

#[test]
fn guid_parse_roundtrip() {
    let s = "01acd177-91f9-4763-a38e-ccbb55ce32e0";
    let g = Guid::parse(s);
    assert_eq!(g.text(), s);
    assert_eq!(g.d1, 0x01acd177);
    assert_eq!(g.d4, [0xa3, 0x8e, 0xcc, 0xbb, 0x55, 0xce, 0x32, 0xe0]);
}

#[test]
fn save_name_encoding_roundtrip() {
    for n in ["Profile.sav", "a/b\\c:d", "CON", "trailing.", "space ", "unicode-\u{e9}", "~tilde"] {
        let e = save::encode_name(n);
        assert!(e.chars().all(|c| c.is_ascii_alphanumeric() || "_-.~".contains(c)), "{e}");
        assert!(!e.ends_with('.') && !e.ends_with(' '));
        assert_eq!(save::decode_name(&e).as_deref(), Some(n));
    }
}

#[test]
fn save_store_commit_read_enumerate_delete() {
    let root = tmp("store");
    let s = save::Store::open(root.clone()).unwrap();
    assert!(s.containers().is_empty());
    s.commit("slot/1", Some("Slot One"), &[("data".into(), b"hello".to_vec()), ("meta:x".into(), vec![1, 2, 3])], &[]).unwrap();
    let info = s.container_info("slot/1").unwrap();
    assert_eq!(info.display, "Slot One");
    assert_eq!(info.blob_count, 2);
    assert_eq!(info.total_size, 8);
    assert_eq!(s.read_blob("slot/1", "data").unwrap(), b"hello");
    s.commit("slot/1", None, &[("data".into(), b"bye".to_vec())], &["meta:x".to_string()]).unwrap();
    assert_eq!(s.blobs("slot/1"), vec![("data".to_string(), 3)]);
    assert_eq!(s.containers().len(), 1);
    assert!(s.delete_container("slot/1").unwrap());
    assert!(s.containers().is_empty());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn layout_and_title_parsing() {
    let xml = r#"<Package><Chunk Id="Registration"></Chunk><Chunk Id="2" Marker="Launch"><FileGroup/></Chunk><Chunk Id="4" Languages="de-DE"></Chunk><Chunk Id="3" OnDemand="true" Tags="StW"></Chunk></Package>"#;
    let l = package::parse_layout(xml);
    assert_eq!(l.chunks, vec![2, 4, 3]);
    assert_eq!(l.languages, vec!["de-DE"]);
    assert_eq!(l.tags, vec!["StW"]);
    let d = tmp("title");
    std::fs::write(d.join("MicrosoftGame.config"), "\u{feff}<Game><Identity Name=\"Pub.Game\" Publisher=\"CN=X\" Version=\"1.2.3.4\" /><TitleId>0FF4B5BD</TitleId><ShellVisuals DefaultDisplayName=\"My Game\" /></Game>").unwrap();
    let sub = d.join("Bin").join("XSX");
    std::fs::create_dir_all(&sub).unwrap();
    let t = gdk::Title::from_dir(&sub, "exe");
    assert_eq!(t.title_id, Some(0x0FF4B5BD));
    assert_eq!(t.key, "0FF4B5BD");
    assert_eq!(t.identity, "Pub.Game");
    assert_eq!(t.display, "My Game");
    assert_eq!(t.version, [1, 2, 3, 4]);
    let none = gdk::Title::from_dir(&tmp("none"), "Fallback Exe");
    assert_eq!(none.key, "Fallback_Exe");
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn real_dump_layouts_parse_when_present() {
    for (p, n) in [("input/extracted/Mount", 8), ("input/xdefiant/Mount", 10)] {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..").join(p);
        if !dir.is_dir() {
            continue;
        }
        let l = package::load_layout(&dir);
        assert!(l.chunks.len() >= n, "{p}: {:?}", l.chunks);
    }
}

#[test]
#[cfg_attr(not(tombox_analysis_gdk), ignore = "needs analysis/gdk call-site maps from local title dumps")]
fn every_title_call_site_is_served_or_forwarded() {
    let imp = gdk::implemented();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../analysis/gdk");
    let mut checked = 0;
    let mut missing = std::collections::BTreeMap::<String, Vec<String>>::new();
    let mut files: Vec<_> = std::fs::read_dir(&root).unwrap().flatten().map(|e| e.path()).filter(|p| p.extension().map(|x| x == "json").unwrap_or(false)).collect();
    files.sort();
    for f in &files {
        let text = std::fs::read_to_string(f).unwrap();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        for s in v.as_array().unwrap() {
            let c = Guid::parse(s["clsid"].as_str().unwrap());
            let i = Guid::parse(s["iid"].as_str().unwrap());
            let slot = s["slot"].as_u64().unwrap() as usize;
            let forwarded = gdk::FORWARDED.iter().any(|(_, g)| *g == c);
            let served = imp.iter().any(|(ic, ii, _, slots)| *ic == c && *ii == i && slots.contains(&slot));
            assert!(!(forwarded && served), "{} {} slot {slot} both forwarded and served", c.text(), i.text());
            if !forwarded && !served {
                let key = format!("{} {} slot {slot:2} args {}", c.text(), i.text(), s["args"]);
                missing.entry(key).or_default().push(format!("{}@{}", f.file_stem().unwrap().to_string_lossy(), s["site"].as_str().unwrap_or("")));
            }
            checked += 1;
        }
    }
    for (k, v) in &missing {
        eprintln!("UNCOVERED {k} <- {}", v.join(" "));
    }
    assert!(checked >= 128 + 102);
    assert!(missing.is_empty(), "{} uncovered (clsid, iid, slot) entries", missing.len());
}

#[test]
fn sync_calls_through_query_api_impl() {
    unsafe {
        let (o, f) = method(gdk::user::CLSID, gdk::user::IID_V1, 6);
        let f: unsafe extern "system" fn(*mut c_void, *mut u32) -> i32 = core::mem::transmute(f);
        let mut n = 0u32;
        assert_eq!(f(o, &mut n), 0);
        assert_eq!(n, 1);

        let (o, f) = method(gdk::user::CLSID, gdk::user::IID_V2, 11);
        let f: unsafe extern "system" fn(*mut c_void, usize, *mut u64) -> i32 = core::mem::transmute(f);
        let mut x = 0u64;
        assert_eq!(f(o, gdk::user::handle(), &mut x), 0);
        assert_eq!(x, gdk::user::user().xuid);

        let (o, f) = method(gdk::user::CLSID, gdk::user::IID_GAMERTAG, 3);
        let f: unsafe extern "system" fn(*mut c_void, usize, u32, usize, *mut u8, *mut usize) -> i32 = core::mem::transmute(f);
        let mut buf = [0u8; 64];
        let mut used = 0usize;
        assert_eq!(f(o, gdk::user::handle(), 0, buf.len(), buf.as_mut_ptr(), &mut used), 0);
        assert_eq!(&buf[..used - 1], gdk::user::user().gamertag.as_bytes());

        let (o, f) = method(gdk::net::CLSID, gdk::net::CLSID, 15);
        let f: unsafe extern "system" fn(*mut c_void, *mut u8) -> i32 = core::mem::transmute(f);
        let mut hint = [0xffu8; 16];
        assert_eq!(f(o, hint.as_mut_ptr()), 0);
        assert_eq!(u32::from_le_bytes(hint[0..4].try_into().unwrap()), gdk::net::connectivity_level());
        assert_eq!(hint[12], 1);

        let (o, f) = method(gdk::system::XSYSTEM, gdk::system::XSYSTEM_V2, 4);
        let f: unsafe extern "system" fn(*mut c_void, usize, *mut u8, *mut usize) -> i32 = core::mem::transmute(f);
        let mut sb = [0u8; 16];
        assert_eq!(f(o, sb.len(), sb.as_mut_ptr(), &mut used), 0);
        assert_eq!(&sb[..used - 1], b"RETAIL");

        let (o, f) = method(gdk::system::ANALYTICS, gdk::system::ANALYTICS, 3);
        let f: unsafe extern "system" fn(*mut c_void, *mut u8) -> *mut u8 = core::mem::transmute(f);
        let mut info = [0u8; 144];
        assert_eq!(f(o, info.as_mut_ptr()), info.as_mut_ptr());
        assert_eq!(&info[16..29], b"Windows.Xbox\0");
        assert!(info[80..].starts_with(b"Xbox Series"));

        let (o, f) = method(package::CLSID, package::IID_V1, 4);
        let f: unsafe extern "system" fn(*mut c_void) -> bool = core::mem::transmute(f);
        assert!(f(o));

        let (o, f) = method(gdk::user::CLSID, gdk::user::IID_V1, 15);
        let f: unsafe extern "system" fn(*mut c_void) -> i32 = core::mem::transmute(f);
        assert_eq!(f(o), gdk::E_NOTIMPL);

        let mut obj: *mut c_void = 1 as *mut c_void;
        let unknown = Guid::parse("11111111-2222-3333-4444-555555555555");
        assert_eq!(gdk::query_api_impl(&unknown, &unknown, &mut obj), gdk::E_NOINTERFACE);
        assert!(obj.is_null());
    }
}

#[test]
fn xgamesave_sync_surface_against_temp_store() {
    unsafe {
        let root = tmp("ffi");
        let p = Box::into_raw(Box::new(save::Provider { store: save::Store::open(root.clone()).unwrap() }));
        let c = save::CLSID;
        let (o, create) = method(c, c, 16);
        let create: unsafe extern "system" fn(*mut c_void, *mut save::Provider, *const u8, *mut usize) -> i32 = core::mem::transmute(create);
        let mut cont = 0usize;
        assert_eq!(create(o, p, b"Profile\0".as_ptr(), &mut cont), 0);
        assert_eq!(create(o, p, b"\0".as_ptr(), &mut cont as *mut usize), save::E_GS_INVALID_CONTAINER_NAME);
        assert_eq!(create(o, p, b"Profile\0".as_ptr(), &mut cont), 0);

        let (_, upd) = method(c, c, 23);
        let upd: unsafe extern "system" fn(*mut c_void, usize, *const u8, *mut usize) -> i32 = core::mem::transmute(upd);
        let mut u = 0usize;
        assert_eq!(upd(o, cont, b"My Profile\0".as_ptr(), &mut u), 0);
        let (_, write) = method(c, c, 25);
        let write: unsafe extern "system" fn(*mut c_void, usize, *const u8, *const u8, usize) -> i32 = core::mem::transmute(write);
        assert_eq!(write(o, u, b"a\0".as_ptr(), b"12345".as_ptr(), 5), 0);
        assert_eq!(write(o, u, b"b\0".as_ptr(), b"xy".as_ptr(), 2), 0);
        let (_, submit) = method(c, c, 27);
        let submit: unsafe extern "system" fn(*mut c_void, usize) -> i32 = core::mem::transmute(submit);
        assert_eq!(submit(o, u), 0);

        unsafe extern "system" fn count_cb(info: *const u8, ctx: *mut c_void) -> bool {
            let n = &mut *(ctx as *mut Vec<(String, u32, u64)>);
            let name = std::ffi::CStr::from_ptr(*(info as *const *const i8)).to_string_lossy().into_owned();
            let blobs = core::ptr::read_unaligned(info.add(16) as *const u32);
            let total = core::ptr::read_unaligned(info.add(24) as *const u64);
            n.push((name, blobs, total));
            true
        }
        let (_, en) = method(c, c, 14);
        let en: unsafe extern "system" fn(*mut c_void, *mut save::Provider, *mut c_void, usize) -> i32 = core::mem::transmute(en);
        let mut seen: Vec<(String, u32, u64)> = Vec::new();
        assert_eq!(en(o, p, &mut seen as *mut _ as *mut c_void, count_cb as usize), 0);
        assert_eq!(seen, vec![("Profile".to_string(), 2, 7)]);

        let (_, read) = method(c, c, 20);
        let read: unsafe extern "system" fn(*mut c_void, usize, *const *const u8, *mut u32, usize, *mut u8) -> i32 = core::mem::transmute(read);
        let mut count = 0u32;
        let mut small = [0u8; 10];
        assert_eq!(read(o, cont, core::ptr::null(), &mut count, small.len(), small.as_mut_ptr()), save::E_GS_PROVIDED_BUFFER_TOO_SMALL);
        let mut buf = vec![0u8; 2 * 24 + 7];
        count = 0;
        assert_eq!(read(o, cont, core::ptr::null(), &mut count, buf.len(), buf.as_mut_ptr()), 0);
        assert_eq!(count, 2);
        let b = buf.as_ptr();
        let name0 = std::ffi::CStr::from_ptr(*(b as *const *const i8)).to_str().unwrap().to_string();
        let size0 = core::ptr::read_unaligned(b.add(8) as *const u32) as usize;
        let data0 = *(b.add(16) as *const *const u8);
        assert_eq!(name0, "a");
        assert_eq!(core::slice::from_raw_parts(data0, size0), b"12345");
        let names = [b"b\0".as_ptr()];
        count = 1;
        assert_eq!(read(o, cont, names.as_ptr(), &mut count, buf.len(), buf.as_mut_ptr()), 0);
        assert_eq!(count, 1);
        let missing = [b"zz\0".as_ptr()];
        count = 1;
        assert_eq!(read(o, cont, missing.as_ptr(), &mut count, buf.len(), buf.as_mut_ptr()), save::E_GS_BLOB_NOT_FOUND);

        let (_, del) = method(c, c, 10);
        let del: unsafe extern "system" fn(*mut c_void, *mut save::Provider, *const u8) -> i32 = core::mem::transmute(del);
        assert_eq!(del(o, p, b"Profile\0".as_ptr()), 0);
        seen.clear();
        assert_eq!(en(o, p, &mut seen as *mut _ as *mut c_void, count_cb as usize), 0);
        assert!(seen.is_empty());
        let _ = std::fs::remove_dir_all(root);
    }
}

#[test]
fn device_type_parsing_and_registry_answer() {
    assert_eq!(gdk::hook::console_mode("anaconda"), Some(0x64));
    assert_eq!(gdk::hook::console_mode("Lockhart"), Some(0x60));
    assert_eq!(gdk::hook::console_mode("pc"), None);
    assert_eq!(gdk::hook::console_mode(""), None);
    assert_eq!(gdk::hook::console_mode(gdk::hook::default_device(Some("scarlett"))), Some(0x64));
    assert_eq!(gdk::hook::console_mode(gdk::hook::default_device(Some("xboxone"))), Some(0x64));
    assert_eq!(gdk::hook::console_mode(gdk::hook::default_device(None)), None);
    unsafe {
        let name: Vec<u16> = "ConsoleMode".encode_utf16().chain(Some(0)).collect();
        let other: Vec<u16> = "Other".encode_utf16().chain(Some(0)).collect();
        let (mut kind, mut data, mut cb) = (0u32, 0u32, 4u32);
        assert_eq!(gdk::hook::query_console_mode(name.as_ptr(), &mut kind, &mut data as *mut u32 as *mut u8, &mut cb), 0);
        assert_eq!((kind, cb), (4, 4));
        assert_eq!(gdk::hook::query_console_mode(other.as_ptr(), &mut kind, &mut data as *mut u32 as *mut u8, &mut cb), 2);
    }
}

#[test]
fn gamer_picture_is_a_valid_png() {
    let p = gdk::user::png_solid(4, 3, [1, 2, 3]);
    assert_eq!(&p[..8], &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
    assert_eq!(&p[12..16], b"IHDR");
    assert_eq!(&p[p.len() - 8..p.len() - 4], b"IEND");
    assert_eq!(&p[p.len() - 4..], &[0xae, 0x42, 0x60, 0x82]);
}
