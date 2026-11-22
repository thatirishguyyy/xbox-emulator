use crate::oracle::{oracle, SERIAL};

fn desc(w: u32, h: u32, samples: u32, bind: u32, mode: u32) -> [u8; 0x34] {
    let mut d = [0u8; 0x34];
    for (i, v) in [w, h, 1, 1, 28, samples, 0, 0, bind, 0, 0, mode, 0].iter().enumerate() {
        d[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    d
}

#[test]
fn oracle_msaa_hresults() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let Some(o) = oracle() else { return };
    type Lay = unsafe extern "system" fn(*const u8, *mut u8) -> i32;
    let lay_o: Lay = unsafe { core::mem::transmute(o.f("XGComputeTexture2DLayout")) };
    let mut bad = Vec::new();
    let mut n = 0;
    for samples in [2u32, 4, 8] {
        for bind in [8u32, 0x28] {
            for mode in [0u32, 1, 5, 9, 10, 17, 21, 24, 25, 26, 27, 31] {
                n += 1;
                let d = desc(256, 256, samples, bind, mode);
                let mut a = vec![0u8; 0x2000];
                let mut b = vec![0u8; 0x2000];
                let ha = unsafe { lay_o(d.as_ptr(), a.as_mut_ptr()) };
                let hb = unsafe { crate::XGComputeTexture2DLayout(d.as_ptr(), b.as_mut_ptr()) };
                if ha != hb {
                    bad.push(format!("samples={samples} bind={bind:#x} mode={mode}: hr console {:#x} native {:#x}", ha as u32, hb as u32));
                }
            }
        }
    }
    eprintln!("msaa hresults: {n} cases, {} mismatches", bad.len());
    for b in &bad {
        eprintln!("  {b}");
    }
    assert!(bad.is_empty());
}

#[test]
fn oracle_rt_cmask_plane() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let Some(o) = oracle() else { return };
    type Lay = unsafe extern "system" fn(*const u8, *mut u8) -> i32;
    let lay_o: Lay = unsafe { core::mem::transmute(o.f("XGComputeTexture2DLayout")) };
    let mut bad = Vec::new();
    let mut n = 0;
    let mut unstable = 0;
    for (w, h) in [(512u32, 512u32), (100, 60), (1920, 1080), (4096, 2048)] {
        for arr in [1u32, 2, 6] {
            let mut cases = vec![(5u32, 28u32), (9, 28), (27, 28), (27, 2), (25, 61), (5, 77), (5, 98), (5, 24), (0, 28), (24, 28), (28, 28), (24, 2), (24, 61), (28, 41)];
            cases.extend((1..32).map(|m| (m, 71)));
            for (mode, fmt) in cases {
                for bind in [0x20u32, 0x28, 0xa8] {
                    if matches!(fmt, 70..=84 | 94..=99) && matches!(mode, 24 | 28) {
                        unstable += 1;
                        continue;
                    }
                    let mut d = desc(w, h, 1, bind, mode);
                    d[12..16].copy_from_slice(&arr.to_le_bytes());
                    d[16..20].copy_from_slice(&fmt.to_le_bytes());
                    let mut a = vec![0u8; 0x2000];
                    let mut b = vec![0u8; 0x2000];
                    let ha = unsafe { lay_o(d.as_ptr(), a.as_mut_ptr()) };
                    let hb = unsafe { crate::XGComputeTexture2DLayout(d.as_ptr(), b.as_mut_ptr()) };
                    n += 1;
                    let diff = (0..0x1b40 / 4).find(|&i| a[i * 4..i * 4 + 4] != b[i * 4..i * 4 + 4]);
                    if ha != hb || (ha == 0 && diff.is_some()) {
                        bad.push(format!("{w}x{h} arr={arr} mode={mode} fmt={fmt} bind={bind:#x}: hr {:#x}/{:#x} first diff {:x?} console {:#x} native {:#x}", ha as u32, hb as u32, diff.map(|i| i * 4), diff.map_or(0, |i| u32::from_le_bytes(a[i * 4..i * 4 + 4].try_into().unwrap())), diff.map_or(0, |i| u32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().unwrap()))));
                    }
                }
            }
        }
    }
    eprintln!("rt cmask: {n} cases, {} mismatches, {unstable} block-compressed depth-swizzle render-target cases skipped (console result depends on prior calls)", bad.len());
    for b in bad.iter().take(40) {
        eprintln!("  {b}");
    }
    assert!(bad.is_empty());
}