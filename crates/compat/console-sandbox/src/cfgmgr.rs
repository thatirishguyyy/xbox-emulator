use core::ffi::c_void;
use std::collections::VecDeque;
use std::sync::{Mutex, MutexGuard, OnceLock};

pub const CR_SUCCESS: u32 = 0;
pub const CR_INVALID_POINTER: u32 = 3;
pub const CR_INVALID_FLAG: u32 = 4;
pub const CR_BUFFER_SMALL: u32 = 0x1a;
pub const CR_INVALID_DATA: u32 = 0x1f;

pub const FILTER_DEVICE_INTERFACE: u32 = 0;
pub const FILTER_DEVICE_HANDLE: u32 = 1;
pub const FILTER_DEVICE_INSTANCE: u32 = 2;
pub const FILTER_FLAG_ALL_INTERFACE_CLASSES: u32 = 1;
pub const FILTER_FLAG_ALL_DEVICE_INSTANCES: u32 = 2;
pub const FILTER_SIZE: u32 = 0x1a0;

pub const ACTION_INTERFACE_ARRIVAL: u32 = 0;
pub const ACTION_INTERFACE_REMOVAL: u32 = 1;
pub const ACTION_QUERY_REMOVE: u32 = 2;
pub const ACTION_QUERY_REMOVE_FAILED: u32 = 3;
pub const ACTION_REMOVE_PENDING: u32 = 4;
pub const ACTION_REMOVE_COMPLETE: u32 = 5;
pub const ACTION_INSTANCE_ENUMERATED: u32 = 7;
pub const ACTION_INSTANCE_STARTED: u32 = 8;
pub const ACTION_INSTANCE_REMOVED: u32 = 9;

type Callback = unsafe extern "system" fn(usize, usize, u32, *const u8, u32) -> u32;

#[link(name = "kernel32")]
extern "system" {
    fn CreateEventW(sa: *const c_void, manual: i32, initial: i32, name: *const u16) -> usize;
    fn SetEvent(h: usize) -> i32;
    fn WaitForSingleObjectEx(h: usize, ms: u32, alertable: i32) -> u32;
}

#[derive(Clone)]
pub struct Interface {
    pub class: [u8; 16],
    pub link: String,
    pub instance: String,
}

#[derive(Clone)]
struct Registration {
    id: usize,
    kind: u32,
    flags: u32,
    class: [u8; 16],
    target: usize,
    instance: String,
    context: usize,
    callback: usize,
}

struct Event {
    reg: Registration,
    action: u32,
    data: Vec<u8>,
}

#[derive(Default)]
struct Pnp {
    next: usize,
    interfaces: Vec<Interface>,
    regs: Vec<Registration>,
    queue: VecDeque<Event>,
    event: usize,
}

fn pnp() -> MutexGuard<'static, Pnp> {
    static P: OnceLock<Mutex<Pnp>> = OnceLock::new();
    P.get_or_init(|| Mutex::new(Pnp::default())).lock().unwrap_or_else(|e| e.into_inner())
}

fn dispatcher() -> usize {
    static D: OnceLock<usize> = OnceLock::new();
    *D.get_or_init(|| {
        let ev = unsafe { CreateEventW(core::ptr::null(), 0, 0, core::ptr::null()) };
        pnp().event = ev;
        std::thread::Builder::new().name("cm-notify".into()).spawn(run).ok();
        ev
    })
}

fn run() {
    loop {
        let next = pnp().queue.pop_front();
        match next {
            Some(e) => {
                let live = pnp().regs.iter().any(|r| r.id == e.reg.id);
                if live {
                    let cb: Callback = unsafe { core::mem::transmute(e.reg.callback) };
                    unsafe { cb(e.reg.id, e.reg.context, e.action, e.data.as_ptr(), e.data.len() as u32) };
                }
            }
            None => {
                let ev = pnp().event;
                unsafe { WaitForSingleObjectEx(ev, u32::MAX, 1) };
            }
        }
    }
}

fn post(p: &mut Pnp, reg: Registration, action: u32, data: Vec<u8>) {
    p.queue.push_back(Event { reg, action, data });
    if p.event != 0 {
        unsafe { SetEvent(p.event) };
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn interface_data(class: [u8; 16], link: &str) -> Vec<u8> {
    let mut d = Vec::new();
    d.extend_from_slice(&FILTER_DEVICE_INTERFACE.to_le_bytes());
    d.extend_from_slice(&0u32.to_le_bytes());
    d.extend_from_slice(&class);
    for c in wide(link) {
        d.extend_from_slice(&c.to_le_bytes());
    }
    d
}

fn handle_data() -> Vec<u8> {
    let mut d = vec![0u8; 0x28];
    d[0..4].copy_from_slice(&FILTER_DEVICE_HANDLE.to_le_bytes());
    d[0x18..0x1c].copy_from_slice(&(-1i32).to_le_bytes());
    d
}

pub fn add_interface(class: [u8; 16], link: &str, instance: &str) {
    dispatcher();
    let mut p = pnp();
    p.interfaces.push(Interface { class, link: link.to_string(), instance: instance.to_string() });
    let regs: Vec<Registration> = p.regs.iter().filter(|r| r.kind == FILTER_DEVICE_INTERFACE && (r.flags & FILTER_FLAG_ALL_INTERFACE_CLASSES != 0 || r.class == class)).cloned().collect();
    for r in regs {
        post(&mut p, r, ACTION_INTERFACE_ARRIVAL, interface_data(class, link));
    }
}

pub fn remove_interface(link: &str) {
    dispatcher();
    let mut p = pnp();
    let Some(i) = p.interfaces.iter().position(|x| x.link.eq_ignore_ascii_case(link)) else { return };
    let iface = p.interfaces.remove(i);
    let regs: Vec<Registration> = p.regs.iter().filter(|r| r.kind == FILTER_DEVICE_INTERFACE && (r.flags & FILTER_FLAG_ALL_INTERFACE_CLASSES != 0 || r.class == iface.class)).cloned().collect();
    for r in regs {
        post(&mut p, r, ACTION_INTERFACE_REMOVAL, interface_data(iface.class, &iface.link));
    }
}

pub fn notify_handles(is_target: impl Fn(usize) -> bool, action: u32) -> usize {
    dispatcher();
    let mut p = pnp();
    let regs: Vec<Registration> = p.regs.iter().filter(|r| r.kind == FILTER_DEVICE_HANDLE && is_target(r.target)).cloned().collect();
    let n = regs.len();
    for r in regs {
        post(&mut p, r, action, handle_data());
    }
    n
}

pub fn interfaces(class: [u8; 16]) -> Vec<String> {
    pnp().interfaces.iter().filter(|i| i.class == class).map(|i| i.link.clone()).collect()
}

pub fn interface_registrations(class: [u8; 16]) -> usize {
    pnp().regs.iter().filter(|r| r.kind == FILTER_DEVICE_INTERFACE && (r.flags & FILTER_FLAG_ALL_INTERFACE_CLASSES != 0 || r.class == class)).count()
}

pub fn handle_registrations(is_target: impl Fn(usize) -> bool) -> usize {
    pnp().regs.iter().filter(|r| r.kind == FILTER_DEVICE_HANDLE && is_target(r.target)).count()
}

unsafe fn guid(p: *const u8) -> [u8; 16] {
    let mut g = [0u8; 16];
    core::ptr::copy_nonoverlapping(p, g.as_mut_ptr(), 16);
    g
}

fn list(class: [u8; 16], instance: Option<&str>) -> Vec<u16> {
    let p = pnp();
    let mut out = Vec::new();
    for i in p.interfaces.iter().filter(|i| i.class == class && instance.is_none_or(|d| d.eq_ignore_ascii_case(&i.instance))) {
        out.extend(i.link.encode_utf16());
        out.push(0);
    }
    out.push(0);
    out
}

pub unsafe extern "system" fn cm_get_device_interface_list_size_w(len: *mut u32, class: *const u8, device: *const u16, flags: u32) -> u32 {
    if len.is_null() || class.is_null() {
        return CR_INVALID_POINTER;
    }
    if flags > 1 {
        return CR_INVALID_FLAG;
    }
    let dev = (!device.is_null()).then(|| crate::wstr::from_ptr(device));
    *len = list(guid(class), dev.as_deref()).len() as u32;
    CR_SUCCESS
}

pub unsafe extern "system" fn cm_get_device_interface_list_w(class: *const u8, device: *const u16, buf: *mut u16, buf_len: u32, flags: u32) -> u32 {
    if class.is_null() || buf.is_null() || buf_len == 0 {
        return CR_INVALID_POINTER;
    }
    if flags > 1 {
        return CR_INVALID_FLAG;
    }
    let dev = (!device.is_null()).then(|| crate::wstr::from_ptr(device));
    let l = list(guid(class), dev.as_deref());
    if l.len() > buf_len as usize {
        *buf = 0;
        return CR_BUFFER_SMALL;
    }
    core::ptr::copy_nonoverlapping(l.as_ptr(), buf, l.len());
    CR_SUCCESS
}

pub unsafe extern "system" fn cm_register_notification(filter: *const u8, context: usize, callback: usize, out: *mut usize) -> u32 {
    if filter.is_null() || out.is_null() || callback == 0 {
        return CR_INVALID_POINTER;
    }
    let size = (filter as *const u32).read_unaligned();
    let flags = (filter.add(4) as *const u32).read_unaligned();
    let kind = (filter.add(8) as *const u32).read_unaligned();
    if size != FILTER_SIZE || kind > FILTER_DEVICE_INSTANCE {
        return CR_INVALID_DATA;
    }
    let (class, target, instance) = match kind {
        FILTER_DEVICE_INTERFACE => (guid(filter.add(0x10)), 0, String::new()),
        FILTER_DEVICE_HANDLE => {
            let t = (filter.add(0x10) as *const usize).read_unaligned();
            if t == 0 || t == usize::MAX {
                return CR_INVALID_DATA;
            }
            ([0u8; 16], t, String::new())
        }
        _ => ([0u8; 16], 0, crate::wstr::from_ptr(filter.add(0x10) as *const u16)),
    };
    dispatcher();
    let mut p = pnp();
    p.next += 1;
    let id = 0x7C3E_0000 + p.next * 8;
    p.regs.push(Registration { id, kind, flags, class, target, instance, context, callback });
    *out = id;
    CR_SUCCESS
}

pub unsafe extern "system" fn cm_unregister_notification(h: usize) -> u32 {
    let mut p = pnp();
    let Some(i) = p.regs.iter().position(|r| r.id == h) else { return CR_INVALID_DATA };
    p.regs.remove(i);
    p.queue.retain(|e| e.reg.id != h);
    CR_SUCCESS
}
