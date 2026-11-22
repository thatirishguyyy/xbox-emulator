use compat_common::gdk::{self, net, xasync};
use core::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};

unsafe fn slot(clsid: gdk::Guid, iid: gdk::Guid, slot: usize) -> (*mut c_void, usize) {
    let mut obj: *mut c_void = core::ptr::null_mut();
    assert_eq!(gdk::query_api_impl(&clsid, &iid, &mut obj), 0);
    let vt = *(obj as *const *const usize);
    (obj, *vt.add(slot))
}

static IMMEDIATE_HITS: AtomicUsize = AtomicUsize::new(0);
static IMMEDIATE_INIT: AtomicUsize = AtomicUsize::new(0);
static MANUAL_HITS: AtomicUsize = AtomicUsize::new(0);
static DROPPED_HITS: AtomicUsize = AtomicUsize::new(0);

unsafe extern "system" fn on_immediate(ctx: *mut c_void, hint: *const u8) {
    assert_eq!(ctx as usize, 0x77);
    IMMEDIATE_INIT.store(*hint.add(12) as usize, Ordering::SeqCst);
    IMMEDIATE_HITS.fetch_add(1, Ordering::SeqCst);
}

unsafe extern "system" fn on_manual(_ctx: *mut c_void, hint: *const u8) {
    assert_eq!(*hint.add(12), 1);
    assert_eq!(u32::from_le_bytes(core::slice::from_raw_parts(hint, 4).try_into().unwrap()), net::connectivity_level());
    MANUAL_HITS.fetch_add(1, Ordering::SeqCst);
}

unsafe extern "system" fn on_dropped(_ctx: *mut c_void, _hint: *const u8) {
    DROPPED_HITS.fetch_add(1, Ordering::SeqCst);
}

#[test]
fn connectivity_hint_registration_delivers_current_hint_once() {
    std::env::set_var("TOMBOX_GDK_XASYNC", "emulated");
    std::env::set_var("TOMBOX_GDK_ROOT", std::env::temp_dir().join(format!("tombox_gdk_net_hint_{}", std::process::id())));
    assert!(gdk::rt::active());
    unsafe {
        let (qo, create) = slot(xasync::XASYNC, xasync::XASYNC, 12);
        let create: unsafe extern "system" fn(*mut c_void, u32, u32, *mut usize) -> i32 = core::mem::transmute(create);
        let (_, dispatch) = slot(xasync::XASYNC, xasync::XASYNC, 16);
        let dispatch: unsafe extern "system" fn(*mut c_void, usize, u32, u32) -> bool = core::mem::transmute(dispatch);
        let (_, close) = slot(xasync::XASYNC, xasync::XASYNC, 17);
        let close: unsafe extern "system" fn(*mut c_void, usize) = core::mem::transmute(close);
        let (no, register) = slot(net::CLSID, net::IID_V2, 16);
        let register: unsafe extern "system" fn(*mut c_void, usize, *mut c_void, unsafe extern "system" fn(*mut c_void, *const u8), *mut u64) -> i32 = core::mem::transmute(register);
        let (_, unregister) = slot(net::CLSID, net::IID_V2, 17);
        let unregister: unsafe extern "system" fn(*mut c_void, u64, bool) -> bool = core::mem::transmute(unregister);

        let mut imm = 0usize;
        assert_eq!(create(qo, 3, 3, &mut imm), 0);
        let mut t1 = 0u64;
        assert_eq!(register(no, imm, 0x77 as *mut c_void, on_immediate, &mut t1), 0);
        assert_ne!(t1, 0);
        assert_eq!(IMMEDIATE_HITS.load(Ordering::SeqCst), 1, "immediate queue must deliver the current hint during registration");
        assert_eq!(IMMEDIATE_INIT.load(Ordering::SeqCst), 1, "networkInitialized must be true");

        let mut man = 0usize;
        assert_eq!(create(qo, 0, 0, &mut man), 0);
        let mut t2 = 0u64;
        assert_eq!(register(no, man, core::ptr::null_mut(), on_manual, &mut t2), 0);
        assert_eq!(MANUAL_HITS.load(Ordering::SeqCst), 0, "manual queue ran the hint callback without dispatch");
        assert_eq!(IMMEDIATE_HITS.load(Ordering::SeqCst), 1, "a new registration re-notified an older registrant");
        assert!(dispatch(qo, man, 1, 0));
        assert_eq!(MANUAL_HITS.load(Ordering::SeqCst), 1);
        assert!(!dispatch(qo, man, 1, 0));
        assert_eq!(MANUAL_HITS.load(Ordering::SeqCst), 1);

        let mut t3 = 0u64;
        assert_eq!(register(no, man, core::ptr::null_mut(), on_dropped, &mut t3), 0);
        assert!(unregister(no, t3, false));
        while dispatch(qo, man, 1, 0) {}
        assert_eq!(DROPPED_HITS.load(Ordering::SeqCst), 0, "hint delivered after unregister");

        assert!(unregister(no, t1, false));
        assert!(unregister(no, t2, false));
        net::notify_connectivity_changed();
        while dispatch(qo, man, 1, 0) {}
        assert_eq!(IMMEDIATE_HITS.load(Ordering::SeqCst), 1);
        assert_eq!(MANUAL_HITS.load(Ordering::SeqCst), 1);
        close(qo, imm);
        close(qo, man);
    }
}
