use compat_common::record;
use core::ffi::c_void;
use core::sync::atomic::{AtomicUsize, Ordering};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Mutex;

const M: &str = "d3d12_xs";
const TAG: &str = "title_heap";

pub const FAMILY_ONE: u32 = 0;
pub const FAMILY_SCARLETT: u32 = 1;
pub const KIND_SRV: u8 = 1;
pub const KIND_UAV: u8 = 2;
pub const KIND_CBV: u8 = 3;
pub const KIND_SAMPLER: u8 = 4;
pub const KIND_DEFAULT: u8 = 0;
const CHUNK: usize = 64;
const SAMPLER_HEADER: u64 = 0x10;
const ROOT_FLAG_DIRECT_RESOURCES: u32 = 0x400;
const ROOT_FLAG_DIRECT_SAMPLERS: u32 = 0x800;
const DEFAULT_MAPPING: u32 = 0x1688;
const E_INVALIDARG: i32 = 0x8007_0057u32 as i32;
const IID_ID3D12_DEVICE: [u8; 16] = [0xf1, 0x19, 0x98, 0x18, 0xb6, 0x1d, 0x57, 0x4b, 0xbe, 0x54, 0x18, 0x21, 0x33, 0x9b, 0x85, 0xf7];

pub fn console_stride(ty: u32) -> u64 {
    if ty == 1 {
        16
    } else {
        32
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Image {
    pub va: u64,
    pub hw: u32,
    pub ty: u32,
    pub base_level: u32,
    pub last_level: u32,
    pub base_array: u32,
    pub last_array: u32,
    pub dst_sel: [u32; 4],
    pub min_lod: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Buffer {
    pub va: u64,
    pub stride: u32,
    pub records: u32,
    pub hw: Option<u32>,
    pub dst_sel: [u32; 4],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Slot {
    Null,
    Image(Image),
    Buffer(Buffer),
    Accel(u64),
}

pub fn hw_from_gcn(df: u32, nf: u32, limit: usize) -> Option<u32> {
    if df == 0 {
        return None;
    }
    HW_TO_GCN.iter().take(limit).enumerate().skip(1).find(|(_, e)| e[0] as u32 == df && e[1] as u32 == nf).map(|(i, _)| i as u32)
}

fn sel(w: u32) -> [u32; 4] {
    [w & 7, w >> 3 & 7, w >> 6 & 7, w >> 9 & 7]
}

pub fn decode_slot(family: u32, d: &[u32; 8]) -> Slot {
    if d.iter().all(|&w| w == 0) {
        return Slot::Null;
    }
    let ty = d[3] >> 28;
    if ty >= 8 {
        let va = ((d[0] as u64) | ((d[1] & 0xff) as u64) << 32) << 8;
        let hw = if family == FAMILY_SCARLETT {
            Some(d[1] >> 20 & 0x1ff).filter(|&h| h != 0)
        } else {
            hw_from_gcn(d[1] >> 20 & 0x3f, d[1] >> 26 & 0xf, HW_TO_GCN.len())
        };
        let Some(hw) = hw else {
            return Slot::Null;
        };
        let (base_array, last_array) = if family == FAMILY_SCARLETT {
            (d[4] >> 16 & 0x1fff, d[4] & 0xffff)
        } else {
            (d[5] & 0x1fff, d[5] >> 13 & 0x1fff)
        };
        return Slot::Image(Image {
            va,
            hw,
            ty,
            base_level: d[3] >> 12 & 0xf,
            last_level: d[3] >> 16 & 0xf,
            base_array,
            last_array,
            dst_sel: sel(d[3]),
            min_lod: (d[1] >> 8 & 0xfff) as f32 / 256.0,
        });
    }
    if d[3] >> 30 != 0 {
        return Slot::Null;
    }
    if family == FAMILY_SCARLETT && d[1] >> 16 == 0 && d[2..].iter().all(|&w| w == 0) {
        return Slot::Accel(d[0] as u64 | (d[1] as u64) << 32);
    }
    let va = d[0] as u64 | ((d[1] & 0xffff) as u64) << 32;
    let hw = if family == FAMILY_SCARLETT {
        Some(d[3] >> 12 & 0x7f).filter(|&h| h != 0)
    } else {
        hw_from_gcn(d[3] >> 15 & 0xf, d[3] >> 12 & 7, 128)
    };
    Slot::Buffer(Buffer { va, stride: d[1] >> 16 & 0x3fff, records: d[2], hw, dst_sel: sel(d[3]) })
}

const DEPTH: [u32; 4] = [20, 40, 45, 55];
const TYPELESS: [u32; 22] = [1, 5, 9, 15, 19, 23, 27, 33, 39, 44, 48, 53, 60, 70, 73, 76, 79, 82, 90, 92, 94, 97];

pub fn dxgi_candidates(hw: u32, buffer: bool) -> Vec<u32> {
    let col = if buffer { 1 } else { 0 };
    DXGI_TO_HW.iter().enumerate().filter(|(i, e)| e[col] as u32 == hw && !TYPELESS.contains(&(*i as u32)) && !DEPTH.contains(&(*i as u32))).map(|(i, _)| i as u32).collect()
}

pub fn group(f: u32) -> u32 {
    const G: [(u32, u32); 24] = [
        (1, 4), (5, 8), (9, 14), (15, 18), (19, 22), (23, 25), (27, 32), (33, 38), (39, 43), (44, 47), (48, 52), (53, 59),
        (60, 64), (70, 72), (73, 75), (76, 78), (79, 81), (82, 84), (94, 96), (97, 99), (87, 87), (88, 88), (90, 91), (92, 93),
    ];
    let f2 = match f {
        87 => 90,
        88 => 92,
        x => x,
    };
    G.iter().find(|(a, b)| (*a..=*b).contains(&f2)).map(|g| g.0).unwrap_or(f)
}

pub fn pick_format(cands: &[u32], resource_format: u32) -> Option<u32> {
    if cands.contains(&resource_format) {
        return Some(resource_format);
    }
    let g = group(resource_format);
    cands.iter().copied().find(|&c| group(c) == g).or_else(|| cands.first().copied())
}

pub fn component_order(dxgi: u32) -> [u32; 4] {
    match dxgi {
        87 | 88 | 90 | 91 | 92 | 93 => [2, 1, 0, 3],
        _ => [0, 1, 2, 3],
    }
}

pub fn mapping(dst_sel: [u32; 4], dxgi: u32) -> u32 {
    let order = component_order(dxgi);
    let mut m = 1 << 12;
    for (j, s) in dst_sel.iter().enumerate() {
        let v = match s {
            0 => 4,
            1 => 5,
            4..=7 => order[(*s - 4) as usize],
            _ => 4,
        };
        m |= v << (3 * j);
    }
    m
}

pub fn element_bytes(dxgi: u32) -> Option<u32> {
    Some(match dxgi {
        2..=4 => 16,
        6..=8 => 12,
        10..=14 | 16..=18 => 8,
        24..=26 | 28..=32 | 34..=38 | 40..=43 | 87 | 88 | 91 | 93 => 4,
        49..=52 | 54..=59 => 2,
        61..=65 => 1,
        _ => return None,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct ResInfo {
    pub dimension: u32,
    pub format: u32,
    pub depth_or_array: u32,
    pub mips: u32,
}

fn put32(b: &mut [u8], o: usize, v: u32) {
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}

fn put64(b: &mut [u8], o: usize, v: u64) {
    b[o..o + 8].copy_from_slice(&v.to_le_bytes());
}

pub fn null_view(uav: bool) -> [u8; 40] {
    let mut b = [0u8; 40];
    put32(&mut b, 0, 28);
    put32(&mut b, 4, 4);
    if !uav {
        put32(&mut b, 8, DEFAULT_MAPPING);
        put32(&mut b, 20, 1);
    }
    b
}

pub fn image_view(img: &Image, res: &ResInfo, uav: bool) -> Option<[u8; 40]> {
    let fmt = pick_format(&dxgi_candidates(img.hw, false), res.format)?;
    let mut b = [0u8; 40];
    put32(&mut b, 0, fmt);
    let mips = img.last_level.saturating_sub(img.base_level) + 1;
    let first = img.base_array;
    let count = img.last_array.saturating_sub(first) + 1;
    let single = img.base_array == 0 && img.last_array == 0 && res.depth_or_array <= 1;
    let ty = match img.ty {
        13 if single => 9,
        15 if single => 14,
        t => t,
    };
    if uav {
        let (dim, fields): (u32, Vec<u32>) = match ty {
            8 => (2, vec![img.base_level]),
            12 => (3, vec![img.base_level, first, count]),
            9 => (4, vec![img.base_level, 0]),
            10 => (8, vec![img.base_level, 0, u32::MAX]),
            11 | 13 => (5, vec![img.base_level, first, count, 0]),
            _ => return None,
        };
        put32(&mut b, 4, dim);
        for (i, v) in fields.iter().enumerate() {
            put32(&mut b, 8 + 4 * i, *v);
        }
        return Some(b);
    }
    put32(&mut b, 8, mapping(img.dst_sel, fmt));
    let lod = img.min_lod.to_bits();
    let (dim, fields): (u32, Vec<u32>) = match ty {
        8 => (2, vec![img.base_level, mips, lod]),
        12 => (3, vec![img.base_level, mips, first, count, lod]),
        9 => (4, vec![img.base_level, mips, 0, lod]),
        13 => (5, vec![img.base_level, mips, first, count, 0, lod]),
        14 => (6, vec![]),
        15 => (7, vec![first, count]),
        10 => (8, vec![img.base_level, mips, lod]),
        11 if count <= 6 && first == 0 && res.depth_or_array <= 6 => (9, vec![img.base_level, mips, lod]),
        11 => (10, vec![img.base_level, mips, first, count / 6, lod]),
        _ => return None,
    };
    put32(&mut b, 4, dim);
    for (i, v) in fields.iter().enumerate() {
        put32(&mut b, 16 + 4 * i, *v);
    }
    Some(b)
}

pub fn buffer_view(buf: &Buffer, offset: u64, uav: bool) -> Option<[u8; 40]> {
    let typed = buf.hw.and_then(|h| dxgi_candidates(h, true).first().copied()).filter(|&f| buf.stride != 0 && element_bytes(f) == Some(buf.stride));
    let (fmt, first, num, stride, flags) = if buf.stride == 0 {
        if offset % 4 != 0 {
            return None;
        }
        (39, offset / 4, buf.records / 4, 0, 1)
    } else {
        if offset % buf.stride as u64 != 0 {
            return None;
        }
        match typed {
            Some(f) => (f, offset / buf.stride as u64, buf.records, 0, 0),
            None => (0, offset / buf.stride as u64, buf.records, buf.stride, 0),
        }
    };
    let mut b = [0u8; 40];
    put32(&mut b, 0, fmt);
    put32(&mut b, 4, 1);
    if uav {
        put64(&mut b, 8, first);
        put32(&mut b, 16, num);
        put32(&mut b, 20, stride);
        put32(&mut b, 32, flags);
    } else {
        put32(&mut b, 8, if typed.is_some() { mapping(buf.dst_sel, fmt) } else { DEFAULT_MAPPING });
        put64(&mut b, 16, first);
        put32(&mut b, 24, num);
        put32(&mut b, 28, stride);
        put32(&mut b, 32, flags);
    }
    Some(b)
}

pub fn accel_view(location: u64) -> [u8; 40] {
    let mut b = [0u8; 40];
    put32(&mut b, 4, 11);
    put32(&mut b, 8, DEFAULT_MAPPING);
    put64(&mut b, 16, location);
    b
}

pub fn cbv_size(buf: &Buffer) -> u32 {
    let bytes = if buf.stride == 0 { buf.records as u64 } else { buf.records as u64 * buf.stride as u64 };
    (bytes.min(65536) as u32 + 255) & !255
}

pub fn sampler_desc(d: &[u32; 4], custom: [f32; 4]) -> [u8; 52] {
    let addr = |c: u32| match c {
        0 => 1,
        1 => 2,
        2 => 3,
        3 => 5,
        4 | 6 => 4,
        _ => 5,
    };
    let xy_mag = d[2] >> 20 & 3;
    let xy_min = d[2] >> 22 & 3;
    let mip = d[2] >> 26 & 3;
    let cmp = d[0] >> 12 & 7;
    let mode = d[0] >> 29 & 3;
    let reduction = match mode {
        1 => 2,
        2 => 3,
        _ => (1..=6).contains(&cmp) as u32,
    };
    let aniso = xy_min >= 2 || xy_mag >= 2;
    let lin = |f: u32| (f == 1 || f == 3) as u32;
    let filter = if aniso { 0x55 | reduction << 7 } else { lin(xy_min) << 4 | lin(xy_mag) << 2 | (mip == 2) as u32 | reduction << 7 };
    let mut b = [0u8; 52];
    put32(&mut b, 0, filter);
    put32(&mut b, 4, addr(d[0] & 7));
    put32(&mut b, 8, addr(d[0] >> 3 & 7));
    put32(&mut b, 12, addr(d[0] >> 6 & 7));
    let bias = ((d[2] & 0x3fff) << 18) as i32 >> 18;
    put32(&mut b, 16, (bias as f32 / 256.0).to_bits());
    put32(&mut b, 20, if aniso { 1 << (d[0] >> 9 & 7).min(4) } else { 1 });
    put32(&mut b, 24, cmp + 1);
    let border: [f32; 4] = match d[3] >> 30 {
        1 => [0.0, 0.0, 0.0, 1.0],
        2 => [1.0, 1.0, 1.0, 1.0],
        3 => custom,
        _ => [0.0; 4],
    };
    for (i, v) in border.iter().enumerate() {
        put32(&mut b, 28 + 4 * i, v.to_bits());
    }
    put32(&mut b, 44, ((d[1] & 0xfff) as f32 / 256.0).to_bits());
    let max = d[1] >> 12 & 0xfff;
    put32(&mut b, 48, if max == 0xfff { f32::MAX.to_bits() } else { (max as f32 / 256.0).to_bits() });
    b
}

struct Heap {
    device: usize,
    memory: u64,
    slot0: u64,
    ty: u32,
    num: u32,
    family: u32,
    cpu: u64,
    gpu: u64,
    inc: u32,
    raw: Vec<u32>,
    written: Vec<u8>,
    kinds: Vec<u8>,
}

impl Heap {
    fn words(&self) -> usize {
        (console_stride(self.ty) / 4) as usize
    }

    fn kind_at(&self, i: usize, kind: u8) -> u8 {
        if self.ty == 1 {
            KIND_SAMPLER
        } else if kind == KIND_DEFAULT {
            self.kinds[i]
        } else {
            kind
        }
    }
}

static HEAPS: Mutex<Option<HashMap<usize, Heap>>> = Mutex::new(None);

fn heaps<R>(f: impl FnOnce(&mut HashMap<usize, Heap>) -> R) -> R {
    let mut g = HEAPS.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(HashMap::new))
}

unsafe fn vfn(o: usize, slot: usize) -> usize {
    *(*(o as *const *const usize)).add(slot)
}

unsafe fn handle_start(heap: usize, slot: usize) -> u64 {
    let f: unsafe extern "system" fn(usize, *mut u64) -> u64 = core::mem::transmute(vfn(heap, slot));
    let mut out = u64::MAX;
    let r = f(heap, &mut out);
    if out == u64::MAX {
        r
    } else {
        out
    }
}

unsafe fn release(o: usize) {
    let f: unsafe extern "system" fn(usize) -> u32 = core::mem::transmute(vfn(o, 2));
    f(o);
}

unsafe fn resource_info(res: usize) -> ResInfo {
    let f: unsafe extern "system" fn(usize, *mut [u8; 0x38]) -> *mut [u8; 0x38] = core::mem::transmute(vfn(res, 10));
    let mut d = [0u8; 0x38];
    f(res, &mut d);
    let u32at = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
    let u16at = |o: usize| u16::from_le_bytes(d[o..o + 2].try_into().unwrap()) as u32;
    ResInfo { dimension: u32at(0), format: u32at(0x20), depth_or_array: u16at(0x1c), mips: u16at(0x1e) }
}

pub fn first_slot(memory: u64, ty: u32) -> u64 {
    memory + if ty == 1 { SAMPLER_HEADER } else { 0 }
}

pub fn palette_entry(memory: u64, header_num: u32, ptr: u32) -> u64 {
    memory + ((header_num as u64 * 16 + 0x10 + 0xff) & !0xff) + ptr as u64 * 16
}

pub unsafe fn register(heap: usize, memory: u64, desc: u64, family: u32) -> i32 {
    if heap == 0 || memory == 0 {
        return E_INVALIDARG;
    }
    let get_device: unsafe extern "system" fn(usize, *const [u8; 16], *mut usize) -> i32 = core::mem::transmute(vfn(heap, 7));
    let mut device = 0usize;
    let hr = get_device(heap, &IID_ID3D12_DEVICE, &mut device);
    if hr < 0 || device == 0 {
        return hr;
    }
    release(device);
    let ty = crate::xone_heap::heap_type(desc as u32);
    let num = (desc >> 32) as u32;
    let inc_f: unsafe extern "system" fn(usize, u32) -> u32 = core::mem::transmute(vfn(device, 15));
    let inc = inc_f(device, ty);
    let cpu = handle_start(heap, 9);
    let gpu = handle_start(heap, 10);
    let kind = if ty == 1 { KIND_SAMPLER } else { KIND_SRV };
    let slot0 = first_slot(memory, ty);
    let words = (console_stride(ty) / 4) as usize;
    heaps(|m| {
        m.insert(heap, Heap { device, memory, slot0, ty, num, family, cpu, gpu, inc, raw: vec![0; num as usize * words], written: vec![0; num as usize], kinds: vec![kind; num as usize] });
    });
    record(M, TAG, &format!("heap {heap:#x} type={ty} num={num} over title memory {memory:#x} (slot 0 at {slot0:#x}, console stride {}) -> PC cpu {cpu:#x} gpu {gpu:#x} inc {inc}", console_stride(ty)));
    0
}

pub fn forget(heap: usize) -> bool {
    heaps(|m| m.remove(&heap).is_some())
}

pub fn set_kind(heap: usize, first: u32, count: u32, kind: u8) -> u32 {
    heaps(|m| {
        let Some(h) = m.get_mut(&heap) else {
            return 0;
        };
        let end = (first as u64 + count as u64).min(h.num as u64) as usize;
        let mut n = 0;
        for i in first as usize..end {
            h.kinds[i] = kind;
            n += 1;
        }
        n
    })
}

fn once(bit: u32) -> bool {
    static SEEN: AtomicUsize = AtomicUsize::new(0);
    SEEN.fetch_or(1 << bit, Ordering::Relaxed) & (1 << bit) == 0
}

unsafe fn pc_va(res: usize) -> u64 {
    let va: unsafe extern "system" fn(usize) -> u64 = core::mem::transmute(vfn(res, 11));
    va(res)
}

unsafe fn read_f32x4(addr: u64) -> [f32; 4] {
    let mut c = [0f32; 4];
    for (i, v) in c.iter_mut().enumerate() {
        *v = f32::from_bits(core::ptr::read_volatile((addr as *const u32).add(i)));
    }
    c
}

unsafe fn write_slot(h: &Heap, i: usize, words: &[u32], kind: u8) -> bool {
    let handle = h.cpu + i as u64 * h.inc as u64;
    let dev = h.device;
    if kind == KIND_SAMPLER {
        let f: unsafe extern "system" fn(usize, *const [u8; 52], u64) = core::mem::transmute(vfn(dev, 22));
        let w = [words[0], words[1], words[2], words[3]];
        let custom = if w[3] >> 30 == 3 { read_f32x4(palette_entry(h.memory, core::ptr::read_volatile(h.memory as *const u32), w[3] & 0xfff)) } else { [0.0; 4] };
        if w[0] >> 15 & 1 != 0 && once(0) {
            record(M, TAG, &format!("heap slot {i}: console sampler sets FORCE_UNNORMALIZED; D3D12_SAMPLER_DESC has no unnormalized-coordinate field, written normalized"));
        }
        f(dev, &sampler_desc(&w, custom), handle);
        return true;
    }
    let mut d = [0u32; 8];
    d[..words.len()].copy_from_slice(words);
    let slot = decode_slot(h.family, &d);
    let mut ok = true;
    if kind == KIND_CBV {
        let f: unsafe extern "system" fn(usize, *const [u8; 16], u64) = core::mem::transmute(vfn(dev, 17));
        let mut c = [0u8; 16];
        if let Slot::Buffer(b) = slot {
            match crate::gpu_va::lookup(b.va).filter(|(e, _)| e.buffer) {
                Some((e, off)) => {
                    put64(&mut c, 0, pc_va(e.res) + off);
                    put32(&mut c, 8, cbv_size(&b));
                }
                None => ok = false,
            }
        } else {
            ok = slot == Slot::Null;
        }
        f(dev, &c, handle);
        return ok;
    }
    let uav = kind == KIND_UAV;
    let (res, view) = match slot {
        Slot::Null => (0, null_view(uav)),
        Slot::Accel(va) => match crate::gpu_va::lookup(va).filter(|(e, _)| e.buffer) {
            Some((e, off)) if !uav => (0, accel_view(pc_va(e.res) + off)),
            _ => {
                ok = false;
                (0, null_view(uav))
            }
        },
        Slot::Image(img) => match crate::gpu_va::lookup(img.va).filter(|(e, _)| !e.buffer) {
            Some((e, 0)) => match image_view(&img, &resource_info(e.res), uav) {
                Some(v) => (e.res, v),
                None => {
                    ok = false;
                    (0, null_view(uav))
                }
            },
            _ => {
                ok = false;
                (0, null_view(uav))
            }
        },
        Slot::Buffer(b) => match crate::gpu_va::lookup(b.va).filter(|(e, _)| e.buffer).and_then(|(e, off)| buffer_view(&b, off, uav).map(|v| (e.res, v))) {
            Some(x) => x,
            None => {
                ok = false;
                (0, null_view(uav))
            }
        },
    };
    if uav {
        let f: unsafe extern "system" fn(usize, usize, usize, *const [u8; 40], u64) = core::mem::transmute(vfn(dev, 19));
        f(dev, res, 0, &view, handle);
    } else {
        let f: unsafe extern "system" fn(usize, usize, *const [u8; 40], u64) = core::mem::transmute(vfn(dev, 18));
        f(dev, res, &view, handle);
    }
    ok
}

unsafe fn sync_range(h: &mut Heap, first: usize, end: usize, kind: u8, failed: &mut Vec<(usize, Vec<u32>)>) -> u32 {
    let w = h.words();
    let end = end.min(h.num as usize);
    let mut buf = [0u32; CHUNK * 8];
    let mut written = 0;
    let mut i = first;
    while i < end {
        let n = (end - i).min(CHUNK - i % CHUNK);
        core::ptr::copy_nonoverlapping((h.slot0 + i as u64 * w as u64 * 4) as *const u32, buf.as_mut_ptr(), n * w);
        let clean = buf[..n * w] == h.raw[i * w..(i + n) * w] && (i..i + n).all(|j| h.written[j] == h.kind_at(j, kind));
        if !clean {
            for j in 0..n {
                let s = i + j;
                let want = h.kind_at(s, kind);
                let words = &buf[j * w..(j + 1) * w];
                if h.written[s] == want && h.raw[s * w..(s + 1) * w] == *words {
                    continue;
                }
                if !write_slot(h, s, words, want) {
                    failed.push((s, words.to_vec()));
                }
                h.raw[s * w..(s + 1) * w].copy_from_slice(words);
                h.written[s] = want;
                written += 1;
            }
        }
        i += n;
    }
    written
}

pub unsafe fn sync_ranges(heap: usize, ranges: &[(u32, u32, u8)]) -> u32 {
    let mut written = 0;
    let mut failed = Vec::new();
    heaps(|m| {
        let Some(h) = m.get_mut(&heap) else {
            return;
        };
        for &(a, b, k) in ranges {
            written += sync_range(h, a as usize, b as usize, k, &mut failed);
        }
    });
    for (i, w) in failed.iter().take(4) {
        record(M, TAG, &format!("heap {heap:#x} slot {i}: console descriptor {w:08x?} has no PC resource/format match; null view written"));
    }
    written
}

pub unsafe fn sync(heap: usize, first: u32, count: u32) -> u32 {
    sync_ranges(heap, &[(first, (first as u64 + count as u64).min(u32::MAX as u64) as u32, KIND_DEFAULT)])
}

pub fn locate(handle: u64) -> Option<(usize, u32, u64, u32)> {
    let hit = heaps(|m| {
        m.iter().find_map(|(k, h)| {
            let stride = console_stride(h.ty);
            let len = h.num as u64 * stride;
            (handle >= h.slot0 && handle < h.slot0 + len).then(|| (*k, ((handle - h.slot0) / stride) as u32, (handle - h.slot0) % stride, h.gpu, h.inc, h.ty))
        })
    });
    let (heap, index, rem, gpu, inc, ty) = hit?;
    if rem != 0 {
        record(M, TAG, &format!("GPU handle {handle:#x} is {rem} bytes into console descriptor {index}"));
    }
    Some((heap, index, gpu + index as u64 * inc as u64, ty))
}

pub unsafe fn translate(handle: u64) -> u64 {
    match locate(handle) {
        Some((heap, index, pc, _)) => {
            sync(heap, index, u32::MAX);
            pc
        }
        None => handle,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TableRange {
    pub offset: u32,
    pub count: u32,
    pub kind: u8,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RootLayout {
    pub tables: Vec<Option<Vec<TableRange>>>,
    pub direct_resources: bool,
    pub direct_samplers: bool,
}

pub fn range_kind(range_type: u32) -> u8 {
    match range_type {
        0 => KIND_SRV,
        1 => KIND_UAV,
        2 => KIND_CBV,
        _ => KIND_SAMPLER,
    }
}

pub fn table_ranges(r: &[tombox_xshader::meta::RootRange]) -> Vec<TableRange> {
    let mut next = 0u32;
    r.iter()
        .map(|x| {
            let offset = if x.offset_in_table == u32::MAX { next } else { x.offset_in_table };
            next = if x.num == u32::MAX { u32::MAX } else { offset.saturating_add(x.num) };
            TableRange { offset, count: x.num, kind: range_kind(x.range_type) }
        })
        .collect()
}

pub fn layout_from_blob(blob: &[u8]) -> Option<RootLayout> {
    let body = if blob.get(0..4) == Some(b"DXBC") {
        let c = tombox_xshader::Dxbc::parse(blob).ok()?;
        let ch = c.chunk(b"RTS0")?;
        blob.get(ch.offset..ch.offset + ch.size)?
    } else {
        blob
    };
    let rs = tombox_xshader::meta::parse_rts0(body).ok()?;
    let tables = rs.params.iter().map(|p| match &p.kind {
        tombox_xshader::meta::RootParamKind::Table(r) => Some(table_ranges(r)),
        _ => None,
    });
    Some(RootLayout { tables: tables.collect(), direct_resources: rs.flags & ROOT_FLAG_DIRECT_RESOURCES != 0, direct_samplers: rs.flags & ROOT_FLAG_DIRECT_SAMPLERS != 0 })
}

static ROOTS: Mutex<Option<HashMap<usize, RootLayout>>> = Mutex::new(None);

fn roots<R>(f: impl FnOnce(&mut HashMap<usize, RootLayout>) -> R) -> R {
    let mut g = ROOTS.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(HashMap::new))
}

pub fn note_root_signature(rs: usize, blob: &[u8]) -> bool {
    if rs == 0 {
        return false;
    }
    match layout_from_blob(blob) {
        Some(l) => {
            roots(|m| m.insert(rs, l));
            true
        }
        None => {
            record(M, TAG, &format!("root signature {rs:#x}: blob ({} bytes) not parsed; its tables mirror to heap end with per-slot default view kinds", blob.len()));
            false
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Ref {
    pub heap: usize,
    pub first: u32,
    pub count: u32,
    pub kind: u8,
}

#[derive(Default)]
struct ListRefs {
    root: [Option<usize>; 2],
    heaps: Vec<usize>,
    refs: Vec<Ref>,
    seen: HashSet<Ref>,
    direct: [bool; 2],
}

static LISTS: Mutex<Option<HashMap<usize, ListRefs>>> = Mutex::new(None);

fn lists<R>(f: impl FnOnce(&mut HashMap<usize, ListRefs>) -> R) -> R {
    let mut g = LISTS.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(HashMap::new))
}

pub fn on_root(list: usize, side: usize, rs: usize) {
    let direct = roots(|m| m.get(&rs).map(|l| (l.direct_resources, l.direct_samplers)));
    lists(|m| {
        let s = m.entry(list).or_default();
        s.root[side] = (rs != 0).then_some(rs);
        if let Some((r, smp)) = direct {
            s.direct[0] |= r;
            s.direct[1] |= smp;
        }
    });
}

pub fn on_heaps(list: usize, hs: &[usize]) {
    lists(|m| m.entry(list).or_default().heaps = hs.to_vec());
}

pub fn on_reset(list: usize) {
    lists(|m| m.remove(&list));
}

pub fn on_clear_state(list: usize) {
    lists(|m| {
        if let Some(s) = m.get_mut(&list) {
            s.root = [None; 2];
            s.heaps.clear();
        }
    });
}

pub fn on_bundle(list: usize, bundle: usize) {
    lists(|m| {
        let Some(b) = m.get(&bundle) else {
            return;
        };
        let (refs, direct) = (b.refs.clone(), b.direct);
        let s = m.entry(list).or_default();
        for r in refs {
            if s.seen.insert(r) {
                s.refs.push(r);
            }
        }
        s.direct[0] |= direct[0];
        s.direct[1] |= direct[1];
    });
}

fn root_of(list: usize, side: usize) -> Option<usize> {
    let chain = crate::list_split::expand(&[list as *mut c_void]);
    lists(|m| chain.iter().rev().find_map(|l| m.get(&(*l as usize)).and_then(|s| s.root[side])))
}

pub fn on_table(list: usize, side: usize, param: u32, handle: u64) -> u64 {
    let Some((heap, index, pc, ty)) = locate(handle) else {
        return handle;
    };
    let rs = root_of(list, side);
    let ranges = rs.and_then(|rs| roots(|m| m.get(&rs).and_then(|l| l.tables.get(param as usize).cloned().flatten())));
    let new: Vec<Ref> = match ranges {
        Some(r) => r
            .iter()
            .map(|t| {
                let kind = match (ty == 1, t.kind == KIND_SAMPLER) {
                    (true, _) => KIND_SAMPLER,
                    (false, false) => t.kind,
                    (false, true) => {
                        if once(1) {
                            record(M, TAG, &format!("root parameter {param} declares a sampler range but its table {handle:#x} is in a CBV_SRV_UAV heap; per-slot default view kinds used"));
                        }
                        KIND_DEFAULT
                    }
                };
                Ref { heap, first: index.saturating_add(t.offset), count: t.count, kind }
            })
            .collect(),
        None => {
            if once(2) {
                record(M, TAG, &format!("descriptor table {handle:#x} (root parameter {param}) bound without a known root signature layout (rs {rs:x?}); mirrored to heap end with per-slot default view kinds"));
            }
            vec![Ref { heap, first: index, count: u32::MAX, kind: KIND_DEFAULT }]
        }
    };
    let bound = lists(|m| {
        let s = m.entry(list).or_default();
        for r in new {
            if s.seen.insert(r) {
                s.refs.push(r);
            }
        }
        s.heaps.clone()
    });
    if !table_heap_bound(&bound, heap) {
        UNBOUND_TABLES.fetch_add(1, Ordering::Relaxed);
        if once(20) {
            record(M, TAG, &format!("descriptor table {handle:#x} (root parameter {param}) is in heap {heap:#x}, which is not among the heaps bound on list {list:#x} ({bound:x?}); on PC the table reads whatever heap is bound"));
        }
    }
    pc
}

pub static UNBOUND_TABLES: AtomicUsize = AtomicUsize::new(0);

pub fn table_heap_bound(bound: &[usize], heap: usize) -> bool {
    bound.contains(&heap)
}

pub fn paint(iv: &[(u32, u32, u8)]) -> (Vec<(u32, u32, u8)>, u32) {
    let mut covered: BTreeMap<u32, (u32, u8)> = BTreeMap::new();
    let mut conflicts = 0;
    let order = iv.iter().rev().filter(|x| x.2 != KIND_DEFAULT).chain(iv.iter().rev().filter(|x| x.2 == KIND_DEFAULT));
    for &(a, b, k) in order {
        if a >= b {
            continue;
        }
        let over: Vec<(u32, u32, u8)> = covered.range(..b).rev().take_while(|(_, (e, _))| *e > a).map(|(s, (e, ck))| (*s, *e, *ck)).collect();
        let mut gaps = Vec::new();
        let mut cur = a;
        for &(s, e, ck) in over.iter().rev() {
            if k != KIND_DEFAULT && ck != k {
                conflicts += 1;
            }
            if s > cur {
                gaps.push((cur, s));
            }
            cur = cur.max(e);
        }
        if cur < b {
            gaps.push((cur, b));
        }
        for (x, y) in gaps {
            covered.insert(x, (y, k));
        }
    }
    (covered.into_iter().map(|(s, (e, k))| (s, e, k)).collect(), conflicts)
}

pub unsafe fn sync_for_execute(execute: &[*mut c_void]) -> u32 {
    let info: HashMap<usize, (u32, u32)> = heaps(|m| m.iter().map(|(k, h)| (*k, (h.num, h.ty))).collect());
    if info.is_empty() {
        return 0;
    }
    let mut per: Vec<(usize, Vec<(u32, u32, u8)>)> = Vec::new();
    let mut add = |r: Ref| {
        let Some(&(num, _)) = info.get(&r.heap) else {
            return;
        };
        let end = (r.first as u64 + r.count as u64).min(num as u64) as u32;
        if r.first >= end {
            return;
        }
        match per.iter_mut().find(|(h, _)| *h == r.heap) {
            Some((_, v)) => v.push((r.first, end, r.kind)),
            None => per.push((r.heap, vec![(r.first, end, r.kind)])),
        }
    };
    lists(|m| {
        for l in execute {
            let Some(s) = m.get(&(*l as usize)) else {
                continue;
            };
            for r in &s.refs {
                add(*r);
            }
            for &h in &s.heaps {
                if let Some(&(_, ty)) = info.get(&h) {
                    if (ty != 1 && s.direct[0]) || (ty == 1 && s.direct[1]) {
                        add(Ref { heap: h, first: 0, count: u32::MAX, kind: KIND_DEFAULT });
                    }
                }
            }
        }
    });
    let mut written = 0;
    for (heap, iv) in per {
        let mut seen = HashSet::new();
        let mut uniq: Vec<(u32, u32, u8)> = iv.into_iter().rev().filter(|x| seen.insert(*x)).collect();
        uniq.reverse();
        let (ranges, conflicts) = paint(&uniq);
        if conflicts != 0 && once(3) {
            record(M, TAG, &format!("heap {heap:#x}: {conflicts} slot ranges are bound as different view kinds by lists in one submission; the most recently recorded binding is mirrored"));
        }
        written += sync_ranges(heap, &ranges);
    }
    written
}

#[no_mangle]
pub unsafe extern "system" fn TomBoxTitleHeapRegister(heap: usize, memory: u64, desc: u64, family: u32) -> i32 {
    register(heap, memory, desc, family)
}

#[no_mangle]
pub unsafe extern "system" fn TomBoxTitleHeapForget(heap: usize) -> u32 {
    forget(heap) as u32
}

#[no_mangle]
pub unsafe extern "system" fn TomBoxTitleHeapSetKind(heap: usize, first: u32, count: u32, kind: u32) -> u32 {
    set_kind(heap, first, count, kind as u8)
}

#[no_mangle]
pub unsafe extern "system" fn TomBoxTitleHeapSync(heap: usize, first: u32, count: u32) -> u32 {
    sync(heap, first, count)
}

#[no_mangle]
pub unsafe extern "system" fn TomBoxTitleHeapTranslate(handle: u64) -> u64 {
    translate(handle)
}

const S_RESET: usize = 10;
const S_CLEAR_STATE: usize = 11;
const S_BUNDLE: usize = 27;
const S_HEAPS: usize = 32;
const S_CROOT: usize = 33;
const S_GROOT: usize = 34;
const S_CTABLE: usize = 35;
const S_GTABLE: usize = 36;

static NEXT: [AtomicUsize; 37] = [const { AtomicUsize::new(0) }; 37];

fn next(slot: usize) -> usize {
    NEXT[slot].load(Ordering::Relaxed)
}

unsafe extern "system" fn h_reset(list: *mut c_void, alloc: *mut c_void, pso: *mut c_void) -> i32 {
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32 = core::mem::transmute(next(S_RESET));
    let hr = f(list, alloc, pso);
    if hr >= 0 {
        on_reset(list as usize);
    }
    hr
}

unsafe extern "system" fn h_clear_state(list: *mut c_void, pso: *mut c_void) {
    on_clear_state(list as usize);
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void) = core::mem::transmute(next(S_CLEAR_STATE));
    f(list, pso)
}

unsafe extern "system" fn h_bundle(list: *mut c_void, bundle: *mut c_void) {
    if !bundle.is_null() {
        on_bundle(list as usize, crate::cmdlist_wrap::unwrap(bundle) as usize);
    }
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void) = core::mem::transmute(next(S_BUNDLE));
    f(list, bundle)
}

unsafe extern "system" fn h_set_heaps(list: *mut c_void, n: u32, p: *const usize) {
    let hs: Vec<usize> = if p.is_null() { Vec::new() } else { (0..n as usize).map(|i| *p.add(i)).collect() };
    on_heaps(list as usize, &hs);
    let f: unsafe extern "system" fn(*mut c_void, u32, *const usize) = core::mem::transmute(next(S_HEAPS));
    f(list, n, p)
}

unsafe extern "system" fn h_croot(list: *mut c_void, rs: *mut c_void) {
    on_root(list as usize, 0, rs as usize);
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void) = core::mem::transmute(next(S_CROOT));
    f(list, rs)
}

unsafe extern "system" fn h_groot(list: *mut c_void, rs: *mut c_void) {
    on_root(list as usize, 1, rs as usize);
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void) = core::mem::transmute(next(S_GROOT));
    f(list, rs)
}

unsafe extern "system" fn h_cmp_table(list: *mut c_void, i: u32, h: u64) {
    let f: unsafe extern "system" fn(*mut c_void, u32, u64) = core::mem::transmute(next(S_CTABLE));
    f(list, i, on_table(list as usize, 0, i, h))
}

unsafe extern "system" fn h_gfx_table(list: *mut c_void, i: u32, h: u64) {
    let f: unsafe extern "system" fn(*mut c_void, u32, u64) = core::mem::transmute(next(S_GTABLE));
    f(list, i, on_table(list as usize, 1, i, h))
}

pub fn install(vt: &[AtomicUsize]) {
    let plan = [
        (S_RESET, h_reset as usize),
        (S_CLEAR_STATE, h_clear_state as usize),
        (S_BUNDLE, h_bundle as usize),
        (S_HEAPS, h_set_heaps as usize),
        (S_CROOT, h_croot as usize),
        (S_GROOT, h_groot as usize),
        (S_CTABLE, h_cmp_table as usize),
        (S_GTABLE, h_gfx_table as usize),
    ];
    for (slot, hook) in plan {
        let prev = vt[slot].load(Ordering::Relaxed);
        if prev == 0 || prev == hook {
            continue;
        }
        NEXT[slot].store(prev, Ordering::Relaxed);
        vt[slot].store(hook, Ordering::Relaxed);
    }
}

static DEV_RS: AtomicUsize = AtomicUsize::new(0);

unsafe extern "system" fn create_root_signature(this: *mut c_void, mask: u32, blob: *const u8, len: usize, riid: *const c_void, ppv: *mut *mut c_void) -> i32 {
    let f: unsafe extern "system" fn(*mut c_void, u32, *const u8, usize, *const c_void, *mut *mut c_void) -> i32 = core::mem::transmute(DEV_RS.load(Ordering::Relaxed));
    let hr = f(this, mask, blob, len, riid, ppv);
    if hr >= 0 && !ppv.is_null() && !(*ppv).is_null() && !blob.is_null() && len != 0 {
        note_root_signature(*ppv as usize, core::slice::from_raw_parts(blob, len));
    }
    hr
}

pub fn hook_device(v: &mut [usize]) {
    if v.len() <= 16 || v[16] == 0 || v[16] == create_root_signature as usize {
        return;
    }
    DEV_RS.store(v[16], Ordering::Relaxed);
    v[16] = create_root_signature as usize;
}

pub static HW_TO_GCN: [[u8; 2]; 288] = [[0, 4], [1, 0], [1, 1], [1, 2], [1, 3], [1, 4], [1, 5], [2, 0], [2, 1], [2, 2], [2, 3], [2, 4], [2, 5], [2, 7], [3, 0], [3, 1], [3, 2], [3, 3], [3, 4], [3, 5], [4, 4], [4, 5], [4, 7], [5, 0], [5, 1], [5, 2], [5, 3], [5, 4], [5, 5], [5, 7], [6, 0], [6, 1], [6, 2], [6, 3], [6, 4], [6, 5], [6, 7], [7, 0], [7, 1], [7, 2], [7, 3], [7, 4], [7, 5], [7, 7], [8, 0], [8, 1], [8, 2], [8, 3], [8, 4], [8, 5], [9, 0], [9, 1], [9, 2], [9, 3], [9, 4], [9, 5], [10, 0], [10, 1], [10, 2], [10, 3], [10, 4], [10, 5], [11, 4], [11, 5], [11, 7], [12, 0], [12, 1], [12, 2], [12, 3], [12, 4], [12, 5], [12, 7], [13, 4], [13, 5], [13, 7], [14, 4], [14, 5], [14, 7], [0, 4], [0, 4], [0, 4], [0, 4], [0, 4], [0, 4], [0, 4], [0, 4], [0, 4], [0, 4], [0, 4], [0, 4], [0, 4], [0, 4], [0, 4], [0, 4], [0, 4], [0, 4], [0, 4], [0, 4], [0, 4], [0, 4], [0, 4], [0, 4], [0, 4], [0, 4], [0, 4], [0, 4], [0, 4], [0, 4], [0, 4], [0, 4], [0, 4], [0, 4], [0, 4], [0, 4], [0, 4], [0, 4], [0, 4], [0, 4], [0, 4], [0, 4], [0, 4], [0, 4], [0, 4], [0, 4], [0, 4], [0, 4], [0, 4], [0, 4], [1, 9], [3, 9], [10, 9], [31, 7], [34, 7], [16, 0], [17, 0], [18, 0], [19, 0], [57, 0], [59, 0], [60, 0], [4, 7], [20, 0], [20, 4], [21, 0], [21, 4], [22, 4], [22, 7], [32, 0], [32, 1], [32, 4], [32, 9], [33, 0], [33, 1], [33, 4], [33, 9], [0, 4], [44, 4], [45, 4], [46, 4], [47, 4], [48, 4], [49, 4], [50, 4], [51, 4], [52, 4], [53, 4], [54, 4], [55, 4], [56, 4], [35, 0], [35, 9], [36, 0], [36, 9], [37, 0], [37, 9], [38, 0], [38, 1], [39, 0], [39, 1], [40, 0], [40, 1], [41, 0], [41, 9], [64, 0], [64, 9], [65, 0], [65, 9], [66, 0], [66, 1], [67, 0], [67, 1], [68, 0], [68, 9], [69, 0], [69, 1], [69, 2], [69, 3], [69, 4], [69, 5], [69, 6], [69, 7], [69, 8], [69, 9], [69, 10], [69, 11], [69, 12], [69, 13], [70, 0], [70, 1], [70, 2], [70, 3], [70, 4], [70, 5], [70, 6], [70, 7], [70, 8], [70, 9], [70, 10], [70, 11], [70, 12], [70, 13], [71, 0], [71, 1], [71, 2], [71, 3], [71, 4], [71, 5], [71, 6], [71, 7], [71, 8], [71, 9], [71, 10], [71, 11], [71, 12], [71, 13], [72, 0], [72, 1], [72, 2], [72, 3], [72, 4], [72, 5], [72, 6], [72, 7], [72, 8], [72, 9], [73, 0], [73, 1], [73, 2], [73, 3], [73, 4], [73, 5], [73, 6], [73, 7], [73, 8], [73, 9], [74, 0], [74, 1], [74, 2], [74, 3], [74, 4], [74, 5], [74, 6], [74, 7], [74, 8], [74, 9], [76, 0], [76, 4], [77, 0], [77, 4], [78, 0], [78, 4], [79, 0], [79, 4], [80, 0], [80, 4], [81, 0], [81, 4], [82, 0], [82, 4], [83, 0], [83, 4], [84, 0], [84, 4], [85, 0], [85, 4], [30, 7], [29, 0], [29, 9]];
pub static DXGI_TO_HW: [[u16; 2]; 193] = [[0, 0], [75, 0], [77, 77], [75, 75], [76, 76], [72, 0], [74, 74], [72, 72], [73, 73], [69, 0], [71, 71], [65, 65], [69, 69], [66, 66], [70, 70], [62, 0], [64, 64], [62, 62], [63, 63], [145, 0], [146, 0], [146, 0], [145, 0], [54, 0], [50, 50], [54, 54], [36, 36], [60, 0], [56, 56], [130, 0], [60, 60], [57, 57], [61, 61], [27, 0], [29, 29], [23, 23], [27, 27], [24, 24], [28, 28], [20, 0], [22, 22], [22, 22], [20, 20], [21, 21], [142, 0], [141, 0], [141, 0], [142, 0], [18, 0], [14, 14], [18, 18], [15, 15], [19, 19], [11, 0], [13, 13], [7, 7], [7, 7], [11, 11], [8, 8], [12, 12], [5, 0], [1, 1], [5, 5], [2, 2], [6, 6], [1, 1], [1, 0], [132, 0], [147, 0], [151, 0], [169, 0], [169, 0], [170, 0], [171, 0], [171, 0], [172, 0], [173, 0], [173, 0], [174, 0], [175, 0], [175, 0], [176, 0], [177, 0], [177, 0], [178, 0], [133, 0], [134, 0], [56, 56], [56, 56], [50, 50], [60, 0], [130, 0], [60, 0], [130, 0], [179, 0], [179, 0], [180, 0], [181, 0], [181, 0], [182, 0], [56, 0], [50, 0], [65, 0], [1, 0], [7, 0], [0, 0], [0, 0], [151, 0], [0, 0], [0, 0], [0, 0], [1, 0], [1, 0], [1, 1], [14, 14], [136, 0], [0, 0], [131, 0], [7, 0], [7, 0], [5, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [0, 0], [51, 51], [137, 0], [141, 0], [5, 5]];

#[cfg(test)]
mod tests {
    use super::*;

    fn t_img(family: u32, va: u64, hw_or_df: u32, nf: u32, ty: u32, base: u32, last: u32, sel: [u32; 4]) -> [u32; 8] {
        let a = (va >> 8) as u32;
        let hi = ((va >> 40) & 0xff) as u32;
        let s = sel[0] | sel[1] << 3 | sel[2] << 6 | sel[3] << 9;
        let fmt = if family == FAMILY_SCARLETT { hw_or_df << 20 } else { hw_or_df << 20 | nf << 26 };
        [a, hi | fmt, 0, s | base << 12 | last << 16 | ty << 28, 0, 0, 0, 0]
    }

    #[test]
    fn format_tables_from_console_driver() {
        assert_eq!(DXGI_TO_HW[28], [56, 56]);
        assert_eq!(DXGI_TO_HW[29], [130, 0]);
        assert_eq!(DXGI_TO_HW[2], [77, 77]);
        assert_eq!(HW_TO_GCN[56], [10, 0]);
        assert_eq!(HW_TO_GCN[130], [10, 9]);
        assert_eq!(dxgi_candidates(56, false), vec![28, 87, 88, 100]);
        assert_eq!(hw_from_gcn(10, 0, 288), Some(56));
        assert_eq!(hw_from_gcn(10, 9, 288), Some(130));
        assert_eq!(pick_format(&dxgi_candidates(56, false), 87), Some(87));
        assert_eq!(pick_format(&dxgi_candidates(56, false), 27), Some(28));
        assert_eq!(pick_format(&dxgi_candidates(130, false), 90), Some(91));
        assert_eq!(pick_format(&dxgi_candidates(22, false), 39), Some(41));
    }

    #[test]
    fn image_descriptors_both_families() {
        let va = 0x12_3456_7800u64;
        let d = t_img(FAMILY_SCARLETT, va, 56, 0, 9, 1, 3, [6, 5, 4, 7]);
        let Slot::Image(i) = decode_slot(FAMILY_SCARLETT, &d) else { panic!() };
        assert_eq!((i.va, i.hw, i.ty, i.base_level, i.last_level), (va, 56, 9, 1, 3));
        let g = t_img(FAMILY_ONE, va, 10, 0, 9, 1, 3, [6, 5, 4, 7]);
        assert_eq!(decode_slot(FAMILY_ONE, &g), Slot::Image(i));
        let res = ResInfo { dimension: 3, format: 87, depth_or_array: 1, mips: 4 };
        let v = image_view(&i, &res, false).unwrap();
        let w = |o: usize| u32::from_le_bytes(v[o..o + 4].try_into().unwrap());
        assert_eq!((w(0), w(4), w(16), w(20)), (87, 4, 1, 3));
        assert_eq!(w(8), 0 | 1 << 3 | 2 << 6 | 3 << 9 | 1 << 12);
        let rgba = ResInfo { format: 28, ..res };
        let v = image_view(&i, &rgba, false).unwrap();
        assert_eq!(u32::from_le_bytes(v[8..12].try_into().unwrap()), 2 | 1 << 3 | 0 << 6 | 3 << 9 | 1 << 12);
        let mut arr = t_img(FAMILY_SCARLETT, va, 56, 0, 13, 0, 0, [4, 5, 6, 7]);
        arr[4] = 2 << 16 | 5;
        let Slot::Image(a) = decode_slot(FAMILY_SCARLETT, &arr) else { panic!() };
        let v = image_view(&a, &ResInfo { format: 28, ..res }, true).unwrap();
        let w = |o: usize| u32::from_le_bytes(v[o..o + 4].try_into().unwrap());
        assert_eq!((w(0), w(4), w(8), w(12), w(16)), (28, 5, 0, 2, 4));
        assert_eq!(decode_slot(FAMILY_SCARLETT, &[0; 8]), Slot::Null);
        assert_eq!(decode_slot(FAMILY_SCARLETT, &t_img(FAMILY_SCARLETT, va, 0, 0, 9, 0, 0, [4, 5, 6, 7])), Slot::Null);
    }

    #[test]
    fn buffer_descriptors() {
        let d = [0x1000, 16 << 16 | 0x12, 100, 77 << 12 | 0x0fac, 0, 0, 0, 0];
        let Slot::Buffer(b) = decode_slot(FAMILY_SCARLETT, &d) else { panic!() };
        assert_eq!((b.va, b.stride, b.records, b.hw), (0x12_0000_1000, 16, 100, Some(77)));
        let v = buffer_view(&b, 32, false).unwrap();
        let w = |o: usize| u32::from_le_bytes(v[o..o + 4].try_into().unwrap());
        assert_eq!((w(0), w(4), w(16), w(24), w(28), w(32)), (2, 1, 2, 100, 0, 0));
        let s = Buffer { stride: 24, hw: None, ..b };
        let v = buffer_view(&s, 48, true).unwrap();
        let w = |o: usize| u32::from_le_bytes(v[o..o + 4].try_into().unwrap());
        assert_eq!((w(0), w(4), w(8), w(16), w(20)), (0, 1, 2, 100, 24));
        let r = Buffer { stride: 0, records: 64, ..b };
        let v = buffer_view(&r, 8, false).unwrap();
        let w = |o: usize| u32::from_le_bytes(v[o..o + 4].try_into().unwrap());
        assert_eq!((w(0), w(16), w(24), w(32)), (39, 2, 16, 1));
        assert!(buffer_view(&r, 6, false).is_none());
        assert_eq!(cbv_size(&r), 256);
        let g = [0x1000, 4 << 16, 10, (4 << 15) | (7 << 12) | 0xfac, 0, 0, 0, 0];
        let Slot::Buffer(gb) = decode_slot(FAMILY_ONE, &g) else { panic!() };
        assert_eq!(gb.hw.and_then(|h| dxgi_candidates(h, true).first().copied()), Some(41));
    }


    #[test]
    fn sampler_words() {
        let d = [2 | 6 << 3 | 0 << 6 | 3 << 9 | 4 << 12, 0x10 | 0xfff << 12, 1 << 20 | 1 << 22 | 2 << 26 | (0x3fff & (-128i32 as u32)), 2 << 30];
        let s = sampler_desc(&d, [0.0; 4]);
        let w = |o: usize| u32::from_le_bytes(s[o..o + 4].try_into().unwrap());
        let f = |o: usize| f32::from_bits(w(o));
        assert_eq!((w(0), w(4), w(8), w(12)), (0x95, 3, 4, 1));
        assert_eq!((f(16), w(20), w(24)), (-0.5, 1, 5));
        assert_eq!((f(28), f(40), f(44), f(48)), (1.0, 1.0, 0.0625, f32::MAX));
        let a = [3 << 9, 0, 3 << 20 | 3 << 22 | 2 << 26, 0];
        let s = sampler_desc(&a, [0.0; 4]);
        assert_eq!((u32::from_le_bytes(s[0..4].try_into().unwrap()), u32::from_le_bytes(s[20..24].try_into().unwrap())), (0x55, 8));
    }

    #[test]
    fn sampler_compare_and_custom_border_follow_the_console_encoder() {
        let w = |s: &[u8; 52], o: usize| u32::from_le_bytes(s[o..o + 4].try_into().unwrap());
        let always = sampler_desc(&[7 << 12, 0, 1 << 20 | 1 << 22 | 2 << 26, 0], [0.0; 4]);
        assert_eq!((w(&always, 0), w(&always, 24)), (0x15, 8));
        let never = sampler_desc(&[0, 0, 0, 0], [0.0; 4]);
        assert_eq!((w(&never, 0), w(&never, 24)), (0, 1));
        let less_equal = sampler_desc(&[3 << 12, 0, 1 << 20 | 1 << 22, 0], [0.0; 4]);
        assert_eq!((w(&less_equal, 0), w(&less_equal, 24)), (0x94, 4));
        let minmax = sampler_desc(&[3 << 12 | 2 << 29, 0, 0, 0], [0.0; 4]);
        assert_eq!(w(&minmax, 0), 0x180);
        let custom = sampler_desc(&[0, 0, 0, 3 << 30 | 5], [0.25, 0.5, 0.75, 1.0]);
        assert_eq!((0..4).map(|i| f32::from_bits(w(&custom, 28 + i * 4))).collect::<Vec<_>>(), vec![0.25, 0.5, 0.75, 1.0]);
        assert_eq!(first_slot(0x1_0000, 1), 0x1_0010);
        assert_eq!(first_slot(0x1_0000, 0), 0x1_0000);
        assert_eq!(palette_entry(0x1_0000, 2, 1), 0x1_0000 + 0x100 + 0x10);
        assert_eq!(palette_entry(0x1_0000, 15, 0), 0x1_0000 + 0x100);
        assert_eq!(palette_entry(0x1_0000, 16, 3), 0x1_0000 + 0x200 + 0x30);
    }

    #[test]
    fn scarlett_single_slice_type13_is_a_2d_view_and_accel_srv_decodes() {
        let va = 0x12_3456_7800u64;
        let d = t_img(FAMILY_SCARLETT, va, 56, 0, 13, 0, 2, [4, 5, 6, 7]);
        let Slot::Image(i) = decode_slot(FAMILY_SCARLETT, &d) else { panic!() };
        let res = ResInfo { dimension: 3, format: 28, depth_or_array: 1, mips: 3 };
        let w = |v: &[u8; 40], o: usize| u32::from_le_bytes(v[o..o + 4].try_into().unwrap());
        let v = image_view(&i, &res, false).unwrap();
        assert_eq!((w(&v, 4), w(&v, 16), w(&v, 20), w(&v, 24)), (4, 0, 3, 0));
        let u = image_view(&i, &res, true).unwrap();
        assert_eq!((w(&u, 4), w(&u, 8), w(&u, 12)), (4, 0, 0));
        let arr = image_view(&i, &ResInfo { depth_or_array: 4, ..res }, false).unwrap();
        assert_eq!((w(&arr, 4), w(&arr, 24), w(&arr, 28)), (5, 0, 1));
        let ms = t_img(FAMILY_SCARLETT, va, 56, 0, 15, 0, 2, [4, 5, 6, 7]);
        let Slot::Image(m) = decode_slot(FAMILY_SCARLETT, &ms) else { panic!() };
        assert_eq!(w(&image_view(&m, &res, false).unwrap(), 4), 6);
        let accel = [0x4000_0000, 0x12, 0, 0, 0, 0, 0, 0];
        assert_eq!(decode_slot(FAMILY_SCARLETT, &accel), Slot::Accel(0x12_4000_0000));
        assert!(matches!(decode_slot(FAMILY_ONE, &accel), Slot::Buffer(_)));
        let a = accel_view(0x7700_1000);
        assert_eq!((w(&a, 0), w(&a, 4), w(&a, 8)), (0, 11, DEFAULT_MAPPING));
        assert_eq!(u64::from_le_bytes(a[16..24].try_into().unwrap()), 0x7700_1000);
    }

    fn rts0(flags: u32, ranges: &[(u32, u32, u32)]) -> Vec<u8> {
        let mut b = Vec::new();
        let mut p = |v: u32| b.extend_from_slice(&v.to_le_bytes());
        let ranges_off = 56u32;
        let consts_off = ranges_off + 24 * ranges.len() as u32;
        for v in [2, 2, 24, 0, consts_off + 12, flags] {
            p(v);
        }
        for v in [0, 0, 48, 1, 0, consts_off] {
            p(v);
        }
        p(ranges.len() as u32);
        p(ranges_off);
        for &(ty, num, off) in ranges {
            for v in [ty, num, 0, 0, 0, off] {
                p(v);
            }
        }
        for v in [0, 0, 4] {
            p(v);
        }
        b
    }

    #[test]
    fn root_layout_gives_kind_and_window_per_table_offset() {
        let l = layout_from_blob(&rts0(0x400, &[(2, 1, 0), (1, 2, u32::MAX), (0, u32::MAX, u32::MAX)])).unwrap();
        assert_eq!(l.tables.len(), 2);
        assert_eq!(l.tables[1], None);
        assert_eq!(
            l.tables[0].clone().unwrap(),
            vec![TableRange { offset: 0, count: 1, kind: KIND_CBV }, TableRange { offset: 1, count: 2, kind: KIND_UAV }, TableRange { offset: 3, count: u32::MAX, kind: KIND_SRV }]
        );
        assert!(l.direct_resources && !l.direct_samplers);
        let s = layout_from_blob(&rts0(0x800, &[(3, 4, 8)])).unwrap();
        assert_eq!(s.tables[0].clone().unwrap(), vec![TableRange { offset: 8, count: 4, kind: KIND_SAMPLER }]);
        assert!(s.direct_samplers);
        assert!(layout_from_blob(&[1, 2, 3]).is_none());
        assert!(!note_root_signature(0x99, &[1, 2, 3]));
    }

    #[test]
    fn paint_resolves_overlaps_last_binding_wins_and_default_fills_gaps() {
        assert_eq!(paint(&[(0, 4, KIND_SRV), (2, 6, KIND_SRV)]), (vec![(0, 2, KIND_SRV), (2, 6, KIND_SRV)], 0));
        assert_eq!(paint(&[(0, 4, KIND_UAV), (2, 3, KIND_SRV)]), (vec![(0, 2, KIND_UAV), (2, 3, KIND_SRV), (3, 4, KIND_UAV)], 1));
        assert_eq!(paint(&[(2, 3, KIND_SRV), (0, 10, KIND_DEFAULT), (5, 6, KIND_CBV)]), (vec![(0, 2, 0), (2, 3, KIND_SRV), (3, 5, 0), (5, 6, KIND_CBV), (6, 10, 0)], 0));
        assert_eq!(paint(&[(3, 3, KIND_SRV)]), (vec![], 0));
    }

    #[repr(C)]
    struct Fake {
        vt: *const usize,
    }

    static CALLS: Mutex<Vec<(usize, usize, Vec<u8>, u64)>> = Mutex::new(Vec::new());
    static SERIAL: Mutex<()> = Mutex::new(());

    unsafe extern "system" fn f_release(_: usize) -> u32 {
        1
    }
    unsafe extern "system" fn f_get_device(_: usize, _: *const [u8; 16], out: *mut usize) -> i32 {
        *out = &**FAKE_DEVICE as *const Fake as usize;
        0
    }
    unsafe extern "system" fn f_cpu(_: usize, out: *mut u64) -> u64 {
        *out = 0x10_0000;
        out as u64
    }
    unsafe extern "system" fn f_gpu(_: usize, out: *mut u64) -> u64 {
        *out = 0xa000_0000;
        out as u64
    }
    unsafe extern "system" fn f_inc(_: usize, _: u32) -> u32 {
        64
    }
    unsafe extern "system" fn f_srv(_: usize, res: usize, d: *const [u8; 40], h: u64) {
        CALLS.lock().unwrap().push((18, res, (*d).to_vec(), h));
    }
    unsafe extern "system" fn f_uav(_: usize, res: usize, _: usize, d: *const [u8; 40], h: u64) {
        CALLS.lock().unwrap().push((19, res, (*d).to_vec(), h));
    }
    unsafe extern "system" fn f_cbv(_: usize, d: *const [u8; 16], h: u64) {
        CALLS.lock().unwrap().push((17, 0, (*d).to_vec(), h));
    }
    unsafe extern "system" fn f_sampler(_: usize, d: *const [u8; 52], h: u64) {
        CALLS.lock().unwrap().push((22, 0, (*d).to_vec(), h));
    }
    unsafe extern "system" fn f_res_desc(_: usize, out: *mut [u8; 0x38]) -> *mut [u8; 0x38] {
        let mut d = [0u8; 0x38];
        d[0] = 3;
        d[0x1c] = 1;
        d[0x1e] = 4;
        d[0x20..0x24].copy_from_slice(&28u32.to_le_bytes());
        *out = d;
        out
    }
    unsafe extern "system" fn f_res_va(_: usize) -> u64 {
        0x7700_0000
    }

    fn vt(entries: &[(usize, usize)]) -> *const usize {
        let mut v = vec![0usize; 24];
        v[2] = f_release as usize;
        for &(i, f) in entries {
            v[i] = f;
        }
        Box::leak(v.into_boxed_slice()).as_ptr()
    }

    static FAKE_DEVICE: std::sync::LazyLock<Box<Fake>> = std::sync::LazyLock::new(|| {
        Box::new(Fake { vt: vt(&[(15, f_inc as usize), (17, f_cbv as usize), (18, f_srv as usize), (19, f_uav as usize), (22, f_sampler as usize)]) })
    });

    unsafe impl Sync for Fake {}
    unsafe impl Send for Fake {}

    fn fake_heap() -> usize {
        Box::leak(Box::new(Fake { vt: vt(&[(7, f_get_device as usize), (9, f_cpu as usize), (10, f_gpu as usize)]) })) as *const Fake as usize
    }

    fn calls() -> Vec<(usize, usize, Vec<u8>, u64)> {
        core::mem::take(&mut *CALLS.lock().unwrap())
    }

    #[test]
    fn descriptors_mirror_at_execute_with_root_signature_kinds_and_windows() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let heap = fake_heap();
        let tex = Box::leak(Box::new(Fake { vt: vt(&[(10, f_res_desc as usize)]) })) as *const Fake as usize;
        let buf = Box::leak(Box::new(Fake { vt: vt(&[(11, f_res_va as usize)]) })) as *const Fake as usize;
        let tex_va = 0x55_0000_0000u64;
        let buf_va = 0x56_0000_0000u64;
        crate::gpu_va::insert(crate::gpu_va::Entry { start: tex_va, len: 0x10000, res: tex, buffer: false });
        crate::gpu_va::insert(crate::gpu_va::Entry { start: buf_va, len: 0x10000, res: buf, buffer: true });
        let mut mem = vec![[0u32; 8]; 8];
        let memory = mem.as_mut_ptr() as u64;
        let rs = 0x5150_0010usize;
        let rs_all_srv = 0x5150_0020usize;
        let rs_direct = 0x5150_0030usize;
        let (l1, l2, l3, l4, bundle, l6) = (0x7171_0010usize, 0x7171_0020, 0x7171_0030, 0x7171_0040, 0x7171_0050, 0x7171_0060);
        let ex = |l: &[usize]| unsafe { sync_for_execute(&l.iter().map(|&x| x as *mut c_void).collect::<Vec<_>>()) };
        unsafe {
            assert_eq!(register(heap, memory, 8u64 << 32, FAMILY_SCARLETT), 0);
            assert!(note_root_signature(rs, &rts0(0, &[(2, 1, 0), (1, 1, u32::MAX), (0, u32::MAX, u32::MAX)])));
            assert!(note_root_signature(rs_all_srv, &rts0(0, &[(0, u32::MAX, 0)])));
            assert!(note_root_signature(rs_direct, &rts0(0x400, &[])));
            calls();
            on_heaps(l1, &[heap]);
            on_root(l1, 0, rs);
            assert_eq!(on_table(l1, 0, 0, memory + 32), 0xa000_0000 + 64);
            assert!(calls().is_empty());
            mem[1] = [(buf_va + 0x100) as u32, (buf_va >> 32) as u32 | 16 << 16, 32, 0, 0, 0, 0, 0];
            mem[2] = [buf_va as u32, (buf_va >> 32) as u32 | 4 << 16, 32, 22 << 12, 0, 0, 0, 0];
            mem[3] = t_img(FAMILY_SCARLETT, tex_va, 56, 0, 9, 0, 0, [4, 5, 6, 7]);
            assert_eq!(ex(&[l1]), 7);
            let c = calls();
            assert_eq!((c[0].0, c[0].3), (17, 0x10_0000 + 64));
            assert_eq!(u64::from_le_bytes(c[0].2[0..8].try_into().unwrap()), 0x7700_0100);
            assert_eq!(u32::from_le_bytes(c[0].2[8..12].try_into().unwrap()), 512);
            assert_eq!((c[1].0, c[1].1, c[1].3), (19, buf, 0x10_0000 + 128));
            assert_eq!(u32::from_le_bytes(c[1].2[0..4].try_into().unwrap()), 41);
            assert_eq!((c[2].0, c[2].1, c[2].3), (18, tex, 0x10_0000 + 192));
            assert_eq!(u32::from_le_bytes(c[2].2[0..4].try_into().unwrap()), 28);
            assert!(c[3..].iter().all(|x| x.0 == 18 && x.1 == 0));
            assert_eq!(c.last().unwrap().3, 0x10_0000 + 7 * 64);
            assert_eq!(ex(&[l1]), 0);
            mem[5][0] = 0x1234;
            mem[5][3] = 0x8000_0000 | 9 << 28;
            assert_eq!(ex(&[l1]), 1);
            assert_eq!(calls()[0].1, 0);
            on_heaps(l2, &[heap]);
            on_root(l2, 1, rs_all_srv);
            on_table(l2, 1, 0, memory + 32);
            assert_eq!(ex(&[l1, l2]), 2);
            assert!(calls().iter().all(|x| x.0 == 18));
            assert_eq!(ex(&[l1]), 2);
            assert_eq!(calls().iter().map(|x| x.0).collect::<Vec<_>>(), vec![17, 19]);
            on_reset(l1);
            mem[4][0] = 0x77;
            mem[4][3] = 9 << 28;
            assert_eq!(ex(&[l1]), 0);
            assert_eq!(on_table(l3, 0, 2, memory + 4 * 32), 0xa000_0000 + 4 * 64);
            set_kind(heap, 4, 1, KIND_UAV);
            assert_eq!(ex(&[l3]), 1);
            assert_eq!(calls()[0].0, 19);
            on_heaps(l4, &[heap]);
            on_root(l4, 1, rs_direct);
            mem[0][0] = 0x99;
            mem[0][3] = 9 << 28;
            assert_eq!(ex(&[l4]), 3);
            assert_eq!(calls().iter().map(|x| (x.0, x.3)).collect::<Vec<_>>(), vec![(18, 0x10_0000), (18, 0x10_0000 + 64), (18, 0x10_0000 + 128)]);
            on_root(bundle, 0, rs);
            on_table(bundle, 0, 0, memory + 6 * 32);
            on_bundle(l6, bundle);
            mem[7][0] = 0x55;
            mem[7][3] = 9 << 28;
            assert_eq!(ex(&[l6]), 2);
            assert_eq!(calls().iter().map(|x| (x.0, x.3)).collect::<Vec<_>>(), vec![(17, 0x10_0000 + 6 * 64), (19, 0x10_0000 + 7 * 64)]);
            assert_eq!(locate(memory + 8 * 32), None);
            assert_eq!(translate(0x40), 0x40);
            assert!(forget(heap));
            assert_eq!(ex(&[l6]), 0);
            assert_eq!(register(0, memory, 0, 0), E_INVALIDARG);
        }
    }

    #[test]
    fn sampler_heap_slots_start_after_the_0x10_header_and_read_the_border_palette() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let sh = fake_heap();
        let mut smem = vec![[0u32; 4]; 18];
        let base = smem.as_mut_ptr() as u64;
        smem[0][0] = 2;
        smem[1] = [2 << 3, 0, 1 << 20, 0];
        smem[2] = [0, 0, 1 << 20, 3 << 30 | 1];
        smem[17] = [0.25f32.to_bits(), 0.5f32.to_bits(), 0.75f32.to_bits(), 1.0f32.to_bits()];
        unsafe {
            assert_eq!(register(sh, base, 1 | 2u64 << 32, FAMILY_ONE), 0);
            assert_eq!(locate(base), None);
            assert_eq!(locate(base + 0x10 + 16).map(|x| (x.1, x.2)), Some((1, 0xa000_0000 + 64)));
            let list = 0x7272_0010usize;
            assert_eq!(on_table(list, 0, 0, base + 0x10), 0xa000_0000);
            calls();
            assert_eq!(sync_for_execute(&[list as *mut c_void]), 2);
            let c = calls();
            assert_eq!(c.iter().map(|x| (x.0, x.3)).collect::<Vec<_>>(), vec![(22, 0x10_0000), (22, 0x10_0000 + 64)]);
            let f = |o: usize| f32::from_bits(u32::from_le_bytes(c[1].2[o..o + 4].try_into().unwrap()));
            assert_eq!((f(28), f(32), f(36), f(40)), (0.25, 0.5, 0.75, 1.0));
            assert_eq!(u32::from_le_bytes(c[0].2[8..12].try_into().unwrap()), 3);
            assert!(forget(sh));
        }
    }

    unsafe extern "system" fn next_create_rs(_: *mut c_void, _: u32, _: *const u8, _: usize, _: *const c_void, ppv: *mut *mut c_void) -> i32 {
        *ppv = 0x5150_0040 as *mut c_void;
        0
    }

    #[test]
    fn device_hook_records_root_signature_layouts() {
        let mut v = vec![0usize; 20];
        v[16] = next_create_rs as usize;
        hook_device(&mut v);
        hook_device(&mut v);
        assert_eq!(v[16], create_root_signature as usize);
        assert_eq!(DEV_RS.load(Ordering::Relaxed), next_create_rs as usize);
        let blob = rts0(0, &[(1, 3, 0)]);
        let mut out: *mut c_void = core::ptr::null_mut();
        let f: unsafe extern "system" fn(*mut c_void, u32, *const u8, usize, *const c_void, *mut *mut c_void) -> i32 = unsafe { core::mem::transmute(v[16]) };
        assert_eq!(unsafe { f(core::ptr::null_mut(), 0, blob.as_ptr(), blob.len(), core::ptr::null(), &mut out) }, 0);
        assert_eq!(roots(|m| m.get(&0x5150_0040).cloned()).unwrap().tables[0].clone().unwrap(), vec![TableRange { offset: 0, count: 3, kind: KIND_UAV }]);
    }

    static NEXT_SEEN: Mutex<Vec<(usize, u64)>> = Mutex::new(Vec::new());

    unsafe extern "system" fn n_table(_: *mut c_void, i: u32, h: u64) {
        NEXT_SEEN.lock().unwrap().push((i as usize, h));
    }
    unsafe extern "system" fn n_one(_: *mut c_void, p: *mut c_void) {
        NEXT_SEEN.lock().unwrap().push((1000, p as u64));
    }
    unsafe extern "system" fn n_heaps(_: *mut c_void, n: u32, _: *const usize) {
        NEXT_SEEN.lock().unwrap().push((2000, n as u64));
    }
    unsafe extern "system" fn n_reset(_: *mut c_void, _: *mut c_void, _: *mut c_void) -> i32 {
        NEXT_SEEN.lock().unwrap().push((3000, 0));
        0
    }

    #[test]
    fn install_chains_every_binding_slot() {
        let saved: Vec<usize> = NEXT.iter().map(|a| a.load(Ordering::Relaxed)).collect();
        let vt: Vec<AtomicUsize> = (0..64).map(|_| AtomicUsize::new(0)).collect();
        for s in [S_CROOT, S_GROOT, S_CLEAR_STATE, S_BUNDLE] {
            vt[s].store(n_one as usize, Ordering::Relaxed);
        }
        vt[S_CTABLE].store(n_table as usize, Ordering::Relaxed);
        vt[S_GTABLE].store(n_table as usize, Ordering::Relaxed);
        vt[S_HEAPS].store(n_heaps as usize, Ordering::Relaxed);
        vt[S_RESET].store(n_reset as usize, Ordering::Relaxed);
        install(&vt);
        install(&vt);
        let hooked = [
            (S_RESET, h_reset as usize),
            (S_CLEAR_STATE, h_clear_state as usize),
            (S_BUNDLE, h_bundle as usize),
            (S_HEAPS, h_set_heaps as usize),
            (S_CROOT, h_croot as usize),
            (S_GROOT, h_groot as usize),
            (S_CTABLE, h_cmp_table as usize),
            (S_GTABLE, h_gfx_table as usize),
        ];
        for (s, h) in hooked {
            assert_eq!(vt[s].load(Ordering::Relaxed), h);
        }
        assert_eq!(NEXT[S_GTABLE].load(Ordering::Relaxed), n_table as usize);
        let list = 0x7373_0010usize as *mut c_void;
        unsafe {
            let call = |s: usize| vt[s].load(Ordering::Relaxed);
            let t: unsafe extern "system" fn(*mut c_void, u32, u64) = core::mem::transmute(call(S_GTABLE));
            t(list, 2, 0x1234);
            let r: unsafe extern "system" fn(*mut c_void, *mut c_void) = core::mem::transmute(call(S_GROOT));
            r(list, 0x5150_0099 as *mut c_void);
            let hp: unsafe extern "system" fn(*mut c_void, u32, *const usize) = core::mem::transmute(call(S_HEAPS));
            let hs = [0x4444usize];
            hp(list, 1, hs.as_ptr());
            assert_eq!(lists(|m| m.get(&(list as usize)).map(|s| (s.root[1], s.heaps.clone()))), Some((Some(0x5150_0099), vec![0x4444])));
            let rs: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32 = core::mem::transmute(call(S_RESET));
            assert_eq!(rs(list, core::ptr::null_mut(), core::ptr::null_mut()), 0);
            assert!(lists(|m| !m.contains_key(&(list as usize))));
        }
        for (a, v) in NEXT.iter().zip(saved) {
            if v != 0 {
                a.store(v, Ordering::Relaxed);
            }
        }
        assert_eq!(*NEXT_SEEN.lock().unwrap(), vec![(2, 0x1234), (1000, 0x5150_0099), (2000, 1), (3000, 0)]);
    }

}

#[cfg(test)]
#[path = "title_heap_hw.rs"]
mod hw_tests;
