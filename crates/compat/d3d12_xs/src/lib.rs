//! `d3d12_xs.dll` compatibility shim (Xbox Series Direct3D 12.x).
//!
//! The Fortnite RHI creates its device via `D3D12XboxCreateDevice(adapter,
//! D3D12XBOX_CREATE_DEVICE_PARAMETERS*, riid, ppDevice)` and the requested device
//! IID is **identical to PC `ID3D12Device`** `{189819f1-...}`. So we create a real
//! PC D3D12 device through the host `d3d12.dll` and hand it back. The Xbox-only
//! create parameters are ignored (PC uses adapter + feature level). Root-signature
//! serialization forwards to the host `d3d12.dll` (same exports).
//!
//! This is the entry to the graphics-translation layer: the base ID3D12Device
//! matches, but D3D12.x adds Xbox-specific device methods/semantics beyond the
//! base interface — those will be addressed as the RHI exercises them.

mod device_wrap;
mod dxgi_device;
mod xone_dxgi;
mod xone_queue;
mod ecl_probe;
mod cbv_raw;
mod bl_alias;
mod vs_snorm;
mod pso_async;
mod xone_heap;
mod agility;
mod cmd_spy;
mod xone_barrier;
mod xone_tex;
mod heap_wrap;
mod cmdlist_wrap;
mod blendclamp;
mod gpuprof;
mod slowcall;
mod console_slots;
mod console_slots_cl2;
mod cl2_state;
mod cmdlist_validate;
mod shader_xlat;
mod mip_bind;
mod present;
mod overlay;
mod frame_csv;
mod overlay_font;
mod overlay_sensors;
#[cfg(test)]
mod overlay_hw;
mod present_snap;
mod present_probe;
mod frame;
mod frame_fast;
mod perf;
mod present_wait;
mod qwait;
mod placed;
mod upload;
mod reaper;
mod rtcap;
mod bindtrace;
mod replay;
mod map_x;
mod alias;
mod replay_ext;
mod live_sw;
mod hot_reload;
mod shader_dump;
mod vmem_x;
mod device_x;
mod device_ext_a;
mod sample_pattern;
mod pso_serial;
mod dma_list;
mod cmdlist_x;
mod query_x;
mod gpu_va;
mod fill;
mod process_debug;
mod timestamp;
mod video_probe;
mod xs_exports;
mod console_slots_dev2;
mod gpu_hw_config;
mod kmd_slots;
mod placed_offset;
mod msaa_place;
mod gpu_wait;
mod cl_ext;
mod dbcount;
mod cl_fw;
mod gds;
pub mod centroid_bind;
mod list_split;
mod sys_ts;
mod ts99;
mod indirect_bundle;
mod bop64;
mod wait_probe;
#[cfg(test)]
mod hw_tests;
mod driver_state;
#[cfg(test)]
mod driver_unload_tests;
mod title_heap;
mod mesh_pso;
mod console_slots_mesh;
mod cl_console_fix;
mod resolve_x;
mod device_console_fix;
mod pso_ext;
mod global_bind;
mod kmd10;
mod dred;
mod title_shadow;
pub mod perfctr;
#[cfg(test)]
mod perfctr_tests;
pub mod perfctr_xs;
#[cfg(test)]
mod perfctr_xs_tests;
pub mod perfctr_cl;
#[cfg(test)]
mod perfctr_cl_hw;
pub mod rs_xbox;
#[cfg(test)]
mod rs_xbox_tests;
#[cfg(test)]
mod slot_audit_tests;
#[cfg(test)]
mod create_device_tests;

use core::ffi::c_void;
use core::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Once;
use compat_common::{record, E_FAIL, S_OK};

const M: &str = "d3d12_xs";

type HMODULE = *mut c_void;
extern "system" {
    fn LoadLibraryW(p: *const u16) -> HMODULE;
    fn GetProcAddress(h: HMODULE, n: *const u8) -> *mut c_void;
}

#[no_mangle]
pub static IID_ID3D12Device: [u8; 16] = [
    0xF1, 0x19, 0x98, 0x18, 0xB6, 0x1D, 0x57, 0x4B, 0xBE, 0x54, 0x18, 0x21, 0x33, 0x9B, 0x85, 0xF7,
];

const D3D_FEATURE_LEVEL_12_0: u32 = 0xc000;
pub const E_INVALIDARG: i32 = 0x8007_0057u32 as i32;

/// IID_ID3D12Debug = {344488b7-6846-474b-b989-f027448245e0}
static IID_ID3D12DEBUG: device_wrap::Guid = device_wrap::Guid {
    d1: 0x3444_88b7, d2: 0x6846, d3: 0x474b, d4: [0xb9, 0x89, 0xf0, 0x27, 0x44, 0x82, 0x45, 0xe0],
};

/// Enable the D3D12 debug layer (best-effort) so validation errors are emitted via
/// OutputDebugString and captured by the tracer. Gated on env TOMBOX_D3D12_DEBUG.
fn maybe_enable_debug_layer(params: *const c_void) {
    if !unsafe { process_debug::on_create_device(params, std::env::var("TOMBOX_D3D12_DEBUG").is_ok()) } {
        return;
    }
    let p = host_proc(b"D3D12GetDebugInterface\0");
    if p == 0 {
        return;
    }
    let f: unsafe extern "system" fn(*const device_wrap::Guid, *mut *mut c_void) -> i32 =
        unsafe { core::mem::transmute(p) };
    let mut dbg: *mut c_void = core::ptr::null_mut();
    let hr = unsafe { f(&IID_ID3D12DEBUG, &mut dbg) };
    if hr == 0 && !dbg.is_null() {
        unsafe {
            let vt = *(dbg as *const *const usize);
            let enable: unsafe extern "system" fn(*mut c_void) = core::mem::transmute(*vt.add(3)); // EnableDebugLayer
            enable(dbg);
            let rel: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(*vt.add(2));
            rel(dbg);
        }
        record(M, "debug", "D3D12 debug layer ENABLED");
    } else {
        record(M, "debug", &format!("D3D12GetDebugInterface hr={hr:#010x} (SDK layers missing?)"));
    }
}

fn host_d3d12() -> HMODULE {
    static CELL: AtomicUsize = AtomicUsize::new(0);
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        // Full path avoids any name-based self-reference (we are d3d12_xs.dll).
        let name: Vec<u16> = r"C:\Windows\System32\d3d12.dll".encode_utf16().chain(std::iter::once(0)).collect();
        let h = unsafe { LoadLibraryW(name.as_ptr()) } as usize;
        record(M, "init", &format!("host d3d12.dll = {h:#x}"));
        CELL.store(h, Ordering::Relaxed);
    });
    CELL.load(Ordering::Relaxed) as HMODULE
}

fn host_proc(name: &[u8]) -> usize {
    let h = host_d3d12();
    if h.is_null() { return 0; }
    unsafe { GetProcAddress(h, name.as_ptr()) as usize }
}

/// Enable experimental shader models so the PC D3D12 runtime accepts the game's
/// DXIL without re-validating/-signing it against the MS validator. Xbox shaders are
/// standard DXIL but signed by the GDK validator; without this the runtime rejects
/// every graphics PSO with E_OUTOFMEMORY. Must be called before device creation.
/// Requires Windows Developer Mode; logs the HRESULT either way.
fn enable_experimental_shader_models() {
    #[repr(C)]
    struct Guid { a: u32, b: u16, c: u16, d: [u8; 8] }
    // D3D12ExperimentalShaderModels {76f5573e-f13a-40f5-b297-81ce9e18933f}
    const EXP_SM: Guid = Guid { a: 0x76f5_573e, b: 0xf13a, c: 0x40f5, d: [0xb2, 0x97, 0x81, 0xce, 0x9e, 0x18, 0x93, 0x3f] };
    let p = host_proc(b"D3D12EnableExperimentalFeatures\0");
    if p == 0 {
        record(M, "exp", "D3D12EnableExperimentalFeatures missing");
        return;
    }
    let f: extern "system" fn(u32, *const Guid, *const c_void, *const u32) -> i32 =
        unsafe { core::mem::transmute(p) };
    let hr = f(1, &EXP_SM, core::ptr::null(), core::ptr::null());
    record(M, "exp", &format!("EnableExperimentalFeatures(ShaderModels) hr={hr:#010x}"));
}

/// Xbox device creation -> real PC D3D12 device. adapter/params ignored; the
/// device IID matches PC ID3D12Device.
#[no_mangle]
pub extern "system" fn D3D12XboxCreateDevice(
    _adapter: *mut c_void,
    _params: *const c_void,
    riid: *const c_void,
    pp_device: *mut *mut c_void,
) -> i32 {
    if _params.is_null() {
        record(M, "D3D12XboxCreateDevice", "NULL creation parameters -> E_INVALIDARG");
        return E_INVALIDARG;
    }
    if !pp_device.is_null() {
        unsafe { *pp_device = core::ptr::null_mut() };
    }
    let _ = xgs_kmd_emu::api();
    present::start_window_clamp();
    agility::select();
    enable_experimental_shader_models();
    maybe_enable_debug_layer(_params);
    dred::enable_before_create();
    let p = host_proc(b"D3D12CreateDevice\0");
    if p == 0 {
        record(M, "D3D12XboxCreateDevice", "host D3D12CreateDevice missing -> E_FAIL");
        return E_FAIL;
    }
    // D3D12CreateDevice(IUnknown* adapter, D3D_FEATURE_LEVEL, REFIID, void**)
    let f: extern "system" fn(*mut c_void, u32, *const c_void, *mut *mut c_void) -> i32 =
        unsafe { core::mem::transmute(p) };
    let hr = agility::create_device(core::ptr::null_mut(), D3D_FEATURE_LEVEL_12_0, riid, pp_device).unwrap_or_else(|| f(core::ptr::null_mut(), D3D_FEATURE_LEVEL_12_0, riid, pp_device));
    process_debug::mark_device_created();
    if hr == S_OK && !pp_device.is_null() {
        let real = unsafe { *pp_device };
        if !real.is_null() {
            dred::watch(real);
            let wrapped = device_wrap::wrap_device(real);
            unsafe { *pp_device = wrapped };
            record(M, "D3D12XboxCreateDevice", &format!("host device {real:p} wrapped as {wrapped:p} (hr={hr:#010x})"));
            return hr;
        }
    }
    record(M, "D3D12XboxCreateDevice", &format!("host device create -> hr={hr:#010x} (unwrapped)"));
    hr
}

#[no_mangle]
pub extern "system" fn D3D12CreateDevice(
    adapter: *mut c_void,
    min_feature_level: u32,
    riid: *const c_void,
    pp_device: *mut *mut c_void,
) -> i32 {
    let params = default_create_params_for(process_debug::current(), process_debug::console_os_build());
    record(M, "D3D12CreateDevice", &format!("min feature level {min_feature_level:#x} ignored as on console; D3D12XboxCreateDevice with version {:#x} flags {:#010x}", params[0], params[1]));
    D3D12XboxCreateDevice(adapter, params.as_ptr() as *const c_void, riid, pp_device)
}

pub const CREATE_PARAMS_VERSION: u32 = 0x20004;
pub const CREATE_PARAMS_VERSION_19041: u32 = 0x20002;
pub const CREATE_PARAMS_BYTES: usize = 0x48;
pub const CREATE_PARAMS_BYTES_19041: usize = 0x40;

pub fn create_params_layout(os_build: u32) -> (u32, usize) {
    if os_build < process_debug::NEW_ABI_BUILD {
        (CREATE_PARAMS_VERSION_19041, CREATE_PARAMS_BYTES_19041)
    } else {
        (CREATE_PARAMS_VERSION, CREATE_PARAMS_BYTES)
    }
}

pub fn default_create_params(flags: u32) -> [u32; 18] {
    default_create_params_for(flags, process_debug::NEW_ABI_BUILD)
}

pub fn default_create_params_for(flags: u32, os_build: u32) -> [u32; 18] {
    let (version, _) = create_params_layout(os_build);
    let mut p = [0u32; 18];
    p[0] = version;
    p[1] = flags;
    p[2] = u32::MAX;
    p[6] = u32::MAX;
    p[7] = u32::MAX;
    p
}

#[no_mangle]
pub extern "system" fn D3D12SerializeRootSignature(
    desc: *const c_void, version: u32, ppblob: *mut *mut c_void, pperror: *mut *mut c_void,
) -> i32 {
    let p = host_proc(b"D3D12SerializeRootSignature\0");
    if p == 0 { return E_FAIL; }
    unsafe { rs_xbox::serialize(core::mem::transmute(p), desc, version, ppblob, pperror) }
}

#[no_mangle]
pub extern "system" fn D3D12SerializeVersionedRootSignature(
    desc: *const c_void, ppblob: *mut *mut c_void, pperror: *mut *mut c_void,
) -> i32 {
    let p = host_proc(b"D3D12SerializeVersionedRootSignature\0");
    if p == 0 { return E_FAIL; }
    unsafe { rs_xbox::serialize_versioned(core::mem::transmute(p), desc, ppblob, pperror) }
}

