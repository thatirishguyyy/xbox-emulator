use crate::com::*;
use std::path::PathBuf;

pub const PATH_NORMAL: u32 = 0;
pub const PATH_CONSOLE: u32 = 1;
pub const PATH_APPLICATION: u32 = 2;

pub const ERROR_PATH_NOT_FOUND: u32 = 3;
pub const ERROR_BAD_NETPATH: u32 = 53;

pub fn path_type(name: &[u16]) -> Result<(u32, usize), HRESULT> {
    if name.is_empty() || name[0] == 0 {
        return Err(E_INVALIDARG);
    }
    let at = |i: usize| name.get(i).copied().unwrap_or(0);
    if at(0) & 0xffdf == b'X' as u16 {
        let c = at(1);
        if ((b'A' as u16..=b'Z' as u16).contains(&c) || (b'a' as u16..=b'z' as u16).contains(&c)) && at(2) == b':' as u16 && at(3) == b'\\' as u16 {
            return Ok((PATH_CONSOLE, 1));
        }
    }
    if at(0) == b'{' as u16 {
        let end = name.iter().position(|&c| c == 0).unwrap_or(name.len());
        let pat = [b'}' as u16, b':' as u16, b'\\' as u16];
        if let Some(p) = name[1..end].windows(3).position(|w| w == pat) {
            return Ok((PATH_APPLICATION, 1 + p + 3));
        }
    }
    Ok((PATH_NORMAL, 0))
}

pub const CURRENT_DIR: &str = "C:\\Windows\\System32";

pub fn full_path_name(path: &str) -> Result<String, HRESULT> {
    let s = path.replace('/', "\\");
    if s.is_empty() {
        return Err(hr_from_win32(123));
    }
    let b: Vec<char> = s.chars().collect();
    let (prefix, rest): (String, String) = if s.starts_with("\\\\") {
        let mut parts = s[2..].splitn(3, '\\');
        let server = parts.next().unwrap_or("");
        let share = parts.next().unwrap_or("");
        let tail = parts.next().unwrap_or("");
        (format!("\\\\{server}\\{share}"), format!("\\{tail}"))
    } else if b.len() >= 2 && b[1] == ':' && b[0].is_ascii_alphabetic() {
        let r: String = b[2..].iter().collect();
        let r = if r.starts_with('\\') { r } else { format!("\\{r}") };
        (format!("{}:", b[0]), r)
    } else if s.starts_with('\\') {
        (CURRENT_DIR[..2].to_string(), s.clone())
    } else {
        (CURRENT_DIR[..2].to_string(), format!("{}\\{}", &CURRENT_DIR[2..], s))
    };
    let trailing = rest.ends_with('\\') && rest.len() > 1;
    let mut comps: Vec<String> = Vec::new();
    for c in rest.split('\\') {
        match c {
            "" | "." => {}
            ".." => {
                comps.pop();
            }
            c => {
                let t = c.trim_end_matches(['.', ' ']);
                if t.is_empty() {
                    continue;
                }
                comps.push(t.to_string());
            }
        }
    }
    let mut out = format!("{prefix}\\{}", comps.join("\\"));
    if trailing && !comps.is_empty() {
        out.push('\\');
    }
    let mut ch: Vec<char> = out.chars().collect();
    if ch.len() >= 2 && ch[1] == ':' {
        let up = ch[0].to_ascii_uppercase();
        if up.is_ascii_uppercase() {
            ch[0] = up;
        }
    }
    out = ch.into_iter().collect();
    Ok(out)
}

pub struct HostPath {
    pub wide: Vec<u16>,
    pub path: PathBuf,
}

pub fn host_path(console_full: &str) -> Result<HostPath, u32> {
    if console_full.starts_with("\\\\") {
        return Err(ERROR_BAD_NETPATH);
    }
    let b = console_full.as_bytes();
    if b.len() < 2 || b[1] != b':' {
        return Err(ERROR_PATH_NOT_FOUND);
    }
    let drive = console_sandbox::fs::drive_dir(b[0] as char);
    if !drive.is_dir() {
        return Err(ERROR_PATH_NOT_FOUND);
    }
    let (_, host) = console_sandbox::fs::try_resolve(console_full).map_err(console_sandbox::fs::win32_error)?;
    let mut s = host.to_string_lossy().to_string();
    if console_full.ends_with('\\') && !s.ends_with('\\') {
        s.push('\\');
    }
    let ext = if s.starts_with("\\\\") { format!("\\\\?\\UNC\\{}", &s[2..]) } else { format!("\\\\?\\{s}") };
    Ok(HostPath { wide: wide(&ext), path: host })
}

pub fn host_search_path(console_full: &str) -> Result<HostPath, u32> {
    let Some(cut) = console_full.rfind('\\') else { return host_path(console_full) };
    let pattern = &console_full[cut + 1..];
    if !pattern.contains(['*', '?', '<', '>', '"']) {
        return host_path(console_full);
    }
    if pattern.contains(':') || pattern.chars().any(|c| (c as u32) < 32 || c == '|' || c == '/') {
        return Err(console_sandbox::fs::ERROR_INVALID_NAME);
    }
    let dir = host_path(&console_full[..cut + 1])?;
    let mut s = dir.path.to_string_lossy().trim_end_matches('\\').to_string();
    s.push('\\');
    s.push_str(pattern);
    let ext = if s.starts_with("\\\\") { format!("\\\\?\\UNC\\{}", &s[2..]) } else { format!("\\\\?\\{s}") };
    Ok(HostPath { wide: wide(&ext), path: PathBuf::from(s) })
}

pub fn console_path_of_host(host: &std::path::Path) -> Option<String> {
    let root = console_sandbox::fs::fs_root();
    let rel = host.strip_prefix(&root).ok()?;
    let mut it = rel.components();
    let drive = it.next()?.as_os_str().to_string_lossy().to_string();
    let rest: Vec<String> = it.map(|c| c.as_os_str().to_string_lossy().to_string()).collect();
    Some(format!("{drive}:\\{}", rest.join("\\")))
}
