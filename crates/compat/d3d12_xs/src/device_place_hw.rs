use super::*;

const D_CREATE_HEAP: usize = 28;
const D_PLACED: usize = 29;
const D_PLACED_X: usize = 44;
const E_NOINTERFACE: i32 = 0x8000_4002u32 as i32;
const IID_HEAP: Guid = Guid { d1: 0x6b3b_2502, d2: 0x6e51, d3: 0x45b3, d4: [0x90, 0xee, 0x98, 0x84, 0x26, 0x5e, 0x8d, 0xf3] };

fn wrapped() -> usize {
    static W: OnceLock<usize> = OnceLock::new();
    *W.get_or_init(|| crate::device_wrap::wrap_device(gpu().dev as *mut c_void) as usize)
}

fn bdesc(width: u64) -> [u8; 56] {
    let mut d = [0u8; 56];
    d[0..4].copy_from_slice(&1u32.to_le_bytes());
    d[0x10..0x18].copy_from_slice(&width.to_le_bytes());
    d[0x18..0x1c].copy_from_slice(&1u32.to_le_bytes());
    d[0x1c..0x1e].copy_from_slice(&1u16.to_le_bytes());
    d[0x1e..0x20].copy_from_slice(&1u16.to_le_bytes());
    d[0x24..0x28].copy_from_slice(&1u32.to_le_bytes());
    d[0x2c..0x30].copy_from_slice(&1u32.to_le_bytes());
    d
}

unsafe fn placed(w: usize, heap: *mut c_void, off: u64, width: u64, riid: &Guid) -> (i32, *mut c_void) {
    let d = bdesc(width);
    let mut r: *mut c_void = core::ptr::null_mut();
    let hr = call!(w, D_PLACED, (*mut c_void, u64, *const u8, u32, *const c_void, *const Guid, *mut *mut c_void) -> i32, heap, off, d.as_ptr(), ST_COMMON, core::ptr::null(), riid, &mut r);
    (hr, r)
}

unsafe fn placed_x(w: usize, va: u64, width: u64, riid: &Guid) -> (i32, *mut c_void) {
    let d = bdesc(width);
    let mut r: *mut c_void = core::ptr::null_mut();
    let hr = call!(w, D_PLACED_X, (u64, *const u8, u32, *const c_void, *const Guid, *mut *mut c_void) -> i32, va, d.as_ptr(), ST_COMMON, core::ptr::null(), riid, &mut r);
    (hr, r)
}

unsafe fn run(g: &Gpu, rec: impl FnOnce(*mut c_void)) {
    let l = unsafe { title_list(g.dev) };
    rec(l);
    assert!(call!(l, X_CLOSE, () -> i32) >= 0);
    let f = unsafe { fence(g.dev) };
    unsafe { submit(g.a, l, f, 1) };
    assert!(reached(f, 1, Duration::from_secs(5)));
}

unsafe fn copy(l: *mut c_void, dst: usize, doff: u64, src: usize, soff: u64, n: u64) {
    call!(l, X_COPY_BUFFER_REGION, (usize, u64, usize, u64, u64) -> (), dst, doff, src, soff, n);
}

unsafe fn pattern_source(g: &Gpu) -> Buf {
    let up = unsafe { buffer(g.dev, HEAP_UPLOAD, 0x1_0000, 0, ST_GENERIC_READ) };
    unsafe {
        core::ptr::write_bytes(up.cpu, 0xaa, 0x1_0000);
        core::ptr::write_bytes(up.cpu.add(0x8000), 0x5c, 0x1000);
    }
    up
}

#[test]
#[ignore = "needs a D3D12 device; run: cargo test -p d3d12_xs --lib hw_ -- --ignored --test-threads=1 (TOMBOX_HW_ADAPTER=warp for WARP)"]
fn hw_slot29_console_packed_buffers_share_heap_bytes_on_the_gpu() {
    let g = gpu();
    let w = wrapped();
    unsafe {
        let mut hd = [0u8; crate::device_console_fix::HEAP_DESC_SIZE];
        hd[0..8].copy_from_slice(&0x1_0000u64.to_le_bytes());
        hd[8..12].copy_from_slice(&HEAP_DEFAULT.to_le_bytes());
        hd[0x28..0x2c].copy_from_slice(&0xc0u32.to_le_bytes());
        let mut h: *mut c_void = core::ptr::null_mut();
        assert!(call!(w, D_CREATE_HEAP, (*const u8, *const Guid, *mut *mut c_void) -> i32, hd.as_ptr(), &IID_HEAP, &mut h) >= 0 && !h.is_null());
        let (hr, bad) = placed(w, h, 0, 200, &IID_HEAP);
        assert_eq!((hr, bad), (E_NOINTERFACE, core::ptr::null_mut()), "release xgs12 0x4765f returns E_NOINTERFACE for any riid but IID_ID3D12Resource and creates nothing");
        let (hr0, r0) = placed(w, h, 0, 200, &IID_RESOURCE);
        let (hr1, r1) = placed(w, h, 100, 100, &IID_RESOURCE);
        assert!(hr0 >= 0 && !r0.is_null(), "buffer at heap offset 0 hr={hr0:#010x}");
        assert!(hr1 >= 0 && !r1.is_null(), "console-packed buffer at heap offset 100 hr={hr1:#010x}");
        let (v0, v1) = (call!(r0, 11, () -> u64), call!(r1, 11, () -> u64));
        assert_eq!(v1, v0 + 100, "release xgs12 0x476df: GPU VA = heap base + HeapOffset, with no 64 KB rounding");
        let up = pattern_source(g);
        let rb = buffer(g.dev, HEAP_READBACK, 0x1000, 0, ST_COPY_DEST);
        run(g, |l| copy(l, r0 as usize, 0, up.res, 0, 200));
        run(g, |l| copy(l, r1 as usize, 0, up.res, 0x8000, 8));
        run(g, |l| copy(l, rb.res, 0, r0 as usize, 0, 200));
        let m = |i: usize| core::ptr::read_volatile(rb.cpu.add(i));
        assert!((0..100).all(|i| m(i) == 0xaa), "bytes below the packed buffer are untouched");
        assert!((100..108).all(|i| m(i) == 0x5c), "a GPU copy into the buffer placed at offset 100 lands at heap byte 100");
        assert!((108..200).all(|i| m(i) == 0xaa), "bytes above the copied range are untouched");
        let rb2 = buffer(g.dev, HEAP_READBACK, 0x1000, 0, ST_COPY_DEST);
        run(g, |l| copy(l, rb2.res, 0, r1 as usize, 0, 100));
        let n = |i: usize| core::ptr::read_volatile(rb2.cpu.add(i));
        assert!((0..8).all(|i| n(i) == 0x5c) && (8..100).all(|i| n(i) == 0xaa), "reading through the packed buffer sees heap bytes 100..200");
        for r in [r1, r0, h] {
            call!(r, 2, () -> u32);
        }
        let base = 0x7320_0000_0000u64;
        let (hp, page) = placed(w, core::ptr::null_mut(), base, 0x1_0000, &IID_RESOURCE);
        let (hb, b) = placed(w, core::ptr::null_mut(), base + 0x2000, 0x1000, &IID_RESOURCE);
        assert!(hp >= 0 && !page.is_null() && hb >= 0 && !b.is_null(), "release xgs12 0x476ec / xgs12_10 0x4d761: with no heap, HeapOffset is the GPU VA itself (hr {hp:#010x} {hb:#010x})");
        assert_eq!(call!(b, 11, () -> u64), call!(page, 11, () -> u64) + 0x2000);
        let rb3 = buffer(g.dev, HEAP_READBACK, 0x1_0000, 0, ST_COPY_DEST);
        run(g, |l| copy(l, page as usize, 0, up.res, 0, 0x1_0000));
        run(g, |l| copy(l, b as usize, 0, up.res, 0x8000, 0x1000));
        run(g, |l| copy(l, rb3.res, 0, page as usize, 0, 0x1_0000));
        let k = |i: usize| core::ptr::read_volatile(rb3.cpu.add(i));
        assert!((0..0x2000).all(|i| k(i) == 0xaa) && (0x2000..0x3000).all(|i| k(i) == 0x5c), "a heapless slot-29 placement at VA base+0x2000 lands at page byte 0x2000, as slot 44 does");
        for r in [b, page] {
            call!(r, 2, () -> u32);
        }
    }
}

#[test]
#[ignore = "needs a D3D12 device; run: cargo test -p d3d12_xs --lib hw_ -- --ignored --test-threads=1 (TOMBOX_HW_ADAPTER=warp for WARP)"]
fn hw_slot44_sub_page_placements_at_title_vas_share_bytes_on_the_gpu() {
    let g = gpu();
    let w = wrapped();
    unsafe {
        let base = 0x7310_0000_0000u64;
        let (hr, bad) = placed_x(w, base, 0x1_0000, &IID_HEAP);
        assert_eq!((hr, bad), (E_NOINTERFACE, core::ptr::null_mut()), "release xgs12 0x477cb returns E_NOINTERFACE for any riid but IID_ID3D12Resource and creates nothing");
        let (hp, page) = placed_x(w, base, 0x1_0000, &IID_RESOURCE);
        assert!(hp >= 0 && !page.is_null(), "whole-page placement hr={hp:#010x}");
        let (hb, b) = placed_x(w, base + 0x2000, 0x1000, &IID_RESOURCE);
        assert!(hb >= 0 && !b.is_null(), "sub-page placement hr={hb:#010x}");
        let (gp, gb) = (call!(page, 11, () -> u64), call!(b, 11, () -> u64));
        assert_eq!(gb, gp + 0x2000, "release xgs12 0x4782f places the resource at the VA it is given, so two placements 0x2000 apart are 0x2000 apart on the GPU");
        let mut gd = [0u8; 56];
        call!(b, 10, (*mut u8) -> *mut u8, gd.as_mut_ptr());
        assert_eq!(u64::from_le_bytes(gd[0x10..0x18].try_into().unwrap()), 0x1000, "GetDesc keeps the title's width");
        let up = pattern_source(g);
        let rb = buffer(g.dev, HEAP_READBACK, 0x1_0000, 0, ST_COPY_DEST);
        run(g, |l| copy(l, page as usize, 0, up.res, 0, 0x1_0000));
        run(g, |l| copy(l, b as usize, 0, up.res, 0x8000, 0x1000));
        run(g, |l| copy(l, rb.res, 0, page as usize, 0, 0x1_0000));
        let m = |i: usize| core::ptr::read_volatile(rb.cpu.add(i));
        assert!((0..0x2000).all(|i| m(i) == 0xaa), "bytes below the sub-page placement are untouched");
        assert!((0x2000..0x3000).all(|i| m(i) == 0x5c), "a GPU copy into the placement at VA base+0x2000 lands at page byte 0x2000");
        assert!((0x3000..0x1_0000).all(|i| m(i) == if (0x8000..0x9000).contains(&i) { 0x5c } else { 0xaa }), "bytes above are untouched");
        for r in [b, page] {
            call!(r, 2, () -> u32);
        }
    }
}

const IID_DESC_HEAP: Guid = Guid { d1: 0x8efb_471d, d2: 0x616c, d3: 0x4f49, d4: [0x90, 0xf7, 0x12, 0x7b, 0xb7, 0x63, 0xfa, 0x51] };

unsafe fn uav_table(dev: usize) -> (usize, usize) {
    let d: [u32; 4] = [0, 4, 0, 0];
    let mut h: *mut c_void = core::ptr::null_mut();
    assert!(call!(dev, 14, (*const u32, *const Guid, *mut *mut c_void) -> i32, d.as_ptr(), &IID_DESC_HEAP, &mut h) >= 0 && !h.is_null());
    let mut cpu = 0usize;
    call!(h, 9, (*mut usize) -> *mut usize, &mut cpu);
    (h as usize, cpu)
}

#[test]
#[ignore = "needs a D3D12 device; run: cargo test -p d3d12_xs --lib hw_ -- --ignored --test-threads=1 (TOMBOX_HW_ADAPTER=warp for WARP)"]
fn hw_structured_uav_on_a_sub_page_uav_buffer_keeps_the_device() {
    let g = gpu();
    let w = wrapped();
    unsafe {
        let va = 0x7340_0000_3a00u64;
        let mut d = bdesc(8);
        d[0x30..0x34].copy_from_slice(&FLAG_UAV.to_le_bytes());
        let mut r: *mut c_void = core::ptr::null_mut();
        let hr = call!(w, D_PLACED_X, (u64, *const u8, u32, *const c_void, *const Guid, *mut *mut c_void) -> i32, va, d.as_ptr(), ST_COMMON, core::ptr::null(), &IID_RESOURCE, &mut r);
        assert!(hr >= 0 && !r.is_null(), "8-byte UAV buffer at a VA 0x3a00 into its page hr={hr:#010x}");
        let mut gd = [0u8; 56];
        call!(r, 10, (*mut u8) -> *mut u8, gd.as_mut_ptr());
        assert_eq!(u64::from_le_bytes(gd[0x10..0x18].try_into().unwrap()), 8, "the title still reads its own width");
        assert_eq!(crate::placed_offset::unpatched_width(r), Some(0x3a08), "the PC buffer starts at the 64 KB base");
        let (_heap, cpu) = uav_table(g.dev);
        let mut v = [0u8; 0x28];
        v[4..8].copy_from_slice(&1u32.to_le_bytes());
        v[0x10..0x14].copy_from_slice(&2u32.to_le_bytes());
        v[0x14..0x18].copy_from_slice(&4u32.to_le_bytes());
        call!(w, 19, (*mut c_void, *mut c_void, *const u8, usize) -> (), r, core::ptr::null_mut(), v.as_ptr(), cpu);
        let reason = call!(g.dev, 37, () -> i32);
        assert_eq!(reason, 0, "XDefiant's structured UAV (first 0, num 2, stride 4) on its 8-byte buffer at VA offset 0x3a00 removed the device: {reason:#010x}");
        let mut raw = [0u8; 0x28];
        raw[4..8].copy_from_slice(&1u32.to_le_bytes());
        raw[0x10..0x14].copy_from_slice(&2u32.to_le_bytes());
        raw[0x14..0x18].copy_from_slice(&4u32.to_le_bytes());
        let (_h2, cpu2) = uav_table(g.dev);
        call!(w, 19, (*mut c_void, *mut c_void, *const u8, usize) -> (), r, core::ptr::null_mut(), raw.as_ptr(), cpu2 + 64);
        assert_eq!(call!(g.dev, 37, () -> i32), 0);
        call!(r, 2, () -> u32);
    }
}

