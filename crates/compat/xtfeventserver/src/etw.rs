use crate::com::*;
use crate::dispatcher::Dispatcher;
use crate::imports::{self, chk};
use crate::shared::*;
use crate::sys::*;
use core::ffi::c_void;
use core::ptr::null_mut;
use core::sync::atomic::{AtomicI32, AtomicU32, AtomicU64, Ordering};
use std::sync::Mutex;

pub const LOGGER_NAME_FORMAT: &str = "XtfEventServerETWLogger_";
pub const PROPS_SIZE: usize = 0x338;
pub const LOGFILE_SIZE: usize = 0x1c0;
pub const MAX_PROVIDERS: usize = 0x20;
pub const MAX_LOGGER_NAME: usize = 0x20;
pub const REALTIME_BUFFER_SIZE: u32 = 0x200;
pub const REALTIME_MIN_BUFFERS: u32 = 0x1e;
pub const REALTIME_MAX_BUFFERS: u32 = 0x32;
pub const EVENT_TRACE_REAL_TIME_MODE: u32 = 0x100;
pub const EVENT_TRACE_SYSTEM_LOGGER_MODE: u32 = 0x0200_0000;
pub const PROCESS_TRACE_MODE: u32 = 0x1000_0100;
pub const EVENT_CONTROL_CODE_ENABLE_PROVIDER: u32 = 1;
pub const EVENT_TRACE_CONTROL_QUERY: u32 = 0;
pub const EVENT_TRACE_CONTROL_STOP: u32 = 1;
pub const ETW_EVENT: &str = "ETWEvent";
pub const SPECIAL_PROVIDER: GUID = GUID::new(0x2cb15d1d, 0x5fc1, 0x11d2, [0xab, 0xe1, 0x00, 0xa0, 0xc9, 0x11, 0xf5, 0x18]);
pub const TRACE_DIRECTORY: &str = "d:\\xbperf";
pub const EXT_TYPE_EVENT_SCHEMA_TL: u16 = 0xb;
pub const RECORD_HEADER: usize = 0x70;

const F_INIT: &[u8] = b"XtfEventServer::ETWEventSource::Initialize\0";
const F_START: &[u8] = b"XtfEventServer::ETWEventSource::StartTracingEvents\0";
const F_P1: &[u8] = b"XtfEventServer::ETWEventSource::SendToClientProtocol1\0";
const F_P2: &[u8] = b"XtfEventServer::ETWEventSource::SendToClientProtocol2\0";

#[repr(C)]
pub struct SourceVtbl {
    pub dtor: unsafe extern "C" fn(*mut EtwSource, u32) -> *mut EtwSource,
    pub initialize: unsafe extern "C" fn(*mut EtwSource, *mut SharedPtr) -> HRESULT,
    pub shutdown: unsafe extern "C" fn(*mut EtwSource) -> HRESULT,
}

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct ProviderEntry {
    pub guid: GUID,
    pub level: u8,
    pub enable_property: u32,
}

#[repr(C)]
pub struct EtwSource {
    pub vtbl: *const SourceVtbl,
    pub dispatcher: SharedPtr,
    pub thread: HANDLE,
    pub consumer: AtomicU64,
    pub running: AtomicU32,
    pub logger_name: Vec<u16>,
    pub data_block: Mutex<Vec<u8>>,
    pub providers: Vec<GUID>,
    pub level: u8,
    pub protocol: u32,
    pub trace_result: AtomicI32,
}

unsafe impl Send for EtwSource {}
unsafe impl Sync for EtwSource {}

pub static TRACE_RESULTS: Mutex<Vec<(Vec<u16>, HRESULT)>> = Mutex::new(Vec::new());

pub fn logger_name(n: u32) -> Vec<u16> {
    wz(&format!("{}{}", LOGGER_NAME_FORMAT, n))
}

pub fn new_source(name: Vec<u16>, level: u8, protocol: u32, providers: Vec<GUID>) -> *mut EtwSource {
    Box::into_raw(Box::new(EtwSource {
        vtbl: &SOURCE_VTBL,
        dispatcher: SharedPtr::null(),
        thread: null_mut(),
        consumer: AtomicU64::new(0),
        running: AtomicU32::new(0),
        logger_name: name,
        data_block: Mutex::new(Vec::new()),
        providers,
        level,
        protocol,
        trace_result: AtomicI32::new(0),
    }))
}

impl Drop for EtwSource {
    fn drop(&mut self) {
        unsafe {
            if !self.thread.is_null() && self.thread != INVALID_HANDLE_VALUE {
                if self.running.load(Ordering::Acquire) != 0 {
                    s_shutdown(self);
                }
                WaitForSingleObject(self.thread, INFINITE);
                CloseHandle(self.thread);
            }
            decweak(self.dispatcher.ctrl);
        }
    }
}

unsafe extern "C" fn s_dtor(this: *mut EtwSource, flags: u32) -> *mut EtwSource {
    if flags & 1 != 0 {
        drop(Box::from_raw(this));
    } else {
        core::ptr::drop_in_place(this);
    }
    this
}

unsafe extern "C" fn s_initialize(this: *mut EtwSource, dispatcher: *mut SharedPtr) -> HRESULT {
    let arg = Strong(*dispatcher);
    let s = &mut *this;
    s.consumer.store(0, Ordering::Release);
    s.data_block.lock().unwrap_or_else(|e| e.into_inner()).clear();
    s.running.store(0, Ordering::Release);
    if arg.0.ptr.is_null() {
        return chk(E_INVALIDARG, b"E_INVALIDARG\0", F_INIT, 54);
    }
    incweak(arg.0.ctrl);
    let old = core::mem::replace(&mut s.dispatcher, arg.0);
    decweak(old.ctrl);
    let t = CreateThread(null_mut(), 0, trace_thread, this as *mut c_void, 0, null_mut());
    s.thread = t;
    if t == INVALID_HANDLE_VALUE || t.is_null() {
        let e = GetLastError() as i32;
        return chk(e, b"GetLastError()\0", F_INIT, 62);
    }
    S_OK
}

unsafe extern "system" fn trace_thread(p: *mut c_void) -> u32 {
    start_tracing_events(p as *mut EtwSource) as u32
}

unsafe extern "C" fn s_shutdown(this: *mut EtwSource) -> HRESULT {
    let s = &mut *this;
    let h = s.consumer.load(Ordering::Acquire);
    if s.running.load(Ordering::Acquire) == 0 || h == u64::MAX {
        return ERROR_INVALID_HANDLE as HRESULT;
    }
    let r = CloseTrace(h);
    if r != ERROR_CTX_CLOSE_PENDING {
        return r as HRESULT;
    }
    let mut unused = vec![0u16; 0x104];
    let hr = chk(stop_etw_trace_generic(&s.logger_name, &mut unused), b"StopEtwTraceGeneric( m_loggerName, unusedFilePath, FALSE )\0", b"XtfEventServer::ETWEventSource::Shutdown\0", 332);
    if hr < 0 {
        return hr;
    }
    s.running.store(0, Ordering::Release);
    if WaitForSingleObject(s.thread, INFINITE) != WAIT_OBJECT_0 {
        return WAIT_FAILED as HRESULT;
    }
    hr
}

pub static SOURCE_VTBL: SourceVtbl = SourceVtbl { dtor: s_dtor, initialize: s_initialize, shutdown: s_shutdown };

fn put32(b: &mut [u8], o: usize, v: u32) {
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}

fn get32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

pub fn trace_file_path(name: &[u16]) -> Vec<u16> {
    let n: Vec<u16> = name.iter().copied().take_while(|c| *c != 0).collect();
    let mut p: Vec<u16> = TRACE_DIRECTORY.encode_utf16().collect();
    p.push(b'\\' as u16);
    p.extend_from_slice(&n);
    p.extend("_u.etl".encode_utf16());
    p.push(0);
    p
}

pub unsafe fn fill_properties(props: &mut [u8], mode: u32, name: &[u16], buffer_size: u32, min_buffers: u32, max_buffers: u32, max_file_size: u32) {
    put32(props, 0x30, buffer_size);
    put32(props, 0x34, min_buffers);
    put32(props, 0x38, max_buffers);
    put32(props, 0, PROPS_SIZE as u32);
    put32(props, 0x2c, 0x20000);
    put32(props, 0x28, 3);
    let mut lm = mode | EVENT_TRACE_SYSTEM_LOGGER_MODE;
    put32(props, 0x40, lm);
    if mode & EVENT_TRACE_REAL_TIME_MODE != 0 {
        lm |= 0x10;
        put32(props, 0x44, 200);
        put32(props, 0x40, lm);
    } else {
        if mode & 2 != 0 {
            put32(props, 0x3c, if max_file_size != 0 { max_file_size } else { 0x20 });
        }
        put32(props, 0x70, 0xb8);
        let path = trace_file_path(name);
        let n = path.len().min(0x104);
        for (k, c) in path[..n].iter().enumerate() {
            props[0xb8 + 2 * k..0xb8 + 2 * k + 2].copy_from_slice(&c.to_le_bytes());
        }
        let dir = wz(TRACE_DIRECTORY);
        if GetFileAttributesW(dir.as_ptr()) == u32::MAX && GetLastError() == ERROR_FILE_NOT_FOUND {
            CreateDirectoryW(dir.as_ptr(), null_mut());
        }
    }
    put32(props, 0x74, 0x78);
    let nl = name.iter().position(|c| *c == 0).unwrap_or(name.len()).min(MAX_LOGGER_NAME - 1);
    for (k, c) in name[..nl].iter().enumerate() {
        props[0x78 + 2 * k..0x78 + 2 * k + 2].copy_from_slice(&c.to_le_bytes());
    }
}

pub fn realtime_properties_layout(props: &mut [u8]) {
    put32(props, 0x48, 0x80ff_02c0);
    put32(props, 0x2c0, 0x0003_001e);
    put32(props, 0x2c4, 0x0003_0011);
    put32(props, 0x308, 0x0001_0009);
    put32(props, 0x32c, 0x0002_0003);
}

pub unsafe fn start_etw_trace_generic(name: &[u16], providers: &[ProviderEntry], handle: &mut u64) -> HRESULT {
    let f = b"StartEtwTraceGeneric\0";
    let nl = name.iter().position(|c| *c == 0).unwrap_or(name.len());
    if nl >= MAX_LOGGER_NAME {
        return E_INVALIDARG;
    }
    let mut props = vec![0u8; PROPS_SIZE];
    fill_properties(&mut props, EVENT_TRACE_REAL_TIME_MODE, name, REALTIME_BUFFER_SIZE, REALTIME_MIN_BUFFERS, REALTIME_MAX_BUFFERS, 0);
    realtime_properties_layout(&mut props);
    let mut h = 0u64;
    let r = StartTraceW(&mut h, name.as_ptr(), props.as_mut_ptr());
    let hr = chk(hr_from_win32(r), b"StartTrace(&LoggerHandle, LoggerName, &TraceProperties.TraceProperties)\0", f, 391);
    if r != 0 {
        return hr;
    }
    *handle = h;
    for p in providers {
        if p.level > 5 {
            continue;
        }
        let mut params = [0u8; 0x30];
        let pp = if p.enable_property != 0 {
            put32(&mut params, 0, 1);
            put32(&mut params, 4, 4);
            params.as_ptr() as *const c_void
        } else {
            core::ptr::null()
        };
        let (any, level) = if p.guid == SPECIAL_PROVIDER { (0x20u64, 2u8) } else { (0u64, p.level) };
        let r = EnableTraceEx2(h, &p.guid, EVENT_CONTROL_CODE_ENABLE_PROVIDER, level, any, 0, 0, pp);
        let hr = chk(hr_from_win32(r), b"EnableTraceEx2(LoggerHandle, &guid, EVENT_CONTROL_CODE_ENABLE_PROVIDER, traceLevel, matchAny, 0, 0, pEnableParameters)\0", f, 423);
        if r != 0 {
            return hr;
        }
    }
    S_OK
}

pub unsafe fn stop_etw_trace_generic(name: &[u16], file_out: &mut [u16]) -> HRESULT {
    let f = b"StopEtwTraceGeneric\0";
    let nl = name.iter().position(|c| *c == 0).unwrap_or(name.len());
    if nl >= MAX_LOGGER_NAME {
        return E_INVALIDARG;
    }
    let mut props = vec![0u8; PROPS_SIZE];
    fill_properties(&mut props, 0, name, 0x80, 0xc, 0x20, 0);
    put32(&mut props, 0x70, 0);
    let r = ControlTraceW(0, name.as_ptr(), props.as_mut_ptr(), EVENT_TRACE_CONTROL_QUERY);
    let hr = chk(hr_from_win32(r), b"ControlTrace(0, LoggerName, &TraceProperties.TraceProperties, EVENT_TRACE_CONTROL_QUERY)\0", f, 634);
    if r != 0 {
        return hr;
    }
    let off = get32(&props, 0x70) as usize;
    let h = u64::from_le_bytes(props[8..16].try_into().unwrap());
    let r = ControlTraceW(h, core::ptr::null(), props.as_mut_ptr(), EVENT_TRACE_CONTROL_STOP);
    let hr = chk(hr_from_win32(r), b"ControlTrace(LoggerHandle, nullptr, &TraceProperties.TraceProperties, EVENT_TRACE_CONTROL_STOP)\0", f, 640);
    if r != 0 {
        return hr;
    }
    let mut k = 0;
    while k + 1 < file_out.len() && off != 0 && off + 2 * k + 2 <= props.len() {
        let c = u16::from_le_bytes([props[off + 2 * k], props[off + 2 * k + 1]]);
        if c == 0 {
            break;
        }
        file_out[k] = c;
        k += 1;
    }
    if k < file_out.len() {
        file_out[k] = 0;
    }
    hr
}

pub unsafe fn start_tracing_events(this: *mut EtwSource) -> HRESULT {
    let s = &*this;
    if s.running.load(Ordering::Acquire) != 0 {
        return S_OK;
    }
    let hr = run_trace(s);
    s.trace_result.store(hr, Ordering::Release);
    TRACE_RESULTS.lock().unwrap_or_else(|e| e.into_inner()).push((s.logger_name.clone(), hr));
    hr
}

unsafe fn run_trace(s: &EtwSource) -> HRESULT {
    if s.providers.len() >= MAX_PROVIDERS {
        return chk(E_INVALIDARG, b"E_INVALIDARG\0", F_START, 270);
    }
    let entries: Vec<ProviderEntry> = s.providers.iter().map(|g| ProviderEntry { guid: *g, level: s.level, enable_property: 0 }).collect();
    let mut h = 0u64;
    let hr = chk(start_etw_trace_generic(&s.logger_name, &entries, &mut h), b"StartEtwTraceGeneric( m_loggerName, nullptr, g_etwTraceSettings )\0", F_START, 285);
    if hr < 0 {
        return hr;
    }
    let mut lf = vec![0u8; LOGFILE_SIZE];
    lf[8..16].copy_from_slice(&(s.logger_name.as_ptr() as u64).to_le_bytes());
    lf[0x1a8..0x1b0].copy_from_slice(&(event_record_callback as *const () as usize as u64).to_le_bytes());
    put32(&mut lf, 0x1c, PROCESS_TRACE_MODE);
    lf[0x1b8..0x1c0].copy_from_slice(&(s as *const EtwSource as u64).to_le_bytes());
    let c = OpenTraceW(lf.as_mut_ptr());
    s.running.store(1, Ordering::Release);
    s.consumer.store(c, Ordering::Release);
    let r = ProcessTrace(&c, 1, core::ptr::null(), core::ptr::null());
    chk(hr_from_win32(r), b"ProcessTrace(&m_hConsumer, 1, NULL, NULL)\0", F_START, 303)
}

pub unsafe extern "system" fn event_record_callback(rec: *mut u8) {
    let s = *(rec.add(0x68) as *const *mut EtwSource);
    if s.is_null() {
        return;
    }
    if (*s).protocol == 2 {
        send_to_client_protocol2(&*s, rec);
    } else {
        send_to_client_protocol1(&*s, rec);
    }
}

unsafe fn rd16(p: *const u8, o: usize) -> u16 {
    core::ptr::read_unaligned(p.add(o) as *const u16)
}

unsafe fn rd64(p: *const u8, o: usize) -> u64 {
    core::ptr::read_unaligned(p.add(o) as *const u64)
}

unsafe fn queue(d: SharedPtr, json: Obj, block: &[u8]) -> HRESULT {
    let disp = d.ptr as *mut Dispatcher;
    let name = wz(ETW_EVENT);
    ((*(*disp).vtbl).queue_json_with_binary_event)(disp, name.as_ptr(), json, block.len() as u32, block.as_ptr())
}

pub unsafe fn ansi_to_bstr(s: *const u8) -> Result<Bstr, HRESULT> {
    if s.is_null() || *s == 0 {
        return Ok(Bstr::null());
    }
    let n = MultiByteToWideChar(0, 0, s, -1, null_mut(), 0);
    if n == 0 {
        let hr = hr_last_error();
        if hr < 0 {
            return Err(hr);
        }
    }
    let b = SysAllocStringLen(null_mut(), (n - 1).max(0) as u32);
    if b.is_null() {
        return Err(E_OUTOFMEMORY);
    }
    let out = Bstr(b);
    if MultiByteToWideChar(0, 0, s, -1, b, n) == 0 && n != 0 {
        *b = 0;
    }
    Ok(out)
}

pub unsafe fn send_to_client_protocol1(s: &EtwSource, rec: *const u8) -> HRESULT {
    let d = Strong(lock_weak(s.dispatcher));
    if d.0.ptr.is_null() || s.running.load(Ordering::Acquire) == 0 {
        return S_OK;
    }
    let i = match imports::get() {
        Ok(i) => i,
        Err(e) => return e,
    };
    let user_len = rd16(rec, 0x56) as usize;
    let mut block = s.data_block.lock().unwrap_or_else(|e| e.into_inner());
    let need = user_len + RECORD_HEADER;
    if block.len() < need {
        let mut v = Vec::new();
        if v.try_reserve_exact(need).is_err() {
            return chk(E_OUTOFMEMORY, b"E_OUTOFMEMORY\0", F_P1, 102);
        }
        v.resize(need, 0);
        *block = v;
    }
    let mut json = Ptr::null();
    let hr = chk((i.create_json_object)(json.out()), b"XbtpCreateJsonObject(&additionalJsonData)\0", F_P1, 109);
    if hr < 0 {
        return hr;
    }
    let fields: [(&str, f64); 8] = [
        ("EventFlags", rd16(rec, 4) as f64),
        ("EventProperty", rd16(rec, 6) as f64),
        ("DescriptorId", rd16(rec, 0x28) as f64),
        ("DescriptorVersion", *rec.add(0x2a) as f64),
        ("DescriptorChannel", *rec.add(0x2b) as f64),
        ("EventDescriptorLevel", *rec.add(0x2c) as f64),
        ("DescriptorOpcode", *rec.add(0x2d) as f64),
        ("DescriptorTask", rd16(rec, 0x2e) as f64),
    ];
    for (k, (n, v)) in fields.iter().enumerate() {
        let hr = chk(obj_set_number(json.0, wz(n).as_ptr(), *v), b"additionalJsonData->SetNumber\0", F_P1, 110 + k as u32);
        if hr < 0 {
            return hr;
        }
    }
    let ext_count = rd16(rec, 0x54) as usize;
    let ext = rd64(rec, 0x58) as *const u8;
    for k in 0..ext_count {
        let item = ext.add(16 * k);
        if rd16(item, 2) == EXT_TYPE_EVENT_SCHEMA_TL {
            let data = rd64(item, 8) as *const u8;
            let conv = match ansi_to_bstr(data.add(3)) {
                Ok(b) => b,
                Err(e) => return e,
            };
            let hr = chk(obj_set_string(json.0, wz("Schema_TL").as_ptr(), conv.0), b"additionalJsonData->SetString(EventExtended_Schema, strConverted)\0", F_P1, 131);
            if hr < 0 {
                return hr;
            }
        }
    }
    core::ptr::copy_nonoverlapping(rec, block.as_mut_ptr(), RECORD_HEADER);
    if user_len != 0 {
        core::ptr::copy_nonoverlapping(rd64(rec, 0x60) as *const u8, block.as_mut_ptr().add(RECORD_HEADER), user_len);
    }
    let hr = queue(d.0, json.0, &block[..need]);
    chk(hr, b"strongDispatcher->QueueJsonWithBinaryEvent(Event_ETWEventValue, additionalJsonData, numBytes, m_pDataBlock)\0", F_P1, 138)
}

pub unsafe fn protocol2_block(rec: *const u8) -> Vec<u8> {
    let ext_count = rd16(rec, 0x54) as usize;
    let user_len = rd16(rec, 0x56) as usize;
    let ext = rd64(rec, 0x58) as *const u8;
    let mut size = ((ext_count + 7) << 4) + user_len;
    for k in 0..ext_count {
        size += rd16(ext.add(16 * k), 6) as usize;
    }
    let mut b = vec![0u8; size];
    core::ptr::copy_nonoverlapping(rec, b.as_mut_ptr(), RECORD_HEADER);
    b[0x68..0x70].fill(0);
    let mut off = RECORD_HEADER;
    if user_len != 0 {
        core::ptr::copy_nonoverlapping(rd64(rec, 0x60) as *const u8, b.as_mut_ptr().add(off), user_len);
        b[0x60..0x68].copy_from_slice(&(off as u64).to_le_bytes());
        off += user_len;
    } else {
        b[0x60..0x68].fill(0);
    }
    if ext_count != 0 {
        let arr = off;
        core::ptr::copy_nonoverlapping(ext, b.as_mut_ptr().add(arr), ext_count * 16);
        off += ext_count * 16;
        for k in 0..ext_count {
            let item = ext.add(16 * k);
            let n = rd16(item, 6) as usize;
            if n != 0 {
                core::ptr::copy_nonoverlapping(rd64(item, 8) as *const u8, b.as_mut_ptr().add(off), n);
            }
            b[arr + 16 * k + 8..arr + 16 * k + 16].copy_from_slice(&(off as u64).to_le_bytes());
            off += n;
        }
        b[0x58..0x60].copy_from_slice(&(arr as u64).to_le_bytes());
    } else {
        b[0x58..0x60].fill(0);
    }
    b
}

pub unsafe fn send_to_client_protocol2(s: &EtwSource, rec: *const u8) -> HRESULT {
    let d = Strong(lock_weak(s.dispatcher));
    if d.0.ptr.is_null() || s.running.load(Ordering::Acquire) == 0 {
        return S_OK;
    }
    let i = match imports::get() {
        Ok(i) => i,
        Err(e) => return e,
    };
    let mut block = s.data_block.lock().unwrap_or_else(|e| e.into_inner());
    *block = protocol2_block(rec);
    let mut json = Ptr::null();
    let hr = chk((i.create_json_object)(json.out()), b"XbtpCreateJsonObject(&additionalJsonData)\0", F_P2, 233);
    if hr < 0 {
        return hr;
    }
    let mut g: *mut u16 = null_mut();
    let hr = chk(StringFromCLSID(rec.add(0x18) as *const GUID, &mut g), b"StringFromCLSID(pEventRecord->EventHeader.ProviderId, &guidBuffer)\0", F_P2, 237);
    if hr < 0 {
        return hr;
    }
    let hr = chk(obj_set_string(json.0, wz("ExclusiveETWProvider").as_ptr(), g), b"additionalJsonData->SetString(ExclusiveETWProvider, guidBuffer)\0", F_P2, 238);
    CoTaskMemFree(g as *mut c_void);
    if hr < 0 {
        return hr;
    }
    let hr = queue(d.0, json.0, &block);
    chk(hr, b"strongDispatcher->QueueJsonWithBinaryEvent(Event_ETWEventValue, additionalJsonData, numBytes, m_pDataBlock)\0", F_P2, 241)
}
