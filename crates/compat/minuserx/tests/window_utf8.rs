use core::ffi::c_void;
use core::ptr::null_mut;

#[link(name = "user32")]
extern "system" {
    fn GetWindowTextW(h: *mut c_void, buf: *mut u16, n: i32) -> i32;
    fn DestroyWindow(h: *mut c_void) -> i32;
    fn DefWindowProcW(h: *mut c_void, m: u32, w: usize, l: isize) -> isize;
    fn IsWindowUnicode(h: *mut c_void) -> i32;
}

unsafe extern "system" fn wndproc(h: *mut c_void, m: u32, w: usize, l: isize) -> isize {
    DefWindowProcW(h, m, w, l)
}

#[test]
fn ansi_window_titles_are_utf8_and_unicode() {
    let mut wc = [0u8; 80];
    wc[0..4].copy_from_slice(&80u32.to_le_bytes());
    wc[8..16].copy_from_slice(&(wndproc as usize as u64).to_le_bytes());
    let cls = b"TbUtf8Class\0";
    wc[64..72].copy_from_slice(&(cls.as_ptr() as u64).to_le_bytes());
    unsafe {
        assert_ne!(minuserx::mux_RegisterClassExA(wc.as_ptr()), 0);
        let name = "Trackmania \u{e9}t\u{e9}\0";
        let h = minuserx::mux_CreateWindowExA(0, cls.as_ptr(), name.as_ptr(), 0, 0, 0, 10, 10, null_mut(), null_mut(), null_mut(), null_mut());
        assert!(!h.is_null());
        assert_ne!(IsWindowUnicode(h), 0);
        let mut buf = [0u16; 64];
        let n = GetWindowTextW(h, buf.as_mut_ptr(), 64) as usize;
        let got = String::from_utf16_lossy(&buf[..n]);
        let want = if minuserx::test_run() { format!("{}Trackmania \u{e9}t\u{e9}", minuserx::TEST_TITLE_PREFIX) } else { "Trackmania \u{e9}t\u{e9}".to_string() };
        assert_eq!(got, want);
        DestroyWindow(h);
    }
}

#[test]
fn test_runs_prefix_titles_and_never_activate() {
    let name: Vec<u16> = "Terraria\0".encode_utf16().collect();
    let t = minuserx::window_title(Some(&name), true).unwrap();
    assert_eq!(String::from_utf16_lossy(&t[..t.len() - 1]), "[TEST RUN - input disabled] Terraria");
    assert_eq!(minuserx::window_title(Some(&name), false).unwrap(), name);
    assert_eq!(minuserx::show_cmd(5, true), 8);
    assert_eq!(minuserx::show_cmd(1, true), 4);
    assert_eq!(minuserx::show_cmd(0, true), 0);
    assert_eq!(minuserx::show_cmd(5, false), 5);
}
