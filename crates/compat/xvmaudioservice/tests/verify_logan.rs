use core::ffi::c_void;
use XVMAudioService::*;

const IID_ICLASSFACTORY: Guid = Guid { data1: 1, data2: 0, data3: 0, data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };
const IID_IUNKNOWN: Guid = Guid { data1: 0, data2: 0, data3: 0, data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };
const STEP: &str = "TOMBOX_VERIFY_LOGAN_STEP";

unsafe fn vslot<T: Copy>(obj: *mut c_void, slot: usize) -> T {
    let vt = *(obj as *const *const usize);
    core::mem::transmute_copy(&*vt.add(slot))
}

unsafe fn add_ref(obj: *mut c_void) -> u32 {
    vslot::<unsafe extern "system" fn(*mut c_void) -> u32>(obj, 1)(obj)
}

unsafe fn release(obj: *mut c_void) -> u32 {
    vslot::<unsafe extern "system" fn(*mut c_void) -> u32>(obj, 2)(obj)
}

fn console() -> Option<String> {
    console_image::dump_dirs(std::path::Path::new(console_image::GAMES)).into_iter().map(|d| d.join("XVMAudioService.dll")).find(|p| p.is_file()).map(|p| p.to_string_lossy().into_owned())
}

fn child(step: &str) -> (bool, String) {
    let exe = std::env::current_exe().unwrap();
    let out = std::process::Command::new(&exe).args(["--exact", "inner", "--nocapture", "--test-threads=1"]).env(STEP, step).output().unwrap();
    let text = format!("exit={:?}\n{}{}", out.status.code().map(|c| c as u32), String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    println!("step {step}:\n{text}");
    (out.status.success(), text)
}

#[test]
fn logan_class_and_unload_refcount_against_the_console() {
    if console().is_none() {
        eprintln!("no console XVMAudioService.dll, skipped");
        return;
    }
    let (ok, text) = child("factory");
    assert!(ok && text.contains("STEP OK"), "{text}");
    let (ok, text) = child("create");
    assert!(ok && text.contains("CREATED") && text.contains("BUFFER OK"), "e782ced1 constructs on the emulated Logan audio pool:\n{text}");
}

#[test]
fn inner() {
    let Ok(step) = std::env::var(STEP) else { return };
    let dir = std::env::temp_dir().join(format!("xvmaudio_verify_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::env::set_var("TOMBOX_LOG", dir.join("tombox_compat.log"));
    std::env::set_var("TOMBOX_GDK_ROOT", dir.join("data"));
    std::env::set_var("TOMBOX_XVMAUDIOSERVICE_CONSOLE", console().unwrap());
    let unknown = Guid { data1: 0x0BADC0DE, data2: 1, data3: 2, data4: [3, 4, 5, 6, 7, 8, 9, 10] };
    let mut ppv = 0x1234usize as *mut c_void;
    assert_eq!(unsafe { DllGetClassObject(&unknown, &IID_ICLASSFACTORY, &mut ppv) }, CLASS_E_CLASSNOTAVAILABLE);
    let m = console_module();
    assert_ne!(m, 0);
    let theirs: unsafe extern "system" fn(*const Guid, *const Guid, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(console_image::symbol(m, "DllGetClassObject").unwrap()) };
    let mut f = core::ptr::null_mut();
    let hr = unsafe { theirs(&CLSID_LOGAN_BOUND, &IID_ICLASSFACTORY, &mut f) };
    println!("console DllGetClassObject(e782ced1) = {:#010x} factory={f:?}", hr as u32);
    if step == "factory" {
        let their_unload: unsafe extern "system" fn() -> i32 = unsafe { core::mem::transmute(console_image::symbol(m, "DllCanUnloadNow").unwrap()) };
        let held = DllCanUnloadNow();
        println!("unload with logan factory held = {held}");
        assert_eq!(held, unsafe { their_unload() });
        if !f.is_null() {
            unsafe {
                let r = add_ref(f);
                assert_eq!(release(f), r - 1, "AddRef/Release balance");
                release(f);
            }
        }
        assert_eq!(DllCanUnloadNow(), unsafe { their_unload() });
        let base = DllCanUnloadNow();
        for c in CLSID_FORWARDED {
            let mut a = core::ptr::null_mut();
            let mut b = core::ptr::null_mut();
            assert_eq!(unsafe { DllGetClassObject(&c, &IID_ICLASSFACTORY, &mut a) }, S_OK);
            assert_eq!(unsafe { DllGetClassObject(&c, &IID_ICLASSFACTORY, &mut b) }, S_OK);
            assert_eq!(a, b, "WRL caches one factory per class, {c:x?}");
            let mid = DllCanUnloadNow();
            unsafe { release(a) };
            assert_eq!(DllCanUnloadNow(), unsafe { their_unload() }, "{c:x?}");
            unsafe { release(b) };
            let after = DllCanUnloadNow();
            println!("{:08x}: two factory refs -> {mid}, released -> {after}", c.data1);
            assert_eq!(after, base, "{c:x?}");
        }
        let mut f2 = core::ptr::null_mut();
        assert_eq!(unsafe { DllGetClassObject(&CLSID_FORWARDED[0], &IID_ICLASSFACTORY, &mut f2) }, S_OK);
        let mut o = core::ptr::null_mut();
        assert_eq!(unsafe { vslot::<unsafe extern "system" fn(*mut c_void, *mut c_void, *const Guid, *mut *mut c_void) -> i32>(f2, 3)(f2, core::ptr::null_mut(), &IID_IUNKNOWN, &mut o) }, S_OK);
        unsafe { release(f2) };
        assert_eq!(DllCanUnloadNow(), 1, "a live object alone pins the module");
        let r = unsafe { add_ref(o) };
        unsafe { release(o) };
        assert_eq!(DllCanUnloadNow(), 1);
        assert_eq!(unsafe { release(o) }, r - 2);
        assert_eq!(DllCanUnloadNow(), S_OK);
        println!("STEP OK");
        return;
    }
    assert_eq!(hr, S_OK, "console hands out the e782ced1 factory");
    let mut o = core::ptr::null_mut();
    let chr = unsafe { vslot::<unsafe extern "system" fn(*mut c_void, *mut c_void, *const Guid, *mut *mut c_void) -> i32>(f, 3)(f, core::ptr::null_mut(), &IID_IUNKNOWN, &mut o) };
    println!("CreateInstance returned {:#010x} obj={o:?}", chr as u32);
    assert!(chr >= 0 && !o.is_null(), "CreateInstance {chr:#x}");
    println!("CREATED");
    let pages = 4usize;
    let index = console_sandbox::logan::pool_alloc(pages).expect("host allocates a run in the audio pool");
    let mut desc = [0u8; 0x30];
    desc[0..4].copy_from_slice(&3u32.to_le_bytes());
    let g = |g: &Guid| -> Vec<u8> { [g.data1.to_le_bytes().to_vec(), g.data2.to_le_bytes().to_vec(), g.data3.to_le_bytes().to_vec(), g.data4.to_vec()].concat() };
    desc[8..0x18].copy_from_slice(&g(&CLSID_LOGAN_BOUND));
    desc[0x18..0x28].copy_from_slice(&g(&Guid { data1: 0x00aa2a8a, data2: 0xacb6, data3: 0x4f3d, data4: [0x9a, 0x7e, 0xbd, 0xe7, 0x12, 0x7b, 0x43, 0x87] }));
    desc[0x28..0x2c].copy_from_slice(&(index as u32).to_le_bytes());
    let ihr = unsafe { vslot::<unsafe extern "system" fn(*mut c_void, *const u8) -> i32>(o, 3)(o, desc.as_ptr()) };
    assert_eq!(ihr, S_OK, "Init with a host-allocated block");
    let mut size = 0u32;
    let mut va = core::ptr::null_mut::<c_void>();
    let ghr = unsafe { vslot::<unsafe extern "system" fn(*mut c_void, *mut u32, *mut *mut c_void) -> i32>(o, 4)(o, &mut size, &mut va) };
    println!("GetBuffer {ghr:#x} size {size:#x} va {va:?} pool {:#x} entry {:#x}", console_sandbox::logan::pool(), console_sandbox::logan::pool_entry(index as usize));
    assert_eq!(ghr, S_OK);
    assert_eq!(va as usize, console_sandbox::logan::pool() + index as usize * console_sandbox::logan::PAGE);
    assert_eq!(size as usize, pages * console_sandbox::logan::PAGE);
    assert_eq!((console_sandbox::logan::pool_entry(index as usize) >> 10) & 0xf, 2, "the guest took its reference on the run");
    unsafe { release(o) };
    println!("entry after release {:#x}", console_sandbox::logan::pool_entry(index as usize));
    println!("BUFFER OK");
}
