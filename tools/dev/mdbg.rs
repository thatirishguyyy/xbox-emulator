// Minimal Windows debug-loop tracer: launches a target under DEBUG_ONLY_THIS_PROCESS,
// captures OutputDebugString output (UE routes its log here), first-chance
// exceptions with module attribution, and the final exit code.
#![allow(non_snake_case, non_camel_case_types)]
use std::ffi::c_void;
use std::collections::BTreeMap;

#[path = "mdbg_heap.rs"]
mod heap;

fn ods_ms() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)
}

type HANDLE = isize;

#[repr(C)]
struct PROCESS_INFORMATION { hProcess: HANDLE, hThread: HANDLE, dwProcessId: u32, dwThreadId: u32 }

#[repr(C)]
struct STARTUPINFOW {
    cb: u32, lpReserved: *mut u16, lpDesktop: *mut u16, lpTitle: *mut u16,
    dwX: u32, dwY: u32, dwXSize: u32, dwYSize: u32, dwXCountChars: u32, dwYCountChars: u32,
    dwFillAttribute: u32, dwFlags: u32, wShowWindow: u16, cbReserved2: u16,
    lpReserved2: *mut u8, hStdInput: HANDLE, hStdOutput: HANDLE, hStdError: HANDLE,
}

#[repr(C)]
struct DEBUG_EVENT { code: u32, pid: u32, tid: u32, _pad: u32, u: [u8; 184] }

extern "system" {
    fn CreateProcessW(app: *const u16, cmd: *mut u16, pa: *mut c_void, ta: *mut c_void,
        inherit: i32, flags: u32, env: *mut c_void, cwd: *const u16,
        si: *const STARTUPINFOW, pi: *mut PROCESS_INFORMATION) -> i32;
    fn WaitForDebugEventEx(evt: *mut DEBUG_EVENT, ms: u32) -> i32;
    fn ContinueDebugEvent(pid: u32, tid: u32, status: u32) -> i32;
    fn ReadProcessMemory(h: HANDLE, addr: *const c_void, buf: *mut u8, n: usize, read: *mut usize) -> i32;
    fn WriteProcessMemory(h: HANDLE, addr: *mut c_void, buf: *const u8, n: usize, wr: *mut usize) -> i32;
    fn TerminateProcess(h: HANDLE, code: u32) -> i32;
    fn GetFinalPathNameByHandleW(h: HANDLE, buf: *mut u16, n: u32, flags: u32) -> u32;
    fn GetModuleHandleW(name: *const u16) -> HANDLE;
    fn GetProcAddress(h: HANDLE, name: *const u8) -> *mut c_void;
    fn OpenThread(access: u32, inherit: i32, tid: u32) -> HANDLE;
    fn SuspendThread(h: HANDLE) -> u32;
    fn ResumeThread(h: HANDLE) -> u32;
    fn GetThreadContext(h: HANDLE, ctx: *mut u8) -> i32;
    fn SetThreadContext(h: HANDLE, ctx: *const u8) -> i32;
    fn CloseHandle(h: HANDLE) -> i32;
    fn GetLastError() -> u32;
}

#[repr(C, align(16))]
struct Ctx([u8; 1256]);

const DEBUG_ONLY_THIS_PROCESS: u32 = 0x2;
const DBG_CONTINUE: u32 = 0x0001_0002;
const DBG_EXCEPTION_NOT_HANDLED: u32 = 0x8001_0001;

fn wide(s: &str) -> Vec<u16> { s.encode_utf16().chain(std::iter::once(0)).collect() }

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let exe = &args[1];
    // Optional: --exebp <rva_hex> plants a breakpoint at exe_base+rva, dumps
    // registers on hit, and terminates. Used to bisect engine init.
    let mut exebp_rva: u64 = 0;
    let mut trace_mode = false; // --trace: exebp logs regs+caller and CONTINUES (log-and-continue)
    let mut trace_limit: u64 = 40;
    let mut rest: Vec<String> = Vec::new();
    let mut it = args[2..].iter();
    while let Some(a) = it.next() {
        if a == "--exebp" {
            if let Some(v) = it.next() { exebp_rva = u64::from_str_radix(v.trim_start_matches("0x"), 16).unwrap_or(0); }
        } else if a == "--trace" {
            trace_mode = true;
        } else if a == "--tracelimit" {
            if let Some(v) = it.next() { trace_limit = v.parse().unwrap_or(40); }
        } else { rest.push(a.clone()); }
    }
    let extra: String = rest.iter().map(|a| match a.split_once('=') {
        Some((k, v)) if v.contains(' ') && !v.starts_with('"') => format!("{k}=\"{v}\""),
        _ if a.contains(' ') && !a.starts_with('"') => format!("\"{a}\""),
        _ => a.clone(),
    }).collect::<Vec<_>>().join(" ");
    let dir = std::path::Path::new(exe).parent().unwrap().to_string_lossy().to_string();
    let cmdline = format!("\"{exe}\" {extra}");
    eprintln!("[trace] launching: {cmdline}\n[trace] cwd: {dir}");

    let mut si: STARTUPINFOW = unsafe { std::mem::zeroed() };
    si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
    let mut pi: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    let mut cmd_w = wide(&cmdline);
    let dir_w = wide(&dir);
    let ok = unsafe {
        CreateProcessW(std::ptr::null(), cmd_w.as_mut_ptr(), std::ptr::null_mut(), std::ptr::null_mut(),
            0, DEBUG_ONLY_THIS_PROCESS, std::ptr::null_mut(), dir_w.as_ptr(), &si, &mut pi)
    };
    if ok == 0 { eprintln!("[trace] CreateProcessW failed err={}", unsafe { GetLastError() }); std::process::exit(1); }
    let hproc = pi.hProcess;

    let mut modules: BTreeMap<u64, String> = BTreeMap::new();
    let start = std::time::Instant::now();
    // Run-time limit in seconds (env MDBG_LIMIT, default 70).
    let limit = std::time::Duration::from_secs(std::env::var("MDBG_LIMIT").ok().and_then(|v| v.parse().ok()).unwrap_or(70));
    let mut exc_count = 0u32;
    let mut av_captured = false;
    let exit_code;
    let mut exe_base = 0u64;
    let exe_size = 0x9a0_0000u64; // size_of_image of the Fortnite EXE (approx)

    // Resolve the process-exit sink. ntdll!RtlExitUserProcess sits at the same
    // address in the child as in us (per-boot ASLR), so we can plant a 0xCC
    // breakpoint there to catch "who is exiting" with a stack scan.
    let sink_addr = unsafe {
        let ntdll = GetModuleHandleW(wide("ntdll.dll").as_ptr());
        GetProcAddress(ntdll, b"RtlExitUserProcess\0".as_ptr()) as u64
    };
    // Continuing breakpoint on CreateFileW to log .uproject opens.
    let tracefile = rest.iter().any(|a| a == "--tracefile");
    let cfw_fn = rest.iter().position(|a| a == "--fn").and_then(|i| rest.get(i + 1)).cloned()
        .unwrap_or_else(|| "NtCreateFile".to_string());
    let nt_mode = cfw_fn.starts_with("Nt");
    let cfw_addr = unsafe {
        let dll = if nt_mode { "ntdll.dll" } else { "kernelbase.dll" };
        let m = GetModuleHandleW(wide(dll).as_ptr());
        let mut cn = cfw_fn.clone().into_bytes(); cn.push(0);
        GetProcAddress(m, cn.as_ptr()) as u64
    };
    eprintln!("[trace] file-trace target: {cfw_fn} @0x{cfw_addr:x} nt_mode={nt_mode}");
    let mut cfw_orig = 0u8;
    let mut cfw_replant_pending = false;
    let mut sink_orig = 0u8;
    let mut sink_planted = false;

    // Probe: breakpoint at xgameruntime!UninitializeApiImpl entry (RVA 0x7b8a0).
    // GDK teardown is called from the WinMain/GuardedMain path that decided to
    // quit; a stack walk here (EXE return addresses) reveals that decision path.
    const PROBE_RVA: u64 = 0x7b8a0;
    let mut xgr_base = 0u64;
    let mut probe_addr = 0u64;
    let mut probe_orig = 0u8;
    let mut exebp_addr = 0u64;
    let mut exebp_orig = 0u8;
    let mut exebp_replant_pending = false;
    let mut exebp_hits: u64 = 0;

    // MDBG_HANGDUMP=<secs>: once, that long after launch, print a stack scan of every thread
    // (for diagnosing stalls: which threads are blocked where).
    let mut hangdump: Vec<u64> = std::env::var("MDBG_HANGDUMP").ok().map(|v| v.split([',', ' ']).filter_map(|x| x.trim().parse::<u64>().ok()).collect()).unwrap_or_default();
    hangdump.sort_unstable();
    hangdump.reverse();
    let mut tids: std::collections::BTreeSet<u32> = std::collections::BTreeSet::new();
    loop {
        if let Some(&s) = hangdump.last() {
            if start.elapsed().as_secs() >= s {
                hangdump.pop();
                eprintln!("[HANG] thread dump at {s}s unix_ms={} ({} threads)", ods_ms(), tids.len());
                for &t in &tids {
                    dump_thread(hproc, t, &modules);
                }
            }
        }
        if start.elapsed() > limit {
            eprintln!("[trace] time limit reached; terminating");
            unsafe { TerminateProcess(hproc, 1); }
            exit_code = None;
            break;
        }
        let mut evt: DEBUG_EVENT = unsafe { std::mem::zeroed() };
        if unsafe { WaitForDebugEventEx(&mut evt, 1000) } == 0 { continue; }
        let mut status = DBG_CONTINUE;
        match evt.code {
            2 | 3 => { tids.insert(evt.tid); }
            4 => { tids.remove(&evt.tid); }
            _ => {}
        }
        match evt.code {
            3 => { // CREATE_PROCESS: hFile(0), hProcess(8), hThread(16), lpBaseOfImage(24)
                let base = rd_u64(&evt.u, 24);
                exe_base = base;
                modules.insert(base, exe.rsplit(['\\','/']).next().unwrap().to_string());
                // Plant a breakpoint at RtlExitUserProcess in the child.
                if sink_addr != 0 && !sink_planted {
                    let mut orig = [0u8; 1];
                    let mut got = 0usize;
                    unsafe { ReadProcessMemory(hproc, sink_addr as *const c_void, orig.as_mut_ptr(), 1, &mut got); }
                    sink_orig = orig[0];
                    let cc = [0xCCu8];
                    let mut wr = 0usize;
                    let ok = unsafe { WriteProcessMemory(hproc, sink_addr as *mut c_void, cc.as_ptr(), 1, &mut wr) };
                    sink_planted = ok != 0 && wr == 1;
                    eprintln!("[trace] exit-sink RtlExitUserProcess @0x{:x} breakpoint planted={}", sink_addr, sink_planted);
                }
                if exebp_rva != 0 {
                    exebp_addr = base + exebp_rva;
                    let mut orig = [0u8; 1]; let mut got = 0usize;
                    unsafe { ReadProcessMemory(hproc, exebp_addr as *const c_void, orig.as_mut_ptr(), 1, &mut got); }
                    exebp_orig = orig[0];
                    let cc = [0xCCu8]; let mut wr = 0usize;
                    let ok = unsafe { WriteProcessMemory(hproc, exebp_addr as *mut c_void, cc.as_ptr(), 1, &mut wr) };
                    eprintln!("[trace] EXE breakpoint @ rva 0x{:x} (0x{:x}) planted={}", exebp_rva, exebp_addr, ok != 0 && wr == 1);
                    let _ = exebp_orig;
                }
                if tracefile && cfw_addr != 0 {
                    let mut orig = [0u8; 1]; let mut got = 0usize;
                    unsafe { ReadProcessMemory(hproc, cfw_addr as *const c_void, orig.as_mut_ptr(), 1, &mut got); }
                    cfw_orig = orig[0];
                    let cc = [0xCCu8]; let mut wr = 0usize;
                    unsafe { WriteProcessMemory(hproc, cfw_addr as *mut c_void, cc.as_ptr(), 1, &mut wr); }
                    eprintln!("[trace] CreateFileW trace bp @0x{:x}", cfw_addr);
                }
            }
            6 => { // LOAD_DLL: hFile(0), lpBaseOfDll(8)
                let hfile = rd_u64(&evt.u, 0) as HANDLE;
                let base = rd_u64(&evt.u, 8);
                let name = dll_name(hfile);
                if name.eq_ignore_ascii_case("xgameruntime.dll") && xgr_base == 0 {
                    xgr_base = base;
                    probe_addr = base + PROBE_RVA;
                    let mut orig = [0u8; 1]; let mut got = 0usize;
                    unsafe { ReadProcessMemory(hproc, probe_addr as *const c_void, orig.as_mut_ptr(), 1, &mut got); }
                    probe_orig = orig[0];
                    let cc = [0xCCu8]; let mut wr = 0usize;
                    let ok = unsafe { WriteProcessMemory(hproc, probe_addr as *mut c_void, cc.as_ptr(), 1, &mut wr) };
                    eprintln!("[trace] xgameruntime.dll @0x{:x}; probe @0x{:x} planted={}", base, probe_addr, ok != 0 && wr == 1);
                }
                modules.insert(base, name);
            }
            8 => { // OUTPUT_DEBUG_STRING: lpDebugStringData(0 u64), fUnicode(8 u16), nLen(10 u16)
                let ptr = rd_u64(&evt.u, 0);
                let unicode = rd_u16(&evt.u, 8);
                let n = rd_u16(&evt.u, 10) as usize;
                if ptr != 0 && n > 0 && n < 65536 {
                    let mut buf = vec![0u8; n];
                    let mut got = 0usize;
                    unsafe { ReadProcessMemory(hproc, ptr as *const c_void, buf.as_mut_ptr(), n, &mut got); }
                    let s = if unicode != 0 {
                        let u16s: Vec<u16> = buf[..got].chunks_exact(2).map(|c| u16::from_le_bytes([c[0],c[1]])).collect();
                        String::from_utf16_lossy(&u16s)
                    } else { String::from_utf8_lossy(&buf[..got]).into_owned() };
                    // The debug string includes its NUL terminator (and usually a newline).
                    let s = s.replace('\0', "");
                    println!("[ODS] {} {}", ods_ms(), s.trim_end_matches(['\n', '\r']));
                    heap::on_ods(hproc, evt.tid, exe_base, tids.len(), &s, &modules);
                }
            }
            1 => { // EXCEPTION. Record: code@0, addr@16, NumParams@24, Info[0]@32, Info[1]@40, firstChance@152
                let ecode = rd_u32(&evt.u, 0);
                let addr = rd_u64(&evt.u, 16);
                let first = rd_u32(&evt.u, 152);
                if ecode == 0x80000003 && tracefile && addr == cfw_addr {
                    cfw_handle(hproc, evt.tid, cfw_addr, cfw_orig, nt_mode);
                    cfw_replant_pending = true;
                    status = DBG_CONTINUE;
                } else if ecode == 0x80000004 {
                    // single-step after a hit: re-plant the breakpoint(s)
                    if cfw_replant_pending {
                        let cc = [0xCCu8]; let mut wr = 0usize;
                        unsafe { WriteProcessMemory(hproc, cfw_addr as *mut c_void, cc.as_ptr(), 1, &mut wr); }
                        cfw_replant_pending = false;
                    }
                    if exebp_replant_pending {
                        let cc = [0xCCu8]; let mut wr = 0usize;
                        unsafe { WriteProcessMemory(hproc, exebp_addr as *mut c_void, cc.as_ptr(), 1, &mut wr); }
                        exebp_replant_pending = false;
                    }
                    status = DBG_CONTINUE;
                } else if ecode == 0x80000003 && exebp_addr != 0 && addr == exebp_addr {
                    exebp_hits += 1;
                    if trace_mode {
                        // Log-and-continue: compact one-liner with caller + key regs, then
                        // restore the original byte, step over it, and re-plant on the
                        // single-step (via exebp_replant_pending + cfw_handle's trap dance).
                        trace_hit(hproc, evt.tid, exe_base, exebp_hits);
                        cfw_handle(hproc, evt.tid, exebp_addr, exebp_orig, nt_mode);
                        exebp_replant_pending = true;
                        if exebp_hits >= trace_limit {
                            eprintln!("[EXEBP] trace limit {trace_limit} reached, terminating");
                            unsafe { TerminateProcess(hproc, 1); }
                        }
                        status = DBG_CONTINUE;
                    } else {
                        eprintln!("[EXEBP] hit rva 0x{:x}", exebp_rva);
                        capture_regs(hproc, evt.tid);
                        eprintln!("[EXEBP] EXE-frame stack:");
                        capture_exit(hproc, evt.tid, exe_base, exe_size, &modules);
                        unsafe { TerminateProcess(hproc, 1); }
                        status = DBG_CONTINUE;
                    }
                } else if ecode == 0x80000003 && probe_addr != 0 && addr == probe_addr {
                    eprintln!("[PROBE] xgameruntime UninitializeApiImpl entered — walking EXE stack");
                    let _ = &xgr_base;
                    capture_exit(hproc, evt.tid, exe_base, exe_size, &modules);
                    unsafe { TerminateProcess(hproc, 1); }
                    status = DBG_CONTINUE;
                } else if ecode == 0x80000003 && addr == sink_addr && sink_planted {
                    eprintln!("[EXIT] RtlExitUserProcess reached — capturing stack");
                    let code = capture_exit(hproc, evt.tid, exe_base, exe_size, &modules);
                    capture_exit_extra(hproc, evt.tid, &modules);
                    unsafe { TerminateProcess(hproc, code.unwrap_or(1)); }
                    status = DBG_CONTINUE;
                } else if ecode == 0x80000003 || ecode == 0x4000001F {
                    status = DBG_CONTINUE; // loader/init breakpoint
                } else if ecode == 0x40010006 || ecode == 0x4001000A {
                    // DBG_PRINTEXCEPTION_C / _WIDE_C: OutputDebugString via RaiseException.
                    // Info[0] = char count (incl NUL), Info[1] = pointer to string.
                    let count = rd_u64(&evt.u, 32) as usize;
                    let ptr = rd_u64(&evt.u, 40);
                    let wide = ecode == 0x4001000A;
                    let bytes = if wide { count.saturating_mul(2) } else { count };
                    if ptr != 0 && bytes > 0 && bytes < 65536 {
                        let mut buf = vec![0u8; bytes];
                        let mut got = 0usize;
                        unsafe { ReadProcessMemory(hproc, ptr as *const c_void, buf.as_mut_ptr(), bytes, &mut got); }
                        let s = if wide {
                            let u: Vec<u16> = buf[..got].chunks_exact(2).map(|c| u16::from_le_bytes([c[0],c[1]])).collect();
                            String::from_utf16_lossy(&u)
                        } else { String::from_utf8_lossy(&buf[..got]).into_owned() };
                        let s = s.trim_end_matches(['\0','\n','\r']);
                        println!("[ODS] {} {s}", ods_ms());
                        heap::on_ods(hproc, evt.tid, exe_base, tids.len(), s, &modules);
                    }
                    status = DBG_CONTINUE;
                } else {
                    exc_count += 1;
                    let m = modules.range(..=addr).next_back().map(|(b,n)| format!("{} +0x{:x}", n, addr-b)).unwrap_or_else(|| "?".into());
                    eprintln!("[EXC] code=0x{:08X} addr=0x{:x} ({}) firstChance={} {}",
                        ecode, addr, m, first, exc_name(ecode));
                    if ecode == 0xC0000005 && !av_captured {
                        av_captured = true;
                        let op = rd_u64(&evt.u, 32); // ExceptionInformation[0]: 0=read 1=write 8=exec
                        let fault = rd_u64(&evt.u, 40); // ExceptionInformation[1]: faulting address
                        let opn = match op { 0 => "READ", 1 => "WRITE", 8 => "EXEC", _ => "?" };
                        eprintln!("[EXC] AV {opn} at 0x{:x}", fault);
                        eprintln!("[EXC] registers + deserialized struct at crash:");
                        capture_regs(hproc, evt.tid);
                        // The package-store lambda keeps the deserialized entry at [rbp+0x70].
                        // Dump rbp-relative so we can see the count vs the null array pointer.
                        dump_frame(hproc, evt.tid);
                        eprintln!("[EXC] access-violation stack (EXE frames):");
                        capture_exit(hproc, evt.tid, exe_base, exe_size, &modules);
                    }
                    status = DBG_EXCEPTION_NOT_HANDLED; // let the app's handler run
                    if exc_count > 400 { eprintln!("[trace] too many exceptions, terminating"); unsafe { TerminateProcess(hproc,1);} }
                }
            }
            5 => { // EXIT_PROCESS: dwExitCode(0)
                let code = rd_u32(&evt.u, 0);
                exit_code = Some(code);
                eprintln!("[trace] EXIT_PROCESS code={} (0x{:x})", code as i32, code);
                unsafe { ContinueDebugEvent(evt.pid, evt.tid, DBG_CONTINUE); }
                break;
            }
            _ => {}
        }
        unsafe { ContinueDebugEvent(evt.pid, evt.tid, status); }
    }
    unsafe { CloseHandle(pi.hThread); CloseHandle(pi.hProcess); }
    eprintln!("[trace] done. exceptions={} exit={:?}", exc_count, exit_code);
}

fn wr_u64(ctx: &mut [u8], o: usize, v: u64) { ctx[o..o+8].copy_from_slice(&v.to_le_bytes()); }
fn wr_u32(ctx: &mut [u8], o: usize, v: u32) { ctx[o..o+4].copy_from_slice(&v.to_le_bytes()); }

/// Handle a CreateFileW breakpoint: log .uproject filenames, then restore the
/// original byte, step back RIP, set the trap flag so we re-plant on the next
/// single-step, and continue.
fn cfw_handle(hproc: HANDLE, tid: u32, cfw_addr: u64, cfw_orig: u8, nt_mode: bool) {
    let hthread = unsafe { OpenThread(0x58, 0, tid) };
    if hthread == 0 { return; }
    let mut ctx = Ctx([0u8; 1256]);
    ctx.0[0x30..0x34].copy_from_slice(&0x0010_0003u32.to_le_bytes());
    if unsafe { GetThreadContext(hthread, ctx.0.as_mut_ptr()) } == 0 { unsafe { CloseHandle(hthread); } return; }
    // Resolve the filename pointer + length.
    let (strptr, strlen_bytes) = if nt_mode {
        // NtCreateFile: r8 = POBJECT_ATTRIBUTES; +0x10 = PUNICODE_STRING; US{len u16, max u16, buf u64}
        let oa = rd_u64(&ctx.0, 0xB8);
        let us = read_u64(hproc, oa + 0x10).unwrap_or(0);
        if us == 0 { unsafe { CloseHandle(hthread); } return; }
        let lenmax = read_u64(hproc, us).unwrap_or(0);
        let len = (lenmax & 0xffff) as usize;
        let buf = read_u64(hproc, us + 8).unwrap_or(0);
        (buf, len)
    } else {
        (rd_u64(&ctx.0, 0x80), 540usize) // CreateFileW/2: rcx = wchar* (NUL-terminated)
    };
    if strptr > 0x10000 && strlen_bytes >= 4 && strlen_bytes < 2048 {
        let mut buf = vec![0u8; strlen_bytes]; let mut got = 0usize;
        unsafe { ReadProcessMemory(hproc, strptr as *const c_void, buf.as_mut_ptr(), buf.len(), &mut got); }
        let u: Vec<u16> = buf[..got].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect();
        let s = String::from_utf16_lossy(&u);
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let l = s.to_lowercase();
        let interesting = l.contains(".u") || l.contains("fortnitegame") || l.contains(".ini") || l.contains(".pak");
        if interesting || n < 30 {
            eprintln!("[FILE #{n}] {:?}", s);
        }
    }
    // restore original byte so the instruction executes
    let ob = [cfw_orig]; let mut wr = 0usize;
    unsafe { WriteProcessMemory(hproc, cfw_addr as *mut c_void, ob.as_ptr(), 1, &mut wr); }
    // rip -= 1, set trap flag
    let rip = rd_u64(&ctx.0, 0xF8).wrapping_sub(1);
    wr_u64(&mut ctx.0, 0xF8, rip);
    let eflags = (rd_u64(&ctx.0, 0x44) as u32) | 0x100;
    wr_u32(&mut ctx.0, 0x44, eflags);
    unsafe { SetThreadContext(hthread, ctx.0.as_ptr()); CloseHandle(hthread); }
}

fn capture_regs(hproc: HANDLE, tid: u32) {
    let hthread = unsafe { OpenThread(0x48, 0, tid) };
    if hthread == 0 { eprintln!("[EXEBP] OpenThread failed"); return; }
    let mut ctx = Ctx([0u8; 1256]);
    ctx.0[0x30..0x34].copy_from_slice(&0x0010_0003u32.to_le_bytes());
    if unsafe { GetThreadContext(hthread, ctx.0.as_mut_ptr()) } == 0 {
        eprintln!("[EXEBP] GetThreadContext failed"); unsafe { CloseHandle(hthread); } return;
    }
    let g = |o: usize| rd_u64(&ctx.0, o);
    eprintln!("[EXEBP] rax=0x{:x} rbx=0x{:x} rcx=0x{:x} rdx=0x{:x} rsi=0x{:x} rdi=0x{:x} r8=0x{:x} r9=0x{:x}",
        g(0x78), g(0x90), g(0x80), g(0x88), g(0xA8), g(0xB0), g(0xB8), g(0xC0));
    eprintln!("[EXEBP] r14=0x{:x} r15=0x{:x} rbp=0x{:x} rsp=0x{:x}", g(0xD8), g(0xE0), g(0xA0), g(0x98));
    // Dump 64 bytes at r14 (commonly a struct/desc pointer in these call sites).
    for (nm, rv) in [("r14", g(0xD8)), ("rdx", g(0x88)), ("r9", g(0xC0))] {
        if rv > 0x10000 {
            let mut buf = [0u8; 64]; let mut got = 0usize;
            unsafe { ReadProcessMemory(hproc, rv as *const c_void, buf.as_mut_ptr(), 64, &mut got); }
            if got >= 16 {
                let hex: String = buf[..got].chunks(4).map(|c| {
                    let mut v=0u32; for (i,&b) in c.iter().enumerate(){v|=(b as u32)<<(i*8);} format!("{v:08x} ")
                }).collect();
                eprintln!("[EXEBP] [{nm}] = {hex}");
            }
        }
    }
    eprintln!("[EXEBP] eax(32)={} (0x{:08x})", g(0x78) as u32 as i32, g(0x78) as u32);
    // Online-heap struct dump: r9 = FD3D12SubAllocatedOnlineHeap, r8 = computed GPU handle.
    // Layout: [r9+0x1C]=increment [r9+0x20]=blockCPUbase [r9+0x28]=blockGPUbase
    //         [r9+8]=parent, parent+0x4C0=globalCPUbase parent+0x4C8=globalGPUbase
    {
        let r9 = g(0xC0); let r8 = g(0xB8);
        if r9 > 0x10000 {
            let incr = read_u64(hproc, r9 + 0x1C).map(|v| v as u32).unwrap_or(0);
            let cpu_base = read_u64(hproc, r9 + 0x20).unwrap_or(0);
            let gpu_base = read_u64(hproc, r9 + 0x28).unwrap_or(0);
            let parent = read_u64(hproc, r9 + 0x8).unwrap_or(0);
            let g_gpu = read_u64(hproc, parent + 0x4C8).unwrap_or(0);
            let g_cpu = read_u64(hproc, parent + 0x4C0).unwrap_or(0);
            eprintln!("[EXEBP] onlineHeap r9=0x{r9:x}: incr={incr} blockCPU=0x{cpu_base:x} blockGPU=0x{gpu_base:x} | parent=0x{parent:x} gCPU=0x{g_cpu:x} gGPU=0x{g_gpu:x} | HANDLE(r8)=0x{r8:x}");
        }
    }
    // AllocateBlock view: rsi = parent (global heap embedded at +0x490), rcx = new block
    // GPU base, rdi = subheap. Dump the global cached bases.
    {
        let rsi = g(0xA8); let rcx = g(0x80); let rdi = g(0xB0);
        if rsi > 0x10000 {
            let g_cpu = read_u64(hproc, rsi + 0x4C0).unwrap_or(0);
            let g_gpu = read_u64(hproc, rsi + 0x4C8).unwrap_or(0);
            let g_incr = read_u64(hproc, rsi + 0x4B8).map(|v| v as u32).unwrap_or(0);
            // global heap embedded at parent+0x490: its own cached base at +0x38 (0x490+0x38=0x4C8 already)
            eprintln!("[EXEBP/AllocBlock] parent(rsi)=0x{rsi:x} gIncr={g_incr} gCPU=0x{g_cpu:x} gGPU=0x{g_gpu:x} | newBlockGPU(rcx)=0x{rcx:x} subheap(rdi)=0x{rdi:x}");
        }
    }
    // Try interpreting each reg as a raw wchar* (direct NUL-terminated string).
    for (name, rv) in [("rcx", g(0x80)), ("rdx", g(0x88)), ("r8", g(0xB8)), ("r9", g(0xC0)), ("rax", g(0x78))] {
        if rv > 0x10000 {
            let mut buf = vec![0u8; 540];
            let mut got = 0usize;
            unsafe { ReadProcessMemory(hproc, rv as *const c_void, buf.as_mut_ptr(), buf.len(), &mut got); }
            if got >= 4 {
                let u: Vec<u16> = buf[..got].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect();
                if u.len() >= 3 {
                    let s = String::from_utf16_lossy(&u);
                    if s.chars().all(|c| (' '..='~').contains(&c)) && s.contains(['/', '\\', '.']) {
                        eprintln!("[EXEBP] {name} as wchar*: {:?}", s);
                    }
                }
            }
        }
    }
    // Try interpreting rcx/rdx/rax as an FString& (TArray<wchar>: [ptr][num i32]).
    for (name, rv) in [("rdx", g(0x88)), ("rcx", g(0x80)), ("rax", g(0x78)), ("r8", g(0xB8))] {
        if let Some(dataptr) = read_u64(hproc, rv) {
            if let Some(lenraw) = read_u64(hproc, rv + 8) {
                let num = (lenraw & 0xffff_ffff) as i32;
                if (1..4096).contains(&num) && dataptr > 0x10000 {
                    let bytes = (num as usize) * 2;
                    let mut buf = vec![0u8; bytes];
                    let mut got = 0usize;
                    unsafe { ReadProcessMemory(hproc, dataptr as *const c_void, buf.as_mut_ptr(), bytes, &mut got); }
                    if got >= 2 {
                        let u: Vec<u16> = buf[..got].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
                        let s = String::from_utf16_lossy(&u);
                        if s.chars().all(|c| c == '\0' || (' '..='~').contains(&c)) {
                            eprintln!("[EXEBP] {name} as FString (len {num}): {:?}", s.trim_end_matches('\0'));
                        }
                    }
                }
            }
        }
    }
    unsafe { CloseHandle(hthread); }
}

fn dump_frame(hproc: HANDLE, tid: u32) {
    let hthread = unsafe { OpenThread(0x48, 0, tid) };
    if hthread == 0 { return; }
    let mut ctx = Ctx([0u8; 1256]);
    ctx.0[0x30..0x34].copy_from_slice(&0x0010_0003u32.to_le_bytes());
    if unsafe { GetThreadContext(hthread, ctx.0.as_mut_ptr()) } == 0 { unsafe { CloseHandle(hthread); } return; }
    let g = |o: usize| rd_u64(&ctx.0, o);
    // x64 CONTEXT: Rbp@0xA0, Rsi@0xA8, R12@0xD8, R13@0xE0.
    let (rbp, rsi, r12) = (g(0xA0), g(0xA8), g(0xD8));
    eprintln!("[FRAME] rbp=0x{:x} r12(this)=0x{:x} rsi=0x{:x}", rbp, r12, rsi);
    for off in [0x70usize, 0x78, 0x98, 0xA0, 0xA8, 0xB0, 0xB8] {
        if let Some(v) = read_u64(hproc, rbp.wrapping_add(off as u64)) {
            eprintln!("[FRAME] [rbp+{:#x}] = 0x{:x}", off, v);
        }
    }
    if let Some(bufp) = read_u64(hproc, r12.wrapping_add(0x18)) {
        eprintln!("[FRAME] [r12+0x18] (srcBuf struct*) = 0x{:x}", bufp);
        if bufp > 0x10000 {
            let ptr = read_u64(hproc, bufp).unwrap_or(0);
            let sz = read_u64(hproc, bufp + 8).unwrap_or(0);
            let b18 = read_u64(hproc, bufp + 0x18).unwrap_or(0);
            eprintln!("[FRAME] srcBuf: dataPtr=0x{:x} size=0x{:x} [+0x18]=0x{:x}", ptr, sz, b18);
            if ptr > 0x10000 {
                let mut buf = [0u8; 48]; let mut got = 0usize;
                unsafe { ReadProcessMemory(hproc, ptr as *const c_void, buf.as_mut_ptr(), 48, &mut got); }
                let hex: String = buf[..got].iter().map(|b| format!("{b:02x}")).collect();
                eprintln!("[FRAME] header data[48] = {hex}");
            }
        }
    }
    unsafe { CloseHandle(hthread); }
}

/// Compact log-and-continue hit logger. Tuned for FCompression::UncompressMemory:
/// rcx=format, rdx=dest, r8=destSize, r9=src, [rsp+0x28]=srcSize, [rsp]=caller.
fn trace_hit(hproc: HANDLE, tid: u32, exe_base: u64, n: u64) {
    let hthread = unsafe { OpenThread(0x48, 0, tid) };
    if hthread == 0 { return; }
    let mut ctx = Ctx([0u8; 1256]);
    ctx.0[0x30..0x34].copy_from_slice(&0x0010_0003u32.to_le_bytes());
    if unsafe { GetThreadContext(hthread, ctx.0.as_mut_ptr()) } == 0 { unsafe { CloseHandle(hthread); } return; }
    let g = |o: usize| rd_u64(&ctx.0, o);
    let rsp = g(0x98);
    let caller = read_u64(hproc, rsp).unwrap_or(0);
    let src_size = read_u64(hproc, rsp + 0x28).unwrap_or(0);
    let (rcx, rdx, r8, r9) = (g(0x80), g(0x88), g(0xB8), g(0xC0));
    let src0 = read_u64(hproc, r9).unwrap_or(0);
    eprintln!("[TRACE {n}] caller=+{:#x} fmt={:#x} dest={:#x} destSize={:#x} src={:#x} srcSize={:#x} src0={:#018x}",
        caller.wrapping_sub(exe_base), rcx, rdx, r8, r9, src_size, src0);
    unsafe { CloseHandle(hthread); }
}

fn read_u64(hproc: HANDLE, addr: u64) -> Option<u64> {
    let mut b = [0u8; 8]; let mut got = 0usize;
    let ok = unsafe { ReadProcessMemory(hproc, addr as *const c_void, b.as_mut_ptr(), 8, &mut got) };
    if ok != 0 && got == 8 { Some(u64::from_le_bytes(b)) } else { None }
}

fn capture_probe(hproc: HANDLE, tid: u32, xgr_base: u64) {
    let hthread = unsafe { OpenThread(0x48, 0, tid) };
    if hthread == 0 { eprintln!("[PROBE] OpenThread failed"); return; }
    let mut ctx = Ctx([0u8; 1256]);
    ctx.0[0x30..0x34].copy_from_slice(&0x0010_0003u32.to_le_bytes());
    if unsafe { GetThreadContext(hthread, ctx.0.as_mut_ptr()) } == 0 {
        eprintln!("[PROBE] GetThreadContext failed"); unsafe { CloseHandle(hthread); } return;
    }
    let rdi = rd_u64(&ctx.0, 0xB0); // edi = failing HRESULT
    let rsi = rd_u64(&ctx.0, 0xA8); // &service slot
    let hr = rdi as u32;
    eprintln!("[PROBE] FAILING service HRESULT = 0x{:08X}", hr);
    eprintln!("[PROBE] service slot rsi=0x{:x}  (array RVA 0x{:x}, index {})",
        rsi, rsi.wrapping_sub(xgr_base), (rsi.wrapping_sub(xgr_base).wrapping_sub(0x126070)) / 8);
    if let Some(svc) = read_u64(hproc, rsi) {
        eprintln!("[PROBE] service object = 0x{:x} (RVA 0x{:x})", svc, svc.wrapping_sub(xgr_base));
        if let Some(vt) = read_u64(hproc, svc) {
            eprintln!("[PROBE] service vtable = 0x{:x}  (RVA 0x{:x}) <- ask IDA what class/RTTI this is", vt, vt.wrapping_sub(xgr_base));
        }
    }
    unsafe { CloseHandle(hthread); }
}

/// Print rip + code-looking stack values of one thread (skips threads idle in the kernel
/// with no title/compat frames near the top of their stack).
fn dump_thread(hproc: HANDLE, tid: u32, modules: &BTreeMap<u64, String>) {
    let hthread = unsafe { OpenThread(0x0002 | 0x0008 | 0x0040, 0, tid) };
    if hthread == 0 { return; }
    let suspended = unsafe { SuspendThread(hthread) } != u32::MAX;
    let mut ctx = Ctx([0u8; 1256]);
    ctx.0[0x30..0x34].copy_from_slice(&0x0010_0003u32.to_le_bytes());
    let got_ctx = unsafe { GetThreadContext(hthread, ctx.0.as_mut_ptr()) } != 0;
    if !got_ctx {
        if suspended { unsafe { ResumeThread(hthread); } }
        unsafe { CloseHandle(hthread); }
        return;
    }
    let rsp = rd_u64(&ctx.0, 0x98);
    let rip = rd_u64(&ctx.0, 0xF8);
    let mut buf = vec![0u8; 16384];
    let mut got = 0usize;
    unsafe { ReadProcessMemory(hproc, rsp as *const c_void, buf.as_mut_ptr(), buf.len(), &mut got); }
    if suspended { unsafe { ResumeThread(hthread); } }
    let name = |v: u64| modules.range(..=v).next_back().filter(|(b, _)| v - **b < 0x6000000 && v > 0x10000).map(|(b, n)| format!("{} +0x{:x}", n, v - b));
    let mut lines = Vec::new();
    let mut i = 0;
    while i + 8 <= got && lines.len() < 60 {
        if let Some(s) = name(rd_u64(&buf, i)) { lines.push(format!("    [rsp+0x{:x}] {}", i, s)); }
        i += 8;
    }
    let interesting = lines.iter().any(|l| !(l.contains("ntdll") || l.contains("kernel32") || l.contains("KernelBase") || l.contains("KERNELBASE")));
    if interesting {
        eprintln!("[HANG] tid={} rip={}", tid, name(rip).unwrap_or_else(|| format!("0x{:x}", rip)));
        for l in lines { eprintln!("{l}"); }
    }
    unsafe { CloseHandle(hthread); }
}

fn capture_exit(hproc: HANDLE, tid: u32, exe_base: u64, exe_size: u64, modules: &BTreeMap<u64, String>) -> Option<u32> {
    const THREAD_GET_CONTEXT: u32 = 0x0008;
    const THREAD_QUERY_INFORMATION: u32 = 0x0040;
    let hthread = unsafe { OpenThread(THREAD_GET_CONTEXT | THREAD_QUERY_INFORMATION, 0, tid) };
    if hthread == 0 { eprintln!("[EXIT] OpenThread failed err={}", unsafe { GetLastError() }); return None; }
    let mut ctx = Ctx([0u8; 1256]);
    ctx.0[0x30..0x34].copy_from_slice(&0x0010_0003u32.to_le_bytes()); // CONTROL|INTEGER
    if unsafe { GetThreadContext(hthread, ctx.0.as_mut_ptr()) } == 0 {
        eprintln!("[EXIT] GetThreadContext failed err={}", unsafe { GetLastError() });
        unsafe { CloseHandle(hthread); } return None;
    }
    let rcx = rd_u64(&ctx.0, 0x80);
    let rsp = rd_u64(&ctx.0, 0x98);
    eprintln!("[EXIT] exitCode arg (rcx)={}  rsp=0x{:x}", rcx as i32, rsp);
    let n = 8192usize;
    let mut buf = vec![0u8; n];
    let mut got = 0usize;
    unsafe { ReadProcessMemory(hproc, rsp as *const c_void, buf.as_mut_ptr(), n, &mut got); }
    eprintln!("[STACK] code-looking stack values, all modules (top first):");
    {
        let mut shown = 0; let mut i = 0;
        while i + 8 <= got && shown < 48 {
            let v = rd_u64(&buf, i);
            if let Some((b, n)) = modules.range(..=v).next_back() {
                if v - b < 0x6000000 && v > 0x10000 {
                    eprintln!("    [rsp+0x{:x}] {} +0x{:x}", i, n, v - b);
                    shown += 1;
                }
            }
            i += 8;
        }
    }
    eprintln!("[EXIT] return addresses on stack that land in the EXE (base 0x{:x}):", exe_base);
    let mut shown = 0; let mut i = 0;
    while i + 8 <= got && shown < 50 {
        let v = rd_u64(&buf, i);
        if v >= exe_base && v < exe_base + exe_size {
            eprintln!("    EXE+0x{:x}", v - exe_base);
            shown += 1;
        }
        i += 8;
    }
    unsafe { CloseHandle(hthread); }
    Some(rcx as u32)
}

fn read_until_fault(hproc: HANDLE, addr: u64, max: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(max);
    let mut a = addr;
    while out.len() < max {
        let page_end = (a | 0xfff) + 1;
        let n = ((page_end - a) as usize).min(max - out.len());
        let mut buf = vec![0u8; n];
        let mut got = 0usize;
        let ok = unsafe { ReadProcessMemory(hproc, a as *const c_void, buf.as_mut_ptr(), n, &mut got) };
        if ok == 0 || got == 0 { break; }
        out.extend_from_slice(&buf[..got]);
        if got < n { break; }
        a += n as u64;
    }
    out
}

fn printable(c: u8) -> bool { (0x20..0x7f).contains(&c) || c == b'\t' || c == b'\n' || c == b'\r' }

fn ascii_runs(buf: &[u8], min: usize) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut s = 0usize;
    for i in 0..=buf.len() {
        let p = i < buf.len() && printable(buf[i]);
        if !p {
            if i - s >= min { out.push((s, String::from_utf8_lossy(&buf[s..i]).into_owned())); }
            s = i + 1;
        }
    }
    out
}

fn leading_ascii(buf: &[u8], min: usize) -> Option<String> {
    let n = buf.iter().position(|&c| !printable(c)).unwrap_or(buf.len());
    if n >= min { Some(String::from_utf8_lossy(&buf[..n]).into_owned()) } else { None }
}

fn stack_module_firsts(stack: &[u8], modules: &BTreeMap<u64, String>) -> Vec<(usize, String, u64)> {
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 8 <= stack.len() {
        let v = rd_u64(stack, i);
        if let Some((b, n)) = modules.range(..=v).next_back() {
            if v - b < 0x6000000 && v > 0x10000 && seen.insert(n.to_ascii_lowercase()) {
                out.push((i, n.clone(), v - b));
            }
        }
        i += 8;
    }
    out
}

fn stack_strings(stack: &[u8], modules: &BTreeMap<u64, String>, mut deref: impl FnMut(u64) -> Vec<u8>) -> Vec<String> {
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    for (off, s) in ascii_runs(stack, 12) {
        if seen.insert(s.clone()) { out.push(format!("    [rsp+0x{:x}] \"{}\"", off, s.escape_debug())); }
    }
    let mut i = 0;
    while i + 8 <= stack.len() && out.len() < 120 {
        let v = rd_u64(stack, i);
        let in_module = modules.range(..=v).next_back().map_or(false, |(b, _)| v - b < 0x6000000);
        if v > 0x10000 && v < 0x8000_0000_0000 && !in_module {
            if let Some(s) = leading_ascii(&deref(v), 12) {
                if seen.insert(s.clone()) { out.push(format!("    [rsp+0x{:x}] -> 0x{:x} \"{}\"", i, v, s.escape_debug())); }
            }
        }
        i += 8;
    }
    out
}

#[repr(C)]
#[derive(Default)]
struct ProcessMemoryCountersEx {
    cb: u32, page_fault_count: u32, peak_working_set: usize, working_set: usize,
    quota_peak_paged_pool: usize, quota_paged_pool: usize, quota_peak_non_paged_pool: usize, quota_non_paged_pool: usize,
    pagefile_usage: usize, peak_pagefile_usage: usize, private_usage: usize,
}

#[repr(C)]
#[derive(Default)]
struct MemoryStatusEx {
    length: u32, memory_load: u32, total_phys: u64, avail_phys: u64, total_page_file: u64, avail_page_file: u64,
    total_virtual: u64, avail_virtual: u64, avail_extended_virtual: u64,
}

#[link(name = "kernel32")]
extern "system" {
    fn K32GetProcessMemoryInfo(h: HANDLE, c: *mut ProcessMemoryCountersEx, cb: u32) -> i32;
    fn GlobalMemoryStatusEx(s: *mut MemoryStatusEx) -> i32;
}

fn capture_exit_extra(hproc: HANDLE, tid: u32, modules: &BTreeMap<u64, String>) {
    const MB: f64 = 1048576.0;
    let mut pmc = ProcessMemoryCountersEx { cb: std::mem::size_of::<ProcessMemoryCountersEx>() as u32, ..Default::default() };
    if unsafe { K32GetProcessMemoryInfo(hproc, &mut pmc, pmc.cb) } != 0 {
        eprintln!("[EXIT] process memory: private={:.0} MB peak_commit={:.0} MB working_set={:.0} MB peak_ws={:.0} MB",
            pmc.private_usage as f64 / MB, pmc.peak_pagefile_usage as f64 / MB, pmc.working_set as f64 / MB, pmc.peak_working_set as f64 / MB);
    } else {
        eprintln!("[EXIT] K32GetProcessMemoryInfo failed err={}", unsafe { GetLastError() });
    }
    let mut ms = MemoryStatusEx { length: std::mem::size_of::<MemoryStatusEx>() as u32, ..Default::default() };
    if unsafe { GlobalMemoryStatusEx(&mut ms) } != 0 {
        eprintln!("[EXIT] system memory: load={}% phys avail={:.0}/{:.0} MB commit avail={:.0}/{:.0} MB",
            ms.memory_load, ms.avail_phys as f64 / MB, ms.total_phys as f64 / MB, ms.avail_page_file as f64 / MB, ms.total_page_file as f64 / MB);
    }
    let hthread = unsafe { OpenThread(0x0002 | 0x0008 | 0x0040, 0, tid) };
    if hthread == 0 { return; }
    let suspended = unsafe { SuspendThread(hthread) } != u32::MAX;
    let mut ctx = Ctx([0u8; 1256]);
    ctx.0[0x30..0x34].copy_from_slice(&0x0010_0003u32.to_le_bytes());
    let ok = unsafe { GetThreadContext(hthread, ctx.0.as_mut_ptr()) };
    unsafe { CloseHandle(hthread); }
    if ok == 0 { return; }
    let rsp = rd_u64(&ctx.0, 0x98);
    let stack = read_until_fault(hproc, rsp, 0x10000);
    eprintln!("[EXIT] exiting tid={} stack bytes read={:#x}; first hit of each module (deepest frames show who started the work):", tid, stack.len());
    for (off, n, rva) in stack_module_firsts(&stack, modules) {
        eprintln!("    [rsp+0x{:x}] {} +0x{:x}", off, n, rva);
    }
    eprintln!("[EXIT] strings on the exiting stack and behind its pointers:");
    for l in stack_strings(&stack, modules, |p| read_until_fault(hproc, p, 512)) { eprintln!("{l}"); }
}

fn rd_u16(u: &[u8], o: usize) -> u16 { u16::from_le_bytes([u[o],u[o+1]]) }
fn rd_u32(u: &[u8], o: usize) -> u32 { u32::from_le_bytes([u[o],u[o+1],u[o+2],u[o+3]]) }
fn rd_u64(u: &[u8], o: usize) -> u64 { let mut b=[0u8;8]; b.copy_from_slice(&u[o..o+8]); u64::from_le_bytes(b) }

fn dll_name(hfile: HANDLE) -> String {
    if hfile == 0 || hfile == -1 { return "<nofile>".into(); }
    let mut buf = [0u16; 1024];
    let n = unsafe { GetFinalPathNameByHandleW(hfile, buf.as_mut_ptr(), buf.len() as u32, 0) };
    if n == 0 || n as usize >= buf.len() { return "<?>".into(); }
    let s = String::from_utf16_lossy(&buf[..n as usize]);
    s.rsplit(['\\','/']).next().unwrap_or("?").to_string()
}

fn exc_name(c: u32) -> &'static str {
    match c {
        0xC0000005 => "ACCESS_VIOLATION", 0xC000001D => "ILLEGAL_INSTRUCTION",
        0xE06D7363 => "C++ exception", 0xC0000409 => "fastfail/stack_buffer_overrun",
        0xC0000374 => "heap_corruption", 0x40010006 => "DBG_PRINTEXCEPTION_C",
        0x4001000A => "DBG_PRINTEXCEPTION_WIDE_C", 0xC00000FD => "stack_overflow",
        0xC0000417 => "invalid_crt_param", 0xC06D007E => "module_not_found",
        0xC06D007F => "procedure_not_found", _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mods() -> BTreeMap<u64, String> {
        let mut m = BTreeMap::new();
        m.insert(0x7ff9_0000_0000, "nvgpucomp64.dll".to_string());
        m.insert(0x7ff9_1000_0000, "d3d12_xs.dll".to_string());
        m
    }

    #[test]
    fn ascii_runs_keep_long_text_only() {
        let mut b = vec![0u8; 4];
        b.extend_from_slice(b"LLVM ERROR: cannot select");
        b.push(0);
        b.extend_from_slice(b"short");
        b.push(0xff);
        let r = ascii_runs(&b, 12);
        assert_eq!(r, vec![(4, "LLVM ERROR: cannot select".to_string())]);
        assert_eq!(leading_ascii(b"GISelFailure: x\0junk", 12).as_deref(), Some("GISelFailure: x"));
        assert_eq!(leading_ascii(b"tiny\0", 12), None);
    }

    #[test]
    fn module_firsts_list_each_module_once_in_stack_order() {
        let m = mods();
        let vals = [0x7ff9_0000_1234u64, 0x7ff9_0000_5678, 0x1234, 0x7ff9_1000_0042, 0x7ff9_0000_9999];
        let stack: Vec<u8> = vals.iter().flat_map(|v| v.to_le_bytes()).collect();
        let r = stack_module_firsts(&stack, &m);
        assert_eq!(r, vec![(0, "nvgpucomp64.dll".to_string(), 0x1234), (0x18, "d3d12_xs.dll".to_string(), 0x42)]);
    }

    #[test]
    fn stack_strings_follow_heap_pointers_but_not_module_code() {
        let m = mods();
        let heap = 0x0000_01c3_6000_0000u64;
        let code = 0x7ff9_0000_4000u64;
        let mut stack: Vec<u8> = Vec::new();
        stack.extend_from_slice(&heap.to_le_bytes());
        stack.extend_from_slice(&code.to_le_bytes());
        stack.extend_from_slice(b"inline stack message\0\0\0\0");
        let mut asked = Vec::new();
        let r = stack_strings(&stack, &m, |p| {
            asked.push(p);
            if p == heap { b"cannot select: %5:_(s32) = G_FOO %3, %4 (in function: main)\0".to_vec() } else { Vec::new() }
        });
        assert!(asked.contains(&heap) && !asked.contains(&code));
        assert!(r.iter().any(|l| l.contains("[rsp+0x10] \"inline stack message\"")));
        assert!(r.iter().any(|l| l.contains("-> 0x1c360000000 \"cannot select: %5:_(s32) = G_FOO %3, %4 (in function: main)\"")));
    }
}