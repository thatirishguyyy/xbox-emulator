use crate::monitor::*;
use crate::profile::{self, Profile};
use crate::win::SetEvent;
use std::sync::atomic::Ordering;
use xtfmsg::chk::Activity;
use xtfmsg::com::{array, json, Com, Obj};
use xtfmsg::msg::{deserialize_body, get_json_u32, serialize_end_body, ErrorContext};
use xtfmsg::sys::*;
use xtfmsg::xbtp;

pub const SERVER_CPP: &str = "xbox\\devex\\xtf\\dll\\debugmonitor\\server.cpp";
pub const EXTENSION_VERSION: &str = profile::P22000.extension_version;

pub const OP_DETACH_PROCESS: u32 = 2;
pub const OP_MONITOR_PROCESS: u32 = 3;
pub const OP_CONTINUE_PROCESS: u32 = 4;
pub const OP_QUERY_MONITOR: u32 = 5;

pub const DEFAULT_FILTER_FLAGS: u32 = 0x100;

struct Op {
    func: &'static str,
    ctx: ErrorContext,
    ret: HRESULT,
    version: &'static str,
    activity: Option<Box<Activity>>,
}

impl Op {
    unsafe fn fail(&mut self, hr: HRESULT, line: u32, expr: &str) {
        self.ctx.append_error(hr, SERVER_CPP, line as i32);
        self.ctx.hr = hr;
        self.ret = hr;
        crate::chk::chk(SERVER_CPP, self.func, line, "_hr", hr, expr);
    }

    unsafe fn begin(p: &Profile, func: &'static str, activity: &'static [u8], reply: usize) -> Op {
        let activity = if p.activities { Some(Activity::start(activity)) } else { None };
        Op { func, ctx: ErrorContext::new(reply as Obj), ret: S_OK, version: p.extension_version, activity }
    }

    unsafe fn finish(mut self) -> HRESULT {
        let ret = self.ret;
        if ret < 0 {
            let v = wide(self.version);
            let empty = wide("");
            xtfmsg::xce::log_operation_result(v.as_ptr(), empty.as_ptr(), empty.as_ptr(), ret);
        }
        let activity = self.activity.take();
        drop(self.ctx);
        if let Some(a) = activity {
            a.stop();
        }
        ret
    }
}

fn w(s: &str) -> Vec<u16> {
    wide(s)
}

unsafe fn read_u32(body: Obj, name: &str, out: &mut u32) -> HRESULT {
    let n = w(name);
    get_json_u32(body, n.as_ptr(), out)
}

pub unsafe extern "system" fn query_monitor_handler(_session: usize, _request: usize, reply: usize) -> HRESULT {
    query_monitor(profile::current(), _session, _request, reply)
}

pub unsafe fn query_monitor(pf: &'static Profile, _session: usize, _request: usize, reply: usize) -> HRESULT {
    let mut op = Op::begin(pf, "QueryMonitorHandler", b"XtfDebugMonitor: QueryMonitorHandler\0", reply);
    let mut reply_body = Com::null();
    let mut processes = Com::null();
    'done: {
        let hr = xbtp::create_json_object(reply_body.put());
        if hr < 0 {
            op.fail(hr, pf.query(166), "XbtpCreateJsonObject(&replyBody)");
            break 'done;
        }
        let hr = xbtp::create_json_array(processes.put());
        if hr < 0 {
            op.fail(hr, pf.query(167), "XbtpCreateJsonArray(&jsonArray)");
            break 'done;
        }
        if let Some(dm) = instance() {
            for p in dm.get_monitor_list() {
                let mut po = Com::null();
                let mut ca = Com::null();
                let hr = xbtp::create_json_object(po.put());
                if hr < 0 {
                    op.fail(hr, pf.query(182), "XbtpCreateJsonObject(&processObject)");
                    break 'done;
                }
                let hr = xbtp::create_json_array(ca.put());
                if hr < 0 {
                    op.fail(hr, pf.query(183), "XbtpCreateJsonArray(&clientArray)");
                    break 'done;
                }
                let hr = json::set_number(po.get(), w("ProcessId").as_ptr(), p.pid as f64);
                if hr < 0 {
                    op.fail(hr, pf.query(185), "processObject->SetNumber(ProcessIdProperty, (double)iter->Pid)");
                    break 'done;
                }
                let hr = json::set_number(po.get(), w("IsActive").as_ptr(), p.active as u32 as f64);
                if hr < 0 {
                    op.fail(hr, pf.query(186), "processObject->SetNumber(IsActiveProperty, (double)iter->Active)");
                    break 'done;
                }
                let hr = json::set_number(po.get(), w("IsBroken").as_ptr(), p.broken as u32 as f64);
                if hr < 0 {
                    op.fail(hr, pf.query(187), "processObject->SetNumber(IsBrokenProperty, (double)iter->Broken)");
                    break 'done;
                }
                for c in &p.clients {
                    let mut co = Com::null();
                    let hr = xbtp::create_json_object(co.put());
                    if hr < 0 {
                        op.fail(hr, pf.query(194), "XbtpCreateJsonObject(&clientObject)");
                        break 'done;
                    }
                    let hr = json::set_number(co.get(), w("ClientIdentifier").as_ptr(), c.client_id as f64);
                    if hr < 0 {
                        op.fail(hr, pf.query(196), "clientObject->SetNumber(ClientSessionIdentifierProperty, (double)iterClient->ClientId)");
                        break 'done;
                    }
                    let hr = json::set_number(co.get(), w("IsActive").as_ptr(), c.active as u32 as f64);
                    if hr < 0 {
                        op.fail(hr, pf.query(197), "clientObject->SetNumber(IsActiveProperty, (double)iterClient->Active)");
                        break 'done;
                    }
                    let hr = array::append_object(ca.get(), co.get());
                    if hr < 0 {
                        op.fail(hr, pf.query(199), "clientArray->AppendObject(clientObject)");
                        break 'done;
                    }
                }
                let hr = json::set_array(po.get(), w("Clients").as_ptr(), ca.get());
                if hr < 0 {
                    op.fail(hr, pf.query(201), "processObject->SetArray(ClientArrayProperty, clientArray)");
                    break 'done;
                }
                let hr = array::append_object(processes.get(), po.get());
                if hr < 0 {
                    op.fail(hr, pf.query(203), "jsonArray->AppendObject(processObject)");
                    break 'done;
                }
            }
        }
        let hr = json::set_array(reply_body.get(), w("Processes").as_ptr(), processes.get());
        if hr < 0 {
            op.fail(hr, pf.query(207), "replyBody->SetArray(ProcessArrayProperty, jsonArray)");
            break 'done;
        }
        let hr = serialize_end_body(reply_body.get(), reply as Obj);
        if hr < 0 {
            op.fail(hr, pf.query(208), "XtfSerializeJsonBody(replyBody, reply)");
            break 'done;
        }
    }
    drop(processes);
    drop(reply_body);
    op.finish()
}

pub unsafe extern "system" fn monitor_process_handler(session: usize, request: usize, reply: usize) -> HRESULT {
    monitor_process(profile::current(), session, request, reply)
}

pub unsafe fn monitor_process(pf: &'static Profile, session: usize, request: usize, reply: usize) -> HRESULT {
    let mut op = Op::begin(pf, "MonitorProcessHandler", b"XtfDebugMonitor: MonitorProcessHandler\0", reply);
    let mut body = Com::null();
    'done: {
        let hr = deserialize_body(request as Obj, body.put());
        if hr < 0 {
            op.fail(hr, pf.monitor(233), "XtfDeserializeJsonBody(received, &body)");
            break 'done;
        }
        let mut pid = 0u32;
        let hr = read_u32(body.get(), "ProcessId", &mut pid);
        if hr < 0 {
            op.fail(hr, pf.monitor(234), "XtfGetJsonProperty(body, ProcessIdProperty, &pid)");
            break 'done;
        }
        let mut flags = 0u32;
        if read_u32(body.get(), "FilterFlags", &mut flags) < 0 {
            flags = DEFAULT_FILTER_FLAGS;
        }
        if flags & 1 != 0 {
            flags |= 2;
        }
        let Some(dm) = instance() else {
            op.fail(E_OUTOFMEMORY, pf.monitor(255), "E_OUTOFMEMORY");
            break 'done;
        };
        let hr = dm.monitor_process(pf, pid, flags, session);
        if hr < 0 {
            op.fail(hr, pf.monitor(268), "dbgMon->MonitorProcess(pid, filterFlags, session)");
            break 'done;
        }
    }
    drop(body);
    op.finish()
}

pub unsafe extern "system" fn detach_process_handler(_session: usize, request: usize, reply: usize) -> HRESULT {
    detach_process(profile::current(), _session, request, reply)
}

pub unsafe fn detach_process(pf: &'static Profile, _session: usize, request: usize, reply: usize) -> HRESULT {
    let mut op = Op::begin(pf, "DetachProcessHandler", b"XtfDebugMonitor: DetachProcessHandler\0", reply);
    if let Some(dm) = instance() {
        let mut body = Com::null();
        'done: {
            let hr = deserialize_body(request as Obj, body.put());
            if hr < 0 {
                op.fail(hr, pf.detach(300), "XtfDeserializeJsonBody(received, &body)");
                break 'done;
            }
            let mut pid = 0u32;
            let hr = read_u32(body.get(), "ProcessId", &mut pid);
            if hr < 0 {
                op.fail(hr, pf.detach(301), "XtfGetJsonProperty(body, ProcessIdProperty, &processId)");
                break 'done;
            }
            let mut force_all = 0u32;
            if read_u32(body.get(), "ForceAll", &mut force_all) < 0 {
                force_all = 0;
            }
            let mut client_id = 0u32;
            if read_u32(body.get(), "ClientIdentifier", &mut client_id) < 0 {
                client_id = 0;
            }
            if let Some(m) = dm.find(pid) {
                if force_all != 0 {
                    m.force_all.store(true, Ordering::SeqCst);
                } else if client_id == 0 {
                    m.detach_count.fetch_add(1, Ordering::SeqCst);
                } else if let Some(c) = m.find_client(client_id) {
                    SetEvent(c.stop_event);
                }
            }
        }
    }
    op.finish()
}

pub unsafe extern "system" fn continue_process_handler(_session: usize, request: usize, reply: usize) -> HRESULT {
    continue_process(profile::current(), _session, request, reply)
}

pub unsafe fn continue_process(pf: &'static Profile, _session: usize, request: usize, reply: usize) -> HRESULT {
    let mut op = Op::begin(pf, "ContinueProcessHandler", b"XtfDebugMonitor: ContinueProcessHandler\0", reply);
    if let Some(dm) = instance() {
        let mut body = Com::null();
        'done: {
            let hr = deserialize_body(request as Obj, body.put());
            if hr < 0 {
                op.fail(hr, pf.cont(345), "XtfDeserializeJsonBody(received, &body)");
                break 'done;
            }
            let mut pid = 0u32;
            let hr = read_u32(body.get(), "ProcessId", &mut pid);
            if hr < 0 {
                op.fail(hr, pf.cont(346), "XtfGetJsonProperty(body, ProcessIdProperty, &processId)");
                break 'done;
            }
            let mut status = 0u32;
            let hr = read_u32(body.get(), "ExceptionContinue", &mut status);
            if hr < 0 {
                op.fail(hr, pf.cont(347), "XtfGetJsonProperty(body, ContinueStatusProperty, &continueStatus)");
                break 'done;
            }
            if let Some(m) = dm.find(pid) {
                m.continue_status.store(status, Ordering::SeqCst);
                m.broken.store(false, Ordering::SeqCst);
                SetEvent(m.continue_event);
            }
        }
    }
    op.finish()
}
