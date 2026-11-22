use crate::config::*;
use crate::op::*;
use crate::process::*;
use crate::screenshot::*;
use crate::system::*;
use crate::*;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Mutex, Once};
use xtfmsg::com::{self, metadata, Com, Obj, IID_XBTP_EXTENSION_METADATA};
use xtfmsg::fixture::{end_json, end_message, load_xbtp, message_with_json};
use xtfmsg::sys::*;

static INIT: Once = Once::new();
static SERIAL: Mutex<()> = Mutex::new(());

fn root() -> PathBuf {
    std::env::temp_dir().join(format!("xtfconsolecontrol-test-{}", std::process::id()))
}

pub(crate) fn guard() -> std::sync::MutexGuard<'static, ()> {
    INIT.call_once(|| {
        let r = root();
        let _ = std::fs::remove_dir_all(&r);
        std::fs::create_dir_all(r.join("console").join("fs").join("C").join("Windows").join("System32")).unwrap();
        std::env::set_var("TOMBOX_GDK_ROOT", &r);
        load_xbtp();
    });
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

type Handler = unsafe extern "system" fn(usize, usize, usize) -> HRESULT;

unsafe fn call(f: Handler, request: Option<&str>) -> (HRESULT, String) {
    let req = match request {
        Some(j) => message_with_json(j),
        None => Com::null(),
    };
    let reply = end_message();
    let hr = f(0, req.get() as usize, reply.get() as usize);
    let text = end_json(reply.get()).unwrap_or_default();
    (hr, text)
}

fn operation_error(file: &str, hr: HRESULT, line: u32) -> J {
    let mut m = BTreeMap::new();
    m.insert("ReturnCode".to_string(), J::Num(hr as f64));
    m.insert("Checkpoint".to_string(), J::Num(0.0));
    m.insert("Errors".to_string(), J::Arr(vec![J::Str(format!("Error 0x{:X} File xbox\\devex\\xtf\\dll\\console\\{} : {}", hr as u32, file, line))]));
    J::Obj(m)
}

#[derive(Clone, Debug, PartialEq)]
enum J {
    Null,
    Num(f64),
    Str(String),
    Arr(Vec<J>),
    Obj(BTreeMap<String, J>),
}

impl J {
    fn get(&self, k: &str) -> &J {
        match self {
            J::Obj(m) => m.get(k).unwrap_or_else(|| panic!("missing {k} in {self:?}")),
            _ => panic!("not an object: {self:?}"),
        }
    }

    fn has(&self, k: &str) -> bool {
        matches!(self, J::Obj(m) if m.contains_key(k))
    }

    fn keys(&self) -> Vec<String> {
        match self {
            J::Obj(m) => m.keys().cloned().collect(),
            _ => vec![],
        }
    }

    fn num(&self) -> f64 {
        match self {
            J::Num(n) => *n,
            _ => panic!("not a number: {self:?}"),
        }
    }

    fn str(&self) -> &str {
        match self {
            J::Str(s) => s,
            _ => panic!("not a string: {self:?}"),
        }
    }

    fn arr(&self) -> &[J] {
        match self {
            J::Arr(v) => v,
            _ => panic!("not an array: {self:?}"),
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
                let mut m = BTreeMap::new();
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

fn result_code(j: &J) -> HRESULT {
    j.get("Result").get("ResultCode").num() as i32
}

#[test]
fn export_table_matches_console_dll() {
    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/exports.txt")).unwrap();
    xtfcommon::exports::check_against_titles("xtfconsolecontrol.dll", &text);
    for e in xtfcommon::exports::parse(&text) {
        if e.status != "missing" {
            assert!(xtfcommon::exports::source_defines(concat!(env!("CARGO_MANIFEST_DIR"), "/src"), &e), "{}", e.name);
        }
    }
}

#[test]
fn server_entry_point_checks_ids_and_registers_thirteen_handlers() {
    let _g = guard();
    unsafe {
        let bad = GUID::new(1, 2, 3, [4; 8]);
        let sentinel = 0x1234usize as Obj;
        let mut out = sentinel;
        assert_eq!(XtfOrdinal_4(&bad, &IID_XBTP_EXTENSION_METADATA, &mut out), E_INVALIDARG);
        assert_eq!(out, sentinel);
        assert_eq!(XtfOrdinal_4(&CONSOLE_EXTENSION_ID, &bad, &mut out), E_INVALIDARG);
        assert_eq!(out, sentinel);
        let mut md: Obj = core::ptr::null_mut();
        assert_eq!(XtfOrdinal_4(&CONSOLE_EXTENSION_ID, &IID_XBTP_EXTENSION_METADATA, &mut md), S_OK);
        assert!(!md.is_null());
        let mut n = 0u32;
        assert_eq!(metadata::get_handler_count(md, &mut n), S_OK);
        assert_eq!(n, 13);
        com::release(md);
    }
    assert_eq!(HANDLERS.iter().map(|h| h.0).collect::<Vec<_>>(), [2, 3, 7, 8, 1, 6, 5, 4, 9, 10, 11, 17, 16]);
    assert_eq!(HANDLERS.iter().map(|h| h.3).collect::<Vec<_>>(), [28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 51]);
}

#[test]
fn settings_table_sorts_case_insensitively_by_category_then_name() {
    let l = sorted_list();
    assert_eq!(l.len(), 78);
    for p in l.windows(2) {
        let o = wcsicmp(p[0].category, p[1].category).then_with(|| wcsicmp(p[0].name, p[1].name));
        assert_eq!(o, std::cmp::Ordering::Less, "{:?} {:?}", p[0], p[1]);
    }
    let debug: Vec<&str> = l.iter().filter(|s| s.category == "Debug").map(|s| s.name).take(5).collect();
    assert_eq!(debug, ["consolemode", "CrashDumpType", "CrashDumpUpload", "debugnic", "EnableEventLimitToast"]);
    assert_eq!(l[0].name, "AudioBitstreamFormat");
    assert_eq!(l[77].name, "HeadlessXUser");
    assert_eq!(PREFERRED_LANGUAGES.len(), 54);
    let hidden: Vec<&str> = SETTINGS.iter().filter(|s| s.hidden).map(|s| s.name).collect();
    assert_eq!(hidden, ["FastIterationMode", "LogLevel", "pixfasttransportmode", "EnableKinectStudio", "sragraphicsdrivermode", "disableforceguest", "EnableInternalSSD", "EnableUnifiedScratch", "accessoryflags", "environment", "oobecompleted", "VideoServer"]);
    assert_eq!(SETTINGS.iter().filter(|s| s.reboot_required).count(), 35);
}

#[test]
fn get_config_value_replies_empty_value_and_successful() {
    let _g = guard();
    unsafe {
        let (hr, text) = call(get_config_value_handler, Some(r#"{"ConfigKey":"hostname"}"#));
        assert_eq!(hr, S_OK);
        let j = parse(&text);
        assert_eq!(j.keys(), ["ConfigValue", "Successful"]);
        assert_eq!(j.get("ConfigValue").str(), "");
        assert_eq!(j.get("Successful").num(), 1.0);
    }
}

#[test]
fn get_config_value_missing_key_fails_at_1452_without_reply_body() {
    let _g = guard();
    unsafe {
        let (hr, text) = call(get_config_value_handler, Some("{}"));
        assert_eq!(hr, E_JSON_NOT_FOUND);
        let j = parse(&text);
        assert_eq!(j.keys(), ["XtfOperationError"]);
        assert_eq!(*j.get("XtfOperationError"), operation_error("config.cpp", E_JSON_NOT_FOUND, 1452));
        let (hr, _) = call(get_config_value_handler, None);
        assert!(hr < 0);
    }
}

#[test]
fn set_config_value_reads_key_and_value_then_completes() {
    let _g = guard();
    unsafe {
        let (hr, text) = call(set_config_value_handler, Some(r#"{"ConfigKey":"hostname","ConfigValue":"devkit"}"#));
        assert_eq!(hr, S_OK);
        assert_eq!(parse(&text).keys(), ["Successful"]);
        assert_eq!(parse(&text).get("Successful").num(), 1.0);
        let (hr, text) = call(set_config_value_handler, Some(r#"{"ConfigKey":"hostname"}"#));
        assert_eq!(hr, E_JSON_NOT_FOUND);
        assert_eq!(*parse(&text).get("XtfOperationError"), operation_error("config.cpp", E_JSON_NOT_FOUND, 1489));
        let (hr, text) = call(get_config_value_handler, Some(r#"{"ConfigKey":"hostname"}"#));
        assert_eq!(hr, S_OK);
        assert_eq!(parse(&text).get("ConfigValue").str(), "");
    }
}

#[test]
fn get_all_settings_lists_sorted_names_with_empty_values() {
    let _g = guard();
    unsafe {
        let (hr, text) = call(get_all_settings_handler, None);
        assert_eq!(hr, S_OK);
        let j = parse(&text);
        assert_eq!(j.keys(), ["SettingsArrayObject", "Successful"]);
        let all = j.get("SettingsArrayObject").get("AllSettings").arr();
        assert_eq!(all.len(), 78);
        for (e, s) in all.iter().zip(sorted_list()) {
            assert_eq!(e.keys(), ["SettingName", "SettingValue"]);
            assert_eq!(e.get("SettingName").str(), s.name);
            assert_eq!(e.get("SettingValue").str(), "");
        }
    }
}

#[test]
fn get_settings_metadata_reports_category_flags_and_joined_ranges() {
    let _g = guard();
    unsafe {
        let (hr, text) = call(get_settings_metadata_handler, None);
        assert_eq!(hr, S_OK);
        let j = parse(&text);
        assert_eq!(j.get("Successful").num(), 1.0);
        let all = j.get("SettingsMetadataArrayObject").get("SettingsMetadata").arr();
        assert_eq!(all.len(), 78);
        let find = |n: &str| all.iter().find(|e| e.get("SettingName").str() == n).unwrap().get("Metadata").arr().to_vec();
        let md = |v: &[J]| v.iter().map(|m| (m.get("Name").str().to_string(), m.get("Value").str().to_string())).collect::<Vec<_>>();
        assert_eq!(md(&find("LogLevel")), [("Category".into(), "Debug".into()), ("RebootRequired".into(), "false".into()), ("Hidden".into(), "true".into()), ("Range".into(), "Quiet;Detailed;Verbose".to_string())]);
        assert_eq!(md(&find("ToolsLogLevel")), [("Category".to_string(), "Debug".to_string()), ("RebootRequired".into(), "false".into()), ("Hidden".into(), "false".into())]);
        assert_eq!(md(&find("hostname")).len(), 3);
        assert_eq!(md(&find("hostname"))[1].1, "true");
        let langs = md(&find("preferredlanguages"));
        assert_eq!(langs[3].1, PREFERRED_LANGUAGES.join(";"));
        assert!(langs[3].1.starts_with("ar-SA;ar-AE;ar-SA;cs-CZ"));
        for e in all {
            assert_eq!(e.get("SettingValue").str(), "");
        }
    }
}

#[test]
fn get_settings_ex_reports_string_flags_and_range_arrays() {
    let _g = guard();
    unsafe {
        let (hr, text) = call(get_settings_ex_handler, None);
        assert_eq!(hr, S_OK);
        let j = parse(&text);
        assert_eq!(j.keys(), ["Settings", "Successful"]);
        let all = j.get("Settings").arr();
        assert_eq!(all.len(), 78);
        let hs = all.iter().find(|e| e.get("Name").str() == "HeadsetFormat").unwrap();
        assert_eq!(hs.keys(), ["Category", "Hidden", "Name", "Range", "RebootRequired", "Value"]);
        assert_eq!(hs.get("Range").arr().iter().map(|v| v.str().to_string()).collect::<Vec<_>>(), ["Stereo", "WindowsSonicForHeadphones", "DolbyAtmosForHeadphones", "DtsHeadphoneX"]);
        let tl = all.iter().find(|e| e.get("Name").str() == "ToolsLogLevel").unwrap();
        assert_eq!(tl.get("Range").arr().len(), 3);
        let ks = all.iter().find(|e| e.get("Name").str() == "EnableKinectStudio").unwrap();
        assert!(!ks.has("Range"));
        assert_eq!(ks.get("RebootRequired").str(), "true");
        assert_eq!(ks.get("Hidden").str(), "true");
        assert_eq!(ks.get("Category").str(), "Legacy");
        assert_eq!(ks.get("Value").str(), "");
    }
}

#[test]
fn shutdown_enables_shutdown_privilege_then_fails_notimpl_at_280() {
    let _g = guard();
    unsafe {
        let (hr, text) = call(shutdown_handler, Some(r#"{"Reboot":1,"PowerModeAware":1}"#));
        assert_eq!(hr, E_NOTIMPL);
        let j = parse(&text);
        assert_eq!(j.keys(), ["XtfOperationError"]);
        assert_eq!(*j.get("XtfOperationError"), operation_error("shutdown.cpp", E_NOTIMPL, 280));
        assert!(crate::privilege::is_enabled(crate::privilege::SE_SHUTDOWN_NAME));
        let (hr, text) = call(shutdown_handler, Some(r#"{"Reboot":0}"#));
        assert_eq!(hr, E_NOTIMPL);
        assert_eq!(*parse(&text).get("XtfOperationError"), operation_error("shutdown.cpp", E_NOTIMPL, 280));
    }
}

#[test]
fn shutdown_request_errors_follow_console_paths() {
    let _g = guard();
    unsafe {
        let (hr, text) = call(shutdown_handler, Some(r#"{"Reboot":1,"PowerOnDelayInMinutes":5}"#));
        assert_eq!(hr, E_INVALIDARG);
        assert_eq!(text, "");
        let (hr, text) = call(shutdown_handler, Some("{}"));
        assert_eq!(hr, E_JSON_NOT_FOUND);
        assert_eq!(*parse(&text).get("XtfOperationError"), operation_error("shutdown.cpp", E_JSON_NOT_FOUND, 225));
        let (hr, text) = call(shutdown_handler, Some(r#"{"Reboot":1,"PowerModeAware":"x"}"#));
        assert!(hr < 0 && hr != E_NOTIMPL, "{hr:x}");
        assert_eq!(text, "");
    }
}

#[test]
fn privilege_model_tracks_enable_and_restore_state() {
    use crate::privilege::*;
    let mut restore = true;
    assert_eq!(set_privilege("SeTimeZonePrivilege", true, Some(&mut restore)), S_OK);
    assert!(!restore);
    assert!(is_enabled("SeTimeZonePrivilege"));
    assert_eq!(set_privilege("SeTimeZonePrivilege", false, Some(&mut restore)), S_OK);
    assert!(restore);
    assert!(!is_enabled("SeTimeZonePrivilege"));
    assert_eq!(set_privilege("SeNoSuchPrivilege", true, None), hr_from_win32(ERROR_NO_SUCH_PRIVILEGE));
}

#[test]
fn get_system_time_reports_console_filetime_as_low_high_pair() {
    let _g = guard();
    unsafe {
        let before = console_time();
        let (hr, text) = call(get_system_time_handler, None);
        let after = console_time();
        assert_eq!(hr, S_OK);
        let j = parse(&text);
        assert_eq!(j.keys(), ["ConsoleTime"]);
        let t = j.get("ConsoleTime");
        let v = ((t.get("H").num() as u64) << 32) | t.get("L").num() as u64;
        assert!(before <= v && v <= after, "{before} {v} {after}");
        assert_eq!(v % 10_000, 0);
    }
}

fn session_manager_path() -> String {
    format!("\\registry\\machine\\{SESSION_MANAGER_KEY}")
}

fn multi_sz(items: &[&str]) -> Vec<u8> {
    let mut v: Vec<u16> = Vec::new();
    for i in items {
        v.extend(i.encode_utf16());
        v.push(0);
    }
    v.push(0);
    v.iter().flat_map(|c| c.to_le_bytes()).collect()
}

#[test]
fn updates_pending_reads_pending_file_renames_from_sandbox_registry() {
    let _g = guard();
    use console_sandbox::registry;
    unsafe {
        let p = session_manager_path();
        registry::create_key(&p);
        assert!(!are_updates_pending());
        registry::set(&p, PENDING_FILE_RENAME_OPERATIONS, registry::REG_MULTI_SZ, &multi_sz(&["\\??\\C:\\a", ""]));
        assert!(are_updates_pending());
        let (hr, text) = call(updates_pending_handler, None);
        assert_eq!(hr, S_OK);
        assert_eq!(parse(&text).get("UpdatesPending").num(), 1.0);
        registry::set(&p, PENDING_FILE_RENAME_OPERATIONS, registry::REG_MULTI_SZ, &multi_sz(&[]));
        assert!(!are_updates_pending());
        registry::set(&p, PENDING_FILE_RENAME_OPERATIONS, registry::REG_SZ, &multi_sz(&["x"]));
        assert!(!are_updates_pending());
        registry::set(&p, PENDING_FILE_RENAME_OPERATIONS, registry::REG_MULTI_SZ, &[0x41, 0x00]);
        assert!(!are_updates_pending());
        registry::set(&p, PENDING_FILE_RENAME_OPERATIONS, registry::REG_MULTI_SZ, &[0x41, 0x00, 0x00, 0x00]);
        assert!(are_updates_pending());
        registry::set(&p, PENDING_FILE_RENAME_OPERATIONS, registry::REG_MULTI_SZ, &[]);
        let (hr, text) = call(updates_pending_handler, None);
        assert_eq!(hr, S_OK);
        assert_eq!(parse(&text).get("UpdatesPending").num(), 0.0);
    }
}

#[test]
fn in_connected_standby_reports_three_false_flags() {
    let _g = guard();
    unsafe {
        let (hr, text) = call(in_connected_standby_handler, None);
        assert_eq!(hr, S_OK);
        let j = parse(&text);
        assert_eq!(j.keys(), ["AllowXboxAppConnections", "AllowsInstantOn", "InConnectedStandby"]);
        for k in j.keys() {
            assert_eq!(j.get(&k).num(), 0.0);
        }
    }
}

#[test]
fn running_processes_lists_only_the_hosted_title_process() {
    let _g = guard();
    unsafe {
        let (hr, text) = call(get_running_processes_handler, None);
        assert_eq!(hr, S_OK);
        let j = parse(&text);
        let ps = j.get("Processes").arr();
        assert_eq!(ps.len(), 1);
        assert_eq!(ps[0].get("ProcessId").num() as u32, std::process::id());
        let exe = std::env::current_exe().unwrap();
        match console_path_of_host(&exe) {
            Some(c) => assert_eq!(ps[0].get("ProcessName").str(), c),
            None => assert!(!ps[0].has("ProcessName")),
        }
        if ps[0].has("ProcessName") {
            assert!(!ps[0].get("ProcessName").str().contains(&*root().to_string_lossy()));
        }
    }
}

#[test]
fn host_paths_map_to_console_paths_or_nothing() {
    let _g = guard();
    let fs = console_sandbox::fs::fs_root();
    assert_eq!(console_path_of_host(&fs.join("d").join("x").join("t.exe")).as_deref(), Some("D:\\x\\t.exe"));
    assert_eq!(console_path_of_host(&fs.join("T")).as_deref(), Some("T:\\"));
    assert_eq!(console_path_of_host(&std::env::temp_dir().join("elsewhere.exe").join("..").join("no")), None);
    assert_eq!(query_full_process_image_name(std::process::id().wrapping_add(4)), None);
    assert_eq!(full_path_name("g:"), "G:\\");
    assert_eq!(full_path_name("g:/a/b.exe"), "G:\\a\\b.exe");
}

#[test]
fn wtoi_and_dump_names_follow_crt_and_strsafe() {
    assert_eq!(wtoi("  42abc"), 42);
    assert_eq!(wtoi("-7"), -7);
    assert_eq!(wtoi("+9"), 9);
    assert_eq!(wtoi("x1"), 0);
    assert_eq!(wtoi("99999999999"), i32::MAX);
    assert_eq!(wtoi("-99999999999"), i32::MIN);
    assert_eq!(dump_kind(0), Some(DUMP_TRIAGE));
    assert_eq!(dump_kind(1), Some(DUMP_MINI));
    assert_eq!(dump_kind(2), Some(DUMP_HEAP));
    assert_eq!(dump_kind(3), None);
    assert_eq!(dump_file_name(Some("title"), (2026, 11, 21, 9, 5), "mini").unwrap(), "D:\\title_2026-11-21-9-5-mini.dmp");
    assert_eq!(dump_file_name(None, (2026, 1, 2, 3, 4), "heap").unwrap(), "D:\\(null)_2026-1-2-3-4-heap.dmp");
    assert_eq!(dump_file_name(Some(&"a".repeat(300)), (1, 1, 1, 1, 1), "mini"), Err(STRSAFE_E_INSUFFICIENT_BUFFER));
}

pub(crate) fn dump_dir() -> PathBuf {
    console_sandbox::fs::drive_dir('D')
}

#[test]
fn generate_stack_dump_writes_minidump_of_hosted_process_to_sandbox_d() {
    let _g = guard();
    unsafe {
        let req = format!(r#"{{"ProcessId":"{}","DumpType":"1","ImageName":"xtfcctest"}}"#, std::process::id());
        let (hr, text) = call(generate_stack_dump_handler, Some(&req));
        assert_eq!(hr, S_OK, "{text}");
        let j = parse(&text);
        assert_eq!(j.keys(), ["FilePath", "Successful"]);
        assert_eq!(j.get("Successful").num(), 1.0);
        let path = j.get("FilePath").str().to_string();
        assert!(path.starts_with("D:\\xtfcctest_") && path.ends_with("-mini.dmp"), "{path}");
        let host = host_of_console(&path).unwrap();
        assert!(host.starts_with(dump_dir()));
        let data = std::fs::read(&host).unwrap();
        assert_eq!(&data[..4], b"MDMP");
        std::fs::remove_file(&host).unwrap();
    }
}

#[test]
fn generate_stack_dump_rejects_bad_requests_at_console_lines() {
    let _g = guard();
    unsafe {
        let (hr, text) = call(generate_stack_dump_handler, Some(r#"{"ProcessId":"5"}"#));
        assert_eq!(hr, E_JSON_NOT_FOUND);
        assert_eq!(*parse(&text).get("XtfOperationError"), operation_error("crashdump.cpp", E_JSON_NOT_FOUND, 554));
        let (hr, text) = call(generate_stack_dump_handler, Some(r#"{"ProcessId":"0","DumpType":"1","ImageName":"a"}"#));
        assert_eq!(hr, E_INVALIDARG);
        assert_eq!(*parse(&text).get("XtfOperationError"), operation_error("crashdump.cpp", E_INVALIDARG, 567));
        let (hr, text) = call(generate_stack_dump_handler, Some(r#"{"ProcessId":"5","DumpType":"3","ImageName":"a"}"#));
        assert_eq!(hr, E_INVALIDARG);
        assert_eq!(*parse(&text).get("XtfOperationError"), operation_error("crashdump.cpp", E_INVALIDARG, 610));
    }
}

#[test]
fn generate_stack_dump_of_unhosted_pid_fails_open_process_at_644() {
    let _g = guard();
    unsafe {
        let other = std::process::id().wrapping_add(4);
        let req = format!(r#"{{"ProcessId":"{other}","DumpType":"0","ImageName":"other"}}"#);
        let (hr, text) = call(generate_stack_dump_handler, Some(&req));
        assert_eq!(hr, S_OK, "{text}");
        let j = parse(&text);
        assert_eq!(j.keys(), ["Result", "Successful", "XtfOperationError"]);
        assert_eq!(j.get("Successful").num(), 0.0);
        let e = hr_from_win32(ERROR_INVALID_PARAMETER);
        assert_eq!(result_code(&j), e);
        assert_eq!(*j.get("XtfOperationError"), operation_error("crashdump.cpp", e, 644));
        let left: Vec<_> = std::fs::read_dir(dump_dir()).unwrap().filter_map(|e| e.ok()).filter(|e| e.file_name().to_string_lossy().starts_with("other_")).collect();
        assert_eq!(left.len(), 1);
        assert_eq!(std::fs::metadata(left[0].path()).unwrap().len(), 0);
        std::fs::remove_file(left[0].path()).unwrap();
    }
}

fn source_file() -> PathBuf {
    host_of_console(SCREENSHOT_SOURCE_FILE).unwrap()
}

#[test]
fn screenshot_marker_round_trips_through_sandbox_t_drive() {
    let _g = guard();
    unsafe {
        let _ = std::fs::remove_file(source_file());
        let (hr, text) = call(is_video_impacted_by_screenshots_handler, Some(r#"{"DurationInSeconds":60}"#));
        assert_eq!(hr, S_OK, "{text}");
        let j = parse(&text);
        assert_eq!(j.keys(), ["IsImpacted", "SecondsSinceLastScreenshot", "SourceNameInitiatingScreenshot", "Successful"]);
        assert_eq!(j.get("IsImpacted").num(), 0.0);
        assert_eq!(j.get("SecondsSinceLastScreenshot").num(), 0.0);
        assert_eq!(j.get("SourceNameInitiatingScreenshot").str(), "");
        let (hr, text) = call(mark_screenshot_taken_handler, Some(r#"{"SourceNameInitiatingScreenshot":"10.0.0.5"}"#));
        assert_eq!(hr, S_OK, "{text}");
        assert_eq!(parse(&text).keys(), ["Successful"]);
        assert_eq!(parse(&text).get("Successful").num(), 1.0);
        assert_eq!(std::fs::read(source_file()).unwrap(), b"10.0.0.5");
        assert!(source_file().starts_with(console_sandbox::fs::drive_dir('T')));
        let (hr, text) = call(is_video_impacted_by_screenshots_handler, Some(r#"{"DurationInSeconds":60}"#));
        assert_eq!(hr, S_OK, "{text}");
        let j = parse(&text);
        assert_eq!(j.get("IsImpacted").num(), 1.0);
        assert!(j.get("SecondsSinceLastScreenshot").num() <= 60.0);
        assert_eq!(j.get("SourceNameInitiatingScreenshot").str(), "10.0.0.5");
        assert_eq!(j.get("Successful").num(), 1.0);
        let (hr, text) = call(mark_screenshot_taken_handler, Some(r#"{"SourceNameInitiatingScreenshot":"a\nb"}"#));
        assert_eq!(hr, S_OK, "{text}");
        assert_eq!(std::fs::read(source_file()).unwrap(), b"a\r\nb");
        let (_, text) = call(is_video_impacted_by_screenshots_handler, Some(r#"{"DurationInSeconds":60}"#));
        assert_eq!(parse(&text).get("SourceNameInitiatingScreenshot").str(), "a\n");
        std::fs::remove_file(source_file()).unwrap();
    }
}

#[test]
fn screenshot_marker_failures_report_unsuccessful() {
    let _g = guard();
    unsafe {
        let (hr, _) = call(mark_screenshot_taken_handler, None);
        assert_eq!(hr, E_POINTER);
        let (hr, _) = call(is_video_impacted_by_screenshots_handler, None);
        assert_eq!(hr, E_POINTER);
        let (hr, text) = call(mark_screenshot_taken_handler, Some("{}"));
        assert_eq!(hr, S_OK);
        let j = parse(&text);
        assert_eq!(j.get("Successful").num(), 0.0);
        assert_eq!(result_code(&j), E_JSON_NOT_FOUND);
        assert_eq!(*j.get("XtfOperationError"), operation_error("videoimpactedbyscreenshot.cpp", E_JSON_NOT_FOUND, 30));
        for src in [""] {
            let req = format!(r#"{{"SourceNameInitiatingScreenshot":"{src}"}}"#);
            let (hr, text) = call(mark_screenshot_taken_handler, Some(&req));
            assert_eq!(hr, S_OK);
            let j = parse(&text);
            assert_eq!(j.keys(), ["Result", "Successful"], "{src:?}");
            assert_eq!(j.get("Successful").num(), 0.0);
            assert!(result_code(&j) < 0);
        }
        let (hr, text) = call(mark_screenshot_taken_handler, Some(r#"{"SourceNameInitiatingScreenshot":"中"}"#));
        assert_eq!(hr, S_OK);
        let j = parse(&text);
        assert_eq!(j.keys(), ["Successful"]);
        assert_eq!(j.get("Successful").num(), 1.0);
        assert_eq!(std::fs::read(source_file()).unwrap(), b"?");
        let (hr, text) = call(is_video_impacted_by_screenshots_handler, Some("{}"));
        assert_eq!(hr, S_OK);
        assert_eq!(*parse(&text).get("XtfOperationError"), operation_error("videoimpactedbyscreenshot.cpp", E_JSON_NOT_FOUND, 87));
        let _ = std::fs::remove_file(source_file());
    }
}

#[test]
fn generate_stack_dump_never_leaves_the_sandbox_d_drive() {
    let _g = guard();
    assert!(dump_host_path(r"D:\title_1-1-1-1-1-mini.dmp").unwrap().starts_with(dump_dir()));
    assert_eq!(dump_host_path(r"D:\a\C:\xtfcc_escape_1.dmp"), None);
    assert_eq!(dump_host_path(r"D:\"), None);
    let escapes = |dir: &str| std::path::Path::new(dir).read_dir().map(|d| d.filter_map(|e| e.ok()).any(|e| e.file_name().to_string_lossy().starts_with("xtfcc_escape"))).unwrap_or(false);
    unsafe {
        for image in [r"a\C:\Users\Public\xtfcc_escape", r"C:\xtfcc_escape", r"..\..\xtfcc_escape"] {
            let req = format!(r#"{{"ProcessId":"{}","DumpType":"0","ImageName":"{}"}}"#, std::process::id(), image.replace('\\', "\\\\"));
            let (hr, text) = call(generate_stack_dump_handler, Some(&req));
            let j = parse(&text);
            if hr == S_OK {
                assert_eq!(j.get("Successful").num(), 1.0, "{text}");
                let host = host_of_console(j.get("FilePath").str()).unwrap();
                assert!(host.starts_with(dump_dir()), "{host:?}");
                std::fs::remove_file(host).unwrap();
            } else {
                let e = hr_from_win32(ERROR_INVALID_NAME);
                assert_eq!(hr, e, "{image} {text}");
                assert_eq!(*j.get("XtfOperationError"), operation_error("crashdump.cpp", e, 636));
            }
        }
    }
    assert!(!escapes(r"C:\Users\Public"));
    assert!(!escapes(r"C:\"));
}

#[test]
fn complete_operation_marks_success_or_result_code() {
    let _g = guard();
    unsafe {
        let reply = end_message();
        let mut body = Com::null();
        assert_eq!(xtfmsg::xbtp::create_json_object(body.put()), S_OK);
        let mut hr = E_FAIL;
        complete_operation(reply.get(), body.get(), &mut hr);
        assert_eq!(hr, S_OK);
        let j = parse(&end_json(reply.get()).unwrap());
        assert_eq!(j.get("Successful").num(), 0.0);
        assert_eq!(result_code(&j), E_FAIL);
        let reply = end_message();
        let mut hr = S_OK;
        complete_operation(reply.get(), core::ptr::null_mut(), &mut hr);
        assert_eq!(end_json(reply.get()), None);
        assert_eq!(hr, S_OK);
    }
}
