use crate::debugclient::*;
use crate::monitor::*;
use crate::server::*;
use crate::win::*;
use crate::*;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};
use xtfmsg::com::{self, metadata, Com, Obj, IID_XBTP_EXTENSION_METADATA};
use xtfmsg::fixture::{self, as_obj, end_json, end_message, load_xbtp, message_with_json};
use xtfmsg::sys::*;

const CHILD_ENV: &str = "XTFDEBUGMONITOR_CHILD";
const CHILD_TEST: &str = "tests::debug_child_main";
const CHILD_TICK: &str = "xtfdebugmonitor child tick";

pub static LIVE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct ForceNull;

impl ForceNull {
    fn on() -> ForceNull {
        FORCE_NULL_INSTANCE.with(|f| f.set(true));
        ForceNull
    }
}

impl Drop for ForceNull {
    fn drop(&mut self) {
        FORCE_NULL_INSTANCE.with(|f| f.set(false));
    }
}

pub(crate) unsafe fn call(f: OperationFnRaw, session: usize, request: Option<&str>) -> (HRESULT, String) {
    let req = match request {
        Some(j) => message_with_json(j),
        None => Com::null(),
    };
    let reply = end_message();
    let hr = f(session, req.get() as usize, reply.get() as usize);
    let text = end_json(reply.get()).unwrap_or_default();
    (hr, text)
}

pub(crate) type OperationFnRaw = unsafe extern "system" fn(usize, usize, usize) -> HRESULT;

pub(crate) fn operation_error(hr: HRESULT, line: u32) -> String {
    format!("\"XtfOperationError\":{{\"ReturnCode\":{},\"Checkpoint\":0,\"Errors\":[\"Error 0x{:X} File xbox\\\\devex\\\\xtf\\\\dll\\\\debugmonitor\\\\server.cpp : {}\"]}}", hr, hr as u32, line)
}

fn num_field(text: &str, key: &str) -> Option<f64> {
    let k = format!("\"{key}\":");
    let i = text.find(&k)? + k.len();
    let rest = &text[i..];
    let end = rest.find([',', '}', ']']).unwrap_or(rest.len());
    rest[..end].trim().parse().ok()
}

#[derive(Clone, Debug, PartialEq)]
enum J {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<J>),
    Obj(std::collections::BTreeMap<String, J>),
}

impl J {
    fn get(&self, k: &str) -> Option<&J> {
        match self {
            J::Obj(m) => m.get(k),
            _ => None,
        }
    }

    fn render(&self) -> String {
        match self {
            J::Null => "null".into(),
            J::Bool(b) => b.to_string(),
            J::Num(n) => n.to_string(),
            J::Str(s) => format!("{s:?}"),
            J::Arr(v) => format!("[{}]", v.iter().map(J::render).collect::<Vec<_>>().join(",")),
            J::Obj(m) => format!("{{{}}}", m.iter().map(|(k, v)| format!("{k:?}:{}", v.render())).collect::<Vec<_>>().join(",")),
        }
    }
}

struct P<'a> {
    b: &'a [u8],
    i: usize,
}

impl P<'_> {
    fn ws(&mut self) {
        while self.i < self.b.len() && self.b[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }

    fn eat(&mut self, c: u8) {
        self.ws();
        assert_eq!(self.b.get(self.i), Some(&c), "json at {}", self.i);
        self.i += 1;
    }

    fn string(&mut self) -> String {
        self.eat(b'"');
        let mut out = Vec::new();
        loop {
            let c = self.b[self.i];
            self.i += 1;
            match c {
                b'"' => break,
                b'\\' => {
                    let e = self.b[self.i];
                    self.i += 1;
                    match e {
                        b'n' => out.push(b'\n'),
                        b't' => out.push(b'\t'),
                        b'r' => out.push(b'\r'),
                        b'u' => {
                            let h = std::str::from_utf8(&self.b[self.i..self.i + 4]).unwrap();
                            self.i += 4;
                            let ch = char::from_u32(u32::from_str_radix(h, 16).unwrap()).unwrap_or('?');
                            out.extend_from_slice(ch.to_string().as_bytes());
                        }
                        o => out.push(o),
                    }
                }
                o => out.push(o),
            }
        }
        String::from_utf8(out).unwrap()
    }

    fn value(&mut self) -> J {
        self.ws();
        match self.b[self.i] {
            b'{' => {
                self.i += 1;
                let mut m = std::collections::BTreeMap::new();
                self.ws();
                if self.b[self.i] == b'}' {
                    self.i += 1;
                    return J::Obj(m);
                }
                loop {
                    let k = self.string();
                    self.eat(b':');
                    let v = self.value();
                    assert!(m.insert(k, v).is_none(), "duplicate key");
                    self.ws();
                    let c = self.b[self.i];
                    self.i += 1;
                    if c == b'}' {
                        return J::Obj(m);
                    }
                    assert_eq!(c, b',');
                }
            }
            b'[' => {
                self.i += 1;
                let mut v = Vec::new();
                self.ws();
                if self.b[self.i] == b']' {
                    self.i += 1;
                    return J::Arr(v);
                }
                loop {
                    v.push(self.value());
                    self.ws();
                    let c = self.b[self.i];
                    self.i += 1;
                    if c == b']' {
                        return J::Arr(v);
                    }
                    assert_eq!(c, b',');
                }
            }
            b'"' => J::Str(self.string()),
            b't' => {
                self.i += 4;
                J::Bool(true)
            }
            b'f' => {
                self.i += 5;
                J::Bool(false)
            }
            b'n' => {
                self.i += 4;
                J::Null
            }
            _ => {
                let st = self.i;
                while self.i < self.b.len() && matches!(self.b[self.i], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') {
                    self.i += 1;
                }
                J::Num(std::str::from_utf8(&self.b[st..self.i]).unwrap().parse().unwrap())
            }
        }
    }
}

fn parse(s: &str) -> J {
    let mut p = P { b: s.as_bytes(), i: 0 };
    let v = p.value();
    p.ws();
    assert_eq!(p.i, s.len(), "trailing json in {s}");
    v
}

pub(crate) fn canon(s: &str) -> String {
    parse(s).render()
}

fn canon_all(v: &[String]) -> Vec<String> {
    v.iter().map(|s| canon(s)).collect()
}

fn ev(m: &str) -> Option<u32> {
    match parse(m).get("EventCode") {
        Some(J::Num(n)) => Some(*n as u32),
        _ => None,
    }
}

pub(crate) fn process_object(query: &str, pid: u32) -> Option<String> {
    let Some(J::Arr(ps)) = parse(query).get("Processes").cloned() else { return None };
    ps.into_iter().find(|p| p.get("ProcessId") == Some(&J::Num(pid as f64))).map(|p| p.render())
}

pub(crate) unsafe fn query() -> String {
    let (hr, text) = call(query_monitor_handler, 0, None);
    assert_eq!(hr, S_OK, "{text}");
    text
}

fn synthetic(pid: u32) -> Arc<ProcessMonitor> {
    assert_eq!(pid & 3, 1, "synthetic monitors use pids Windows never assigns");
    let dm = instance().unwrap();
    let m = Arc::new(ProcessMonitor::new(pid));
    assert_eq!(m.init_result, S_OK);
    lock(&dm.monitors).push(m.clone());
    m
}

fn remove(pid: u32) {
    let dm = instance().unwrap();
    let mut v = lock(&dm.monitors);
    if let Some(i) = v.iter().position(|m| m.pid == pid) {
        let m = v.remove(i);
        lock(&m.clients).clear();
    }
}

fn signaled(h: HANDLE) -> bool {
    unsafe { WaitForSingleObject(h, 0) == 0 }
}

fn sent(s: *mut fixture::Session) -> Vec<String> {
    unsafe { lock(&(*s).sent).clone() }
}

#[test]
fn export_table_matches_console_dll() {
    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/exports.txt")).unwrap();
    xtfcommon::exports::check_against_titles("xtfdebugmonitor.dll", &text);
    for e in xtfcommon::exports::parse(&text) {
        if e.status != "missing" {
            assert!(xtfcommon::exports::source_defines(concat!(env!("CARGO_MANIFEST_DIR"), "/src"), &e), "{}", e.name);
        }
    }
}

#[test]
fn server_entry_point_checks_ids_and_registers_four_handlers() {
    load_xbtp();
    unsafe {
        let bad = GUID::new(1, 2, 3, [4; 8]);
        let sentinel = 0x1234usize as Obj;
        let mut out = sentinel;
        assert_eq!(XtfOrdinal_4(&bad, &IID_XBTP_EXTENSION_METADATA, &mut out), E_UNEXPECTED);
        assert_eq!(out, sentinel);
        assert_eq!(XtfOrdinal_4(&DEBUG_MONITOR_EXTENSION_ID, &bad, &mut out), E_UNEXPECTED);
        assert_eq!(out, sentinel);
        let mut md: Obj = core::ptr::null_mut();
        assert_eq!(XtfOrdinal_4(&DEBUG_MONITOR_EXTENSION_ID, &IID_XBTP_EXTENSION_METADATA, &mut md), S_OK);
        assert!(!md.is_null());
        let mut n = 0u32;
        assert_eq!(metadata::get_handler_count(md, &mut n), S_OK);
        assert_eq!(n, 4);
        com::release(md);
        let ids: Vec<u32> = HANDLERS.iter().map(|h| h.0).collect();
        assert_eq!(ids, [5, 3, 2, 4]);
        assert_eq!(HANDLERS.iter().map(|h| h.3).collect::<Vec<_>>(), [134, 135, 136, 137]);
    }
}

#[test]
fn server_entry_point_out_of_memory_without_singleton() {
    load_xbtp();
    let _g = ForceNull::on();
    unsafe {
        let mut out: Obj = core::ptr::null_mut();
        assert_eq!(XtfOrdinal_4(&DEBUG_MONITOR_EXTENSION_ID, &IID_XBTP_EXTENSION_METADATA, &mut out), E_OUTOFMEMORY);
        assert!(out.is_null());
    }
}

#[test]
fn query_monitor_without_singleton_replies_empty_process_list() {
    load_xbtp();
    let _g = ForceNull::on();
    unsafe {
        let (hr, text) = call(query_monitor_handler, 0, None);
        assert_eq!(hr, S_OK);
        assert_eq!(canon(&text), canon("{\"Processes\":[]}"));
    }
}

#[test]
fn query_monitor_reports_console_process_and_client_fields() {
    load_xbtp();
    let pid = 0xFFFF_FF01u32;
    let m = synthetic(pid);
    let c = m.add_debug_client(FILTER_OUTPUT_DEBUG_STRING, 0);
    unsafe {
        let p = process_object(&query(), pid).unwrap();
        assert_eq!(p, canon(&format!("{{\"ProcessId\":{pid},\"IsActive\":1,\"IsBroken\":0,\"Clients\":[{{\"ClientIdentifier\":0,\"IsActive\":1}}]}}")));
        c.monitoring.store(true, Ordering::SeqCst);
        m.broken.store(true, Ordering::SeqCst);
        m.active.store(false, Ordering::SeqCst);
        let p = process_object(&query(), pid).unwrap();
        assert_eq!(p, canon(&format!("{{\"ProcessId\":{pid},\"IsActive\":0,\"IsBroken\":1,\"Clients\":[{{\"ClientIdentifier\":1,\"IsActive\":1}}]}}")));
    }
    c.monitoring.store(false, Ordering::SeqCst);
    remove(pid);
}

#[test]
fn monitor_process_null_session_fails_invalidarg_at_268() {
    load_xbtp();
    unsafe {
        let (hr, text) = call(monitor_process_handler, 0, Some("{\"ProcessId\":4294967043}"));
        assert_eq!(hr, E_INVALIDARG);
        assert_eq!(canon(&text), canon(&format!("{{{}}}", operation_error(E_INVALIDARG, 268))));
    }
    assert!(instance().unwrap().find(4294967043).is_none());
}

#[test]
fn monitor_process_body_errors_and_missing_singleton() {
    load_xbtp();
    unsafe {
        let (hr, text) = call(monitor_process_handler, 1, Some("{}"));
        assert_eq!(hr, E_JSON_NOT_FOUND);
        assert_eq!(canon(&text), canon(&format!("{{{}}}", operation_error(E_JSON_NOT_FOUND, 234))));
        let (hr, text) = call(monitor_process_handler, 1, None);
        assert_eq!(hr, E_INVALIDARG);
        assert_eq!(canon(&text), canon(&format!("{{{}}}", operation_error(E_INVALIDARG, 233))));
        let _g = ForceNull::on();
        let (hr, text) = call(monitor_process_handler, 1, Some("{\"ProcessId\":4294967047}"));
        assert_eq!(hr, E_OUTOFMEMORY);
        assert_eq!(canon(&text), canon(&format!("{{{}}}", operation_error(E_OUTOFMEMORY, 255))));
    }
}

#[test]
fn detach_process_signals_client_counts_detaches_and_forces_all() {
    load_xbtp();
    let pid = 0xFFFF_FF05u32;
    let m = synthetic(pid);
    let a = m.add_debug_client(FILTER_OUTPUT_DEBUG_STRING, 0);
    let b = m.add_debug_client(FILTER_OUTPUT_DEBUG_STRING, 0);
    assert_eq!(b.session_identifier, a.session_identifier.wrapping_add(1).max(b.session_identifier));
    unsafe {
        let (hr, text) = call(detach_process_handler, 0, Some(&format!("{{\"ProcessId\":{pid},\"ClientIdentifier\":{}}}", b.session_identifier)));
        assert_eq!((hr, text.as_str()), (S_OK, ""));
        assert!(signaled(b.stop_event));
        assert!(!signaled(a.stop_event));
        assert_eq!(m.detach_count.load(Ordering::SeqCst), 0);
        let (hr, _) = call(detach_process_handler, 0, Some(&format!("{{\"ProcessId\":{pid}}}")));
        assert_eq!(hr, S_OK);
        assert_eq!(m.detach_count.load(Ordering::SeqCst), 1);
        assert!(!m.force_all.load(Ordering::SeqCst));
        let (hr, _) = call(detach_process_handler, 0, Some(&format!("{{\"ProcessId\":{pid},\"ForceAll\":1,\"ClientIdentifier\":{}}}", a.session_identifier)));
        assert_eq!(hr, S_OK);
        assert!(m.force_all.load(Ordering::SeqCst));
        assert!(!signaled(a.stop_event));
        let (hr, text) = call(detach_process_handler, 0, Some("{\"ForceAll\":1}"));
        assert_eq!(hr, E_JSON_NOT_FOUND);
        assert_eq!(canon(&text), canon(&format!("{{{}}}", operation_error(E_JSON_NOT_FOUND, 301))));
        let (hr, text) = call(detach_process_handler, 0, None);
        assert_eq!(hr, E_INVALIDARG);
        assert_eq!(canon(&text), canon(&format!("{{{}}}", operation_error(E_INVALIDARG, 300))));
        let (hr, text) = call(detach_process_handler, 0, Some("{\"ProcessId\":4294967049,\"ForceAll\":1}"));
        assert_eq!((hr, text.as_str()), (S_OK, ""));
        let _g = ForceNull::on();
        let (hr, text) = call(detach_process_handler, 0, None);
        assert_eq!((hr, text.as_str()), (S_OK, ""));
    }
    remove(pid);
}

#[test]
fn continue_process_stores_status_clears_break_and_signals() {
    load_xbtp();
    let pid = 0xFFFF_FF0Du32;
    let m = synthetic(pid);
    m.broken.store(true, Ordering::SeqCst);
    m.continue_status.store(DBG_EXCEPTION_NOT_HANDLED, Ordering::SeqCst);
    unsafe {
        let (hr, text) = call(continue_process_handler, 0, Some(&format!("{{\"ProcessId\":{pid},\"ExceptionContinue\":65538}}")));
        assert_eq!((hr, text.as_str()), (S_OK, ""));
        assert_eq!(m.continue_status.load(Ordering::SeqCst), DBG_CONTINUE);
        assert!(!m.broken.load(Ordering::SeqCst));
        assert!(signaled(m.continue_event));
        let (hr, text) = call(continue_process_handler, 0, Some(&format!("{{\"ProcessId\":{pid}}}")));
        assert_eq!(hr, E_JSON_NOT_FOUND);
        assert_eq!(canon(&text), canon(&format!("{{{}}}", operation_error(E_JSON_NOT_FOUND, 347))));
        let (hr, text) = call(continue_process_handler, 0, Some("{\"ExceptionContinue\":65538}"));
        assert_eq!(hr, E_JSON_NOT_FOUND);
        assert_eq!(canon(&text), canon(&format!("{{{}}}", operation_error(E_JSON_NOT_FOUND, 346))));
        let (hr, text) = call(continue_process_handler, 0, None);
        assert_eq!(hr, E_INVALIDARG);
        assert_eq!(canon(&text), canon(&format!("{{{}}}", operation_error(E_INVALIDARG, 345))));
        let _g = ForceNull::on();
        let (hr, text) = call(continue_process_handler, 0, None);
        assert_eq!((hr, text.as_str()), (S_OK, ""));
    }
    remove(pid);
}

fn entry(code: u32) -> Entry {
    let mut e = Entry::new();
    e.code = code;
    e.pid = 0x1234;
    e.tid = 0x5678;
    e
}

#[test]
fn serializers_follow_filter_gates_and_console_fields() {
    load_xbtp();
    let m = Arc::new(ProcessMonitor::new(0xFFFF_FF11));
    unsafe {
        let s = fixture::session();
        let c = DebugClient::new(&m, 0x3ff, as_obj(s) as usize);
        let id = c.session_identifier;
        assert_eq!(c.send_on_attach(E_INVALIDARG), S_OK);
        let mut ex = entry(EXCEPTION_DEBUG_EVENT);
        ex.data1 = 0x8000_0003;
        ex.data2 = 1;
        ex.data3 = 1;
        ex.address = 0x7ffe_1234_5678;
        assert_eq!(c.send_on_exception(&ex), S_OK);
        let mut ct = entry(CREATE_THREAD_DEBUG_EVENT);
        ct.address = 0x1_0000_0010;
        assert_eq!(c.send_on_create_thread(&ct), S_OK);
        let mut cp = entry(CREATE_PROCESS_DEBUG_EVENT);
        cp.address = 0x14000_1000;
        assert_eq!(c.send_on_create_process(&cp), S_OK);
        let mut et = entry(EXIT_THREAD_DEBUG_EVENT);
        et.data1 = 0xC000_0005;
        assert_eq!(c.send_on_exit_thread(&et), S_OK);
        let mut ep = entry(EXIT_PROCESS_DEBUG_EVENT);
        ep.data1 = 7;
        assert_eq!(c.send_on_exit_process(&ep), S_OK);
        let mut ld = entry(LOAD_DLL_DEBUG_EVENT);
        ld.data1 = 1;
        ld.address = 0x7ff8_0000_0000;
        for (i, u) in "C:\\x\\a.dll".encode_utf16().enumerate() {
            ld.message[i * 2..i * 2 + 2].copy_from_slice(&u.to_le_bytes());
        }
        assert_eq!(c.send_on_load_dll(&ld), S_OK);
        let mut la = entry(LOAD_DLL_DEBUG_EVENT);
        la.message[..6].copy_from_slice(b"b.dll\0");
        assert_eq!(c.send_on_load_dll(&la), S_OK);
        let mut ud = entry(UNLOAD_DLL_DEBUG_EVENT);
        ud.address = 0x1000;
        assert_eq!(c.send_on_unload_dll(&ud), S_OK);
        let mut od = entry(OUTPUT_DEBUG_STRING_EVENT);
        od.message[..4].copy_from_slice(b"hi\n\0");
        assert_eq!(c.send_on_output_debug_string(&od), S_OK);
        let empty = entry(OUTPUT_DEBUG_STRING_EVENT);
        assert_eq!(c.send_on_output_debug_string(&empty), S_OK);
        let mut rp = entry(RIP_EVENT);
        rp.data1 = 5;
        rp.data2 = 2;
        assert_eq!(c.send_on_rip(&rp), S_OK);
        let head = "\"ProcessId\":4660,\"ThreadId\":22136";
        let want = vec![
            format!("{{\"AttachResult\":-2147024809,\"ClientIdentifier\":{id}}}"),
            format!("{{\"EventCode\":1,{head},\"ExceptionCode\":2147483651,\"ExceptionFirstChance\":1,\"ContinueRequested\":1,\"Address\":{{\"L\":305419896,\"H\":32766}}}}"),
            format!("{{\"EventCode\":2,{head},\"Address\":{{\"L\":16,\"H\":1}}}}"),
            format!("{{\"EventCode\":3,{head},\"Address\":{{\"L\":1073745920,\"H\":1}}}}"),
            format!("{{\"EventCode\":4,{head},\"ExitCode\":3221225477}}"),
            format!("{{\"EventCode\":5,{head},\"ExitCode\":7}}"),
            format!("{{\"EventCode\":6,{head},\"Address\":{{\"L\":0,\"H\":32760}},\"Message\":\"C:\\\\x\\\\a.dll\"}}"),
            format!("{{\"EventCode\":6,{head},\"Address\":{{\"L\":0,\"H\":0}},\"Message\":\"b.dll\"}}"),
            format!("{{\"EventCode\":7,{head},\"Address\":{{\"L\":4096,\"H\":0}}}}"),
            format!("{{\"EventCode\":8,{head},\"DebugString\":\"hi\\n\"}}"),
            format!("{{\"EventCode\":8,{head},\"DebugString\":\"\"}}"),
            format!("{{\"EventCode\":9,{head},\"ExceptionCode\":5,\"ExitType\":2}}"),
        ];
        assert_eq!(canon_all(&sent(s)), canon_all(&want));
        let gated = DebugClient::new(&m, 0, as_obj(s) as usize);
        for (f, e) in [
            (DebugClient::send_on_exception as unsafe fn(&DebugClient, &Entry) -> HRESULT, &ex),
            (DebugClient::send_on_create_thread, &ct),
            (DebugClient::send_on_create_process, &cp),
            (DebugClient::send_on_exit_thread, &et),
            (DebugClient::send_on_exit_process, &ep),
            (DebugClient::send_on_load_dll, &ld),
            (DebugClient::send_on_unload_dll, &ud),
            (DebugClient::send_on_output_debug_string, &od),
            (DebugClient::send_on_rip, &rp),
        ] {
            assert_eq!(f(&gated, e), S_OK);
        }
        assert_eq!(sent(s).len(), want.len());
        for (bit, f, e) in [
            (FILTER_EXCEPTION, DebugClient::send_on_exception as unsafe fn(&DebugClient, &Entry) -> HRESULT, &ex),
            (FILTER_CREATE_THREAD, DebugClient::send_on_create_thread, &ct),
            (FILTER_CREATE_PROCESS, DebugClient::send_on_create_process, &cp),
            (FILTER_EXIT_THREAD, DebugClient::send_on_exit_thread, &et),
            (FILTER_EXIT_PROCESS, DebugClient::send_on_exit_process, &ep),
            (FILTER_LOAD_DLL, DebugClient::send_on_load_dll, &ld),
            (FILTER_UNLOAD_DLL, DebugClient::send_on_unload_dll, &ud),
            (FILTER_OUTPUT_DEBUG_STRING, DebugClient::send_on_output_debug_string, &od),
            (FILTER_RIP, DebugClient::send_on_rip, &rp),
        ] {
            let only = DebugClient::new(&m, bit, as_obj(s) as usize);
            let before = sent(s).len();
            assert_eq!(f(&only, e), S_OK);
            assert_eq!(sent(s).len(), before + 1, "filter bit {bit:#x}");
        }
        drop(c);
        drop(gated);
        com::release(as_obj(s));
    }
}

#[test]
fn process_next_event_walks_ring_and_wraps_read_head() {
    load_xbtp();
    let m = Arc::new(ProcessMonitor::new(0xFFFF_FF15));
    unsafe {
        let s = fixture::session();
        let c = DebugClient::new(&m, FILTER_EXIT_PROCESS, as_obj(s) as usize);
        c.read_head.store(255, Ordering::SeqCst);
        let mut e = entry(EXIT_PROCESS_DEBUG_EVENT);
        e.data1 = 3;
        *lock(&m.ring[255]) = e;
        *lock(&m.ring[0]) = entry(0x77);
        assert_eq!(c.process_next_event(), S_OK);
        assert_eq!(c.read_head.load(Ordering::SeqCst), 0);
        assert_eq!(c.process_next_event(), S_OK);
        assert_eq!(c.read_head.load(Ordering::SeqCst), 1);
        assert_eq!(canon_all(&sent(s)), canon_all(&["{\"EventCode\":5,\"ProcessId\":4660,\"ThreadId\":22136,\"ExitCode\":3}".to_string()]));
        drop(c);
        com::release(as_obj(s));
    }
}

#[test]
fn client_monitor_sends_attach_drains_ring_and_stops() {
    load_xbtp();
    let m = Arc::new(ProcessMonitor::new(0xFFFF_FF19));
    unsafe {
        let s = fixture::session();
        let c = DebugClient::new(&m, FILTER_EXIT_THREAD, as_obj(s) as usize);
        m.attach_done.store(true, Ordering::SeqCst);
        m.attach_result.store(E_INVALIDARG, Ordering::SeqCst);
        *lock(&m.ring[0]) = entry(EXIT_THREAD_DEBUG_EVENT);
        m.write_head.store(1, Ordering::SeqCst);
        m.active.store(false, Ordering::SeqCst);
        assert_eq!(c.monitor(), S_OK);
        assert!(!c.monitoring.load(Ordering::SeqCst));
        assert_eq!(canon_all(&sent(s)), canon_all(&[format!("{{\"AttachResult\":-2147024809,\"ClientIdentifier\":{}}}", c.session_identifier)]));
        assert_eq!(c.read_head.load(Ordering::SeqCst), 0);
        m.active.store(true, Ordering::SeqCst);
        m.attach_result.store(S_OK, Ordering::SeqCst);
        SetEvent(c.stop_event);
        assert_eq!(c.monitor(), S_OK);
        let got = sent(s);
        assert_eq!(got.len(), 3);
        assert_eq!(canon(&got[1]), canon(&format!("{{\"AttachResult\":0,\"ClientIdentifier\":{}}}", c.session_identifier)));
        assert_eq!(canon(&got[2]), canon("{\"EventCode\":4,\"ProcessId\":4660,\"ThreadId\":22136,\"ExitCode\":0}"));
        assert_eq!(c.read_head.load(Ordering::SeqCst), 1);
        drop(c);
        com::release(as_obj(s));
    }
}

#[test]
fn ansi_conversion_matches_set_ansi() {
    unsafe {
        assert!(bstr_ansi(b"").unwrap().is_null());
        let b = bstr_ansi(b"abc").unwrap();
        assert_eq!(SysStringLen(b), 3);
        assert_eq!(from_wide(b), "abc");
        SysFreeString(b);
    }
}

#[test]
fn entry_strings_are_bounded_by_the_message_buffer() {
    let mut e = Entry::new();
    e.message.iter_mut().for_each(|b| *b = b'a');
    assert_eq!(e.ansi_message().len(), MESSAGE_BYTES);
    assert_eq!(e.wide_message().len(), MESSAGE_BYTES / 2);
    e.clear();
    assert!(e.ansi_message().is_empty());
    assert_eq!(core::mem::size_of::<DebugEvent>(), 0xb0);
}

#[test]
#[ignore]
fn debug_child_main() {
    let Ok(mode) = std::env::var(CHILD_ENV) else { return };
    for i in 0..100 {
        if mode == "ods" {
            let s = format!("{CHILD_TICK} {i}\n\0");
            unsafe { xtfcommon::sys::OutputDebugStringA(s.as_ptr()) };
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

struct LiveRun {
    child: Child,
    pid: u32,
    session: *mut fixture::Session,
    done: mpsc::Receiver<HRESULT>,
    finished: Option<HRESULT>,
    _guard: std::sync::MutexGuard<'static, ()>,
}

impl LiveRun {
    fn start(mode: &str, filter: u32) -> LiveRun {
        let guard = LIVE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        load_xbtp();
        let child = Command::new(std::env::current_exe().unwrap())
            .args([CHILD_TEST, "--exact", "--ignored", "--test-threads=1", "-q"])
            .env(CHILD_ENV, mode)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let pid = child.id();
        assert_ne!(pid, 0);
        assert_ne!(pid, std::process::id());
        let session = unsafe { fixture::session() };
        let s = session as usize;
        let (tx, done) = mpsc::channel();
        std::thread::spawn(move || {
            let (hr, _) = unsafe { call(monitor_process_handler, s, Some(&format!("{{\"ProcessId\":{pid},\"FilterFlags\":{filter}}}"))) };
            let _ = tx.send(hr);
        });
        LiveRun { child, pid, session, done, finished: None, _guard: guard }
    }

    fn wait_sent(&self, what: &str, pred: impl Fn(&str) -> bool) -> String {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(m) = sent(self.session).into_iter().find(|m| pred(m)) {
                return m;
            }
            assert!(Instant::now() < deadline, "timed out waiting for {what}; got {:?}", sent(self.session));
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn wait_query(&self, what: &str, pred: impl Fn(&str) -> bool) -> String {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let p = unsafe { process_object(&query(), self.pid) }.unwrap_or_default();
            if pred(&p) {
                return p;
            }
            assert!(Instant::now() < deadline, "timed out waiting for {what}; last {p}");
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn attach(&self) -> (f64, u32) {
        let a = self.wait_sent("attach result", |m| m.contains("\"AttachResult\""));
        let hr = num_field(&a, "AttachResult").unwrap();
        assert_eq!(hr, 0.0, "DebugActiveProcess on spawned child {} failed: AttachResult {hr} (0x{:08X})", self.pid, hr as i32 as u32);
        (hr, num_field(&a, "ClientIdentifier").unwrap() as u32)
    }

    fn handler_result(&mut self) -> HRESULT {
        if self.finished.is_none() {
            self.finished = Some(self.done.recv_timeout(Duration::from_secs(20)).expect("MonitorProcess handler did not return"));
        }
        self.finished.unwrap()
    }
}

impl Drop for LiveRun {
    fn drop(&mut self) {
        unsafe {
            let pid = self.pid;
            call(continue_process_handler, 0, Some(&format!("{{\"ProcessId\":{pid},\"ExceptionContinue\":65538}}")));
            call(detach_process_handler, 0, Some(&format!("{{\"ProcessId\":{pid},\"ForceAll\":1}}")));
        }
        if self.finished.is_none() {
            self.finished = self.done.recv_timeout(Duration::from_secs(20)).ok();
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        if self.finished.is_some() {
            unsafe { com::release(as_obj(self.session)) };
        }
    }
}

#[test]
fn live_monitor_process_streams_events_from_spawned_child() {
    let mut run = LiveRun::start("ods", FILTER_CREATE_PROCESS | FILTER_LOAD_DLL | FILTER_OUTPUT_DEBUG_STRING);
    let pid = run.pid;
    let (_, id) = run.attach();
    let cp = run.wait_sent("create process", |m| ev(m) == Some(3));
    assert_eq!(num_field(&cp, "ProcessId"), Some(pid as f64));
    assert!(matches!(parse(&cp).get("Address"), Some(J::Obj(a)) if a.contains_key("L") && a.contains_key("H")));
    let dll = run.wait_sent("load dll with a name", |m| ev(m) == Some(6) && m.to_ascii_lowercase().contains(".dll\""));
    assert_eq!(num_field(&dll, "ProcessId"), Some(pid as f64));
    let ods = run.wait_sent("debug string", |m| ev(m) == Some(8) && m.contains(CHILD_TICK));
    assert_eq!(num_field(&ods, "ProcessId"), Some(pid as f64));
    assert!(sent(run.session).iter().all(|m| ev(m) != Some(2) && ev(m) != Some(1)));
    let p = run.wait_query("monitoring client", |p| p.contains("\"Clients\":[{\"ClientIdentifier\":1,\"IsActive\":1}]"));
    assert!(p.contains("\"IsActive\":1,\"IsBroken\":0"));
    unsafe {
        let (hr, text) = call(detach_process_handler, 0, Some(&format!("{{\"ProcessId\":{pid},\"ClientIdentifier\":{id}}}")));
        assert_eq!((hr, text.as_str()), (S_OK, ""));
    }
    assert_eq!(run.handler_result(), S_OK);
    run.wait_query("worker exit", |p| p.contains("\"IsActive\":0,\"IsBroken\":0"));
    assert!(run.child.try_wait().unwrap().is_none(), "child must survive DebugActiveProcessStop");
}

#[test]
fn live_break_on_exception_holds_child_until_continue() {
    let mut run = LiveRun::start("sleep", FILTER_BREAK_ON_EXCEPTION);
    let pid = run.pid;
    run.attach();
    let ex = run.wait_sent("breakpoint exception", |m| ev(m) == Some(1) && m.contains("\"ContinueRequested\":1"));
    assert_eq!(num_field(&ex, "ExceptionCode"), Some(0x8000_0003u32 as f64));
    assert_eq!(num_field(&ex, "ProcessId"), Some(pid as f64));
    assert!(sent(run.session).iter().all(|m| m.contains("\"AttachResult\"") || ev(m) == Some(1)));
    run.wait_query("broken", |p| p.contains("\"IsBroken\":1"));
    unsafe {
        let (hr, text) = call(continue_process_handler, 0, Some(&format!("{{\"ProcessId\":{pid},\"ExceptionContinue\":65538}}")));
        assert_eq!((hr, text.as_str()), (S_OK, ""));
    }
    run.wait_query("continued", |p| p.contains("\"IsActive\":1,\"IsBroken\":0"));
    unsafe {
        let (hr, text) = call(detach_process_handler, 0, Some(&format!("{{\"ProcessId\":{pid},\"ForceAll\":1}}")));
        assert_eq!((hr, text.as_str()), (S_OK, ""));
    }
    assert_eq!(run.handler_result(), S_OK);
    run.wait_query("worker exit", |p| p.contains("\"IsActive\":0"));
    assert!(run.child.try_wait().unwrap().is_none(), "child must survive the continue and detach");
}
