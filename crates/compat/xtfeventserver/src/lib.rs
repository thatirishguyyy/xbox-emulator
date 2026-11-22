#![allow(non_snake_case, clippy::missing_safety_doc)]

pub mod sys;
pub mod com;
pub mod imports;
pub mod shared;
pub mod dispatcher;
pub mod etw;
pub mod server;

#[cfg(test)]
mod tests;

use core::ffi::c_void;
use std::sync::Once;
use xtfcommon::msvc::{operator_delete, operator_new};
use xtfcommon::sys::atexit;

#[repr(C)]
pub struct TreeNode {
    pub left: *mut TreeNode,
    pub parent: *mut TreeNode,
    pub right: *mut TreeNode,
    pub color: u8,
    pub isnil: u8,
}

#[repr(C)]
pub struct EventSourceMap {
    pub head: *mut TreeNode,
    pub size: usize,
}

pub const NODE_SIZE: usize = 0x80;

static mut MAP: EventSourceMap = EventSourceMap { head: core::ptr::null_mut(), size: 0 };
static INIT: Once = Once::new();

unsafe fn erase(mut n: *mut TreeNode) {
    while (*n).isnil == 0 {
        erase((*n).right);
        let l = (*n).left;
        operator_delete(n as *mut c_void);
        n = l;
    }
}

unsafe extern "C" fn destroy_map() {
    let m = core::ptr::addr_of_mut!(MAP);
    let head = (*m).head;
    if head.is_null() {
        return;
    }
    erase((*head).parent);
    operator_delete(head as *mut c_void);
    (*m).head = core::ptr::null_mut();
    (*m).size = 0;
}

#[no_mangle]
pub extern "C" fn GetEventSourceMap() -> *mut EventSourceMap {
    INIT.call_once(|| unsafe {
        let h = operator_new(NODE_SIZE) as *mut TreeNode;
        (*h).left = h;
        (*h).parent = h;
        (*h).right = h;
        (*h).color = 1;
        (*h).isnil = 1;
        let m = core::ptr::addr_of_mut!(MAP);
        (*m).head = h;
        atexit(destroy_map);
    });
    core::ptr::addr_of_mut!(MAP)
}

#[no_mangle]
pub unsafe extern "C" fn XtfOrdinal_4(extension: *const sys::GUID, iid: *const sys::GUID, ppv: *mut *mut c_void) -> sys::HRESULT {
    server::extension_entry_point(extension, iid, ppv)
}

#[cfg(test)]
mod map_tests {
    use super::*;

    #[test]
    fn get_event_source_map_is_empty_msvc_tree() {
        let a = GetEventSourceMap();
        let b = GetEventSourceMap();
        assert_eq!(a, b);
        unsafe {
            let h = (*a).head;
            assert!(!h.is_null());
            assert_eq!((*h).left, h);
            assert_eq!((*h).parent, h);
            assert_eq!((*h).right, h);
            assert_eq!(((*h).color, (*h).isnil), (1, 1));
            assert_eq!((*a).size, 0);
        }
    }

    #[test]
    fn export_table_matches_console_dll() {
        let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/exports.txt")).unwrap();
        xtfcommon::exports::check_against_titles("xtfeventserver.dll", &text);
        for e in xtfcommon::exports::parse(&text) {
            if e.status != "missing" {
                assert!(xtfcommon::exports::source_defines(concat!(env!("CARGO_MANIFEST_DIR"), "/src"), &e));
            }
        }
    }
}
