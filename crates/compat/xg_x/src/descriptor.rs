use crate::computer::{self, E_INVALIDARG};
use crate::layout::{compute, format, Desc};
use crate::tileshape::{gcn_shape, DXGI_TO_GCN};
use compat_common::{record, S_OK};
use core::ffi::c_void;

const E_FAIL: i32 = 0x8000_4005u32 as i32;
const NAME: &str = "XGCreateTextureComputerFromDescriptor";

fn dxgi_for(data_format: u32) -> Option<u32> {
    let want = gcn_shape(data_format);
    DXGI_TO_GCN.iter().enumerate().skip(1).find_map(|(f, &i)| {
        let f = f as u32;
        let fm = format(f)?;
        let block = want[2].max(want[3]) as u32;
        (i as u32 == data_format && fm.block == block).then_some(f)
    })
}

pub fn decode(d: &[u32; 8]) -> Result<Desc, (i32, String)> {
    if d[3] >> 31 == 0 || d[1] & 0x3f0_0000 == 0 {
        return Err((E_INVALIDARG, "Descriptor is not valid.".into()));
    }
    let ty = d[3] >> 28;
    let dimension = match ty {
        9 | 11 | 13 | 14 | 15 => 3,
        8 | 12 => 2,
        10 => 4,
        _ => return Err((E_INVALIDARG, "Descriptor dimension is not recognized.".into())),
    };
    let data_format = (d[1] >> 20) & 0x3f;
    let width = (d[2] & 0x3fff) as u64 + 1;
    let height = if dimension == 2 { 1 } else { ((d[2] >> 14) & 0x3fff) + 1 };
    let last_level = (d[3] >> 16) & 0xf;
    let msaa = ty == 14 || ty == 15;
    let mips = if msaa { 1 } else { last_level + 1 };
    let samples = if msaa { 1 << last_level } else { 1 };
    let mode = (d[3] >> 20) & 0x1f;
    let depth_or_array = match ty {
        10 | 11 | 12 | 13 | 15 => (d[4] & 0x1fff) + 1,
        _ => 1,
    };
    let dxgi = dxgi_for(data_format).ok_or((E_FAIL, format!("GCN data format {data_format} has no surface layout")))?;
    Ok(Desc { dimension, width, height, depth_or_array, mips, dxgi, samples, mode, pitch: 0 })
}

#[no_mangle]
pub unsafe extern "system" fn XGCreateTextureComputerFromDescriptor(desc: *const u8, pp: *mut *mut c_void) -> i32 {
    if desc.is_null() || pp.is_null() {
        return E_INVALIDARG;
    }
    let locked = crate::XGEnterExclusiveLock();
    let mut d = [0u32; 8];
    for (i, v) in d.iter_mut().enumerate() {
        *v = unsafe { core::ptr::read_unaligned(desc.add(i * 4) as *const u32) };
    }
    let r = decode(&d).and_then(|x| compute(&x).ok_or((E_FAIL, "surface layout for the descriptor could not be computed".into())));
    if locked != 0 {
        crate::XGLeaveExclusiveLock();
    }
    match r {
        Ok(l) => {
            unsafe { *pp = computer::create(l) };
            S_OK
        }
        Err((hr, why)) => {
            record(crate::M, NAME, &why);
            unsafe { *pp = core::ptr::null_mut() };
            hr
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn srd(data_format: u32, w: u32, h: u32, ty: u32, last_level: u32, mode: u32, depth: u32) -> [u32; 8] {
        [0, data_format << 20, (w - 1) | ((h - 1) << 14), (ty << 28) | (mode << 20) | (last_level << 16), depth - 1, 0, 0, 0]
    }

    unsafe fn size_of(obj: *mut c_void) -> u64 {
        let vt = unsafe { *(obj as *const *const usize) };
        let f: unsafe extern "system" fn(*mut c_void) -> u64 = unsafe { core::mem::transmute(*vt.add(3)) };
        f(obj)
    }

    unsafe fn release(obj: *mut c_void) {
        let vt = unsafe { *(obj as *const *const usize) };
        let f: unsafe extern "system" fn(*mut c_void) -> u32 = unsafe { core::mem::transmute(*vt.add(1)) };
        f(obj);
    }

    #[test]
    fn valid_2d_descriptor_builds_matching_computer() {
        let d = srd(10, 256, 128, 9, 0, 14, 1);
        let x = decode(&d).unwrap();
        assert_eq!((x.dimension, x.width, x.height, x.mips, x.mode, x.samples), (3, 256, 128, 1, 14, 1));
        assert_eq!(format(x.dxgi).unwrap().bpe, 4);
        let want = compute(&x).unwrap().size;
        let mut p: *mut c_void = core::ptr::null_mut();
        let hr = unsafe { XGCreateTextureComputerFromDescriptor(d.as_ptr() as *const u8, &mut p) };
        assert_eq!(hr, S_OK);
        assert!(!p.is_null());
        assert_eq!(unsafe { size_of(p) }, want);
        unsafe { release(p) };
        let bc1 = decode(&srd(35, 64, 64, 13, 2, 14, 4)).unwrap();
        assert_eq!((format(bc1.dxgi).unwrap().block, bc1.depth_or_array, bc1.mips), (4, 4, 3));
        let ms = decode(&srd(10, 64, 64, 14, 2, 14, 1)).unwrap();
        assert_eq!((ms.mips, ms.samples), (1, 4));
    }

    #[test]
    fn invalid_descriptors_fail_like_console() {
        let mut p: *mut c_void = 1 as *mut c_void;
        let d = srd(10, 4, 4, 9, 0, 14, 1);
        assert_eq!(unsafe { XGCreateTextureComputerFromDescriptor(core::ptr::null(), &mut p) }, E_INVALIDARG);
        assert_eq!(p as usize, 1);
        assert_eq!(unsafe { XGCreateTextureComputerFromDescriptor(d.as_ptr() as *const u8, core::ptr::null_mut()) }, E_INVALIDARG);
        let not_image = srd(10, 4, 4, 1, 0, 14, 1);
        assert_eq!(unsafe { XGCreateTextureComputerFromDescriptor(not_image.as_ptr() as *const u8, &mut p) }, E_INVALIDARG);
        assert!(p.is_null());
        let no_format = srd(0, 4, 4, 9, 0, 14, 1);
        assert_eq!(decode(&no_format).unwrap_err().0, E_INVALIDARG);
        let dims: Vec<u32> = (8..16).map(|t| decode(&srd(10, 4, 4, t, 0, 14, 1)).unwrap().dimension).collect();
        assert_eq!(dims, [2, 3, 4, 3, 2, 3, 3, 3]);
        let unlaid = srd(63, 4, 4, 9, 0, 14, 1);
        assert_eq!(unsafe { XGCreateTextureComputerFromDescriptor(unlaid.as_ptr() as *const u8, &mut p) }, E_FAIL);
        assert!(p.is_null());
    }
}
