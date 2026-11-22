use crate::chk::take_recorded;
use crate::monitor::*;
use crate::profile::*;
use crate::server::*;
use crate::tests::{canon, operation_error};
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};
use xtfmsg::com::{self, Com};
use xtfmsg::fixture::{self, as_obj, end_json, end_message, load_xbtp, message_with_json};
use xtfmsg::sys::*;

pub const XDEFIANT_DLL: &str = r"D:\xbox-pc-port\games\XDefiant\extracted\EmbeddedXvd\Windows\System32\xtfdebugmonitor.dll";
pub const FORTNITE_DLL: &str = r"D:\xbox-pc-port\games\Fortnite\extracted\EmbeddedXvd\Windows\System32\xtfdebugmonitor.dll";

type Inner = unsafe fn(&'static Profile, usize, usize, usize) -> HRESULT;

unsafe fn call_p(f: Inner, p: &'static Profile, session: usize, request: Option<&str>) -> (HRESULT, String) {
    let req = match request {
        Some(j) => message_with_json(j),
        None => Com::null(),
    };
    let reply = end_message();
    let hr = f(p, session, req.get() as usize, reply.get() as usize);
    (hr, end_json(reply.get()).unwrap_or_default())
}

fn console_chk_sites(path: &str) -> std::collections::BTreeSet<(String, u32)> {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let mut out = std::collections::BTreeSet::new();
    let pat = b"XtfCHK: ";
    let mut i = 0;
    while let Some(off) = bytes[i..].windows(pat.len()).position(|w| w == pat) {
        let start = i + off + pat.len();
        let end = start + bytes[start..].iter().position(|&b| b == 0).unwrap_or(0);
        let s = String::from_utf8_lossy(&bytes[start..end]).into_owned();
        if !s.contains('\\') {
            if let Some((func, line)) = s.rsplit_once(": ") {
                if let Ok(n) = line.parse::<u32>() {
                    out.insert((func.to_string(), n));
                }
            }
        }
        i = end.max(start);
    }
    out
}

fn our_server_sites(p: &Profile) -> Vec<(&'static str, u32)> {
    let mut v = Vec::new();
    for l in [166, 167, 182, 183, 185, 186, 187, 194, 196, 197, 199, 201, 203, 207, 208] {
        v.push(("QueryMonitorHandler", p.query(l)));
    }
    for l in [233, 234, 255, 268] {
        v.push(("MonitorProcessHandler", p.monitor(l)));
    }
    for l in [300, 301] {
        v.push(("DetachProcessHandler", p.detach(l)));
    }
    for l in [345, 346, 347] {
        v.push(("ContinueProcessHandler", p.cont(l)));
    }
    for l in [151, 174] {
        v.push(("ProcessDebugEventMonitor::AddDebugClient", p.add_client(l)));
    }
    for l in [387, 402, 429, 433, 437, 441, 445, 449, 453, 457, 461, 479] {
        v.push(("ProcessDebugEventMonitor::WorkerThread", p.pdem(l)));
    }
    v.push(("ProcessDebugEventMonitor::OnCreateProcessDebugEvent", p.pdem(594)));
    v.push(("ProcessDebugEventMonitor::OnLoadDllDebugEvent", p.pdem(795)));
    v.push(("ProcessDebugEventMonitor::OnOutputDebugStringEvent", p.pdem(836)));
    match p.flow {
        MonitorFlow::AddMonitorClient => {
            for l in [85, 86, 88, 96, 104] {
                v.push(("DebugMonitor::AddMonitorClient", l));
            }
            v.push(("DebugMonitor::MonitorProcess", 173));
            v.push(("DebugMonitor::MonitorProcess", 176));
        }
        MonitorFlow::AddProcessMonitor => {
            for l in [114, 122, 123, 127, 131, 135] {
                v.push(("DebugMonitor::MonitorProcess", l));
            }
            v.push(("DebugMonitor::AddProcessMonitor", 60));
            v.push(("DebugMonitor::RemoveProcessMonitor", 82));
        }
    }
    v
}

#[test]
fn profile_selects_19041_layout_below_22000() {
    assert!(core::ptr::eq(for_build(19041), &P19041));
    assert!(core::ptr::eq(for_build(21999), &P19041));
    assert!(core::ptr::eq(for_build(22000), &P22000));
    assert!(core::ptr::eq(for_build(22621), &P22000));
    assert_eq!((P19041.extension_version, P22000.extension_version), ("200804", "220604"));
    assert!(!P19041.activities && P22000.activities);
    assert!(!P19041.open_retry && P22000.open_retry);
    assert_eq!((P19041.split_line, P19041.open_line, P22000.split_line, P22000.open_line), (0xa4, 0xc6, 0xb2, 0x105));
    let _g = UseProfile::on(&P19041);
    assert!(core::ptr::eq(current(), &P19041));
    drop(_g);
    assert!(core::ptr::eq(current(), &P22000));
}

#[test]
fn every_server_chk_site_exists_in_the_matching_console_copy() {
    for (p, own, other) in [(&P22000, XDEFIANT_DLL, FORTNITE_DLL), (&P19041, FORTNITE_DLL, XDEFIANT_DLL)] {
        let own_sites = console_chk_sites(own);
        let other_sites = console_chk_sites(other);
        assert!(own_sites.len() > 100, "{own}: {}", own_sites.len());
        let ours = our_server_sites(p);
        for (f, l) in &ours {
            assert!(own_sites.contains(&(f.to_string(), *l)), "{} build: XtfCHK: {f}: {l} is not in {own}", p.os_build);
        }
        let differs = ours.iter().filter(|(f, l)| !other_sites.contains(&(f.to_string(), *l))).count();
        assert!(differs >= 40, "{} build: only {differs} sites differ from {other}", p.os_build);
        let server_funcs = ["QueryMonitorHandler", "MonitorProcessHandler", "DetachProcessHandler", "ContinueProcessHandler", "DebugMonitor::MonitorProcess", "DebugMonitor::AddMonitorClient", "DebugMonitor::AddProcessMonitor", "DebugMonitor::RemoveProcessMonitor"];
        for (f, l) in own_sites.iter().filter(|(f, _)| server_funcs.contains(&f.as_str())) {
            let skip = (f == "QueryMonitorHandler" && *l == p.query(174)) || (f == "DebugMonitor::AddMonitorClient" && *l == 77);
            assert!(skip || ours.iter().any(|(g, m)| g == f && m == l), "{} build: {own} has XtfCHK: {f}: {l} which this port never emits", p.os_build);
        }
    }
}

#[test]
fn server_error_lines_follow_the_emulated_build() {
    load_xbtp();
    for (p, mon, det, cont) in [(&P22000, [233, 234, 268], [300, 301], [345, 346, 347]), (&P19041, [231, 232, 266], [297, 298], [341, 342, 343])] {
        unsafe {
            let (hr, text) = call_p(monitor_process, p, 1, None);
            assert_eq!(hr, E_INVALIDARG);
            assert_eq!(canon(&text), canon(&format!("{{{}}}", operation_error(E_INVALIDARG, mon[0]))), "{}", p.os_build);
            let (hr, text) = call_p(monitor_process, p, 1, Some("{}"));
            assert_eq!(hr, E_JSON_NOT_FOUND);
            assert_eq!(canon(&text), canon(&format!("{{{}}}", operation_error(E_JSON_NOT_FOUND, mon[1]))), "{}", p.os_build);
            let (hr, text) = call_p(monitor_process, p, 0, Some("{\"ProcessId\":4294967059}"));
            assert_eq!(hr, E_INVALIDARG);
            assert_eq!(canon(&text), canon(&format!("{{{}}}", operation_error(E_INVALIDARG, mon[2]))), "{}", p.os_build);
            let (hr, text) = call_p(detach_process, p, 0, None);
            assert_eq!(hr, E_INVALIDARG);
            assert_eq!(canon(&text), canon(&format!("{{{}}}", operation_error(E_INVALIDARG, det[0]))), "{}", p.os_build);
            let (hr, text) = call_p(detach_process, p, 0, Some("{\"ForceAll\":1}"));
            assert_eq!(hr, E_JSON_NOT_FOUND);
            assert_eq!(canon(&text), canon(&format!("{{{}}}", operation_error(E_JSON_NOT_FOUND, det[1]))), "{}", p.os_build);
            let (hr, text) = call_p(continue_process, p, 0, None);
            assert_eq!(hr, E_INVALIDARG);
            assert_eq!(canon(&text), canon(&format!("{{{}}}", operation_error(E_INVALIDARG, cont[0]))), "{}", p.os_build);
            let (hr, text) = call_p(continue_process, p, 0, Some("{\"ExceptionContinue\":65538}"));
            assert_eq!(hr, E_JSON_NOT_FOUND);
            assert_eq!(canon(&text), canon(&format!("{{{}}}", operation_error(E_JSON_NOT_FOUND, cont[1]))), "{}", p.os_build);
            let (hr, text) = call_p(continue_process, p, 0, Some("{\"ProcessId\":4294967059}"));
            assert_eq!(hr, E_JSON_NOT_FOUND);
            assert_eq!(canon(&text), canon(&format!("{{{}}}", operation_error(E_JSON_NOT_FOUND, cont[2]))), "{}", p.os_build);
            let (hr, text) = call_p(query_monitor, p, 0, None);
            assert_eq!(hr, S_OK);
            assert!(text.starts_with("{\"Processes\":["), "{text}");
        }
    }
}

fn dm_sites(recs: &[(String, u32, String, HRESULT)]) -> Vec<(String, u32, HRESULT)> {
    recs.iter().filter(|r| r.0.starts_with("DebugMonitor::") || r.0 == "ProcessDebugEventMonitor::AddDebugClient").map(|r| (r.0.clone(), r.1, r.3)).collect()
}

fn s(f: &str, l: u32) -> (String, u32, HRESULT) {
    (f.to_string(), l, S_OK)
}

#[test]
fn monitor_process_flow_replaces_an_inactive_monitor_per_build() {
    load_xbtp();
    let cases: [(&'static Profile, u32, Vec<(String, u32, HRESULT)>); 2] = [
        (
            &P22000,
            0xFFFF_FF21,
            vec![
                s("DebugMonitor::AddMonitorClient", 85),
                s("DebugMonitor::AddMonitorClient", 86),
                s("DebugMonitor::AddMonitorClient", 88),
                s("ProcessDebugEventMonitor::AddDebugClient", 151),
                s("ProcessDebugEventMonitor::AddDebugClient", 174),
                s("DebugMonitor::AddMonitorClient", 96),
                s("DebugMonitor::AddMonitorClient", 104),
                s("DebugMonitor::MonitorProcess", 173),
                s("DebugMonitor::MonitorProcess", 176),
            ],
        ),
        (
            &P19041,
            0xFFFF_FF25,
            vec![
                s("DebugMonitor::MonitorProcess", 114),
                s("DebugMonitor::MonitorProcess", 122),
                s("DebugMonitor::AddProcessMonitor", 60),
                s("DebugMonitor::MonitorProcess", 123),
                s("ProcessDebugEventMonitor::AddDebugClient", 141),
                s("ProcessDebugEventMonitor::AddDebugClient", 164),
                s("DebugMonitor::MonitorProcess", 127),
                s("DebugMonitor::MonitorProcess", 131),
                s("DebugMonitor::MonitorProcess", 135),
            ],
        ),
    ];
    for (p, pid, want) in cases {
        let dm = instance().unwrap();
        let old = Arc::new(ProcessMonitor::with_profile(pid, p));
        old.active.store(false, Ordering::SeqCst);
        lock(&dm.monitors).push(old.clone());
        let session = unsafe { fixture::session() };
        let sa = session as usize;
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _g = UseProfile::on(p);
            take_recorded();
            let r = unsafe { call_p(monitor_process, p, sa, Some(&format!("{{\"ProcessId\":{pid}}}"))) };
            let _ = tx.send((r, take_recorded()));
        });
        let ((hr, text), recs) = rx.recv_timeout(Duration::from_secs(20)).expect("MonitorProcess on an unattachable pid did not return");
        assert_eq!((hr, text.as_str()), (S_OK, ""), "{} build", p.os_build);
        assert_eq!(dm_sites(&recs), want, "{} build", p.os_build);
        let now = dm.find(pid).expect("replacement monitor");
        assert!(!Arc::ptr_eq(&now, &old), "{} build kept the inactive monitor", p.os_build);
        assert!(core::ptr::eq(now.profile, p));
        let deadline = Instant::now() + Duration::from_secs(20);
        while now.active.load(Ordering::SeqCst) {
            assert!(Instant::now() < deadline, "worker for pid {pid} did not stop");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(now.attach_result.load(Ordering::SeqCst) < 0);
        let sent = unsafe { lock(&(*session).sent).clone() };
        assert_eq!(sent.len(), 1, "{sent:?}");
        assert!(sent[0].contains("\"AttachResult\""), "{sent:?}");
        assert_eq!(lock(&dm.monitors).iter().filter(|m| m.pid == pid).count(), 1);
        lock(&dm.monitors).retain(|m| m.pid != pid);
        unsafe { com::release(as_obj(session)) };
    }
}

#[test]
fn remove_process_monitor_reports_e_not_set_at_82() {
    let dm = instance().unwrap();
    let m = Arc::new(ProcessMonitor::with_profile(0xFFFF_FF29, &P19041));
    take_recorded();
    assert_eq!(dm.remove_process_monitor(&m), crate::win::E_NOT_SET);
    let recs = take_recorded();
    assert_eq!(recs.len(), 1);
    assert_eq!((recs[0].0.as_str(), recs[0].1, recs[0].3), ("DebugMonitor::RemoveProcessMonitor", 82, crate::win::E_NOT_SET));
    assert_eq!(dm.add_process_monitor(&m), S_OK);
    assert_eq!(dm.remove_process_monitor(&m), S_OK);
    assert!(dm.find(0xFFFF_FF29).is_none());
}

#[test]
fn open_session_retries_only_on_the_22000_layout() {
    load_xbtp();
    unsafe {
        let conn = wide("10.255.0.7");
        let mut out: *mut core::ffi::c_void = core::ptr::null_mut();
        let t = Instant::now();
        let hr = crate::client::open_session_impl(&P19041, conn.as_ptr(), crate::client::XBTP_PORT_TITLE, &crate::DEBUG_MONITOR_EXTENSION_ID, &com::IID_XBTP_CLIENT_SESSION, &mut out);
        let spent = t.elapsed();
        assert_eq!(hr, crate::client::E_COULD_NOT_CONNECT);
        assert!(out.is_null());
        assert!(spent < Duration::from_secs(5), "19041 XtfOpenSessionImpl has no retry loop, took {spent:?}");
    }
}
