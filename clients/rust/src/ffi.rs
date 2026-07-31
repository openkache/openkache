//! Thin C-ABI symbol layer over [`openkache_client_core::ffi`].
//!
//! Keeping the implementation in the shared core makes this ABI available to
//! Swift, C, and future native bindings without copying transport or
//! protection behavior into each language package.

#![allow(
    clippy::missing_safety_doc,
    clippy::too_many_arguments,
    reason = "Safety contracts are documented once on the shared core and C header."
)]

pub use openkache_client_core::ffi::{FfiClient, FfiResult};

#[unsafe(no_mangle)]
pub extern "C" fn openkache_client_abi_version() -> u32 {
    openkache_client_core::ffi::openkache_client_abi_version()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_client_connect(
    address: *const u8,
    address_length: usize,
    server_name: *const u8,
    server_name_length: usize,
    certificate: *const u8,
    certificate_length: usize,
    data_protection_key: *const u8,
    data_protection_key_length: usize,
    compression_enabled: u8,
    compression_level: i32,
    minimum_input_size: usize,
    minimum_savings: usize,
    connect_timeout_ms: u64,
    request_timeout_ms: u64,
) -> *mut FfiResult {
    unsafe {
        openkache_client_core::ffi::openkache_client_connect(
            address,
            address_length,
            server_name,
            server_name_length,
            certificate,
            certificate_length,
            data_protection_key,
            data_protection_key_length,
            compression_enabled,
            compression_level,
            minimum_input_size,
            minimum_savings,
            connect_timeout_ms,
            request_timeout_ms,
        )
    }
}

#[allow(clippy::too_many_arguments)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_client_connect_ex(
    address: *const u8,
    address_length: usize,
    server_name: *const u8,
    server_name_length: usize,
    certificate: *const u8,
    certificate_length: usize,
    client_certificate: *const u8,
    client_certificate_length: usize,
    client_private_key: *const u8,
    client_private_key_length: usize,
    data_protection_key: *const u8,
    data_protection_key_length: usize,
    compression_enabled: u8,
    compression_level: i32,
    minimum_input_size: usize,
    minimum_savings: usize,
    encryption: u32,
    retry_max_attempts: usize,
    max_in_flight: usize,
    connect_timeout_ms: u64,
    request_timeout_ms: u64,
) -> *mut FfiResult {
    unsafe {
        openkache_client_core::ffi::openkache_client_connect_ex(
            address,
            address_length,
            server_name,
            server_name_length,
            certificate,
            certificate_length,
            client_certificate,
            client_certificate_length,
            client_private_key,
            client_private_key_length,
            data_protection_key,
            data_protection_key_length,
            compression_enabled,
            compression_level,
            minimum_input_size,
            minimum_savings,
            encryption,
            retry_max_attempts,
            max_in_flight,
            connect_timeout_ms,
            request_timeout_ms,
        )
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_client_execute(
    client: *const FfiClient,
    operation: u32,
    application_key: *const u8,
    application_key_length: usize,
    value: *const u8,
    value_length: usize,
    set_condition: u32,
    ttl_enabled: u8,
    ttl_ms: u64,
) -> *mut FfiResult {
    unsafe {
        openkache_client_core::ffi::openkache_client_execute(
            client,
            operation,
            application_key,
            application_key_length,
            value,
            value_length,
            set_condition,
            ttl_enabled,
            ttl_ms,
        )
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_client_execute_raw(
    client: *const FfiClient,
    operation: u32,
    item_id: *const u8,
    item_id_length: usize,
    value: *const u8,
    value_length: usize,
    set_condition: u32,
    ttl_enabled: u8,
    ttl_ms: u64,
) -> *mut FfiResult {
    unsafe {
        openkache_client_core::ffi::openkache_client_execute_raw(
            client,
            operation,
            item_id,
            item_id_length,
            value,
            value_length,
            set_condition,
            ttl_enabled,
            ttl_ms,
        )
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_client_connection_state(client: *const FfiClient) -> u32 {
    unsafe { openkache_client_core::ffi::openkache_client_connection_state(client) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_client_result_kind(result: *const FfiResult) -> u32 {
    unsafe { openkache_client_core::ffi::openkache_client_result_kind(result) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_client_result_data(result: *const FfiResult) -> *const u8 {
    unsafe { openkache_client_core::ffi::openkache_client_result_data(result) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_client_result_data_length(result: *const FfiResult) -> usize {
    unsafe { openkache_client_core::ffi::openkache_client_result_data_length(result) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_client_result_take_client(
    result: *mut FfiResult,
) -> *mut FfiClient {
    unsafe { openkache_client_core::ffi::openkache_client_result_take_client(result) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_client_result_free(result: *mut FfiResult) {
    unsafe { openkache_client_core::ffi::openkache_client_result_free(result) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_client_free(client: *mut FfiClient) {
    unsafe { openkache_client_core::ffi::openkache_client_free(client) }
}
