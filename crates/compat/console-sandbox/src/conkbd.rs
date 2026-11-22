use core::ffi::c_void;
use std::collections::VecDeque;
use std::sync::{Mutex, MutexGuard, OnceLock};

pub const KEYBOARD_INTERFACE: [u8; 16] = [0xc3, 0x96, 0x4b, 0x88, 0xef, 0x56, 0xd1, 0x11, 0xbc, 0x8c, 0x00, 0xa0, 0xc9, 0x14, 0x05, 0xdd];
pub const KEY_BREAK: u16 = 1;
pub const KEY_E0: u16 = 2;
pub const KEY_E1: u16 = 4;
pub const INPUT_SIZE: usize = 12;

const STATUS_SUCCESS: i32 = 0;
const STATUS_PENDING: i32 = 0x103;
const STATUS_INVALID_PARAMETER: i32 = 0xC000_000Du32 as i32;
const STATUS_BUFFER_TOO_SMALL: i32 = 0xC000_0023u32 as i32;
const STATUS_INSUFFICIENT_RESOURCES: i32 = 0xC000_009Au32 as i32;
const STATUS_CANCELLED: i32 = 0xC000_0120u32 as i32;
const STATUS_DELETE_PENDING: i32 = 0xC000_0056u32 as i32;
const FILE_OPENED: usize = 1;

#[link(name = "ntdll")]
extern "system" {
    fn NtReadFile(h: usize, ev: usize, apc: usize, ctx: usize, io: *mut usize, buf: *mut u8, len: u32, off: *const i64, key: *const u32) -> i32;
    fn NtQueueApcThread(thread: usize, f: usize, a1: usize, a2: usize, a3: usize) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn CreateFileW(name: *const u16, access: u32, share: u32, sa: *const c_void, disp: u32, flags: u32, template: isize) -> isize;
    fn SetEvent(h: usize) -> i32;
    fn ResetEvent(h: usize) -> i32;
    fn GetCurrentThread() -> usize;
    fn GetCurrentProcess() -> usize;
    fn DuplicateHandle(sp: usize, sh: usize, tp: usize, th: *mut usize, access: u32, inherit: i32, opts: u32) -> i32;
    fn CloseHandle(h: usize) -> i32;
}

struct Read {
    handle: usize,
    io: usize,
    event: usize,
    apc: usize,
    ctx: usize,
    thread: usize,
    buf: usize,
    len: usize,
}

impl Read {
    unsafe fn complete(self, status: i32, data: &[u8]) {
        let n = data.len().min(self.len);
        if n > 0 {
            core::ptr::copy_nonoverlapping(data.as_ptr(), self.buf as *mut u8, n);
        }
        if self.io != 0 {
            let io = self.io as *mut usize;
            io.write_unaligned(status as u32 as usize);
            io.add(1).write_unaligned(n);
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

struct Keyboard {
    link: String,
    instance: String,
    handles: Vec<usize>,
    reads: VecDeque<Read>,
    buffered: VecDeque<[u8; INPUT_SIZE]>,
    removed: bool,
}

#[derive(Default)]
struct Devices {
    next: u32,
    keyboards: Vec<Keyboard>,
}

fn devices() -> MutexGuard<'static, Devices> {
    static D: OnceLock<Mutex<Devices>> = OnceLock::new();
    D.get_or_init(|| Mutex::new(Devices::default())).lock().unwrap_or_else(|e| e.into_inner())
}

fn strip(name: &str) -> &str {
    for p in ["\\??\\", "\\\\?\\", "\\DosDevices\\", "\\GLOBAL??\\"] {
        if name.len() >= p.len() && name[..p.len()].eq_ignore_ascii_case(p) {
            return &name[p.len()..];
        }
    }
    name
}

pub fn attach() -> String {
    let mut d = devices();
    d.next += 1;
    let n = d.next;
    let instance = format!("TOMBOX\\KEYBOARD\\{n}");
    let link = format!("\\\\?\\TOMBOX#KEYBOARD#{n}#{{884b96c3-56ef-11d1-bc8c-00a0c91405dd}}");
    d.keyboards.push(Keyboard { link: link.clone(), instance: instance.clone(), handles: Vec::new(), reads: VecDeque::new(), buffered: VecDeque::new(), removed: false });
    drop(d);
    crate::cfgmgr::add_interface(KEYBOARD_INTERFACE, &link, &instance);
    compat_common::record_raw("conkbd", "attach", &format!("emulated keyboard {link} arrived"));
    link
}

pub fn detach(link: &str) -> usize {
    let handles: Vec<usize> = {
        let mut d = devices();
        let Some(k) = d.keyboards.iter_mut().find(|k| k.link.eq_ignore_ascii_case(link)) else { return 0 };
        k.removed = true;
        k.handles.clone()
    };
    crate::cfgmgr::remove_interface(link);
    crate::cfgmgr::notify_handles(|t| handles.contains(&t), crate::cfgmgr::ACTION_REMOVE_COMPLETE)
}

pub fn input(link: &str, make: u16, flags: u16) -> bool {
    let mut rec = [0u8; INPUT_SIZE];
    rec[2..4].copy_from_slice(&make.to_le_bytes());
    rec[4..6].copy_from_slice(&flags.to_le_bytes());
    let read = {
        let mut d = devices();
        let Some(k) = d.keyboards.iter_mut().find(|k| k.link.eq_ignore_ascii_case(link) && !k.removed) else { return false };
        match k.reads.pop_front() {
            Some(r) => Some(r),
            None => {
                k.buffered.push_back(rec);
                None
            }
        }
    };
    if let Some(r) = read {
        unsafe { r.complete(STATUS_SUCCESS, &rec) };
    }
    true
}

pub fn pending_reads(link: &str) -> usize {
    devices().keyboards.iter().find(|k| k.link.eq_ignore_ascii_case(link)).map(|k| k.reads.len()).unwrap_or(0)
}

pub fn open_handles(link: &str) -> Vec<usize> {
    devices().keyboards.iter().find(|k| k.link.eq_ignore_ascii_case(link)).map(|k| k.handles.clone()).unwrap_or_default()
}

pub fn is_keyboard_name(name: &str) -> bool {
    let n = strip(name.trim_end_matches('\\'));
    devices().keyboards.iter().any(|k| strip(&k.link).eq_ignore_ascii_case(n))
}

pub fn is_device(h: usize) -> bool {
    devices().keyboards.iter().any(|k| k.handles.contains(&h))
}

pub unsafe fn open(name: &str, h: *mut usize, io: *mut usize) -> i32 {
    let n = strip(name.trim_end_matches('\\')).to_string();
    let nul: Vec<u16> = "NUL".encode_utf16().chain(Some(0)).collect();
    let f = CreateFileW(nul.as_ptr(), 0xC000_0000, 3, core::ptr::null(), 3, 0, 0);
    if f == -1 {
        return STATUS_INSUFFICIENT_RESOURCES;
    }
    let mut d = devices();
    let Some(k) = d.keyboards.iter_mut().find(|k| strip(&k.link).eq_ignore_ascii_case(&n)) else {
        CloseHandle(f as usize);
        return crate::xvcmgmt::STATUS_OBJECT_NAME_NOT_FOUND;
    };
    if k.removed {
        CloseHandle(f as usize);
        return STATUS_DELETE_PENDING;
    }
    k.handles.push(f as usize);
    *h = f as usize;
    if !io.is_null() {
        io.write_unaligned(0);
        io.add(1).write_unaligned(FILE_OPENED);
    }
    let instance = k.instance.clone();
    drop(d);
    compat_common::record_raw("conkbd", "NtCreateFile", &format!("opened emulated keyboard {instance}"));
    STATUS_SUCCESS
}

pub unsafe extern "system" fn nt_read_file(h: usize, ev: usize, apc: usize, ctx: usize, io: *mut usize, buf: *mut u8, len: u32, off: *const i64, key: *const u32) -> i32 {
    if !is_device(h) {
        return NtReadFile(h, ev, apc, ctx, io, buf, len, off, key);
    }
    if buf.is_null() {
        return STATUS_INVALID_PARAMETER;
    }
    if (len as usize) < INPUT_SIZE {
        return STATUS_BUFFER_TOO_SMALL;
    }
    if ev != 0 {
        ResetEvent(ev);
    }
    let mut thread = 0usize;
    if apc != 0 {
        DuplicateHandle(GetCurrentProcess(), GetCurrentThread(), GetCurrentProcess(), &mut thread, 0, 0, 2);
    }
    let read = Read { handle: h, io: io as usize, event: ev, apc, ctx, thread, buf: buf as usize, len: len as usize };
    let mut d = devices();
    let Some(k) = d.keyboards.iter_mut().find(|k| k.handles.contains(&h)) else {
        drop(d);
        read.complete(STATUS_CANCELLED, &[]);
        return STATUS_CANCELLED;
    };
    let n = (len as usize / INPUT_SIZE).max(1);
    if !k.buffered.is_empty() {
        let mut data = Vec::new();
        while data.len() / INPUT_SIZE < n {
            match k.buffered.pop_front() {
                Some(r) => data.extend_from_slice(&r),
                None => break,
            }
        }
        drop(d);
        read.complete(STATUS_SUCCESS, &data);
        return STATUS_SUCCESS;
    }
    k.reads.push_back(read);
    STATUS_PENDING
}

pub fn before_close(h: usize) {
    let reads: Vec<Read> = {
        let mut d = devices();
        let Some(k) = d.keyboards.iter_mut().find(|k| k.handles.contains(&h)) else { return };
        k.handles.retain(|x| *x != h);
        let (gone, keep): (VecDeque<Read>, VecDeque<Read>) = k.reads.drain(..).partition(|r| r.handle == h);
        k.reads = keep;
        let reads: Vec<Read> = gone.into_iter().collect();
        d.keyboards.retain(|k| !(k.removed && k.handles.is_empty()));
        reads
    };
    for r in reads {
        unsafe { r.complete(STATUS_CANCELLED, &[]) };
    }
}
