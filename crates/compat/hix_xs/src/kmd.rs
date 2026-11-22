use core::ffi::c_void;
use std::sync::atomic::{fence, AtomicU32, AtomicUsize, Ordering};

pub const DEVICE: &str = r"\\.\XgsKmd10";
pub const PSEUDO: isize = -0x4b4d0;
pub const IOCTL_HANDSHAKE: u32 = 0x7900_c190;
pub const IOCTL_DOORBELL: u32 = 0x7900_c19c;
pub const IOCTL_ARM: u32 = 0x7900_c1a3;
pub const IOCTL_MAP: u32 = 0x7900_c1b0;
pub const DOORBELL_MAGIC: u32 = 0x4321_4321;
pub const REGION0_SIZE: usize = 0x49_0000;
pub const REGION1_SIZE: usize = crate::REGION1_SIZE;
pub const MAILBOX_SEQ: usize = 0x1430;
pub const MAILBOX_CMD: usize = 0x1434;
pub const DATA_ACK: usize = 0xa040;
pub const DATA_STATUS: usize = 0xa048;
pub const DATA_SIZE: usize = 0xa04c;
pub const STATUS_NOT_SUPPORTED: u32 = 0xc000_00bb;
pub const ERROR_INVALID_FUNCTION: u32 = 1;
pub const ERROR_INVALID_PARAMETER: u32 = 87;
pub const ERROR_INVALID_HANDLE: u32 = 6;

static REGION0: AtomicUsize = AtomicUsize::new(0);
static REGION1: AtomicUsize = AtomicUsize::new(0);
static ARMED: AtomicU32 = AtomicU32::new(0);
static ARM_VALUE: AtomicU32 = AtomicU32::new(0);
static LAST_CMD: AtomicU32 = AtomicU32::new(u32::MAX);
static REQUESTS: AtomicU32 = AtomicU32::new(0);
static HISTORY: std::sync::Mutex<Vec<u32>> = std::sync::Mutex::new(Vec::new());

extern "system" {
    fn VirtualAlloc(addr: *mut c_void, size: usize, kind: u32, protect: u32) -> *mut c_void;
    fn SetLastError(e: u32);
}

fn region(slot: &AtomicUsize, size: usize) -> usize {
    let cur = slot.load(Ordering::Acquire);
    if cur != 0 {
        return cur;
    }
    let p = unsafe { VirtualAlloc(core::ptr::null_mut(), size, 0x3000, 4) } as usize;
    match slot.compare_exchange(0, p, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => {
            if slot as *const _ == &REGION1 as *const _ {
                unsafe { crate::set_region1(p as *mut u8) };
            }
            p
        }
        Err(other) => other,
    }
}

pub fn region0() -> usize {
    region(&REGION0, REGION0_SIZE)
}

pub fn region1() -> usize {
    region(&REGION1, REGION1_SIZE)
}

pub fn armed() -> Option<u32> {
    (ARMED.load(Ordering::Acquire) != 0).then(|| ARM_VALUE.load(Ordering::Acquire))
}

pub fn disarm() {
    ARMED.store(0, Ordering::Release);
    ARM_VALUE.store(0, Ordering::Release);
}

pub fn last_command() -> Option<u32> {
    let c = LAST_CMD.load(Ordering::Acquire);
    (c != u32::MAX).then_some(c)
}

pub fn history() -> Vec<u32> {
    HISTORY.lock().unwrap().clone()
}

pub fn clear_history() {
    HISTORY.lock().unwrap().clear();
}

pub fn last_status() -> u32 {
    unsafe { rd(region0() + DATA_STATUS) }
}

pub fn requests() -> u32 {
    REQUESTS.load(Ordering::Acquire)
}

pub fn is_device(name: &str) -> bool {
    name.eq_ignore_ascii_case(DEVICE)
}

unsafe fn rd(p: usize) -> u32 {
    (p as *const u32).read_volatile()
}

unsafe fn wr(p: usize, v: u32) {
    (p as *mut u32).write_volatile(v)
}

unsafe fn service() {
    let mb = region1();
    let data = region0();
    fence(Ordering::SeqCst);
    let seq = rd(mb + MAILBOX_SEQ);
    let cmd = rd(mb + MAILBOX_CMD);
    LAST_CMD.store(cmd, Ordering::Release);
    REQUESTS.fetch_add(1, Ordering::AcqRel);
    HISTORY.lock().unwrap().push(cmd);
    let (status, size) = crate::hang_stream::respond(cmd, data, mb);
    wr(data + DATA_STATUS, status);
    wr(data + DATA_SIZE, size);
    fence(Ordering::SeqCst);
    wr(data + DATA_ACK, seq);
    fence(Ordering::SeqCst);
}

pub unsafe fn ioctl(code: u32, inb: *const u8, inl: u32, outb: *mut u8, outl: u32, ret: *mut u32) -> i32 {
    if !ret.is_null() {
        *ret = 0;
    }
    match code {
        IOCTL_HANDSHAKE => 1,
        IOCTL_MAP => {
            if inb.is_null() || inl < 4 || outb.is_null() || outl < 16 {
                SetLastError(ERROR_INVALID_PARAMETER);
                return 0;
            }
            let (p, n) = match (inb as *const u32).read_unaligned() {
                0 => (region0(), REGION0_SIZE),
                1 => (region1(), REGION1_SIZE),
                _ => {
                    SetLastError(ERROR_INVALID_PARAMETER);
                    return 0;
                }
            };
            (outb as *mut u64).write_unaligned(p as u64);
            (outb.add(8) as *mut u64).write_unaligned(n as u64);
            if !ret.is_null() {
                *ret = 16;
            }
            1
        }
        IOCTL_ARM => {
            if inb.is_null() || inl < 4 {
                SetLastError(ERROR_INVALID_PARAMETER);
                return 0;
            }
            ARM_VALUE.store((inb as *const u32).read_unaligned(), Ordering::Release);
            ARMED.store(1, Ordering::Release);
            1
        }
        IOCTL_DOORBELL => {
            if inb.is_null() || inl < 8 || (inb as *const u32).read_unaligned() != DOORBELL_MAGIC {
                SetLastError(ERROR_INVALID_PARAMETER);
                return 0;
            }
            service();
            1
        }
        _ => {
            SetLastError(ERROR_INVALID_FUNCTION);
            0
        }
    }
}

pub unsafe extern "system" fn direct_graphics_call(_code: u32, _a: *mut c_void, _b: *mut c_void, _c: *mut c_void, _d: *mut c_void) -> u32 {
    STATUS_NOT_SUPPORTED
}
