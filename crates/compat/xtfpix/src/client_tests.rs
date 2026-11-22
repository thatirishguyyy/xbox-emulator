use crate::api::*;
use crate::client::*;
use crate::com::*;
use core::ffi::c_void;
use core::ptr::null_mut;
use std::sync::Mutex;
use xbtp::dispatcher::IID_XBTP_DISPATCHER;
use xbtp::dispatcher_com::DispatcherCom;
use xbtp::start::XbtpStartDispatcherEx;
use xbtp::xbtp2_ext::{EndMessage, Message, MetadataVtbl, IID_XBTP_EXTENSION_METADATA};

pub static PORT_LOCK: Mutex<()> = Mutex::new(());
static SEEN: Mutex<Vec<(u32, String)>> = Mutex::new(Vec::new());
static REPLY: Mutex<String> = Mutex::new(String::new());
static RESULT: Mutex<HRESULT> = Mutex::new(0);

const SYSTEM_TCP_PORT: u32 = 0x1073;
const TITLE_TCP_PORT: u32 = 0x107d;

unsafe fn msg_text(m: *mut c_void) -> String {
    let m = m as *mut Message;
    let mut n = 0u32;
    let mut p: *mut u8 = null_mut();
    assert_eq!(((*(*m).vtbl).get_json_body)(m, &mut n, &mut p), S_OK);
    if p.is_null() {
        return String::new();
    }
    String::from_utf8_lossy(core::slice::from_raw_parts(p, n as usize)).trim_end_matches('\0').to_string()
}

unsafe fn record(op: u32, received: usize, reply: usize) -> HRESULT {
    SEEN.lock().unwrap().push((op, msg_text(received as *mut c_void)));
    let r = REPLY.lock().unwrap().clone();
    if !r.is_empty() {
        let m = reply as *mut EndMessage;
        let t = w(&r);
        assert_eq!(((*(*m).vtbl).set_json_body_w)(m, t.as_ptr()), S_OK);
    }
    *RESULT.lock().unwrap()
}

unsafe extern "system" fn on_dvr(_s: usize, received: usize, reply: usize) -> HRESULT {
    record(MSG_ENABLE_GAME_DVR, received, reply)
}

unsafe extern "system" fn on_clip(_s: usize, received: usize, reply: usize) -> HRESULT {
    record(MSG_REQUEST_VIDEO_CLIP, received, reply)
}

unsafe extern "system" fn on_telemetry(_s: usize, received: usize, reply: usize) -> HRESULT {
    record(MSG_PC_TELEMETRY, received, reply)
}

pub struct Server(*mut DispatcherCom);

impl Server {
    pub unsafe fn start(port: u32) -> Server {
        let mut out: *mut c_void = null_mut();
        assert_eq!(XbtpStartDispatcherEx(port, 0, &IID_XBTP_DISPATCHER, &mut out), S_OK);
        Server(out as *mut DispatcherCom)
    }

    pub unsafe fn add(&self, ext: &GUID, md: *mut c_void) {
        assert_eq!(((*(*self.0).vtbl).add_extension)(self.0, ext as *const GUID as *const _, md), S_OK);
    }

    pub unsafe fn add_recorder(&self) {
        let mut md: *mut c_void = null_mut();
        assert_eq!(xbtp::xbtp2_ext::XbtpCreateExtensionMetadata(&IID_XBTP_EXTENSION_METADATA, &mut md), S_OK);
        let mv = &**(md as *const *const MetadataVtbl);
        assert_eq!((mv.add_function)(md as *mut _, MSG_ENABLE_GAME_DVR, Some(on_dvr)), S_OK);
        assert_eq!((mv.add_function)(md as *mut _, MSG_REQUEST_VIDEO_CLIP, Some(on_clip)), S_OK);
        assert_eq!((mv.add_function)(md as *mut _, MSG_PC_TELEMETRY, Some(on_telemetry)), S_OK);
        self.add(&PIX_EXTENSION_ID, md);
        com_release(md);
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        unsafe {
            assert_eq!(((*(*self.0).vtbl).shutdown)(self.0), S_OK);
            ((*(*self.0).vtbl).release)(self.0);
        }
    }
}

fn reset(reply: &str, result: HRESULT) {
    SEEN.lock().unwrap().clear();
    *REPLY.lock().unwrap() = reply.to_string();
    *RESULT.lock().unwrap() = result;
}

fn seen() -> Vec<(u32, String)> {
    SEEN.lock().unwrap().clone()
}

#[derive(Debug, PartialEq, Clone)]
pub enum J {
    N(String),
    S(String),
    A(Vec<J>),
    O(std::collections::BTreeMap<String, J>),
}

pub fn canon(t: &str) -> J {
    let b: Vec<char> = t.chars().collect();
    let mut i = 0;
    parse_j(&b, &mut i)
}

fn parse_j(b: &[char], i: &mut usize) -> J {
    match b[*i] {
        '{' => {
            *i += 1;
            let mut m = std::collections::BTreeMap::new();
            while b[*i] != '}' {
                if b[*i] == ',' {
                    *i += 1;
                }
                let k = match parse_j(b, i) {
                    J::S(s) => s,
                    o => panic!("{o:?}"),
                };
                assert_eq!(b[*i], ':');
                *i += 1;
                m.insert(k, parse_j(b, i));
            }
            *i += 1;
            J::O(m)
        }
        '[' => {
            *i += 1;
            let mut v = Vec::new();
            while b[*i] != ']' {
                if b[*i] == ',' {
                    *i += 1;
                }
                v.push(parse_j(b, i));
            }
            *i += 1;
            J::A(v)
        }
        '"' => {
            *i += 1;
            let mut s = String::new();
            while b[*i] != '"' {
                if b[*i] == '\\' {
                    *i += 1;
                }
                s.push(b[*i]);
                *i += 1;
            }
            *i += 1;
            J::S(s)
        }
        _ => {
            let mut s = String::new();
            while *i < b.len() && !matches!(b[*i], ',' | '}' | ']') {
                s.push(b[*i]);
                *i += 1;
            }
            J::N(s)
        }
    }
}

fn canon_seen() -> Vec<(u32, J)> {
    seen().into_iter().map(|(o, s)| (o, canon(&s))).collect()
}
unsafe fn split(s: &str, port: u32) -> (HRESULT, String, String, u32) {
    let a = w(s);
    let mut ip = Bstr::null();
    let mut key = Bstr::null();
    let mut p = 0xffu32;
    let hr = split_console_address(a.as_ptr(), port, &mut ip.0, &mut key.0, &mut p);
    let t = |b: &Bstr| if b.0.is_null() { "<null>".to_string() } else { String::from_utf16_lossy(core::slice::from_raw_parts(b.0, wlen(b.0))) };
    (hr, t(&ip), t(&key), p)
}

#[test]
fn split_console_address_follows_console_rules() {
    unsafe {
        assert_eq!(split("10.0.0.5", 1), (S_OK, "10.0.0.5".into(), "<null>".into(), 1));
        assert_eq!(split("10.0.0.5/system", 1), (S_OK, "10.0.0.5".into(), "<null>".into(), 0));
        assert_eq!(split("10.0.0.5/TITLE", 0), (S_OK, "10.0.0.5".into(), "<null>".into(), 1));
        assert_eq!(split("10.0.0.5/Host", 0), (S_OK, "10.0.0.5".into(), "<null>".into(), 3));
        assert_eq!(split("10.0.0.5/era", 0).0, E_INVALID_VM_TARGET);
        assert_eq!(split("10.0.0.5/systems", 0).0, E_INVALID_VM_TARGET);
        assert_eq!(split("box+secret", 1), (S_OK, "box".into(), "secret".into(), 1));
        assert_eq!(split("box+secret/system", 1), (S_OK, "box".into(), "secret".into(), 0));
        assert_eq!(split("+k", 1), (S_OK, "<null>".into(), "k".into(), 1));
        assert_eq!(split("box+", 1), (S_OK, "box".into(), "<null>".into(), 1));
        assert_eq!(split("box/title+x", 1).0, E_INVALID_VM_TARGET);
        assert_eq!(split("", 1).0, E_NO_IPADDRESS_IN_CONNSTR);
        assert_eq!(split("/system", 1).0, E_NO_IPADDRESS_IN_CONNSTR);
        let long: String = core::iter::repeat('a').take(MAX_ADDRESS_CHARS).collect();
        assert_eq!(split(&long, 1).0, E_NO_IPADDRESS_IN_CONNSTR);
        let ok: String = core::iter::repeat('a').take(MAX_ADDRESS_CHARS - 1).collect();
        assert_eq!(split(&ok, 1).0, S_OK);
        let mut ip = Bstr::null();
        let mut p = 0u32;
        assert_eq!(split_console_address(null_mut(), 0, &mut ip.0, null_mut(), &mut p), E_INVALIDARG);
        let a = w("box+key/system");
        assert_eq!(split_address_port(a.as_ptr(), 1, &mut ip, &mut p), S_OK);
        assert_eq!(String::from_utf16_lossy(core::slice::from_raw_parts(ip.0, wlen(ip.0))), "box+key");
        assert_eq!(p, 0);
        let mut raw = Bstr::null();
        assert_eq!(get_raw_address(a.as_ptr(), &mut raw), S_OK);
        assert_eq!(String::from_utf16_lossy(core::slice::from_raw_parts(raw.0, wlen(raw.0))), "box+key");
        let e = w("");
        assert_eq!(get_raw_address(e.as_ptr(), &mut raw), E_NO_IPADDRESS_IN_CONNSTR);
    }
}

#[test]
fn enable_game_dvr_sends_flag_and_returns_reply_code() {
    let _g = PORT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        let srv = Server::start(SYSTEM_TCP_PORT);
        srv.add_recorder();
        reset("{}", S_OK);
        let a = w("127.0.0.1");
        assert_eq!(XtfPixEnableGameDvr(a.as_ptr(), 1), S_OK);
        assert_eq!(seen(), vec![(MSG_ENABLE_GAME_DVR, "{\"EnableGameDvr\":1}".to_string())]);
        reset("{\"XtfOperationError\":{\"ReturnCode\":-2147024891,\"Checkpoint\":2,\"Errors\":[\"x\"]}}", S_OK);
        let b = w("localhost/system");
        assert_eq!(XtfPixEnableGameDvr(b.as_ptr(), 0), E_ACCESSDENIED);
        assert_eq!(seen(), vec![(MSG_ENABLE_GAME_DVR, "{\"EnableGameDvr\":0}".to_string())]);
        reset("", 0x8007_0057u32 as i32);
        assert_eq!(XtfPixEnableGameDvr(a.as_ptr(), -3), 0x8c11_01f8u32 as i32);
        assert_eq!(seen(), vec![(MSG_ENABLE_GAME_DVR, "{\"EnableGameDvr\":-3}".to_string())]);
        assert_eq!(XtfPixEnableGameDvr(null_mut(), 1), E_POINTER);
        let bad = w("127.0.0.1/nowhere");
        assert_eq!(XtfPixEnableGameDvr(bad.as_ptr(), 1), E_INVALID_VM_TARGET);
        drop(srv);
    }
}

#[test]
fn request_video_clip_round_trips_ticks_and_path() {
    let _g = PORT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        let srv = Server::start(SYSTEM_TCP_PORT);
        srv.add_recorder();
        reset("{\"StartTime\":{\"L\":5,\"H\":1},\"EndTime\":{\"L\":4294967295,\"H\":2},\"FilePath\":\"D:\\\\clips\\\\a.mp4\"}", S_OK);
        let a = w("127.0.0.1");
        let (mut s, mut e) = (0u64, 0u64);
        let mut path: BSTR = null_mut();
        let start = 0x0000_0003_8000_0001u64;
        let end = 0x0000_0004_0000_0002u64;
        assert_eq!(XtfPixRequestVideoClip(a.as_ptr(), start, end, &mut s, &mut e, &mut path), S_OK);
        assert_eq!(canon_seen(), vec![(MSG_REQUEST_VIDEO_CLIP, canon("{\"StartTime\":{\"L\":2147483649,\"H\":3},\"EndTime\":{\"L\":2,\"H\":4}}"))]);
        assert_eq!(s, 0x1_0000_0005);
        assert_eq!(e, 0x2_ffff_ffff);
        assert!(!path.is_null());
        assert_eq!(String::from_utf16_lossy(core::slice::from_raw_parts(path, wlen(path))), "D:\\clips\\a.mp4");
        SysFreeString(path);
        reset("{\"XtfOperationError\":{\"ReturnCode\":-2147467259,\"Checkpoint\":0,\"Errors\":[]}}", S_OK);
        let mut p2: BSTR = null_mut();
        assert_eq!(XtfPixRequestVideoClip(a.as_ptr(), 1, 2, &mut s, &mut e, &mut p2), E_FAIL);
        assert!(p2.is_null());
        reset("{\"StartTime\":{\"L\":1,\"H\":0}}", S_OK);
        assert_eq!(XtfPixRequestVideoClip(a.as_ptr(), 1, 2, &mut s, &mut e, &mut p2), E_JSON_NOT_FOUND);
        assert_eq!(s, 1);
        assert_eq!(XtfPixRequestVideoClip(a.as_ptr(), 1, 2, null_mut(), &mut e, &mut p2), E_POINTER);
        assert_eq!(XtfPixRequestVideoClip(a.as_ptr(), 1, 2, &mut s, null_mut(), &mut p2), E_POINTER);
        assert_eq!(XtfPixRequestVideoClip(a.as_ptr(), 1, 2, &mut s, &mut e, null_mut()), E_POINTER);
        drop(srv);
    }
}

#[test]
fn clear_running_process_list_reaches_ported_pix_server() {
    let _g = PORT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        let srv = Server::start(TITLE_TCP_PORT);
        let mut md: *mut c_void = null_mut();
        assert_eq!(crate::server::XbtpServerEntryPoint(&PIX_EXTENSION_ID, &IID_EXTENSION_METADATA, &mut md), S_OK);
        srv.add(&PIX_EXTENSION_ID, md);
        com_release(md);
        let a = w("localhost");
        assert_eq!(XtfPixClearRunningProcessList(a.as_ptr()), S_OK);
        let t = w("127.0.0.1/title");
        assert_eq!(XtfPixClearRunningProcessList(t.as_ptr()), S_OK);
        assert_eq!(XtfPixClearRunningProcessList(null_mut()), E_POINTER);
        drop(srv);
        let srv = Server::start(TITLE_TCP_PORT);
        srv.add_recorder();
        assert_eq!(XtfPixClearRunningProcessList(a.as_ptr()), E_FAIL);
        drop(srv);
    }
}

fn put_w(v: &mut Vec<u8>, s: &str) {
    for u in s.encode_utf16().chain(Some(0)) {
        v.extend_from_slice(&u.to_le_bytes());
    }
}

fn guid_bytes(g: &GUID) -> Vec<u8> {
    let mut v = g.data1.to_le_bytes().to_vec();
    v.extend_from_slice(&g.data2.to_le_bytes());
    v.extend_from_slice(&g.data3.to_le_bytes());
    v.extend_from_slice(&g.data4);
    v
}

fn header(kind: u32) -> Vec<u8> {
    let mut v = kind.to_le_bytes().to_vec();
    v.extend_from_slice(&0x0000_0002_0000_0010u64.to_le_bytes());
    v.extend_from_slice(&guid_bytes(&GUID::new(0x01234567, 0x89ab, 0xcdef, [1, 2, 3, 4, 5, 6, 0xa, 0xbc])));
    put_w(&mut v, "10.0.1");
    v.extend_from_slice(&7u32.to_le_bytes());
    v.extend_from_slice(&8u32.to_le_bytes());
    put_w(&mut v, "wpix");
    v.extend_from_slice(&9u32.to_le_bytes());
    v.extend_from_slice(&guid_bytes(&GUID::new(0xdeadbeef, 1, 2, [0; 8])));
    v.extend_from_slice(&1234u64.to_le_bytes());
    v
}

const HEADER_JSON: &str = "\"EventType\":KIND,\"EventTime\":{\"L\":16,\"H\":2},\"SessionGUID\":\"{01234567-89AB-CDEF-0102-030405060ABC}\",\"OSVersion\":\"10.0.1\",\"RecoveryVersion\":7,\"XdkVersion\":8,\"CaptureFileType\":\"wpix\",\"CaptureFileXdkVersion\":9,\"CaptureFileGUID\":\"{DEADBEEF-0001-0002-0000-000000000000}\",\"CaptureFileSize\":{\"L\":1234,\"H\":0}";

unsafe fn telemetry_json(buf: &mut Vec<u8>) -> (HRESULT, String) {
    let mut body = ComPtr::null();
    assert_eq!(XbtpCreateJsonObject(body.out()), S_OK);
    let hr = fill_telemetry(body.0, buf.as_mut_ptr(), buf.len() as i32);
    let mut s = Bstr::null();
    assert_eq!(crate::json::obj_serialize(body.0, &mut s.0), S_OK);
    (hr, String::from_utf16_lossy(core::slice::from_raw_parts(s.0, wlen(s.0))))
}

#[test]
fn telemetry_buffer_layout_per_event_kind() {
    unsafe {
        let mut b = header(0);
        put_w(&mut b, "ctx");
        put_w(&mut b, "name");
        put_w(&mut b, "msg");
        b.extend_from_slice(&0x8000_4005u32.to_le_bytes());
        put_w(&mut b, "stack");
        let (hr, j) = telemetry_json(&mut b);
        assert_eq!(hr, S_OK);
        assert_eq!(canon(&j), canon(&format!("{{{},\"ErrorContext\":\"ctx\",\"ErrorName\":\"name\",\"ErrorMessage\":\"msg\",\"ErrorCode\":2147500037,\"StackTrace\":\"stack\"}}", HEADER_JSON.replace("KIND", "0"))));
        let mut b = header(1);
        put_w(&mut b, "fps");
        b.extend_from_slice(&59.5f64.to_le_bytes());
        let (hr, j) = telemetry_json(&mut b);
        assert_eq!(hr, S_OK);
        assert_eq!(canon(&j), canon(&format!("{{{},\"CounterName\":\"fps\",\"CounterValue\":59.5}}", HEADER_JSON.replace("KIND", "1"))));
        let mut b = header(2);
        put_w(&mut b, "connect");
        b.extend_from_slice(&guid_bytes(&GUID::new(0xa, 0xb, 0xc, [0xd, 0xe, 0xf, 0x10, 0x11, 0x12, 0x13, 0x14])));
        let (hr, j) = telemetry_json(&mut b);
        assert_eq!(hr, S_OK);
        assert_eq!(canon(&j), canon(&format!("{{{},\"Action\":\"connect\",\"DeviceConnectionGUID\":\"{{0000000A-000B-000C-0D0E-0F1011121314}}\"}}", HEADER_JSON.replace("KIND", "2"))));
        let mut b = header(3);
        put_w(&mut b, "frames");
        b.extend_from_slice(&0x0000_0001_0000_0003u64.to_le_bytes());
        let (hr, j) = telemetry_json(&mut b);
        assert_eq!(hr, S_OK);
        assert_eq!(canon(&j), canon(&format!("{{{},\"CountName\":\"frames\",\"CountValue\":{{\"L\":3,\"H\":1}}}}", HEADER_JSON.replace("KIND", "3"))));
        let mut b = header(9);
        let (hr, j) = telemetry_json(&mut b);
        assert_eq!(hr, S_OK);
        assert_eq!(canon(&j), canon(&format!("{{{}}}", HEADER_JSON.replace("KIND", "9"))));
        let mut b = header(1);
        put_w(&mut b, "fps");
        b.extend_from_slice(&[0u8; 7]);
        assert_eq!(telemetry_json(&mut b).0, E_OUTOFMEMORY);
        let mut b = header(0);
        let n = b.len();
        b.truncate(n - 1);
        assert_eq!(telemetry_json(&mut b).0, E_OUTOFMEMORY);
        let mut b = 3u32.to_le_bytes().to_vec();
        assert_eq!(telemetry_json(&mut b).0, E_OUTOFMEMORY);
        let mut b = header(2);
        b.extend_from_slice(&[b'a', 0, b'b']);
        let (hr, j) = telemetry_json(&mut b);
        assert_eq!(hr, E_OUTOFMEMORY);
        assert!(!j.contains("Action"));
        assert_eq!(&b[b.len() - 3..], &[0, 0, b'b']);
        let mut raw = vec![b'x', 0, b'y', 0, b'z'];
        let mut c = Cursor { cur: raw.as_mut_ptr(), end: raw.as_mut_ptr().add(raw.len()) };
        let s = c.string().unwrap();
        assert_eq!(s as *const u8, raw.as_ptr());
        assert_eq!(c.cur, c.end);
        assert_eq!(raw, vec![b'x', 0, 0, 0, b'z']);
        assert!(c.string().is_err());
    }
}

#[test]
fn telemetry_event_is_sent_to_system_port() {
    let _g = PORT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        let srv = Server::start(SYSTEM_TCP_PORT);
        srv.add_recorder();
        reset("", S_OK);
        let mut b = header(3);
        put_w(&mut b, "frames");
        b.extend_from_slice(&5u64.to_le_bytes());
        let a = w("127.0.0.1/title");
        assert_eq!(XtfPixPCLogTelemetryEvent(a.as_ptr(), b.as_mut_ptr(), b.len() as i32), S_OK);
        let got = seen();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, MSG_PC_TELEMETRY);
        assert_eq!(canon(&got[0].1), canon(&format!("{{{},\"CountName\":\"frames\",\"CountValue\":{{\"L\":5,\"H\":0}}}}", HEADER_JSON.replace("KIND", "3"))));
        reset("", S_OK);
        let mut short = vec![1u8, 2];
        assert_eq!(XtfPixPCLogTelemetryEvent(a.as_ptr(), short.as_mut_ptr(), 2), E_OUTOFMEMORY);
        assert!(seen().is_empty());
        let e = w("");
        assert_eq!(XtfPixPCLogTelemetryEvent(e.as_ptr(), b.as_mut_ptr(), b.len() as i32), E_INVALIDARG);
        assert_eq!(XtfPixPCLogTelemetryEvent(null_mut(), b.as_mut_ptr(), b.len() as i32), E_INVALIDARG);
        drop(srv);
    }
}

#[test]
fn remote_console_addresses_fail_offline_with_no_route() {
    let _g = PORT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        let mut b = header(9);
        let a = w("192.168.1.20");
        let t0 = std::time::Instant::now();
        assert_eq!(XtfPixPCLogTelemetryEvent(a.as_ptr(), b.as_mut_ptr(), b.len() as i32), E_COULD_NOT_CONNECT);
        assert!(t0.elapsed().as_millis() >= 700, "{:?}", t0.elapsed());
        let mut s = ComPtr::null();
        let t0 = std::time::Instant::now();
        assert_eq!(open_session_impl(a.as_ptr(), XBTP_PORT_SYSTEM, &PIX_EXTENSION_ID, s.out(), Some(3000)), E_COULD_NOT_CONNECT);
        assert!(t0.elapsed().as_millis() >= 1200, "{:?}", t0.elapsed());
        assert!(s.0.is_null());
    }
}
