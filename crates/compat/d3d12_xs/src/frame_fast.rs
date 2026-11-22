use core::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, Ordering};
use std::sync::OnceLock;

static HOST_PERIOD: AtomicI64 = AtomicI64::new(0);
static HOST_MHZ: AtomicU32 = AtomicU32::new(0);
static LOGGED: AtomicI64 = AtomicI64::new(0);
static LOGGED_OFF: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Uncapped,
    Console,
    Host,
    Hz(u32),
}

fn mode() -> Mode {
    static CFG: OnceLock<Option<String>> = OnceLock::new();
    static LIVE: crate::live_sw::Text = crate::live_sw::Text::new("TOMBOX_FRAME_HZ", "frame_hz");
    let v = LIVE.get().or_else(|| CFG.get_or_init(|| compat_common::cfg("frame_hz")).clone()).filter(|v| !v.trim().is_empty() && v.trim() != "auto");
    let m = match v.as_deref().map(str::trim) {
        None => policy().mode(),
        Some("console" | "0" | "native") => Mode::Console,
        Some("uncapped" | "off") => Mode::Uncapped,
        Some("host") => Mode::Host,
        Some(n) => n.parse::<u32>().ok().filter(|&h| (30..=1000).contains(&h)).map(Mode::Hz).unwrap_or(Mode::Console),
    };
    static LAST: std::sync::Mutex<Option<Mode>> = std::sync::Mutex::new(None);
    let mut l = LAST.lock().unwrap_or_else(|e| e.into_inner());
    if l.is_some_and(|p| p != m) {
        LOGGED.store(0, Ordering::Relaxed);
        LOGGED_OFF.store(false, Ordering::Relaxed);
    }
    *l = Some(m);
    m
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Timing {
    Variable,
    Fixed,
    Unknown,
}

impl Timing {
    fn mode(self) -> Mode {
        if self == Timing::Variable { Mode::Uncapped } else { Mode::Console }
    }
}

pub fn parse_timing(text: &str) -> Timing {
    match text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("") {
        "variable" => Timing::Variable,
        "fixed" | "fixed-step" => Timing::Fixed,
        _ => Timing::Unknown,
    }
}

fn policy() -> Timing {
    static P: OnceLock<Timing> = OnceLock::new();
    *P.get_or_init(|| {
        let title = std::env::current_exe().ok().and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned())).unwrap_or_else(|| "title".into());
        let path = tombox_xshader::shader_cache::title_dir(&title).join("timing");
        let t = std::fs::read_to_string(&path).map(|s| parse_timing(&s)).unwrap_or(Timing::Unknown);
        compat_common::record("d3d12_xs", "frame.clock", &match t {
            Timing::Variable => format!("timing policy for {title}: variable (simulation uses real delta time), so frame events are uncapped; from {}", path.display()),
            Timing::Fixed => format!("timing policy for {title}: fixed-step (one frame event is one logic tick), so frame events stay at the title's native interval to keep game speed correct; from {}", path.display()),
            Timing::Unknown => format!("timing policy for {title}: unknown ({} missing or unreadable), so frame events stay at the title's native interval to keep game speed correct; write variable or fixed there, or test with TOMBOX_FRAME_HZ=uncapped|host|<hz>", path.display()),
        });
        t
    })
}

pub fn uncapped() -> bool {
    mode() == Mode::Uncapped
}

pub fn host_period(period_qpc: i64, refresh_mhz: u32) {
    if period_qpc > 0 {
        HOST_PERIOD.store(period_qpc, Ordering::Relaxed);
        HOST_MHZ.store(refresh_mhz, Ordering::Relaxed);
    }
}

fn target(console: i64) -> Option<i64> {
    let t = match mode() {
        Mode::Console | Mode::Uncapped => return None,
        Mode::Host => HOST_PERIOD.load(Ordering::Relaxed),
        Mode::Hz(h) => crate::frame::qpf() / h as i64,
    };
    if t <= 0 || t * 100 >= console * 95 {
        return None;
    }
    Some(t)
}

pub fn retime<T>(console: i64, events: Vec<(usize, i64, isize, T)>) -> (i64, Vec<(usize, i64, isize, T)>) {
    let Some(t) = target(console) else {
        if !LOGGED_OFF.swap(true, Ordering::Relaxed) {
            compat_common::record("d3d12_xs", "frame.clock", if mode() == Mode::Uncapped {
                "frame events uncapped: each present releases the next frame at once; the title's interval only applies when no present arrives in time (TOMBOX_FRAME_HZ=native|host|<hz> caps it)"
            } else {
                "frame events at the title's interval (TOMBOX_FRAME_HZ=console, or the host refresh is not faster)"
            });
        }
        return (console, events);
    };
    if LOGGED.swap(t, Ordering::Relaxed) != t {
        let f = crate::frame::qpf() as f64;
        compat_common::record("d3d12_xs", "frame.clock", &format!(
            "frame events follow the host: interval {:.0}us -> {:.0}us ({:.1} Hz, host refresh {}.{:03} Hz); TOMBOX_FRAME_HZ=console restores the title's interval",
            console as f64 * 1e6 / f, t as f64 * 1e6 / f, f / t as f64,
            HOST_MHZ.load(Ordering::Relaxed) / 1000, HOST_MHZ.load(Ordering::Relaxed) % 1000));
    }
    let events = events.into_iter().map(|(ty, off, h, d)| (ty, (off as i128 * t as i128 / console as i128) as i64, h, d)).collect();
    (t, events)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timing_policy_file_parses_and_unknown_stays_native() {
        assert_eq!(parse_timing("variable
"), Timing::Variable);
        assert_eq!(parse_timing("
 fixed-step"), Timing::Fixed);
        assert_eq!(parse_timing("fixed"), Timing::Fixed);
        assert_eq!(parse_timing(""), Timing::Unknown);
        assert!(Timing::Unknown.mode() == Mode::Console);
        assert!(Timing::Fixed.mode() == Mode::Console);
        assert!(Timing::Variable.mode() == Mode::Uncapped);
    }
}
