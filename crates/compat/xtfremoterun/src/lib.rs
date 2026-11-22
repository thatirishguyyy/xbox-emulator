#![allow(non_snake_case, clippy::missing_safety_doc)]

pub mod win;
pub mod chk;
pub mod console;
pub mod launch;
pub mod server;
pub mod client;
pub mod title;
pub mod variant;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod client_tests;

use xtfmsg::com::metadata::OperationFn;
use xtfmsg::com::Obj;
use xtfmsg::entry::{extension_entry_point, EntrySite};
use xtfmsg::sys::*;

pub const REMOTE_RUN_EXTENSION_ID: GUID = GUID::new(0xd77fe8b8, 0xa034, 0x46ec, [0xac, 0xc3, 0x6b, 0xc8, 0x1c, 0x61, 0x07, 0x8c]);
pub const SERVER_CPP: &str = "xbox\\devex\\xtf\\dll\\remoterun\\server.cpp";

pub static HANDLERS: [(u32, OperationFn, &str, u32); 1] = [(server::RUN_OPERATION_ID, server::run_handler, "metadata->AddFunction(RunOperationId, RunHandler)", 27)];

pub static SITE: EntrySite = EntrySite {
    file: SERVER_CPP,
    func: "XtfExtensionEntryPoint",
    bad_extension_line: 16,
    bad_metadata_line: 21,
    create_line: 25,
    handlers: &HANDLERS,
    bad_extension_msg: "Invalid extension ID",
    bad_metadata_msg: "Invalid metadataId",
};

#[no_mangle]
pub unsafe extern "system" fn XtfOrdinal_4(extension: *const GUID, iid: *const GUID, ppv: *mut Obj) -> HRESULT {
    let hr = extension_entry_point(&SITE, &REMOTE_RUN_EXTENSION_ID, extension, iid, ppv);
    if hr >= 0 {
        title::ensure();
    }
    hr
}
