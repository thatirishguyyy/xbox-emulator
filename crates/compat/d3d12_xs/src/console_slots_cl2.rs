use core::ffi::c_void;
use core::sync::atomic::{AtomicUsize, Ordering};
use std::collections::HashMap;
use std::sync::Mutex;
use compat_common::record;

pub const CL_COPY_RESOURCE_X: usize = 84;
pub const CL_GET_EXECUTION_COMMAND_SIZE_X: usize = 91;
pub const CL_CLOSE_BUNDLE_X: usize = 92;
pub const CL_SET_GRAPHICS_SHADER_LIMITS_X: usize = 95;
pub const CL_PREFETCH_PIPELINE_STATE_X: usize = 99;
pub const CL_TRANSCODE_TEXTURE_REGION_X: usize = 103;

const PC_SLOT_CLOSE: usize = 9;
const PC_SLOT_COPY_TEXTURE_REGION: usize = 16;
const PC_SLOT_COPY_RESOURCE: usize = 17;
const RES_SLOT_GET_DESC: usize = 10;
const LIST_TYPE_BUNDLE: u32 = 1;

pub const COPY_X_VALID_FLAGS: u32 = 0x1fff;
pub const COPY_X_RESUMMARIZE_HTILE: u32 = 0x10;
pub const COPY_X_FORCE_DECOMPRESS: u32 = 0x60;
pub const COPY_X_COLOR_METADATA: u32 = 0x1c00;
pub const COPY_X_COMPRESSED_SOURCE_MODES: u32 = 0x380;

pub const BUNDLE_RECORD_SIZE: u32 = 16;
pub const BUNDLE_RECORD_MAGIC: u32 = u32::from_le_bytes(*b"TBnd");

pub const PREFETCH_PSO_STAGE_MASK: u32 = 0x7f;
pub const PREFETCH_PSO_MAX_BYTES: u64 = 0x20_0000;

const DIM_BUFFER: u32 = 1;
const DIM_TEXTURE3D: u32 = 4;
const LAYOUT_UNKNOWN: u32 = 0;
const COPY_TYPE_SUBRESOURCE_INDEX: u32 = 0;
const COPY_TYPE_PLACED_FOOTPRINT: u32 = 1;

fn once(c: &AtomicUsize, n: usize) -> bool {
    c.fetch_add(1, Ordering::Relaxed) < n
}

unsafe fn pc_fn(obj: *mut c_void, slot: usize) -> usize {
    unsafe { *(*(obj as *const *const usize)).add(slot) }
}

pub fn fill64_pattern(value: u64) -> [u8; 16] {
    let hi = ((value >> 32) as u32).to_le_bytes();
    let lo = (value as u32).to_le_bytes();
    core::array::from_fn(|i| if (i >> 2) & 1 == 0 { hi[i & 3] } else { lo[i & 3] })
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Desc {
    pub dim: u32,
    pub width: u64,
    pub height: u32,
    pub depth_or_array: u16,
    pub mips: u16,
    pub format: u32,
    pub layout: u32,
}

impl Desc {
    pub fn from_bytes(d: &[u8; 56]) -> Desc {
        let u32_at = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
        Desc {
            dim: u32_at(0),
            width: u64::from_le_bytes(d[16..24].try_into().unwrap()),
            height: u32_at(24),
            depth_or_array: u16::from_le_bytes(d[28..30].try_into().unwrap()),
            mips: u16::from_le_bytes(d[30..32].try_into().unwrap()).max(1),
            format: u32_at(32),
            layout: u32_at(44),
        }
    }

    pub fn subresources(&self) -> u32 {
        if self.dim == DIM_BUFFER {
            return 1;
        }
        let arr = if self.dim == DIM_TEXTURE3D { 1 } else { self.depth_or_array.max(1) as u32 };
        arr * self.mips as u32
    }

    pub fn extent(&self) -> (u64, u32, u32) {
        let d = if self.dim == DIM_TEXTURE3D { self.depth_or_array.max(1) as u32 } else { 1 };
        (self.width, self.height.max(1), d)
    }

    pub fn mip_extent(&self, sub: u32) -> (u32, u32, u32) {
        let mip = sub % self.mips as u32;
        let (w, h, d) = self.extent();
        (((w >> mip) as u32).max(1), (h >> mip).max(1), (d >> mip).max(1))
    }

    pub fn is_opaque_texture(&self) -> bool {
        self.dim > DIM_BUFFER && self.dim <= DIM_TEXTURE3D && self.layout == LAYOUT_UNKNOWN
    }
}

unsafe fn res_desc(res: *mut c_void) -> Option<Desc> {
    if res.is_null() {
        return None;
    }
    let get: unsafe extern "system" fn(*mut c_void, *mut u8) -> *mut u8 = unsafe { core::mem::transmute(pc_fn(res, RES_SLOT_GET_DESC)) };
    let mut d = [0u8; 56];
    unsafe { get(res, d.as_mut_ptr()) };
    Some(Desc::from_bytes(&d))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CopyXPlan {
    Copy,
    InPlace,
}

pub fn copy_resource_x_plan(dst: Option<Desc>, src: Option<Desc>, same: bool, flags: u32) -> Result<CopyXPlan, &'static str> {
    let (Some(d), Some(s)) = (dst, src) else {
        return Err(if dst.is_none() { "pDstResource must not be NULL." } else { "pSrcResource must not be NULL." });
    };
    if flags & !COPY_X_VALID_FLAGS != 0 {
        return Err("Invalid flags.");
    }
    if d.dim != s.dim {
        return Err("Source and destination resources must have the same resource dimension.");
    }
    if flags & COPY_X_COMPRESSED_SOURCE_MODES == 0 && d.subresources() != s.subresources() {
        return Err("Source and destination resources must have the same subresource count.");
    }
    if d.extent() != s.extent() {
        return Err("Source and destination resources must have the same width, height, and depth.");
    }
    if flags & COPY_X_RESUMMARIZE_HTILE != 0 && !same {
        return Err("Source must equal destination resource for D3D12XBOX_COPY_FLAG_RESUMMARIZE_HTILE.");
    }
    if flags & COPY_X_COLOR_METADATA != 0 && flags & COPY_X_FORCE_DECOMPRESS == 0 && !same {
        return Err("Source must equal destination resource for color metadata operations without D3D12XBOX_COPY_FLAG_FORCE_DECOMPRESS.");
    }
    if same {
        return Ok(CopyXPlan::InPlace);
    }
    if d.subresources() != s.subresources() {
        return Err("PC CopyResource needs equal subresource counts; compressed-source copy mode has no PC form.");
    }
    Ok(CopyXPlan::Copy)
}

static COPYX_LOG: AtomicUsize = AtomicUsize::new(0);

unsafe extern "system" fn cl_copy_resource_x(this: *mut c_void, dst: *mut c_void, src: *mut c_void, flags: u32) {
    let plan = copy_resource_x_plan(unsafe { res_desc(dst) }, unsafe { res_desc(src) }, dst == src, flags);
    match plan {
        Err(why) => {
            if once(&COPYX_LOG, 8) {
                record("d3d12_xs", "console_slots_cl2", &format!("CopyResourceX dst={dst:p} src={src:p} flags={flags:#x}: {why} (console validation); not copied"));
            }
        }
        Ok(CopyXPlan::InPlace) => {
            if once(&COPYX_LOG, 8) {
                record("d3d12_xs", "console_slots_cl2", &format!("CopyResourceX {dst:p} in place flags={flags:#x}: compression-metadata operation; PC keeps metadata internal, texel contents unchanged"));
            }
        }
        Ok(CopyXPlan::Copy) => {
            crate::placed::note_gpu_write(dst);
            crate::res3d::note(dst as usize, "cl2 CopyResourceX", &format!("src={src:p} flags={flags:#x}"));
            let f: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) = unsafe { core::mem::transmute(pc_fn(this, PC_SLOT_COPY_RESOURCE)) };
            unsafe { f(this, dst, src) };
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BundleRecord {
    pub magic: u32,
    pub size: u32,
    pub bundle: u64,
}

struct BundleTables {
    by_buffer: HashMap<usize, usize>,
    owned: HashMap<usize, Box<BundleRecord>>,
    failed: HashMap<usize, bool>,
}

static BUNDLES: Mutex<Option<BundleTables>> = Mutex::new(None);

fn with_bundles<R>(f: impl FnOnce(&mut BundleTables) -> R) -> R {
    let mut g = BUNDLES.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(|| BundleTables { by_buffer: HashMap::new(), owned: HashMap::new(), failed: HashMap::new() }))
}

#[allow(dead_code)]
pub fn bundle_for_buffer(addr: usize) -> Option<usize> {
    with_bundles(|t| t.by_buffer.get(&addr).copied())
}

pub fn live_bundles() -> Vec<usize> {
    with_bundles(|t| {
        let mut v: Vec<usize> = t.by_buffer.values().copied().filter(|b| !t.failed.get(b).copied().unwrap_or(false)).collect();
        v.sort_unstable();
        v.dedup();
        v
    })
}

pub fn forget_bundle(key: usize) {
    with_bundles(|t| {
        t.by_buffer.retain(|_, b| *b != key);
        t.owned.remove(&key);
        t.failed.remove(&key);
    });
}

pub fn execution_command_size(list_type: u32, failed: bool) -> u32 {
    if list_type != LIST_TYPE_BUNDLE || failed {
        return 0;
    }
    BUNDLE_RECORD_SIZE
}

unsafe extern "system" fn cl_get_execution_command_size_x(this: *mut c_void) -> u32 {
    let ty = unsafe { crate::fill::list_type(this) };
    let failed = with_bundles(|t| t.failed.get(&(this as usize)).copied().unwrap_or(false));
    execution_command_size(ty, failed)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CloseBundlePlan {
    NotBundle,
    BufferTooSmall,
    Close { title_buffer: bool },
}

pub fn close_bundle_plan(list_type: u32, title_buffer: bool, max_size: u32) -> CloseBundlePlan {
    if list_type != LIST_TYPE_BUNDLE {
        return CloseBundlePlan::NotBundle;
    }
    if title_buffer && max_size < BUNDLE_RECORD_SIZE {
        return CloseBundlePlan::BufferTooSmall;
    }
    CloseBundlePlan::Close { title_buffer }
}

static BUNDLE_LOG: AtomicUsize = AtomicUsize::new(0);

unsafe fn close_list(this: *mut c_void) -> i32 {
    let p = crate::cmdlist_wrap::vt_get(PC_SLOT_CLOSE);
    let p = if p != 0 { p } else { unsafe { pc_fn(this, PC_SLOT_CLOSE) } };
    let f: unsafe extern "system" fn(*mut c_void) -> i32 = unsafe { core::mem::transmute(p) };
    unsafe { f(this) }
}

unsafe extern "system" fn cl_close_bundle_x(this: *mut c_void, pp: *mut *mut c_void, max_size: u32) -> u32 {
    let ty = unsafe { crate::fill::list_type(this) };
    let title_buffer = !pp.is_null() && unsafe { !(*pp).is_null() };
    match close_bundle_plan(ty, title_buffer, max_size) {
        CloseBundlePlan::NotBundle => {
            if once(&BUNDLE_LOG, 8) {
                record("d3d12_xs", "console_slots_cl2", "CloseBundleX: the given command list is not a bundle (console returns 0)");
            }
            0
        }
        CloseBundlePlan::BufferTooSmall => {
            if once(&BUNDLE_LOG, 8) {
                record("d3d12_xs", "console_slots_cl2", &format!("CloseBundleX: BundleBufferMaxSize {max_size} is too small for the execution size {BUNDLE_RECORD_SIZE} (console validation)"));
            }
            0
        }
        CloseBundlePlan::Close { title_buffer } => {
            let hr = unsafe { close_list(this) };
            let key = this as usize;
            if hr < 0 {
                with_bundles(|t| { t.failed.insert(key, true); });
                if once(&BUNDLE_LOG, 8) {
                    record("d3d12_xs", "console_slots_cl2", &format!("CloseBundleX: PC Close failed {hr:#x}; size 0"));
                }
                return 0;
            }
            let rec = BundleRecord { magic: BUNDLE_RECORD_MAGIC, size: BUNDLE_RECORD_SIZE, bundle: key as u64 };
            let dest = with_bundles(|t| {
                t.failed.remove(&key);
                t.by_buffer.retain(|_, b| *b != key);
                if title_buffer {
                    t.owned.remove(&key);
                    let d = unsafe { *pp } as usize;
                    unsafe { core::ptr::write_unaligned(d as *mut BundleRecord, rec) };
                    t.by_buffer.insert(d, key);
                    d
                } else {
                    let b = t.owned.entry(key).or_insert_with(|| Box::new(rec));
                    **b = rec;
                    let d = &**b as *const BundleRecord as usize;
                    t.by_buffer.insert(d, key);
                    d
                }
            });
            if !pp.is_null() && !title_buffer {
                unsafe { *pp = dest as *mut c_void };
            }
            crate::indirect_bundle::note_close(key);
            BUNDLE_RECORD_SIZE
        }
    }
}

static LIMITS_LOG: AtomicUsize = AtomicUsize::new(0);

unsafe extern "system" fn cl_set_graphics_shader_limits_x(this: *mut c_void, ps: u32, tess: u32, gs: u32, gs_on_chip: u32, limits: *const c_void) {
    let ty = unsafe { crate::fill::list_type(this) };
    if ty == LIST_TYPE_BUNDLE {
        if once(&LIMITS_LOG, 8) {
            record("d3d12_xs", "console_slots_cl2", "SetGraphicsShaderLimitsX cannot be used in a bundle (console validation)");
        }
        return;
    }
    if once(&LIMITS_LOG, 4) {
        record("d3d12_xs", "console_slots_cl2", &format!("SetGraphicsShaderLimitsX ps={ps} tess={tess} gs={gs} gs_on_chip={gs_on_chip} limits={limits:p}: CU/wave scheduling limits, no architectural effect on PC"));
    }
}

pub fn prefetch_pso_check(pso_null: bool, size: u64, stage_flags: u32, flags: u32) -> Result<(), &'static str> {
    if pso_null {
        return Err("pPipelineState cannot be NULL.");
    }
    if stage_flags & !PREFETCH_PSO_STAGE_MASK != 0 {
        return Err("PipelineStateFlags contains invalid bits.");
    }
    if stage_flags == 0 {
        return Err("PipelineStateFlags must specify at least one shader stage.");
    }
    if size == 0 {
        return Err("ShaderPrefetchSizeBytes must be greater than 0.");
    }
    if size >= PREFETCH_PSO_MAX_BYTES {
        return Err("ShaderPrefetchSizeBytes must be less than 2097152 bytes.");
    }
    if size % 4 != 0 {
        return Err("ShaderPrefetchSizeBytes must be a multiple of 4.");
    }
    if flags != 0 {
        return Err("Flags contains invalid bits.");
    }
    Ok(())
}

static PREFETCH_PSO_LOG: AtomicUsize = AtomicUsize::new(0);

unsafe extern "system" fn cl_prefetch_pipeline_state_x(_this: *mut c_void, pso: *mut c_void, size: u64, stage_flags: u32, flags: u32) {
    if let Err(why) = prefetch_pso_check(pso.is_null(), size, stage_flags, flags) {
        if once(&PREFETCH_PSO_LOG, 8) {
            record("d3d12_xs", "console_slots_cl2", &format!("PrefetchPipelineStateX pso={pso:p} size={size:#x} stages={stage_flags:#x} flags={flags:#x}: {why} (console validation)"));
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TranscodeCopy {
    pub src_sub: u32,
    pub dst_sub: u32,
    pub dst_offset: u64,
    pub src_box: Option<[u32; 6]>,
}

pub fn transcode_plan(src: &Desc, src_sub: u32, dst_type: u32, dst_sub: u32, dst_mips: u32, dst_offset: u64, row_pitch: u32, fp_height: u32, slices: u32) -> Vec<TranscodeCopy> {
    if slices == 0 {
        return Vec::new();
    }
    if src.dim == DIM_TEXTURE3D {
        let (w, h, d) = src.mip_extent(src_sub);
        let bx = if slices < d { Some([0, 0, 0, w, h, slices]) } else { None };
        return vec![TranscodeCopy { src_sub, dst_sub, dst_offset, src_box: bx }];
    }
    let src_stride = src.mips as u32;
    let slice_bytes = row_pitch as u64 * fp_height.max(1) as u64;
    (0..slices)
        .map(|i| TranscodeCopy {
            src_sub: src_sub + i * src_stride,
            dst_sub: if dst_type == COPY_TYPE_SUBRESOURCE_INDEX { dst_sub + i * dst_mips.max(1) } else { dst_sub },
            dst_offset: if dst_type == COPY_TYPE_PLACED_FOOTPRINT { dst_offset + i as u64 * slice_bytes } else { dst_offset },
            src_box: None,
        })
        .take_while(|c| c.src_sub < src.subresources())
        .collect()
}

pub const PLACEMENT_ALIGNMENT: u64 = 512;

pub fn transcode_pc_ok(plan: &[TranscodeCopy], dst_type: u32, src_format: u32, dst_format: u32) -> Result<(), &'static str> {
    if src_format != dst_format {
        return Err("destination format differs from the opaque source; PC CopyTextureRegion cannot convert formats");
    }
    if dst_type == COPY_TYPE_PLACED_FOOTPRINT && plan.iter().skip(1).any(|c| c.dst_offset % PLACEMENT_ALIGNMENT != 0) {
        return Err("a later slice lands at a footprint offset that is not 512-byte aligned, which PC CopyTextureRegion rejects");
    }
    Ok(())
}

static TRANSCODE_LOG: AtomicUsize = AtomicUsize::new(0);

unsafe extern "system" fn cl_transcode_texture_region_x(this: *mut c_void, dst: *const u8, src: *const u8, slices: u32) {
    let note = |m: &str| {
        if once(&TRANSCODE_LOG, 8) {
            record("d3d12_xs", "console_slots_cl2", &format!("TranscodeTextureRegionX: {m}"));
        }
    };
    if dst.is_null() || src.is_null() {
        note(if dst.is_null() { "pDst cannot be NULL. (console validation)" } else { "pSrc cannot be NULL. (console validation)" });
        return;
    }
    let q = |p: *const u8, o: usize| unsafe { core::ptr::read_unaligned(p.add(o) as *const u64) };
    let d32 = |p: *const u8, o: usize| unsafe { core::ptr::read_unaligned(p.add(o) as *const u32) };
    let (src_res, src_type, dst_res, dst_type) = (q(src, 0) as *mut c_void, d32(src, 8), q(dst, 0) as *mut c_void, d32(dst, 8));
    if dst_res.is_null() {
        note("The destination copy location requires a non-null resource. (console validation)");
        return;
    }
    let src_desc = if src_type == COPY_TYPE_SUBRESOURCE_INDEX { unsafe { res_desc(src_res) } } else { None };
    let Some(sd) = src_desc.filter(|d| d.is_opaque_texture()) else {
        note("source is not an opaque texture subresource; the console transcodes only out of an opaque resource and records nothing otherwise");
        return;
    };
    let dst_desc = if dst_type == COPY_TYPE_SUBRESOURCE_INDEX { unsafe { res_desc(dst_res) } } else { None };
    let dst_mips = dst_desc.map(|d| d.mips as u32).unwrap_or(1);
    let dst_format = dst_desc.map(|d| d.format).unwrap_or_else(|| d32(dst, 24));
    let (dst_sub, dst_offset) = if dst_type == COPY_TYPE_SUBRESOURCE_INDEX { (d32(dst, 16), 0) } else { (0, q(dst, 16)) };
    let plan = transcode_plan(&sd, d32(src, 16), dst_type, dst_sub, dst_mips, dst_offset, d32(dst, 40), d32(dst, 32), slices);
    if plan.is_empty() {
        return;
    }
    if let Err(why) = transcode_pc_ok(&plan, dst_type, sd.format, dst_format) {
        note(&format!("{why}; nothing recorded"));
        return;
    }
    crate::placed::note_gpu_write(dst_res);
    let copy: unsafe extern "system" fn(*mut c_void, *const u8, u32, u32, u32, *const u8, *const u32) = unsafe { core::mem::transmute(pc_fn(this, PC_SLOT_COPY_TEXTURE_REGION)) };
    for c in plan {
        let mut dl = [0u8; 48];
        let mut sl = [0u8; 48];
        unsafe {
            core::ptr::copy_nonoverlapping(dst, dl.as_mut_ptr(), 48);
            core::ptr::copy_nonoverlapping(src, sl.as_mut_ptr(), 48);
        }
        sl[16..20].copy_from_slice(&c.src_sub.to_le_bytes());
        if dst_type == COPY_TYPE_SUBRESOURCE_INDEX {
            dl[16..20].copy_from_slice(&c.dst_sub.to_le_bytes());
        } else {
            dl[16..24].copy_from_slice(&c.dst_offset.to_le_bytes());
        }
        let bx = c.src_box;
        let bp = bx.as_ref().map(|b| b.as_ptr()).unwrap_or(core::ptr::null());
        unsafe { copy(this, dl.as_ptr(), 0, 0, 0, sl.as_ptr(), bp) };
    }
}

fn plan() -> Vec<(usize, usize)> {
    vec![
        (CL_COPY_RESOURCE_X, cl_copy_resource_x as usize),
        (CL_GET_EXECUTION_COMMAND_SIZE_X, cl_get_execution_command_size_x as usize),
        (CL_CLOSE_BUNDLE_X, cl_close_bundle_x as usize),
        (CL_SET_GRAPHICS_SHADER_LIMITS_X, cl_set_graphics_shader_limits_x as usize),
        (CL_PREFETCH_PIPELINE_STATE_X, cl_prefetch_pipeline_state_x as usize),
        (CL_TRANSCODE_TEXTURE_REGION_X, cl_transcode_texture_region_x as usize),
    ]
}

pub fn install_cmdlist(vt: &[AtomicUsize], defaults: &[usize]) -> usize {
    let mut n = 0;
    for (slot, f) in plan() {
        if slot < vt.len() && slot < defaults.len() && vt[slot].load(Ordering::Relaxed) == defaults[slot] {
            vt[slot].store(f, Ordering::Relaxed);
            n += 1;
        }
    }
    record("d3d12_xs", "console_slots_cl2", &format!("command-list slots filled from the console UMD map (cl2): {n}"));
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tex2d(w: u64, h: u32, arr: u16, mips: u16) -> Desc {
        Desc { dim: 3, width: w, height: h, depth_or_array: arr, mips, format: 28, layout: 0 }
    }

    #[test]
    fn fill64_writes_high_dword_first_like_the_console_filler() {
        let p = fill64_pattern(0x1122_3344_5566_7788);
        assert_eq!(&p[0..8], &[0x44, 0x33, 0x22, 0x11, 0x88, 0x77, 0x66, 0x55]);
        assert_eq!(&p[8..16], &p[0..8]);
        assert_eq!(fill64_pattern(0xffff_ffff_0000_0000)[0..4], [0xff; 4]);
    }

    #[test]
    fn desc_decodes_pc_resource_desc() {
        let mut b = [0u8; 56];
        b[0..4].copy_from_slice(&3u32.to_le_bytes());
        b[16..24].copy_from_slice(&256u64.to_le_bytes());
        b[24..28].copy_from_slice(&128u32.to_le_bytes());
        b[28..30].copy_from_slice(&6u16.to_le_bytes());
        b[30..32].copy_from_slice(&0u16.to_le_bytes());
        b[32..36].copy_from_slice(&28u32.to_le_bytes());
        b[44..48].copy_from_slice(&0u32.to_le_bytes());
        let d = Desc::from_bytes(&b);
        assert_eq!(d, tex2d(256, 128, 6, 1));
        assert_eq!(d.subresources(), 6);
        assert!(d.is_opaque_texture());
        assert!(!Desc { layout: 1, ..d }.is_opaque_texture());
        assert!(!Desc { dim: 1, ..d }.is_opaque_texture());
        let v = Desc { dim: 4, width: 64, height: 32, depth_or_array: 16, mips: 3, format: 28, layout: 0 };
        assert_eq!(v.subresources(), 3);
        assert_eq!(v.mip_extent(1), (32, 16, 8));
    }

    #[test]
    fn copy_resource_x_follows_debug_umd_validation() {
        let t = tex2d(64, 64, 1, 4);
        assert_eq!(copy_resource_x_plan(Some(t), Some(t), false, 0), Ok(CopyXPlan::Copy));
        assert_eq!(copy_resource_x_plan(Some(t), Some(t), true, COPY_X_RESUMMARIZE_HTILE), Ok(CopyXPlan::InPlace));
        assert!(copy_resource_x_plan(Some(t), Some(t), false, COPY_X_RESUMMARIZE_HTILE).is_err());
        assert!(copy_resource_x_plan(Some(t), Some(t), false, 0x400).is_err());
        assert_eq!(copy_resource_x_plan(Some(t), Some(t), false, 0x400 | 0x40), Ok(CopyXPlan::Copy));
        assert_eq!(copy_resource_x_plan(Some(t), Some(t), true, 0x1000), Ok(CopyXPlan::InPlace));
        assert_eq!(copy_resource_x_plan(Some(t), Some(t), false, 0x2000), Err("Invalid flags."));
        assert_eq!(copy_resource_x_plan(None, Some(t), false, 0), Err("pDstResource must not be NULL."));
        assert_eq!(copy_resource_x_plan(Some(t), None, false, 0), Err("pSrcResource must not be NULL."));
        assert!(copy_resource_x_plan(Some(t), Some(tex2d(64, 32, 1, 4)), false, 0).is_err());
        assert!(copy_resource_x_plan(Some(t), Some(tex2d(64, 64, 1, 2)), false, 0).is_err());
        assert!(copy_resource_x_plan(Some(t), Some(Desc { dim: 4, ..t }), false, 0).is_err());
        assert!(copy_resource_x_plan(Some(t), Some(tex2d(64, 64, 1, 2)), false, 0x80).is_err());
    }

    #[test]
    fn bundle_size_and_close_rules_match_release_umd() {
        assert_eq!(execution_command_size(0, false), 0);
        assert_eq!(execution_command_size(2, false), 0);
        assert_eq!(execution_command_size(1, true), 0);
        assert_eq!(execution_command_size(1, false), BUNDLE_RECORD_SIZE);
        assert_eq!(close_bundle_plan(0, false, 0), CloseBundlePlan::NotBundle);
        assert_eq!(close_bundle_plan(1, true, 8), CloseBundlePlan::BufferTooSmall);
        assert_eq!(close_bundle_plan(1, true, 16), CloseBundlePlan::Close { title_buffer: true });
        assert_eq!(close_bundle_plan(1, false, 0), CloseBundlePlan::Close { title_buffer: false });
        assert_eq!(core::mem::size_of::<BundleRecord>(), BUNDLE_RECORD_SIZE as usize);
        assert_eq!(&BUNDLE_RECORD_MAGIC.to_le_bytes(), b"TBnd");
    }

    #[test]
    fn prefetch_pipeline_state_validation_matches_debug_umd() {
        assert_eq!(prefetch_pso_check(false, 0x100, 1, 0), Ok(()));
        assert_eq!(prefetch_pso_check(false, 0x1f_fffc, 0x7e, 0), Ok(()));
        assert!(prefetch_pso_check(true, 0x100, 1, 0).is_err());
        assert!(prefetch_pso_check(false, 0x100, 0x80, 0).is_err());
        assert!(prefetch_pso_check(false, 0x100, 0, 0).is_err());
        assert!(prefetch_pso_check(false, 0, 1, 0).is_err());
        assert!(prefetch_pso_check(false, 0x20_0000, 1, 0).is_err());
        assert!(prefetch_pso_check(false, 6, 1, 0).is_err());
        assert!(prefetch_pso_check(false, 4, 1, 1).is_err());
    }

    #[test]
    fn transcode_plan_walks_slices_like_the_console_dispatch() {
        let a = tex2d(64, 64, 4, 3);
        assert!(transcode_plan(&a, 0, 1, 0, 1, 0, 256, 64, 0).is_empty());
        let one = transcode_plan(&a, 1, 0, 2, 3, 0, 0, 0, 1);
        assert_eq!(one, vec![TranscodeCopy { src_sub: 1, dst_sub: 2, dst_offset: 0, src_box: None }]);
        let fp = transcode_plan(&a, 0, 1, 0, 1, 0x1000, 256, 64, 3);
        assert_eq!(fp.iter().map(|c| (c.src_sub, c.dst_offset)).collect::<Vec<_>>(), vec![(0, 0x1000), (3, 0x1000 + 0x4000), (6, 0x1000 + 0x8000)]);
        let sub = transcode_plan(&a, 0, 0, 0, 2, 0, 0, 0, 2);
        assert_eq!(sub.iter().map(|c| (c.src_sub, c.dst_sub)).collect::<Vec<_>>(), vec![(0, 0), (3, 2)]);
        assert_eq!(transcode_plan(&a, 0, 1, 0, 1, 0, 256, 64, 9).len(), 4);
        let v = Desc { dim: 4, width: 32, height: 32, depth_or_array: 8, mips: 1, format: 28, layout: 0 };
        assert_eq!(transcode_plan(&v, 0, 1, 0, 1, 0, 128, 32, 4)[0].src_box, Some([0, 0, 0, 32, 32, 4]));
        assert_eq!(transcode_plan(&v, 0, 1, 0, 1, 0, 128, 32, 8)[0].src_box, None);
        assert_eq!(transcode_pc_ok(&fp, 1, 28, 28), Ok(()));
        assert!(transcode_pc_ok(&fp, 1, 28, 87).is_err());
        let odd = transcode_plan(&a, 0, 1, 0, 1, 0, 256, 3, 2);
        assert_eq!(odd[1].dst_offset, 768);
        assert!(transcode_pc_ok(&odd, 1, 28, 28).is_err());
        assert_eq!(transcode_pc_ok(&odd[..1], 1, 28, 28), Ok(()));
        assert_eq!(transcode_pc_ok(&sub, 0, 28, 28), Ok(()));
    }

    #[test]
    fn install_replaces_only_untouched_slots() {
        let vt: Vec<AtomicUsize> = (0..128).map(|i| AtomicUsize::new(7000 + i)).collect();
        let defaults: Vec<usize> = vt.iter().map(|s| s.load(Ordering::Relaxed)).collect();
        vt[95].store(1, Ordering::Relaxed);
        assert_eq!(install_cmdlist(&vt, &defaults), 5);
        assert_eq!(vt[84].load(Ordering::Relaxed), cl_copy_resource_x as usize);
        assert_eq!(vt[91].load(Ordering::Relaxed), cl_get_execution_command_size_x as usize);
        assert_eq!(vt[92].load(Ordering::Relaxed), cl_close_bundle_x as usize);
        assert_eq!(vt[95].load(Ordering::Relaxed), 1);
        assert_eq!(vt[99].load(Ordering::Relaxed), cl_prefetch_pipeline_state_x as usize);
        assert_eq!(vt[103].load(Ordering::Relaxed), cl_transcode_texture_region_x as usize);
        assert_eq!(vt[93].load(Ordering::Relaxed), 7093);
        assert_eq!(vt[100].load(Ordering::Relaxed), 7100);
    }
}
