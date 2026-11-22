use crate::layout::{element_offset, mip_offset, write_layout, Layout, LAYOUT_BYTES};
use core::ffi::c_void;
use core::sync::atomic::{AtomicU32, Ordering};
use std::sync::OnceLock;

pub const E_INVALIDARG: i32 = 0x8007_0057u32 as i32;
pub const E_POINTER: i32 = 0x8000_4003u32 as i32;

#[repr(C)]
pub struct Computer {
    vtable: *const usize,
    refs: AtomicU32,
    pub layout: Layout,
}

unsafe fn me<'a>(this: *mut Computer) -> &'a Computer {
    unsafe { &*this }
}

unsafe extern "system" fn add_ref(this: *mut Computer) -> u32 {
    unsafe { me(this).refs.fetch_add(1, Ordering::SeqCst) + 1 }
}

unsafe extern "system" fn release(this: *mut Computer) -> u32 {
    let n = unsafe { me(this).refs.fetch_sub(1, Ordering::SeqCst) } - 1;
    if n == 0 {
        drop(unsafe { Box::from_raw(this) });
    }
    n
}

unsafe extern "system" fn get_resource_layout(this: *mut Computer, out: *mut u8) -> i32 {
    if out.is_null() {
        return E_POINTER;
    }
    let buf = unsafe { core::slice::from_raw_parts_mut(out, LAYOUT_BYTES) };
    write_layout(&unsafe { me(this) }.layout, buf);
    0
}

unsafe extern "system" fn get_size(this: *mut Computer) -> u64 {
    unsafe { me(this) }.layout.size
}

unsafe extern "system" fn get_align(this: *mut Computer) -> u64 {
    unsafe { me(this) }.layout.align
}

unsafe extern "system" fn get_mip_offset(this: *mut Computer, plane: u32, mip: u32) -> u64 {
    if plane != 0 {
        return u64::MAX;
    }
    mip_offset(&unsafe { me(this) }.layout, mip as usize).unwrap_or(u64::MAX)
}

unsafe extern "system" fn get_element_offset(this: *mut Computer, plane: u32, mip: u32, x: u64, y: u32, z: u32, _sample: u32, bit: *mut u32) -> u64 {
    if !bit.is_null() {
        unsafe { *bit = 0 };
    }
    if plane != 0 {
        return u64::MAX;
    }
    element_offset(&unsafe { me(this) }.layout, mip as usize, x, y as u64, z as u64).unwrap_or(u64::MAX)
}

unsafe extern "system" fn get_texel_coordinate(this: *mut Computer, plane: u32, offset: u64, mip: *mut u32, x: *mut u64, y: *mut u32, z: *mut u32, sample: *mut u32) -> i32 {
    if plane != 0 {
        return E_INVALIDARG;
    }
    let l = &unsafe { me(this) }.layout;
    for (i, m) in l.mips.iter().enumerate() {
        let base = mip_offset(l, i).unwrap_or(0);
        if offset < base || offset >= base + m.size {
            continue;
        }
        let bpe = l.fmt.bpe as u64;
        let zs = (offset - base) / m.slice.max(1);
        for yy in 0..m.padded_h as u64 {
            for xx in 0..m.padded_w as u64 {
                if let Some(o) = element_offset(l, i, xx, yy, zs) {
                    if offset >= o && offset < o + bpe {
                        unsafe {
                            if !mip.is_null() { *mip = i as u32; }
                            if !x.is_null() { *x = xx; }
                            if !y.is_null() { *y = yy as u32; }
                            if !z.is_null() { *z = zs as u32; }
                            if !sample.is_null() { *sample = 0; }
                        }
                        return 0;
                    }
                }
            }
        }
    }
    E_INVALIDARG
}

fn sub_split(l: &Layout, sub: u32) -> Option<(usize, u64, u64)> {
    let n = l.mips.len() as u32;
    let (mip, slice) = (sub % n, sub / n);
    let m = l.mips.get(mip as usize)?;
    if l.is_3d {
        if slice != 0 {
            return None;
        }
        Some((mip as usize, 0, m.depth as u64))
    } else {
        if slice >= l.array {
            return None;
        }
        Some((mip as usize, slice as u64, 1))
    }
}

fn copy(l: &Layout, sub: u32, row_pitch: u32, slice_pitch: u32, mut f: impl FnMut(u64, usize, usize)) -> i32 {
    let Some((mip, first, count)) = sub_split(l, sub) else { return E_INVALIDARG };
    let m = &l.mips[mip];
    let bpe = l.fmt.bpe as usize;
    for zi in 0..count {
        for y in 0..m.height as u64 {
            for x in 0..m.width as u64 {
                let Some(o) = element_offset(l, mip, x, y, first + zi) else { return E_INVALIDARG };
                let lin = zi as usize * slice_pitch as usize + y as usize * row_pitch as usize + x as usize * bpe;
                f(o, lin, bpe);
            }
        }
    }
    0
}

unsafe extern "system" fn copy_into(this: *mut Computer, res: *mut u8, plane: u32, sub: u32, src: *const u8, row_pitch: u32, slice_pitch: u32) -> i32 {
    if res.is_null() || src.is_null() {
        return E_POINTER;
    }
    if plane != 0 {
        return E_INVALIDARG;
    }
    let l = &unsafe { me(this) }.layout;
    copy(l, sub, row_pitch, slice_pitch, |o, lin, n| unsafe { core::ptr::copy_nonoverlapping(src.add(lin), res.add(o as usize), n) })
}

unsafe extern "system" fn copy_from(this: *mut Computer, dst: *mut u8, plane: u32, sub: u32, res: *const u8, row_pitch: u32, slice_pitch: u32) -> i32 {
    if res.is_null() || dst.is_null() {
        return E_POINTER;
    }
    if plane != 0 {
        return E_INVALIDARG;
    }
    let l = &unsafe { me(this) }.layout;
    copy(l, sub, row_pitch, slice_pitch, |o, lin, n| unsafe { core::ptr::copy_nonoverlapping(res.add(o as usize), dst.add(lin), n) })
}

unsafe extern "system" fn is_tiled(this: *mut Computer, plane: u32, mip: u32, _a: usize, _b: usize) -> i32 {
    let l = &unsafe { me(this) }.layout;
    match (plane, l.mips.get(mip as usize)) {
        (0, Some(m)) => m.is_tiled() as i32,
        _ => 0,
    }
}

fn vtable() -> *const usize {
    static VT: OnceLock<usize> = OnceLock::new();
    *VT.get_or_init(|| {
        let v: Vec<usize> = vec![
            add_ref as usize,
            release as usize,
            get_resource_layout as usize,
            get_size as usize,
            get_align as usize,
            get_mip_offset as usize,
            get_element_offset as usize,
            get_texel_coordinate as usize,
            copy_into as usize,
            copy_from as usize,
            is_tiled as usize,
        ];
        Box::leak(v.into_boxed_slice()).as_ptr() as usize
    }) as *const usize
}

pub fn create(layout: Layout) -> *mut c_void {
    Box::into_raw(Box::new(Computer { vtable: vtable(), refs: AtomicU32::new(1), layout })) as *mut c_void
}
