use crate::console::*;
use crate::launch::remote_run_create_process;
use crate::variant::ConsoleCopy;
use crate::win::*;
use core::ptr::null_mut;
use xtfmsg::chk::Activity;
use xtfmsg::com::{self, json, Com, Obj};
use xtfmsg::msg::{deserialize_body, get_json_i32, get_json_u32, serialize_end_body, set_number, ErrorContext};
use xtfmsg::sys::*;
use xtfmsg::xbtp;

pub const RUN_OPERATION_ID: u32 = 1;
pub const RUN_FLAG_WAIT: u32 = 1;
pub const RUN_FLAG_REDIRECT_OUTPUT: u32 = 0x4000_0000;
pub const RUN_FLAG_REDIRECT_INPUT: u32 = 0x8000_0000;
pub const RUN_FLAG_REDIRECT_MASK: u32 = 0xC000_0000;

struct Op {
    ctx: ErrorContext,
    hr: HRESULT,
    copy: ConsoleCopy,
}

impl Op {
    fn l(&self, line_22000: u32) -> u32 {
        self.copy.run_handler_line(line_22000)
    }

    unsafe fn fail(&mut self, hr: HRESULT, line_22000: u32, expr: &str) {
        let line = self.l(line_22000);
        self.ctx.append_error(hr, RUN_CPP, line as i32);
        self.hr = hr;
        ck("RunHandler", line, expr, hr, "");
    }
}

struct Pipes {
    stdin_read: OwnedHandle,
    stdin_write: OwnedHandle,
    stdout_read: OwnedHandle,
    stdout_write: OwnedHandle,
    stderr_read: OwnedHandle,
    stderr_write: OwnedHandle,
    process: OwnedHandle,
    thread: OwnedHandle,
}

unsafe fn create_pipe(sa: &SecurityAttributes, read: &mut OwnedHandle, write: &mut OwnedHandle, parent_end_is_write: bool) -> Result<(), HRESULT> {
    let mut r: HANDLE = null_mut();
    let mut wr: HANDLE = null_mut();
    if CreatePipe(&mut r, &mut wr, sa, 0) == 0 {
        return Err(last_error_or_fail());
    }
    *read = OwnedHandle(r);
    *write = OwnedHandle(wr);
    let parent = if parent_end_is_write { wr } else { r };
    if SetHandleInformation(parent, HANDLE_FLAG_INHERIT, 0) == 0 {
        return Err(last_error_or_fail());
    }
    Ok(())
}

pub unsafe extern "system" fn run_handler(session: usize, received: usize, reply: usize) -> HRESULT {
    let copy = crate::variant::current();
    let activity = copy.has_run_handler_activity().then(|| Activity::start(b"XtfRemoteRun: RunHandler\0"));
    let mut op = Op { ctx: ErrorContext::new(reply as Obj), hr: S_OK, copy };
    run(&mut op, session as Obj, received as Obj, reply as Obj);
    op.ctx.hr = op.hr;
    drop(op);
    if let Some(a) = activity {
        a.stop();
    }
    S_OK
}

unsafe fn run(op: &mut Op, session: Obj, received: Obj, reply: Obj) {
    let mut req = Com::null();
    let mut result_body = Com::null();
    let mut command: BSTR = null_mut();
    let mut directory: BSTR = null_mut();
    let mut pipes = Pipes {
        stdin_read: OwnedHandle::null(),
        stdin_write: OwnedHandle::null(),
        stdout_read: OwnedHandle::null(),
        stdout_write: OwnedHandle::null(),
        stderr_read: OwnedHandle::null(),
        stderr_write: OwnedHandle::null(),
        process: OwnedHandle::null(),
        thread: OwnedHandle::null(),
    };
    run_inner(op, session, received, reply, &mut req, &mut result_body, &mut command, &mut directory, &mut pipes);
    free_bstr(command);
    free_bstr(directory);
}

#[allow(clippy::too_many_arguments)]
unsafe fn run_inner(op: &mut Op, session: Obj, received: Obj, reply: Obj, req: &mut Com, result_body: &mut Com, command: &mut BSTR, directory: &mut BSTR, p: &mut Pipes) {
    let hr = deserialize_body(received, req.put());
    if hr < 0 {
        return op.fail(hr, 925, "XtfDeserializeJsonBody(received, &reqBody)");
    }
    let mut flags = 0u32;
    let n = w(P_FLAGS);
    let hr = get_json_u32(req.get(), n.as_ptr(), &mut flags);
    if hr < 0 {
        return op.fail(hr, 926, "XtfGetJsonProperty(reqBody, FlagsProperty, &dwFlags)");
    }
    let mut period = 0u32;
    let n = w(P_PERIOD);
    let hr = get_json_u32(req.get(), n.as_ptr(), &mut period);
    if hr < 0 {
        return op.fail(hr, 927, "XtfGetJsonProperty(reqBody, PeriodProperty, &dwPeriod)");
    }
    let n = w(P_COMMAND_LINE);
    let hr = json::get_string(req.get(), n.as_ptr(), command);
    if hr < 0 {
        return op.fail(hr, 928, "reqBody->GetString(CommandLineProperty, &pwszCommandLine)");
    }
    let n = w(P_WORKING_DIRECTORY);
    let hr = json::get_string(req.get(), n.as_ptr(), directory);
    if hr < 0 {
        return op.fail(hr, 929, "reqBody->GetString(WorkingDirProperty, &pwszWorkingDirectory)");
    }
    let mut unused = 0u32;
    let n = w(P_RUN_ERROR_OUTPUT);
    let hr = get_json_u32(req.get(), n.as_ptr(), &mut unused);
    let supports_stderr = hr != E_JSON_NOT_FOUND;
    let hr = if supports_stderr { hr } else { S_OK };
    if hr < 0 {
        return op.fail(hr, 932, "XtfTryGetJsonProperty(reqBody, RunErrorOutputProperty, &fSupportsStdErr, &dwUnused)");
    }
    let hr = xbtp::create_json_object(result_body.put());
    if hr < 0 {
        return op.fail(hr, 934, "XbtpCreateJsonObject(&resultBody)");
    }
    let mut si = StartupInfoExW::zeroed();
    si.si.cb = core::mem::size_of::<StartupInfoExW>() as u32;
    let sa = SecurityAttributes { length: core::mem::size_of::<SecurityAttributes>() as u32, descriptor: null_mut(), inherit: 1 };
    if flags & RUN_FLAG_REDIRECT_MASK != 0 {
        si.si.flags |= STARTF_USESTDHANDLES;
        if flags & RUN_FLAG_REDIRECT_INPUT != 0 {
            if let Err(hr) = create_pipe(&sa, &mut p.stdin_read, &mut p.stdin_write, true) {
                op.hr = hr;
                return;
            }
            si.si.std_input = p.stdin_read.get();
        } else {
            si.si.std_input = INVALID_HANDLE_VALUE;
        }
        let mut out = INVALID_HANDLE_VALUE;
        if flags & RUN_FLAG_REDIRECT_OUTPUT != 0 {
            if let Err(hr) = create_pipe(&sa, &mut p.stdout_read, &mut p.stdout_write, false) {
                op.hr = hr;
                return;
            }
            out = p.stdout_write.get();
            si.si.std_output = out;
        } else {
            si.si.std_output = INVALID_HANDLE_VALUE;
        }
        if supports_stderr {
            if let Err(hr) = create_pipe(&sa, &mut p.stderr_read, &mut p.stderr_write, false) {
                op.hr = hr;
                return;
            }
            si.si.std_error = p.stderr_write.get();
        } else {
            si.si.std_error = out;
        }
    }
    let mut pi = ProcessInformation::zeroed();
    let hr = remote_run_create_process(op.copy, *command, *directory, flags, &mut si, &mut pi);
    ck("RunHandler", op.l(984), "RemoteRunCreateProcess(pwszCommandLine, pwszWorkingDirectory, dwFlags, &StartupInfo, &ProcessInfo)", hr, "");
    op.hr = hr;
    if hr < 0 {
        return;
    }
    p.process = OwnedHandle(pi.process);
    p.thread = OwnedHandle(pi.thread);
    if flags & RUN_FLAG_WAIT == 0 {
        return;
    }
    let mut running = true;
    let mut out_read = false;
    let mut err_read = false;
    loop {
        if running {
            let start = GetTickCount64();
            let mut remaining = period;
            let mut hr = S_OK;
            let handles = [p.process.get()];
            let ret = loop {
                let ret = WaitForMultipleObjectsEx(1, handles.as_ptr(), 0, remaining, 1);
                let spent = GetTickCount64().wrapping_sub(start) as u32;
                if remaining != INFINITE {
                    remaining = if spent < period { period - spent } else { 0 };
                }
                if ret == WAIT_IO_COMPLETION {
                    if hr >= 0 {
                        continue;
                    }
                    break ret;
                }
                if ret == WAIT_FAILED {
                    hr = hr_from_wait(xtfmsg::sys::GetLastError());
                    if hr >= 0 {
                        continue;
                    }
                }
                break ret;
            };
            let poll = if ret == WAIT_OBJECT_0 && hr >= 0 {
                let mut code = 0u32;
                if GetExitCodeProcess(p.process.get(), &mut code) == 0 {
                    op.hr = last_error_or_fail();
                    return;
                }
                let n = w(P_EXIT_CODE);
                let hr = set_number(result_body.get(), n.as_ptr(), code as f64);
                if hr < 0 {
                    return op.fail(hr, 1017, "XtfSetJsonProperty(resultBody, ExitCodeProperty, dwExitCode)");
                }
                running = false;
                false
            } else if ret == WAIT_TIMEOUT && remaining != INFINITE {
                true
            } else if hr < 0 && hr == E_PENDING {
                true
            } else {
                let hr = if hr < 0 { hr } else { E_FAIL };
                op.hr = hr;
                ck("RunHandler", op.l(1027), "XtfWaitForSingleObject", hr, "");
                return;
            };
            if poll {
                loop {
                    let mut msg = Com::null();
                    let hr = com::session::receive_more_message(session, 0, msg.put());
                    if hr == E_PENDING {
                        break;
                    }
                    if hr < 0 {
                        op.ctx.append_error(hr, RUN_CPP, op.l(1068) as i32);
                        op.hr = hr;
                        ck("RunHandler", op.l(1068), "hr", hr, "");
                        return;
                    }
                    let mut body = Com::null();
                    let hr = deserialize_body(msg.get(), body.put());
                    if hr < 0 {
                        return op.fail(hr, 1044, "XtfDeserializeJsonBody(reply, &replyBody)");
                    }
                    let mut msg_id = 0u32;
                    let n = w(P_MSG_ID);
                    let hr = get_json_u32(body.get(), n.as_ptr(), &mut msg_id);
                    if hr < 0 {
                        return op.fail(hr, 1046, "XtfGetJsonProperty(replyBody, MsgIdProperty, &msgId)");
                    }
                    if msg_id == MSGID_RUN_INPUT {
                        if flags & RUN_FLAG_REDIRECT_INPUT != 0 {
                            let hr = recv_console_input(p.stdin_write.get(), msg.get());
                            ck("RunHandler", op.l(1050), "RecvConsoleInput(hStdInWrite, reply)", hr, "");
                            if hr < 0 {
                                op.hr = hr;
                                return;
                            }
                        }
                    } else if msg_id == MSGID_RUN_RESULT {
                        let mut result = 0i32;
                        let n = w(P_RESULT);
                        let hr = get_json_i32(body.get(), n.as_ptr(), &mut result);
                        if hr < 0 {
                            return op.fail(hr, 1056, "XtfGetJsonProperty(replyBody, ResultProperty, &resultHr)");
                        }
                        if result < 0 {
                            return op.fail(result, 1057, "resultHr");
                        }
                        op.hr = S_OK;
                        return;
                    }
                }
            }
        }
        if flags & RUN_FLAG_REDIRECT_OUTPUT != 0 {
            let hr = send_console_output(p.stdout_read.get(), session, Some(&mut out_read));
            ck("RunHandler", op.l(1078), "SendConsoleOutput(hStdOutRead, session, &fStdOutRead)", hr, "");
            if hr < 0 {
                op.hr = hr;
                return;
            }
            if !p.stderr_read.get().is_null() {
                let hr = send_console_error(p.stderr_read.get(), session, Some(&mut err_read));
                ck("RunHandler", op.l(1082), "SendConsoleError(hStdErrRead, session, &fStdErrRead)", hr, "");
                if hr < 0 {
                    op.hr = hr;
                    return;
                }
            }
        }
        if !running && !out_read && !err_read {
            break;
        }
    }
    let hr = serialize_end_body(result_body.get(), reply);
    if hr < 0 {
        return op.fail(hr, 1088, "XtfSerializeJsonBody(resultBody, reply)");
    }
    op.hr = S_OK;
}
