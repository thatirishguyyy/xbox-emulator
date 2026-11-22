use crate::frame::{qpc, qpf};
use crate::overlay_font::Atlas;
use crate::overlay_sensors::Snapshot;
use compat_common::record;
use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::collections::VecDeque;
use std::sync::Mutex;

extern "system" {
    fn LoadLibraryExW(name: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
    fn GetProcAddress(h: *mut c_void, name: *const u8) -> *mut c_void;
}

#[link(name = "user32")]
extern "system" {
    fn GetAsyncKeyState(vk: i32) -> i16;
    fn GetForegroundWindow() -> *mut c_void;
}

#[repr(C)]
struct Guid(u32, u16, u16, [u8; 8]);

const IID_ROOT_SIGNATURE: Guid = Guid(0xc54a6b66, 0x72df, 0x4ee8, [0x8b, 0xe5, 0xa9, 0x46, 0xa1, 0x42, 0x92, 0x14]);
const IID_PSO: Guid = Guid(0x765a30f3, 0xf624, 0x4c6f, [0xa8, 0x28, 0xac, 0xe9, 0x48, 0x62, 0x24, 0x45]);
const IID_RESOURCE: Guid = Guid(0x696442be, 0xa72e, 0x4059, [0xbc, 0x79, 0x5b, 0x5c, 0x98, 0x04, 0x0f, 0xad]);
const IID_DESC_HEAP: Guid = Guid(0x8efb471d, 0x616c, 0x4f49, [0x90, 0xf7, 0x12, 0x7b, 0xb7, 0x63, 0xfa, 0x51]);
const IID_QUERY_HEAP: Guid = Guid(0x0d9658ae, 0xed45, 0x469e, [0xa6, 0x1d, 0x97, 0x0e, 0xc5, 0x83, 0xca, 0xb4]);

const COPY_DEST: u32 = 0x400;
const RENDER_TARGET: u32 = 0x4;
const MAX_INST: usize = 1536;
const INST_BYTES: usize = 64;
const RING: usize = 512;
const VK_F10: i32 = 0x79;

const LABEL: [f32; 3] = [0.56, 0.84, 1.0];
const VALUE: [f32; 3] = [1.0, 1.0, 1.0];
const UNIT: [f32; 3] = [0.66, 0.80, 0.94];
const GRAPH: [f32; 3] = [0.56, 0.86, 1.0];

const HLSL: &str = r#"#define RS "RootConstants(num32BitConstants=4, b0), SRV(t0), SRV(t1)"
cbuffer C : register(b0) { float2 inv; uint aw; uint texels; };
struct Q { float4 a; float4 uv; float4 col; float4 p; };
StructuredBuffer<Q> qs : register(t0);
ByteAddressBuffer atlas : register(t1);
struct V { float4 pos : SV_Position; float2 uv : TEXCOORD0; float4 col : COLOR0; nointerpolation uint kind : KIND0; };
[RootSignature(RS)]
V vs_main(uint vid : SV_VertexID, uint iid : SV_InstanceID) {
    Q q = qs[iid];
    float2 c = float2((vid == 1 || vid == 2 || vid == 4) ? 1.0 : 0.0, (vid == 2 || vid == 4 || vid == 5) ? 1.0 : 0.0);
    uint kind = min((uint)max(q.p.x, 0.0), 2u);
    float2 pos;
    if (kind == 2) {
        float2 d = q.a.zw - q.a.xy;
        d /= max(length(d), 1e-4);
        float2 n = float2(-d.y, d.x);
        float h = q.p.y * 0.5;
        pos = lerp(q.a.xy - d * h, q.a.zw + d * h, c.x) + n * (c.y * 2.0 - 1.0) * h;
    } else {
        pos = lerp(q.a.xy, q.a.zw, c);
    }
    V o;
    o.pos = float4(pos.x * inv.x * 2.0 - 1.0, 1.0 - pos.y * inv.y * 2.0, 0.0, 1.0);
    o.uv = lerp(q.uv.xy, q.uv.zw, c);
    o.col = q.col;
    o.kind = kind;
    return o;
}
[RootSignature(RS)]
float4 ps_main(V i) : SV_Target {
    if (i.kind == 1) {
        uint2 t = (uint2)i.uv;
        uint v = atlas.Load(min(t.y * aw + min(t.x, aw - 1), texels - 1) * 4);
        float f = (v & 255) / 255.0;
        float o = ((v >> 8) & 255) / 255.0;
        return float4(i.col.rgb * f, max(f, o * i.col.a));
    }
    return float4(i.col.rgb * i.col.a, i.col.a);
}
"#;

static SW: crate::live_sw::Sw = crate::live_sw::Sw::new("TOMBOX_OVERLAY", "overlay", true);
static NODRAW: crate::live_sw::Sw = crate::live_sw::Sw::new("TOMBOX_OVERLAY_NODRAW", "overlay_nodraw", false);
static USER: AtomicU8 = AtomicU8::new(0);
static CFG_LAST: AtomicU8 = AtomicU8::new(0);
static KEY_DOWN: AtomicBool = AtomicBool::new(false);
static DISABLED: AtomicBool = AtomicBool::new(false);

pub fn active() -> bool {
    let cfg = SW.on();
    let c = if cfg { 2 } else { 1 };
    if CFG_LAST.swap(c, Ordering::Relaxed) != c {
        USER.store(0, Ordering::Relaxed);
    }
    match USER.load(Ordering::Relaxed) {
        1 => true,
        2 => false,
        _ => cfg,
    }
}

fn poll_hotkey(hwnd: usize) {
    if hwnd == 0 || compat_common::host_input::kbm_hidden() || unsafe { GetForegroundWindow() } as usize != hwnd {
        KEY_DOWN.store(false, Ordering::Relaxed);
        return;
    }
    let down = unsafe { GetAsyncKeyState(VK_F10) } as u16 & 0x8000 != 0;
    if !down {
        KEY_DOWN.store(false, Ordering::Relaxed);
        return;
    }
    if !KEY_DOWN.swap(true, Ordering::Relaxed) {
        let on = !active();
        USER.store(if on { 1 } else { 2 }, Ordering::Relaxed);
        record("d3d12_xs", "[overlay]", if on { "F10: overlay on" } else { "F10: overlay off" });
    }
}

struct Timing {
    last: i64,
    ring: [f32; RING],
    head: usize,
    count: usize,
    win_start: i64,
    win_frames: u32,
    cur_fps: Option<f32>,
    cur_ft: Option<f32>,
    sec_start: i64,
    sec_frames: u32,
    secs: VecDeque<f32>,
}

static TIMING: Mutex<Timing> = Mutex::new(Timing {
    last: 0,
    ring: [0.0; RING],
    head: 0,
    count: 0,
    win_start: 0,
    win_frames: 0,
    cur_fps: None,
    cur_ft: None,
    sec_start: 0,
    sec_frames: 0,
    secs: VecDeque::new(),
});

pub fn tick(hwnd: usize) {
    poll_hotkey(hwnd);
    let now = qpc();
    let f = qpf();
    let mut t = TIMING.lock().unwrap();
    if t.last == 0 || now - t.last > f * 2 {
        t.last = now;
        t.win_start = now;
        t.win_frames = 0;
        t.sec_start = now;
        t.sec_frames = 0;
        return;
    }
    let dt = ((now - t.last) as f64 * 1000.0 / f as f64) as f32;
    t.last = now;
    let h = t.head;
    t.ring[h] = dt;
    t.head = (h + 1) % RING;
    t.count = (t.count + 1).min(RING);
    t.win_frames += 1;
    t.sec_frames += 1;
    if now - t.win_start >= f / 4 {
        let ms = (now - t.win_start) as f64 * 1000.0 / f as f64;
        t.cur_fps = Some((t.win_frames as f64 * 1000.0 / ms) as f32);
        t.cur_ft = Some((ms / t.win_frames as f64) as f32);
        t.win_start = now;
        t.win_frames = 0;
    }
    if now - t.sec_start >= f {
        let fps = (t.sec_frames as f64 * f as f64 / (now - t.sec_start) as f64) as f32;
        t.secs.push_back(fps);
        if t.secs.len() > 30 {
            t.secs.pop_front();
        }
        t.sec_start = now;
        t.sec_frames = 0;
    }
}

struct Gpu {
    fmt: u32,
    rs: usize,
    pso: usize,
    rtv: usize,
    inst_map: [*mut u8; 2],
    inst_va: [u64; 2],
    atlas_va: u64,
    font: Atlas,
    qheap: usize,
    rb: usize,
    rb_map: *const u64,
    ts_freq: u64,
    slot: usize,
    ts_valid: [bool; 2],
    frames: u64,
    cpu_ticks: i64,
    gpu_ticks: u64,
    gpu_n: u64,
    keep: Vec<usize>,
}
unsafe impl Send for Gpu {}

static GPU: Mutex<Option<Gpu>> = Mutex::new(None);

unsafe fn vslot(o: usize, i: usize) -> usize {
    unsafe { *(*(o as *const *const usize)).add(i) }
}

macro_rules! vc {
    ($o:expr, $i:expr, fn($($t:ty),*) -> $r:ty $(, $a:expr)*) => {{
        let o = $o as usize;
        let f: unsafe extern "system" fn(usize $(, $t)*) -> $r = core::mem::transmute(vslot(o, $i));
        f(o $(, $a)*)
    }};
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

unsafe fn blob_bytes(b: usize) -> Vec<u8> {
    unsafe {
        let p = vc!(b, 3, fn() -> *const u8);
        let n = vc!(b, 4, fn() -> usize);
        let v = core::slice::from_raw_parts(p, n).to_vec();
        vc!(b, 2, fn() -> u32);
        v
    }
}

unsafe fn compile(entry: &str, profile: &str) -> Result<Vec<u8>, String> {
    type D3DCompile = unsafe extern "system" fn(*const u8, usize, *const u8, *const c_void, *const c_void, *const u8, *const u8, u32, u32, *mut usize, *mut usize) -> i32;
    unsafe {
        let h = LoadLibraryExW(wide("d3dcompiler_47.dll").as_ptr(), core::ptr::null_mut(), 0x800);
        if h.is_null() {
            return Err("d3dcompiler_47.dll not found in System32".into());
        }
        let f = GetProcAddress(h, b"D3DCompile\0".as_ptr());
        if f.is_null() {
            return Err("D3DCompile export missing".into());
        }
        let f: D3DCompile = core::mem::transmute(f);
        let e = format!("{entry}\0");
        let p = format!("{profile}\0");
        let (mut code, mut err) = (0usize, 0usize);
        let hr = f(HLSL.as_ptr(), HLSL.len(), b"tombox_overlay\0".as_ptr(), core::ptr::null(), core::ptr::null(), e.as_ptr(), p.as_ptr(), 1 << 15, 0, &mut code, &mut err);
        let msg = if err != 0 { String::from_utf8_lossy(&blob_bytes(err)).into_owned() } else { String::new() };
        if hr < 0 || code == 0 {
            return Err(format!("D3DCompile {entry} hr={hr:#010x} {msg}"));
        }
        Ok(blob_bytes(code))
    }
}

unsafe fn buffer(dev: usize, heap: u32, size: u64, state: u32) -> Result<usize, String> {
    let props: [u32; 5] = [heap, 0, 0, 1, 1];
    let mut d = [0u8; 0x38];
    d[0..4].copy_from_slice(&1u32.to_le_bytes());
    d[0x10..0x18].copy_from_slice(&size.to_le_bytes());
    d[0x18..0x1c].copy_from_slice(&1u32.to_le_bytes());
    d[0x1c..0x1e].copy_from_slice(&1u16.to_le_bytes());
    d[0x1e..0x20].copy_from_slice(&1u16.to_le_bytes());
    d[0x24..0x28].copy_from_slice(&1u32.to_le_bytes());
    d[0x2c..0x30].copy_from_slice(&1u32.to_le_bytes());
    let mut r = 0usize;
    let hr = unsafe { vc!(dev, 27, fn(*const u32, u32, *const u8, u32, *const c_void, *const Guid, *mut usize) -> i32, props.as_ptr(), 0, d.as_ptr(), state, core::ptr::null(), &IID_RESOURCE, &mut r) };
    if hr < 0 || r == 0 {
        return Err(format!("CreateCommittedResource heap={heap} size={size} hr={hr:#010x}"));
    }
    Ok(r)
}

unsafe fn map(r: usize, read: bool) -> Result<*mut u8, String> {
    let mut p: *mut u8 = core::ptr::null_mut();
    let empty = [0usize; 2];
    let range = if read { core::ptr::null() } else { empty.as_ptr() };
    let hr = unsafe { vc!(r, 8, fn(u32, *const usize, *mut *mut u8) -> i32, 0, range, &mut p) };
    if hr < 0 || p.is_null() {
        return Err(format!("Map hr={hr:#010x}"));
    }
    Ok(p)
}

fn put(d: &mut [u8], o: usize, v: &[u8]) {
    d[o..o + v.len()].copy_from_slice(v);
}

fn pso_desc(rs: usize, vs: &[u8], ps: &[u8], fmt: u32) -> Vec<u8> {
    let mut d = vec![0u8; crate::pso_serial::GFX_DESC_SIZE];
    put(&mut d, 0, &(rs as u64).to_le_bytes());
    put(&mut d, 0x08, &(vs.as_ptr() as u64).to_le_bytes());
    put(&mut d, 0x10, &(vs.len() as u64).to_le_bytes());
    put(&mut d, 0x18, &(ps.as_ptr() as u64).to_le_bytes());
    put(&mut d, 0x20, &(ps.len() as u64).to_le_bytes());
    for rt in 0..8 {
        let o = 0x80 + rt * 40;
        let v: [u32; 9] = [(rt == 0) as u32, 0, 2, 6, 1, 1, 2, 1, 4];
        for (i, x) in v.iter().enumerate() {
            put(&mut d, o + i * 4, &x.to_le_bytes());
        }
        d[o + 36] = if rt == 0 { 7 } else { 0xf };
    }
    put(&mut d, 0x1c0, &u32::MAX.to_le_bytes());
    put(&mut d, 0x1c4, &3u32.to_le_bytes());
    put(&mut d, 0x1c8, &1u32.to_le_bytes());
    put(&mut d, 0x1dc, &1u32.to_le_bytes());
    put(&mut d, 0x1f8, &8u32.to_le_bytes());
    d[0x200] = 0xff;
    d[0x201] = 0xff;
    for face in [0x204usize, 0x214] {
        put(&mut d, face, &1u32.to_le_bytes());
        put(&mut d, face + 4, &1u32.to_le_bytes());
        put(&mut d, face + 8, &1u32.to_le_bytes());
        put(&mut d, face + 12, &8u32.to_le_bytes());
    }
    put(&mut d, 0x23c, &3u32.to_le_bytes());
    put(&mut d, 0x240, &1u32.to_le_bytes());
    put(&mut d, 0x244, &fmt.to_le_bytes());
    put(&mut d, 0x268, &1u32.to_le_bytes());
    d
}

unsafe fn init(dev: usize, queue: usize, fmt: u32, height: u32) -> Result<Gpu, String> {
    unsafe {
        let vs = compile("vs_main", "vs_5_1")?;
        let ps = compile("ps_main", "ps_5_1")?;
        let orig = crate::device_wrap::real_device_orig(16);
        let create_rs: unsafe extern "system" fn(usize, u32, *const u8, usize, *const Guid, *mut usize) -> i32 =
            core::mem::transmute(if orig != 0 { orig } else { vslot(dev, 16) });
        let mut rs = 0usize;
        let hr = create_rs(dev, 0, vs.as_ptr(), vs.len(), &IID_ROOT_SIGNATURE, &mut rs);
        if hr < 0 || rs == 0 {
            return Err(format!("CreateRootSignature hr={hr:#010x}"));
        }
        let desc = pso_desc(rs, &vs, &ps, fmt);
        let mut pso = 0usize;
        let hr = vc!(dev, 10, fn(*const u8, *const Guid, *mut usize) -> i32, desc.as_ptr(), &IID_PSO, &mut pso);
        if hr < 0 || pso == 0 {
            return Err(format!("CreateGraphicsPipelineState fmt={fmt} hr={hr:#010x}"));
        }
        let hd: [u32; 4] = [2, 1, 0, 0];
        let mut heap = 0usize;
        let hr = vc!(dev, 14, fn(*const u32, *const Guid, *mut usize) -> i32, hd.as_ptr(), &IID_DESC_HEAP, &mut heap);
        if hr < 0 || heap == 0 {
            return Err(format!("CreateDescriptorHeap(RTV) hr={hr:#010x}"));
        }
        let mut out = 0usize;
        let ret = vc!(heap, 9, fn(*mut usize) -> usize, &mut out);
        let rtv = if out != 0 { out } else { ret };
        if rtv == 0 {
            return Err("RTV heap start handle is null".into());
        }
        let px = (height as i32 / 54).clamp(12, 40);
        let font = crate::overlay_font::build(px)?;
        let atlas_bytes = (font.texels.len() * 4) as u64;
        let atlas = buffer(dev, 2, atlas_bytes.max(256), 0xac3)?;
        let ap = map(atlas, false)?;
        core::ptr::copy_nonoverlapping(font.texels.as_ptr() as *const u8, ap, atlas_bytes as usize);
        let atlas_va = vc!(atlas, 11, fn() -> u64);
        let mut inst = [0usize; 2];
        let mut inst_map = [core::ptr::null_mut(); 2];
        let mut inst_va = [0u64; 2];
        for i in 0..2 {
            inst[i] = buffer(dev, 2, (MAX_INST * INST_BYTES) as u64, 0xac3)?;
            inst_map[i] = map(inst[i], false)?;
            inst_va[i] = vc!(inst[i], 11, fn() -> u64);
        }
        let qd: [u32; 3] = [1, 4, 0];
        let mut qheap = 0usize;
        let use_ts = std::env::var("TOMBOX_OVERLAY_TS").map(|v| v.trim() != "0").unwrap_or(true);
        let hr = if use_ts { vc!(dev, 39, fn(*const u32, *const Guid, *mut usize) -> i32, qd.as_ptr(), &IID_QUERY_HEAP, &mut qheap) } else { -1 };
        let mut rb = 0usize;
        let mut rb_map: *const u64 = core::ptr::null();
        let mut ts_freq = 0u64;
        if hr >= 0 && qheap != 0 {
            rb = buffer(dev, 3, 256, COPY_DEST)?;
            rb_map = map(rb, true)? as *const u64;
            let f = crate::present::real_queue_slot(16);
            if f != 0 {
                let f: unsafe extern "system" fn(usize, *mut u64) -> i32 = core::mem::transmute(f);
                f(queue, &mut ts_freq);
            }
        } else {
            qheap = 0;
        }
        let offs: Vec<String> = [atlas, inst[0], inst[1], rb].iter().map(|&r| format!("{:#x}/{}", crate::placed_offset::bytes(r), crate::title_shadow::map_va(r as *mut c_void).is_some() as u8)).collect();
        record("d3d12_xs", "[overlay]", &format!("buffers atlas va={atlas_va:#x} inst va={:#x},{:#x} placed_offset/shadow={}", inst_va[0], inst_va[1], offs.join(" ")));
        record("d3d12_xs", "[overlay]", &format!(
            "ready fmt={fmt} font=Consolas bold {px}px cell {}x{} atlas {}x{} ({} KB) timestamps={} (freq {ts_freq})",
            font.cell_w, font.cell_h, font.width, font.height, atlas_bytes / 1024, if qheap != 0 { "on" } else { "off" }));
        Ok(Gpu {
            fmt, rs, pso, rtv, inst_map, inst_va, atlas_va, font, qheap, rb, rb_map, ts_freq,
            slot: 0, ts_valid: [false; 2], frames: 0, cpu_ticks: 0, gpu_ticks: 0, gpu_n: 0,
            keep: vec![heap, atlas, inst[0], inst[1]],
        })
    }
}

struct Builder<'a> {
    q: Vec<[f32; 16]>,
    font: &'a Atlas,
}

impl Builder<'_> {
    fn push(&mut self, v: [f32; 16]) {
        if self.q.len() < MAX_INST {
            self.q.push(v);
        }
    }

    fn rect(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, c: [f32; 3], a: f32) {
        self.push([x0, y0, x1, y1, 0.0, 0.0, 0.0, 0.0, c[0], c[1], c[2], a, 0.0, 0.0, 0.0, 0.0]);
    }

    fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, w: f32, c: [f32; 3], a: f32) {
        self.push([x0, y0, x1, y1, 0.0, 0.0, 0.0, 0.0, c[0], c[1], c[2], a, 2.0, w, 0.0, 0.0]);
    }

    fn text(&mut self, x: f32, y: f32, s: &str, c: [f32; 3]) {
        let f = self.font;
        let (x, y) = (x.round(), y.round());
        for (i, ch) in s.chars().enumerate() {
            let Some(g) = f.glyph(ch) else { continue };
            let (u, v) = f.origin(g);
            let gx = x + (i as u32 * f.advance) as f32 - f.pad as f32;
            let gy = y - f.pad as f32;
            let (cw, chh) = (f.cell_w as f32, f.cell_h as f32);
            self.push([gx, gy, gx + cw, gy + chh, u as f32, v as f32, u as f32 + cw, v as f32 + chh, c[0], c[1], c[2], 0.95, 1.0, 0.0, 0.0, 0.0]);
        }
    }
}

fn num(v: Option<f32>, prec: usize) -> String {
    match v {
        Some(v) if v.is_finite() => format!("{v:.prec$}"),
        _ => "--".into(),
    }
}

fn layout(b: &mut Builder, s: &Snapshot, w: u32) {
    let (fps, ft, secs, graph) = {
        let t = TIMING.lock().unwrap();
        let n = t.count;
        let graph: Vec<f32> = (0..n).map(|i| t.ring[(t.head + RING - n + i) % RING]).collect();
        (t.cur_fps, t.cur_ft, t.secs.iter().copied().collect::<Vec<f32>>(), graph)
    };
    let (mn, avg, mx) = if secs.is_empty() {
        (None, None, None)
    } else {
        let mn = secs.iter().copied().fold(f32::MAX, f32::min);
        let mx = secs.iter().copied().fold(0.0f32, f32::max);
        (Some(mn), Some(secs.iter().sum::<f32>() / secs.len() as f32), Some(mx))
    };
    let minavg = match (mn, avg) {
        (Some(a), Some(b)) => format!("{a:.0}/{b:.0}"),
        _ => "--/--".into(),
    };
    let rows: Vec<(String, Vec<(String, &str)>)> = vec![
        (s.cpu_name.clone().unwrap_or_else(|| "CPU".into()), vec![(num(s.cpu_usage, 0), "%"), (num(s.cpu_temp, 0), "°C"), (num(s.cpu_mhz, 0), "MHz")]),
        (s.gpu_name.clone().unwrap_or_else(|| "GPU".into()), vec![(num(s.gpu_usage, 0), "%"), (num(s.gpu_temp, 0), "°C"), (num(s.gpu_mhz, 0), "MHz"), (num(s.vram_mb, 0), "MB")]),
        ("FPS".into(), vec![(num(fps, 0), ""), (minavg, ""), (num(mx, 0), "")]),
        ("Frametime".into(), vec![(num(ft, 1), "ms")]),
    ];
    let f = b.font;
    let adv = f.advance as f32;
    let unit_gap = (adv * 0.3).round();
    let line_h = f.line as f32 + (f.line as f32 * 0.12).round();
    let width_of = |v: &str, u: &str| v.chars().count() as f32 * adv + if u.is_empty() { 0.0 } else { unit_gap + u.chars().count() as f32 * adv };
    let name_w = rows.iter().map(|r| r.0.chars().count()).max().unwrap_or(0) as f32 * adv;
    let ncols = rows.iter().map(|r| r.1.len()).max().unwrap_or(0);
    let mut colw = vec![0.0f32; ncols];
    for r in &rows {
        for (k, (v, u)) in r.1.iter().enumerate() {
            colw[k] = colw[k].max(width_of(v, u));
        }
    }
    let gap = (adv * 1.5).round();
    let margin = (adv * 0.8).round();
    let x0 = margin;
    let y0 = margin;
    let total_w = name_w + colw.iter().map(|c| c + gap).sum::<f32>();
    let graph_w = total_w.max(adv * 24.0).min(w as f32 * 0.5).round();
    let graph_h = (line_h * 3.0).round();
    let pad = (adv * 0.5).round();
    let text_h = line_h * rows.len() as f32;
    let bg_h = text_h + pad * 0.5 + graph_h + pad * 2.0;
    b.rect(x0 - pad, y0 - pad, x0 + total_w.max(graph_w) + pad, y0 + bg_h - pad, [0.0, 0.0, 0.0], 0.32);
    for (ri, (name, fields)) in rows.iter().enumerate() {
        let y = y0 + ri as f32 * line_h;
        b.text(x0, y, name, LABEL);
        let mut cx = x0 + name_w + gap;
        for (k, (v, u)) in fields.iter().enumerate() {
            let right = cx + colw[k];
            let vx = right - width_of(v, u);
            b.text(vx, y, v, VALUE);
            if !u.is_empty() {
                b.text(vx + v.chars().count() as f32 * adv + unit_gap, y, u, UNIT);
            }
            cx = right + gap;
        }
    }
    let gy0 = (y0 + text_h + pad * 0.5).round();
    let gy1 = gy0 + graph_h;
    let gx1 = x0 + graph_w;
    b.rect(x0, gy0, gx1, gy1, [0.0, 0.0, 0.0], 0.25);
    b.line(x0, gy0, gx1, gy0, 1.0, UNIT, 0.35);
    b.line(x0, gy1, gx1, gy1, 1.0, UNIT, 0.35);
    b.line(x0, (gy0 + gy1) * 0.5, gx1, (gy0 + gy1) * 0.5, 1.0, UNIT, 0.15);
    let step = 2.0f32;
    let n = graph.len().min((graph_w / step) as usize + 1);
    if n >= 2 {
        let pts = &graph[graph.len() - n..];
        let peak = pts.iter().copied().fold(0.0f32, f32::max) * 1.1;
        let scale = [8.33f32, 16.67, 33.33, 50.0, 66.67, 100.0, 200.0, 500.0, 1000.0, 5000.0].into_iter().find(|&s| s >= peak).unwrap_or(peak.max(1.0));
        let pt = |i: usize| (gx1 - (n - 1 - i) as f32 * step, gy1 - (pts[i] / scale).clamp(0.0, 1.0) * (graph_h - 2.0) - 1.0);
        for pass in 0..2 {
            for i in 1..n {
                let (ax, ay) = pt(i - 1);
                let (bx, by) = pt(i);
                if pass == 0 {
                    b.line(ax, ay, bx, by, 3.5, [0.0, 0.0, 0.0], 0.55);
                } else {
                    b.line(ax, ay, bx, by, 1.6, GRAPH, 1.0);
                }
            }
        }
        let label = format!("{scale:.0} ms");
        b.text(gx1 - label.chars().count() as f32 * adv - pad * 0.5, gy0 + pad * 0.3, &label, UNIT);
    }
}

pub unsafe fn draw(device: *mut c_void, queue: *mut c_void, list: *mut c_void, dst: *mut c_void, fmt: u32, width: u64, height: u32) -> bool {
    if DISABLED.load(Ordering::Relaxed) || !active() {
        return false;
    }
    let t0 = qpc();
    let mut guard = GPU.lock().unwrap();
    if guard.as_ref().is_some_and(|g| g.fmt != fmt) {
        *guard = None;
    }
    if guard.is_none() {
        let ti = qpc();
        match unsafe { init(device as usize, queue as usize, fmt, height) } {
            Ok(g) => {
                record("d3d12_xs", "[overlay]", &format!("init (shader compile, root signature, PSO, font atlas, buffers) took {:.1} ms on the present thread", (qpc() - ti) as f64 * 1000.0 / qpf() as f64));
                *guard = Some(g)
            }
            Err(e) => {
                DISABLED.store(true, Ordering::Relaxed);
                record("d3d12_xs", "[overlay]", &format!("init FAILED, overlay disabled: {e}"));
                return false;
            }
        }
    }
    let g = guard.as_mut().unwrap();
    let snap = crate::overlay_sensors::latest().unwrap_or_default();
    let mut b = Builder { q: Vec::with_capacity(512), font: &g.font };
    layout(&mut b, &snap, width as u32);
    let quads = b.q;
    let slot = g.slot;
    static RAMP: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let ramp = *RAMP.get_or_init(|| std::env::var("TOMBOX_OVERLAY_RAMP").is_ok_and(|v| v == "1"));
    let stage = if ramp { (g.frames / 150 + 1).min(4) } else { 4 };
    if ramp && g.frames % 150 == 0 && g.frames <= 450 {
        record("d3d12_xs", "[overlay]", &format!("frame {} stage {stage}", g.frames));
    }
    unsafe {
        core::ptr::copy_nonoverlapping(quads.as_ptr() as *const u8, g.inst_map[slot], quads.len() * INST_BYTES);
    }
    if stage == 1 {
        g.frames += 1;
        return false;
    }
    unsafe {
        if g.qheap != 0 && stage >= 4 && g.ts_valid[slot] && g.ts_freq != 0 {
            let a = core::ptr::read_volatile(g.rb_map.add(slot * 2));
            let z = core::ptr::read_volatile(g.rb_map.add(slot * 2 + 1));
            if z > a && z - a < g.ts_freq {
                g.gpu_ticks += z - a;
                g.gpu_n += 1;
            }
        }
        let l = list as usize;
        let bar = [[0u64, dst as u64, 0xffff_ffffu64 | ((COPY_DEST as u64) << 32), RENDER_TARGET as u64]];
        vc!(l, 26, fn(u32, *const [u64; 4]) -> (), 1, bar.as_ptr());
        vc!(device as usize, 20, fn(*mut c_void, *const c_void, usize) -> (), dst, core::ptr::null(), g.rtv);
        if stage == 2 {
            let rtv = [g.rtv];
            vc!(l, 46, fn(u32, *const usize, i32, *const usize) -> (), 1, rtv.as_ptr(), 0, core::ptr::null());
            g.frames += 1;
            return true;
        }
        vc!(l, 30, fn(usize) -> (), g.rs);
        vc!(l, 25, fn(usize) -> (), g.pso);
        vc!(l, 20, fn(u32) -> (), 4);
        let vp: [f32; 6] = [0.0, 0.0, width as f32, height as f32, 0.0, 1.0];
        vc!(l, 21, fn(u32, *const f32) -> (), 1, vp.as_ptr());
        let sc: [i32; 4] = [0, 0, width as i32, height as i32];
        vc!(l, 22, fn(u32, *const i32) -> (), 1, sc.as_ptr());
        let rtv = [g.rtv];
        vc!(l, 46, fn(u32, *const usize, i32, *const usize) -> (), 1, rtv.as_ptr(), 0, core::ptr::null());
        let consts: [u32; 4] = [(1.0 / width as f32).to_bits(), (1.0 / height as f32).to_bits(), g.font.width, g.font.width * g.font.height];
        vc!(l, 36, fn(u32, u32, *const u32, u32) -> (), 0, 4, consts.as_ptr(), 0);
        vc!(l, 40, fn(u32, u64) -> (), 1, g.inst_va[slot]);
        vc!(l, 40, fn(u32, u64) -> (), 2, g.atlas_va);
        let ts = g.qheap != 0 && stage >= 4;
        if ts {
            vc!(l, 53, fn(usize, u32, u32) -> (), g.qheap, 2, (slot * 2) as u32);
        }
        if !NODRAW.on() {
            vc!(l, 12, fn(u32, u32, u32, u32) -> (), 6, quads.len() as u32, 0, 0);
        }
        if ts {
            vc!(l, 53, fn(usize, u32, u32) -> (), g.qheap, 2, (slot * 2 + 1) as u32);
            vc!(l, 54, fn(usize, u32, u32, u32, usize, u64) -> (), g.qheap, 2, (slot * 2) as u32, 2, g.rb, (slot * 16) as u64);
            g.ts_valid[slot] = true;
        }
    }
    g.slot ^= 1;
    g.frames += 1;
    let dt = qpc() - t0;
    g.cpu_ticks += dt;
    crate::perf::add(crate::perf::OVERLAY, dt);
    if g.frames % 600 == 0 {
        let cpu_us = g.cpu_ticks as f64 * 1e6 / qpf() as f64 / 600.0;
        let gpu = if g.gpu_n > 0 && g.ts_freq > 0 { format!("{:.1}us", g.gpu_ticks as f64 * 1e6 / g.ts_freq as f64 / g.gpu_n as f64) } else { "--".into() };
        record("d3d12_xs", "[overlay]", &format!("cost over 600 frames: cpu {cpu_us:.1}us/frame gpu {gpu}/frame, {} quads", quads.len()));
        g.cpu_ticks = 0;
        g.gpu_ticks = 0;
        g.gpu_n = 0;
    }
    let _ = &g.keep;
    true
}
