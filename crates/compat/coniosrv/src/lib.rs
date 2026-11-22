use core::ffi::c_void;
use std::sync::OnceLock;

use console_image::ConsoleDll;

pub static CONSOLE: ConsoleDll = ConsoleDll::new("ConIoSrv.dll", "TOMBOX_CONIOSRV_CONSOLE", "coniosrv_console.dll", "ServerDllInitialization", &[]);
pub const PORT_NAME: &str = "\\ConsoleInputServerPort";
pub const STATUS_DLL_NOT_FOUND: i32 = 0xC000_0135u32 as i32;
pub const STATUS_NOT_SUPPORTED: i32 = 0xC000_00BBu32 as i32;

type ConsoleInit = unsafe extern "system" fn(*mut c_void) -> i32;

fn own() -> usize {
    console_module as *const () as usize
}

static ATTACHED: OnceLock<Vec<String>> = OnceLock::new();

pub fn console_module() -> usize {
    let m = CONSOLE.module(own());
    if m != 0 {
        ATTACHED.get_or_init(|| unsafe { console_sandbox::attach(m) });
    }
    m
}

pub fn sandbox_hits() -> Vec<String> {
    console_module();
    ATTACHED.get().cloned().unwrap_or_default()
}

pub fn unrouted() -> Vec<String> {
    let m = console_module();
    if m == 0 {
        return Vec::new();
    }
    unsafe { console_sandbox::unrouted(m) }
}

#[no_mangle]
pub unsafe extern "system" fn ServerDllInitialization(server: *mut c_void) -> i32 {
    let m = console_module();
    let Some(init) = console_image::symbol(m, "ServerDllInitialization") else {
        CONSOLE.missing("ServerDllInitialization", STATUS_DLL_NOT_FOUND as u32 as usize);
        return STATUS_DLL_NOT_FOUND;
    };
    let open = unrouted();
    if !open.is_empty() {
        compat_common::record_raw("ConIoSrv.dll", "ServerDllInitialization", &format!("console imports not routed to the sandbox, refusing to run the console code: {}", open.join(", ")));
        return STATUS_NOT_SUPPORTED;
    }
    let init: ConsoleInit = core::mem::transmute(init);
    let st = init(server);
    compat_common::record_raw("ConIoSrv.dll", "ServerDllInitialization", &format!("console ServerDllInitialization -> {st:#010x}"));
    st
}

#[cfg(any(test, feature = "testing"))]
pub mod sandbox_test;
