#![allow(non_snake_case, clippy::missing_safety_doc)]

pub mod win;
pub mod chk;
pub mod monitor;
pub mod debugclient;
pub mod server;
pub mod client;
pub mod profile;
pub mod imports;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod client_tests;
#[cfg(test)]
mod oracle_tests;
#[cfg(test)]
mod build_tests;

use xtfmsg::com::metadata::OperationFn;
use xtfmsg::com::Obj;
use xtfmsg::entry::{extension_entry_point_with, log_site, EntrySite};
use xtfmsg::sys::*;

pub const DEBUG_MONITOR_EXTENSION_ID: GUID = GUID::new(0x9141933c, 0xdca8, 0x43d0, [0xb6, 0x09, 0xb4, 0x07, 0x09, 0xbf, 0xe2, 0xd5]);

pub static HANDLERS: [(u32, OperationFn, &str, u32); 4] = [
    (server::OP_QUERY_MONITOR, server::query_monitor_handler, "metadata->AddFunction(QueryMonitorOperationId, QueryMonitorHandler)", 134),
    (server::OP_MONITOR_PROCESS, server::monitor_process_handler, "metadata->AddFunction(MonitorProcessOperationId, MonitorProcessHandler)", 135),
    (server::OP_DETACH_PROCESS, server::detach_process_handler, "metadata->AddFunction(DetachProcessOperationId, DetachProcessHandler)", 136),
    (server::OP_CONTINUE_PROCESS, server::continue_process_handler, "metadata->AddFunction(ContinueProcessOperationId, ContinueProcessHandler)", 137),
];

pub static SITE: EntrySite = EntrySite {
    file: server::SERVER_CPP,
    func: "XtfServerEntryPoint",
    bad_extension_line: 118,
    bad_metadata_line: 122,
    create_line: 132,
    handlers: &HANDLERS,
    bad_extension_msg: "",
    bad_metadata_msg: "",
};

#[no_mangle]
pub unsafe extern "system" fn XtfOrdinal_4(extension: *const GUID, iid: *const GUID, ppv: *mut Obj) -> HRESULT {
    imports::bind();
    extension_entry_point_with(&SITE, &DEBUG_MONITOR_EXTENSION_ID, extension, iid, ppv, &mut || {
        if monitor::instance().is_none() {
            log_site(&SITE, 129, "E_OUTOFMEMORY", E_OUTOFMEMORY, "");
            return E_OUTOFMEMORY;
        }
        S_OK
    })
}
