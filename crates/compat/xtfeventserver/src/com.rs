use crate::sys::*;
use core::ffi::c_void;
use core::mem::transmute;
use core::ptr::null_mut;

pub type Obj = *mut c_void;

pub unsafe fn slot(p: Obj, i: usize) -> usize {
    *(*(p as *const *const usize)).add(i)
}

pub unsafe fn query_interface(p: Obj, iid: *const GUID, out: *mut Obj) -> HRESULT {
    let f: unsafe extern "system" fn(Obj, *const GUID, *mut Obj) -> HRESULT = transmute(slot(p, 0));
    f(p, iid, out)
}

pub unsafe fn add_ref(p: Obj) -> u32 {
    let f: unsafe extern "system" fn(Obj) -> u32 = transmute(slot(p, 1));
    f(p)
}

pub unsafe fn release(p: Obj) -> u32 {
    let f: unsafe extern "system" fn(Obj) -> u32 = transmute(slot(p, 2));
    f(p)
}

pub struct Ptr(pub Obj);

impl Ptr {
    pub fn null() -> Self {
        Ptr(null_mut())
    }

    pub fn reset(&mut self) {
        let p = core::mem::replace(&mut self.0, null_mut());
        if !p.is_null() {
            unsafe { release(p) };
        }
    }

    pub fn out(&mut self) -> *mut Obj {
        self.reset();
        &mut self.0
    }

    pub fn detach(&mut self) -> Obj {
        core::mem::replace(&mut self.0, null_mut())
    }
}

impl Drop for Ptr {
    fn drop(&mut self) {
        self.reset();
    }
}

pub struct Bstr(pub BSTR);

impl Bstr {
    pub fn null() -> Self {
        Bstr(null_mut())
    }

    pub fn reset(&mut self) {
        let p = core::mem::replace(&mut self.0, null_mut());
        if !p.is_null() {
            unsafe { SysFreeString(p) };
        }
    }

    pub fn out(&mut self) -> *mut BSTR {
        self.reset();
        &mut self.0
    }
}

impl Drop for Bstr {
    fn drop(&mut self) {
        self.reset();
    }
}

pub unsafe fn obj_get_number(o: Obj, name: *const u16, out: *mut f64) -> HRESULT {
    let f: unsafe extern "system" fn(Obj, *const u16, *mut f64) -> HRESULT = transmute(slot(o, 3));
    f(o, name, out)
}

pub unsafe fn obj_set_number(o: Obj, name: *const u16, v: f64) -> HRESULT {
    let f: unsafe extern "system" fn(Obj, *const u16, f64) -> HRESULT = transmute(slot(o, 4));
    f(o, name, v)
}

pub unsafe fn obj_get_string(o: Obj, name: *const u16, out: *mut BSTR) -> HRESULT {
    let f: unsafe extern "system" fn(Obj, *const u16, *mut BSTR) -> HRESULT = transmute(slot(o, 5));
    f(o, name, out)
}

pub unsafe fn obj_set_string(o: Obj, name: *const u16, v: *const u16) -> HRESULT {
    let f: unsafe extern "system" fn(Obj, *const u16, *const u16) -> HRESULT = transmute(slot(o, 6));
    f(o, name, v)
}

pub unsafe fn obj_set_object(o: Obj, name: *const u16, v: Obj) -> HRESULT {
    let f: unsafe extern "system" fn(Obj, *const u16, Obj) -> HRESULT = transmute(slot(o, 9));
    f(o, name, v)
}

pub unsafe fn obj_stringify(o: Obj, out: *mut BSTR) -> HRESULT {
    let f: unsafe extern "system" fn(Obj, *mut BSTR) -> HRESULT = transmute(slot(o, 12));
    f(o, out)
}

pub unsafe fn msg_get_json_body(m: Obj, count: *mut u32, body: *mut *mut u8) -> HRESULT {
    let f: unsafe extern "system" fn(Obj, *mut u32, *mut *mut u8) -> HRESULT = transmute(slot(m, 3));
    f(m, count, body)
}

pub unsafe fn reply_set_json_body_w(r: Obj, s: *const u16) -> HRESULT {
    let f: unsafe extern "system" fn(Obj, *const u16) -> HRESULT = transmute(slot(r, 6));
    f(r, s)
}

pub unsafe fn msg_set_json_body_w(m: Obj, s: *const u16) -> HRESULT {
    let f: unsafe extern "system" fn(Obj, *const u16) -> HRESULT = transmute(slot(m, 7));
    f(m, s)
}

pub unsafe fn msg_set_binary_body(m: Obj, n: u32, data: *const u8) -> HRESULT {
    let f: unsafe extern "system" fn(Obj, u32, *const u8) -> HRESULT = transmute(slot(m, 9));
    f(m, n, data)
}

pub unsafe fn session_send_more(s: Obj, m: Obj) -> HRESULT {
    let f: unsafe extern "system" fn(Obj, Obj) -> HRESULT = transmute(slot(s, 3));
    f(s, m)
}

pub unsafe fn props_get_property(p: Obj, name: *const u16, out: *mut BSTR) -> HRESULT {
    let f: unsafe extern "system" fn(Obj, *const u16, *mut BSTR) -> HRESULT = transmute(slot(p, 3));
    f(p, name, out)
}

pub type HandlerFn = unsafe extern "system" fn(usize, usize, usize) -> HRESULT;

pub unsafe fn md_add_function(m: Obj, id: u32, func: HandlerFn) -> HRESULT {
    let f: unsafe extern "system" fn(Obj, u32, HandlerFn) -> HRESULT = transmute(slot(m, 6));
    f(m, id, func)
}
