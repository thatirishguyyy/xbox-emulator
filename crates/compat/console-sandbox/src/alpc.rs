use core::ffi::c_void;
use std::collections::{HashMap, VecDeque};
use std::sync::{Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

pub const LPC_REQUEST: u16 = 1;
pub const LPC_REPLY: u16 = 2;
pub const LPC_DATAGRAM: u16 = 3;
pub const LPC_PORT_CLOSED: u16 = 5;
pub const LPC_CONNECTION_REQUEST: u16 = 10;

pub const ATTR_SECURITY: u32 = 0x8000_0000;
pub const ATTR_VIEW: u32 = 0x4000_0000;
pub const ATTR_CONTEXT: u32 = 0x2000_0000;
pub const ATTR_HANDLE: u32 = 0x1000_0000;
pub const HANDLEFLG_INDIRECT: u32 = 0x0004_0000;
pub const MESSAGE_HANDLE_INFORMATION: u32 = 3;
pub const HEADER: usize = 0x28;
pub const PORT_CONNECT: u32 = 1;
pub const DEFAULT_MAX_MESSAGE: usize = 0xFFFF;
pub const SECTION_GRANULARITY: usize = 0x1_0000;

pub const STATUS_SUCCESS: i32 = 0;
pub const STATUS_TIMEOUT: i32 = 0x102;
pub const STATUS_INVALID_INFO_CLASS: i32 = 0xC000_0003u32 as i32;
pub const STATUS_INFO_LENGTH_MISMATCH: i32 = 0xC000_0004u32 as i32;
pub const STATUS_INVALID_HANDLE: i32 = 0xC000_0008u32 as i32;
pub const STATUS_INVALID_PARAMETER: i32 = 0xC000_000Du32 as i32;
pub const STATUS_NOT_MAPPED_VIEW: i32 = 0xC000_0019u32 as i32;
pub const STATUS_INVALID_VIEW_SIZE: i32 = 0xC000_001Fu32 as i32;
pub const STATUS_ACCESS_DENIED: i32 = 0xC000_0022u32 as i32;
pub const STATUS_BUFFER_TOO_SMALL: i32 = 0xC000_0023u32 as i32;
pub const STATUS_PORT_MESSAGE_TOO_LONG: i32 = 0xC000_002Fu32 as i32;
pub const STATUS_OBJECT_NAME_NOT_FOUND: i32 = 0xC000_0034u32 as i32;
pub const STATUS_OBJECT_NAME_COLLISION: i32 = 0xC000_0035u32 as i32;
pub const STATUS_PORT_DISCONNECTED: i32 = 0xC000_0037u32 as i32;
pub const STATUS_OBJECT_PATH_SYNTAX_BAD: i32 = 0xC000_003Bu32 as i32;
pub const STATUS_SECTION_TOO_BIG: i32 = 0xC000_0040u32 as i32;
pub const STATUS_PORT_CONNECTION_REFUSED: i32 = 0xC000_0041u32 as i32;
pub const STATUS_NO_MEMORY: i32 = 0xC000_0017u32 as i32;
pub const STATUS_REPLY_MESSAGE_MISMATCH: i32 = 0xC000_021Fu32 as i32;

#[link(name = "ntdll")]
extern "system" {
    fn AlpcGetMessageAttribute(buf: *mut u8, attr: u32) -> *mut u8;
    fn RtlGetDaclSecurityDescriptor(sd: *const c_void, present: *mut u8, dacl: *mut *const u8, defaulted: *mut u8) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn CreateFileW(name: *const u16, access: u32, share: u32, sa: *const c_void, disp: u32, flags: u32, template: isize) -> isize;
    fn DuplicateHandle(sp: usize, sh: usize, tp: usize, th: *mut usize, access: u32, inherit: i32, opts: u32) -> i32;
    fn GetCurrentProcess() -> usize;
    fn GetCurrentProcessId() -> u32;
    fn GetCurrentThreadId() -> u32;
    fn VirtualAlloc(addr: usize, size: usize, kind: u32, protect: u32) -> usize;
    fn VirtualFree(addr: usize, size: usize, kind: u32) -> i32;
    fn MapViewOfFile(h: usize, access: u32, hi: u32, lo: u32, size: usize) -> usize;
    fn UnmapViewOfFile(base: usize) -> i32;
    fn GetSystemTimeAsFileTime(ft: *mut u64);
    fn CloseHandle(h: usize) -> i32;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PortAttrs {
    pub flags: u32,
    pub max_message: usize,
    pub max_pool: usize,
    pub max_section: usize,
    pub max_view: usize,
    pub max_total_section: usize,
    pub dup_object_types: u32,
}

impl PortAttrs {
    unsafe fn read(p: *const u8) -> PortAttrs {
        if p.is_null() {
            return PortAttrs { max_message: DEFAULT_MAX_MESSAGE, ..Default::default() };
        }
        let q = |o: usize| (p.add(o) as *const usize).read_unaligned();
        let max_message = q(0x10);
        PortAttrs {
            flags: (p as *const u32).read_unaligned(),
            max_message: if max_message == 0 { DEFAULT_MAX_MESSAGE } else { max_message },
            max_pool: q(0x20),
            max_section: q(0x28),
            max_view: q(0x30),
            max_total_section: q(0x38),
            dup_object_types: (p.add(0x40) as *const u32).read_unaligned(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    Pending,
    Connected,
    Refused,
    Disconnected,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Obj {
    Listen(u64),
    Server(u64),
    Client(u64),
}

struct Listener {
    name: Option<String>,
    attrs: PortAttrs,
    connect_allowed: bool,
    queue: VecDeque<u32>,
    closed: bool,
}

struct Msg {
    conn: u64,
    kind: u16,
    bytes: Vec<u8>,
    pid: u32,
    tid: u32,
    reply: Option<Vec<u8>>,
}

struct Conn {
    listener: u64,
    state: State,
    context: usize,
    client_attrs: PortAttrs,
    accept_reply: Vec<u8>,
    view: Option<(usize, usize)>,
    handles: Vec<(usize, u32)>,
    to_client: VecDeque<Vec<u8>>,
    client_open: bool,
    server_open: bool,
    closed_sent: bool,
}

struct Section {
    owner: u64,
    base: usize,
    size: usize,
    mapped: bool,
    deleted: bool,
    views: u32,
}

struct View {
    base: usize,
    size: usize,
    section: usize,
    owner: Obj,
}

#[derive(Default)]
struct Model {
    next: u64,
    next_msg: u32,
    handles: HashMap<usize, Obj>,
    names: HashMap<String, u64>,
    listeners: HashMap<u64, Listener>,
    conns: HashMap<u64, Conn>,
    msgs: HashMap<u32, Msg>,
    sections: HashMap<usize, Section>,
    views: Vec<View>,
}

fn cell() -> &'static (Mutex<Model>, Condvar) {
    static M: OnceLock<(Mutex<Model>, Condvar)> = OnceLock::new();
    M.get_or_init(|| (Mutex::new(Model::default()), Condvar::new()))
}

fn model() -> MutexGuard<'static, Model> {
    cell().0.lock().unwrap_or_else(|e| e.into_inner())
}

fn wake() {
    cell().1.notify_all();
}

fn wait(g: MutexGuard<'static, Model>, until: Option<Instant>) -> (MutexGuard<'static, Model>, bool) {
    match until {
        None => (cell().1.wait(g).unwrap_or_else(|e| e.into_inner()), false),
        Some(t) => {
            let now = Instant::now();
            if now >= t {
                return (g, true);
            }
            let (g, r) = cell().1.wait_timeout(g, t - now).unwrap_or_else(|e| e.into_inner());
            (g, r.timed_out() && Instant::now() >= t)
        }
    }
}

unsafe fn deadline(timeout: *const i64) -> Option<Instant> {
    if timeout.is_null() {
        return None;
    }
    let t = timeout.read_unaligned();
    let ticks = if t < 0 {
        t.unsigned_abs()
    } else {
        let mut now = 0u64;
        GetSystemTimeAsFileTime(&mut now);
        (t as u64).saturating_sub(now)
    };
    Some(Instant::now() + Duration::from_nanos(ticks.saturating_mul(100)))
}

fn key(name: &str) -> String {
    name.to_lowercase()
}

impl Model {
    fn id(&mut self) -> u64 {
        self.next += 1;
        self.next
    }

    fn msg_id(&mut self) -> u32 {
        self.next_msg = self.next_msg.wrapping_add(1).max(1);
        self.next_msg
    }

    fn queue(&mut self, listener: u64, conn: u64, kind: u16, bytes: Vec<u8>) -> u32 {
        let id = self.msg_id();
        let (pid, tid) = unsafe { (GetCurrentProcessId(), GetCurrentThreadId()) };
        self.msgs.insert(id, Msg { conn, kind, bytes, pid, tid, reply: None });
        if let Some(l) = self.listeners.get_mut(&listener) {
            l.queue.push_back(id);
        }
        id
    }

    fn release_view(&mut self, owner: Obj) {
        let (gone, keep): (Vec<View>, Vec<View>) = self.views.drain(..).partition(|v| v.owner == owner);
        self.views = keep;
        for v in gone {
            self.unref_section(v.section);
        }
    }

    fn unref_section(&mut self, id: usize) {
        let free = match self.sections.get_mut(&id) {
            Some(s) => {
                s.views = s.views.saturating_sub(1);
                s.deleted && s.views == 0
            }
            None => false,
        };
        if free {
            self.free_section(id);
        }
    }

    fn free_section(&mut self, id: usize) {
        if let Some(s) = self.sections.remove(&id) {
            unsafe {
                if s.mapped {
                    UnmapViewOfFile(s.base);
                } else {
                    VirtualFree(s.base, 0, 0x8000);
                }
            }
        }
    }

    fn client_gone(&mut self, conn: u64) {
        let Some(c) = self.conns.get_mut(&conn) else { return };
        let listener = c.listener;
        let notify = !c.closed_sent && c.state == State::Connected && c.server_open;
        if c.state == State::Connected {
            c.state = State::Disconnected;
        }
        c.closed_sent = true;
        if notify {
            let mut b = vec![0u8; HEADER];
            b[2] = HEADER as u8;
            self.queue(listener, conn, LPC_PORT_CLOSED, b);
        }
        self.release_view(Obj::Client(conn));
        self.reap(conn);
    }

    fn reap(&mut self, conn: u64) {
        if self.conns.get(&conn).is_some_and(|c| !c.client_open && !c.server_open) {
            let still_queued = self.msgs.values().any(|m| m.conn == conn);
            if !still_queued {
                self.conns.remove(&conn);
            }
        }
    }
}

unsafe fn nul_handle() -> Option<usize> {
    let nul: Vec<u16> = "NUL".encode_utf16().chain(Some(0)).collect();
    let h = CreateFileW(nul.as_ptr(), 0xC000_0000, 3, core::ptr::null(), 3, 0, 0);
    (h != -1).then_some(h as usize)
}

unsafe fn object_name(oa: *const u8) -> Result<Option<String>, i32> {
    if oa.is_null() {
        return Ok(None);
    }
    let us = (oa.add(0x10) as *const usize).read_unaligned();
    if us == 0 {
        return Ok(None);
    }
    if (oa.add(8) as *const usize).read_unaligned() != 0 {
        return Err(STATUS_OBJECT_PATH_SYNTAX_BAD);
    }
    let n = crate::wstr::from_unicode_string(us as *const u8);
    if !n.starts_with('\\') {
        return Err(STATUS_OBJECT_PATH_SYNTAX_BAD);
    }
    Ok(Some(n))
}

const WORLD_SID: [u8; 12] = [1, 1, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0];

unsafe fn world_may_connect(oa: *const u8) -> bool {
    let sd = if oa.is_null() { 0 } else { (oa.add(0x20) as *const usize).read_unaligned() };
    if sd == 0 {
        return true;
    }
    let mut present = 0u8;
    let mut dacl: *const u8 = core::ptr::null();
    let mut defaulted = 0u8;
    if RtlGetDaclSecurityDescriptor(sd as *const c_void, &mut present, &mut dacl, &mut defaulted) < 0 {
        return false;
    }
    if present == 0 || dacl.is_null() {
        return true;
    }
    let count = (dacl.add(4) as *const u16).read_unaligned() as usize;
    let mut ace = dacl.add(8);
    for _ in 0..count {
        let kind = *ace;
        let size = (ace.add(2) as *const u16).read_unaligned() as usize;
        let mask = (ace.add(4) as *const u32).read_unaligned();
        let sid = std::slice::from_raw_parts(ace.add(8), 12);
        if (kind == 0 || kind == 1) && sid == WORLD_SID && mask & (PORT_CONNECT | 0x1000_0000 | 0x2000_0000) != 0 {
            return kind == 0;
        }
        if size == 0 {
            break;
        }
        ace = ace.add(size);
    }
    false
}

unsafe fn total_of(msg: *const u8) -> usize {
    ((msg.add(2) as *const u16).read_unaligned() as usize).max(HEADER)
}

unsafe fn message_bytes(msg: *const u8) -> Vec<u8> {
    std::slice::from_raw_parts(msg, total_of(msg)).to_vec()
}

unsafe fn attr(attrs: *mut u8, bit: u32) -> *mut u8 {
    if attrs.is_null() || (attrs as *const u32).read_unaligned() & bit == 0 {
        return core::ptr::null_mut();
    }
    AlpcGetMessageAttribute(attrs, bit)
}

unsafe fn valid(attrs: *const u8, bit: u32) -> bool {
    !attrs.is_null() && (attrs.add(4) as *const u32).read_unaligned() & bit != 0
}

unsafe fn set_valid(attrs: *mut u8, bits: u32) {
    if !attrs.is_null() {
        (attrs.add(4) as *mut u32).write_unaligned(bits);
    }
}

unsafe fn deliver(m: &Model, id: u32, recv: *mut u8, len: *mut usize, attrs: *mut u8) -> Result<(), i32> {
    let msg = &m.msgs[&id];
    let total = msg.bytes.len();
    if !len.is_null() && *len < total {
        return Err(STATUS_BUFFER_TOO_SMALL);
    }
    core::ptr::copy_nonoverlapping(msg.bytes.as_ptr(), recv, total);
    (recv.add(4) as *mut u16).write_unaligned(msg.kind);
    (recv.add(6) as *mut u16).write_unaligned(0);
    (recv.add(8) as *mut u64).write_unaligned(msg.pid as u64);
    (recv.add(0x10) as *mut u64).write_unaligned(msg.tid as u64);
    (recv.add(0x18) as *mut u32).write_unaligned(id);
    if !len.is_null() {
        *len = total;
    }
    let mut bits = 0;
    let ctx = attr(attrs, ATTR_CONTEXT);
    if !ctx.is_null() {
        let context = if msg.kind == LPC_CONNECTION_REQUEST { 0 } else { m.conns.get(&msg.conn).map(|c| c.context).unwrap_or(0) };
        (ctx as *mut usize).write_unaligned(context);
        (ctx.add(8) as *mut usize).write_unaligned(0);
        (ctx.add(0x10) as *mut u32).write_unaligned(id);
        (ctx.add(0x14) as *mut u32).write_unaligned(id);
        (ctx.add(0x18) as *mut u32).write_unaligned(0);
        bits |= ATTR_CONTEXT;
    }
    set_valid(attrs, bits);
    Ok(())
}

pub unsafe extern "system" fn nt_alpc_create_port(ph: *mut usize, oa: *const u8, attrs: *const u8) -> i32 {
    if ph.is_null() {
        return STATUS_INVALID_PARAMETER;
    }
    let name = match object_name(oa) {
        Ok(n) => n,
        Err(st) => return st,
    };
    let a = PortAttrs::read(attrs);
    let allowed = world_may_connect(oa);
    let mut m = model();
    if let Some(n) = &name {
        if m.names.contains_key(&key(n)) {
            return STATUS_OBJECT_NAME_COLLISION;
        }
    }
    let Some(h) = nul_handle() else { return STATUS_NO_MEMORY };
    let id = m.id();
    if let Some(n) = &name {
        m.names.insert(key(n), id);
    }
    m.listeners.insert(id, Listener { name: name.clone(), attrs: a, connect_allowed: allowed, queue: VecDeque::new(), closed: false });
    m.handles.insert(h, Obj::Listen(id));
    drop(m);
    *ph = h;
    compat_common::record_raw("alpc", "NtAlpcCreatePort", &format!("in-process server port {} (max message {:#x}, max section {:#x})", name.as_deref().unwrap_or("<unnamed>"), a.max_message, a.max_section));
    STATUS_SUCCESS
}

pub unsafe extern "system" fn nt_alpc_connect_port(
    ph: *mut usize,
    name: *const u8,
    _oa: *const u8,
    attrs: *const u8,
    _flags: u32,
    _sid: *const c_void,
    msg: *mut u8,
    len: *mut usize,
    _out_attrs: *mut u8,
    in_attrs: *mut u8,
    timeout: *const i64,
) -> i32 {
    if ph.is_null() || name.is_null() {
        return STATUS_INVALID_PARAMETER;
    }
    let n = crate::wstr::from_unicode_string(name);
    let client_attrs = PortAttrs::read(attrs);
    let until = deadline(timeout);
    let mut m = model();
    let Some(&lid) = m.names.get(&key(&n)) else { return STATUS_OBJECT_NAME_NOT_FOUND };
    let l = &m.listeners[&lid];
    if !l.connect_allowed {
        return STATUS_ACCESS_DENIED;
    }
    let bytes = if msg.is_null() {
        let mut b = vec![0u8; HEADER];
        b[2] = HEADER as u8;
        b
    } else {
        message_bytes(msg)
    };
    if bytes.len() > l.attrs.max_message.min(client_attrs.max_message) {
        return STATUS_PORT_MESSAGE_TOO_LONG;
    }
    let cid = m.id();
    m.conns.insert(
        cid,
        Conn {
            listener: lid,
            state: State::Pending,
            context: 0,
            client_attrs,
            accept_reply: Vec::new(),
            view: None,
            handles: Vec::new(),
            to_client: VecDeque::new(),
            client_open: true,
            server_open: false,
            closed_sent: false,
        },
    );
    let mid = m.queue(lid, cid, LPC_CONNECTION_REQUEST, bytes);
    wake();
    loop {
        if m.conns.get(&cid).map(|c| c.state) != Some(State::Pending) {
            break;
        }
        let (g, timed_out) = wait(m, until);
        m = g;
        if timed_out && m.conns.get(&cid).map(|c| c.state) == Some(State::Pending) {
            m.msgs.remove(&mid);
            if let Some(l) = m.listeners.get_mut(&lid) {
                l.queue.retain(|x| *x != mid);
            }
            m.conns.remove(&cid);
            return STATUS_TIMEOUT;
        }
    }
    let c = m.conns.get(&cid).map(|c| (c.state, c.accept_reply.clone(), c.view, c.handles.clone()));
    let Some((state, reply, view, handles)) = c else { return STATUS_PORT_CONNECTION_REFUSED };
    if state != State::Connected {
        m.conns.remove(&cid);
        return STATUS_PORT_CONNECTION_REFUSED;
    }
    let Some(h) = nul_handle() else {
        m.conns.remove(&cid);
        return STATUS_NO_MEMORY;
    };
    m.handles.insert(h, Obj::Client(cid));
    drop(m);
    if !msg.is_null() && !reply.is_empty() {
        let n = if len.is_null() { reply.len() } else { reply.len().min(*len) };
        core::ptr::copy_nonoverlapping(reply.as_ptr(), msg, n);
        if !len.is_null() {
            *len = n;
        }
    }
    let mut bits = 0;
    let v = attr(in_attrs, ATTR_VIEW);
    if let (false, Some((base, size))) = (v.is_null(), view) {
        (v as *mut u32).write_unaligned(0);
        (v.add(8) as *mut usize).write_unaligned(0);
        (v.add(0x10) as *mut usize).write_unaligned(base);
        (v.add(0x18) as *mut usize).write_unaligned(size);
        bits |= ATTR_VIEW;
    }
    let ha = attr(in_attrs, ATTR_HANDLE);
    if !ha.is_null() && !handles.is_empty() {
        (ha as *mut u32).write_unaligned(if handles.len() > 1 { HANDLEFLG_INDIRECT } else { 0 });
        (ha.add(8) as *mut usize).write_unaligned(handles[0].0);
        (ha.add(0x10) as *mut u32).write_unaligned(handles.len() as u32);
        (ha.add(0x14) as *mut u32).write_unaligned(handles[0].1);
        bits |= ATTR_HANDLE;
    }
    set_valid(in_attrs, bits);
    *ph = h;
    STATUS_SUCCESS
}

pub unsafe extern "system" fn nt_alpc_accept_connect_port(
    ph: *mut usize,
    port: usize,
    _flags: u32,
    _oa: *const u8,
    attrs: *const u8,
    context: usize,
    msg: *const u8,
    msg_attrs: *mut u8,
    accept: u8,
) -> i32 {
    if msg.is_null() {
        return STATUS_INVALID_PARAMETER;
    }
    let mid = (msg.add(0x18) as *const u32).read_unaligned();
    let mut m = model();
    let Some(Obj::Listen(lid)) = m.handles.get(&port).copied() else { return STATUS_INVALID_HANDLE };
    let Some(cid) = m.msgs.get(&mid).filter(|x| x.kind == LPC_CONNECTION_REQUEST).map(|x| x.conn) else { return STATUS_REPLY_MESSAGE_MISMATCH };
    if m.conns.get(&cid).map(|c| c.listener) != Some(lid) {
        return STATUS_REPLY_MESSAGE_MISMATCH;
    }
    m.msgs.remove(&mid);
    if accept == 0 {
        if let Some(c) = m.conns.get_mut(&cid) {
            c.state = State::Refused;
        }
        wake();
        return STATUS_SUCCESS;
    }
    if ph.is_null() {
        if let Some(c) = m.conns.get_mut(&cid) {
            c.state = State::Refused;
        }
        wake();
        return STATUS_INVALID_PARAMETER;
    }
    let mut handles = Vec::new();
    if valid(msg_attrs, ATTR_HANDLE) {
        let ha = attr(msg_attrs, ATTR_HANDLE);
        if !ha.is_null() {
            let flags = (ha as *const u32).read_unaligned();
            let src: Vec<(usize, u32)> = if flags & HANDLEFLG_INDIRECT != 0 {
                let arr = (ha.add(8) as *const usize).read_unaligned() as *const u32;
                let n = (ha.add(0x10) as *const u32).read_unaligned() as usize;
                (0..n).map(|i| (arr.add(i * 4 + 1).read_unaligned() as usize, arr.add(i * 4 + 3).read_unaligned())).collect()
            } else {
                vec![((ha.add(8) as *const usize).read_unaligned(), (ha.add(0x14) as *const u32).read_unaligned())]
            };
            for (h, access) in src {
                let mut dup = 0usize;
                let opts = if access == 0 { 2 } else { 0 };
                if DuplicateHandle(GetCurrentProcess(), h, GetCurrentProcess(), &mut dup, access, 0, opts) == 0 {
                    for (d, _) in handles.drain(..) {
                        CloseHandle(d);
                    }
                    if let Some(c) = m.conns.get_mut(&cid) {
                        c.state = State::Refused;
                    }
                    wake();
                    return STATUS_INVALID_HANDLE;
                }
                handles.push((dup, access));
            }
        }
    }
    let _server_attrs = PortAttrs::read(attrs);
    let mut view = None;
    if valid(msg_attrs, ATTR_VIEW) {
        let v = attr(msg_attrs, ATTR_VIEW);
        if !v.is_null() {
            let base = (v.add(0x10) as *const usize).read_unaligned();
            let size = (v.add(0x18) as *const usize).read_unaligned();
            let Some(sec) = m.views.iter().find(|x| x.base == base).map(|x| x.section) else {
                if let Some(c) = m.conns.get_mut(&cid) {
                    c.state = State::Refused;
                }
                wake();
                return STATUS_NOT_MAPPED_VIEW;
            };
            m.views.push(View { base, size, section: sec, owner: Obj::Client(cid) });
            if let Some(s) = m.sections.get_mut(&sec) {
                s.views += 1;
            }
            view = Some((base, size));
        }
    }
    let Some(h) = nul_handle() else { return STATUS_NO_MEMORY };
    let reply = message_bytes(msg);
    if let Some(c) = m.conns.get_mut(&cid) {
        c.state = State::Connected;
        c.context = context;
        c.accept_reply = reply;
        c.view = view;
        c.handles = handles;
        c.server_open = true;
    }
    m.handles.insert(h, Obj::Server(cid));
    drop(m);
    wake();
    *ph = h;
    STATUS_SUCCESS
}

pub unsafe extern "system" fn nt_alpc_send_wait_receive_port(
    port: usize,
    _flags: u32,
    send: *const u8,
    _send_attrs: *mut u8,
    recv: *mut u8,
    len: *mut usize,
    recv_attrs: *mut u8,
    timeout: *const i64,
) -> i32 {
    let until = deadline(timeout);
    let mut m = model();
    let Some(obj) = m.handles.get(&port).copied() else { return STATUS_INVALID_HANDLE };
    match obj {
        Obj::Listen(lid) => {
            if !send.is_null() {
                let id = (send.add(0x18) as *const u32).read_unaligned();
                let bytes = message_bytes(send);
                let max = m.listeners[&lid].attrs.max_message;
                if bytes.len() > max {
                    return STATUS_PORT_MESSAGE_TOO_LONG;
                }
                let ok = m.msgs.get(&id).is_some_and(|x| x.kind == LPC_REQUEST && x.reply.is_none() && m.conns.get(&x.conn).is_some_and(|c| c.listener == lid));
                if !ok {
                    return STATUS_REPLY_MESSAGE_MISMATCH;
                }
                m.msgs.get_mut(&id).unwrap().reply = Some(bytes);
                wake();
            }
            if recv.is_null() {
                return STATUS_SUCCESS;
            }
            loop {
                let Some(l) = m.listeners.get(&lid) else { return STATUS_PORT_DISCONNECTED };
                if let Some(&id) = l.queue.front() {
                    if let Err(st) = deliver(&m, id, recv, len, recv_attrs) {
                        return st;
                    }
                    m.listeners.get_mut(&lid).unwrap().queue.pop_front();
                    let (kind, conn) = { let x = &m.msgs[&id]; (x.kind, x.conn) };
                    if kind == LPC_PORT_CLOSED || kind == LPC_DATAGRAM {
                        m.msgs.remove(&id);
                        m.reap(conn);
                    }
                    return STATUS_SUCCESS;
                }
                if l.closed {
                    return STATUS_PORT_DISCONNECTED;
                }
                let (g, timed_out) = wait(m, until);
                m = g;
                if timed_out && m.listeners.get(&lid).is_some_and(|l| l.queue.is_empty()) {
                    return STATUS_TIMEOUT;
                }
            }
        }
        Obj::Client(cid) => {
            let Some(c) = m.conns.get(&cid) else { return STATUS_PORT_DISCONNECTED };
            let lid = c.listener;
            if send.is_null() {
                if recv.is_null() {
                    return STATUS_INVALID_PARAMETER;
                }
                loop {
                    let Some(c) = m.conns.get_mut(&cid) else { return STATUS_PORT_DISCONNECTED };
                    if let Some(b) = c.to_client.front() {
                        if !len.is_null() && *len < b.len() {
                            return STATUS_BUFFER_TOO_SMALL;
                        }
                        let b = c.to_client.pop_front().unwrap();
                        core::ptr::copy_nonoverlapping(b.as_ptr(), recv, b.len());
                        if !len.is_null() {
                            *len = b.len();
                        }
                        set_valid(recv_attrs, 0);
                        return STATUS_SUCCESS;
                    }
                    if c.state != State::Connected {
                        return STATUS_PORT_DISCONNECTED;
                    }
                    let (g, timed_out) = wait(m, until);
                    m = g;
                    if timed_out {
                        return STATUS_TIMEOUT;
                    }
                }
            }
            if c.state != State::Connected {
                return STATUS_PORT_DISCONNECTED;
            }
            let bytes = message_bytes(send);
            let max = m.listeners.get(&lid).map(|l| l.attrs.max_message).unwrap_or(0).min(c.client_attrs.max_message);
            if bytes.len() > max {
                return STATUS_PORT_MESSAGE_TOO_LONG;
            }
            let kind = if recv.is_null() { LPC_DATAGRAM } else { LPC_REQUEST };
            let id = m.queue(lid, cid, kind, bytes);
            wake();
            if recv.is_null() {
                return STATUS_SUCCESS;
            }
            loop {
                if let Some(r) = m.msgs.get(&id).and_then(|x| x.reply.clone()) {
                    m.msgs.remove(&id);
                    if !len.is_null() && *len < r.len() {
                        return STATUS_BUFFER_TOO_SMALL;
                    }
                    core::ptr::copy_nonoverlapping(r.as_ptr(), recv, r.len());
                    (recv.add(4) as *mut u16).write_unaligned(LPC_REPLY);
                    (recv.add(0x18) as *mut u32).write_unaligned(id);
                    if !len.is_null() {
                        *len = r.len();
                    }
                    set_valid(recv_attrs, 0);
                    return STATUS_SUCCESS;
                }
                if m.conns.get(&cid).map(|c| c.state) != Some(State::Connected) {
                    m.msgs.remove(&id);
                    if let Some(l) = m.listeners.get_mut(&lid) {
                        l.queue.retain(|x| *x != id);
                    }
                    return STATUS_PORT_DISCONNECTED;
                }
                let (g, timed_out) = wait(m, until);
                m = g;
                if timed_out && m.msgs.get(&id).is_some_and(|x| x.reply.is_none()) {
                    m.msgs.remove(&id);
                    if let Some(l) = m.listeners.get_mut(&lid) {
                        l.queue.retain(|x| *x != id);
                    }
                    return STATUS_TIMEOUT;
                }
            }
        }
        Obj::Server(cid) => {
            if !recv.is_null() && send.is_null() {
                return STATUS_INVALID_PARAMETER;
            }
            let Some(c) = m.conns.get_mut(&cid) else { return STATUS_PORT_DISCONNECTED };
            if c.state != State::Connected {
                return STATUS_PORT_DISCONNECTED;
            }
            if send.is_null() {
                return STATUS_INVALID_PARAMETER;
            }
            let mut b = message_bytes(send);
            (b.as_mut_ptr().add(4) as *mut u16).write_unaligned(LPC_DATAGRAM);
            c.to_client.push_back(b);
            wake();
            STATUS_SUCCESS
        }
    }
}

pub unsafe extern "system" fn nt_alpc_open_sender_process(ph: *mut usize, port: usize, msg: *const u8, _flags: u32, access: u32, _oa: *const u8) -> i32 {
    if ph.is_null() || msg.is_null() {
        return STATUS_INVALID_PARAMETER;
    }
    let id = (msg.add(0x18) as *const u32).read_unaligned();
    let m = model();
    let Some(obj) = m.handles.get(&port).copied() else { return STATUS_INVALID_HANDLE };
    let lid = match obj {
        Obj::Listen(l) => l,
        Obj::Server(c) | Obj::Client(c) => match m.conns.get(&c) {
            Some(c) => c.listener,
            None => return STATUS_PORT_DISCONNECTED,
        },
    };
    let Some(x) = m.msgs.get(&id) else { return STATUS_INVALID_PARAMETER };
    if m.conns.get(&x.conn).map(|c| c.listener) != Some(lid) || x.pid != GetCurrentProcessId() {
        return STATUS_INVALID_PARAMETER;
    }
    drop(m);
    let mut h = 0usize;
    let same = access & 0x0200_0000 != 0;
    if DuplicateHandle(GetCurrentProcess(), GetCurrentProcess(), GetCurrentProcess(), &mut h, if same { 0 } else { access }, 0, if same { 2 } else { 0 }) == 0 {
        return STATUS_ACCESS_DENIED;
    }
    *ph = h;
    STATUS_SUCCESS
}

fn port_listener(m: &Model, port: usize) -> Option<(u64, Obj)> {
    let obj = m.handles.get(&port).copied()?;
    let lid = match obj {
        Obj::Listen(l) => l,
        Obj::Server(c) | Obj::Client(c) => m.conns.get(&c)?.listener,
    };
    Some((lid, obj))
}

pub unsafe extern "system" fn nt_alpc_create_port_section(port: usize, _flags: u32, section: usize, size: usize, out: *mut usize, actual: *mut usize) -> i32 {
    if out.is_null() || size == 0 {
        return STATUS_INVALID_PARAMETER;
    }
    let mut m = model();
    let Some((lid, _)) = port_listener(&m, port) else { return STATUS_INVALID_HANDLE };
    let a = m.listeners.get(&lid).map(|l| l.attrs).unwrap_or_default();
    let rounded = size.div_ceil(SECTION_GRANULARITY) * SECTION_GRANULARITY;
    if a.max_section != 0 && size > a.max_section {
        return STATUS_SECTION_TOO_BIG;
    }
    let used: usize = m.sections.values().filter(|s| s.owner == lid && !s.deleted).map(|s| s.size).sum();
    if a.max_total_section != 0 && used + rounded > a.max_total_section {
        return STATUS_SECTION_TOO_BIG;
    }
    let (base, mapped) = if section != 0 { (MapViewOfFile(section, 6, 0, 0, size), true) } else { (VirtualAlloc(0, rounded, 0x3000, 4), false) };
    if base == 0 {
        return STATUS_NO_MEMORY;
    }
    let id = 0x5A1C_0000 + (m.id() as usize) * 8;
    m.sections.insert(id, Section { owner: lid, base, size: rounded, mapped, deleted: false, views: 0 });
    *out = id;
    if !actual.is_null() {
        *actual = rounded;
    }
    STATUS_SUCCESS
}

pub unsafe extern "system" fn nt_alpc_delete_port_section(port: usize, _flags: u32, section: usize) -> i32 {
    let mut m = model();
    let Some((lid, _)) = port_listener(&m, port) else { return STATUS_INVALID_HANDLE };
    let free = match m.sections.get_mut(&section) {
        Some(s) if s.owner == lid && !s.deleted => {
            s.deleted = true;
            s.views == 0
        }
        _ => return STATUS_INVALID_HANDLE,
    };
    if free {
        m.free_section(section);
    }
    STATUS_SUCCESS
}

pub unsafe extern "system" fn nt_alpc_create_section_view(port: usize, _flags: u32, view: *mut u8) -> i32 {
    if view.is_null() {
        return STATUS_INVALID_PARAMETER;
    }
    let mut m = model();
    let Some((lid, obj)) = port_listener(&m, port) else { return STATUS_INVALID_HANDLE };
    let sec = (view.add(8) as *const usize).read_unaligned();
    let want = (view.add(0x18) as *const usize).read_unaligned();
    let max_view = m.listeners.get(&lid).map(|l| l.attrs.max_view).unwrap_or(0);
    let Some(s) = m.sections.get_mut(&sec).filter(|s| s.owner == lid && !s.deleted) else { return STATUS_INVALID_HANDLE };
    let size = if want == 0 { s.size } else { want };
    if size > s.size || (max_view != 0 && size > max_view) {
        return STATUS_INVALID_VIEW_SIZE;
    }
    s.views += 1;
    let base = s.base;
    m.views.push(View { base, size, section: sec, owner: obj });
    (view.add(0x10) as *mut usize).write_unaligned(base);
    (view.add(0x18) as *mut usize).write_unaligned(size);
    STATUS_SUCCESS
}

pub unsafe extern "system" fn nt_alpc_delete_section_view(port: usize, _flags: u32, base: usize) -> i32 {
    let mut m = model();
    let Some((_, obj)) = port_listener(&m, port) else { return STATUS_INVALID_HANDLE };
    let owners = |o: Obj| o == obj || matches!((o, obj), (Obj::Listen(a), Obj::Listen(b)) if a == b);
    let Some(i) = m.views.iter().position(|v| v.base == base && owners(v.owner)) else { return STATUS_NOT_MAPPED_VIEW };
    let v = m.views.remove(i);
    m.unref_section(v.section);
    STATUS_SUCCESS
}

pub unsafe extern "system" fn nt_alpc_disconnect_port(port: usize, _flags: u32) -> i32 {
    let mut m = model();
    let Some(obj) = m.handles.get(&port).copied() else { return STATUS_INVALID_HANDLE };
    match obj {
        Obj::Server(cid) => {
            if let Some(c) = m.conns.get_mut(&cid) {
                if c.state == State::Connected {
                    c.state = State::Disconnected;
                }
                c.closed_sent = true;
            }
        }
        Obj::Client(cid) => m.client_gone(cid),
        Obj::Listen(lid) => close_listener(&mut m, lid),
    }
    drop(m);
    wake();
    STATUS_SUCCESS
}

fn close_listener(m: &mut Model, lid: u64) {
    let Some(l) = m.listeners.get_mut(&lid) else { return };
    l.closed = true;
    let name = l.name.take();
    if let Some(n) = name {
        m.names.remove(&key(&n));
    }
    let ids: Vec<u64> = m.conns.iter().filter(|(_, c)| c.listener == lid).map(|(k, _)| *k).collect();
    for cid in ids {
        if let Some(c) = m.conns.get_mut(&cid) {
            c.state = if c.state == State::Pending { State::Refused } else { State::Disconnected };
        }
    }
}

pub unsafe extern "system" fn nt_alpc_query_information_message(port: usize, _msg: *const u8, class: u32, info: *mut u8, len: u32, ret: *mut u32) -> i32 {
    let m = model();
    let Some(obj) = m.handles.get(&port).copied() else { return STATUS_INVALID_HANDLE };
    if class != MESSAGE_HANDLE_INFORMATION {
        return STATUS_INVALID_INFO_CLASS;
    }
    if info.is_null() || len < 0x14 {
        return STATUS_INFO_LENGTH_MISMATCH;
    }
    let Obj::Client(cid) = obj else { return STATUS_INVALID_PARAMETER };
    let Some(c) = m.conns.get(&cid) else { return STATUS_PORT_DISCONNECTED };
    let index = (info as *const u32).read_unaligned() as usize;
    let Some(&(h, access)) = c.handles.get(index) else { return STATUS_INVALID_PARAMETER };
    (info.add(4) as *mut u32).write_unaligned(0);
    (info.add(8) as *mut u32).write_unaligned(h as u32);
    (info.add(0xc) as *mut u32).write_unaligned(0);
    (info.add(0x10) as *mut u32).write_unaligned(access);
    if !ret.is_null() {
        *ret = 0x14;
    }
    STATUS_SUCCESS
}

pub fn is_port(h: usize) -> bool {
    model().handles.contains_key(&h)
}

pub fn before_close(h: usize) {
    let mut m = model();
    let Some(obj) = m.handles.remove(&h) else { return };
    match obj {
        Obj::Client(cid) => {
            m.client_gone(cid);
            if let Some(c) = m.conns.get_mut(&cid) {
                c.client_open = false;
            }
            m.reap(cid);
        }
        Obj::Server(cid) => {
            m.release_view(Obj::Server(cid));
            if let Some(c) = m.conns.get_mut(&cid) {
                c.server_open = false;
                if c.state == State::Connected {
                    c.state = State::Disconnected;
                }
            }
            m.reap(cid);
        }
        Obj::Listen(lid) => {
            close_listener(&mut m, lid);
            m.release_view(Obj::Listen(lid));
            let queued: Vec<u32> = m.listeners.get(&lid).map(|l| l.queue.iter().copied().collect()).unwrap_or_default();
            for id in queued {
                if m.msgs.get(&id).is_some_and(|x| x.kind != LPC_REQUEST) {
                    m.msgs.remove(&id);
                }
            }
            m.listeners.remove(&lid);
        }
    }
    drop(m);
    wake();
}

pub fn port_exists(name: &str) -> bool {
    model().names.contains_key(&key(name))
}

pub fn connection_count(name: &str) -> usize {
    let m = model();
    let Some(&lid) = m.names.get(&key(name)) else { return 0 };
    m.conns.values().filter(|c| c.listener == lid && c.state == State::Connected).count()
}

pub fn section_count(name: &str) -> usize {
    let m = model();
    let Some(&lid) = m.names.get(&key(name)) else { return 0 };
    m.sections.values().filter(|s| s.owner == lid).count()
}

pub fn view_count() -> usize {
    model().views.len()
}

pub fn wait_for(cond: impl Fn() -> bool, limit: Duration) -> bool {
    let t = Instant::now();
    while t.elapsed() < limit {
        if cond() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    cond()
}

#[cfg(test)]
mod tests;
