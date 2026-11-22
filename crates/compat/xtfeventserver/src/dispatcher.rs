use crate::com::*;
use crate::imports::{self, chk};
use crate::shared::*;
use crate::sys::*;
use core::ffi::c_void;
use core::mem::ManuallyDrop;
use core::ptr::null_mut;
use core::sync::atomic::{AtomicBool, Ordering};
use std::collections::VecDeque;
use std::sync::Mutex;

pub const KEEP_ALIVE_MS: u32 = 30000;
pub const IID_XBTP_MESSAGE_IMPL: GUID = GUID::new(0x6965e04a, 0x2829, 0x41a8, [0xb1, 0x94, 0x48, 0x73, 0x15, 0x07, 0x5e, 0x7a]);
pub const EVENT_TYPE: &str = "EventType";
pub const EVENT_DATA: &str = "EventData";
pub const KEEP_ALIVE_EVENT: &str = "KeepAliveEvent";

const F_HANDLE: &[u8] = b"XtfEventServer::EventMessageDispatcher::HandleEvents\0";
const F_QUEUE: &[u8] = b"XtfEventServer::EventMessageDispatcher::QueueJsonWithBinaryEvent\0";

pub struct Event {
    pub name: Vec<u16>,
    pub binary: Vec<u8>,
    pub data: Ptr,
}

#[repr(C)]
pub struct DispatcherVtbl {
    pub dtor: unsafe extern "C" fn(*mut Dispatcher, u32) -> *mut Dispatcher,
    pub queue_json_event: unsafe extern "C" fn(*mut Dispatcher, *const u16, Obj) -> HRESULT,
    pub queue_binary_event: unsafe extern "C" fn(*mut Dispatcher, *const u16, u32, *const u8) -> HRESULT,
    pub queue_json_with_binary_event: unsafe extern "C" fn(*mut Dispatcher, *const u16, Obj, u32, *const u8) -> HRESULT,
}

#[repr(C)]
pub struct Dispatcher {
    pub vtbl: *const DispatcherVtbl,
    pub initialized: AtomicBool,
    pub session: Obj,
    pub event: HANDLE,
    pub queue: Mutex<VecDeque<Event>>,
}

unsafe impl Send for Dispatcher {}
unsafe impl Sync for Dispatcher {}

impl Drop for Dispatcher {
    fn drop(&mut self) {
        let q = core::mem::take(&mut *self.queue.lock().unwrap_or_else(|e| e.into_inner()));
        drop(q);
        let s = core::mem::replace(&mut self.session, null_mut());
        if !s.is_null() {
            unsafe { release(s) };
        }
        if !self.event.is_null() {
            unsafe { CloseHandle(self.event) };
            self.event = null_mut();
        }
    }
}

#[repr(C)]
pub struct DispatcherBlock {
    pub ctrl: Ctrl,
    pub disp: ManuallyDrop<Dispatcher>,
}

unsafe extern "C" fn block_destroy(c: *mut Ctrl) {
    ManuallyDrop::drop(&mut (*(c as *mut DispatcherBlock)).disp);
}

unsafe extern "C" fn block_delete_this(c: *mut Ctrl) {
    block_deleting_dtor(c, 1);
}

unsafe extern "C" fn block_deleting_dtor(c: *mut Ctrl, flags: u32) -> *mut Ctrl {
    if flags & 1 != 0 {
        drop(Box::from_raw(c as *mut DispatcherBlock));
    }
    c
}

static BLOCK_VTBL: CtrlVtbl = CtrlVtbl { destroy: block_destroy, delete_this: block_delete_this, deleting_dtor: block_deleting_dtor, get_deleter: no_deleter };

unsafe extern "C" fn d_dtor(this: *mut Dispatcher, _flags: u32) -> *mut Dispatcher {
    core::ptr::drop_in_place(this);
    this
}

unsafe extern "C" fn d_queue_json_event(this: *mut Dispatcher, name: *const u16, data: Obj) -> HRESULT {
    ((*(*this).vtbl).queue_json_with_binary_event)(this, name, data, 0, core::ptr::null())
}

unsafe extern "C" fn d_queue_binary_event(this: *mut Dispatcher, name: *const u16, n: u32, data: *const u8) -> HRESULT {
    let i = match imports::get() {
        Ok(i) => i,
        Err(e) => return e,
    };
    let mut body = Ptr::null();
    let hr = chk((i.create_json_object)(body.out()), b"XbtpCreateJsonObject(&messageBody)\0", b"XtfEventServer::EventMessageDispatcher::QueueBinaryEvent\0", 177);
    if hr < 0 {
        return hr;
    }
    let hr = ((*(*this).vtbl).queue_json_with_binary_event)(this, name, body.0, n, data);
    chk(hr, b"QueueJsonWithBinaryEvent(eventType, messageBody, numBinaryDataBytes, binaryData)\0", b"XtfEventServer::EventMessageDispatcher::QueueBinaryEvent\0", 178)
}

unsafe extern "C" fn d_queue_json_with_binary_event(this: *mut Dispatcher, name: *const u16, data: Obj, n: u32, bytes: *const u8) -> HRESULT {
    let d = &*this;
    if !d.initialized.load(Ordering::Acquire) {
        return chk(E_UNEXPECTED, b"E_UNEXPECTED\0", F_QUEUE, 117);
    }
    if name.is_null() || *name == 0 {
        return chk(E_INVALIDARG, b"E_INVALIDARG\0", F_QUEUE, 122);
    }
    let mut ev = Event { name: core::slice::from_raw_parts(name, wlen(name) + 1).to_vec(), binary: Vec::new(), data: Ptr::null() };
    if !data.is_null() {
        add_ref(data);
        ev.data = Ptr(data);
    }
    if n != 0 {
        if bytes.is_null() {
            return chk(E_INVALIDARG, b"E_INVALIDARG\0", F_QUEUE, 139);
        }
        ev.binary = core::slice::from_raw_parts(bytes, n as usize).to_vec();
    }
    d.queue.lock().unwrap_or_else(|e| e.into_inner()).push_back(ev);
    if SetEvent(d.event) == 0 {
        let e = GetLastError();
        return if e == 0 { E_FAIL } else { hr_from_win32(e) };
    }
    S_OK
}

pub static DISPATCHER_VTBL: DispatcherVtbl = DispatcherVtbl {
    dtor: d_dtor,
    queue_json_event: d_queue_json_event,
    queue_binary_event: d_queue_binary_event,
    queue_json_with_binary_event: d_queue_json_with_binary_event,
};

pub unsafe fn make_dispatcher() -> SharedPtr {
    let b = Box::into_raw(Box::new(DispatcherBlock {
        ctrl: Ctrl::new(&BLOCK_VTBL),
        disp: ManuallyDrop::new(Dispatcher { vtbl: &DISPATCHER_VTBL, initialized: AtomicBool::new(false), session: null_mut(), event: null_mut(), queue: Mutex::new(VecDeque::new()) }),
    }));
    SharedPtr { ptr: &mut *(*b).disp as *mut Dispatcher as *mut c_void, ctrl: b as *mut Ctrl }
}

pub unsafe fn initialize(d: *mut Dispatcher, session: Obj) -> HRESULT {
    let f = b"XtfEventServer::EventMessageDispatcher::Initialize\0";
    if session.is_null() {
        return chk(E_INVALIDARG, b"E_INVALIDARG\0", f, 13);
    }
    let old = core::mem::replace(&mut (*d).session, session);
    if !old.is_null() {
        release(old);
    }
    add_ref(session);
    let e = CreateEventW(null_mut(), 0, 0, null_mut());
    (*d).event = e;
    if e.is_null() {
        return chk(hr_last_error(), b"HRESULT_FROM_WIN32(GetLastError())\0", f, 22);
    }
    (*d).initialized.store(true, Ordering::Release);
    S_OK
}

pub unsafe fn serialize_json_body(body: Obj, message: Obj) -> HRESULT {
    if body.is_null() || message.is_null() {
        return E_INVALIDARG;
    }
    let mut w = Bstr::null();
    let hr = obj_stringify(body, w.out());
    if hr < 0 {
        return hr;
    }
    msg_set_json_body_w(message, w.0)
}

pub unsafe fn send_more_json_message(session: Obj, body: Obj) -> HRESULT {
    if session.is_null() || body.is_null() {
        return E_INVALIDARG;
    }
    let i = match imports::get() {
        Ok(i) => i,
        Err(e) => return e,
    };
    let mut m = Ptr::null();
    let hr = (i.create_message)(&IID_XBTP_MESSAGE_IMPL, m.out());
    if hr < 0 {
        return hr;
    }
    let hr = serialize_json_body(body, m.0);
    if hr < 0 {
        return hr;
    }
    session_send_more(session, m.0)
}

pub unsafe fn handle_events(d: *mut Dispatcher) -> HRESULT {
    let d = &*d;
    if !d.initialized.load(Ordering::Acquire) {
        return chk(E_UNEXPECTED, b"E_UNEXPECTED\0", F_HANDLE, 45);
    }
    let i = match imports::get() {
        Ok(i) => i,
        Err(e) => return e,
    };
    let mut body = Ptr::null();
    let hr = chk((i.create_json_object)(body.out()), b"XbtpCreateJsonObject(&messageBody)\0", F_HANDLE, 48);
    if hr < 0 {
        return hr;
    }
    let mut message = Ptr::null();
    let hr = chk((i.create_message)(&IID_XBTP_MESSAGE_IMPL, message.out()), b"XbtpCreateMessage(IID_PPV_ARGS(&spMessage))\0", F_HANDLE, 49);
    if hr < 0 {
        return hr;
    }
    let mut keep = Ptr::null();
    let hr = chk((i.create_json_object)(keep.out()), b"XbtpCreateJsonObject(&keepAliveMessageBody)\0", F_HANDLE, 50);
    if hr < 0 {
        return hr;
    }
    let mut keep_obj = Ptr::null();
    let hr = chk((i.create_json_object)(keep_obj.out()), b"XbtpCreateJsonObject(&keepAliveMessageObject)\0", F_HANDLE, 51);
    if hr < 0 {
        return hr;
    }
    let hr = chk(obj_set_string(keep.0, wz(EVENT_TYPE).as_ptr(), wz(KEEP_ALIVE_EVENT).as_ptr()), b"keepAliveMessageBody->SetString\0", F_HANDLE, 52);
    if hr < 0 {
        return hr;
    }
    let hr = chk(obj_set_object(keep.0, wz(EVENT_DATA).as_ptr(), keep_obj.0), b"keepAliveMessageBody->SetObject\0", F_HANDLE, 53);
    if hr < 0 {
        return hr;
    }
    loop {
        match WaitForSingleObject(d.event, KEEP_ALIVE_MS) {
            WAIT_FAILED => return chk(hr_last_error(), b"HRESULT_FROM_WIN32(GetLastError())\0", F_HANDLE, 62),
            WAIT_OBJECT_0 => {
                let mut pending = core::mem::take(&mut *d.queue.lock().unwrap_or_else(|e| e.into_inner()));
                while let Some(ev) = pending.front() {
                    let hr = chk(obj_set_string(body.0, wz(EVENT_TYPE).as_ptr(), ev.name.as_ptr()), b"messageBody->SetString(Event_EventTypeProperty, e->eventName)\0", F_HANDLE, 79);
                    if hr < 0 {
                        return hr;
                    }
                    let hr = chk(obj_set_object(body.0, wz(EVENT_DATA).as_ptr(), ev.data.0), b"messageBody->SetObject(Event_EventDataProperty, e->eventData)\0", F_HANDLE, 80);
                    if hr < 0 {
                        return hr;
                    }
                    let hr = chk(serialize_json_body(body.0, message.0), b"XtfSerializeJsonBody(messageBody, spMessage)\0", F_HANDLE, 81);
                    if hr < 0 {
                        return hr;
                    }
                    let p = if ev.binary.is_empty() { core::ptr::null() } else { ev.binary.as_ptr() };
                    let hr = chk(msg_set_binary_body(message.0, ev.binary.len() as u32, p), b"spMessage->SetBinaryBody(e->numBinaryDataBytes, binaryData)\0", F_HANDLE, 84);
                    if hr < 0 {
                        return hr;
                    }
                    let hr = chk(session_send_more(d.session, message.0), b"m_session->SendMoreMessage(spMessage)\0", F_HANDLE, 86);
                    if hr < 0 {
                        return hr;
                    }
                    pending.pop_front();
                }
            }
            WAIT_TIMEOUT => {
                let hr = chk(send_more_json_message(d.session, keep.0), b"XtfSendMoreJsonMessage(m_session, keepAliveMessageBody)\0", F_HANDLE, 97);
                if hr < 0 {
                    return hr;
                }
            }
            _ => return chk(E_UNEXPECTED, b"E_UNEXPECTED\0", F_HANDLE, 101),
        }
    }
}
