use core::ffi::c_void;
use std::time::Duration;

use console_sandbox::alpc::{self, ATTR_CONTEXT, ATTR_HANDLE, ATTR_VIEW, HEADER};
use console_sandbox::{cfgmgr, conkbd, modules};

use crate::{console_module, sandbox_hits, unrouted, ServerDllInitialization, PORT_NAME};

#[link(name = "ntdll")]
extern "system" {
    fn AlpcInitializeMessageAttribute(flags: u32, buf: *mut u8, len: usize, req: *mut usize) -> i32;
    fn AlpcGetMessageAttribute(buf: *mut u8, attr: u32) -> *mut u8;
}

#[link(name = "kernel32")]
extern "system" {
    fn ReadFile(h: usize, b: *mut u8, n: u32, done: *mut u32, ov: *mut c_void) -> i32;
    fn PeekNamedPipe(h: usize, b: *mut u8, n: u32, read: *mut u32, avail: *mut u32, left: *mut u32) -> i32;
    fn GetProcAddress(m: usize, name: *const u8) -> usize;
}

pub struct Layout {
    pub vsc_to_vk: Vec<u16>,
    pub vk_to_char: Vec<(u8, u16)>,
}

pub unsafe fn layout(kbd: usize) -> Layout {
    let f = GetProcAddress(kbd, b"KbdLayerDescriptor\0".as_ptr());
    assert_ne!(f, 0, "console KBDUS exports KbdLayerDescriptor");
    let desc: unsafe extern "system" fn() -> *const u8 = core::mem::transmute(f);
    let t = desc();
    let vsc = (t.add(0x30) as *const *const u16).read();
    let max = *t.add(0x38) as usize;
    let vsc_to_vk = std::slice::from_raw_parts(vsc, max).to_vec();
    let mut tables = (t.add(8) as *const *const u8).read();
    let mut vk_to_char = Vec::new();
    loop {
        let mut e = (tables as *const *const u8).read();
        if e.is_null() {
            break;
        }
        let size = *tables.add(9) as usize;
        while *e != 0 {
            vk_to_char.push((*e, (e.add(2) as *const u16).read_unaligned()));
            e = e.add(size);
        }
        tables = tables.add(16);
    }
    Layout { vsc_to_vk, vk_to_char }
}

fn msg_attrs(flags: u32) -> Vec<u8> {
    let mut b = vec![0u8; 0x60];
    let mut got = 0usize;
    assert_eq!(unsafe { AlpcInitializeMessageAttribute(flags, b.as_mut_ptr(), b.len(), &mut got) }, 0);
    b
}

fn request(port: usize, code: u8, params: &[u8]) -> Vec<u8> {
    let mut m = vec![0u8; 0x40];
    m[0..8].copy_from_slice(&0x0040_0018u64.to_le_bytes());
    m[HEADER] = code;
    m[0x30..0x30 + params.len()].copy_from_slice(params);
    let mut len = 0x40usize;
    let st = unsafe { alpc::nt_alpc_send_wait_receive_port(port, 0, m.as_ptr(), core::ptr::null_mut(), m.as_mut_ptr(), &mut len, core::ptr::null_mut(), core::ptr::null()) };
    assert_eq!(st, 0, "request {code} through the ALPC port model");
    assert_eq!(m[4], alpc::LPC_REPLY as u8);
    m
}

fn read_pipe(h: usize, n: usize) -> Vec<u8> {
    let t = std::time::Instant::now();
    loop {
        let mut avail = 0u32;
        unsafe { PeekNamedPipe(h, core::ptr::null_mut(), 0, core::ptr::null_mut(), &mut avail, core::ptr::null_mut()) };
        if avail as usize >= n {
            break;
        }
        assert!(t.elapsed() < Duration::from_secs(10), "console wrote {avail} of {n} bytes to the event pipe");
        std::thread::sleep(Duration::from_millis(5));
    }
    let mut b = vec![0u8; n];
    let mut got = 0u32;
    assert_ne!(unsafe { ReadFile(h, b.as_mut_ptr(), n as u32, &mut got, core::ptr::null_mut()) }, 0);
    assert_eq!(got as usize, n);
    b
}

fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}

fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

pub fn run_console_conio(console: &str) {
    let dir = std::env::temp_dir().join(format!("tombox-coniosrv-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::env::set_var("TOMBOX_GDK_ROOT", &dir);
    std::env::set_var("TOMBOX_CONIOSRV_CONSOLE", console);
    assert!(std::path::Path::new(console).is_file(), "{console}");
    let m = console_module();
    assert_ne!(m, 0, "console ConIoSrv.dll loads");
    let hits = sandbox_hits();
    for f in ["NtAlpcCreatePort", "NtAlpcSendWaitReceivePort", "NtAlpcAcceptConnectPort", "NtAlpcCreatePortSection", "NtAlpcOpenSenderProcess", "RtlGetDeviceFamilyInfoEnum", "CM_Register_Notification", "CM_Get_Device_Interface_ListW", "LoadLibraryExW", "RegOpenKeyExW", "RegQueryValueExW", "NtOpenFile", "NtCreateFile", "NtReadFile", "NtClose"] {
        assert!(hits.iter().any(|h| h.ends_with(&format!("!{f}"))), "{f} routed to the sandbox: {hits:?}");
    }
    assert_eq!(unrouted(), Vec::<String>::new(), "every console import is sandboxed or a pure in-process call");
    let mut csr = vec![0u8; 0x78 + 0x20];
    csr[0x24..0x38].fill(0xaa);
    let st = unsafe { ServerDllInitialization(csr.as_mut_ptr() as *mut c_void) };
    assert_eq!(st, 0, "console ServerDllInitialization succeeds");
    assert!(csr[0x24..0x38].iter().all(|&b| b == 0), "console code cleared the CSR_SERVER_DLL api fields");
    let console_dir = std::path::Path::new(console).parent().unwrap().to_path_buf();
    let kbd = modules::loaded()
        .into_iter()
        .find(|&x| modules::module_file(x).is_some_and(|p| p.file_name().is_some_and(|f| f.to_string_lossy().eq_ignore_ascii_case("KBDUS.DLL")) && p.parent().is_some_and(|d| d.to_string_lossy().eq_ignore_ascii_case(&console_dir.to_string_lossy()))))
        .expect("the console KBDUS.DLL beside ConIoSrv.dll is the loaded layout");
    let lay = unsafe { layout(kbd) };
    assert!(alpc::wait_for(|| alpc::port_exists(PORT_NAME), Duration::from_secs(5)), "console created \\ConsoleInputServerPort");
    let w: Vec<u16> = PORT_NAME.encode_utf16().collect();
    let us: [usize; 2] = [(w.len() * 2) | ((w.len() * 2) << 16), w.as_ptr() as usize];
    let mut pa = [0u8; 0x48];
    pa[0..4].copy_from_slice(&0x0208_0000u32.to_le_bytes());
    pa[4..8].copy_from_slice(&0xcu32.to_le_bytes());
    pa[0xc] = 1;
    pa[0x10..0x18].copy_from_slice(&0x40usize.to_le_bytes());
    pa[0x20..0x28].copy_from_slice(&0x4000usize.to_le_bytes());
    pa[0x40..0x44].copy_from_slice(&1u32.to_le_bytes());
    let mut conn = vec![0u8; 0x40];
    conn[0..8].copy_from_slice(&0x0040_0018u64.to_le_bytes());
    let mut len = 0x40usize;
    let mut ia = msg_attrs(ATTR_VIEW | ATTR_CONTEXT | ATTR_HANDLE);
    let mut port = 0usize;
    let st = unsafe { alpc::nt_alpc_connect_port(&mut port, us.as_ptr() as *const u8, core::ptr::null(), pa.as_ptr(), 0x20000, core::ptr::null(), conn.as_mut_ptr(), &mut len, core::ptr::null_mut(), ia.as_mut_ptr(), core::ptr::null()) };
    assert_eq!(st, 0, "conhost-style NtAlpcConnectPort to \\ConsoleInputServerPort");
    let (view_base, view_size, count) = unsafe {
        let va = AlpcGetMessageAttribute(ia.as_mut_ptr(), ATTR_VIEW);
        let ha = AlpcGetMessageAttribute(ia.as_mut_ptr(), ATTR_HANDLE);
        ((va.add(0x10) as *const usize).read(), (va.add(0x18) as *const usize).read(), (ha.add(0x10) as *const u32).read())
    };
    assert_eq!(count, 2, "conhost requires exactly two handles");
    assert_eq!(view_size, 0x10000, "client view of the console's 0x10000 port section");
    assert_ne!(view_base, 0);
    assert_eq!(u16_at(&conn, 0x30), 0, "display mode in the connection reply: no ConDrv display device, so mode 0");
    let mut handles = [0usize; 2];
    for (i, h) in handles.iter_mut().enumerate() {
        let mut info = [i as u32, 0, 0, 0, 0];
        assert_eq!(unsafe { alpc::nt_alpc_query_information_message(port, conn.as_ptr(), alpc::MESSAGE_HANDLE_INFORMATION, info.as_mut_ptr() as *mut u8, 0x14, core::ptr::null_mut()) }, 0);
        *h = info[2] as usize;
        assert_ne!(*h, 0);
    }
    assert_eq!(alpc::connection_count(PORT_NAME), 1);
    let scan_a = lay.vsc_to_vk.iter().position(|&v| v & 0xff == 0x41).expect("layout maps VK_A") as u32;
    let r = request(port, 0, &[0x41, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(r[0x2c], 1, "MapVirtualKey handled");
    assert_eq!(u32_at(&r, 0x38), scan_a, "MapVirtualKey(VK_A, MAPVK_VK_TO_VSC) from the console layout");
    let r = request(port, 0, &[0x1e, 0, 0, 0, 1, 0, 0, 0]);
    assert_eq!(u32_at(&r, 0x38), (lay.vsc_to_vk[0x1e] & 0xff) as u32, "MapVirtualKey(0x1e, MAPVK_VSC_TO_VK)");
    let ch_a = lay.vk_to_char.iter().find(|e| e.0 == 0x41).map(|e| e.1).expect("layout has VK_A characters");
    let r = request(port, 1, &ch_a.to_le_bytes());
    assert_eq!(r[0x2c], 1);
    assert_eq!(u16_at(&r, 0x32) & 0xff, 0x41, "VkKeyScan of the console layout's VK_A character");
    let r = request(port, 9, &[]);
    assert_eq!(r[0x2c], 0, "unknown request code is left unhandled by the console server");
    for (code, at) in [(3u8, 0x38usize), (4, 0x38), (6, 0x3c)] {
        let r = request(port, code, &[]);
        assert_eq!(r[0x2c], 1, "display request {code} handled");
        assert_eq!(u32_at(&r, at), 0xC000_0001, "display request {code} fails STATUS_UNSUCCESSFUL with no ConDrv display");
    }
    assert!(alpc::wait_for(|| cfgmgr::interface_registrations(conkbd::KEYBOARD_INTERFACE) >= 1, Duration::from_secs(5)), "console keyboard thread registered for keyboard arrival");
    assert_eq!(cfgmgr::interfaces(conkbd::KEYBOARD_INTERFACE), Vec::<String>::new());
    let link = conkbd::attach();
    assert!(alpc::wait_for(|| conkbd::pending_reads(&link) == 1, Duration::from_secs(5)), "console opened the arrived keyboard and posted a read");
    let dev = conkbd::open_handles(&link);
    assert_eq!(dev.len(), 1);
    assert_eq!(cfgmgr::handle_registrations(|t| dev.contains(&t)), 1, "console registered removal notification on its keyboard handle");
    let scan = scan_a as u16;
    assert!(conkbd::input(&link, scan, 0));
    let ev = read_pipe(handles[0], 0x18);
    assert_eq!(ev[0], 0, "key event record");
    assert_eq!(u16_at(&ev, 4), 1, "KEY_EVENT");
    assert_eq!(u32_at(&ev, 8), 1, "key down");
    assert_eq!(u16_at(&ev, 0xc), 1, "repeat count");
    assert_eq!(u16_at(&ev, 0xe), 0x41, "virtual key from the console layout");
    assert_eq!(u16_at(&ev, 0x10), scan, "scan code");
    assert_eq!(u16_at(&ev, 0x12), ch_a, "character from the console layout");
    let r = request(port, 2, &[0x41, 0, 0, 0]);
    assert_eq!(r[0x2c], 1);
    assert_ne!(u16_at(&r, 0x34) & 0x8000, 0, "GetKeyState reports VK_A down");
    assert!(alpc::wait_for(|| conkbd::pending_reads(&link) == 1, Duration::from_secs(5)));
    assert!(conkbd::input(&link, scan, conkbd::KEY_BREAK));
    let ev = read_pipe(handles[0], 0x18);
    assert_eq!(u32_at(&ev, 8), 0, "key up");
    let r = request(port, 2, &[0x41, 0, 0, 0]);
    assert_eq!(u16_at(&r, 0x34) & 0x8000, 0, "GetKeyState reports VK_A up");
    assert!(alpc::wait_for(|| conkbd::pending_reads(&link) == 1, Duration::from_secs(5)));
    assert_eq!(conkbd::detach(&link), 1, "remove-complete delivered to the console's handle registration");
    assert!(alpc::wait_for(|| conkbd::open_handles(&link).is_empty(), Duration::from_secs(5)), "console closed its keyboard handle on removal");
    assert!(alpc::wait_for(|| cfgmgr::handle_registrations(|t| dev.contains(&t)) == 0, Duration::from_secs(5)), "console unregistered the handle notification");
    for h in handles {
        unsafe { console_sandbox::nt_close(h) };
    }
    unsafe { console_sandbox::nt_close(port) };
    assert!(alpc::wait_for(|| alpc::connection_count(PORT_NAME) == 0 && alpc::section_count(PORT_NAME) == 0, Duration::from_secs(5)), "console handled LPC_PORT_CLOSED: disconnected, deleted its view and section");
    assert!(alpc::port_exists(PORT_NAME), "server port stays up for the next client");
}
