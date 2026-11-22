#![allow(non_snake_case, clippy::missing_safety_doc)]

pub mod chk;
pub mod op;
pub mod config;
pub mod privilege;
pub mod system;
pub mod process;
pub mod screenshot;
pub mod client;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod client_tests;

use xtfmsg::com::metadata::OperationFn;
use xtfmsg::com::{metadata, Com, Obj, IID_XBTP_EXTENSION_METADATA};
use xtfmsg::sys::*;
use xtfmsg::xbtp;

pub const SERVER_CPP: &str = "xbox\\devex\\xtf\\dll\\console\\server.cpp";
pub const ENTRY_FUNC: &str = "XbtpServerEntryPoint";

pub const CONSOLE_EXTENSION_ID: GUID = GUID::new(0x5b45d0e0, 0x858b, 0x49fd, [0xa3, 0x32, 0x7b, 0xdf, 0x8d, 0x25, 0x82, 0x44]);

pub const OP_SHUTDOWN: u32 = 1;
pub const OP_GET_CONFIG_VALUE: u32 = 2;
pub const OP_SET_CONFIG_VALUE: u32 = 3;
pub const OP_UPDATES_PENDING: u32 = 4;
pub const OP_GET_SYSTEM_TIME: u32 = 5;
pub const OP_GET_RUNNING_PROCESSES: u32 = 6;
pub const OP_GET_ALL_SETTINGS: u32 = 7;
pub const OP_GET_SETTINGS_METADATA: u32 = 8;
pub const OP_MARK_SCREENSHOT_TAKEN: u32 = 9;
pub const OP_IS_VIDEO_IMPACTED_BY_SCREENSHOTS: u32 = 10;
pub const OP_GET_SETTINGS_EX: u32 = 11;
pub const OP_NOTIFY_CRASH_DUMP_START: u32 = 14;
pub const OP_NOTIFY_CRASH_DUMP_FINISH: u32 = 15;
pub const OP_GENERATE_STACK_DUMP: u32 = 16;
pub const OP_IN_CONNECTED_STANDBY: u32 = 17;

pub static HANDLERS: [(u32, OperationFn, &str, u32); 13] = [
    (OP_GET_CONFIG_VALUE, config::get_config_value_handler, "metadata->AddFunction(ConsoleGetConfigValueOperationId, GetConfigValueHandler)", 28),
    (OP_SET_CONFIG_VALUE, config::set_config_value_handler, "metadata->AddFunction(ConsoleSetConfigValueOperationId, SetConfigValueHandler)", 29),
    (OP_GET_ALL_SETTINGS, config::get_all_settings_handler, "metadata->AddFunction(ConsoleGetAllConfigSettingsOperationId, GetAllSettingsHandler)", 30),
    (OP_GET_SETTINGS_METADATA, config::get_settings_metadata_handler, "metadata->AddFunction(ConsoleGetSettingsMetadataOperationId, GetSettingsMetadataHandler)", 31),
    (OP_SHUTDOWN, system::shutdown_handler, "metadata->AddFunction(ConsoleShutdownOperationId, ShutdownHandler)", 32),
    (OP_GET_RUNNING_PROCESSES, process::get_running_processes_handler, "metadata->AddFunction(ConsoleGetRunningProcessesOperationId, GetRunningProcessesHandler)", 33),
    (OP_GET_SYSTEM_TIME, system::get_system_time_handler, "metadata->AddFunction(ConsoleGetSystemTimeOperationId, GetSystemTimeHandler)", 34),
    (OP_UPDATES_PENDING, system::updates_pending_handler, "metadata->AddFunction(ConsoleUpdatesPendingOperationId, UpdatesPendingHandler)", 35),
    (OP_MARK_SCREENSHOT_TAKEN, screenshot::mark_screenshot_taken_handler, "metadata->AddFunction(ConsoleMarkScreenshotTakenOperationId, MarkScreenshotTakenHandler)", 36),
    (OP_IS_VIDEO_IMPACTED_BY_SCREENSHOTS, screenshot::is_video_impacted_by_screenshots_handler, "metadata->AddFunction(ConsoleIsVideoImpactedByScreenshotsOperationId, IsVideoImpactedByScreenshotsHandler)", 37),
    (OP_GET_SETTINGS_EX, config::get_settings_ex_handler, "metadata->AddFunction(ConsoleGetSettingsExOperationId, GetSettingsExHandler)", 38),
    (OP_IN_CONNECTED_STANDBY, system::in_connected_standby_handler, "metadata->AddFunction(ConsoleInConnectedStandbyOperationId, InConnectedStandbyHandler)", 39),
    (OP_GENERATE_STACK_DUMP, process::generate_stack_dump_handler, "metadata->AddFunction(GenerateStackDumpOperationId, GenerateStackDump)", 51),
];

fn entry_chk(line: u32, name: &str, hr: HRESULT) {
    chk::chk(SERVER_CPP, ENTRY_FUNC, line, name, hr, "");
}

#[no_mangle]
pub unsafe extern "system" fn XtfOrdinal_4(extension: *const GUID, iid: *const GUID, ppv: *mut Obj) -> HRESULT {
    if *extension != CONSOLE_EXTENSION_ID {
        entry_chk(17, "E_INVALIDARG", E_INVALIDARG);
        return E_INVALIDARG;
    }
    if *iid != IID_XBTP_EXTENSION_METADATA {
        entry_chk(21, "E_INVALIDARG", E_INVALIDARG);
        return E_INVALIDARG;
    }
    let mut md = Com::null();
    let hr = xbtp::create_extension_metadata(&IID_XBTP_EXTENSION_METADATA, md.put());
    entry_chk(25, "XbtpCreateExtensionMetadata(IID_PPV_ARGS(&metadata))", hr);
    if hr < 0 {
        return hr;
    }
    for (id, f, text, line) in HANDLERS.iter() {
        let hr = metadata::add_function(md.get(), *id, Some(*f));
        entry_chk(*line, text, hr);
        if hr < 0 {
            return hr;
        }
    }
    *ppv = md.detach();
    S_OK
}
