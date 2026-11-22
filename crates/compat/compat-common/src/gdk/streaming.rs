use super::{log, Guid, Iface, This, E_INVALIDARG, S_OK};
use core::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

pub const CLSID: Guid = Guid::parse("0a2192aa-b2d5-4d58-83be-383b6d80799e");
pub const IID_5F5E: Guid = Guid::parse("5f5e5169-746c-4001-ad1c-da728d01c9eb");

pub const E_GAMESTREAMING_NOT_INITIALIZED: i32 = 0x8924_5400u32 as i32;
pub const E_GAMESTREAMING_CLIENT_NOT_CONNECTED: i32 = 0x8924_5401u32 as i32;
pub const E_POINTER: i32 = 0x8000_4003u32 as i32;
pub const E_NO_TASK_QUEUE: i32 = 0x8007_01ABu32 as i32;
pub const ALL_CLIENTS: u64 = u64::MAX;

static INITIALIZED: AtomicBool = AtomicBool::new(false);
static NEXT_CONNECTION_TOKEN: AtomicU64 = AtomicU64::new(1);
static NEXT_PROPERTIES_TOKEN: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Connection,
    Properties(u64),
}

fn registrations() -> &'static Mutex<Vec<(Kind, u64)>> {
    static R: Mutex<Vec<(Kind, u64)>> = Mutex::new(Vec::new());
    &R
}

pub fn registration_count() -> usize {
    registrations().lock().unwrap_or_else(|e| e.into_inner()).len()
}

pub fn initialized() -> bool {
    INITIALIZED.load(Ordering::Acquire)
}

fn lookup_client(client: u64) -> i32 {
    if !initialized() {
        return E_GAMESTREAMING_NOT_INITIALIZED;
    }
    match client {
        0 => E_GAMESTREAMING_CLIENT_NOT_CONNECTED,
        ALL_CLIENTS => E_INVALIDARG,
        _ => E_GAMESTREAMING_CLIENT_NOT_CONNECTED,
    }
}

unsafe extern "system" fn initialize(_t: This) -> i32 {
    if !INITIALIZED.swap(true, Ordering::AcqRel) {
        log("XGameStreamingInitialize", "initialized; this PC session has no streaming clients");
    }
    S_OK
}

unsafe extern "system" fn uninitialize(_t: This) {
    let mut r = registrations().lock().unwrap_or_else(|e| e.into_inner());
    INITIALIZED.store(false, Ordering::Release);
    r.clear();
}

unsafe extern "system" fn is_streaming(_t: This) -> bool {
    false
}

unsafe extern "system" fn client_count(_t: This) -> u32 {
    0
}

unsafe fn register(kind: Kind, cb: usize, token: *mut u64) -> i32 {
    if cb == 0 || token.is_null() {
        return E_POINTER;
    }
    let counter = match kind {
        Kind::Connection => &NEXT_CONNECTION_TOKEN,
        Kind::Properties(_) => &NEXT_PROPERTIES_TOKEN,
    };
    let t = counter.fetch_add(1, Ordering::AcqRel);
    registrations().lock().unwrap_or_else(|e| e.into_inner()).push((kind, t));
    *token = t;
    S_OK
}

fn unregister(kind: Kind, token: u64) -> bool {
    if !initialized() {
        return false;
    }
    registrations().lock().unwrap_or_else(|e| e.into_inner()).retain(|r| *r != (kind, token));
    true
}

unsafe extern "system" fn register_client_properties(_t: This, client: u64, _queue: usize, _ctx: *mut c_void, cb: usize, token: *mut u64) -> i32 {
    if !token.is_null() {
        *token = 0;
    }
    if !initialized() {
        return E_GAMESTREAMING_NOT_INITIALIZED;
    }
    if client == ALL_CLIENTS {
        return E_INVALIDARG;
    }
    if cb == 0 {
        return E_POINTER;
    }
    E_GAMESTREAMING_CLIENT_NOT_CONNECTED
}

unsafe extern "system" fn unregister_client_properties(_t: This, client: u64, token: u64, _wait: bool) -> bool {
    unregister(Kind::Properties(client), token)
}

unsafe extern "system" fn physical_dimensions(_t: This, client: u64, horizontal: *mut u32, vertical: *mut u32) -> i32 {
    if !horizontal.is_null() {
        *horizontal = 0;
    }
    if !vertical.is_null() {
        *vertical = 0;
    }
    lookup_client(client)
}

unsafe extern "system" fn register_connection_state(_t: This, queue: usize, _ctx: *mut c_void, cb: usize, token: *mut u64) -> i32 {
    if !token.is_null() {
        *token = 0;
    }
    if !initialized() {
        return E_GAMESTREAMING_NOT_INITIALIZED;
    }
    if cb == 0 || token.is_null() {
        return E_POINTER;
    }
    if queue == 0 && !super::rt::process_queue_available() {
        return E_NO_TASK_QUEUE;
    }
    register(Kind::Connection, cb, token)
}

unsafe extern "system" fn unregister_connection_state(_t: This, token: u64, _wait: bool) -> bool {
    unregister(Kind::Connection, token)
}

unsafe extern "system" fn hide_touch_controls(_t: This) {
    log("XGameStreamingHideTouchControls", "no streaming clients; nothing to hide");
}

pub fn interfaces() -> Vec<Iface> {
    vec![Iface {
        clsid: CLSID,
        iid: IID_5F5E,
        name: "XGameStreaming(5f5e5169)",
        slots: vec![
            (3, initialize as usize),
            (4, uninitialize as usize),
            (5, is_streaming as usize),
            (6, register_client_properties as usize),
            (7, unregister_client_properties as usize),
            (8, physical_dimensions as usize),
            (9, client_count as usize),
            (12, register_connection_state as usize),
            (13, unregister_connection_state as usize),
            (17, hide_touch_controls as usize),
        ],
    }]
}
