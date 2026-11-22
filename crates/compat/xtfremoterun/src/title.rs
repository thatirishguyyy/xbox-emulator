use crate::launch::{register_image, register_job, HOST_TITLE_JOB, NT_TITLE_JOB};
use crate::win::*;
use xtfmsg::sys::HANDLE;
use core::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

pub const DEFAULT_GAME_DRIVE: char = 'G';
pub const CONFIG_NAMES: [&str; 2] = ["MicrosoftGame.config", "MicrosoftGame.Config"];
const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION: u32 = 9;
const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x2000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TitleExecutable {
    pub console_path: String,
    pub host_path: PathBuf,
}

#[derive(Debug, Default)]
pub struct TitleState {
    pub dir: Option<PathBuf>,
    pub drive: char,
    pub executables: Vec<TitleExecutable>,
    pub rejected: Vec<String>,
    pub job: usize,
    pub job_error: u32,
}

#[repr(C)]
struct ExtendedLimit {
    basic: [u64; 8],
    io: [u64; 6],
    process_memory: usize,
    job_memory: usize,
    peak_process: usize,
    peak_job: usize,
}

static STATE: OnceLock<TitleState> = OnceLock::new();
static INSTALLED: Mutex<bool> = Mutex::new(false);

pub fn executable_names(config: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = config;
    while let Some(i) = rest.find("<Executable") {
        let after = &rest[i + "<Executable".len()..];
        let end = after.find('>').unwrap_or(after.len());
        let el = &after[..end];
        if el.starts_with(|c: char| c.is_whitespace() || c == '/') {
            if let Some(a) = el.find(" Name=\"").map(|a| a + 7).or_else(|| el.find("\tName=\"").map(|a| a + 7)) {
                if let Some(b) = el[a..].find('"') {
                    out.push(el[a..a + b].to_string());
                }
            }
        }
        rest = &after[end..];
    }
    out
}

pub fn safe_relative_name(name: &str) -> Option<String> {
    let n = name.replace('/', "\\");
    if n.is_empty() || n.starts_with('\\') || n.contains(':') {
        return None;
    }
    let parts: Vec<&str> = n.split('\\').collect();
    if parts.iter().any(|p| p.is_empty() || *p == "." || *p == "..") {
        return None;
    }
    Some(n)
}

fn read_config(dir: &Path) -> Option<String> {
    CONFIG_NAMES.iter().find_map(|n| std::fs::read(dir.join(n)).ok()).map(|b| String::from_utf8_lossy(&b).trim_start_matches('\u{feff}').to_string())
}

pub fn executables_for(dir: &Path, drive: char) -> (Vec<TitleExecutable>, Vec<String>) {
    let mut ok = Vec::new();
    let mut rejected = Vec::new();
    let Some(text) = read_config(dir) else { return (ok, rejected) };
    for name in executable_names(&text) {
        let Some(rel) = safe_relative_name(&name) else {
            rejected.push(name);
            continue;
        };
        let host = dir.join(&rel);
        if !host.is_file() {
            rejected.push(name);
            continue;
        }
        let console_path = format!("{drive}:\\{rel}");
        if crate::launch::console_key(&console_path).is_none() {
            rejected.push(name);
            continue;
        }
        ok.push(TitleExecutable { console_path, host_path: host });
    }
    (ok, rejected)
}

unsafe fn create_title_job() -> Result<usize, u32> {
    let job = CreateJobObjectW(core::ptr::null(), core::ptr::null());
    if job.is_null() {
        return Err(xtfmsg::sys::GetLastError());
    }
    let mut info: ExtendedLimit = core::mem::zeroed();
    *(&mut info.basic[2] as *mut u64 as *mut u32) = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    if SetInformationJobObject(job, JOB_OBJECT_EXTENDED_LIMIT_INFORMATION, &info as *const ExtendedLimit as *const c_void, core::mem::size_of::<ExtendedLimit>() as u32) == 0 {
        let e = xtfmsg::sys::GetLastError();
        CloseHandle(job);
        return Err(e);
    }
    Ok(job as usize)
}

#[cfg(test)]
pub(crate) static TEST_TITLE_DIR: OnceLock<PathBuf> = OnceLock::new();

fn running_title() -> (Option<PathBuf>, char) {
    #[cfg(test)]
    if let Some(d) = TEST_TITLE_DIR.get() {
        return (Some(d.clone()), DEFAULT_GAME_DRIVE);
    }
    let drive = compat_common::gdk::fs::game_drive().map(|d| d.letter).unwrap_or(DEFAULT_GAME_DRIVE);
    (compat_common::gdk::title().dir.clone(), drive)
}

fn build_state() -> TitleState {
    let (dir, drive) = running_title();
    let Some(dir) = dir else { return TitleState { drive, ..Default::default() } };
    let (executables, rejected) = executables_for(&dir, drive);
    let (job, job_error) = match unsafe { create_title_job() } {
        Ok(j) => (j, 0),
        Err(e) => (0, e),
    };
    TitleState { dir: Some(dir), drive, executables, rejected, job, job_error }
}

pub fn state() -> &'static TitleState {
    STATE.get_or_init(build_state)
}

pub fn install_jobs() {
    let s = state();
    if s.job == 0 {
        return;
    }
    register_job(NT_TITLE_JOB, s.job as HANDLE);
    register_job(HOST_TITLE_JOB, s.job as HANDLE);
}

pub fn ensure() {
    let mut done = INSTALLED.lock().unwrap_or_else(|e| e.into_inner());
    if *done {
        return;
    }
    let s = state();
    for e in &s.executables {
        register_image(&e.console_path, e.host_path.clone());
    }
    install_jobs();
    *done = true;
}
