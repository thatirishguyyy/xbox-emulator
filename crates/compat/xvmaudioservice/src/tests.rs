use super::*;

#[link(name = "runtimeobject")]
extern "system" {
    fn WindowsCreateString(s: *const u16, len: u32, out: *mut HSTRING) -> i32;
    fn WindowsDeleteString(s: HSTRING) -> i32;
}

fn hstring(s: &[u16]) -> HSTRING {
    let mut h: HSTRING = core::ptr::null_mut();
    assert_eq!(unsafe { WindowsCreateString(s.as_ptr(), s.len() as u32, &mut h) }, 0);
    h
}

#[test]
fn dll_can_unload_now_with_no_module_objects() {
    assert_eq!(DllCanUnloadNow(), S_OK);
}

#[test]
fn activation_factory_table_is_empty_like_console() {
    unsafe {
        let mut out = 0x44usize as *mut c_void;
        assert_eq!(DllGetActivationFactory(core::ptr::null_mut(), &mut out), E_INVALIDARG);
        assert!(out.is_null());
        let name: Vec<u16> = "Windows.Xbox.Audio.Anything".encode_utf16().collect();
        let h = hstring(&name);
        out = 0x44usize as *mut c_void;
        assert_eq!(DllGetActivationFactory(h, &mut out), CLASS_E_CLASSNOTAVAILABLE);
        assert!(out.is_null());
        WindowsDeleteString(h);
        let embedded: Vec<u16> = vec![b'a' as u16, 0, b'b' as u16];
        let h = hstring(&embedded);
        assert_eq!(DllGetActivationFactory(h, &mut out), E_INVALIDARG);
        WindowsDeleteString(h);
    }
}

const IID_IUNKNOWN: Guid = Guid { data1: 0, data2: 0, data3: 0, data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };
const IID_ICLASSFACTORY: Guid = Guid { data1: 1, data2: 0, data3: 0, data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };
const CASE: &str = "TOMBOX_TEST_XVMAUDIOSERVICE_CASE";

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleExW(flags: u32, addr: *const u16, out: *mut *mut c_void) -> i32;
}

fn module_of(addr: usize) -> usize {
    let mut h = core::ptr::null_mut();
    if unsafe { GetModuleHandleExW(6, addr as *const u16, &mut h) } == 0 {
        0
    } else {
        h as usize
    }
}

fn consoles() -> Vec<String> {
    console_image::dump_dirs(std::path::Path::new(console_image::GAMES)).into_iter().map(|d| d.join("XVMAudioService.dll")).filter(|p| p.is_file()).map(|p| p.to_string_lossy().into_owned()).collect()
}

unsafe fn vslot<T: Copy>(obj: *mut c_void, slot: usize) -> T {
    let vt = *(obj as *const *const usize);
    core::mem::transmute_copy(&*vt.add(slot))
}

unsafe fn release(obj: *mut c_void) -> u32 {
    vslot::<unsafe extern "system" fn(*mut c_void) -> u32>(obj, 2)(obj)
}

unsafe fn create(factory: *mut c_void, out: &mut *mut c_void) -> i32 {
    vslot::<unsafe extern "system" fn(*mut c_void, *mut c_void, *const Guid, *mut *mut c_void) -> i32>(factory, 3)(factory, core::ptr::null_mut(), &IID_IUNKNOWN, out)
}

#[test]
fn class_objects_run_every_console_dump_under_the_sandbox() {
    let mut cases = consoles();
    if cases.is_empty() {
        eprintln!("no console XVMAudioService.dll in the title dumps, oracle skipped");
        return;
    }
    cases.push("auto".into());
    let exe = std::env::current_exe().unwrap();
    for c in cases {
        let out = std::process::Command::new(&exe).args(["--exact", "tests::inner_class_oracle", "--nocapture", "--test-threads=1"]).env(CASE, &c).output().unwrap();
        let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        assert!(out.status.success() && text.contains("ORACLE OK"), "case {c}:\n{text}");
    }
}

#[test]
fn inner_class_oracle() {
    let Ok(case) = std::env::var(CASE) else { return };
    let dir = std::env::temp_dir().join(format!("xvmaudioservice_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::env::set_var("TOMBOX_LOG", dir.join("tombox_compat.log"));
    std::env::set_var("TOMBOX_GDK_ROOT", dir.join("data"));
    if case == "auto" {
        std::env::remove_var("TOMBOX_XVMAUDIOSERVICE_CONSOLE");
    } else {
        std::env::set_var("TOMBOX_XVMAUDIOSERVICE_CONSOLE", &case);
    }
    assert_eq!(DllCanUnloadNow(), S_OK);
    assert!(CONSOLE.handle.get().is_none(), "DllCanUnloadNow never loads the console on demand");

    let unknown = Guid { data1: 0x0BADC0DE, data2: 1, data3: 2, data4: [3, 4, 5, 6, 7, 8, 9, 10] };
    let mut ppv = 0x1234usize as *mut c_void;
    let ours = unsafe { DllGetClassObject(&unknown, &IID_ICLASSFACTORY, &mut ppv) };
    assert_eq!((ours, ppv.is_null()), (CLASS_E_CLASSNOTAVAILABLE, true));
    let m = *CONSOLE.handle.get().expect("console loaded by DllGetClassObject");
    assert_ne!(m, 0);
    let path = console_sandbox::modules::module_file(m).unwrap();
    if case != "auto" {
        assert!(path.to_string_lossy().eq_ignore_ascii_case(&case), "{} != {case}", path.display());
    }
    assert!(!sandbox_hits().is_empty(), "console imports are routed to console-sandbox");
    let routed = unsafe { console_sandbox::unrouted(m) };
    for bad in ["RegOpenKeyExW", "RegQueryValueExW", "RegSetValueExW", "RegCreateKeyExW", "CreateFileW", "DeleteFileW"] {
        assert!(!routed.iter().any(|u| u.ends_with(&format!("!{bad}"))), "{bad} reaches the PC: {routed:?}");
    }
    let theirs: unsafe extern "system" fn(*const Guid, *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(console_image::symbol(m, "DllGetClassObject").unwrap()) };
    let their_unload: unsafe extern "system" fn() -> i32 = unsafe { core::mem::transmute(console_image::symbol(m, "DllCanUnloadNow").unwrap()) };
    ppv = 0x1234usize as *mut c_void;
    assert_eq!((unsafe { theirs(&unknown, &IID_ICLASSFACTORY, &mut ppv) }, ppv.is_null()), (ours, true));

    let mut live = Vec::new();
    for c in CLSID_FORWARDED.into_iter().chain([CLSID_LOGAN_BOUND]) {
        let mut f = core::ptr::null_mut();
        assert_eq!(unsafe { DllGetClassObject(&c, &IID_ICLASSFACTORY, &mut f) }, S_OK, "{c:x?}");
        assert_eq!(module_of(f as usize), 0, "the factory is a heap object of the console module, {c:x?}");
        assert_eq!(module_of(unsafe { *(f as *const usize) }), m, "factory vtable lives in the console image, {c:x?}");
        let mut o = core::ptr::null_mut();
        assert_eq!(unsafe { create(f, &mut o) }, S_OK, "{c:x?} constructs under the sandbox");
        assert!(!o.is_null());
        assert_eq!(module_of(unsafe { *(o as *const usize) }), m, "object vtable lives in the console image, {c:x?}");
        live.push((f, o));
    }
    assert_eq!(DllCanUnloadNow(), 1, "live console objects keep the module");
    assert_eq!(DllCanUnloadNow(), unsafe { their_unload() });
    for (f, o) in live {
        unsafe {
            release(o);
            release(f);
        }
    }
    let after = DllCanUnloadNow();
    println!("ORACLE OK {} unload-after-release={after}", path.display());
    assert_eq!(after, S_OK, "every console object released");
}
