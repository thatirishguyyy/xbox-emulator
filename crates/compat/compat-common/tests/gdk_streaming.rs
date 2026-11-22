use compat_common::gdk::{self, streaming};
use core::ffi::c_void;

unsafe fn method(slot: usize) -> (*mut c_void, usize) {
    let mut obj: *mut c_void = core::ptr::null_mut();
    assert_eq!(gdk::query_api_impl(&streaming::CLSID, &streaming::IID_5F5E, &mut obj), 0);
    (obj, *(*(obj as *const *const usize)).add(slot))
}

unsafe extern "system" fn on_props(_ctx: *mut c_void, _client: u64, _n: u32, _p: *const u32) {}
unsafe extern "system" fn on_state(_ctx: *mut c_void, _client: u64, _state: u32) {}

#[test]
fn xgamestreaming_slots_follow_the_console_runtime_without_clients() {
    std::env::set_var("TOMBOX_GDK_ROOT", std::env::temp_dir().join(format!("tombox_gdk_stream_{}", std::process::id())));
    unsafe {
        let (o, init) = method(3);
        let init: unsafe extern "system" fn(*mut c_void) -> i32 = core::mem::transmute(init);
        let (_, uninit) = method(4);
        let uninit: unsafe extern "system" fn(*mut c_void) = core::mem::transmute(uninit);
        let (_, streaming_now) = method(5);
        let streaming_now: unsafe extern "system" fn(*mut c_void) -> bool = core::mem::transmute(streaming_now);
        let (_, reg_props) = method(6);
        let reg_props: unsafe extern "system" fn(*mut c_void, u64, usize, *mut c_void, usize, *mut u64) -> i32 = core::mem::transmute(reg_props);
        let (_, unreg_props) = method(7);
        let unreg_props: unsafe extern "system" fn(*mut c_void, u64, u64, bool) -> bool = core::mem::transmute(unreg_props);
        let (_, dims) = method(8);
        let dims: unsafe extern "system" fn(*mut c_void, u64, *mut u32, *mut u32) -> i32 = core::mem::transmute(dims);
        let (_, count) = method(9);
        let count: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(count);
        let (_, reg_state) = method(12);
        let reg_state: unsafe extern "system" fn(*mut c_void, usize, *mut c_void, usize, *mut u64) -> i32 = core::mem::transmute(reg_state);
        let (_, unreg_state) = method(13);
        let unreg_state: unsafe extern "system" fn(*mut c_void, u64, bool) -> bool = core::mem::transmute(unreg_state);
        let (_, hide) = method(17);
        let hide: unsafe extern "system" fn(*mut c_void) = core::mem::transmute(hide);

        let props = on_props as *const () as usize;
        let state = on_state as *const () as usize;
        let mut tok = 77u64;
        assert_eq!(reg_props(o, 1, 0, core::ptr::null_mut(), props, &mut tok), streaming::E_GAMESTREAMING_NOT_INITIALIZED);
        assert_eq!(tok, 0);
        assert!(!unreg_props(o, 1, 1, true));
        let (mut h, mut v) = (5u32, 6u32);
        assert_eq!(dims(o, 1, &mut h, &mut v), streaming::E_GAMESTREAMING_NOT_INITIALIZED);
        assert_eq!((h, v), (0, 0));
        tok = 77;
        assert_eq!(reg_state(o, 0, core::ptr::null_mut(), state, &mut tok), streaming::E_GAMESTREAMING_NOT_INITIALIZED);
        assert_eq!(tok, 0);
        assert!(!unreg_state(o, 1, false));
        assert!(!streaming_now(o));
        assert_eq!(count(o), 0);

        assert_eq!(init(o), 0);
        assert_eq!(init(o), 0);
        assert!(!streaming_now(o));
        assert_eq!(count(o), 0);

        assert_eq!(reg_props(o, 1, 0, core::ptr::null_mut(), props, &mut tok), streaming::E_GAMESTREAMING_CLIENT_NOT_CONNECTED);
        assert_eq!(reg_props(o, 0, 0, core::ptr::null_mut(), props, &mut tok), streaming::E_GAMESTREAMING_CLIENT_NOT_CONNECTED);
        assert_eq!(reg_props(o, streaming::ALL_CLIENTS, 0, core::ptr::null_mut(), props, &mut tok), 0x8007_0057u32 as i32);
        assert_eq!(reg_props(o, 1, 0, core::ptr::null_mut(), 0, &mut tok), streaming::E_POINTER);
        assert!(unreg_props(o, 1, 12345, true));

        assert_eq!(dims(o, 0, &mut h, &mut v), streaming::E_GAMESTREAMING_CLIENT_NOT_CONNECTED);
        assert_eq!(dims(o, 7, &mut h, &mut v), streaming::E_GAMESTREAMING_CLIENT_NOT_CONNECTED);
        assert_eq!(dims(o, streaming::ALL_CLIENTS, &mut h, &mut v), 0x8007_0057u32 as i32);

        assert_eq!(reg_state(o, 0, core::ptr::null_mut(), 0, &mut tok), streaming::E_POINTER);
        assert_eq!(reg_state(o, 0, core::ptr::null_mut(), state, core::ptr::null_mut()), streaming::E_POINTER);
        let mut t1 = 0u64;
        let mut t2 = 0u64;
        assert_eq!(reg_state(o, 0, core::ptr::null_mut(), state, &mut t1), 0);
        assert_eq!(reg_state(o, 0, core::ptr::null_mut(), state, &mut t2), 0);
        assert!(t1 >= 1);
        assert_eq!(t2, t1 + 1);
        assert!(unreg_state(o, t1, true));
        assert!(unreg_state(o, t1, false));
        assert!(unreg_state(o, 0xdead, false));

        hide(o);

        assert_eq!(streaming::registration_count(), 1);
        let mut t3 = 0u64;
        assert_eq!(reg_state(o, 0, core::ptr::null_mut(), state, &mut t3), 0);
        assert_eq!(t3, t2 + 1);
        assert_eq!(streaming::registration_count(), 2);
        uninit(o);
        assert_eq!(streaming::registration_count(), 0);
        assert!(!unreg_state(o, t2, true));
        assert_eq!(dims(o, 7, &mut h, &mut v), streaming::E_GAMESTREAMING_NOT_INITIALIZED);
        assert_eq!(init(o), 0);
        assert_eq!(streaming::registration_count(), 0);
        let mut t4 = 0u64;
        assert_eq!(reg_state(o, 0, core::ptr::null_mut(), state, &mut t4), 0);
        assert_eq!(t4, t3 + 1);
        assert_eq!(streaming::registration_count(), 1);
        uninit(o);
        assert_eq!(streaming::registration_count(), 0);
    }
}
