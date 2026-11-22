use core::ffi::c_void;
use core::ptr::null_mut;
use core::sync::atomic::{AtomicU32, Ordering};

#[repr(C)]
pub struct CtrlVtbl {
    pub destroy: unsafe extern "C" fn(*mut Ctrl),
    pub delete_this: unsafe extern "C" fn(*mut Ctrl),
    pub deleting_dtor: unsafe extern "C" fn(*mut Ctrl, u32) -> *mut Ctrl,
    pub get_deleter: unsafe extern "C" fn(*mut Ctrl, *const c_void) -> *mut c_void,
}

#[repr(C)]
pub struct Ctrl {
    pub vtbl: *const CtrlVtbl,
    pub uses: AtomicU32,
    pub weaks: AtomicU32,
}

impl Ctrl {
    pub const fn new(vtbl: &'static CtrlVtbl) -> Self {
        Ctrl { vtbl, uses: AtomicU32::new(1), weaks: AtomicU32::new(1) }
    }
}

pub unsafe extern "C" fn no_deleter(_c: *mut Ctrl, _t: *const c_void) -> *mut c_void {
    null_mut()
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SharedPtr {
    pub ptr: *mut c_void,
    pub ctrl: *mut Ctrl,
}

impl SharedPtr {
    pub const fn null() -> Self {
        SharedPtr { ptr: null_mut(), ctrl: null_mut() }
    }
}

pub unsafe fn incref(c: *mut Ctrl) {
    if !c.is_null() {
        (*c).uses.fetch_add(1, Ordering::AcqRel);
    }
}

pub unsafe fn incweak(c: *mut Ctrl) {
    if !c.is_null() {
        (*c).weaks.fetch_add(1, Ordering::AcqRel);
    }
}

pub unsafe fn decref(c: *mut Ctrl) {
    if c.is_null() {
        return;
    }
    if (*c).uses.fetch_sub(1, Ordering::AcqRel) == 1 {
        ((*(*c).vtbl).destroy)(c);
        decweak(c);
    }
}

pub unsafe fn decweak(c: *mut Ctrl) {
    if !c.is_null() && (*c).weaks.fetch_sub(1, Ordering::AcqRel) == 1 {
        ((*(*c).vtbl).delete_this)(c);
    }
}

pub unsafe fn lock_weak(w: SharedPtr) -> SharedPtr {
    if w.ctrl.is_null() {
        return SharedPtr::null();
    }
    let u = &(*w.ctrl).uses;
    let mut cur = u.load(Ordering::Acquire);
    loop {
        if cur == 0 {
            return SharedPtr::null();
        }
        match u.compare_exchange(cur, cur + 1, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => return w,
            Err(n) => cur = n,
        }
    }
}

pub struct Strong(pub SharedPtr);

impl Strong {
    pub unsafe fn copy(&self) -> SharedPtr {
        incref(self.0.ctrl);
        self.0
    }
}

impl Drop for Strong {
    fn drop(&mut self) {
        unsafe { decref(self.0.ctrl) };
    }
}

#[repr(C)]
pub struct PtrCtrl {
    pub ctrl: Ctrl,
    pub obj: *mut c_void,
}

unsafe extern "C" fn ptr_destroy(c: *mut Ctrl) {
    let o = (*(c as *mut PtrCtrl)).obj;
    if !o.is_null() {
        let dtor: unsafe extern "C" fn(*mut c_void, u32) -> *mut c_void = core::mem::transmute(*(*(o as *const *const usize)));
        dtor(o, 1);
    }
}

unsafe extern "C" fn ptr_delete_this(c: *mut Ctrl) {
    ptr_deleting_dtor(c, 1);
}

unsafe extern "C" fn ptr_deleting_dtor(c: *mut Ctrl, flags: u32) -> *mut Ctrl {
    if flags & 1 != 0 {
        drop(Box::from_raw(c as *mut PtrCtrl));
    }
    c
}

pub static PTR_CTRL_VTBL: CtrlVtbl = CtrlVtbl { destroy: ptr_destroy, delete_this: ptr_delete_this, deleting_dtor: ptr_deleting_dtor, get_deleter: no_deleter };

pub unsafe fn wrap_object(obj: *mut c_void) -> SharedPtr {
    let c = Box::into_raw(Box::new(PtrCtrl { ctrl: Ctrl::new(&PTR_CTRL_VTBL), obj })) as *mut Ctrl;
    SharedPtr { ptr: obj, ctrl: c }
}
