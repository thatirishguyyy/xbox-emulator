use super::*;
use crate::console::{map_path, Mapped};

pub(crate) fn scratch() -> std::path::PathBuf {
    static S: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    S.get_or_init(|| {
        let d = std::env::temp_dir().join(format!("hix_xs_scratch_{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::env::set_var("TOMBOX_SCRATCH_DIR", &d);
        d
    })
    .clone()
}

#[test]
fn title_drives_map_into_scratch_and_never_reach_the_host_drive() {
    let s = scratch();
    match map_path(r"d:\HIX\gpuhang-1.hix") {
        Mapped::Title(p) => assert_eq!(p, s.join("d").join("HIX").join("gpuhang-1.hix")),
        _ => panic!(),
    }
    match map_path(r"\\?\D:/gpuhang-latest.hix") {
        Mapped::Title(p) => assert_eq!(p, s.join("d").join("gpuhang-latest.hix")),
        _ => panic!(),
    }
    match map_path(r"t:\HIX") {
        Mapped::Title(p) => assert_eq!(p, s.join("t").join("HIX")),
        _ => panic!(),
    }
    assert!(matches!(map_path(r"d:\..\escape"), Mapped::Refused));
    assert!(matches!(map_path(r"C:\Windows\x"), Mapped::Host(_)));
    assert!(matches!(map_path("relative"), Mapped::Host(_)));
}

#[test]
fn kmd_maps_regions_and_acks_doorbell_requests_synchronously() {
    let _g = crate::tests::LOCK.lock().unwrap();
    unsafe {
        let mut out = [0u8; 16];
        let mut ret = 0u32;
        let idx = 1u32;
        assert_eq!(kmd::ioctl(kmd::IOCTL_MAP, &idx as *const u32 as *const u8, 4, out.as_mut_ptr(), 16, &mut ret), 1);
        assert_eq!(ret, 16);
        let mb = u64::from_le_bytes(out[..8].try_into().unwrap()) as usize;
        assert_eq!(u64::from_le_bytes(out[8..].try_into().unwrap()) as usize, kmd::REGION1_SIZE);
        assert_eq!(mb, kmd::region1());
        let bad = 2u32;
        assert_eq!(kmd::ioctl(kmd::IOCTL_MAP, &bad as *const u32 as *const u8, 4, out.as_mut_ptr(), 16, &mut ret), 0);
        let data = kmd::region0();
        let seq = (mb as *const u32).add(kmd::MAILBOX_SEQ / 4).read_volatile();
        (mb as *mut u32).add(kmd::MAILBOX_CMD / 4).write_volatile(0x17);
        (mb as *mut u32).add(kmd::MAILBOX_SEQ / 4).write_volatile(seq + 1);
        let ring = [kmd::DOORBELL_MAGIC, 1];
        assert_eq!(kmd::ioctl(kmd::IOCTL_DOORBELL, ring.as_ptr() as *const u8, 8, core::ptr::null_mut(), 0, &mut ret), 1);
        assert_eq!(((data + kmd::DATA_ACK) as *const u32).read_volatile(), seq + 1);
        assert_eq!(kmd::last_command(), Some(0x17));
        let wrong = [0u32, 1];
        assert_eq!(kmd::ioctl(kmd::IOCTL_DOORBELL, wrong.as_ptr() as *const u8, 8, core::ptr::null_mut(), 0, &mut ret), 0);
        assert_eq!(kmd::ioctl(0x7900_0000, core::ptr::null(), 0, core::ptr::null_mut(), 0, &mut ret), 0);
        assert_eq!(kmd::direct_graphics_call(0x7900_0000, core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut()), kmd::STATUS_NOT_SUPPORTED);
    }
}

#[test]
fn initialize_and_terminate_run_the_console_code_against_the_emulated_kmd() {
    scratch();
    let _g = crate::tests::LOCK.lock().unwrap();
    assert_ne!(console::module(), 0, "console hix_xs.dll not found (TOMBOX_HIX_XS_CONSOLE, hix_xs_console.dll beside the DLL, or a title EmbeddedXvd dump); this test needs it");
    unsafe {
        let mb = kmd::region1() as *mut u8;
        (mb.add(BLOB_COUNT) as *mut u32).write_unaligned(7);
        let first = crate::tests::init_hix(0);
        assert!(kmd::armed().is_some());
        assert_eq!((mb.add(BLOB_COUNT) as *const u32).read_unaligned(), if first { 0 } else { 7 });
        (mb.add(BLOB_COUNT) as *mut u32).write_unaligned(0);
        set_region1(mb);
        assert_eq!(SaveDeveloperBlobInfo(0xabc0, 0x20), 0);
        assert_eq!((mb.add(BLOB_COUNT) as *const u32).read_unaligned(), 1);
        assert_eq!((mb.add(BLOB_PTRS) as *const u64).read_unaligned(), 0xabc0);
        let before = kmd::requests();
        assert_eq!(TerminateHix(), 0);
        assert_eq!(kmd::requests(), before + 2);
        assert_eq!(kmd::last_command(), Some(0x17));
        assert_eq!(kmd::armed(), None, "DISARM clears the arm state set by IOCTL_ARM");
        let mb_seq = (mb.add(kmd::MAILBOX_SEQ) as *const u32).read_unaligned();
        assert_eq!(((kmd::region0() + kmd::DATA_ACK) as *const u32).read_volatile(), mb_seq);
    }
}
