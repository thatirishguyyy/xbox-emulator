use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

pub const KIND_KBM: u32 = 0x0000_0030;

static PAD: OnceLock<bool> = OnceLock::new();
static KBM: OnceLock<bool> = OnceLock::new();
static TAKEN: AtomicBool = AtomicBool::new(false);
static HOOKS: Mutex<Vec<fn()>> = Mutex::new(Vec::new());

pub fn hide_choice(v: Option<&str>) -> bool {
    matches!(v.map(str::trim), Some("0") | Some("off") | Some("false"))
}

fn read(cell: &OnceLock<bool>, var: &str, what: &str) -> bool {
    *cell.get_or_init(|| {
        let h = hide_choice(std::env::var(var).ok().as_deref());
        if h {
            crate::record_raw("host input", var, &format!("{var}=0: {what}"));
        }
        h
    })
}

pub fn pad_hidden() -> bool {
    !taken_over() && read(&PAD, "TOMBOX_HOST_PAD", "host gamepads are hidden from the title (no host gamepad device callbacks, host gamepad readings dropped); only the virtual pad is visible")
}

pub fn kbm_hidden() -> bool {
    !taken_over() && read(&KBM, "TOMBOX_HOST_KBM", "host keyboard and mouse are hidden from the title (GameInput keyboard/mouse readings and device callbacks dropped, Win32 keyboard/mouse window messages dropped, overlay hotkey ignored)")
}

pub fn test_run() -> bool {
    static T: OnceLock<bool> = OnceLock::new();
    *T.get_or_init(|| std::env::var("TOMBOX_TEST_RUN").map(|v| v.trim() == "1").unwrap_or(false))
}

pub const TAKEN_ENV: &str = "TOMBOX_TAKEN";

pub fn taken_over() -> bool {
    if TAKEN.load(Ordering::Acquire) {
        return true;
    }
    if !test_run() {
        return false;
    }
    static START: OnceLock<std::time::Instant> = OnceLock::new();
    static LAST: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let now = START.get_or_init(std::time::Instant::now).elapsed().as_millis() as u64 + 1000;
    let last = LAST.load(Ordering::Relaxed);
    if now < last + 100 || LAST.compare_exchange(last, now, Ordering::AcqRel, Ordering::Relaxed).is_err() {
        return false;
    }
    match std::env::var(TAKEN_ENV) {
        Ok(v) if !v.trim().is_empty() => {
            if !TAKEN.swap(true, Ordering::AcqRel) {
                crate::record_raw("host input", "takeover", &format!("takeover seen from another module ({}): host gamepad, keyboard and mouse now visible to the title", v.trim()));
                run_hooks();
            }
            true
        }
        _ => false,
    }
}

pub fn on_takeover(f: fn()) {
    let mut g = HOOKS.lock().unwrap_or_else(|e| e.into_inner());
    if !g.iter().any(|h| *h as usize == f as usize) {
        g.push(f);
    }
}

fn run_hooks() {
    let hooks: Vec<fn()> = HOOKS.lock().unwrap_or_else(|e| e.into_inner()).clone();
    for h in hooks {
        h();
    }
}

pub fn user_play_record(pid: u32, title: &str, at: &str) -> String {
    format!("script_pid=0 game_pid={pid} title={title} started={at} takeover=1")
}

pub fn take_over(reason: &str) {
    if !test_run() || TAKEN.swap(true, Ordering::AcqRel) {
        return;
    }
    let already = std::env::var(TAKEN_ENV).map(|v| !v.trim().is_empty()).unwrap_or(false);
    if !already {
        std::env::set_var(TAKEN_ENV, reason);
        let pid = std::process::id();
        let title = std::env::current_exe().ok().and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned())).unwrap_or_default();
        let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0).to_string();
        crate::record_raw("host input", "takeover", &format!("user took over the test window ({reason}): host gamepad, keyboard and mouse are visible to the title from now on, the pad script stops and this run counts as the user playing"));
        for (var, body) in [("TOMBOX_USER_PLAYING_FILE", user_play_record(pid, &title, &at)), ("TOMBOX_TAKEOVER_FILE", format!("pid={pid} reason={reason} at={at}"))] {
            if let Ok(path) = std::env::var(var) {
                if !path.trim().is_empty() {
                    if let Err(e) = std::fs::write(path.trim(), body) {
                        crate::record_raw("host input", "takeover", &format!("could not write {var} {}: {e}", path.trim()));
                    }
                }
            }
        }
    }
    run_hooks();
}

pub fn user_press_message(msg: u32) -> bool {
    matches!(msg, 0x0100 | 0x0104 | 0x0201 | 0x0204 | 0x0207 | 0x020B)
}

pub fn kbm_message(msg: u32) -> bool {
    msg == 0x00FF || (0x0100..=0x0109).contains(&msg) || (0x0200..=0x020E).contains(&msg) || (0x02A0..=0x02A3).contains(&msg)
}

pub fn hide_reading(kind: u32, pad_hidden: bool, kbm_hidden: bool) -> bool {
    (pad_hidden && kind & crate::gamepad::KIND_GAMEPAD != 0) || (kbm_hidden && kind & KIND_KBM != 0)
}

pub fn hide_device(supported: u32, pad_hidden: bool, kbm_hidden: bool) -> bool {
    hide_reading(supported, pad_hidden, kbm_hidden)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takeover_record_names_the_game_pid_for_stale_checks() {
        let r = user_play_record(4242, "GbxGame", "1795340000");
        assert!(r.contains("game_pid=4242"));
        assert!(r.contains("script_pid=0"));
        assert!(r.contains("takeover=1"));
    }

    #[test]
    fn zero_off_false_hide() {
        assert!(hide_choice(Some("0")));
        assert!(hide_choice(Some(" off ")));
        assert!(hide_choice(Some("false")));
        assert!(!hide_choice(Some("1")));
        assert!(!hide_choice(None));
    }

    #[test]
    fn keyboard_and_mouse_messages_are_input() {
        for m in [0x00FF, 0x0100, 0x0101, 0x0102, 0x0104, 0x0105, 0x0109, 0x0200, 0x0201, 0x0202, 0x020A, 0x020E, 0x02A1, 0x02A3] {
            assert!(kbm_message(m), "{m:#x}");
        }
        for m in [0x0012, 0x000F, 0x0005, 0x0006, 0x0007, 0x0008, 0x001C, 0x00A0, 0x00A1, 0x0112, 0x0113, 0x0020, 0x8000] {
            assert!(!kbm_message(m), "{m:#x}");
        }
    }

    #[test]
    fn readings_hidden_by_switch() {
        let pad = crate::gamepad::KIND_GAMEPAD;
        assert!(hide_reading(pad, true, false));
        assert!(!hide_reading(pad, false, true));
        assert!(hide_reading(0x10, false, true));
        assert!(hide_reading(0x20, false, true));
        assert!(hide_reading(0x30, false, true));
        assert!(!hide_reading(0x30, true, false));
        assert!(!hide_reading(0x10, false, false));
        assert!(hide_device(0x30, false, true));
        assert!(!hide_device(pad | 0xE, false, true));
    }
}
