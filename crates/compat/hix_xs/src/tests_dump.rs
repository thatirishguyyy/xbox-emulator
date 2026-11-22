use super::*;
use crate::hang_stream::{CMD_HALT_WAVES, CMD_LIVE_DEBUG, CMD_SQ_DUMP, CMD_UNHALT_WAVES, E_NOTIMPL};

unsafe extern "system" fn sink(_s: *const u8) {}

fn files(d: &std::path::Path) -> Vec<std::path::PathBuf> {
    let Ok(rd) = std::fs::read_dir(d) else { return Vec::new() };
    rd.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect()
}

fn summary() -> Vec<u8> {
    let ctx = [0u8; 0x60];
    let mut report = vec![0u8; 0x1000];
    assert_eq!(unsafe { CollectHangSummary(ctx.as_ptr(), report.as_mut_ptr()) }, 0);
    report
}

#[test]
fn kmd_declines_unmapped_capture_commands_and_serves_control_ones() {
    let _g = crate::tests::LOCK.lock().unwrap();
    for c in [0xf, 0x17] {
        assert!(hang_stream::served(c), "{c:#x}");
    }
    for c in [0x28, 0x29, 0x10, 0x13, 0x14, 0x15, 0x16, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f, 0x20, 0x21, 0x22, 0x23, 0x2a, 0x1234] {
        assert!(!hang_stream::served(c), "{c:#x}");
    }
    unsafe {
        let mb = kmd::region1();
        let data = kmd::region0();
        let ring = [kmd::DOORBELL_MAGIC, 1];
        let mut ret = 0u32;
        for (cmd, status) in [(CMD_SQ_DUMP, E_NOTIMPL), (CMD_HALT_WAVES, E_NOTIMPL), (CMD_UNHALT_WAVES, E_NOTIMPL), (hang_stream::CMD_DISARM, 0)] {
            let seq = ((mb + kmd::MAILBOX_SEQ) as *const u32).read_volatile();
            ((mb + kmd::MAILBOX_CMD) as *mut u32).write_volatile(cmd);
            ((mb + kmd::MAILBOX_SEQ) as *mut u32).write_volatile(seq + 1);
            assert_eq!(kmd::ioctl(kmd::IOCTL_DOORBELL, ring.as_ptr() as *const u8, 8, core::ptr::null_mut(), 0, &mut ret), 1);
            assert_eq!(((data + kmd::DATA_ACK) as *const u32).read_volatile(), seq + 1);
            assert_eq!(kmd::last_status(), status);
            assert_eq!(((data + kmd::DATA_SIZE) as *const u32).read_volatile(), 0);
        }
    }
}

#[test]
fn console_write_hang_dump_takes_its_failed_capture_path() {
    let s = crate::tests_kmd::scratch();
    let _g = crate::tests::LOCK.lock().unwrap();
    assert_ne!(console::module(), 0, "console hix_xs.dll not found (TOMBOX_HIX_XS_CONSOLE, hix_xs_console.dll beside the DLL, or a title EmbeddedXvd dump); this test needs it");
    unsafe {
        crate::tests::init_hix(sink as *const () as usize);
        let report = summary();
        for (flags, drive) in [(0u32, "t"), (4, "d")] {
            let dir = s.join(drive).join("HIX");
            let before = files(&dir);
            kmd::clear_history();
            let mut out = vec![0x5a5au16; 260];
            assert_eq!(WriteHangDump(flags, out.as_mut_ptr(), out.len() as u32, report.as_ptr()), 0);
            assert_eq!(kmd::history(), vec![CMD_HALT_WAVES, CMD_SQ_DUMP, CMD_UNHALT_WAVES]);
            assert!(out.iter().all(|c| *c == 0x5a5a));
            let new: Vec<_> = files(&dir).into_iter().filter(|p| !before.contains(p)).collect();
            assert_eq!(new.len(), 1, "{new:?}");
            let name = new[0].file_name().unwrap().to_string_lossy().to_string();
            assert!(name.starts_with("gpuhang-") && name.ends_with(".hix"), "{name}");
            let b = std::fs::read(&new[0]).unwrap();
            assert_eq!(u32::from_le_bytes(b[0..4].try_into().unwrap()), 0x78_6948);
            assert_eq!(u32::from_le_bytes(b[4..8].try_into().unwrap()), 0x22_0200);
            assert_eq!(&b[8..12], b"IciT");
            assert!(b.windows(4).any(|w| w == b"IsyS"));
            assert!(!s.join("d").join("gpuhang-latest.hix").exists());
        }
        assert_eq!(TerminateHix(), 0);
    }
}

#[test]
fn console_debug_live_reports_no_live_debugging() {
    crate::tests_kmd::scratch();
    let _g = crate::tests::LOCK.lock().unwrap();
    assert_ne!(console::module(), 0, "console hix_xs.dll not found (TOMBOX_HIX_XS_CONSOLE, hix_xs_console.dll beside the DLL, or a title EmbeddedXvd dump); this test needs it");
    unsafe {
        crate::tests::init_hix(sink as *const () as usize);
        let report = summary();
        kmd::clear_history();
        let mut out = vec![0u8; 0x1000];
        assert_eq!(DebugLive(0, report.as_ptr(), out.as_mut_ptr()), E_FAIL);
        assert_eq!(kmd::history(), vec![CMD_LIVE_DEBUG]);
        assert_eq!(kmd::last_status(), E_NOTIMPL);
        assert_eq!(TerminateHix(), 0);
    }
}
