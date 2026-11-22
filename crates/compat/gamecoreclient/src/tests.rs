use super::*;

const CONSOLE_PATH: &str = r"D:\xbox-pc-port\games\Fortnite\extracted\EmbeddedXvd\Windows\System32\gamecoreclient.dll";

#[link(name = "kernel32")]
extern "system" {
    fn LoadLibraryW(name: *const u16) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
}

#[link(name = "runtimeobject")]
extern "system" {
    fn WindowsCreateString(s: *const u16, len: u32, out: *mut Hstring) -> i32;
    fn WindowsDeleteString(s: Hstring) -> i32;
    fn GetRestrictedErrorInfo(info: *mut *mut c_void) -> i32;
}

#[link(name = "oleaut32")]
extern "system" {
    fn SysFreeString(s: *mut u16);
    fn SysStringLen(s: *mut u16) -> u32;
}

struct Console {
    module: *mut c_void,
}

impl Console {
    fn load() -> Option<Console> {
        if !std::path::Path::new(CONSOLE_PATH).is_file() {
            eprintln!("{CONSOLE_PATH} absent, oracle skipped");
            return None;
        }
        let w: Vec<u16> = CONSOLE_PATH.encode_utf16().chain(Some(0)).collect();
        let h = unsafe { LoadLibraryW(w.as_ptr()) };
        assert!(!h.is_null(), "console gamecoreclient.dll must load on the PC for the oracle");
        Some(Console { module: h })
    }
    unsafe fn sym<T: Copy>(&self, n: &str) -> T {
        let c = format!("{n}\0");
        let p = GetProcAddress(self.module, c.as_ptr());
        assert!(!p.is_null(), "{n}");
        core::mem::transmute_copy(&p)
    }
}

fn hstring(s: &[u16]) -> Hstring {
    let mut h = core::ptr::null_mut();
    assert_eq!(unsafe { WindowsCreateString(s.as_ptr(), s.len() as u32, &mut h) }, S_OK);
    h
}

fn take_error_message() -> Option<(i32, String)> {
    let mut info = core::ptr::null_mut();
    if unsafe { GetRestrictedErrorInfo(&mut info) } != S_OK || info.is_null() {
        return None;
    }
    unsafe {
        let vt = *(info as *const *const usize);
        let details: unsafe extern "system" fn(*mut c_void, *mut *mut u16, *mut i32, *mut *mut u16, *mut *mut u16) -> i32 = core::mem::transmute(*vt.add(3));
        let mut d = core::ptr::null_mut();
        let mut e = 0;
        let mut r = core::ptr::null_mut();
        let mut sid = core::ptr::null_mut();
        assert_eq!(details(info, &mut d, &mut e, &mut r, &mut sid), S_OK);
        let msg = if r.is_null() { String::new() } else { String::from_utf16_lossy(core::slice::from_raw_parts(r, SysStringLen(r) as usize)) };
        for b in [d, r, sid] {
            if !b.is_null() {
                SysFreeString(b);
            }
        }
        let release: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(*vt.add(2));
        release(info);
        Some((e, msg))
    }
}

type Activation = unsafe extern "system" fn(Hstring, *mut *mut c_void) -> i32;
type ClassObject = unsafe extern "system" fn(*const Guid, *const Guid, *mut *mut c_void) -> i32;

fn activation_cases(f: Activation) -> Vec<(i32, bool, Option<(i32, String)>)> {
    let name: Vec<u16> = "Windows.Xbox.GameCore.Client".encode_utf16().collect();
    let embedded: Vec<u16> = "Game\0Core".encode_utf16().collect();
    let mut out = Vec::new();
    for s in [core::ptr::null_mut(), hstring(&name), hstring(&embedded)] {
        take_error_message();
        let mut factory = 0x1234usize as *mut c_void;
        let hr = unsafe { f(s, &mut factory) };
        out.push((hr, factory.is_null(), take_error_message()));
        if !s.is_null() {
            unsafe { WindowsDeleteString(s) };
        }
    }
    out
}

#[test]
fn dll_get_activation_factory_has_no_activatable_classes() {
    let ours = activation_cases(DllGetActivationFactory);
    assert_eq!(ours[0].0, E_INVALIDARG);
    assert_eq!(ours[1].0, CLASS_E_CLASSNOTAVAILABLE);
    assert_eq!(ours[2].0, E_INVALIDARG);
    assert!(ours.iter().all(|r| r.1), "factory is cleared on every path");
    if let Some((e, msg)) = &ours[0].2 {
        assert_eq!(*e, E_INVALIDARG);
        assert_eq!(msg, ACTIVATION_ARG_NAME);
    }
    if let Some(c) = Console::load() {
        let theirs = activation_cases(unsafe { c.sym::<Activation>("DllGetActivationFactory") });
        assert_eq!(ours, theirs);
    }
}

fn class_cases(f: ClassObject) -> Vec<(i32, bool)> {
    let unknown = Guid { data1: 0x0BADC0DE, data2: 1, data3: 2, data4: [3, 4, 5, 6, 7, 8, 9, 10] };
    let mut ppv = 0x1234usize as *mut c_void;
    let hr = unsafe { f(&unknown, &IID_ICLASSFACTORY, &mut ppv) };
    vec![(hr, ppv.is_null())]
}

const IID_IUNKNOWN: Guid = Guid { data1: 0, data2: 0, data3: 0, data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };
const IID_ICLASSFACTORY: Guid = Guid { data1: 1, data2: 0, data3: 0, data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };
const CASE: &str = "TOMBOX_TEST_GAMECORECLIENT_CASE";

#[link(name = "ole32")]
extern "system" {
    fn CoInitializeEx(r: *mut c_void, flags: u32) -> i32;
}

fn consoles() -> Vec<String> {
    console_image::dump_dirs(std::path::Path::new(console_image::GAMES)).into_iter().map(|d| d.join("gamecoreclient.dll")).filter(|p| p.is_file()).map(|p| p.to_string_lossy().into_owned()).collect()
}

unsafe fn vcall<T: Copy>(obj: *mut c_void, slot: usize) -> T {
    let vt = *(obj as *const *const usize);
    core::mem::transmute_copy(&*vt.add(slot))
}

unsafe fn release(obj: *mut c_void) -> u32 {
    vcall::<unsafe extern "system" fn(*mut c_void) -> u32>(obj, 2)(obj)
}

unsafe fn create(factory: *mut c_void, out: &mut *mut c_void) -> i32 {
    vcall::<unsafe extern "system" fn(*mut c_void, *mut c_void, *const Guid, *mut *mut c_void) -> i32>(factory, 3)(factory, core::ptr::null_mut(), &IID_IUNKNOWN, out)
}

const IID_DISPLAY_MODE: Guid = Guid { data1: 0x9E345409, data2: 0x2CF2, data3: 0x4DD1, data4: [0x8F, 0xCA, 0x2F, 0x96, 0xCE, 0x75, 0x81, 0xC0] };
const IID_SIPC_PROXY_CONTROL: Guid = Guid { data1: 0x9A57863E, data2: 0xB229, data3: 0x4696, data4: [0xA8, 0xD4, 0x40, 0xE5, 0xEB, 0x79, 0x2D, 0x0F] };
const IID_APP_MQ: Guid = Guid { data1: 0xAF7F45CF, data2: 0x0C68, data3: 0x46DE, data4: [0xAF, 0xED, 0x25, 0x2B, 0xBF, 0xF0, 0xC9, 0xC4] };

unsafe fn query(obj: *mut c_void, iid: &Guid) -> Option<*mut c_void> {
    let mut out = core::ptr::null_mut();
    let hr = vcall::<unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32>(obj, 0)(obj, iid, &mut out);
    (hr == S_OK && !out.is_null()).then_some(out)
}

unsafe fn dcom_proxy_app(app: *mut c_void) {
    let id = Guid { data1: 0x1111_2222, data2: 3, data3: 4, data4: [5, 6, 7, 8, 9, 10, 11, 12] };
    let mut out = 0xDEAD_BEEFu32;
    let hr = vcall::<unsafe extern "system" fn(*mut c_void, *const Guid, u8, u8, u16, u64, *mut u32) -> i32>(app, 3)(app, &id, 1, 2, 3, 4, &mut out);
    assert_eq!(hr, S_OK, "ISIPCProxyControl2 slot 3 completes through the emulated host IGameCoreApiService broker");
    assert_eq!(out, console_sandbox::gcdcom::result_value());
    assert_eq!(console_sandbox::gcdcom::live_refs(), (0, 0), "the client releases the broker and the target after each call");
    println!("DCOM PROXY APP OK");
}

unsafe fn host_requests(obj: *mut c_void, factory: *mut c_void, conn: u64) -> Vec<u32> {
    let (Some(dm), Some(app)) = (query(obj, &IID_DISPLAY_MODE), query(obj, &IID_APP_MQ)) else {
        let mut sipc = core::ptr::null_mut();
        assert_eq!(create(factory, &mut sipc), S_OK);
        if let Some(app) = query(obj, &IID_APP_MQ).or_else(|| query(sipc, &IID_APP_MQ)) {
            dcom_proxy_app(app);
            release(app);
        }
        if let Some(v1) = query(sipc, &IID_SIPC_PROXY_CONTROL) {
            let id = Guid { data1: 0x1111_2222, data2: 3, data3: 4, data4: [5, 6, 7, 8, 9, 10, 11, 12] };
            let hr = vcall::<unsafe extern "system" fn(*mut c_void, *const Guid, u16) -> i32>(v1, 4)(v1, &id, 3);
            assert_eq!(hr, S_OK, "ISIPCProxyControl slot 4 completes through IGameCoreSipcProxyController on the emulated host");
            assert_eq!(console_sandbox::gcdcom::all_refs(), [0; 4]);
            release(v1);
            println!("DCOM PROXY CLOSE OK");
        }
        release(sipc);
        let raw = console_sandbox::xvcmgmt::host_take_received(conn);
        let (msgs, used, _) = console_sandbox::gchost::parse(&raw, 0);
        assert_eq!(used, raw.len());
        let pid = std::process::id();
        assert_eq!(msgs, vec![(console_sandbox::gchost::WINDOW_MESSAGING, pid.to_le_bytes().to_vec())], "the only queue traffic is the windowmessaging.cpp registration of this process");
        assert_eq!(console_sandbox::gchost::window_pids(conn), vec![pid]);
        return Vec::new();
    };
    let found = [(dm, 3usize), (dm, 4), (app, 3)];
    let call = |i: usize| -> (*mut c_void, usize) { found[i] };
    let d = console_sandbox::gcreply::display();
    let mut kinds = Vec::new();
    let take = |kinds: &mut Vec<u32>| {
        let raw = console_sandbox::xvcmgmt::host_take_received(conn);
        let (msgs, used, _) = console_sandbox::gchost::parse(&raw, 0);
        assert_eq!(used, raw.len(), "one whole request frame reached the host");
        let (last, opens) = msgs.split_last().expect("a request reached the host");
        for (k, p) in opens {
            assert_eq!(*k, console_sandbox::gchost::CHANNEL_OPEN, "{msgs:x?}");
            assert_eq!(p.len(), 6);
        }
        kinds.extend(msgs.iter().map(|m| m.0));
        last.1.clone()
    };

    let mut caps = [0u32; 6];
    let (o, slot) = call(0);
    let hr = vcall::<unsafe extern "system" fn(*mut c_void, *mut u32) -> i32>(o, slot)(o, caps.as_mut_ptr());
    assert_eq!(hr, S_OK, "DisplayMode sub 1 reply consumed");
    let req = take(&mut kinds);
    assert_eq!(req.len(), 8);
    let want = console_sandbox::gcreply::display_mode_1(&req, &d);
    let flags = u32::from_le_bytes(want[0x10..0x14].try_into().unwrap());
    assert_eq!(&caps[..4], &[flags & 1, (flags >> 1) & 1, (flags >> 2) & 1, (flags >> 3) & 1]);
    assert_eq!(caps[4].to_le_bytes(), want[8..12]);
    assert_eq!(caps[5].to_le_bytes(), want[12..16]);

    let mut mode = [0u32; 12];
    let (o, slot) = call(1);
    let hr = vcall::<unsafe extern "system" fn(*mut c_void, *mut u32) -> i32>(o, slot)(o, mode.as_mut_ptr());
    assert_eq!(hr, S_OK, "DisplayMode sub 2 record consumed exactly");
    let req = take(&mut kinds);
    assert_eq!(req.len(), 8);
    let want = console_sandbox::gcreply::display_mode_2(&req, &d);
    let u = |o: usize| u32::from_le_bytes(want[o..o + 4].try_into().unwrap());
    let mut expect = vec![u(8), u(0xc), want[0x10] as u32];
    expect.extend((0..8).map(|i| u(0x14 + 4 * i)));
    expect.push(want[0x34] as u32);
    assert_eq!(&mode[..], &expect[..]);

    let id = Guid { data1: 0x1111_2222, data2: 3, data3: 4, data4: [5, 6, 7, 8, 9, 10, 11, 12] };
    let mut out = 0xDEAD_BEEFu32;
    let (o, slot) = call(2);
    let hr = vcall::<unsafe extern "system" fn(*mut c_void, *const Guid, u8, u8, u16, u64, *mut u32) -> i32>(o, slot)(o, &id, 1, 2, 3, 4, &mut out);
    assert_eq!(hr, S_OK, "GameCoreProxyApp reply consumed with S_OK");
    let req = take(&mut kinds);
    assert_eq!(req.len(), console_sandbox::gcreply::PROXY_REQUEST_LEN);
    let want = console_sandbox::gcreply::proxy_app(&req);
    assert_eq!(out.to_le_bytes(), want[0xc..0x10]);
    assert_eq!(u64::from_le_bytes(req[8..16].try_into().unwrap()), 4);
    assert_eq!(&req[0x10..0x14], &0x1111_2222u32.to_le_bytes());
    assert_eq!(&req[0x20..0x24], &[3, 0, 1, 2]);
    assert_eq!(console_sandbox::gchost::opened(conn), vec![1, 0xf], "DisplayMode opens class 1 and AppMQ class 0xf before their first request");
    release(dm);
    release(app);
    kinds
}

#[test]
fn class_objects_run_every_console_dump_under_the_sandbox() {
    let mut cases = consoles();
    if cases.is_empty() {
        eprintln!("no console gamecoreclient.dll in the title dumps, oracle skipped");
        return;
    }
    cases.push("auto".into());
    let exe = std::env::current_exe().unwrap();
    for c in cases {
        let out = std::process::Command::new(&exe).args(["--exact", "tests::inner_class_oracle", "--nocapture", "--test-threads=1"]).env(CASE, &c).output().unwrap();
        let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        assert!(out.status.success() && text.contains("ORACLE OK"), "case {c}:\n{text}");
        if c != "auto" {
            let want = if c.contains("Fortnite") { "HOST REQUESTS []" } else { "HOST REQUESTS [5, 65543, 131079, 5, 6553663]" };
            assert!(text.contains(want), "case {c} must issue {want}:\n{text}");
            if c.contains("Fortnite") {
                assert!(text.contains("DCOM PROXY APP OK") && text.contains("DCOM PROXY CLOSE OK"), "case {c}: the proxy-app calls go over DCOM to the host:\n{text}");
            }
        }
    }
}

#[test]
fn inner_class_oracle() {
    let Ok(case) = std::env::var(CASE) else { return };
    let dir = std::env::temp_dir().join(format!("gamecoreclient_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::env::set_var("TOMBOX_LOG", dir.join("tombox_compat.log"));
    std::env::set_var("TOMBOX_GDK_ROOT", dir.join("data"));
    if case == "auto" {
        std::env::remove_var("TOMBOX_GAMECORECLIENT_CONSOLE");
    } else {
        std::env::set_var("TOMBOX_GAMECORECLIENT_CONSOLE", &case);
    }
    assert_eq!(DllCanUnloadNow(), S_OK, "nothing loaded yet: no console objects can be alive");
    assert!(CONSOLE.handle.get().is_none(), "DllCanUnloadNow never loads the console on demand");
    assert!(unsafe { CoInitializeEx(core::ptr::null_mut(), 0) } >= 0);

    assert_eq!(class_cases(DllGetClassObject), vec![(CLASS_E_CLASSNOTAVAILABLE, true)]);
    let m = *CONSOLE.handle.get().expect("console loaded by DllGetClassObject");
    assert_ne!(m, 0);
    let path = console_sandbox::modules::module_file(m).unwrap();
    if case != "auto" {
        assert!(path.to_string_lossy().eq_ignore_ascii_case(&case), "{} != {case}", path.display());
    }
    let routed = unsafe { console_sandbox::unrouted(m) };
    for bad in ["OpenSCManagerA", "OpenServiceA", "NotifyServiceStatusChangeA", "CloseServiceHandle", "RegOpenKeyExW", "RegQueryValueExW", "CreateFileW", "NtOpenFile", "NtDeviceIoControlFile", "NtQueryInformationToken"] {
        assert!(!routed.iter().any(|u| u.ends_with(&format!("!{bad}"))), "{bad} reaches the PC: {routed:?}");
    }
    let theirs: ClassObject = unsafe { core::mem::transmute(console_image::symbol(m, "DllGetClassObject").unwrap()) };
    let their_unload: unsafe extern "system" fn() -> i32 = unsafe { core::mem::transmute(console_image::symbol(m, "DllCanUnloadNow").unwrap()) };
    assert_eq!(class_cases(theirs), class_cases(DllGetClassObject));

    let mut f1 = core::ptr::null_mut();
    assert_eq!(unsafe { DllGetClassObject(&CLSID_SIPC_PROXY_CONTROL, &IID_ICLASSFACTORY, &mut f1) }, S_OK);
    assert!(!f1.is_null());
    let mut obj = core::ptr::null_mut();
    assert_eq!(unsafe { create(f1, &mut obj) }, S_OK, "SipcProxyControl constructs: RpcSs status notification and COM security run in the sandbox");
    assert!(!obj.is_null());
    assert_eq!(DllCanUnloadNow(), S_FALSE, "a live console object keeps the module");
    assert_eq!(DllCanUnloadNow(), unsafe { their_unload() });
    unsafe { release(obj) };

    let mut f2 = core::ptr::null_mut();
    assert_eq!(unsafe { DllGetClassObject(&CLSID_SECOND, &IID_ICLASSFACTORY, &mut f2) }, S_OK);
    assert!(!f2.is_null());
    let mut obj2 = core::ptr::null_mut();
    let hr = unsafe { create(f2, &mut obj2) };
    assert_eq!(hr, S_OK, "7FA7A8FD constructs once the XBOX://LI launch claim and the xvcmgmt queue exist; got {hr:#x}");
    assert!(!obj2.is_null());

    let li = console_sandbox::claims::launch_instance();
    let conns = console_sandbox::xvcmgmt::host_connections(li);
    assert_eq!(conns.len(), 1, "the console queue client connected to the emulated host on {{c12a416b, XBOX://LI}}");
    assert!(console_sandbox::xvcmgmt::host_take_received(conns[0]).is_empty(), "construction sends nothing to the host");
    let issued = unsafe { host_requests(obj2, f1, conns[0]) };
    println!("HOST REQUESTS {issued:?}");
    unsafe {
        release(obj2);
        release(f1);
        release(f2);
    }
    let after = DllCanUnloadNow();
    assert_eq!(after, S_FALSE, "the console keeps its init-once queue singleton for the module lifetime");
    assert_eq!(after, unsafe { their_unload() });
    println!("ORACLE OK {} unload-after-release={after}", path.display());
}
