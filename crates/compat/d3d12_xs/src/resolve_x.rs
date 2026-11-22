use compat_common::record;
use core::ffi::c_void;
use core::sync::atomic::{AtomicUsize, Ordering};

pub const SLOT_RESOLVE: usize = 19;
const PC_SLOT_RESOLVE_REGION: usize = 64;
const RES_GET_DESC: usize = 10;
const RESOLVE_MODE_AVERAGE: u32 = 3;

static PREV: AtomicUsize = AtomicUsize::new(0);
static LOGGED: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Extent {
    pub dim: u32,
    pub width: u64,
    pub height: u32,
    pub mips: u16,
}

pub fn extent_of(desc: &[u8; 0x38]) -> Extent {
    Extent {
        dim: u32::from_le_bytes(desc[0..4].try_into().unwrap()),
        width: u64::from_le_bytes(desc[0x10..0x18].try_into().unwrap()),
        height: u32::from_le_bytes(desc[0x18..0x1c].try_into().unwrap()),
        mips: u16::from_le_bytes(desc[0x1e..0x20].try_into().unwrap()).max(1),
    }
}

pub fn mip_size(e: Extent, sub: u32) -> (u64, u32) {
    let mip = sub % e.mips as u32;
    ((e.width >> mip).max(1), (e.height >> mip).max(1))
}

pub fn overlap_rect(dst: Extent, dst_sub: u32, src: Extent, src_sub: u32) -> Option<[i32; 4]> {
    if dst.dim < 2 || src.dim < 2 {
        return None;
    }
    let (dw, dh) = mip_size(dst, dst_sub);
    let (sw, sh) = mip_size(src, src_sub);
    if (dw, dh) == (sw, sh) {
        return None;
    }
    Some([0, 0, dw.min(sw) as i32, dh.min(sh) as i32])
}

unsafe fn desc(res: *mut c_void) -> Option<[u8; 0x38]> {
    if res.is_null() {
        return None;
    }
    let mut d = [0u8; 0x38];
    let f: unsafe extern "system" fn(*mut c_void, *mut u8) -> *mut u8 = unsafe { core::mem::transmute(*(*(res as *const *const usize)).add(RES_GET_DESC)) };
    unsafe { f(res, d.as_mut_ptr()) };
    Some(d)
}

unsafe extern "system" fn resolve_subresource_x(list: *mut c_void, dst: *mut c_void, dst_sub: u32, src: *mut c_void, src_sub: u32, format: u32) {
    let rect = match unsafe { (desc(dst), desc(src)) } {
        (Some(d), Some(s)) => overlap_rect(extent_of(&d), dst_sub, extent_of(&s), src_sub),
        _ => None,
    };
    if let Some(r) = rect {
        if LOGGED.fetch_add(1, Ordering::Relaxed) < 8 {
            record("d3d12_xs", "resolve_x", &format!("ResolveSubresource dst={dst:p} sub {dst_sub} src={src:p} sub {src_sub} fmt={format}: extents differ, resolving the shared region {r:?} as the console resolve does (PC rejects the mismatch at Close)"));
        }
        let f: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, u32, u32, *mut c_void, u32, *const [i32; 4], u32, u32) =
            unsafe { core::mem::transmute(*(*(list as *const *const usize)).add(PC_SLOT_RESOLVE_REGION)) };
        unsafe { f(list, dst, dst_sub, 0, 0, src, src_sub, &r, format, RESOLVE_MODE_AVERAGE) };
        return;
    }
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void, u32, *mut c_void, u32, u32) = unsafe { core::mem::transmute(PREV.load(Ordering::Acquire)) };
    unsafe { f(list, dst, dst_sub, src, src_sub, format) }
}

pub fn install(vt: &[AtomicUsize]) -> bool {
    if SLOT_RESOLVE >= vt.len() {
        return false;
    }
    let cur = vt[SLOT_RESOLVE].load(Ordering::Relaxed);
    if cur == resolve_subresource_x as *const () as usize || cur == 0 {
        return false;
    }
    PREV.store(cur, Ordering::Release);
    vt[SLOT_RESOLVE].store(resolve_subresource_x as *const () as usize, Ordering::Relaxed);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tex(w: u64, h: u32, mips: u16) -> Extent {
        Extent { dim: 3, width: w, height: h, mips }
    }

    #[test]
    fn minecraft_resize_resolve_uses_the_shared_region() {
        assert_eq!(overlap_rect(tex(1904, 1041, 1), 0, tex(1920, 1049, 1), 0), Some([0, 0, 1904, 1041]));
        assert_eq!(overlap_rect(tex(1920, 1049, 1), 0, tex(1904, 1041, 1), 0), Some([0, 0, 1904, 1041]));
        assert_eq!(overlap_rect(tex(1920, 1049, 1), 0, tex(1920, 1049, 1), 0), None);
        assert_eq!(overlap_rect(tex(1920, 1080, 2), 1, tex(960, 540, 1), 0), None);
        assert_eq!(overlap_rect(Extent { dim: 1, width: 64, height: 1, mips: 1 }, 0, tex(8, 8, 1), 0), None);
    }

    #[test]
    fn install_chains_the_previous_body() {
        let vt: Vec<AtomicUsize> = (0..32).map(|i| AtomicUsize::new(1000 + i)).collect();
        assert!(install(&vt));
        assert_eq!(PREV.load(Ordering::Acquire), 1019);
        assert_eq!(vt[SLOT_RESOLVE].load(Ordering::Relaxed), resolve_subresource_x as *const () as usize);
        assert!(!install(&vt));
    }

    #[test]
    fn desc_extent_reads_the_d3d12_layout() {
        let mut d = [0u8; 0x38];
        d[0..4].copy_from_slice(&3u32.to_le_bytes());
        d[0x10..0x18].copy_from_slice(&1920u64.to_le_bytes());
        d[0x18..0x1c].copy_from_slice(&1049u32.to_le_bytes());
        d[0x1e..0x20].copy_from_slice(&0u16.to_le_bytes());
        assert_eq!(extent_of(&d), tex(1920, 1049, 1));
        assert_eq!(mip_size(tex(1920, 1049, 3), 2), (480, 262));
    }
}
