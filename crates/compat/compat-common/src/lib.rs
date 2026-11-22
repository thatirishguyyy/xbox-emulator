//! Shared call-logging for the compat DLLs.
//!
//! Every stubbed export records a line so we can reconstruct the exact runtime
//! call order of the real game — which is how we prioritize what to implement
//! for real next. Log target: `%TOMBOX_LOG%` if set, else
//! `%TEMP%\tombox_compat.log`. Appends; one open per call (robust across DLLs,
//! fast enough for this purpose).

use std::io::Write;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

fn log_path() -> &'static std::path::PathBuf {
    static PATH: OnceLock<std::path::PathBuf> = OnceLock::new();
    PATH.get_or_init(|| {
        if let Ok(p) = std::env::var("TOMBOX_LOG") {
            return std::path::PathBuf::from(p);
        }
        let mut d = std::env::temp_dir();
        d.push("tombox_compat.log");
        d
    })
}

/// Record a call from `module` (e.g. "xmem") to `func` with a short `detail`.
pub mod net_guard;
pub mod gdk_user;
pub mod gdk_store;
pub mod gdk;
pub mod gamepad;
pub mod host_input;
pub mod gameinput_ext;
pub mod online;
pub mod os_host;
pub mod timer;
pub mod live;
pub mod title_os;
pub mod kmd_bridge;
pub mod private_heap;

#[global_allocator]
static TB_ALLOC: private_heap::PrivateHeap = private_heap::PrivateHeap;

/// Value of `key=` in `tombox.cfg` next to the host executable (user-side configuration that
/// lives with the game data, not in the repository).
pub fn cfg(key: &str) -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let text = std::fs::read_to_string(exe.parent()?.join("tombox.cfg")).ok()?;
    let prefix = format!("{key}=");
    text.lines().find_map(|l| l.trim().strip_prefix(prefix.as_str()).map(|v| v.trim().to_string()))
}

/// Log a compat-layer event. The first call from each compat DLL also installs the
/// in-process network guard (see `net_guard`), which happens before the title's online
/// subsystems start because the allocator/runtime shims log during early startup.
pub fn record(module: &str, func: &str, detail: &str) {
    core::hint::black_box(&gdk::EARLY_INIT);
    net_guard::ensure();
    gamepad::ensure();
    os_host::ensure();
    record_raw(module, func, detail);
}

/// Log without triggering the network guard (used by the guard itself).
pub fn record_raw(module: &str, func: &str, detail: &str) {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path())
    {
        let _ = f.write_all(format!("{ts} [{module}] {func} {detail}\n").as_bytes());
    }
}

/// Common HRESULTs.
pub const S_OK: i32 = 0;
pub const E_NOTIMPL: i32 = 0x8000_4001u32 as i32;
pub const E_FAIL: i32 = 0x8000_4005u32 as i32;
