use core::ffi::c_void;
use core::sync::atomic::{AtomicI32, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;

extern "system" {
    fn GetModuleHandleExW(flags: u32, addr: *const u16, out: *mut *mut c_void) -> i32;
    fn GetModuleFileNameW(m: *mut c_void, buf: *mut u16, n: u32) -> u32;
}

const SLOTS: usize = 24;
static CALLS: [AtomicU64; SLOTS] = [const { AtomicU64::new(0) }; SLOTS];
static LAST_HR: AtomicI32 = AtomicI32::new(1);
static HR_LOGS: AtomicU32 = AtomicU32::new(0);
static STATE_LOGS: AtomicU32 = AtomicU32::new(0);
static LAST_DEV: AtomicUsize = AtomicUsize::new(0);
static LAST_STATUS: AtomicU32 = AtomicU32::new(u32::MAX);
static LAST_STATE: Mutex<Option<[u8; 28]>> = Mutex::new(None);
static SEEN: Mutex<Vec<(usize, u32)>> = Mutex::new(Vec::new());

const SLOT_NAMES: [&str; SLOTS] = [
    "QueryInterface", "AddRef", "Release", "GetCurrentTimestamp", "GetCurrentReading", "GetNextReading", "GetPreviousReading",
    "GetTemporalReading", "RegisterReadingCallback", "RegisterDeviceCallback", "RegisterSystemButtonCallback",
    "RegisterKeyboardLayoutCallback", "StopCallback", "UnregisterCallback", "CreateDispatcher", "CreateAggregateDevice",
    "FindDeviceFromId", "FindDeviceFromObject", "FindDeviceFromPlatformHandle", "FindDeviceFromPlatformString",
    "EnableOemDeviceSupport", "SetFocusPolicy", "slot22", "slot23",
];

pub fn module_of(addr: usize) -> String {
    let mut m: *mut c_void = core::ptr::null_mut();
    if unsafe { GetModuleHandleExW(0x6, addr as *const u16, &mut m) } == 0 || m.is_null() {
        return format!("{addr:#x} (no module)");
    }
    let mut buf = [0u16; 520];
    let n = unsafe { GetModuleFileNameW(m, buf.as_mut_ptr(), buf.len() as u32) } as usize;
    format!("{} (+{:#x})", String::from_utf16_lossy(&buf[..n]), addr - m as usize)
}

static HIDDEN_DROPS: AtomicU64 = AtomicU64::new(0);

fn hide_reading_kind(kind: u32) -> bool {
    crate::host_input::hide_reading(kind, crate::host_input::pad_hidden(), crate::host_input::kbm_hidden())
}

unsafe fn supported(dev: *mut c_void) -> u32 {
    let info: unsafe extern "system" fn(*mut c_void) -> *const u8 = core::mem::transmute(vfn(dev, 3));
    let p = info(dev);
    if p.is_null() {
        return 0;
    }
    u32::from_le_bytes([*p.add(0x68), *p.add(0x69), *p.add(0x6a), *p.add(0x6b)])
}

unsafe fn hidden_reading(rd: *mut c_void) -> bool {
    if rd.is_null() || super::is_reading(rd) {
        return false;
    }
    let kind_f: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(vfn(rd, 3));
    let kind = kind_f(rd);
    if !hide_reading_kind(kind) {
        return false;
    }
    if user_pressed(rd, kind) {
        crate::host_input::take_over(&format!("host input reading kind={kind:#x} with a button pressed while the title window has focus"));
        return false;
    }
    if HIDDEN_DROPS.fetch_add(1, Ordering::Relaxed) == 0 {
        super::log("host input", &format!("first hidden host reading dropped (kind={kind:#x}; TOMBOX_HOST_PAD/TOMBOX_HOST_KBM)"));
    }
    true
}

#[link(name = "user32")]
extern "system" {
    fn GetForegroundWindow() -> *mut c_void;
    fn GetWindowThreadProcessId(h: *mut c_void, pid: *mut u32) -> u32;
}

fn title_focused() -> bool {
    let mut pid = 0u32;
    let h = unsafe { GetForegroundWindow() };
    !h.is_null() && unsafe { GetWindowThreadProcessId(h, &mut pid) } != 0 && pid == std::process::id()
}

unsafe fn user_pressed(rd: *mut c_void, kind: u32) -> bool {
    if !crate::host_input::test_run() || crate::host_input::taken_over() || !title_focused() {
        return false;
    }
    if kind & super::KIND_GAMEPAD != 0 {
        let pad_f: unsafe extern "system" fn(*mut c_void, *mut u8) -> u32 = core::mem::transmute(vfn(rd, 22));
        let mut st = [0u8; 28];
        if pad_f(rd, st.as_mut_ptr()) == 0 {
            return false;
        }
        let buttons = u32::from_le_bytes([st[0], st[1], st[2], st[3]]);
        let trig = |o: usize| f32::from_le_bytes([st[o], st[o + 1], st[o + 2], st[o + 3]]);
        return buttons != 0 || trig(4) > 0.5 || trig(8) > 0.5;
    }
    if kind & 0x10 != 0 {
        let count_f: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(vfn(rd, 14));
        return count_f(rd) != 0;
    }
    if kind & 0x20 != 0 {
        let mouse_f: unsafe extern "system" fn(*mut c_void, *mut u8) -> u32 = core::mem::transmute(vfn(rd, 16));
        let mut st = [0u8; 64];
        if mouse_f(rd, st.as_mut_ptr()) == 0 {
            return false;
        }
        return u32::from_le_bytes([st[0], st[1], st[2], st[3]]) & 0x1f != 0;
    }
    false
}

pub unsafe fn drop_host_pad(hr: i32, out: *mut *mut c_void) -> i32 {
    if hr < 0 || out.is_null() || !hidden_reading(*out) {
        return hr;
    }
    let rd = *out;
    let rel: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(vfn(rd, 2));
    rel(rd);
    *out = core::ptr::null_mut();
    super::READING_NOT_FOUND
}

type ReadingCb = unsafe extern "system" fn(u64, *mut c_void, *mut c_void, bool);

unsafe extern "system" fn reading_tramp(token: u64, ctx: *mut c_void, rd: *mut c_void, overrun: bool) {
    let t = &*(ctx as *const Tramp);
    if hidden_reading(rd) {
        return;
    }
    let f: ReadingCb = core::mem::transmute(t.cb);
    f(token, t.ctx as *mut c_void, rd, overrun);
}

pub fn wrap_reading_callback(ctx: *mut c_void, cb: usize) -> (*mut c_void, usize) {
    if cb == 0 || !(crate::host_input::pad_hidden() || crate::host_input::kbm_hidden()) {
        return (ctx, cb);
    }
    let t = Box::into_raw(Box::new(Tramp { cb, ctx: ctx as usize }));
    (t as *mut c_void, reading_tramp as *const () as usize)
}

pub fn count(slot: usize) {
    if slot < SLOTS && CALLS[slot].fetch_add(1, Ordering::Relaxed) == 0 {
        super::log("IGameInput", &format!("title called slot {slot} {} for the first time", SLOT_NAMES[slot]));
    }
}

unsafe fn vfn(obj: *mut c_void, i: usize) -> usize {
    *(*(obj as *const *const usize)).add(i)
}

unsafe fn device_desc(dev: *mut c_void) -> String {
    let info: unsafe extern "system" fn(*mut c_void) -> *const u8 = core::mem::transmute(vfn(dev, 3));
    let status: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(vfn(dev, 4));
    let p = info(dev);
    let st = status(dev);
    if p.is_null() {
        return format!("device {dev:p} (no info) status={st:#x}");
    }
    let vid = u16::from_le_bytes([*p.add(4), *p.add(5)]);
    let pid = u16::from_le_bytes([*p.add(6), *p.add(7)]);
    let kinds = u32::from_le_bytes([*p.add(0x68), *p.add(0x69), *p.add(0x6a), *p.add(0x6b)]);
    let id: String = (0..8).map(|i| format!("{:02x}", *p.add(0x20 + i))).collect();
    format!("device {dev:p} vid={vid:04x} pid={pid:04x} id={id}.. supportedInput={kinds:#x} status={st:#x}{}",
        if st & super::STATUS_INPUT_ENABLED == 0 { " (INPUT_ENABLED not set by host GameInput)" } else { "" })
}

pub unsafe fn note_device(dev: *mut c_void, why: &str) {
    if dev.is_null() || dev == super::vdev() {
        return;
    }
    let status: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(vfn(dev, 4));
    let st = status(dev);
    let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    match seen.iter_mut().find(|(d, _)| *d == dev as usize) {
        Some((_, s)) if *s == st => return,
        Some((_, s)) => *s = st,
        None => seen.push((dev as usize, st)),
    }
    drop(seen);
    super::log("host device", &format!("{why}: {}", device_desc(dev)));
}

pub unsafe fn note_reading(hr: i32, out: *mut *mut c_void) {
    if LAST_HR.swap(hr, Ordering::Relaxed) != hr && HR_LOGS.fetch_add(1, Ordering::Relaxed) < 64 {
        super::log("host reading", &format!("forwarded GetCurrentReading hr changed to {hr:#x}{}",
            if hr as u32 == 0x838A_0003 { " (no reading: no host device has reported input of that kind)" } else { "" }));
    }
    if hr < 0 || out.is_null() || (*out).is_null() || super::is_reading(*out) {
        return;
    }
    let rd = *out;
    let kind_f: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(vfn(rd, 3));
    let kind = kind_f(rd);
    super::mouse::patch_reading(rd, kind);
    let dev_f: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) = core::mem::transmute(vfn(rd, 6));
    let mut dev: *mut c_void = core::ptr::null_mut();
    dev_f(rd, &mut dev);
    if !dev.is_null() {
        patch_device(dev);
        let status: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(vfn(dev, 4));
        let st = status(dev);
        if LAST_DEV.swap(dev as usize, Ordering::Relaxed) != dev as usize || LAST_STATUS.swap(st, Ordering::Relaxed) != st {
            super::log("host reading", &format!("reading kind={kind:#x} from {}", device_desc(dev)));
        }
        let rel: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(vfn(dev, 2));
        rel(dev);
    }
    if kind & super::KIND_GAMEPAD == 0 {
        return;
    }
    let pad_f: unsafe extern "system" fn(*mut c_void, *mut u8) -> u32 = core::mem::transmute(vfn(rd, 22));
    let mut st = [0u8; 28];
    if pad_f(rd, st.as_mut_ptr()) == 0 {
        return;
    }
    let mut last = LAST_STATE.lock().unwrap_or_else(|e| e.into_inner());
    if last.as_ref() == Some(&st) {
        return;
    }
    let first = last.is_none();
    *last = Some(st);
    drop(last);
    if STATE_LOGS.fetch_add(1, Ordering::Relaxed) >= 400 {
        return;
    }
    let f = |i: usize| f32::from_le_bytes([st[4 + i * 4], st[5 + i * 4], st[6 + i * 4], st[7 + i * 4]]);
    super::log("host state", &format!("{} host gamepad state buttons={:#06x} lt={:.2} rt={:.2} lx={:.2} ly={:.2} rx={:.2} ry={:.2}",
        if first { "first" } else { "changed:" }, u32::from_le_bytes([st[0], st[1], st[2], st[3]]), f(0), f(1), f(2), f(3), f(4), f(5)));
}

#[repr(C)]
struct Tramp {
    cb: usize,
    ctx: usize,
}

unsafe extern "system" fn device_tramp(token: u64, ctx: *mut c_void, dev: *mut c_void, ts: u64, cur: u32, prev: u32) {
    let t = &*(ctx as *const Tramp);
    if !dev.is_null() && dev != super::vdev() && crate::host_input::hide_device(supported(dev), crate::host_input::pad_hidden(), crate::host_input::kbm_hidden()) {
        note_device(dev, &format!("device callback token={token:#x} status {prev:#x} -> {cur:#x} NOT passed to the title (TOMBOX_HOST_PAD/TOMBOX_HOST_KBM)"));
        if crate::host_input::test_run() {
            let mut held = HELD_DEVICES.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(h) = held.iter_mut().find(|h| h.ctx == ctx as usize && h.dev == dev as usize) {
                h.token = token;
                h.ts = ts;
                h.cur = cur;
                h.prev = prev;
            } else {
                let add: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(vfn(dev, 1));
                add(dev);
                held.push(Held { ctx: ctx as usize, dev: dev as usize, token, ts, cur, prev });
            }
            crate::host_input::on_takeover(replay_held_devices);
        }
        return;
    }
    patch_device(dev);
    let (cur, prev) = if dev.is_null() || dev == super::vdev() { (cur, prev) } else { (console_status(cur), console_status(prev)) };
    note_device(dev, &format!("device callback to title token={token:#x} status {prev:#x} -> {cur:#x}"));
    let f: super::DeviceCb = core::mem::transmute(t.cb);
    f(token, t.ctx as *mut c_void, dev, ts, cur, prev);
}

struct Held {
    ctx: usize,
    dev: usize,
    token: u64,
    ts: u64,
    cur: u32,
    prev: u32,
}

static HELD_DEVICES: Mutex<Vec<Held>> = Mutex::new(Vec::new());

fn replay_held_devices() {
    let held: Vec<Held> = core::mem::take(&mut *HELD_DEVICES.lock().unwrap_or_else(|e| e.into_inner()));
    super::log("host input", &format!("takeover: replaying {} held host device callback(s) to the title", held.len()));
    for h in held {
        unsafe {
            device_tramp(h.token, h.ctx as *mut c_void, h.dev as *mut c_void, h.ts, h.cur, h.prev);
            let rel: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(vfn(h.dev as *mut c_void, 2));
            rel(h.dev as *mut c_void);
        }
    }
}

pub fn wrap_device_callback(ctx: *mut c_void, cb: usize) -> (*mut c_void, usize) {
    if cb == 0 {
        return (ctx, cb);
    }
    let t = Box::into_raw(Box::new(Tramp { cb, ctx: ctx as usize }));
    (t as *mut c_void, device_tramp as *const () as usize)
}

extern "system" {
    fn VirtualProtect(addr: *mut c_void, size: usize, new_protect: u32, old_protect: *mut u32) -> i32;
}

static STATUS_ORIG: Mutex<Vec<(usize, usize)>> = Mutex::new(Vec::new());
static ENABLED_LOGS: AtomicU32 = AtomicU32::new(0);

fn status_orig_for(vt: usize) -> usize {
    STATUS_ORIG.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|(v, _)| *v == vt).map(|(_, f)| *f).unwrap_or(0)
}

pub fn console_status(st: u32) -> u32 {
    if st & super::STATUS_CONNECTED != 0 { st | super::STATUS_INPUT_ENABLED } else { st }
}

unsafe extern "system" fn device_status(this: *mut c_void) -> u32 {
    let f = status_orig_for(*(this as *const usize));
    if f == 0 {
        return 0;
    }
    let real: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(f);
    let st = real(this);
    let out = console_status(st);
    if out != st && ENABLED_LOGS.fetch_add(1, Ordering::Relaxed) == 0 {
        super::log("host device", &format!("GetDeviceStatus on host device {this:p}: {st:#x} -> {out:#x}; a console title only uses a connected pad that reports INPUT_ENABLED, and host GameInput delivers this pad's readings without setting it"));
    }
    out
}

pub unsafe fn patch_device(dev: *mut c_void) {
    if dev.is_null() || dev == super::vdev() {
        return;
    }
    let vt = *(dev as *const usize);
    let mut list = STATUS_ORIG.lock().unwrap_or_else(|e| e.into_inner());
    if list.iter().any(|(v, _)| *v == vt) {
        return;
    }
    let slot = (vt as *mut usize).add(4);
    let cur = *slot;
    if cur == device_status as *const () as usize {
        return;
    }
    let mut old = 0u32;
    if VirtualProtect(slot as *mut c_void, 8, 0x04, &mut old) == 0 {
        super::log("host device", &format!("VirtualProtect on host device vtable {vt:#x} failed; GetDeviceStatus left as is"));
        return;
    }
    list.push((vt, cur));
    core::ptr::write_volatile(slot, device_status as *const () as usize);
    let mut tmp = 0u32;
    VirtualProtect(slot as *mut c_void, 8, old, &mut tmp);
    drop(list);
    super::log("host device", &format!("host device vtable {vt:#x} ({}) GetDeviceStatus routed so connected pads report INPUT_ENABLED like on the console", module_of(cur)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connected_host_pads_report_input_enabled() {
        assert_eq!(console_status(0x1), 0x3);
        assert_eq!(console_status(0x3), 0x3);
        assert_eq!(console_status(0x0), 0x0);
        assert_eq!(console_status(0x5), 0x7);
    }
}
