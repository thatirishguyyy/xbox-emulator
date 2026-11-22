//! Xbox `ID3D12CommandQueue::PresentX` → host window + DXGI flip-model swap chain.
//!
//! Xbox has no swap chain: the title renders into its own back-buffer texture and hands it
//! to `PresentX(planeCount, pPlaneParameters, pPresentParameters)` (command-queue vtable
//! slot 30; call site `FD3D12Viewport::PresentInternal`, EXE+0x704a01). The plane struct,
//! as filled at that call site, is `{ u64 Token @0; u32 ResourceCount @8; ID3D12Resource**
//! ppResources @0x10; ... }`.
//!
//! Host implementation: on the first present, create a top-level window (own thread +
//! message pump) and a swap chain on the title's queue with the back buffer's size/format.
//! Each present copies the title's back buffer into the current swap-chain buffer with a
//! private command list, executes it on the queue, and calls `Present`.
//!
//! The PC queue vtable has 19 entries, so the queue object is repointed at an extended
//! copy (`apply_queue_vtable`) whose slot 30 is `present_x`; other Xbox-only slots are
//! logging no-ops until mapped from evidence.

use compat_common::record;
use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

#[repr(C)]
#[derive(Clone, Copy)]
struct Guid(u32, u16, u16, [u8; 8]);

const IID_DEVICE: Guid = Guid(0x189819f1, 0x1db6, 0x4b57, [0xbe, 0x54, 0x18, 0x21, 0x33, 0x9b, 0x85, 0xf7]);
const IID_ALLOCATOR: Guid = Guid(0x6102dee4, 0xaf59, 0x4b09, [0xb9, 0x99, 0xb4, 0x4d, 0x73, 0xf0, 0x9b, 0x24]);
const IID_GFX_LIST: Guid = Guid(0x5b160d0f, 0xac1b, 0x4185, [0x8b, 0xa8, 0xb3, 0xae, 0x42, 0xa5, 0xa4, 0x55]);
const IID_FENCE: Guid = Guid(0x0a753dcf, 0xc4d8, 0x4b91, [0xad, 0xf6, 0xbe, 0x5a, 0x60, 0xd9, 0x5a, 0x76]);
const IID_RESOURCE: Guid = Guid(0x696442be, 0xa72e, 0x4059, [0xbc, 0x79, 0x5b, 0x5c, 0x98, 0x04, 0x0f, 0xad]);
const IID_FACTORY2: Guid = Guid(0x50c83a1c, 0xe072, 0x4c48, [0x87, 0xb0, 0x36, 0x30, 0xfa, 0x36, 0xa6, 0xd0]);
const IID_SWAPCHAIN3: Guid = Guid(0x94d99bdb, 0xf1f8, 0x4ab0, [0xb2, 0x36, 0x7d, 0xa0, 0x17, 0x0e, 0xda, 0xb1]);
const IID_FACTORY5: Guid = Guid(0x7632e1f5, 0xee65, 0x4dca, [0x87, 0xfd, 0x84, 0xcd, 0x75, 0xf8, 0x83, 0x8d]);
const SWAP_CHAIN_FLAG_ALLOW_TEARING: u32 = 0x800;
const PRESENT_ALLOW_TEARING: u32 = 0x200;

extern "system" {
    fn LoadLibraryA(name: *const u8) -> *mut c_void;
    fn GetProcAddress(h: *mut c_void, name: *const u8) -> *mut c_void;
    fn GetModuleHandleW(n: *const u16) -> *mut c_void;
    fn VirtualProtect(addr: *mut c_void, size: usize, new_protect: u32, old_protect: *mut u32) -> i32;
    fn Sleep(ms: u32);
}

#[link(name = "user32")]
extern "system" {
    fn RegisterClassExW(wc: *const WndClassExW) -> u16;
    fn CreateWindowExW(
        ex: u32, class: *const u16, title: *const u16, style: u32, x: i32, y: i32, w: i32, h: i32,
        parent: *mut c_void, menu: *mut c_void, inst: *mut c_void, param: *mut c_void,
    ) -> *mut c_void;
    fn DefWindowProcW(h: *mut c_void, msg: u32, w: usize, l: isize) -> isize;
    fn GetMessageW(msg: *mut [u8; 48], h: *mut c_void, min: u32, max: u32) -> i32;
    fn TranslateMessage(msg: *const [u8; 48]) -> i32;
    fn DispatchMessageW(msg: *const [u8; 48]) -> isize;
    fn ShowWindow(h: *mut c_void, cmd: i32) -> i32;
    fn AdjustWindowRect(r: *mut [i32; 4], style: u32, menu: i32) -> i32;
    fn LoadCursorW(inst: *mut c_void, name: *const u16) -> *mut c_void;
    fn EnumWindows(cb: unsafe extern "system" fn(*mut c_void, isize) -> i32, l: isize) -> i32;
    fn GetWindowThreadProcessId(h: *mut c_void, pid: *mut u32) -> u32;
    fn IsWindowVisible(h: *mut c_void) -> i32;
    fn GetClientRect(h: *mut c_void, r: *mut [i32; 4]) -> i32;
    fn GetWindow(h: *mut c_void, cmd: u32) -> *mut c_void;
    fn GetClassNameW(h: *mut c_void, buf: *mut u16, n: i32) -> i32;
    fn GetWindowLongW(h: *mut c_void, idx: i32) -> i32;
    fn AdjustWindowRectEx(r: *mut [i32; 4], style: u32, menu: i32, ex: u32) -> i32;
    fn SetWindowPos(h: *mut c_void, after: *mut c_void, x: i32, y: i32, cx: i32, cy: i32, flags: u32) -> i32;
    fn GetSystemMetrics(i: i32) -> i32;
    fn GetWindowRect(h: *mut c_void, r: *mut [i32; 4]) -> i32;
}

pub(crate) fn start_window_clamp() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        std::thread::spawn(|| unsafe {
            for _ in 0..600 {
                let h = find_title_window();
                if h != 0 {
                    let mut r = [0i32; 4];
                    GetWindowRect(h as *mut c_void, &mut r);
                    let (sw, sh) = (GetSystemMetrics(0), GetSystemMetrics(1));
                    if r[0] < 0 || r[1] < 0 || r[2] > sw || r[3] > sh {
                        fit_window_to_aspect(h, 3840, 2160);
                        record("d3d12_xs", "window", &format!("clamped title window {r:?} to primary screen {sw}x{sh}"));
                    }
                }
                Sleep(100);
            }
        });
    });
}

/// Give the title's window a client area with the back buffer's aspect ratio (the swap
/// chain stretches the back buffer to the client area, so any other shape distorts the
/// image). Picks the largest standard size that fits on the primary screen.
unsafe fn fit_window_to_aspect(hwnd: usize, bb_w: u64, bb_h: u32) -> (i32, i32) {
    let (sw, sh) = (GetSystemMetrics(0), GetSystemMetrics(1));
    let mut cw = 1280i32;
    for w in [3840i32, 2560, 1920, 1600, 1280] {
        let h = (w as u64 * bb_h as u64 / bb_w) as i32;
        if w <= sw - 40 && h <= sh - 120 {
            cw = w;
            break;
        }
    }
    let ch = (cw as u64 * bb_h as u64 / bb_w) as i32;
    let (style, ex) = (GetWindowLongW(hwnd as *mut c_void, -16) as u32, GetWindowLongW(hwnd as *mut c_void, -20) as u32);
    let mut r = [0, 0, cw, ch];
    AdjustWindowRectEx(&mut r, style, 0, ex);
    // SWP_NOZORDER | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS: the window belongs to another
    // thread of the title; never block the presenting thread on it.
    SetWindowPos(hwnd as *mut c_void, core::ptr::null_mut(), 40, 40, r[2] - r[0], r[3] - r[1], 0x0004 | 0x0010 | 0x4000);
    (cw, ch)
}
extern "system" {
    fn GetCurrentProcessId() -> u32;
}

/// The title's own top-level window: it creates one itself
/// (`FXboxCommonGDKWindow::Initialize`, EXE+0x10d0dc3 `CreateWindowExW`), so presentation
/// goes there rather than into a second window. Picks the largest visible, unowned
/// top-level window of this process.
unsafe fn find_title_window() -> usize {
    unsafe extern "system" fn cb(h: *mut c_void, l: isize) -> i32 {
        let best = &mut *(l as *mut (usize, i64));
        let mut pid = 0u32;
        GetWindowThreadProcessId(h, &mut pid);
        if pid != GetCurrentProcessId() || IsWindowVisible(h) == 0 || !GetWindow(h, 4 /* GW_OWNER */).is_null() {
            return 1;
        }
        let mut r = [0i32; 4];
        GetClientRect(h, &mut r);
        let area = r[2] as i64 * r[3] as i64;
        if area > best.1 {
            *best = (h as usize, area);
        }
        1
    }
    let mut best: (usize, i64) = (0, 0);
    EnumWindows(cb, &mut best as *mut _ as isize);
    best.0
}

#[repr(C)]
struct WndClassExW {
    size: u32,
    style: u32,
    wndproc: unsafe extern "system" fn(*mut c_void, u32, usize, isize) -> isize,
    cls_extra: i32,
    wnd_extra: i32,
    instance: *mut c_void,
    icon: *mut c_void,
    cursor: *mut c_void,
    background: *mut c_void,
    menu_name: *const u16,
    class_name: *const u16,
    icon_sm: *mut c_void,
}

unsafe extern "system" fn wndproc(h: *mut c_void, msg: u32, w: usize, l: isize) -> isize {
    DefWindowProcW(h, msg, w, l)
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(core::iter::once(0)).collect()
}

/// Create the host window on its own thread (it needs a message pump the RHI thread
/// will never run). Returns the HWND.
fn create_window(client_w: i32, client_h: i32) -> usize {
    let (tx, rx) = std::sync::mpsc::channel::<usize>();
    std::thread::spawn(move || unsafe {
        let class = wide("XboxEmulatorHostWindow");
        let title = wide("Xbox Emulator");
        let inst = GetModuleHandleW(core::ptr::null());
        let wc = WndClassExW {
            size: core::mem::size_of::<WndClassExW>() as u32,
            style: 0,
            wndproc,
            cls_extra: 0,
            wnd_extra: 0,
            instance: inst,
            icon: core::ptr::null_mut(),
            cursor: LoadCursorW(core::ptr::null_mut(), 32512 as *const u16), // IDC_ARROW
            background: core::ptr::null_mut(),
            menu_name: core::ptr::null(),
            class_name: class.as_ptr(),
            icon_sm: core::ptr::null_mut(),
        };
        RegisterClassExW(&wc);
        const WS_OVERLAPPEDWINDOW: u32 = 0x00CF_0000;
        let mut r = [0i32, 0, client_w, client_h];
        AdjustWindowRect(&mut r, WS_OVERLAPPEDWINDOW, 0);
        let hwnd = CreateWindowExW(
            0, class.as_ptr(), title.as_ptr(), WS_OVERLAPPEDWINDOW, 80, 80, r[2] - r[0], r[3] - r[1],
            core::ptr::null_mut(), core::ptr::null_mut(), inst, core::ptr::null_mut(),
        );
        if !hwnd.is_null() {
            ShowWindow(hwnd, 5); // SW_SHOW
        }
        let _ = tx.send(hwnd as usize);
        let mut msg = [0u8; 48];
        while GetMessageW(&mut msg, core::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    });
    rx.recv().unwrap_or(0)
}

unsafe fn vt(obj: *mut c_void) -> *const usize {
    *(obj as *const *const usize)
}
macro_rules! vcall {
    ($obj:expr, $slot:expr, fn($($at:ty),*) -> $rt:ty $(, $arg:expr)*) => {{
        let f: unsafe extern "system" fn(*mut c_void $(, $at)*) -> $rt =
            core::mem::transmute(*vt($obj).add($slot));
        f($obj $(, $arg)*)
    }};
}

struct State {
    swapchain: *mut c_void, // IDXGISwapChain3
    alloc: *mut c_void,
    list: *mut c_void, // private, PC-layout command list
    fence: *mut c_void,
    fence_value: u64,
    width: u64,
    height: u32,
    format: u32,
    frames: u64,
    device: *mut c_void,
    hwnd: usize,
    sc_format: u32,
}
unsafe impl Send for State {}
static STATE: Mutex<Option<State>> = Mutex::new(None);
static FAILED: AtomicBool = AtomicBool::new(false);
static TEARING: AtomicBool = AtomicBool::new(false);

fn present_flags() -> u32 {
    if TEARING.load(Ordering::Relaxed) && crate::frame_fast::uncapped() { PRESENT_ALLOW_TEARING } else { 0 }
}

unsafe fn tearing_supported(factory: *mut c_void) -> bool {
    let mut f5: *mut c_void = core::ptr::null_mut();
    if vcall!(factory, 0, fn(*const Guid, *mut *mut c_void) -> i32, &IID_FACTORY5, &mut f5) < 0 || f5.is_null() {
        return false;
    }
    let mut allow: i32 = 0;
    let hr = vcall!(f5, 28, fn(u32, *mut c_void, u32) -> i32, 0, &mut allow as *mut i32 as *mut c_void, 4);
    vcall!(f5, 2, fn() -> u32);
    hr >= 0 && allow != 0
}

/// Flip-model swap chains accept only a few formats; map the title's back-buffer format.
fn swapchain_format(fmt: u32) -> Option<u32> {
    Some(match fmt {
        23 | 24 => 24,      // R10G10B10A2 typeless/unorm
        27 | 28 | 29 => 28, // R8G8B8A8 typeless/unorm/srgb -> unorm
        90 | 87 | 91 => 87, // B8G8R8A8 typeless/unorm/srgb -> unorm
        9 | 10 => 10,       // R16G16B16A16 typeless/float
        _ => return None,
    })
}

unsafe fn init(queue: *mut c_void, src: *mut c_void) -> Result<State, String> {
    // ID3D12Resource::GetDesc (slot 10) returns the struct via hidden out pointer.
    let mut desc = [0u8; 0x38];
    vcall!(src, 10, fn(*mut u8) -> *mut u8, desc.as_mut_ptr());
    let dim = u32::from_le_bytes(desc[0..4].try_into().unwrap());
    let width = u64::from_le_bytes(desc[0x10..0x18].try_into().unwrap());
    let height = u32::from_le_bytes(desc[0x18..0x1c].try_into().unwrap());
    let format = u32::from_le_bytes(desc[0x20..0x24].try_into().unwrap());
    if dim != 3 || width == 0 || height == 0 || width > 16384 || height > 16384 {
        return Err(format!("back buffer is not a sane 2D texture (dim={dim} {width}x{height} fmt={format})"));
    }
    let sc_format = swapchain_format(format).ok_or_else(|| format!("back-buffer format {format} unsupported for flip model"))?;

    let mut device: *mut c_void = core::ptr::null_mut();
    let hr = vcall!(queue, 7, fn(*const Guid, *mut *mut c_void) -> i32, &IID_DEVICE, &mut device);
    if hr < 0 || device.is_null() {
        return Err(format!("queue->GetDevice hr={hr:#010x}"));
    }

    // Present into the title's own window; the swap chain stretches the back buffer to the
    // client area. Only if the title has no window (or TOMBOX_OWN_WINDOW=1) open our own,
    // scaled to fit a 1280-wide client area.
    let mut cw = 1280i32;
    let mut ch = ((height as u64 * 1280) / width).max(1) as i32;
    let own = std::env::var("TOMBOX_OWN_WINDOW").map(|v| v == "1").unwrap_or(false);
    let mut hwnd = if own { 0 } else { find_title_window() };
    if hwnd != 0 {
        (cw, ch) = fit_window_to_aspect(hwnd, width, height);
        let mut cls = [0u16; 128];
        let n = GetClassNameW(hwnd as *mut c_void, cls.as_mut_ptr(), 128).max(0) as usize;
        record("d3d12_xs", "PresentX", &format!(
            "presenting into the title's own window hwnd={hwnd:#x} class='{}' client {cw}x{ch}", String::from_utf16_lossy(&cls[..n])));
    } else {
        hwnd = create_window(cw, ch);
        if hwnd == 0 {
            return Err("CreateWindowExW failed".into());
        }
        record("d3d12_xs", "PresentX", "title window not found (or TOMBOX_OWN_WINDOW=1): presenting into a TomBox host window");
    }

    let dxgi = LoadLibraryA(b"dxgi.dll\0".as_ptr());
    let create = GetProcAddress(dxgi, b"CreateDXGIFactory2\0".as_ptr());
    if create.is_null() {
        return Err("CreateDXGIFactory2 unavailable".into());
    }
    let create: unsafe extern "system" fn(u32, *const Guid, *mut *mut c_void) -> i32 = core::mem::transmute(create);
    let mut factory: *mut c_void = core::ptr::null_mut();
    let hr = create(0, &IID_FACTORY2, &mut factory);
    if hr < 0 {
        return Err(format!("CreateDXGIFactory2 hr={hr:#010x}"));
    }
    // DXGI_SWAP_CHAIN_DESC1
    let mut scd = [0u32; 12];
    scd[0] = width as u32;
    scd[1] = height;
    scd[2] = sc_format;
    scd[4] = 1; // SampleDesc.Count
    scd[6] = 0x20; // DXGI_USAGE_RENDER_TARGET_OUTPUT
    scd[7] = 2; // BufferCount
    scd[8] = 0; // DXGI_SCALING_STRETCH
    scd[9] = 4; // DXGI_SWAP_EFFECT_FLIP_DISCARD
    let tearing = tearing_supported(factory);
    if tearing {
        scd[11] = SWAP_CHAIN_FLAG_ALLOW_TEARING;
    }
    TEARING.store(tearing, Ordering::Relaxed);
    let mut sc1: *mut c_void = core::ptr::null_mut();
    let hr = vcall!(factory, 15,
        fn(*mut c_void, usize, *const u32, *const c_void, *mut c_void, *mut *mut c_void) -> i32,
        queue, hwnd, scd.as_ptr(), core::ptr::null(), core::ptr::null_mut(), &mut sc1);
    if hr < 0 || sc1.is_null() {
        return Err(format!("CreateSwapChainForHwnd hr={hr:#010x} ({width}x{height} fmt={sc_format})"));
    }
    let mut sc3: *mut c_void = core::ptr::null_mut();
    let hr = vcall!(sc1, 0, fn(*const Guid, *mut *mut c_void) -> i32, &IID_SWAPCHAIN3, &mut sc3);
    if hr < 0 || sc3.is_null() {
        return Err(format!("QI IDXGISwapChain3 hr={hr:#010x}"));
    }

    let mut alloc: *mut c_void = core::ptr::null_mut();
    let hr = vcall!(device, 9, fn(u32, *const Guid, *mut *mut c_void) -> i32, 0, &IID_ALLOCATOR, &mut alloc);
    if hr < 0 {
        return Err(format!("CreateCommandAllocator hr={hr:#010x}"));
    }
    let mut list: *mut c_void = core::ptr::null_mut();
    let hr = vcall!(device, 12,
        fn(u32, u32, *mut c_void, *mut c_void, *const Guid, *mut *mut c_void) -> i32,
        0, 0, alloc, core::ptr::null_mut(), &IID_GFX_LIST, &mut list);
    if hr < 0 {
        return Err(format!("CreateCommandList hr={hr:#010x}"));
    }
    vcall!(list, 9, fn() -> i32); // Close (created open)
    let mut fence: *mut c_void = core::ptr::null_mut();
    let hr = vcall!(device, 36, fn(u64, u32, *const Guid, *mut *mut c_void) -> i32, 0, 0, &IID_FENCE, &mut fence);
    if hr < 0 {
        return Err(format!("CreateFence hr={hr:#010x}"));
    }
    record("d3d12_xs", "PresentX", &format!(
        "host swap chain created: {width}x{height} titleFmt={format} scFmt={sc_format} window {cw}x{ch} hwnd={hwnd:#x}; present SyncInterval 0, tearing {}", if tearing { "allowed (no vsync wait while frame_hz is uncapped)" } else { "not supported by this display path, flips may still wait for DWM" }));
    std::env::set_var("TOMBOX_PRESENT_TARGET", format!("{hwnd:x},{width},{height}"));
    Ok(State { swapchain: sc3, alloc, list, fence, fence_value: 0, width, height, format, frames: 0, device, hwnd, sc_format })
}

fn barrier(res: *mut c_void, before: u32, after: u32) -> [u64; 4] {
    // D3D12_RESOURCE_BARRIER { Type=TRANSITION(0), Flags=0, pResource, Subresource=ALL, Before, After }
    [0, res as u64, 0xffff_ffffu64 | ((before as u64) << 32), after as u64]
}

unsafe fn snap_buffers(queue: *mut c_void, st: &State) -> Option<(*mut c_void, *mut c_void)> {
    let mut device: *mut c_void = core::ptr::null_mut();
    if vcall!(queue, 7, fn(*const Guid, *mut *mut c_void) -> i32, &IID_DEVICE, &mut device) < 0 || device.is_null() {
        return None;
    }
    let size = crate::present_snap::row_pitch(st.width as u32) as u64 * st.height as u64;
    let heap: [u32; 5] = [3, 0, 0, 1, 1];
    let mut bd = [0u8; 0x38];
    bd[0..4].copy_from_slice(&1u32.to_le_bytes());
    bd[0x10..0x18].copy_from_slice(&size.to_le_bytes());
    bd[0x18..0x1c].copy_from_slice(&1u32.to_le_bytes());
    bd[0x1c..0x1e].copy_from_slice(&1u16.to_le_bytes());
    bd[0x1e..0x20].copy_from_slice(&1u16.to_le_bytes());
    bd[0x24..0x28].copy_from_slice(&1u32.to_le_bytes());
    bd[0x2c..0x30].copy_from_slice(&1u32.to_le_bytes());
    let mut out = [core::ptr::null_mut::<c_void>(); 2];
    for o in out.iter_mut() {
        let hr = vcall!(device, 27, fn(*const u32, u32, *const u8, u32, *const c_void, *const Guid, *mut *mut c_void) -> i32,
            heap.as_ptr(), 0, bd.as_ptr(), 0x400, core::ptr::null(), &IID_RESOURCE, o);
        if hr < 0 || o.is_null() {
            record("d3d12_xs", "PresentX.snap", &format!("readback buffer {size:#x} hr={hr:#010x}"));
        }
    }
    vcall!(device, 2, fn() -> u32);
    if out.iter().any(|p| p.is_null()) {
        for p in out.into_iter().filter(|p| !p.is_null()) {
            vcall!(p, 2, fn() -> u32);
        }
        return None;
    }
    Some((out[0], out[1]))
}

unsafe fn present_frame(st: &mut State, queue: *mut c_void, src: *mut c_void) -> Result<(), String> {
    const COMMON: u32 = 0; // == PRESENT
    const COPY_DEST: u32 = 0x400;
    const COPY_SOURCE: u32 = 0x800;
    const RENDER_TARGET: u32 = 0x4;
    // Wait for the previous copy before reusing the allocator.
    let wait_t0 = crate::frame::qpc();
    let waited = crate::perf::timed(crate::perf::PWAIT, || crate::present_wait::wait_fence(st.fence, st.fence_value, 5000));
    crate::frame_csv::add_wait(crate::frame::qpc() - wait_t0);
    if !waited {
        return Err("timed out waiting for previous present copy".into());
    }
    crate::present_probe::collect();
    crate::overlay::tick(st.hwnd);
    let idx = vcall!(st.swapchain, 36, fn() -> u32);
    let mut dst: *mut c_void = core::ptr::null_mut();
    let hr = vcall!(st.swapchain, 9, fn(u32, *const Guid, *mut *mut c_void) -> i32, idx, &IID_RESOURCE, &mut dst);
    if hr < 0 || dst.is_null() {
        return Err(format!("swapchain GetBuffer({idx}) hr={hr:#010x}"));
    }
    vcall!(st.alloc, 8, fn() -> i32); // allocator Reset
    vcall!(st.list, 10, fn(*mut c_void, *mut c_void) -> i32, st.alloc, core::ptr::null_mut());
    crate::frame_csv::list_begin(st.device, st.list, st.frames + 1, ts_freq(queue));
    let pre = [barrier(src, COMMON, COPY_SOURCE), barrier(dst, COMMON, COPY_DEST)];
    vcall!(st.list, 26, fn(u32, *const [u64; 4]) -> (), 2, pre.as_ptr());
    vcall!(st.list, 17, fn(*mut c_void, *mut c_void) -> (), dst, src); // CopyResource
    let dst_state = if crate::overlay::draw(st.device, queue, st.list, dst, st.sc_format, st.width, st.height) { RENDER_TARGET } else { COPY_DEST };
    let snap_frame = st.frames + 1;
    let snaps = if crate::present_snap::wanted(snap_frame) { snap_buffers(queue, st) } else { None };
    if snaps.is_none() {
        crate::present_probe::record_copy(queue, st.list, src, st.format, st.width as u32, st.height, snap_frame);
    }
    let dst_after = if let Some((rb_src, rb_dst)) = snaps {
        let mid = [barrier(dst, dst_state, COPY_SOURCE)];
        vcall!(st.list, 26, fn(u32, *const [u64; 4]) -> (), 1, mid.as_ptr());
        for (rb, tex) in [(rb_src, src), (rb_dst, dst)] {
            let d = crate::present_snap::footprint_location(rb, st.format, st.width as u32, st.height);
            let s = crate::present_snap::subresource_location(tex);
            vcall!(st.list, 16, fn(*const u8, u32, u32, u32, *const u8, *const c_void) -> (), d.as_ptr(), 0, 0, 0, s.as_ptr(), core::ptr::null());
        }
        COPY_SOURCE
    } else {
        dst_state
    };
    let post = [barrier(src, COPY_SOURCE, COMMON), barrier(dst, dst_after, COMMON)];
    vcall!(st.list, 26, fn(u32, *const [u64; 4]) -> (), 2, post.as_ptr());
    crate::frame_csv::list_end(st.list, st.frames + 1);
    let hr = vcall!(st.list, 9, fn() -> i32);
    if let Some((rb_src, rb_dst)) = snaps {
        if hr >= 0 {
            let lists = [st.list];
            vcall!(queue, 10, fn(u32, *const *mut c_void) -> (), 1, lists.as_ptr());
            st.fence_value += 1;
            vcall!(queue, 14, fn(*mut c_void, u64) -> i32, st.fence, st.fence_value);
            if crate::present_wait::wait_fence(st.fence, st.fence_value, 5000) {
                let state = crate::gpu_va::state(src as usize);
                for (tag, rb) in [("title_backbuffer", rb_src), ("presented_swapchain", rb_dst)] {
                    let len = crate::present_snap::row_pitch(st.width as u32) as usize * st.height as usize;
                    let mut p: *mut u8 = core::ptr::null_mut();
                    if vcall!(rb, 8, fn(u32, *const c_void, *mut *mut u8) -> i32, 0, core::ptr::null(), &mut p) >= 0 && !p.is_null() {
                        let data = core::slice::from_raw_parts(p, len);
                        let extra = format!(" | src={src:p} trackedState={state:#x} titleFmt={}", st.format);
                        crate::present_snap::finish(tag, snap_frame, data, st.width as u32, st.height, st.format, &extra);
                        vcall!(rb, 9, fn(u32, *const c_void) -> (), 0, core::ptr::null());
                    }
                }
            }
        }
        vcall!(rb_src, 2, fn() -> u32);
        vcall!(rb_dst, 2, fn() -> u32);
        vcall!(dst, 2, fn() -> u32);
        if hr < 0 {
            return Err(format!("present list Close hr={hr:#010x}"));
        }
        let hr = vcall!(st.swapchain, 8, fn(u32, u32) -> i32, 0, present_flags());
        if hr < 0 {
            return Err(format!("swapchain Present hr={hr:#010x}"));
        }
        st.frames += 1;
        return Ok(());
    }
    vcall!(dst, 2, fn() -> u32); // Release our GetBuffer ref
    if hr < 0 {
        return Err(format!("present list Close hr={hr:#010x}"));
    }
    let lists = [st.list];
    vcall!(queue, 10, fn(u32, *const *mut c_void) -> (), 1, lists.as_ptr());
    st.fence_value += 1;
    vcall!(queue, 14, fn(*mut c_void, u64) -> i32, st.fence, st.fence_value);
    let hr = crate::perf::timed(crate::perf::PFLIP, || vcall!(st.swapchain, 8, fn(u32, u32) -> i32, 0, present_flags()));
    if hr < 0 {
        return Err(format!("swapchain Present hr={hr:#010x}"));
    }
    st.frames += 1;
    Ok(())
}

unsafe fn ts_freq(queue: *mut c_void) -> u64 {
    static F: AtomicUsize = AtomicUsize::new(0);
    let v = F.load(Ordering::Relaxed);
    if v != 0 {
        return v as u64;
    }
    let f = REAL_QVT[16].load(Ordering::Relaxed);
    if f == 0 {
        return 0;
    }
    let f: unsafe extern "system" fn(*mut c_void, *mut u64) -> i32 = core::mem::transmute(f);
    let mut hz = 0u64;
    if f(queue, &mut hz) >= 0 && hz != 0 {
        F.store(hz as usize, Ordering::Relaxed);
    }
    hz
}

/// Xbox `PresentX` (queue slot 30).
unsafe extern "system" fn present_x(queue: *mut c_void, plane_count: u32, planes: *const u8, _params: *const u8) -> i32 {
    static CALLS: AtomicUsize = AtomicUsize::new(0);
    let n = CALLS.fetch_add(1, Ordering::Relaxed);
    if plane_count != 0 && !planes.is_null() {
        // The frame identified by this token counts as flipped from here on (the title's
        // flip tracking keys off it), even if the host-side present below fails.
        crate::frame::mark_flipped(core::ptr::read_unaligned(planes as *const u64));
    }
    if FAILED.load(Ordering::Relaxed) || plane_count == 0 || planes.is_null() {
        crate::frame::on_present();
        return 0;
    }
    let res_count = core::ptr::read_unaligned(planes.add(8) as *const u32);
    let pp = core::ptr::read_unaligned(planes.add(0x10) as *const *const *mut c_void);
    if res_count == 0 || pp.is_null() || (*pp).is_null() {
        if n < 4 {
            record("d3d12_xs", "PresentX", &format!("no resource in plane 0 (count={res_count}) — nothing presented"));
        }
        return 0;
    }
    let src = *pp;
    let entry = crate::frame::qpc();
    crate::perf::on_present();
    crate::qwait::poll();
    crate::gpuprof::on_present();
    crate::hot_reload::on_present();
    crate::present_probe::on_present(src, plane_count, planes);
    crate::slowcall::time("PresentX upload pump", || crate::perf::timed(crate::perf::UPLOAD, || crate::upload::pump(queue)));
    crate::slowcall::time("PresentX replay/rtcap", || crate::perf::timed(crate::perf::RTCAP, || {
        crate::replay::on_present(queue);
        crate::rtcap::maybe_capture(queue, src)
    }));
    let mut guard = crate::slowcall::time("PresentX state lock", || STATE.lock().unwrap());
    if guard.is_none() {
        match init(queue, src) {
            Ok(s) => *guard = Some(s),
            Err(e) => {
                FAILED.store(true, Ordering::Relaxed);
                record("d3d12_xs", "PresentX", &format!("host present init FAILED: {e}"));
                return 0;
            }
        }
    }
    let st = guard.as_mut().unwrap();
    let presented = crate::slowcall::time("PresentX host present", || crate::perf::timed(crate::perf::COPY, || present_frame(st, queue, src)));
    crate::frame::on_present();
    match presented {
        Ok(()) => {
            crate::alias::tick();
            if st.frames == 2 || st.frames % 300 == 0 {
                crate::timestamp::check_clock(queue, REAL_QVT[16].load(Ordering::Relaxed), REAL_QVT[17].load(Ordering::Relaxed));
            }
            if st.frames <= 3 || st.frames % 300 == 0 {
                record("d3d12_xs", "PresentX", &format!(
                    "frame {} presented ({}x{} fmt={})", st.frames, st.width, st.height, st.format));
            }
            if crate::frame_csv::on() {
                crate::frame_csv::present_done(st.frames, entry);
            }
        }
        Err(e) => {
            if n < 8 {
                record("d3d12_xs", "PresentX", &format!("present FAILED: {e}"));
            }
        }
    }
    0
}

// ---- extended queue vtable ------------------------------------------------------------
//
// The real PC queue vtable continues past the 19 public methods with private entries that
// D3D12/DXGI call internally (observed: slots 19..24), while the title uses the same slot
// numbers with Xbox meanings (23, and 30 = PresentX). Each extended slot therefore
// dispatches on the caller: return address inside the title EXE → Xbox handler, anything
// else → the real PC entry.

const QN: usize = 64;
const PC_QUEUE_SLOTS: usize = 19;
static QUEUE_VT: [AtomicUsize; QN] = [const { AtomicUsize::new(0) }; QN];
static REAL_QVT: [AtomicUsize; QN] = [const { AtomicUsize::new(0) }; QN];
static XBOX_QVT: [AtomicUsize; QN] = [const { AtomicUsize::new(0) }; QN];
static QUEUE_BUILT: AtomicBool = AtomicBool::new(false);
static EXE_BASE: AtomicUsize = AtomicUsize::new(0);
static EXE_SIZE: AtomicUsize = AtomicUsize::new(0);

unsafe extern "system" fn qprobe_impl(_this: *mut c_void, a: u64, caller: usize, slot: u64) -> i32 {
    static PER: [AtomicUsize; QN] = [const { AtomicUsize::new(0) }; QN];
    if PER[(slot as usize).min(QN - 1)].fetch_add(1, Ordering::Relaxed) < 3 {
        let base = EXE_BASE.load(Ordering::Relaxed);
        record("d3d12_xs", "queue.ext", &format!("slot={slot} arg1={a:#x} caller=EXE+{:#x} -> S_OK (no-op)", caller.wrapping_sub(base)));
    }
    0
}

macro_rules! qthunks {
    ($(($name:ident, $probe:ident, $slot:literal)),* $(,)?) => {
        $(
            /// Xbox-side default for this slot: log + no-op.
            #[unsafe(naked)]
            unsafe extern "system" fn $probe() {
                core::arch::naked_asm!(
                    "mov r8, [rsp]",
                    "mov r9d, {s}",
                    "jmp {i}",
                    s = const $slot, i = sym qprobe_impl,
                );
            }
            /// Caller-dispatching entry placed in the queue vtable.
            #[unsafe(naked)]
            unsafe extern "system" fn $name() {
                core::arch::naked_asm!(
                    "mov rax, [rsp]",
                    "sub rax, [rip + {base}]",
                    "cmp rax, [rip + {size}]",
                    "jb 2f",
                    "lea rax, [rip + {real}]",
                    "jmp qword ptr [rax + {off}]",
                    "2:",
                    "lea rax, [rip + {xbox}]",
                    "jmp qword ptr [rax + {off}]",
                    base = sym EXE_BASE, size = sym EXE_SIZE, real = sym REAL_QVT, xbox = sym XBOX_QVT,
                    off = const $slot * 8,
                );
            }
        )*
        unsafe fn install_qthunks() {
            $(
                QUEUE_VT[$slot].store($name as usize, Ordering::Relaxed);
                XBOX_QVT[$slot].store($probe as usize, Ordering::Relaxed);
            )*
        }
    };
}
qthunks!(
    (q19,x19,19),(q20,x20,20),(q21,x21,21),(q22,x22,22),(q23,x23,23),(q24,x24,24),(q25,x25,25),(q26,x26,26),
    (q27,x27,27),(q28,x28,28),(q29,x29,29),(q30,x30,30),(q31,x31,31),(q32,x32,32),(q33,x33,33),(q34,x34,34),
    (q35,x35,35),(q36,x36,36),(q37,x37,37),(q38,x38,38),(q39,x39,39),(q40,x40,40),(q41,x41,41),(q42,x42,42),
    (q43,x43,43),(q44,x44,44),(q45,x45,45),(q46,x46,46),(q47,x47,47),(q48,x48,48),(q49,x49,49),(q50,x50,50),
    (q51,x51,51),(q52,x52,52),(q53,x53,53),(q54,x54,54),(q55,x55,55),(q56,x56,56),(q57,x57,57),(q58,x58,58),
    (q59,x59,59),(q60,x60,60),(q61,x61,61),(q62,x62,62),(q63,x63,63),
);

/// Xbox queue slot 23 = Signal(pFence, Value, Flags) — called from FD3D12Fence::
/// InternalSignal (EXE+0x7061d3). PC `Signal` is slot 14 and takes (pFence, Value).
unsafe extern "system" fn signal_x(queue: *mut c_void, fence: *mut c_void, value: u64, _flags: u32) -> i32 {
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void, u64) -> i32 =
        core::mem::transmute(REAL_QVT[14].load(Ordering::Relaxed));
    let hr = crate::slowcall::time("queue SignalX", || f(queue, fence, value));
    static N: AtomicUsize = AtomicUsize::new(0);
    if N.fetch_add(1, Ordering::Relaxed) < 3 {
        record("d3d12_xs", "queue.SignalX", &format!("fence={fence:p} value={value} -> hr={hr:#010x}"));
    }
    hr
}

/// Diagnostic for title calls on the PC-range queue slots (8..18): logs the first few per
/// slot so we can verify the Xbox queue really uses PC numbering there.
unsafe extern "system" fn qlow_log(slot: u64, caller: usize) {
    static PER: [AtomicUsize; 32] = [const { AtomicUsize::new(0) }; 32];
    if PER[(slot as usize).min(31)].fetch_add(1, Ordering::Relaxed) < 2 {
        let base = EXE_BASE.load(Ordering::Relaxed);
        record("d3d12_xs", "queue.pc", &format!("title called PC-range queue slot {slot} from EXE+{:#x}", caller.wrapping_sub(base)));
    }
}

macro_rules! qlow {
    ($(($name:ident, $slot:literal)),* $(,)?) => {
        $(
            #[unsafe(naked)]
            unsafe extern "system" fn $name() {
                core::arch::naked_asm!(
                    "mov rax, [rsp]",
                    "sub rax, [rip + {base}]",
                    "cmp rax, [rip + {size}]",
                    "jae 2f",
                    // title caller: log (preserving argument registers), then forward
                    "push rcx", "push rdx", "push r8", "push r9",
                    "sub rsp, 0x28",
                    "mov ecx, {s}",
                    "mov rdx, [rsp + 0x48]",
                    "call {log}",
                    "add rsp, 0x28",
                    "pop r9", "pop r8", "pop rdx", "pop rcx",
                    "2:",
                    "lea rax, [rip + {real}]",
                    "jmp qword ptr [rax + {off}]",
                    base = sym EXE_BASE, size = sym EXE_SIZE, real = sym REAL_QVT, log = sym qlow_log,
                    s = const $slot, off = const $slot * 8,
                );
            }
        )*
        unsafe fn install_qlow() {
            $( QUEUE_VT[$slot].store($name as usize, Ordering::Relaxed); )*
        }
    };
}
qlow!((l8,8),(l9,9),(l10,10),(l11,11),(l12,12),(l13,13),(l14,14),(l15,15),(l16,16),(l17,17),(l18,18));

/// Title-side ExecuteCommandLists (PC slot 10): the title passes command-list *shims*
/// (see cmdlist_wrap); translate them to the real lists before submitting.
unsafe extern "system" fn exec_lists_x(queue: *mut c_void, n: u32, lists: *const *mut c_void) {
    let f: unsafe extern "system" fn(*mut c_void, u32, *const *mut c_void) =
        core::mem::transmute(REAL_QVT[10].load(Ordering::Relaxed));
    if lists.is_null() || n == 0 {
        return f(queue, n, lists);
    }
    crate::qwait::poll();
    let title: Vec<*mut c_void> = (0..n as usize).map(|i| *lists.add(i)).collect();
    let before = crate::ecl_probe::removed_reason(queue);
    let real: Vec<*mut c_void> =
        (0..n as usize).map(|i| crate::cmdlist_wrap::unwrap(*lists.add(i))).collect();
    let mut real = crate::cl2_state::skip_presubmitted(crate::list_split::expand(&real));
    crate::indirect_bundle::resolve(&mut real);
    let real = crate::ecl_probe::refuse_failed(real);
    if real.is_empty() {
        return;
    }
    crate::sys_ts::note_queue(queue, REAL_QVT[16].load(Ordering::Relaxed), REAL_QVT[17].load(Ordering::Relaxed));
    let split = crate::cl2_state::before_execute(queue, &real);
    crate::title_heap::sync_for_execute(&real);
    crate::title_shadow::before_execute();
    crate::slowcall::time("queue ExecuteCommandLists replay hook", || crate::replay::on_execute(queue, &real));
    crate::slowcall::time("queue ExecuteCommandLists", || crate::gpu_wait::submit(queue, &real, f, REAL_QVT[14].load(Ordering::Relaxed), REAL_QVT[15].load(Ordering::Relaxed)));
    crate::ecl_probe::check("ExecuteCommandLists", queue, REAL_QVT[18].load(Ordering::Relaxed), &title, &real, before);
    crate::title_shadow::after_execute(queue, REAL_QVT[14].load(Ordering::Relaxed), REAL_QVT[15].load(Ordering::Relaxed));
    crate::list_split::after_submit(queue, &real);
    crate::cl2_state::note_executed(queue, &real);
    crate::cl2_state::after_execute(queue, &split);
    crate::ts99::after_execute(queue, &real);
}

unsafe extern "system" fn queue_freq(queue: *mut c_void, out: *mut u64) -> i32 {
    let f: crate::timestamp::FreqFn = core::mem::transmute(REAL_QVT[16].load(Ordering::Relaxed));
    let mut host = 0u64;
    let hr = f(queue, &mut host);
    crate::timestamp::on_frequency(queue, &mut host, hr);
    crate::ts99::title_frequency(out)
}

unsafe extern "system" fn queue_calib(queue: *mut c_void, gpu: *mut u64, cpu: *mut u64) -> i32 {
    let hr = crate::ts99::title_calibration(queue, gpu, cpu);
    crate::timestamp::on_calibration(queue, gpu, cpu, hr);
    hr
}

pub fn real_queue_slot(i: usize) -> usize {
    REAL_QVT.get(i).map_or(0, |v| v.load(Ordering::Relaxed))
}

#[unsafe(naked)]
unsafe extern "system" fn q_freq_dispatch() {
    core::arch::naked_asm!(
        "mov rax, [rsp]",
        "sub rax, [rip + {base}]",
        "cmp rax, [rip + {size}]",
        "jb 2f",
        "lea rax, [rip + {real}]",
        "jmp qword ptr [rax + 128]",
        "2:",
        "jmp {x}",
        base = sym EXE_BASE, size = sym EXE_SIZE, real = sym REAL_QVT, x = sym queue_freq,
    );
}

#[unsafe(naked)]
unsafe extern "system" fn q_calib_dispatch() {
    core::arch::naked_asm!(
        "mov rax, [rsp]",
        "sub rax, [rip + {base}]",
        "cmp rax, [rip + {size}]",
        "jb 2f",
        "lea rax, [rip + {real}]",
        "jmp qword ptr [rax + 136]",
        "2:",
        "jmp {x}",
        base = sym EXE_BASE, size = sym EXE_SIZE, real = sym REAL_QVT, x = sym queue_calib,
    );
}

/// Vtable entry for slot 10: title callers get `exec_lists_x`, everyone else the real method.
#[unsafe(naked)]
unsafe extern "system" fn q_exec_dispatch() {
    core::arch::naked_asm!(
        "mov rax, [rsp]",
        "sub rax, [rip + {base}]",
        "cmp rax, [rip + {size}]",
        "jb 2f",
        "lea rax, [rip + {real}]",
        "jmp qword ptr [rax + 80]",
        "2:",
        "jmp {x}",
        base = sym EXE_BASE, size = sym EXE_SIZE, real = sym REAL_QVT, x = sym exec_lists_x,
    );
}

/// Repoint a freshly created real command queue at the extended (Xbox-sized) vtable.
pub fn apply_queue_vtable(queue: *mut c_void) {
    if queue.is_null() {
        return;
    }
    unsafe {
        let real_vt = *(queue as *const *const usize);
        if !QUEUE_BUILT.swap(true, Ordering::SeqCst) {
            for i in 0..QN {
                // Entries past the real table's end are only reachable by non-title callers,
                // which would have read the same memory through the original vtable anyway.
                REAL_QVT[i].store(*real_vt.add(i), Ordering::Relaxed);
            }
            for i in 0..PC_QUEUE_SLOTS {
                QUEUE_VT[i].store(*real_vt.add(i), Ordering::Relaxed);
            }
            install_qthunks();
            install_qlow();
            QUEUE_VT[10].store(q_exec_dispatch as usize, Ordering::Relaxed);
            QUEUE_VT[16].store(q_freq_dispatch as usize, Ordering::Relaxed);
            QUEUE_VT[17].store(q_calib_dispatch as usize, Ordering::Relaxed);
            XBOX_QVT[30].store(present_x as usize, Ordering::Relaxed);
            XBOX_QVT[23].store(signal_x as usize, Ordering::Relaxed);
            let base = GetModuleHandleW(core::ptr::null()) as usize;
            let e_lfanew = core::ptr::read_unaligned((base + 0x3c) as *const u32) as usize;
            let size = core::ptr::read_unaligned((base + e_lfanew + 0x50) as *const u32) as usize;
            EXE_BASE.store(base, Ordering::Relaxed);
            EXE_SIZE.store(size, Ordering::Relaxed);
            record("d3d12_xs", "queue.vtable", "built extended queue vtable (title slot 30 = PresentX; caller-dispatched)");
        }
        let slot0 = queue as *mut usize;
        let mut old: u32 = 0;
        if VirtualProtect(slot0 as *mut c_void, 8, 0x04, &mut old) != 0 {
            *slot0 = QUEUE_VT.as_ptr() as usize;
            let mut tmp: u32 = 0;
            VirtualProtect(slot0 as *mut c_void, 8, old, &mut tmp);
        }
    }
}
