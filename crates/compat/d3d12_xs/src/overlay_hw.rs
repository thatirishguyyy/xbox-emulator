use crate::device_wrap::Guid;
use core::ffi::c_void;

const IID_DEVICE: Guid = Guid { d1: 0x1898_19f1, d2: 0x1db6, d3: 0x4b57, d4: [0xbe, 0x54, 0x18, 0x21, 0x33, 0x9b, 0x85, 0xf7] };
const IID_QUEUE: Guid = Guid { d1: 0x0ec8_70a6, d2: 0x5d7e, d3: 0x4c22, d4: [0x8c, 0xfc, 0x5b, 0xaa, 0xe0, 0x76, 0x16, 0xed] };
const IID_ALLOCATOR: Guid = Guid { d1: 0x6102_dee4, d2: 0xaf59, d3: 0x4b09, d4: [0xb9, 0x99, 0xb4, 0x4d, 0x73, 0xf0, 0x9b, 0x24] };
const IID_LIST: Guid = Guid { d1: 0x5b16_0d0f, d2: 0xac1b, d3: 0x4185, d4: [0x8b, 0xa8, 0xb3, 0xae, 0x42, 0xa5, 0xa4, 0x55] };
const IID_RESOURCE: Guid = Guid { d1: 0x6964_42be, d2: 0xa72e, d3: 0x4059, d4: [0xbc, 0x79, 0x5b, 0x5c, 0x98, 0x04, 0x0f, 0xad] };
const IID_FENCE: Guid = Guid { d1: 0x0a75_3dcf, d2: 0xc4d8, d3: 0x4b91, d4: [0xad, 0xf6, 0xbe, 0x5a, 0x60, 0xd9, 0x5a, 0x76] };

const W: u32 = 1280;
const H: u32 = 720;

unsafe fn vs(o: usize, i: usize) -> usize {
    unsafe { *(*(o as *const *const usize)).add(i) }
}

macro_rules! vc {
    ($o:expr, $i:expr, fn($($t:ty),*) -> $r:ty $(, $a:expr)*) => {{
        let o = $o as usize;
        let f: unsafe extern "system" fn(usize $(, $t)*) -> $r = core::mem::transmute(vs(o, $i));
        f(o $(, $a)*)
    }};
}

unsafe fn res(dev: usize, heap: u32, dim: u32, w: u64, h: u32, fmt: u32, flags: u32, state: u32) -> usize {
    let props: [u32; 5] = [heap, 0, 0, 1, 1];
    let mut d = [0u8; 0x38];
    d[0..4].copy_from_slice(&dim.to_le_bytes());
    d[0x10..0x18].copy_from_slice(&w.to_le_bytes());
    d[0x18..0x1c].copy_from_slice(&h.to_le_bytes());
    d[0x1c..0x1e].copy_from_slice(&1u16.to_le_bytes());
    d[0x1e..0x20].copy_from_slice(&1u16.to_le_bytes());
    d[0x20..0x24].copy_from_slice(&fmt.to_le_bytes());
    d[0x24..0x28].copy_from_slice(&1u32.to_le_bytes());
    d[0x2c..0x30].copy_from_slice(&(if dim == 1 { 1u32 } else { 0 }).to_le_bytes());
    d[0x30..0x34].copy_from_slice(&flags.to_le_bytes());
    let mut r = 0usize;
    let hr = unsafe { vc!(dev, 27, fn(*const u32, u32, *const u8, u32, *const c_void, *const Guid, *mut usize) -> i32, props.as_ptr(), 0, d.as_ptr(), state, core::ptr::null(), &IID_RESOURCE, &mut r) };
    assert!(hr >= 0 && r != 0, "CreateCommittedResource hr={hr:#010x}");
    r
}

#[test]
#[ignore = "needs a D3D12 GPU: TOMBOX_EXEC_HW=1 cargo test -p d3d12_xs --lib overlay_hw -- --ignored"]
fn overlay_hw_renders_text_and_graph() {
    if std::env::var_os("TOMBOX_EXEC_HW").is_none() {
        return;
    }
    unsafe {
        let create = crate::host_proc(b"D3D12CreateDevice\0");
        assert!(create != 0);
        let f: extern "system" fn(*mut c_void, u32, *const Guid, *mut usize) -> i32 = core::mem::transmute(create);
        let mut dev = 0usize;
        assert!(f(core::ptr::null_mut(), 0xb000, &IID_DEVICE, &mut dev) >= 0 && dev != 0);
        let qd = [0u32; 4];
        let mut queue = 0usize;
        assert!(vc!(dev, 8, fn(*const u32, *const Guid, *mut usize) -> i32, qd.as_ptr(), &IID_QUEUE, &mut queue) >= 0);
        let mut alloc = 0usize;
        assert!(vc!(dev, 9, fn(u32, *const Guid, *mut usize) -> i32, 0, &IID_ALLOCATOR, &mut alloc) >= 0);
        let mut list = 0usize;
        assert!(vc!(dev, 12, fn(u32, u32, usize, usize, *const Guid, *mut usize) -> i32, 0, 0, alloc, 0, &IID_LIST, &mut list) >= 0);
        vc!(list, 9, fn() -> i32);
        let mut fence = 0usize;
        assert!(vc!(dev, 36, fn(u64, u32, *const Guid, *mut usize) -> i32, 0, 0, &IID_FENCE, &mut fence) >= 0);
        let rt = res(dev, 1, 3, W as u64, H, 28, 1, 0x400);
        let pitch = (W * 4).div_ceil(256) * 256;
        let rb = res(dev, 3, 1, (pitch * H) as u64, 1, 0, 0, 0x400);
        for i in 0..200 {
            crate::overlay::tick(0);
            std::thread::sleep(std::time::Duration::from_millis(if i % 37 == 0 { 30 } else { 6 }));
        }
        for _ in 0..3 {
            crate::overlay_sensors::latest();
            std::thread::sleep(std::time::Duration::from_millis(1100));
        }
        let mut cpu = Vec::new();
        let csv_freq = {
            let mut hz = 0u64;
            vc!(queue, 16, fn(*mut u64) -> i32, &mut hz);
            hz
        };
        for frame in 0..4u64 {
            let entry = crate::frame::qpc();
            vc!(list, 10, fn(usize, usize) -> i32, alloc, 0);
            crate::frame_csv::list_begin(dev as *mut c_void, list as *mut c_void, frame + 1, csv_freq);
            let t0 = crate::frame::qpc();
            assert!(crate::overlay::draw(dev as *mut c_void, queue as *mut c_void, list as *mut c_void, rt as *mut c_void, 28, W as u64, H));
            cpu.push((crate::frame::qpc() - t0) as f64 * 1e6 / crate::frame::qpf() as f64);
            let bar = [[0u64, rt as u64, 0xffff_ffffu64 | (4u64 << 32), 0x800u64]];
            vc!(list, 26, fn(u32, *const [u64; 4]) -> (), 1, bar.as_ptr());
            if frame == 3 {
                let mut dl = [0u8; 48];
                dl[0..8].copy_from_slice(&(rb as u64).to_le_bytes());
                dl[8..12].copy_from_slice(&1u32.to_le_bytes());
                dl[24..28].copy_from_slice(&28u32.to_le_bytes());
                dl[28..32].copy_from_slice(&W.to_le_bytes());
                dl[32..36].copy_from_slice(&H.to_le_bytes());
                dl[36..40].copy_from_slice(&1u32.to_le_bytes());
                dl[40..44].copy_from_slice(&pitch.to_le_bytes());
                let mut sl = [0u8; 48];
                sl[0..8].copy_from_slice(&(rt as u64).to_le_bytes());
                vc!(list, 16, fn(*const u8, u32, u32, u32, *const u8, *const c_void) -> (), dl.as_ptr(), 0, 0, 0, sl.as_ptr(), core::ptr::null());
            }
            let back = [[0u64, rt as u64, 0xffff_ffffu64 | (0x800u64 << 32), 0x400u64]];
            vc!(list, 26, fn(u32, *const [u64; 4]) -> (), 1, back.as_ptr());
            crate::frame_csv::list_end(list as *mut c_void, frame + 1);
            assert!(vc!(list, 9, fn() -> i32) >= 0);
            let lists = [list];
            vc!(queue, 10, fn(u32, *const usize) -> (), 1, lists.as_ptr());
            vc!(queue, 14, fn(usize, u64) -> i32, fence, frame + 1);
            while vc!(fence, 8, fn() -> u64) < frame + 1 {
                std::thread::yield_now();
            }
            crate::overlay::tick(0);
            crate::frame_csv::present_done(frame + 1, entry);
        }
        let reason = vc!(dev, 37, fn() -> i32);
        assert!(reason >= 0, "device removed {reason:#010x}");
        let mut p: *mut u8 = core::ptr::null_mut();
        assert!(vc!(rb, 8, fn(u32, *const c_void, *mut *mut u8) -> i32, 0, core::ptr::null(), &mut p) >= 0);
        let data = core::slice::from_raw_parts(p, (pitch * H) as usize);
        let lit = (0..H as usize).flat_map(|y| (0..W as usize).map(move |x| (x, y))).filter(|&(x, y)| data[y * pitch as usize + x * 4] > 200).count();
        let out = std::env::var("TOMBOX_OVERLAY_SHOT").unwrap_or_else(|_| std::env::temp_dir().join("tombox_overlay_hw.bmp").to_string_lossy().into_owned());
        let mut bmp = Vec::with_capacity(54 + (W * H * 4) as usize);
        bmp.extend_from_slice(b"BM");
        bmp.extend_from_slice(&(54 + W * H * 4).to_le_bytes());
        bmp.extend_from_slice(&0u32.to_le_bytes());
        bmp.extend_from_slice(&54u32.to_le_bytes());
        bmp.extend_from_slice(&40u32.to_le_bytes());
        bmp.extend_from_slice(&(W as i32).to_le_bytes());
        bmp.extend_from_slice(&(-(H as i32)).to_le_bytes());
        bmp.extend_from_slice(&1u16.to_le_bytes());
        bmp.extend_from_slice(&32u16.to_le_bytes());
        bmp.extend_from_slice(&[0u8; 24]);
        for y in 0..H as usize {
            for x in 0..W as usize {
                let o = y * pitch as usize + x * 4;
                bmp.extend_from_slice(&[data[o + 2], data[o + 1], data[o], 255]);
            }
        }
        std::fs::write(&out, bmp).unwrap();
        crate::frame_csv::close();
        if let Ok(p) = std::env::var("TOMBOX_LOG") {
            let csv = std::path::Path::new(&p).parent().unwrap().join("tombox_frames.csv");
            if let Ok(t) = std::fs::read_to_string(&csv) {
                println!("frame csv:
{t}");
                assert!(t.lines().count() >= 3, "frame csv has too few rows");
                assert!(t.lines().nth(2).is_some_and(|l| !l.ends_with(",,")), "frame csv rows lack gpu columns");
            }
        }
        println!("overlay shot {out}: {lit} bright pixels, cpu record us {cpu:?}");
        assert!(lit > 500, "overlay drew only {lit} bright pixels");
    }
}
