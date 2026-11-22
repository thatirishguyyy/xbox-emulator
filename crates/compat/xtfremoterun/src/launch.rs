use crate::console::{ck, w};
use crate::variant::ConsoleCopy;
use crate::win::*;
use core::ffi::c_void;
use core::ptr::{null, null_mut};
use std::path::PathBuf;
use std::sync::Mutex;
use xtfmsg::sys::*;

pub const NT_TOOLS_JOB: &str = "\\??\\JobObjectXboxTools";
pub const NT_TITLE_JOB: &str = "\\??\\JobObjectXboxTitle";
pub const HOST_TOOLS_JOB: &str = "Global\\XtfHostControlledToolsJobObject";
pub const HOST_TITLE_JOB: &str = "Global\\XtfHostControlledTitleJobObject";
pub const DEFAULT_WORKING_DIRECTORY: &str = "%SystemRoot%";
pub const STATUS_OBJECT_NAME_NOT_FOUND: u32 = 0xC000_0034;
pub const FACILITY_NT_BIT: u32 = 0x1000_0000;
pub const RUN_FLAG_TOOLS_JOB: u32 = 4;
pub const RUN_FLAG_KEEP_TOKEN: u32 = 2;
pub const ATTRIBUTE_LIST_SIZE: usize = 0x30;
pub const PROC_THREAD_ATTRIBUTE_HANDLE_LIST: usize = 0x2_0002;
pub const DISABLE_MAX_PRIVILEGE: u32 = 1;

pub unsafe fn inherited_handles(si: *const StartupInfoExW) -> Vec<HANDLE> {
    let mut v: Vec<HANDLE> = Vec::new();
    if (*si).si.flags & STARTF_USESTDHANDLES == 0 {
        return v;
    }
    for h in [(*si).si.std_input, (*si).si.std_output, (*si).si.std_error] {
        if !h.is_null() && h != INVALID_HANDLE_VALUE && !v.contains(&h) {
            v.push(h);
        }
    }
    v
}

static JOBS: Mutex<Vec<(String, usize)>> = Mutex::new(Vec::new());
static IMAGES: Mutex<Vec<(String, PathBuf)>> = Mutex::new(Vec::new());

pub(crate) fn console_key(console_path: &str) -> Option<String> {
    console_sandbox::fs::try_resolve(console_path).ok().map(|(c, _)| c.to_ascii_lowercase())
}

pub(crate) fn register_job(name: &str, job: HANDLE) {
    let mut j = JOBS.lock().unwrap_or_else(|e| e.into_inner());
    j.retain(|(n, _)| n != name);
    j.push((name.to_string(), job as usize));
}

#[cfg(test)]
pub(crate) fn unregister_job(name: &str) {
    JOBS.lock().unwrap_or_else(|e| e.into_inner()).retain(|(n, _)| n != name);
}

pub(crate) fn register_image(console_path: &str, host: PathBuf) {
    let Some(key) = console_key(console_path) else { return };
    let mut i = IMAGES.lock().unwrap_or_else(|e| e.into_inner());
    i.retain(|(n, _)| *n != key);
    i.push((key, host));
}

unsafe fn duplicate(h: usize) -> Result<HANDLE, u32> {
    let mut out: HANDLE = null_mut();
    if DuplicateHandle(GetCurrentProcess(), h as HANDLE, GetCurrentProcess(), &mut out, 0, 0, DUPLICATE_SAME_ACCESS) == 0 {
        return Err(xtfmsg::sys::GetLastError());
    }
    Ok(out)
}

pub unsafe fn nt_open_job_object(name: &str) -> (u32, HANDLE) {
    let found = JOBS.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|(n, _)| n.starts_with("\\??\\") && n.eq_ignore_ascii_case(name)).map(|(_, h)| *h);
    match found {
        None => (STATUS_OBJECT_NAME_NOT_FOUND, null_mut()),
        Some(h) => match duplicate(h) {
            Ok(d) => (0, d),
            Err(e) => (0xC000_0000 | (e & 0xffff), null_mut()),
        },
    }
}

pub unsafe fn open_job_object_w(name: &str) -> HANDLE {
    let found = JOBS.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|(n, _)| !n.starts_with("\\??\\") && n == name).map(|(_, h)| *h);
    match found {
        None => {
            SetLastError(ERROR_FILE_NOT_FOUND);
            null_mut()
        }
        Some(h) => match duplicate(h) {
            Ok(d) => d,
            Err(e) => {
                SetLastError(e);
                null_mut()
            }
        },
    }
}

fn split_image(cmd: &str) -> (String, String) {
    let c = cmd.trim_start();
    if let Some(r) = c.strip_prefix('"') {
        return match r.find('"') {
            Some(i) => (r[..i].to_string(), r[i + 1..].to_string()),
            None => (r.to_string(), String::new()),
        };
    }
    match c.find(|ch: char| ch == ' ' || ch == '\t') {
        Some(i) => (c[..i].to_string(), c[i..].to_string()),
        None => (c.to_string(), String::new()),
    }
}

pub fn resolve_image(cmd: &str) -> Result<String, u32> {
    let (image, rest) = split_image(cmd);
    let mut cands = vec![image.clone()];
    if !image.to_ascii_lowercase().ends_with(".exe") {
        cands.push(format!("{image}.exe"));
    }
    let images = IMAGES.lock().unwrap_or_else(|e| e.into_inner());
    for key in cands.iter().filter_map(|c| console_key(c)) {
        if let Some((_, host)) = images.iter().find(|(n, _)| *n == key) {
            return Ok(format!("\"{}\"{}", host.display(), rest));
        }
    }
    Err(ERROR_FILE_NOT_FOUND)
}

pub fn resolve_directory(dir: &str) -> Result<PathBuf, u32> {
    console_sandbox::fs::ensure_system_drive();
    match console_sandbox::fs::resolve(dir) {
        Some((_, host)) if host.is_dir() => Ok(host),
        _ => Err(ERROR_DIRECTORY),
    }
}

fn expand_environment_strings(p: *const u16) -> Option<String> {
    if p.is_null() {
        return None;
    }
    let s = console_sandbox::system::expand(&from_wide(p));
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

fn hr_last() -> HRESULT {
    last_error_or_fail()
}

pub unsafe fn remote_run_create_process(copy: ConsoleCopy, cmd: *const u16, wd: *const u16, flags: u32, si: *mut StartupInfoExW, pi: *mut ProcessInformation) -> HRESULT {
    const F: &str = "RemoteRunCreateProcess";
    crate::title::ensure();
    *pi = ProcessInformation::zeroed();
    let default_wd = w(DEFAULT_WORKING_DIRECTORY);
    let wd = if wd.is_null() || *wd == 0 { default_wd.as_ptr() } else { wd };
    let command = expand_environment_strings(cmd);
    ck(F, copy.create_process_line(432), "strCommandLine.ExpandEnvironmentStrings(pszCommandLine)", S_OK, "");
    let directory = expand_environment_strings(wd);
    ck(F, copy.create_process_line(433), "strWorkingDirectory.ExpandEnvironmentStrings(pszWorkingDirectory)", S_OK, "");
    let mut tools = flags & RUN_FLAG_TOOLS_JOB != 0;
    let nt_job = loop {
        let (status, h) = nt_open_job_object(if tools { NT_TOOLS_JOB } else { NT_TITLE_JOB });
        if status != 0 && tools {
            tools = false;
            continue;
        }
        if (status as i32) < 0 {
            return (status | FACILITY_NT_BIT) as i32;
        }
        break OwnedHandle(h);
    };
    let host_job = if copy.has_host_jobs() { OwnedHandle(open_job_object_w(if tools { HOST_TOOLS_JOB } else { HOST_TITLE_JOB })) } else { OwnedHandle::null() };
    if copy.has_host_jobs() && host_job.get().is_null() {
        let hr = hr_from_wait(xtfmsg::sys::GetLastError());
        ck(F, 503, "HRESULT_FROM_WIN32(GetLastError())", hr, "");
        if hr < 0 {
            return hr;
        }
    }
    let handles = inherited_handles(si);
    let count = if handles.is_empty() { 1 } else { 2 };
    let mut size = 0usize;
    InitializeProcThreadAttributeList(null_mut(), count, 0, &mut size);
    let mut list = vec![0u64; size.max(ATTRIBUTE_LIST_SIZE) / 8 + 1];
    let mut size = list.len() * 8;
    let list_ptr = list.as_mut_ptr() as *mut c_void;
    if InitializeProcThreadAttributeList(list_ptr, count, 0, &mut size) == 0 {
        return hr_last();
    }
    let hr = 'attr: {
        let jobs: [HANDLE; 2] = [nt_job.get(), host_job.get()];
        let list_bytes = if copy.has_host_jobs() { 2 * core::mem::size_of::<HANDLE>() } else { core::mem::size_of::<HANDLE>() };
        if UpdateProcThreadAttribute(list_ptr, 0, PROC_THREAD_ATTRIBUTE_JOB_LIST, jobs.as_ptr() as *const c_void, list_bytes, null_mut(), null_mut()) == 0 {
            break 'attr hr_last();
        }
        if !handles.is_empty() && UpdateProcThreadAttribute(list_ptr, 0, PROC_THREAD_ATTRIBUTE_HANDLE_LIST, handles.as_ptr() as *const c_void, handles.len() * core::mem::size_of::<HANDLE>(), null_mut(), null_mut()) == 0 {
            break 'attr hr_last();
        }
        (*si).attribute_list = list_ptr;
        let mut token = OwnedHandle::null();
        let mut restricted = OwnedHandle::null();
        if flags & RUN_FLAG_KEEP_TOKEN == 0 {
            if OpenProcessToken(GetCurrentProcess(), TOKEN_ALL_ACCESS, &mut token.0) == 0 {
                break 'attr hr_last();
            }
            if CreateRestrictedToken(token.get(), DISABLE_MAX_PRIVILEGE, 0, null(), 0, null(), 0, null(), &mut restricted.0) == 0 {
                break 'attr hr_last();
            }
        }
        let host_cmd = match command.as_deref().map(resolve_image) {
            Some(Ok(c)) => c,
            Some(Err(e)) => {
                SetLastError(e);
                break 'attr hr_last();
            }
            None => {
                SetLastError(ERROR_FILE_NOT_FOUND);
                break 'attr hr_last();
            }
        };
        let host_dir = match directory.as_deref().map(resolve_directory) {
            Some(Ok(d)) => Some(d),
            Some(Err(e)) => {
                SetLastError(e);
                break 'attr hr_last();
            }
            None => None,
        };
        let mut cmd_w = w(&host_cmd);
        let dir_w = host_dir.map(|d| w(&d.to_string_lossy()));
        let creation = (CONSOLE_CREATION_FLAGS & !(CREATE_NEW_CONSOLE | CREATE_BREAKAWAY_FROM_JOB)) | CREATE_NO_WINDOW;
        let inherit = if handles.is_empty() { 0 } else { 1 };
        if CreateProcessAsUserW(restricted.get(), null(), cmd_w.as_mut_ptr(), null(), null(), inherit, creation, null(), dir_w.as_ref().map_or(null(), |d| d.as_ptr()), si, pi) == 0 {
            break 'attr hr_last();
        }
        let mut process_mask = 0usize;
        let mut system_mask = 0usize;
        if GetProcessAffinityMask((*pi).process, &mut process_mask, &mut system_mask) != 0 {
            SetProcessAffinityMask((*pi).process, process_mask);
        }
        ResumeThread((*pi).thread);
        FACILITY_NT_BIT as i32
    };
    DeleteProcThreadAttributeList(list_ptr);
    (*si).attribute_list = null_mut();
    hr
}
