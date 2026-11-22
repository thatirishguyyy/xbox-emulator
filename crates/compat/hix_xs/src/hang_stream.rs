pub const CMD_HANG_SUMMARY: u32 = 0xf;
pub const STREAM_VERSION: u32 = 0x20_1001;
pub const VERSION_OFFSET: usize = 0x40_a050;
pub const STREAM_OFFSET: usize = 0xa050;
pub const STREAM_LEN: usize = 0x2_7377;
pub const CONSOLE_XBOX: u8 = 0;
pub const CONSOLE_SCARLETT: u8 = 1;
pub const CONSOLE_LOCKHART: u8 = 2;
pub const GCEA_STATUS_OFFSET: usize = 0x2_1826;
pub const GCEA_COUNT: usize = 20;
pub const GCEA_STRIDE: usize = 8;
pub const GCEA_NO_ERROR: u32 = 0x300;

pub fn console_type_for_mode(mode: Option<u32>) -> u8 {
    match mode {
        Some(0x64) | Some(0x68) => CONSOLE_SCARLETT,
        Some(0x60) => CONSOLE_LOCKHART,
        _ => CONSOLE_XBOX,
    }
}

pub fn emulated_console_type() -> u8 {
    console_type_for_mode(compat_common::gdk::hook::console_mode(&compat_common::gdk::identity::device()))
}

pub fn summary_stream_for(console: u8) -> Vec<u8> {
    let mut s = vec![0u8; STREAM_LEN];
    s[0] = console;
    for i in 0..GCEA_COUNT {
        let o = GCEA_STATUS_OFFSET + i * GCEA_STRIDE;
        s[o..o + 4].copy_from_slice(&GCEA_NO_ERROR.to_le_bytes());
    }
    s
}

pub fn summary_stream() -> Vec<u8> {
    summary_stream_for(emulated_console_type())
}

pub const E_NOTIMPL: u32 = 0x8000_4001;

pub const CMD_DISARM: u32 = 0x17;
pub const CMD_HALT_WAVES: u32 = 0x28;
pub const CMD_UNHALT_WAVES: u32 = 0x29;
pub const CMD_SQ_DUMP: u32 = 0x14;
pub const CMD_LIVE_DEBUG: u32 = 0x1b;

pub fn served(cmd: u32) -> bool {
    matches!(cmd, CMD_HANG_SUMMARY | CMD_DISARM)
}

pub unsafe fn respond(cmd: u32, data: usize, _mailbox: usize) -> (u32, u32) {
    match cmd {
        CMD_HANG_SUMMARY => {
            ((data + VERSION_OFFSET) as *mut u32).write_volatile(STREAM_VERSION);
            let s = summary_stream();
            core::ptr::copy_nonoverlapping(s.as_ptr(), (data + STREAM_OFFSET) as *mut u8, s.len());
            (0, s.len() as u32)
        }
        CMD_DISARM => {
            crate::kmd::disarm();
            (0, 0)
        }
        CMD_HALT_WAVES | CMD_UNHALT_WAVES => {
            compat_common::record_raw("hix_xs.dll", "XgsKmd10", &format!("command {cmd:#x} ({}) needs the console GPU's SQ wave control; the PC D3D12 device exposes none, returning E_NOTIMPL", if cmd == CMD_HALT_WAVES { "halt waves" } else { "unhalt waves" }));
            (E_NOTIMPL, 0)
        }
        _ => (E_NOTIMPL, 0),
    }
}
