use crate::chk::chk;
use crate::win::*;
use core::ffi::c_void;
use core::ptr::null_mut;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use crate::profile::{self, MonitorFlow, Profile};
use xtfmsg::sys::{hr_from_win32, E_FAIL, E_INVALIDARG, HANDLE, HRESULT, S_OK};

pub const DEBUGMONITOR_CPP: &str = "xbox\\devex\\xtf\\dll\\debugmonitor\\debugmonitor.cpp";
pub const PDEM_CPP: &str = "xbox\\devex\\xtf\\dll\\debugmonitor\\processdebugeventmonitor.cpp";

pub const RING_ENTRIES: usize = 256;
pub const MESSAGE_BYTES: usize = 0x2000;
pub const PROCESS_MONITOR_BYTES: usize = 0x20_2100;
pub const WAIT_TIMEOUT_MS: u32 = 500;
pub const WAIT_TIMEOUT_BROKEN_MS: u32 = 1;

pub const EXCEPTION_DEBUG_EVENT: u32 = 1;
pub const CREATE_THREAD_DEBUG_EVENT: u32 = 2;
pub const CREATE_PROCESS_DEBUG_EVENT: u32 = 3;
pub const EXIT_THREAD_DEBUG_EVENT: u32 = 4;
pub const EXIT_PROCESS_DEBUG_EVENT: u32 = 5;
pub const LOAD_DLL_DEBUG_EVENT: u32 = 6;
pub const UNLOAD_DLL_DEBUG_EVENT: u32 = 7;
pub const OUTPUT_DEBUG_STRING_EVENT: u32 = 8;
pub const RIP_EVENT: u32 = 9;

pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn hr_last_error() -> HRESULT {
    hr_from_win32(unsafe { GetLastError() })
}

pub fn hr_last_error_or_fail() -> HRESULT {
    let e = unsafe { GetLastError() };
    if e == 0 {
        E_FAIL
    } else {
        hr_from_win32(e)
    }
}

#[derive(Clone)]
pub struct Entry {
    pub code: u32,
    pub pid: u32,
    pub tid: u32,
    pub data1: u32,
    pub data2: u32,
    pub data3: u32,
    pub address: u64,
    pub message: Vec<u8>,
}

impl Entry {
    pub fn new() -> Entry {
        Entry { code: 0, pid: 0, tid: 0, data1: 0, data2: 0, data3: 0, address: 0, message: vec![0; MESSAGE_BYTES] }
    }

    pub fn clear(&mut self) {
        self.code = 0;
        self.pid = 0;
        self.tid = 0;
        self.data1 = 0;
        self.data2 = 0;
        self.data3 = 0;
        self.address = 0;
        self.message.iter_mut().for_each(|b| *b = 0);
    }

    pub fn wide_message(&self) -> Vec<u16> {
        let mut v = Vec::new();
        for c in self.message.chunks_exact(2) {
            let w = u16::from_le_bytes([c[0], c[1]]);
            if w == 0 {
                break;
            }
            v.push(w);
        }
        v
    }

    pub fn ansi_message(&self) -> &[u8] {
        let n = self.message.iter().position(|&b| b == 0).unwrap_or(self.message.len());
        &self.message[..n]
    }
}

impl Default for Entry {
    fn default() -> Entry {
        Entry::new()
    }
}

pub struct DebugClient {
    pub monitor: Arc<ProcessMonitor>,
    pub filter_flags: u32,
    pub session: usize,
    pub read_head: AtomicU32,
    pub monitoring: AtomicBool,
    pub stop_event: HANDLE,
    pub event_ready: HANDLE,
    pub init_result: HRESULT,
    pub session_identifier: u32,
}

unsafe impl Send for DebugClient {}
unsafe impl Sync for DebugClient {}

static NEXT_CLIENT_ID: AtomicU32 = AtomicU32::new(1);

impl DebugClient {
    pub fn new(monitor: &Arc<ProcessMonitor>, filter_flags: u32, session: usize) -> DebugClient {
        let id = NEXT_CLIENT_ID.fetch_add(1, Ordering::SeqCst).wrapping_add(1);
        let read_head = monitor.write_head.load(Ordering::SeqCst);
        let stop = unsafe { CreateEventW(null_mut(), 0, 0, core::ptr::null()) };
        let (stop_event, event_ready, init_result) = if stop == INVALID_HANDLE_VALUE {
            (null_mut(), null_mut(), hr_last_error())
        } else {
            (stop, monitor.event_ready, S_OK)
        };
        DebugClient { monitor: monitor.clone(), filter_flags, session, read_head: AtomicU32::new(read_head), monitoring: AtomicBool::new(false), stop_event, event_ready, init_result, session_identifier: id }
    }
}

impl Drop for DebugClient {
    fn drop(&mut self) {
        if !self.stop_event.is_null() {
            unsafe { CloseHandle(self.stop_event) };
        }
    }
}

pub struct ProcessMonitor {
    pub active: AtomicBool,
    pub broken: AtomicBool,
    pub pid: u32,
    pub continue_tid: AtomicU32,
    pub continue_status: AtomicU32,
    pub write_head: AtomicU32,
    pub event_ready: HANDLE,
    pub init_result: HRESULT,
    pub thread: AtomicUsize,
    pub continue_event: HANDLE,
    pub ring: Box<[Mutex<Entry>]>,
    pub clients: Mutex<Vec<Arc<DebugClient>>>,
    pub attach_done: AtomicBool,
    pub attach_result: AtomicI32,
    pub break_on_exception: AtomicBool,
    pub force_all: AtomicBool,
    pub detach_count: AtomicU32,
    pub profile: &'static Profile,
}

unsafe impl Send for ProcessMonitor {}
unsafe impl Sync for ProcessMonitor {}

impl ProcessMonitor {
    pub fn new(pid: u32) -> ProcessMonitor {
        ProcessMonitor::with_profile(pid, profile::current())
    }

    pub fn with_profile(pid: u32, profile: &'static Profile) -> ProcessMonitor {
        let ring: Vec<Mutex<Entry>> = (0..RING_ENTRIES).map(|_| Mutex::new(Entry::new())).collect();
        let mut m = ProcessMonitor {
            active: AtomicBool::new(false),
            broken: AtomicBool::new(false),
            pid,
            continue_tid: AtomicU32::new(0),
            continue_status: AtomicU32::new(0),
            write_head: AtomicU32::new(0),
            event_ready: null_mut(),
            init_result: S_OK,
            thread: AtomicUsize::new(0),
            continue_event: null_mut(),
            ring: ring.into_boxed_slice(),
            clients: Mutex::new(Vec::new()),
            attach_done: AtomicBool::new(false),
            attach_result: AtomicI32::new(S_OK),
            break_on_exception: AtomicBool::new(false),
            force_all: AtomicBool::new(false),
            detach_count: AtomicU32::new(0),
            profile,
        };
        unsafe {
            let e = CreateEventW(null_mut(), 0, 0, core::ptr::null());
            if e == INVALID_HANDLE_VALUE {
                m.init_result = hr_last_error();
                return m;
            }
            m.event_ready = e;
            let c = CreateEventW(null_mut(), 0, 0, core::ptr::null());
            if c == INVALID_HANDLE_VALUE {
                m.init_result = hr_last_error();
                return m;
            }
            m.continue_event = c;
        }
        m.active.store(true, Ordering::SeqCst);
        m
    }

    pub fn add_debug_client(self: &Arc<Self>, filter_flags: u32, session: usize) -> Arc<DebugClient> {
        let client = Arc::new(DebugClient::new(self, filter_flags, session));
        chk(PDEM_CPP, "ProcessDebugEventMonitor::AddDebugClient", self.profile.add_client(151), "pDebugClient", S_OK, "");
        let mut clients = lock(&self.clients);
        clients.push(client.clone());
        chk(PDEM_CPP, "ProcessDebugEventMonitor::AddDebugClient", self.profile.add_client(174), "m_debugClients.Add(debugClient)", S_OK, "");
        let any = clients.iter().any(|c| c.filter_flags & 1 != 0);
        self.break_on_exception.store(any, Ordering::SeqCst);
        client
    }

    pub fn find_client(&self, id: u32) -> Option<Arc<DebugClient>> {
        lock(&self.clients).iter().find(|c| c.session_identifier == id).cloned()
    }

    pub unsafe fn start_monitoring(self: &Arc<Self>) -> HRESULT {
        if self.thread.load(Ordering::SeqCst) != 0 {
            return E_ILLEGAL_METHOD_CALL;
        }
        let param = Arc::into_raw(self.clone()) as *mut c_void;
        let t = CreateThread(null_mut(), 0, worker_thread_proc, param, 0, null_mut());
        self.thread.store(t as usize, Ordering::SeqCst);
        if t.is_null() {
            self.active.store(false, Ordering::SeqCst);
            let hr = hr_last_error();
            drop(Arc::from_raw(param as *const ProcessMonitor));
            return hr;
        }
        S_OK
    }

    pub fn destroy(&self) {
        let clients: Vec<Arc<DebugClient>> = lock(&self.clients).drain(..).collect();
        for c in clients {
            if c.monitoring.load(Ordering::SeqCst) {
                unsafe { SetEvent(c.stop_event) };
                while c.monitoring.load(Ordering::SeqCst) {
                    unsafe { Sleep(100) };
                }
            }
        }
    }

    unsafe fn cont(pid: u32, tid: u32, status: u32) -> HRESULT {
        if ContinueDebugEvent(pid, tid, status) == 0 {
            hr_last_error_or_fail()
        } else {
            S_OK
        }
    }

    pub unsafe fn worker(&self) -> HRESULT {
        let mut ev = DebugEvent::zeroed();
        if DebugActiveProcess(self.pid) == 0 {
            self.attach_result.store(hr_last_error(), Ordering::SeqCst);
        }
        self.attach_done.store(true, Ordering::SeqCst);
        SetEvent(self.event_ready);
        let mut hr = self.attach_result.load(Ordering::SeqCst);
        chk(PDEM_CPP, "ProcessDebugEventMonitor::WorkerThread", self.profile.pdem(387), "m_attachResult", hr, "");
        'run: {
            if hr < 0 {
                break 'run;
            }
            loop {
                let timeout = if self.broken.load(Ordering::SeqCst) { WAIT_TIMEOUT_BROKEN_MS } else { WAIT_TIMEOUT_MS };
                let got = WaitForDebugEvent(&mut ev, timeout);
                if got == 0 {
                    let e = GetLastError();
                    if e != ERROR_SEM_TIMEOUT {
                        hr = hr_from_win32(e);
                        chk(PDEM_CPP, "ProcessDebugEventMonitor::WorkerThread", self.profile.pdem(402), "dwError", hr, "");
                        if e != 0 {
                            break 'run;
                        }
                    }
                }
                let (total, monitoring) = {
                    let clients = lock(&self.clients);
                    (clients.len() as u32, clients.iter().filter(|c| c.monitoring.load(Ordering::SeqCst)).count() as u32)
                };
                if monitoring == 0 || self.detach_count.load(Ordering::SeqCst) >= total {
                    self.force_all.store(true, Ordering::SeqCst);
                    break 'run;
                }
                if self.force_all.load(Ordering::SeqCst) {
                    break 'run;
                }
                if got == 0 {
                    continue;
                }
                let idx = self.write_head.load(Ordering::SeqCst) as usize;
                let (line, expr, r) = {
                    let mut entry = lock(&self.ring[idx]);
                    entry.clear();
                    entry.code = ev.code;
                    entry.pid = ev.pid;
                    entry.tid = ev.tid;
                    self.on_event(&ev, &mut entry)
                };
                if line != 0 {
                    hr = r;
                    chk(PDEM_CPP, "ProcessDebugEventMonitor::WorkerThread", self.profile.pdem(line), expr, hr, "");
                    if hr < 0 {
                        break 'run;
                    }
                }
                self.write_head.store(((idx + 1) & 0xff) as u32, Ordering::SeqCst);
                SetEvent(self.event_ready);
                if self.broken.load(Ordering::SeqCst) {
                    if WaitForSingleObject(self.continue_event, INFINITE) == 0 {
                        let r = Self::cont(self.pid, self.continue_tid.load(Ordering::SeqCst), self.continue_status.load(Ordering::SeqCst));
                        if r < 0 {
                            hr = r;
                            break 'run;
                        }
                    } else {
                        hr = hr_last_error();
                        chk(PDEM_CPP, "ProcessDebugEventMonitor::WorkerThread", self.profile.pdem(479), "HRESULT_FROM_WIN32(::GetLastError())", hr, "");
                        if hr < 0 {
                            break 'run;
                        }
                    }
                }
            }
        }
        if self.broken.load(Ordering::SeqCst) {
            if ContinueDebugEvent(self.pid, self.continue_tid.load(Ordering::SeqCst), self.continue_status.load(Ordering::SeqCst)) == 0 {
                hr = hr_last_error();
            }
            self.broken.store(false, Ordering::SeqCst);
        }
        if self.attach_result.load(Ordering::SeqCst) >= 0 {
            if DebugActiveProcessStop(self.pid) == 0 {
                hr = hr_last_error();
            }
            while WaitForDebugEvent(&mut ev, 0) != 0 {}
        }
        self.active.store(false, Ordering::SeqCst);
        SetEvent(self.event_ready);
        hr
    }

    unsafe fn on_event(&self, ev: &DebugEvent, entry: &mut Entry) -> (u32, &'static str, HRESULT) {
        match ev.code {
            EXCEPTION_DEBUG_EVENT => {
                entry.data1 = ev.u32_at(0x10);
                let first = ev.u32_at(0xa8);
                entry.data2 = first;
                entry.address = ev.u64_at(0x20);
                let status = if first != 0 { DBG_EXCEPTION_NOT_HANDLED } else { DBG_CONTINUE };
                let hr = if self.break_on_exception.load(Ordering::SeqCst) {
                    entry.data3 = 1;
                    self.continue_tid.store(ev.tid, Ordering::SeqCst);
                    self.continue_status.store(status, Ordering::SeqCst);
                    self.broken.store(true, Ordering::SeqCst);
                    S_OK
                } else {
                    Self::cont(ev.pid, ev.tid, status)
                };
                (429, "OnExceptionDebugEvent(dbgEvent, m_eventBuffer[m_writeHead])", hr)
            }
            CREATE_THREAD_DEBUG_EVENT => {
                entry.address = ev.u64_at(0x20);
                (433, "OnCreateThreadDebugEvent(dbgEvent, m_eventBuffer[m_writeHead])", Self::cont(ev.pid, ev.tid, DBG_CONTINUE))
            }
            CREATE_PROCESS_DEBUG_EVENT => (437, "OnCreateProcessDebugEvent(dbgEvent, m_eventBuffer[m_writeHead])", on_create_process(self.profile, ev, entry)),
            EXIT_THREAD_DEBUG_EVENT => {
                entry.data1 = ev.u32_at(0x10);
                (441, "OnExitThreadDebugEvent(dbgEvent, m_eventBuffer[m_writeHead])", Self::cont(ev.pid, ev.tid, DBG_CONTINUE))
            }
            EXIT_PROCESS_DEBUG_EVENT => {
                entry.data1 = ev.u32_at(0x10);
                (445, "OnExitProcessDebugEvent(dbgEvent, m_eventBuffer[m_writeHead])", Self::cont(ev.pid, ev.tid, DBG_CONTINUE))
            }
            LOAD_DLL_DEBUG_EVENT => (449, "OnLoadDllDebugEvent(dbgEvent, m_eventBuffer[m_writeHead])", on_load_dll(self.profile, ev, entry)),
            UNLOAD_DLL_DEBUG_EVENT => {
                entry.address = ev.u64_at(0x10);
                (453, "OnUnloadDllDebugEvent(dbgEvent, m_eventBuffer[m_writeHead])", Self::cont(ev.pid, ev.tid, DBG_CONTINUE))
            }
            OUTPUT_DEBUG_STRING_EVENT => (457, "OnOutputDebugStringEvent(dbgEvent, m_eventBuffer[m_writeHead])", on_output_debug_string(self.profile, ev, entry)),
            RIP_EVENT => {
                entry.data1 = ev.u32_at(0x10);
                entry.data2 = ev.u32_at(0x14);
                (461, "OnRipEvent(dbgEvent, m_eventBuffer[m_writeHead])", Self::cont(ev.pid, ev.tid, DBG_CONTINUE))
            }
            _ => (0, "", S_OK),
        }
    }
}

impl Drop for ProcessMonitor {
    fn drop(&mut self) {
        unsafe {
            if !self.event_ready.is_null() {
                CloseHandle(self.event_ready);
            }
            if !self.continue_event.is_null() {
                CloseHandle(self.continue_event);
            }
            let t = self.thread.load(Ordering::SeqCst);
            if t != 0 {
                CloseHandle(t as HANDLE);
            }
        }
    }
}

unsafe extern "system" fn worker_thread_proc(param: *mut c_void) -> u32 {
    if param.is_null() {
        return 0x57;
    }
    let m = Arc::from_raw(param as *const ProcessMonitor);
    m.worker() as u32
}

unsafe fn close_file(h: u64) -> HRESULT {
    if h != 0 && CloseHandle(h as HANDLE) == 0 {
        return hr_last_error();
    }
    S_OK
}

pub unsafe fn on_create_process(p: &Profile, ev: &DebugEvent, entry: &mut Entry) -> HRESULT {
    entry.address = ev.u64_at(0x40);
    let hr = close_file(ev.u64_at(0x10));
    if ContinueDebugEvent(ev.pid, ev.tid, DBG_CONTINUE) == 0 {
        return hr_last_error_or_fail();
    }
    chk(PDEM_CPP, "ProcessDebugEventMonitor::OnCreateProcessDebugEvent", p.pdem(594), "hrCloseFile", hr, "");
    hr
}

unsafe fn read(h: HANDLE, addr: u64, buf: *mut c_void, n: usize) -> bool {
    let mut got = 0usize;
    ReadProcessMemory(h, addr as *const c_void, buf, n, &mut got) != 0
}

pub unsafe fn read_image_name(h: HANDLE, base: u64, image_name: u64, out: &mut [u8]) -> bool {
    out[0] = 0;
    out[1] = 0;
    if image_name != 0 {
        let mut p = 0u64;
        if read(h, image_name, &mut p as *mut u64 as *mut c_void, 8) && read(h, p, out.as_mut_ptr() as *mut c_void, MESSAGE_BYTES) {
            let wide_nonempty = out[0] != 0 || out[1] != 0;
            if wide_nonempty {
                return true;
            }
        }
    }
    let mut dos = [0u8; 0x40];
    if !read(h, base, dos.as_mut_ptr() as *mut c_void, dos.len()) || u16::from_le_bytes([dos[0], dos[1]]) != 0x5a4d {
        return false;
    }
    let lfanew = i32::from_le_bytes(dos[0x3c..0x40].try_into().unwrap()) as i64;
    let mut nt = [0u8; 0x108];
    if !read(h, base.wrapping_add(lfanew as u64), nt.as_mut_ptr() as *mut c_void, nt.len()) {
        return false;
    }
    let export_rva = u32::from_le_bytes(nt[0x88..0x8c].try_into().unwrap());
    if export_rva == 0 {
        return false;
    }
    let mut exp = [0u8; 0x28];
    if !read(h, base.wrapping_add(export_rva as u64), exp.as_mut_ptr() as *mut c_void, exp.len()) {
        return false;
    }
    let name_rva = u32::from_le_bytes(exp[0xc..0x10].try_into().unwrap());
    read(h, base.wrapping_add(name_rva as u64), out.as_mut_ptr() as *mut c_void, MESSAGE_BYTES)
}

pub unsafe fn on_load_dll(p: &Profile, ev: &DebugEvent, entry: &mut Entry) -> HRESULT {
    entry.address = ev.u64_at(0x18);
    let image_name = ev.u64_at(0x28);
    entry.data1 = if image_name != 0 { ev.u16_at(0x30) as u32 } else { 0 };
    entry.message[0] = 0;
    entry.message[1] = 0;
    let h = OpenProcess(PROCESS_VM_READ, 0, ev.pid);
    if h != INVALID_HANDLE_VALUE {
        if !read_image_name(h, entry.address, image_name, &mut entry.message) {
            entry.message[0] = 0;
            entry.message[1] = 0;
        }
        CloseHandle(h);
    }
    let hr = close_file(ev.u64_at(0x10));
    if ContinueDebugEvent(ev.pid, ev.tid, DBG_CONTINUE) == 0 {
        return hr_last_error_or_fail();
    }
    chk(PDEM_CPP, "ProcessDebugEventMonitor::OnLoadDllDebugEvent", p.pdem(795), "hrCloseFile", hr, "");
    hr
}

pub unsafe fn on_output_debug_string(p: &Profile, ev: &DebugEvent, entry: &mut Entry) -> HRESULT {
    let mut hr = S_OK;
    let h = OpenProcess(PROCESS_VM_READ, 0, ev.pid);
    if h == INVALID_HANDLE_VALUE {
        hr = hr_last_error();
        chk(PDEM_CPP, "ProcessDebugEventMonitor::OnOutputDebugStringEvent", p.pdem(836), "HRESULT_FROM_WIN32(GetLastError())", hr, "");
    }
    if hr >= 0 {
        let unicode = ev.u16_at(0x18);
        entry.data1 = unicode as u32;
        let size = (if unicode != 0 { 2 } else { 1 }) * ev.u16_at(0x1a) as usize;
        let mut tmp = vec![0u8; size];
        if read(h, ev.u64_at(0x10), tmp.as_mut_ptr() as *mut c_void, size) {
            let n = size.min(MESSAGE_BYTES);
            entry.message[..n].copy_from_slice(&tmp[..n]);
        } else {
            hr = hr_last_error_or_fail();
        }
    }
    let cont = if ContinueDebugEvent(ev.pid, ev.tid, DBG_CONTINUE) == 0 { hr_last_error() } else { S_OK };
    if h != INVALID_HANDLE_VALUE {
        CloseHandle(h);
        hr = cont;
    }
    hr
}

pub struct ProcessInfo {
    pub pid: u32,
    pub active: bool,
    pub broken: bool,
    pub clients: Vec<ClientInfo>,
}

pub struct ClientInfo {
    pub client_id: u32,
    pub active: bool,
}

pub struct DebugMonitor {
    pub monitors: Mutex<Vec<Arc<ProcessMonitor>>>,
}

static INSTANCE: OnceLock<Arc<DebugMonitor>> = OnceLock::new();

#[cfg(test)]
thread_local! {
    pub static FORCE_NULL_INSTANCE: core::cell::Cell<bool> = const { core::cell::Cell::new(false) };
}

pub fn instance() -> Option<Arc<DebugMonitor>> {
    #[cfg(test)]
    if FORCE_NULL_INSTANCE.with(|f| f.get()) {
        return None;
    }
    Some(INSTANCE.get_or_init(|| Arc::new(DebugMonitor { monitors: Mutex::new(Vec::new()) })).clone())
}

impl DebugMonitor {
    pub fn find(&self, pid: u32) -> Option<Arc<ProcessMonitor>> {
        lock(&self.monitors).iter().find(|m| m.pid == pid).cloned()
    }

    pub fn get_monitor_list(&self) -> Vec<ProcessInfo> {
        let monitors = lock(&self.monitors);
        monitors
            .iter()
            .map(|m| {
                let clients = lock(&m.clients).iter().map(|c| ClientInfo { client_id: c.monitoring.load(Ordering::SeqCst) as u32, active: c.session_identifier != 0 }).collect();
                ProcessInfo { pid: m.pid, active: m.active.load(Ordering::SeqCst), broken: m.broken.load(Ordering::SeqCst), clients }
            })
            .collect()
    }

    pub unsafe fn monitor_process(&self, p: &'static Profile, pid: u32, filter_flags: u32, session: usize) -> HRESULT {
        if session == 0 {
            return E_INVALIDARG;
        }
        match p.flow {
            MonitorFlow::AddMonitorClient => match self.add_monitor_client(p, pid, filter_flags, session) {
                Err(hr) => {
                    chk(DEBUGMONITOR_CPP, "DebugMonitor::MonitorProcess", 173, "AddMonitorClient(pid, filterFlags, session, &pDebugClient)", hr, "");
                    hr
                }
                Ok(client) => {
                    chk(DEBUGMONITOR_CPP, "DebugMonitor::MonitorProcess", 173, "AddMonitorClient(pid, filterFlags, session, &pDebugClient)", S_OK, "");
                    let hr = client.monitor();
                    chk(DEBUGMONITOR_CPP, "DebugMonitor::MonitorProcess", 176, "pDebugClient->Monitor()", hr, "");
                    hr
                }
            },
            MonitorFlow::AddProcessMonitor => self.monitor_process_separate_add(p, pid, filter_flags, session),
        }
    }

    pub fn remove_process_monitor(&self, monitor: &Arc<ProcessMonitor>) -> HRESULT {
        let mut monitors = lock(&self.monitors);
        match monitors.iter().position(|m| Arc::ptr_eq(m, monitor)) {
            Some(i) => {
                monitors.remove(i);
                S_OK
            }
            None => {
                chk(DEBUGMONITOR_CPP, "DebugMonitor::RemoveProcessMonitor", 82, "E_NOT_SET", E_NOT_SET, "Process monitor could not be removed.");
                E_NOT_SET
            }
        }
    }

    pub fn add_process_monitor(&self, monitor: &Arc<ProcessMonitor>) -> HRESULT {
        lock(&self.monitors).push(monitor.clone());
        chk(DEBUGMONITOR_CPP, "DebugMonitor::AddProcessMonitor", 60, "m_processMonitors.Add(pMonitor)", S_OK, "");
        S_OK
    }

    unsafe fn monitor_process_separate_add(&self, p: &'static Profile, pid: u32, filter_flags: u32, session: usize) -> HRESULT {
        let mut monitor = self.find(pid);
        if let Some(m) = monitor.clone() {
            if !m.active.load(Ordering::SeqCst) {
                let hr = self.remove_process_monitor(&m);
                chk(DEBUGMONITOR_CPP, "DebugMonitor::MonitorProcess", 114, "RemoveProcessMonitor(pMonitor)", hr, "");
                if hr < 0 {
                    return hr;
                }
                m.destroy();
                monitor = None;
            }
        }
        let (monitor, created) = match monitor {
            Some(m) => (m, false),
            None => {
                let m = Arc::new(ProcessMonitor::with_profile(pid, p));
                chk(DEBUGMONITOR_CPP, "DebugMonitor::MonitorProcess", 122, "pMonitor = new ProcessDebugEventMonitor(pid)", S_OK, "");
                let hr = self.add_process_monitor(&m);
                chk(DEBUGMONITOR_CPP, "DebugMonitor::MonitorProcess", 123, "AddProcessMonitor(pMonitor)", hr, "");
                (m, true)
            }
        };
        let client = monitor.add_debug_client(filter_flags, session);
        chk(DEBUGMONITOR_CPP, "DebugMonitor::MonitorProcess", 127, "pMonitor->AddDebugClient(filterFlags, session, &pDebugClient)", S_OK, "");
        if created {
            let hr = monitor.start_monitoring();
            chk(DEBUGMONITOR_CPP, "DebugMonitor::MonitorProcess", 131, "pMonitor->StartMonitoring()", hr, "");
            if hr < 0 {
                return hr;
            }
        }
        let hr = client.monitor();
        chk(DEBUGMONITOR_CPP, "DebugMonitor::MonitorProcess", 135, "pDebugClient->Monitor()", hr, "");
        hr
    }

    pub unsafe fn add_monitor_client(&self, p: &'static Profile, pid: u32, filter_flags: u32, session: usize) -> Result<Arc<DebugClient>, HRESULT> {
        let mut monitors = lock(&self.monitors);
        let mut existing = None;
        if let Some(i) = monitors.iter().position(|m| m.pid == pid) {
            if monitors[i].active.load(Ordering::SeqCst) {
                existing = Some(monitors[i].clone());
            } else {
                let old = monitors.remove(i);
                old.destroy();
            }
        }
        let (monitor, created) = match existing {
            Some(m) => (m, false),
            None => {
                let m = Arc::new(ProcessMonitor::with_profile(pid, p));
                chk(DEBUGMONITOR_CPP, "DebugMonitor::AddMonitorClient", 85, "pMonitor = new ProcessDebugEventMonitor(pid)", S_OK, "");
                let hr = m.init_result;
                chk(DEBUGMONITOR_CPP, "DebugMonitor::AddMonitorClient", 86, "pMonitor->GetInitResult()", hr, "");
                if hr < 0 {
                    return Err(hr);
                }
                monitors.push(m.clone());
                chk(DEBUGMONITOR_CPP, "DebugMonitor::AddMonitorClient", 88, "m_processMonitors.Add(pMonitor)", S_OK, "");
                (m, true)
            }
        };
        let client = monitor.add_debug_client(filter_flags, session);
        chk(DEBUGMONITOR_CPP, "DebugMonitor::AddMonitorClient", 96, "pMonitor->AddDebugClient(filterFlags, pSession, ppDebugClient)", S_OK, "");
        if created {
            let hr = monitor.start_monitoring();
            chk(DEBUGMONITOR_CPP, "DebugMonitor::AddMonitorClient", 104, "pMonitor->StartMonitoring()", hr, "");
            if hr < 0 {
                return Err(hr);
            }
        }
        Ok(client)
    }
}
