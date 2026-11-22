use core::ffi::c_void;
use console_sandbox::crossvm;
use XVMAudioService::*;

const IID_ICLASSFACTORY: Guid = Guid { data1: 1, data2: 0, data3: 0, data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };
const CLSID_PLUGIN: Guid = Guid { data1: 0x7f34f6c9, data2: 0xbed0, data3: 0x456d, data4: [0x97, 0x72, 0x0d, 0x5b, 0x52, 0x4a, 0x9d, 0x69] };
const IID_PLUGIN: Guid = Guid { data1: 0xf36b549a, data2: 0x9e77, data3: 0x402e, data4: [0xaa, 0x83, 0x97, 0xb8, 0xd0, 0xda, 0x8d, 0x23] };
const CASE: &str = "TOMBOX_VERIFY_PLUGIN_CASE";

unsafe fn vslot<T: Copy>(obj: *mut c_void, slot: usize) -> T {
    let vt = *(obj as *const *const usize);
    core::mem::transmute_copy(&*vt.add(slot))
}

fn consoles() -> Vec<String> {
    console_image::dump_dirs(std::path::Path::new(console_image::GAMES)).into_iter().map(|d| d.join("XVMAudioService.dll")).filter(|p| p.is_file()).map(|p| p.to_string_lossy().into_owned()).collect()
}

#[test]
fn console_plugin_calls_reach_the_host_on_every_dump() {
    let cases = consoles();
    if cases.is_empty() {
        eprintln!("no console XVMAudioService.dll, skipped");
        return;
    }
    let exe = std::env::current_exe().unwrap();
    for c in cases {
        let mut child = std::process::Command::new(&exe).args(["--exact", "inner_plugin", "--nocapture", "--test-threads=1"]).env(CASE, &c).env("TOMBOX_AUDIO_MUTE", "1").stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().unwrap();
        let t = std::time::Instant::now();
        let status = loop {
            if let Some(s) = child.try_wait().unwrap() {
                break Some(s);
            }
            if t.elapsed().as_secs() > 60 {
                let _ = child.kill();
                break None;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        };
        let out = child.wait_with_output().unwrap();
        let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        println!("== {c}\n{text}");
        assert!(status.map(|s| s.success()).unwrap_or(false) && (text.contains("PLUGIN OK") || text.contains("NO PC AUDIO ENDPOINT")), "case {c} (timed out: {}):\n{text}", status.is_none());
    }
}

#[test]
fn inner_plugin() {
    let Ok(case) = std::env::var(CASE) else { return };
    let dir = std::env::temp_dir().join(format!("xvmaudio_plugin_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::env::set_var("TOMBOX_LOG", dir.join("tombox_compat.log"));
    std::env::set_var("TOMBOX_GDK_ROOT", dir.join("data"));
    std::env::set_var("TOMBOX_XVMAUDIOSERVICE_CONSOLE", &case);
    let mut f = core::ptr::null_mut();
    assert_eq!(unsafe { DllGetClassObject(&CLSID_PLUGIN, &IID_ICLASSFACTORY, &mut f) }, S_OK);
    let mut p = core::ptr::null_mut();
    assert_eq!(unsafe { vslot::<unsafe extern "system" fn(*mut c_void, *mut c_void, *const Guid, *mut *mut c_void) -> i32>(f, 3)(f, core::ptr::null_mut(), &IID_PLUGIN, &mut p) }, S_OK, "plugin class answers IID f36b549a");
    let init = unsafe { vslot::<unsafe extern "system" fn(*mut c_void, u32) -> i32>(p, 3)(p, 0) };
    println!("plugin Init -> {init:#010x}, host saw hello {:#x} before Init returned", crossvm::hello_seen());
    assert_eq!(init, S_OK, "Init returns once the host acks the audio hello");
    let call = unsafe { vslot::<unsafe extern "system" fn(*mut c_void, u32, *mut u8, u32) -> i32>(p, 4) };
    let mut env = vec![0u8; crossvm::ENVELOPE];
    env[8..12].copy_from_slice(&crossvm::GET_MIX_FORMAT.to_le_bytes());
    assert_eq!(unsafe { call(p, 0, env.as_mut_ptr(), env.len() as u32) }, S_OK);
    let hr = i32::from_le_bytes(env[4..8].try_into().unwrap());
    assert_eq!(u32::from_le_bytes(env[8..12].try_into().unwrap()), crossvm::GET_MIX_FORMAT, "id echoed through the console plugin");
    if hr < 0 {
        println!("NO PC AUDIO ENDPOINT {hr:#010x}");
        return;
    }
    let hello = crossvm::hello_seen();
    println!("hello seen after the first Call: {hello:#x}");
    assert!(hello == 1 || hello == 2, "the console plugin sent its generation hello before the first reply");
    assert_eq!(crossvm::generation(), hello, "host dispatch uses the generation the console sent");
    let rate = u32::from_le_bytes(env[0x14..0x18].try_into().unwrap());
    println!("GetMixFormat through the console plugin: {rate} Hz");
    assert!(rate > 0);
    let mut env = vec![0u8; crossvm::ENVELOPE];
    env[8..12].copy_from_slice(&crossvm::CREATE_OBJECT.to_le_bytes());
    env[0x10..0x14].copy_from_slice(&2u32.to_le_bytes());
    env[0x14..0x18].copy_from_slice(&1u32.to_le_bytes());
    env[0x20..0x24].copy_from_slice(&0x2fu32.to_le_bytes());
    assert_eq!(unsafe { call(p, 0, env.as_mut_ptr(), env.len() as u32) }, S_OK);
    assert_eq!(i32::from_le_bytes(env[4..8].try_into().unwrap()), S_OK);
    let object = u32::from_le_bytes(env[0x1c..0x20].try_into().unwrap());
    assert_eq!(crossvm::object_kind(object), Some(2), "the host created the audio client core object the guest asked for");
    let mut env = vec![0u8; crossvm::ENVELOPE];
    env[8..12].copy_from_slice(&crossvm::RELEASE_OBJECT.to_le_bytes());
    env[0xc..0x10].copy_from_slice(&object.to_le_bytes());
    assert_eq!(unsafe { call(p, 0, env.as_mut_ptr(), env.len() as u32) }, S_OK);
    assert_eq!(crossvm::object_kind(object), None);
    println!("PLUGIN OK");
}
