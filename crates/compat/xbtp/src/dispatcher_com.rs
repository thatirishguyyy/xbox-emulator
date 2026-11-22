use crate::dispatcher::*;
use crate::msvc::SharedPtr;
use crate::registry::{new_definition, new_router, XbtpExtensionRegistry_AddExtension, XbtpExtensionRegistry_RegisterExtension};
use crate::sys::*;
use crate::xbtp2_json::IID_IUNKNOWN;
use core::ffi::c_void;
use core::ptr::null_mut;
use core::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

#[repr(C)]
pub struct DispatcherComVtbl {
    pub query_interface: unsafe extern "system" fn(*mut DispatcherCom, *const GUID, *mut *mut c_void) -> HRESULT,
    pub add_ref: unsafe extern "system" fn(*mut DispatcherCom) -> u32,
    pub release: unsafe extern "system" fn(*mut DispatcherCom) -> u32,
    pub shutdown: unsafe extern "system" fn(*mut DispatcherCom) -> HRESULT,
    pub add_extension: unsafe extern "system" fn(*mut DispatcherCom, *const GUID, *mut c_void) -> HRESULT,
    pub register_extension: unsafe extern "system" fn(*mut DispatcherCom, *const GUID, *const u16) -> HRESULT,
    pub register_redirect: unsafe extern "system" fn(*mut DispatcherCom, u32, *const u16, u32) -> HRESULT,
    pub set_session_key: unsafe extern "system" fn(*mut DispatcherCom, *const u16) -> HRESULT,
}

#[repr(C)]
pub struct DispatcherCom {
    pub vtbl: *const DispatcherComVtbl,
    pub refs: AtomicU32,
    pub lock: Mutex<()>,
    pub dispatcher: SharedPtr<XbtpConnectionDispatcher>,
}

pub static DISPATCHER_COM_VTBL: DispatcherComVtbl = DispatcherComVtbl {
    query_interface: dc_qi,
    add_ref: dc_add_ref,
    release: dc_release,
    shutdown: dc_shutdown,
    add_extension: dc_add_extension,
    register_extension: dc_register_extension,
    register_redirect: dc_register_redirect,
    set_session_key: dc_set_session_key,
};

pub unsafe fn wrap_dispatcher(dispatcher: SharedPtr<XbtpConnectionDispatcher>, iid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    let c = Box::into_raw(Box::new(DispatcherCom { vtbl: &DISPATCHER_COM_VTBL, refs: AtomicU32::new(1), lock: Mutex::new(()), dispatcher }));
    let hr = dc_qi(c, iid, out);
    dc_release(c);
    hr
}

#[export_name = "?XbtpWrapDispatcher@@YAJV?$shared_ptr@VXbtpConnectionDispatcher@@@std@@AEBU_GUID@@PEAPEAX@Z"]
pub unsafe extern "C" fn XbtpWrapDispatcher(dispatcher: *mut SharedPtr<XbtpConnectionDispatcher>, iid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    let d = SharedPtr::take(dispatcher);
    if d.ptr.is_null() || out.is_null() {
        drop(d);
        return E_INVALIDARG;
    }
    wrap_dispatcher(d, iid, out)
}

unsafe extern "system" fn dc_qi(this: *mut DispatcherCom, iid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    if *iid == IID_XBTP_DISPATCHER || *iid == IID_IUNKNOWN || *iid == IID_XBTP_DISPATCHER_ALT {
        *out = this as *mut c_void;
        dc_add_ref(this);
        S_OK
    } else {
        *out = null_mut();
        E_NOINTERFACE
    }
}

unsafe extern "system" fn dc_add_ref(this: *mut DispatcherCom) -> u32 {
    (*this).refs.fetch_add(1, Ordering::AcqRel) + 1
}

unsafe extern "system" fn dc_release(this: *mut DispatcherCom) -> u32 {
    let n = (*this).refs.fetch_sub(1, Ordering::AcqRel) - 1;
    if n == 0 {
        drop(Box::from_raw(this));
    }
    n
}

unsafe extern "system" fn dc_shutdown(this: *mut DispatcherCom) -> HRESULT {
    let _g = (*this).lock.lock().unwrap_or_else(|e| e.into_inner());
    XbtpConnectionDispatcher_Shutdown((*this).dispatcher.ptr);
    S_OK
}

unsafe extern "system" fn dc_add_extension(this: *mut DispatcherCom, id: *const GUID, metadata: *mut c_void) -> HRESULT {
    let _g = (*this).lock.lock().unwrap_or_else(|e| e.into_inner());
    if metadata.is_null() || id.is_null() {
        return E_INVALIDARG;
    }
    let d = match dispatcher_impl((*this).dispatcher.ptr) {
        Some(d) => d,
        None => return E_INVALIDARG,
    };
    let router = match new_router() {
        Some(r) => r,
        None => return E_OUTOFMEMORY,
    };
    let hr = (*router.ptr).initialize(metadata);
    if hr < 0 {
        return hr;
    }
    let mut def = match new_definition(*id, router, None) {
        Some(x) => x,
        None => return E_OUTOFMEMORY,
    };
    XbtpExtensionRegistry_AddExtension(d.registry.ptr, &mut def)
}

unsafe extern "system" fn dc_register_extension(this: *mut DispatcherCom, id: *const GUID, path: *const u16) -> HRESULT {
    let _g = (*this).lock.lock().unwrap_or_else(|e| e.into_inner());
    let d = match dispatcher_impl((*this).dispatcher.ptr) {
        Some(d) => d,
        None => return E_INVALIDARG,
    };
    XbtpExtensionRegistry_RegisterExtension(d.registry.ptr, id, path)
}

unsafe extern "system" fn dc_register_redirect(this: *mut DispatcherCom, port: u32, address: *const u16, redirect_port: u32) -> HRESULT {
    let d = match dispatcher_impl((*this).dispatcher.ptr) {
        Some(d) => d,
        None => return E_INVALIDARG,
    };
    crate::dispatcher::add_redirect(&d, port, address, redirect_port)
}

unsafe extern "system" fn dc_set_session_key(this: *mut DispatcherCom, key: *const u16) -> HRESULT {
    let _g = (*this).lock.lock().unwrap_or_else(|e| e.into_inner());
    if !key.is_null() && wcslen(key) > MAX_SESSION_KEY {
        return S_OK;
    }
    if let Some(d) = dispatcher_impl((*this).dispatcher.ptr) {
        set_session_key(&d, key);
    }
    S_OK
}
