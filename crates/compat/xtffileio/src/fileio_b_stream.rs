use super::*;
use crate::paths::full_path_name;
use super::xfer::host_wide;

pub const STREAM_READ_CHUNK: u32 = 0x40000;
pub const ACTION_READ: u32 = 1;
pub const ACTION_WRITE: u32 = 2;
pub const SOURCE_FILE: &str = "xbox\\devex\\xtf\\lib\\filestream\\filestreamserver.cpp";
const INVALID_HANDLE: isize = -1;
const ERROR_HANDLE_EOF: u32 = 38;
const ERROR_PATH_NOT_FOUND: u32 = 3;
const INFINITE: u32 = 0xFFFF_FFFF;

#[repr(C)]
#[derive(Default)]
struct Overlapped {
    internal: usize,
    internal_high: usize,
    offset: u32,
    offset_high: u32,
    event: isize,
}

#[repr(C)]
#[derive(Default)]
struct ByHandleInfo {
    attributes: u32,
    times: [u32; 6],
    serial: u32,
    size_high: u32,
    size_low: u32,
    links: u32,
    index_high: u32,
    index_low: u32,
}

#[allow(clashing_extern_declarations)]
#[link(name = "kernel32")]
extern "system" {
    fn CreateFileW(name: *const u16, access: u32, share: u32, sa: *mut c_void, disp: u32, flags: u32, tmpl: isize) -> isize;
    fn ReadFile(h: isize, buf: *mut u8, n: u32, read: *mut u32, ov: *mut Overlapped) -> i32;
    fn GetFileInformationByHandle(h: isize, info: *mut ByHandleInfo) -> i32;
    fn CloseHandle(h: isize) -> i32;
    fn WaitForSingleObject(h: isize, ms: u32) -> u32;
    fn GetLastError() -> u32;
}

unsafe fn get_choice(body: *mut c_void, name: &str, table: &[(&str, u32)]) -> Result<u32, HRESULT> {
    if body.is_null() {
        return Err(E_INVALIDARG);
    }
    let mut s = Bstr::null();
    let hr = get_bstr(body, name, &mut s);
    if hr < 0 {
        return Err(hr);
    }
    let v = s.to_string();
    table.iter().find(|(k, _)| k.eq_ignore_ascii_case(&v)).map(|e| e.1).ok_or(E_INVALIDARG)
}

pub unsafe fn get_access(body: *mut c_void) -> Result<u32, HRESULT> {
    get_choice(body, "Access", &[("Read", 0x8000_0000), ("Write", 0x4000_0000), ("Both", 0xC000_0000)])
}

pub unsafe fn get_share(body: *mut c_void) -> Result<u32, HRESULT> {
    get_choice(body, "Share", &[("None", 0), ("Read", 1), ("Write", 2), ("Both", 3)])
}

pub unsafe fn get_disposition(body: *mut c_void) -> Result<u32, HRESULT> {
    get_choice(body, "OpenAs", &[("CreateAlways", 2), ("CreateNew", 1), ("OpenAlways", 4), ("OpenExisting", 3), ("TruncateExisting", 5)])
}

pub unsafe fn get_file_path(body: *mut c_void) -> Result<Option<String>, HRESULT> {
    if body.is_null() {
        return Err(E_INVALIDARG);
    }
    let mut s = Bstr::null();
    let hr = get_bstr(body, "Path", &mut s);
    if hr < 0 {
        return Err(hr);
    }
    if s.0.is_null() {
        return Ok(None);
    }
    let u = bstr_units(&s);
    if !u.is_empty() && (u[0] == b'x' as u16 || u[0] == b'X' as u16) && !(u.len() > 1 && u[1] == b':' as u16) {
        if u.len() == 1 {
            return Ok(None);
        }
        return Ok(Some(String::from_utf16_lossy(&u[1..])));
    }
    Ok(Some(String::from_utf16_lossy(&u)))
}

pub fn open_server_file(path: Option<&str>, access: u32, share: u32, disp: u32) -> Result<isize, HRESULT> {
    let path = path.ok_or(E_INVALIDARG)?;
    if access & 0x3FFF_FFFF != 0 || access == 0 || share > 3 || !(1..=5).contains(&disp) {
        return Err(E_INVALIDARG);
    }
    let host = if path.is_empty() {
        Err(hr_from_win32(ERROR_PATH_NOT_FOUND))
    } else {
        full_path_name(path).and_then(|f| host_wide(&f))
    };
    let w = host?;
    let h = unsafe { CreateFileW(w.as_ptr(), access, share, null_mut(), disp, 0x80, 0) };
    if h == INVALID_HANDLE {
        let hr = hr_from_win32(unsafe { GetLastError() });
        return if hr < 0 { Err(hr) } else { Ok(h) };
    }
    Ok(h)
}

unsafe fn send_json(session: Session, body: *mut c_void) -> HRESULT {
    let m = match new_message() {
        Ok(m) => m,
        Err(e) => return e,
    };
    let hr = serialize_object(body, m.0);
    if hr < 0 {
        return hr;
    }
    session.send_more(m.0)
}

unsafe fn ack_opened_stream(session: Session, h: isize) -> HRESULT {
    let mut info = ByHandleInfo::default();
    if GetFileInformationByHandle(h, &mut info) == 0 {
        return hr_from_last_error_or_fail(GetLastError());
    }
    let ack = match new_object() {
        Ok(o) => o,
        Err(e) => return e,
    };
    let hr = set_u64(ack.0, "Length", ((info.size_high as u64) << 32) | info.size_low as u64);
    if hr < 0 {
        return hr;
    }
    send_json(session, ack.0)
}

unsafe fn do_read(session: Session, body: *mut c_void, h: isize, mut offset: u64) -> HRESULT {
    let mut requested = 0u32;
    let hr = get_u32(body, "Length", &mut requested);
    if hr < 0 {
        return hr;
    }
    let mut bits = vec![0u8; STREAM_READ_CHUNK as usize];
    let reply = match new_message() {
        Ok(m) => m,
        Err(e) => return e,
    };
    let rbody = match new_object() {
        Ok(o) => o,
        Err(e) => return e,
    };
    let mut total = 0u32;
    loop {
        let mut ov = Overlapped { offset: offset as u32, offset_high: (offset >> 32) as u32, ..Default::default() };
        let mut n = 0u32;
        let to_read = (requested - total).min(STREAM_READ_CHUNK);
        if ReadFile(h, bits.as_mut_ptr(), to_read, &mut n, &mut ov) == 0 {
            let e = GetLastError();
            if e == ERROR_HANDLE_EOF {
                n = 0;
            } else if e != 0 {
                return hr_from_win32(e);
            }
        }
        let done;
        let finalize;
        if n == 0 {
            done = true;
            finalize = true;
            set_binary(reply.0, &[]);
        } else {
            offset += n as u64;
            total += n;
            done = total >= requested;
            let hr = set_binary(reply.0, &bits[..n as usize]);
            if hr < 0 {
                return hr;
            }
            finalize = done;
        }
        if finalize {
            let hr = set_number(rbody.0, "Completed", 1.0);
            if hr < 0 {
                return hr;
            }
            let hr = serialize_object(rbody.0, reply.0);
            if hr < 0 {
                return hr;
            }
        }
        session.send_more(reply.0);
        if done {
            return S_OK;
        }
    }
}

pub unsafe fn stream_impl(session: Session, body: *mut c_void, path: Option<&str>, access: u32, share: u32, disp: u32) -> HRESULT {
    if session.0.is_null() || body.is_null() || path.is_none() {
        return E_INVALIDARG;
    }
    let h = match open_server_file(path, access, share, disp) {
        Ok(h) => h,
        Err(e) => return e,
    };
    let hr = 'exit: {
        let hr = ack_opened_stream(session, h);
        if hr < 0 {
            break 'exit hr;
        }
        let ev = match session.received_event() {
            Ok(e) => e,
            Err(e) => break 'exit e,
        };
        loop {
            let msg = match session.receive_more(DEFAULT_RECEIVE_TIMEOUT) {
                Ok(m) => m,
                Err(e) if e == E_PENDING => {
                    match WaitForSingleObject(ev, INFINITE) {
                        0 => continue,
                        0x80 | 0x102 => break 'exit E_UNEXPECTED,
                        _ => {
                            let g = GetLastError();
                            if g != 0 {
                                break 'exit hr_from_win32(g);
                            }
                            continue;
                        }
                    }
                }
                Err(e) => break 'exit e,
            };
            if msg.0.is_null() {
                continue;
            }
            let mut cmd = Com::null();
            let hr = deserialize_json_body(msg.0, &mut cmd);
            if hr < 0 {
                break 'exit hr;
            }
            let mut action = 0u32;
            let hr = get_u32(cmd.0, "Action", &mut action);
            if hr < 0 {
                break 'exit hr;
            }
            let mut offset = 0u64;
            let hr = get_u64(cmd.0, "Offset", &mut offset);
            if hr < 0 {
                break 'exit hr;
            }
            let hr = match action {
                ACTION_READ => do_read(session, cmd.0, h, offset),
                _ => E_NOTIMPL,
            };
            if hr < 0 {
                break 'exit hr;
            }
        }
    };
    if h != INVALID_HANDLE {
        CloseHandle(h);
    }
    hr
}

pub fn error_text(hr: HRESULT, line: u32) -> String {
    format!("Error 0x{:X} File {} : {}", hr as u32, SOURCE_FILE, line)
}

unsafe fn write_operation_error(response: *mut c_void, code: HRESULT, errors: &[String]) -> HRESULT {
    if response.is_null() {
        return S_OK;
    }
    let mut body = Com::null();
    let hr = deserialize_json_body(response, &mut body);
    if hr < 0 {
        return hr;
    }
    let err = match new_object() {
        Ok(o) => o,
        Err(e) => return e,
    };
    let hr = set_number(err.0, "ReturnCode", code as f64);
    if hr < 0 {
        return hr;
    }
    let hr = set_number(err.0, "Checkpoint", 0.0);
    if hr < 0 {
        return hr;
    }
    let mut arr = Com::null();
    let hr = XbtpCreateJsonArray(arr.out());
    if hr < 0 {
        return hr;
    }
    for e in errors {
        let w = wide(e);
        let hr = (vt::<JsonArrayVtbl>(arr.0).add_string)(arr.0, w.as_ptr());
        if hr < 0 {
            return hr;
        }
    }
    let n = wide("Errors");
    let hr = (vt::<JsonObjectVtbl>(err.0).set_array)(err.0, n.as_ptr(), arr.0);
    if hr < 0 {
        return hr;
    }
    let n = wide("XtfOperationError");
    let hr = (vt::<JsonObjectVtbl>(body.0).set_object)(body.0, n.as_ptr(), err.0);
    if hr < 0 {
        return hr;
    }
    serialize_response_object(body.0, response)
}

pub unsafe fn stream_operation_handler(session: Session, request: *mut c_void, response: *mut c_void) -> HRESULT {
    let mut errors: Vec<String> = Vec::new();
    let mut m_hr = S_OK;
    let mut body = Com::null();
    let mut hr = 'fin: {
        let hr = deserialize_json_body(request, &mut body);
        if hr < 0 {
            errors.push(error_text(hr, 439));
            m_hr = hr;
            break 'fin hr;
        }
        let path = match get_file_path(body.0) {
            Ok(p) => p,
            Err(e) => {
                errors.push(error_text(e, 440));
                m_hr = e;
                break 'fin e;
            }
        };
        let access = match get_access(body.0) {
            Ok(v) => v,
            Err(e) => break 'fin e,
        };
        let share = match get_share(body.0) {
            Ok(v) => v,
            Err(e) => break 'fin e,
        };
        let disp = match get_disposition(body.0) {
            Ok(v) => v,
            Err(e) => break 'fin e,
        };
        let r = stream_impl(session, body.0, path.as_deref(), access, share, disp);
        if r < 0 {
            errors.push(error_text(r, 455));
            m_hr = r;
            break 'fin r;
        }
        S_OK
    };
    if !response.is_null() && (m_hr < 0 || hr < 0) {
        let code = if m_hr < 0 { m_hr } else { hr };
        let mut w = write_operation_error(response, code, &errors);
        if w < 0 && m_hr < 0 {
            w = write_operation_error(response, m_hr, &errors);
        }
        if w >= 0 {
            hr = S_OK;
        }
    }
    hr
}
