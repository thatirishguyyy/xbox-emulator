#![allow(non_snake_case, clippy::missing_safety_doc)]

pub mod client;
pub mod client_copy;
pub mod com;
pub mod file;
pub mod find;
pub mod handlers;
pub mod msg;
pub mod paths;
pub mod transfer;

#[cfg(fileio_b)]
pub mod fileio_b;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod client_tests;

#[cfg(test)]
mod client_copy_tests;

use com::*;
use core::ffi::c_void;

pub fn operation_table() -> Vec<(u32, OperationFn)> {
    let mut v: Vec<(u32, OperationFn)> = vec![
        (6, handlers::create_directory_handler),
        (5, handlers::delete_file_handler),
        (2, handlers::get_file_attributes_handler),
        (1, handlers::get_file_size_handler),
        (4, handlers::modify_file_attributes_handler),
        (7, handlers::remove_directory_handler),
        (3, handlers::set_file_attributes_handler),
        (0xa, transfer::send_file_handler),
        (0xb, transfer::recv_file_handler),
        (9, find::find_files_handler),
        (0xc, handlers::get_file_info_handler),
        (0xd, handlers::application_operations_start_handler),
        (0xe, handlers::application_operations_end_handler),
        (0xf, handlers::package_mount_hold_handler),
        (0x10, handlers::get_storage_device_free_space_handler),
    ];
    #[cfg(fileio_b)]
    v.extend_from_slice(&[
        (0x11, fileio_b::stream_operation_handler as OperationFn),
        (0x19, fileio_b::transfer_files_supported),
        (0x1a, fileio_b::calculate_incremental_changes),
        (0x1b, fileio_b::transfer_files_to_console),
        (0x1c, fileio_b::transfer_files_from_console),
        (0x1e, fileio_b::transfer_files_to_console_no_json),
        (0x1f, fileio_b::transfer_files_from_console_no_json),
        (0x1d, fileio_b::get_last_direct_protocol_error),
    ]);
    v
}

static BOOT_MOVES: std::sync::Once = std::sync::Once::new();

#[no_mangle]
pub unsafe extern "C" fn XtfOrdinal_4(extension: *const GUID, iid: *const GUID, ppv: *mut *mut c_void) -> HRESULT {
    BOOT_MOVES.call_once(|| {
        file::apply_pending_moves();
    });
    if extension.is_null() || *extension != FILEIO_EXTENSION_ID {
        return E_INVALIDARG;
    }
    if iid.is_null() || *iid != IID_XBTP_EXTENSION_METADATA_IMPL {
        return E_INVALIDARG;
    }
    let mut md = Com::null();
    let mut hr = XbtpCreateExtensionMetadata(&IID_XBTP_EXTENSION_METADATA_IMPL, md.out());
    if hr < 0 {
        return hr;
    }
    for (id, f) in operation_table() {
        hr = (vt::<MetadataVtbl>(md.0).add_function)(md.0, id, Some(f));
        if hr < 0 {
            return hr;
        }
    }
    (vt::<UnknownVtbl>(md.0).query_interface)(md.0, iid, ppv)
}
