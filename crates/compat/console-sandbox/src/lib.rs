use std::path::PathBuf;

use compat_common::os_host::patch::{patch_image, Hook};

pub mod claims;
pub mod fs;
pub mod gcdcom;
pub mod gchost;
pub mod gcreply;
pub mod hive;
pub mod logan;
pub mod vab;
pub mod wasapi;
pub mod crossvm;
pub mod modules;
pub mod power;
pub mod regnt;
pub mod rpc;
pub mod wnf;
pub mod host;
pub mod notify;
pub mod registry;
pub mod regw;
pub mod system;
pub mod wstr;
pub mod xvcmgmt;
pub mod xvcsys;

#[link(name = "ntdll")]
extern "system" {
    fn NtClose(h: usize) -> i32;
}

pub fn root() -> PathBuf {
    compat_common::gdk::data_root().join("console")
}

unsafe extern "system" fn nt_close(h: usize) -> i32 {
    if registry::close(h) {
        0
    } else {
        xvcmgmt::before_close(h);
        NtClose(h)
    }
}

const NTDLL: &[&str] = &["ntdll"];
const REG: &[&str] = &["api-ms-win-core-registry-", "advapi32", "kernelbase", "kernel32"];
const FILE: &[&str] = &["api-ms-win-core-file-", "kernelbase", "kernel32"];
const ENVIRON: &[&str] = &["api-ms-win-core-processenvironment-", "kernelbase", "kernel32"];
const SYSINFO: &[&str] = &["api-ms-win-core-sysinfo-", "kernelbase", "kernel32"];
const THREADS: &[&str] = &["api-ms-win-core-processthreads-", "kernelbase", "kernel32"];
const SERVICE: &[&str] = &["api-ms-win-service-", "advapi32", "sechost"];
const RPC: &[&str] = &["rpcrt4"];
const POWER: &[&str] = &["api-ms-win-power-setting-", "powrprof", "umpdc"];

macro_rules! hook {
    ($name:literal, $dlls:expr, $f:expr) => {
        Hook { name: $name, dlls: $dlls, hook: $f as *const () as usize }
    };
}

pub fn hooks() -> Vec<Hook> {
    vec![
        hook!("NtCreateKey", NTDLL, registry::nt_create_key),
        hook!("NtOpenKey", NTDLL, registry::nt_open_key),
        hook!("NtSetValueKey", NTDLL, registry::nt_set_value_key),
        hook!("NtFlushKey", NTDLL, registry::nt_flush_key),
        hook!("NtClose", NTDLL, nt_close),
        hook!("ZwCreateKey", NTDLL, registry::nt_create_key),
        hook!("ZwOpenKey", NTDLL, registry::nt_open_key),
        hook!("ZwSetValueKey", NTDLL, registry::nt_set_value_key),
        hook!("ZwClose", NTDLL, nt_close),
        hook!("NtQueryValueKey", NTDLL, regnt::nt_query_value_key),
        hook!("ZwQueryValueKey", NTDLL, regnt::nt_query_value_key),
        hook!("NtEnumerateKey", NTDLL, regnt::nt_enumerate_key),
        hook!("ZwEnumerateKey", NTDLL, regnt::nt_enumerate_key),
        hook!("NtEnumerateValueKey", NTDLL, regnt::nt_enumerate_value_key),
        hook!("ZwEnumerateValueKey", NTDLL, regnt::nt_enumerate_value_key),
        hook!("NtQueryKey", NTDLL, regnt::nt_query_key),
        hook!("ZwQueryKey", NTDLL, regnt::nt_query_key),
        hook!("NtDeleteKey", NTDLL, regnt::nt_delete_key),
        hook!("ZwDeleteKey", NTDLL, regnt::nt_delete_key),
        hook!("NtDeleteValueKey", NTDLL, regnt::nt_delete_value_key),
        hook!("ZwDeleteValueKey", NTDLL, regnt::nt_delete_value_key),
        hook!("ZwFlushKey", NTDLL, registry::nt_flush_key),
        hook!("NtOpenFile", NTDLL, xvcmgmt::nt_open_file),
        hook!("ZwOpenFile", NTDLL, xvcmgmt::nt_open_file),
        hook!("NtCreateFile", NTDLL, xvcmgmt::nt_create_file),
        hook!("ZwCreateFile", NTDLL, xvcmgmt::nt_create_file),
        hook!("NtDeviceIoControlFile", NTDLL, xvcmgmt::nt_device_io_control_file),
        hook!("ZwDeviceIoControlFile", NTDLL, xvcmgmt::nt_device_io_control_file),
        hook!("NtCancelIoFile", NTDLL, xvcmgmt::nt_cancel_io_file),
        hook!("ZwCancelIoFile", NTDLL, xvcmgmt::nt_cancel_io_file),
        hook!("NtCancelIoFileEx", NTDLL, xvcmgmt::nt_cancel_io_file_ex),
        hook!("ZwCancelIoFileEx", NTDLL, xvcmgmt::nt_cancel_io_file_ex),
        hook!("NtQueryInformationToken", NTDLL, claims::nt_query_information_token),
        hook!("ZwQueryInformationToken", NTDLL, claims::nt_query_information_token),
        hook!("NtCreateWnfStateName", NTDLL, wnf::nt_create_wnf_state_name),
        hook!("ZwCreateWnfStateName", NTDLL, wnf::nt_create_wnf_state_name),
        hook!("NtDeleteWnfStateName", NTDLL, wnf::nt_delete_wnf_state_name),
        hook!("ZwDeleteWnfStateName", NTDLL, wnf::nt_delete_wnf_state_name),
        hook!("NtUpdateWnfStateData", NTDLL, wnf::nt_update_wnf_state_data),
        hook!("ZwUpdateWnfStateData", NTDLL, wnf::nt_update_wnf_state_data),
        hook!("NtQueryWnfStateData", NTDLL, wnf::nt_query_wnf_state_data),
        hook!("ZwQueryWnfStateData", NTDLL, wnf::nt_query_wnf_state_data),
        hook!("NtQueryWnfStateNameInformation", NTDLL, wnf::nt_query_wnf_state_name_information),
        hook!("ZwQueryWnfStateNameInformation", NTDLL, wnf::nt_query_wnf_state_name_information),
        hook!("RtlPublishWnfStateData", NTDLL, wnf::rtl_publish_wnf_state_data),
        hook!("RtlSubscribeWnfStateChangeNotification", NTDLL, wnf::rtl_subscribe_wnf_state_change_notification),
        hook!("RtlUnsubscribeWnfStateChangeNotification", NTDLL, wnf::rtl_unsubscribe_wnf_state_change_notification),
        hook!("RtlUnsubscribeWnfNotificationWaitForCompletion", NTDLL, wnf::rtl_unsubscribe_wnf_notification_wait_for_completion),
        hook!("RtlAllocateWnfSerializationGroup", NTDLL, wnf::rtl_allocate_wnf_serialization_group),
        hook!("RtlWaitForWnfMetaNotification", NTDLL, wnf::rtl_wait_for_wnf_meta_notification),
        hook!("RpcServerUseProtseqW", RPC, rpc::rpc_server_use_protseq_w),
        hook!("RpcServerUseProtseqEpW", RPC, rpc::rpc_server_use_protseq_ep_w),
        hook!("RpcEpRegisterW", RPC, rpc::rpc_ep_register_w),
        hook!("RpcEpRegisterNoReplaceW", RPC, rpc::rpc_ep_register_no_replace_w),
        hook!("RpcEpUnregister", RPC, rpc::rpc_ep_unregister),
        hook!("RpcBindingCreateW", RPC, rpc::rpc_binding_create_w),
        hook!("RpcBindingBind", RPC, rpc::rpc_binding_bind),
        hook!("RpcBindingFree", RPC, rpc::rpc_binding_free),
        hook!("RpcStringBindingComposeW", RPC, rpc::rpc_string_binding_compose_w),
        hook!("RpcBindingFromStringBindingW", RPC, rpc::rpc_binding_from_string_binding_w),
        hook!("PowerSettingRegisterNotification", POWER, power::power_setting_register_notification),
        hook!("PowerSettingRegisterNotificationEx", POWER, power::power_setting_register_notification_ex),
        hook!("PowerSettingUnregisterNotification", POWER, power::power_setting_unregister_notification),
        hook!("RegOpenKeyExW", REG, registry::reg_open_key_ex_w),
        hook!("RegCreateKeyExW", REG, registry::reg_create_key_ex_w),
        hook!("RegCloseKey", REG, registry::reg_close_key),
        hook!("RegSetValueExW", REG, registry::reg_set_value_ex_w),
        hook!("RegGetValueW", REG, registry::reg_get_value_w),
        hook!("RegQueryInfoKeyW", REG, registry::reg_query_info_key_w),
        hook!("RegEnumValueW", REG, registry::reg_enum_value_w),
        hook!("RegQueryValueExW", REG, regw::reg_query_value_ex_w),
        hook!("RegEnumKeyExW", REG, regw::reg_enum_key_ex_w),
        hook!("RegEnumKeyW", REG, regw::reg_enum_key_w),
        hook!("RegDeleteValueW", REG, regw::reg_delete_value_w),
        hook!("RegDeleteKeyW", REG, regw::reg_delete_key_w),
        hook!("CreateFileW", FILE, fs::create_file_w),
        hook!("CloseHandle", &["api-ms-win-core-handle-", "kernelbase", "kernel32"], xvcmgmt::close_handle),
        hook!("CancelIo", &["api-ms-win-core-io-", "kernelbase", "kernel32"], xvcmgmt::cancel_io),
        hook!("CancelIoEx", &["api-ms-win-core-io-", "kernelbase", "kernel32"], xvcmgmt::cancel_io_ex),
        hook!("DeviceIoControl", &["api-ms-win-core-io-", "kernelbase", "kernel32"], xvcmgmt::device_io_control),
        hook!("OpenEventW", &["api-ms-win-core-synch-", "kernelbase", "kernel32"], vab::open_event_w),
        hook!("CoCreateInstanceEx", &["api-ms-win-core-com-", "combase", "ole32"], gcdcom::co_create_instance_ex),
        hook!("GetFileAttributesW", FILE, fs::get_file_attributes_w),
        hook!("DeleteFileW", FILE, fs::delete_file_w),
        hook!("SetFileInformationByHandle", FILE, fs::set_file_information_by_handle),
        hook!("GetDriveTypeW", FILE, fs::get_drive_type_w),
        hook!("SearchPathW", ENVIRON, fs::search_path_w),
        hook!("SetEnvironmentVariableW", ENVIRON, system::set_environment_variable_w),
        hook!("GetSystemWindowsDirectoryW", SYSINFO, fs::get_system_windows_directory_w),
        hook!("GetComputerNameExW", SYSINFO, system::get_computer_name_ex_w),
        hook!("CreateProcessW", THREADS, system::create_process_w),
        hook!("SetPriorityClass", THREADS, system::set_priority_class),
        hook!("RegisterServiceCtrlHandlerExW", SERVICE, host::register_service_ctrl_handler_ex_w),
        hook!("SetServiceStatus", SERVICE, host::set_service_status),
        hook!("OpenSCManagerW", SERVICE, host::open_sc_manager_w),
        hook!("OpenServiceW", SERVICE, host::open_service_w),
        hook!("OpenSCManagerA", SERVICE, notify::open_sc_manager_a),
        hook!("OpenServiceA", SERVICE, notify::open_service_a),
        hook!("NotifyServiceStatusChangeA", SERVICE, notify::notify_service_status_change),
        hook!("NotifyServiceStatusChangeW", SERVICE, notify::notify_service_status_change),
        hook!("CloseServiceHandle", SERVICE, host::close_service_handle),
        hook!("QueryServiceStatusEx", SERVICE, host::query_service_status_ex),
        hook!("QueryServiceConfigW", SERVICE, host::query_service_config_w),
    ]
}

pub unsafe fn attach(module: usize) -> Vec<String> {
    fs::ensure_system_drive();
    let mut hits = Vec::new();
    patch_image(module as *mut u8, &hooks(), &mut hits);
    hits.extend(modules::attach_tree(module));
    hits
}

pub const PASSTHROUGH: &[&str] = &[
    "wcscpy_s", "_vsnwprintf_s", "memcpy", "memset", "__C_specific_handler", "_initterm", "_initterm_e",
    "RtlInitUnicodeString", "DbgPrintEx", "DbgPrint", "RtlGetLastNtStatus", "RtlFreeHeap", "RtlAllocateHeap", "NtQueryInformationProcess", "RtlCaptureContext", "RtlLookupFunctionEntry", "RtlVirtualUnwind", "RtlRandomEx",
    "WaitForSingleObject", "CreateEventW", "SetEvent", "GetLastError", "UnhandledExceptionFilter", "SetUnhandledExceptionFilter", "CloseHandle",
    "WriteFile", "ReadFile", "CompareStringOrdinal", "GetLocalTime", "GetTickCount", "GetSystemTimeAsFileTime", "HeapSetInformation", "HeapFree", "GetProcessHeap",
    "DecodePointer", "EncodePointer", "QueryPerformanceCounter", "UnregisterWaitEx", "TerminateProcess", "GetCurrentThread", "GetCurrentThreadId", "GetCurrentProcessId", "GetCurrentProcess", "SetThreadPriority",
    "GetPersistedRegistryLocationW", "SleepEx", "Sleep",
];

pub unsafe fn imports(module: usize) -> Vec<(String, String)> {
    let base = module as *const u8;
    let rd = |o: usize| (base.add(o) as *const u32).read_unaligned() as usize;
    let cstr = |o: usize| {
        let mut v = Vec::new();
        while *base.add(o + v.len()) != 0 {
            v.push(*base.add(o + v.len()));
        }
        String::from_utf8_lossy(&v).into_owned()
    };
    let pe = rd(0x3c);
    let imp = rd(pe + 24 + 112 + 8);
    let mut out = Vec::new();
    let mut d = imp;
    while imp != 0 && (rd(d + 12) != 0 || rd(d + 16) != 0) {
        let dll = cstr(rd(d + 12));
        let lookup = if rd(d) != 0 { rd(d) } else { rd(d + 16) };
        let mut i = 0;
        loop {
            let t = (base.add(lookup + i * 8) as *const u64).read_unaligned();
            if t == 0 {
                break;
            }
            out.push((dll.clone(), if t >> 63 == 1 { format!("#{}", t & 0xffff) } else { cstr(t as usize + 2) }));
            i += 1;
        }
        d += 20;
    }
    let delay = rd(pe + 24 + 112 + 13 * 8);
    let mut d = delay;
    while delay != 0 && rd(d + 4) != 0 {
        let dll = cstr(rd(d + 4));
        let lookup = rd(d + 16);
        let mut i = 0;
        while lookup != 0 {
            let t = (base.add(lookup + i * 8) as *const u64).read_unaligned();
            if t == 0 {
                break;
            }
            out.push((dll.clone(), if t >> 63 == 1 { format!("#{}", t & 0xffff) } else { cstr(t as usize + 2) }));
            i += 1;
        }
        d += 32;
    }
    out
}

pub unsafe fn unrouted(module: usize) -> Vec<String> {
    let hooked: Vec<&str> = hooks().iter().map(|h| h.name).collect();
    imports(module).into_iter().filter(|(_, n)| !hooked.contains(&n.as_str()) && !PASSTHROUGH.contains(&n.as_str())).map(|(d, n)| format!("{d}!{n}")).collect()
}
#[cfg(test)]
mod tests;
