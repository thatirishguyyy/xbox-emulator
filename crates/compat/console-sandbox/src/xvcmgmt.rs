use core::ffi::c_void;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Mutex, MutexGuard, OnceLock};

pub const DEVICE: &str = "\\Device\\xvcmgmt";
pub const HOST_VM: [u8; 16] = [0x6b, 0x41, 0x2a, 0xc1, 0x47, 0xf3, 0x5c, 0x40, 0xb7, 0x65, 0x2d, 0xe0, 0x27, 0xa6, 0x24, 0x9e];
pub const LOOPBACK_VM: [u8; 16] = [0x97, 0x61, 0xe1, 0xe0, 0x56, 0xdd, 0x10, 0x4a, 0x91, 0x95, 0x5e, 0xe7, 0xa1, 0x55, 0xa8, 0x38];

pub const IOCTL_CREATE: u32 = 0x150098;
pub const IOCTL_LISTEN: u32 = 0x15009c;
pub const IOCTL_CONNECT: u32 = 0x1500a0;
pub const IOCTL_ACCEPT: u32 = 0x1500a4;
pub const IOCTL_SEND: u32 = 0x1500a8;
pub const IOCTL_RECEIVE: u32 = 0x1500ac;
pub const IOCTL_CLOSE: u32 = 0x1500b0;
pub const IOCTL_SHUTDOWN: u32 = 0x1500f4;

pub const STATUS_SUCCESS: i32 = 0;
pub const STATUS_PENDING: i32 = 0x103;
pub const STATUS_OBJECT_NAME_NOT_FOUND: i32 = 0xC000_0034u32 as i32;
pub const STATUS_INVALID_HANDLE: i32 = 0xC000_0008u32 as i32;
pub const STATUS_INVALID_PARAMETER: i32 = 0xC000_000Du32 as i32;
pub const STATUS_INVALID_DEVICE_REQUEST: i32 = 0xC000_0010u32 as i32;
pub const STATUS_BUFFER_TOO_SMALL: i32 = 0xC000_0023u32 as i32;
pub const STATUS_INSUFFICIENT_RESOURCES: i32 = 0xC000_009Au32 as i32;
pub const STATUS_CANCELLED: i32 = 0xC000_0120u32 as i32;
pub const STATUS_ADDRESS_ALREADY_ASSOCIATED: i32 = 0xC000_0184u32 as i32;
pub const STATUS_NOT_FOUND: i32 = 0xC000_0225u32 as i32;
pub const STATUS_CONNECTION_INVALID: i32 = 0xC000_0140u32 as i32;
pub const STATUS_CONNECTION_RESET: i32 = 0xC000_020Du32 as i32;
pub const STATUS_CONNECTION_REFUSED: i32 = 0xC000_0236u32 as i32;
pub const STATUS_LOCAL_DISCONNECT: i32 = 0xC000_0241u32 as i32;
pub const STATUS_NOT_IMPLEMENTED_PEER: i32 = 0xC000_0002u32 as i32;
pub const STATUS_NOT_CONNECTED: i32 = 0xC000_02CCu32 as i32;
const FILE_OPENED: usize = 1;

#[link(name = "ntdll")]
extern "system" {
    fn NtOpenFile(h: *mut usize, access: u32, oa: *const u8, io: *mut usize, share: u32, opts: u32) -> i32;
    fn NtCreateFile(h: *mut usize, access: u32, oa: *const u8, io: *mut usize, alloc: *const i64, attrs: u32, share: u32, disp: u32, opts: u32, ea: *const c_void, ea_len: u32) -> i32;
    fn NtDeviceIoControlFile(h: usize, ev: usize, apc: usize, ctx: usize, io: *mut usize, code: u32, inb: *const u8, inl: u32, outb: *mut u8, outl: u32) -> i32;
    fn NtQueueApcThread(thread: usize, f: usize, a1: usize, a2: usize, a3: usize) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn CreateFileW(name: *const u16, access: u32, share: u32, sa: *const c_void, disp: u32, flags: u32, template: isize) -> isize;
    fn SetEvent(h: usize) -> i32;
    fn ResetEvent(h: usize) -> i32;
    fn GetCurrentThread() -> usize;
    fn GetCurrentThreadId() -> u32;
    fn GetCurrentProcess() -> usize;
    fn DuplicateHandle(sp: usize, sh: usize, tp: usize, th: *mut usize, access: u32, inherit: i32, opts: u32) -> i32;
    fn CloseHandle(h: usize) -> i32;
}

struct Irp {
    io: usize,
    event: usize,
    apc: usize,
    ctx: usize,
    thread: usize,
    out: usize,
    out_len: usize,
    file: usize,
    tid: u32,
}

thread_local! {
    static CURRENT_FILE: core::cell::Cell<usize> = const { core::cell::Cell::new(0) };
}

impl Irp {
    unsafe fn new(io: *mut usize, event: usize, apc: usize, ctx: usize, out: *mut u8, out_len: u32) -> Irp {
        let mut thread = 0;
        if apc != 0 {
            DuplicateHandle(GetCurrentProcess(), GetCurrentThread(), GetCurrentProcess(), &mut thread, 0, 0, 2);
        }
        Irp { io: io as usize, event, apc, ctx, thread, out: out as usize, out_len: out_len as usize, file: CURRENT_FILE.with(|c| c.get()), tid: GetCurrentThreadId() }
    }

    unsafe fn complete(self, status: i32, info: usize) {
        if self.io != 0 {
            let io = self.io as *mut usize;
            io.write_unaligned(status as u32 as usize);
            io.add(1).write_unaligned(info);
        }
        if self.event != 0 {
            SetEvent(self.event);
        }
        if self.apc != 0 && self.thread != 0 {
            NtQueueApcThread(self.thread, self.apc, self.ctx, self.io, 0);
        }
        if self.thread != 0 {
            CloseHandle(self.thread);
        }
    }
}

#[derive(Default)]
struct Endpoint {
    vm: [u8; 16],
    service: [u8; 16],
    host: bool,
    listening: bool,
    connected: bool,
    peer: Option<u64>,
    rx: VecDeque<u8>,
    eof: Option<i32>,
    accepted: VecDeque<u64>,
    accepts: Vec<Irp>,
    receives: VecDeque<Irp>,
}

#[derive(Default)]
struct Fabric {
    next: u64,
    endpoints: HashMap<u64, Endpoint>,
    listeners: HashMap<([u8; 16], [u8; 16]), u64>,
    handles: HashSet<usize>,
}

fn fabric() -> MutexGuard<'static, Fabric> {
    static F: OnceLock<Mutex<Fabric>> = OnceLock::new();
    let f = F.get_or_init(|| Mutex::new(Fabric { next: 1, ..Default::default() }));
    let mut g = f.lock().unwrap_or_else(|e| e.into_inner());
    if g.listeners.is_empty() && g.next == 1 {
        host_listen_locked(&mut g, HOST_VM, crate::claims::launch_instance());
    }
    g
}

fn alloc(f: &mut Fabric) -> u64 {
    f.next += 1;
    f.next
}

fn host_listen_locked(f: &mut Fabric, vm: [u8; 16], service: [u8; 16]) -> u64 {
    let id = alloc(f);
    f.endpoints.insert(id, Endpoint { vm, service, host: true, listening: true, ..Default::default() });
    f.listeners.insert((vm, service), id);
    id
}

pub fn host_listen(vm: [u8; 16], service: [u8; 16]) -> u64 {
    let mut f = fabric();
    if let Some(&id) = f.listeners.get(&(vm, service)) {
        return id;
    }
    host_listen_locked(&mut f, vm, service)
}

pub fn host_connections(service: [u8; 16]) -> Vec<u64> {
    let f = fabric();
    let mut v: Vec<u64> = f.endpoints.iter().filter(|(_, e)| e.host && !e.listening && e.service == service).map(|(&id, _)| id).collect();
    v.sort();
    v
}

pub fn host_take_received(id: u64) -> Vec<u8> {
    let mut f = fabric();
    f.endpoints.get_mut(&id).map(|e| e.rx.drain(..).collect()).unwrap_or_default()
}

pub fn host_send(id: u64, data: &[u8]) -> i32 {
    let mut f = fabric();
    deliver(&mut f, id, data)
}

pub fn host_close(id: u64) {
    let mut f = fabric();
    close(&mut f, id, false);
}

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleW(name: *const u16) -> usize;
    fn GetProcAddress(m: usize, name: *const u8) -> usize;
}

type CompareFn = unsafe extern "system" fn(usize, usize) -> i32;

fn compare_object_handles() -> Option<CompareFn> {
    static F: OnceLock<usize> = OnceLock::new();
    let p = *F.get_or_init(|| unsafe {
        let kb: Vec<u16> = "kernelbase.dll".encode_utf16().chain(Some(0)).collect();
        GetProcAddress(GetModuleHandleW(kb.as_ptr()), b"CompareObjectHandles\0".as_ptr())
    });
    (p != 0).then(|| unsafe { core::mem::transmute::<usize, CompareFn>(p) })
}

pub fn device_of(h: usize) -> Option<usize> {
    if h == 0 || h == usize::MAX {
        return None;
    }
    let privs: Vec<usize> = fabric().handles.iter().copied().collect();
    let cmp = compare_object_handles()?;
    privs.into_iter().find(|&p| unsafe { cmp(h, p) } != 0)
}

pub fn is_device(h: usize) -> bool {
    device_of(h).is_some()
}

fn cancel(file: usize, io: Option<usize>, tid: Option<u32>) -> usize {
    let mut f = fabric();
    let hit = |i: &Irp| i.file == file && io.is_none_or(|x| x == i.io) && tid.is_none_or(|t| t == i.tid);
    let mut done = Vec::new();
    for e in f.endpoints.values_mut() {
        let (c, keep): (Vec<Irp>, Vec<Irp>) = e.accepts.drain(..).partition(|i| hit(i));
        e.accepts = keep;
        done.extend(c);
        let (c, keep): (VecDeque<Irp>, VecDeque<Irp>) = e.receives.drain(..).partition(|i| hit(i));
        e.receives = keep;
        done.extend(c);
    }
    drop(f);
    let n = done.len();
    for irp in done {
        unsafe { irp.complete(STATUS_CANCELLED, 0) };
    }
    n
}

unsafe fn put_iosb(io: *mut usize, st: i32) {
    if !io.is_null() {
        io.write_unaligned(st as u32 as usize);
        io.add(1).write_unaligned(0);
    }
}

#[link(name = "ntdll")]
extern "system" {
    fn NtCancelIoFile(h: usize, io: *mut usize) -> i32;
    fn NtCancelIoFileEx(h: usize, req: *mut usize, io: *mut usize) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn CancelIo(h: usize) -> i32;
    fn CancelIoEx(h: usize, ov: *mut c_void) -> i32;
}

pub unsafe extern "system" fn nt_cancel_io_file(h: usize, io: *mut usize) -> i32 {
    let Some(file) = device_of(h) else { return NtCancelIoFile(h, io) };
    cancel(file, None, Some(GetCurrentThreadId()));
    put_iosb(io, STATUS_SUCCESS);
    STATUS_SUCCESS
}

pub unsafe extern "system" fn nt_cancel_io_file_ex(h: usize, req: *mut usize, io: *mut usize) -> i32 {
    let Some(file) = device_of(h) else { return NtCancelIoFileEx(h, req, io) };
    if cancel(file, (!req.is_null()).then_some(req as usize), None) == 0 {
        return STATUS_NOT_FOUND;
    }
    put_iosb(io, STATUS_SUCCESS);
    STATUS_SUCCESS
}

pub unsafe extern "system" fn cancel_io(h: usize) -> i32 {
    let Some(file) = device_of(h) else { return CancelIo(h) };
    cancel(file, None, Some(GetCurrentThreadId()));
    1
}

pub unsafe extern "system" fn cancel_io_ex(h: usize, ov: *mut c_void) -> i32 {
    let Some(file) = device_of(h) else { return CancelIoEx(h, ov) };
    if cancel(file, (!ov.is_null()).then_some(ov as usize), None) == 0 {
        SetLastError(1168);
        return 0;
    }
    1
}

#[link(name = "ntdll")]
extern "system" {
    fn NtQueryObject(h: usize, class: u32, buf: *mut u8, len: u32, ret: *mut u32) -> i32;
}

fn handle_count(h: usize) -> u32 {
    let mut b = [0u8; 56];
    if unsafe { NtQueryObject(h, 0, b.as_mut_ptr(), 56, core::ptr::null_mut()) } < 0 {
        return 0;
    }
    u32::from_le_bytes(b[8..12].try_into().unwrap())
}

pub fn before_close(h: usize) {
    if h == 0 || h == usize::MAX {
        return;
    }
    let Some(cmp) = compare_object_handles() else { return };
    let mut f = fabric();
    let Some(p) = f.handles.iter().copied().find(|&p| p != h && unsafe { cmp(h, p) } != 0) else { return };
    if handle_count(p) <= 2 {
        f.handles.remove(&p);
        unsafe { CloseHandle(p) };
    }
}

pub fn device_handles() -> usize {
    fabric().handles.len()
}

fn deliver(f: &mut Fabric, from: u64, data: &[u8]) -> i32 {
    let Some(peer) = f.endpoints.get(&from).and_then(|e| e.connected.then_some(e.peer).flatten()) else { return STATUS_CONNECTION_INVALID };
    let Some(p) = f.endpoints.get_mut(&peer) else { return STATUS_SUCCESS };
    let gamecore = p.host && p.vm == HOST_VM && p.service == crate::claims::launch_instance();
    if p.host {
        compat_common::record_raw("xvcmgmt", "host", &format!("queue {from} sent {} bytes to emulated host endpoint {peer}", data.len()));
    }
    p.rx.extend(data.iter().copied());
    pump(p);
    if gamecore {
        let reply = crate::gchost::on_bytes(peer, data);
        if !reply.is_empty() {
            if let Some(g) = f.endpoints.get_mut(&from) {
                g.rx.extend(reply);
                pump(g);
            }
        }
    }
    STATUS_SUCCESS
}

fn pump(e: &mut Endpoint) {
    while !e.receives.is_empty() {
        if e.rx.is_empty() && e.eof.is_none() {
            break;
        }
        let irp = e.receives.pop_front().unwrap();
        if e.rx.is_empty() {
            let st = e.eof.unwrap();
            unsafe { irp.complete(st, 0) };
            continue;
        }
        let n = irp.out_len.min(e.rx.len());
        let bytes: Vec<u8> = e.rx.drain(..n).collect();
        unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), irp.out as *mut u8, n);
            irp.complete(STATUS_SUCCESS, n);
        }
    }
}

fn close(f: &mut Fabric, id: u64, graceful: bool) {
    let Some(mut e) = f.endpoints.remove(&id) else { return };
    if e.peer.is_some() {
        crate::gchost::forget(id);
        if let Some(p) = e.peer {
            crate::gchost::forget(p);
        }
    }
    if e.listening {
        f.listeners.retain(|_, v| *v != id);
    }
    for irp in e.accepts.drain(..) {
        unsafe { irp.complete(STATUS_LOCAL_DISCONNECT, 0) };
    }
    for irp in e.receives.drain(..) {
        unsafe { irp.complete(STATUS_CANCELLED, 0) };
    }
    let queued: Vec<u64> = e.accepted.drain(..).collect();
    for q in queued {
        close(f, q, false);
    }
    if let Some(p) = e.peer.filter(|_| e.connected) {
        if let Some(pe) = f.endpoints.get_mut(&p) {
            pe.eof = Some(if graceful { STATUS_SUCCESS } else { STATUS_CONNECTION_RESET });
            if !graceful {
                pe.connected = false;
            }
            pump(pe);
        }
    }
}

enum Work {
    Listen(u64, Irp),
    Connect(u64, Irp),
}

fn worker() -> &'static Mutex<Sender<Work>> {
    static W: OnceLock<Mutex<Sender<Work>>> = OnceLock::new();
    W.get_or_init(|| {
        let (tx, rx) = channel::<Work>();
        std::thread::Builder::new()
            .name("xvcmgmt-worker".into())
            .spawn(move || {
                for w in rx {
                    run(w);
                }
            })
            .ok();
        Mutex::new(tx)
    })
}

fn run(w: Work) {
    let mut f = fabric();
    match w {
        Work::Listen(id, irp) => {
            let st = match f.endpoints.get_mut(&id) {
                None => STATUS_INVALID_HANDLE,
                Some(e) if e.listening || e.connected => STATUS_ADDRESS_ALREADY_ASSOCIATED,
                Some(e) => {
                    let key = (e.vm, e.service);
                    if f.listeners.contains_key(&key) {
                        STATUS_ADDRESS_ALREADY_ASSOCIATED
                    } else {
                        f.endpoints.get_mut(&id).unwrap().listening = true;
                        f.listeners.insert(key, id);
                        STATUS_SUCCESS
                    }
                }
            };
            unsafe { irp.complete(st, 0) };
        }
        Work::Connect(id, irp) => {
            let st = connect(&mut f, id);
            unsafe { irp.complete(st, 0) };
        }
    }
}

fn connect(f: &mut Fabric, id: u64) -> i32 {
    let (vm, service) = match f.endpoints.get(&id) {
        None => return STATUS_INVALID_HANDLE,
        Some(e) if e.listening || e.connected || e.peer.is_some() => return STATUS_ADDRESS_ALREADY_ASSOCIATED,
        Some(e) => (e.vm, e.service),
    };
    let Some(&lid) = f.listeners.get(&(vm, service)) else { return STATUS_CONNECTION_REFUSED };
    let host = f.endpoints.get(&lid).map(|l| l.host).unwrap_or(false);
    let aid = alloc(f);
    f.endpoints.insert(aid, Endpoint { vm, service, host, connected: true, peer: Some(id), ..Default::default() });
    if let Some(e) = f.endpoints.get_mut(&id) {
        e.connected = true;
        e.peer = Some(aid);
    }
    if let Some(l) = f.endpoints.get_mut(&lid) {
        if let Some(irp) = (!l.accepts.is_empty()).then(|| l.accepts.remove(0)) {
            unsafe {
                if irp.out != 0 {
                    (irp.out as *mut u64).write_unaligned(aid);
                }
                irp.complete(STATUS_SUCCESS, 8);
            }
        } else if !l.host {
            l.accepted.push_back(aid);
        }
    }
    STATUS_SUCCESS
}

unsafe fn id_of(inb: *const u8, inl: u32) -> Option<u64> {
    (inl >= 8 && !inb.is_null()).then(|| (inb as *const u64).read_unaligned())
}

unsafe fn finish(irp: Irp, st: i32, info: usize) -> i32 {
    if st >= 0 || (st as u32) >> 30 == 2 {
        irp.complete(st, info);
    } else if irp.thread != 0 {
        CloseHandle(irp.thread);
    }
    st
}

pub unsafe fn ioctl(io: *mut usize, event: usize, apc: usize, ctx: usize, code: u32, inb: *const u8, inl: u32, outb: *mut u8, outl: u32) -> i32 {
    if event != 0 {
        ResetEvent(event);
    }
    let irp = Irp::new(io, event, apc, ctx, outb, outl);
    match code {
        IOCTL_CREATE => {
            if inl < 0x20 || outl < 8 || inb.is_null() || outb.is_null() {
                return finish(irp, STATUS_INVALID_PARAMETER, 0);
            }
            let mut vm = [0u8; 16];
            let mut service = [0u8; 16];
            core::ptr::copy_nonoverlapping(inb, vm.as_mut_ptr(), 16);
            core::ptr::copy_nonoverlapping(inb.add(16), service.as_mut_ptr(), 16);
            let id = {
                let mut f = fabric();
                let id = alloc(&mut f);
                f.endpoints.insert(id, Endpoint { vm, service, ..Default::default() });
                id
            };
            (outb as *mut u64).write_unaligned(id);
            finish(irp, STATUS_SUCCESS, 8)
        }
        IOCTL_LISTEN | IOCTL_CONNECT | IOCTL_RECEIVE => {
            let Some(id) = id_of(inb, inl) else {
                return finish(irp, if code == IOCTL_RECEIVE { STATUS_BUFFER_TOO_SMALL } else { STATUS_INVALID_PARAMETER }, 0);
            };
            if code == IOCTL_RECEIVE && (outl == 0 || outb.is_null()) {
                return finish(irp, STATUS_BUFFER_TOO_SMALL, 0);
            }
            {
                let mut f = fabric();
                match f.endpoints.get_mut(&id) {
                    None => {
                        drop(f);
                        return finish(irp, STATUS_INVALID_HANDLE, 0);
                    }
                    Some(e) if code == IOCTL_RECEIVE => {
                        e.receives.push_back(irp);
                        pump(e);
                        return STATUS_PENDING;
                    }
                    Some(_) => {}
                }
            }
            let w = if code == IOCTL_LISTEN { Work::Listen(id, irp) } else { Work::Connect(id, irp) };
            if let Err(e) = worker().lock().unwrap_or_else(|e| e.into_inner()).send(w) {
                run(e.0);
            }
            STATUS_PENDING
        }
        IOCTL_ACCEPT => {
            if inl < 8 || outl < 8 || inb.is_null() || outb.is_null() {
                return finish(irp, STATUS_BUFFER_TOO_SMALL, 0);
            }
            let id = id_of(inb, inl).unwrap();
            let mut f = fabric();
            let Some(e) = f.endpoints.get_mut(&id) else {
                drop(f);
                return finish(irp, STATUS_INVALID_HANDLE, 0);
            };
            if let Some(a) = e.accepted.pop_front() {
                drop(f);
                (outb as *mut u64).write_unaligned(a);
                return finish(irp, STATUS_SUCCESS, 8);
            }
            e.accepts.push(irp);
            STATUS_PENDING
        }
        IOCTL_SEND => {
            if inl < 0x14 || (inl - 8) % 12 != 0 || inb.is_null() {
                return finish(irp, STATUS_INVALID_PARAMETER, 0);
            }
            let id = id_of(inb, inl).unwrap();
            let n = ((inl - 8) / 12) as usize;
            let mut data = Vec::new();
            for i in 0..n {
                let ent = inb.add(8 + i * 12);
                let p = (ent as *const u64).read_unaligned() as usize;
                let l = (ent.add(8) as *const u32).read_unaligned() as usize;
                if l == 0 {
                    return finish(irp, STATUS_INVALID_PARAMETER, 0);
                }
                data.extend_from_slice(std::slice::from_raw_parts(p as *const u8, l));
            }
            let st = {
                let mut f = fabric();
                if !f.endpoints.contains_key(&id) {
                    STATUS_INVALID_HANDLE
                } else {
                    deliver(&mut f, id, &data)
                }
            };
            finish(irp, st, 0)
        }
        IOCTL_CLOSE => {
            if let Some(id) = id_of(inb, inl) {
                let mut f = fabric();
                close(&mut f, id, false);
                drop(f);
                finish(irp, STATUS_SUCCESS, 0)
            } else {
                finish(irp, STATUS_INVALID_PARAMETER, 0)
            }
        }
        IOCTL_SHUTDOWN => {
            let Some(id) = id_of(inb, inl) else { return finish(irp, STATUS_INVALID_PARAMETER, 0) };
            let mut f = fabric();
            let st = match f.endpoints.get_mut(&id) {
                None => STATUS_INVALID_HANDLE,
                Some(e) if e.vm != LOOPBACK_VM => STATUS_NOT_IMPLEMENTED_PEER,
                Some(e) if !e.connected => STATUS_NOT_CONNECTED,
                Some(e) => {
                    e.connected = false;
                    if let Some(p) = e.peer.and_then(|p| f.endpoints.get_mut(&p)) {
                        p.eof = Some(STATUS_SUCCESS);
                        pump(p);
                    }
                    STATUS_SUCCESS
                }
            };
            drop(f);
            finish(irp, st, 0)
        }
        _ if crate::xvcsys::known(code) => {
            let (st, info) = crate::xvcsys::ioctl(code, inb, inl, outb, outl);
            compat_common::record_raw("xvcmgmt", "DeviceIoControl", &format!("ioctl {code:#x} (in {inl}, out {outl}) -> {st:#010x} info {info}"));
            finish(irp, st, info)
        }
        _ => {
            compat_common::record_raw("xvcmgmt", "DeviceIoControl", &format!("ioctl {code:#x} (in {inl}, out {outl}) is not emulated; STATUS_INVALID_DEVICE_REQUEST"));
            finish(irp, STATUS_INVALID_DEVICE_REQUEST, 0)
        }
    }
}

unsafe fn object_name(oa: *const u8) -> Option<String> {
    if oa.is_null() || (oa.add(8) as *const usize).read_unaligned() != 0 {
        return None;
    }
    let u = (oa.add(16) as *const usize).read_unaligned();
    if u == 0 {
        return None;
    }
    let len = (u as *const u16).read_unaligned() as usize / 2;
    let buf = ((u + 8) as *const usize).read_unaligned();
    (buf != 0).then(|| String::from_utf16_lossy(std::slice::from_raw_parts(buf as *const u16, len)))
}

fn is_device_name(n: &str) -> bool {
    let n = n.trim_end_matches('\\');
    n.eq_ignore_ascii_case(DEVICE)
}

unsafe fn open_device(h: *mut usize, io: *mut usize) -> i32 {
    let nul: Vec<u16> = "NUL".encode_utf16().chain(Some(0)).collect();
    let f = CreateFileW(nul.as_ptr(), 0xC000_0000, 3, core::ptr::null(), 3, 0, 0);
    if f == -1 {
        return STATUS_INSUFFICIENT_RESOURCES;
    }
    let mut private = 0usize;
    if DuplicateHandle(GetCurrentProcess(), f as usize, GetCurrentProcess(), &mut private, 0, 0, 2) == 0 {
        CloseHandle(f as usize);
        return STATUS_INSUFFICIENT_RESOURCES;
    }
    fabric().handles.insert(private);
    *h = f as usize;
    if !io.is_null() {
        io.write_unaligned(0);
        io.add(1).write_unaligned(FILE_OPENED);
    }
    STATUS_SUCCESS
}

pub unsafe extern "system" fn nt_open_file(h: *mut usize, access: u32, oa: *const u8, io: *mut usize, share: u32, opts: u32) -> i32 {
    if object_name(oa).is_some_and(|n| is_device_name(&n)) {
        return open_device(h, io);
    }
    NtOpenFile(h, access, oa, io, share, opts)
}

pub unsafe extern "system" fn nt_create_file(h: *mut usize, access: u32, oa: *const u8, io: *mut usize, alloc: *const i64, attrs: u32, share: u32, disp: u32, opts: u32, ea: *const c_void, ea_len: u32) -> i32 {
    if object_name(oa).is_some_and(|n| is_device_name(&n)) {
        return if disp == 1 || disp == 3 { open_device(h, io) } else { STATUS_OBJECT_NAME_NOT_FOUND };
    }
    NtCreateFile(h, access, oa, io, alloc, attrs, share, disp, opts, ea, ea_len)
}

pub unsafe extern "system" fn nt_device_io_control_file(h: usize, ev: usize, apc: usize, ctx: usize, io: *mut usize, code: u32, inb: *const u8, inl: u32, outb: *mut u8, outl: u32) -> i32 {
    if let Some(file) = device_of(h) {
        CURRENT_FILE.with(|c| c.set(file));
        return ioctl(io, ev, apc, ctx, code, inb, inl, outb, outl);
    }
    NtDeviceIoControlFile(h, ev, apc, ctx, io, code, inb, inl, outb, outl)
}

#[link(name = "kernel32")]
extern "system" {
    fn DeviceIoControl(h: usize, code: u32, inb: *const u8, inl: u32, outb: *mut u8, outl: u32, ret: *mut u32, ov: *mut c_void) -> i32;
    fn CreateEventW(sa: *const c_void, manual: i32, initial: i32, name: *const u16) -> usize;
    fn WaitForSingleObject(h: usize, ms: u32) -> u32;
    fn SetLastError(e: u32);
}

#[link(name = "ntdll")]
extern "system" {
    fn RtlNtStatusToDosError(st: i32) -> u32;
}

pub unsafe extern "system" fn close_handle(h: usize) -> i32 {
    crate::logan::forget(h);
    crate::vab::forget(h);
    before_close(h);
    CloseHandle(h)
}

pub unsafe extern "system" fn device_io_control(h: usize, code: u32, inb: *const u8, inl: u32, outb: *mut u8, outl: u32, ret: *mut u32, ov: *mut c_void) -> i32 {
    if crate::logan::is_device(h) {
        return crate::logan::ioctl(code, inb as *mut u8, inl, outb, outl, ret);
    }
    if crate::vab::is_device(h) {
        return crate::vab::ioctl(code, inb, inl, ret);
    }
    let Some(file) = device_of(h) else {
        return DeviceIoControl(h, code, inb, inl, outb, outl, ret, ov);
    };
    CURRENT_FILE.with(|c| c.set(file));
    let mut io = [0usize; 2];
    let (iop, ev, own) = if ov.is_null() {
        (io.as_mut_ptr(), CreateEventW(core::ptr::null(), 1, 0, core::ptr::null()), true)
    } else {
        (ov as *mut usize, (ov as *const usize).add(3).read_unaligned() & !1, false)
    };
    let mut st = ioctl(iop, ev, 0, 0, code, inb, inl, outb, outl);
    if st == STATUS_PENDING && own {
        WaitForSingleObject(ev, u32::MAX);
        st = io[0] as u32 as i32;
    }
    if own {
        CloseHandle(ev);
    }
    if !ret.is_null() && st >= 0 && st != STATUS_PENDING {
        *ret = (*iop.add(1)) as u32;
    }
    if st < 0 || st == STATUS_PENDING {
        SetLastError(if st == STATUS_PENDING { 997 } else { RtlNtStatusToDosError(st) });
        return 0;
    }
    1
}
