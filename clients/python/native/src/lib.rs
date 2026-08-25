//! C ABI surface for the Python ctypes adapter.
//!
//! The shared FFI remains exported for package and compatibility tooling.  The
//! maintained facade uses the private Gate 0 entry points below so its fixed
//! development profile cannot be replaced by the shared FFI defaults.

pub use openkache_client_core::ffi::*;

use std::net::SocketAddr;
use std::ptr;
use std::slice;
use std::str;
use std::sync::{Mutex, MutexGuard};

use openkache_client_core::contract::{
    CLIENT_GATE0_ALPN_VERSION, CLIENT_GATE0_COMPRESSION, CLIENT_GATE0_ENCRYPTION,
    CLIENT_GATE0_ITEM_ID_ROOT, CLIENT_GATE0_NAMESPACE_ID, CLIENT_GATE0_VALUE_SELECTOR,
    FFI_RESULT_CONNECTED, FFI_RESULT_CREATED, FFI_RESULT_DELETED, FFI_RESULT_ERROR,
    FFI_RESULT_NOT_DELETED, FFI_RESULT_NOT_FOUND, FFI_RESULT_NOT_STORED, FFI_RESULT_REPLACED,
    FFI_RESULT_UNKNOWN_MUTATION, FFI_RESULT_VALUE, VALUE_FORMAT_COMPRESSION_NONE,
    VALUE_FORMAT_ENCRYPTION_NONE, VALUE_FORMAT_ENCRYPTION_SHIFT,
    VALUE_FORMAT_SERIALIZATION_STRUCTURED,
};
use openkache_client_core::value::{Compression, Encryption};
use openkache_client_core::{
    AlpnPolicy, ClientRootKey, DeleteOutcome, Endpoint, Error, GetOutcome, ProtectedClient,
    ServerTrust, SetOptions, SetOutcome,
};
use tokio::runtime::Runtime;

/// One long-lived Gate 0 client.
///
/// The mutex serializes access to the current-thread Tokio runtime and the
/// underlying core client.  ctypes releases Python's GIL around foreign
/// calls, so relying on the GIL here would allow concurrent callers to alias
/// the raw handle and enter `Runtime::block_on` at the same time.
pub struct Gate0Client {
    inner: Mutex<Gate0ClientInner>,
}

struct Gate0ClientInner {
    runtime: Runtime,
    client: ProtectedClient,
}

/// Owned result returned by the private Python ABI.
pub struct Gate0Result {
    kind: u32,
    payload: Vec<u8>,
    client: *mut Gate0Client,
}

impl Gate0Result {
    fn success(kind: u32, payload: impl Into<Vec<u8>>) -> Self {
        Self {
            kind,
            payload: payload.into(),
            client: ptr::null_mut(),
        }
    }

    fn error(error: impl Into<String>) -> Self {
        Self::success(FFI_RESULT_ERROR, error.into().into_bytes())
    }

    fn core_error(error: Error) -> Self {
        if matches!(error, Error::AmbiguousOutcome { .. }) {
            Self::error_with_kind(FFI_RESULT_UNKNOWN_MUTATION, error.to_string())
        } else {
            Self::error(error.to_string())
        }
    }

    fn error_with_kind(kind: u32, message: impl Into<String>) -> Self {
        Self::success(kind, message.into().into_bytes())
    }
}

unsafe fn copy_bytes(pointer: *const u8, length: usize, name: &str) -> Result<Vec<u8>, String> {
    if length == 0 {
        return Ok(Vec::new());
    }
    if pointer.is_null() {
        return Err(format!(
            "{name} pointer must not be null for a non-empty buffer"
        ));
    }
    Ok(unsafe { slice::from_raw_parts(pointer, length) }.to_vec())
}

unsafe fn copy_utf8(pointer: *const u8, length: usize, name: &str) -> Result<String, String> {
    let bytes = unsafe { copy_bytes(pointer, length, name)? };
    str::from_utf8(&bytes)
        .map(str::to_owned)
        .map_err(|error| format!("{name} must be valid UTF-8: {error}"))
}

fn boxed_result(result: Gate0Result) -> *mut Gate0Result {
    Box::into_raw(Box::new(result))
}

fn connect_gate0(address: String) -> Gate0Result {
    validate_gate0_selector();
    let address = match address.parse::<SocketAddr>() {
        Ok(address) => address,
        Err(error) => return Gate0Result::error(format!("invalid server address: {error}")),
    };
    let endpoint = match Endpoint::from_socket_addr(address, "localhost") {
        Ok(endpoint) => endpoint,
        Err(error) => return Gate0Result::core_error(error),
    };
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            return Gate0Result::error(format!("failed to create client runtime: {error}"));
        }
    };
    // The core's default ALPN policy is the sole `openkache/1` protocol and
    // its QUIC transport uses TLS 1.3 with the required hybrid group.
    let client = runtime.block_on(
        ProtectedClient::builder(
            endpoint,
            ClientRootKey::from_bytes(CLIENT_GATE0_ITEM_ID_ROOT),
        )
        .alpn_policy(
            AlpnPolicy::from_versions(vec![CLIENT_GATE0_ALPN_VERSION], CLIENT_GATE0_ALPN_VERSION)
                .expect("the generated Gate 0 ALPN profile must be valid"),
        )
        .server_trust(ServerTrust::Insecure)
        .compression(gate0_compression())
        .encryption(gate0_encryption())
        .connect(),
    );
    match client {
        Ok(client) => {
            let gate0 = Box::new(Gate0Client {
                inner: Mutex::new(Gate0ClientInner { runtime, client }),
            });
            let mut result = Gate0Result::success(FFI_RESULT_CONNECTED, []);
            result.client = Box::into_raw(gate0);
            result
        }
        Err(error) => Gate0Result::core_error(error),
    }
}

unsafe fn as_client<'a>(pointer: *mut Gate0Client) -> Result<&'a Gate0Client, Gate0Result> {
    if pointer.is_null() {
        return Err(Gate0Result::error("client pointer must not be null"));
    }
    Ok(unsafe { &*pointer })
}

fn lock_client<'a>(
    client: &'a Gate0Client,
) -> Result<MutexGuard<'a, Gate0ClientInner>, Gate0Result> {
    client
        .inner
        .lock()
        .map_err(|_| Gate0Result::error("native client mutex is poisoned"))
}

fn ensure_gate0_namespace(client: &Gate0ClientInner) -> Result<(), Gate0Result> {
    let namespace_id = match client
        .runtime
        .block_on(client.client.raw().ensure_namespace_id())
    {
        Ok(namespace_id) => namespace_id,
        Err(error) => return Err(Gate0Result::core_error(error)),
    };
    if namespace_id != CLIENT_GATE0_NAMESPACE_ID {
        return Err(Gate0Result::error(format!(
            "server selected namespace {namespace_id}, expected Gate 0 namespace {CLIENT_GATE0_NAMESPACE_ID}"
        )));
    }
    Ok(())
}

unsafe fn run_get(client: *mut Gate0Client, key: *const u8, key_length: usize) -> Gate0Result {
    let key = match unsafe { copy_bytes(key, key_length, "key") } {
        Ok(key) => key,
        Err(error) => return Gate0Result::error(error),
    };
    let client = match unsafe { as_client(client) } {
        Ok(client) => client,
        Err(error) => return error,
    };
    let client = match lock_client(client) {
        Ok(client) => client,
        Err(error) => return error,
    };
    if let Err(error) = ensure_gate0_namespace(&client) {
        return error;
    }
    match client
        .runtime
        .block_on(client.client.get_structured_canonical_key_cbor(key))
    {
        Ok(GetOutcome::NotFound) => Gate0Result::success(FFI_RESULT_NOT_FOUND, []),
        Ok(GetOutcome::Found(value)) => Gate0Result::success(FFI_RESULT_VALUE, value),
        Err(error) => Gate0Result::core_error(error),
    }
}

unsafe fn run_set(
    client: *mut Gate0Client,
    key: *const u8,
    key_length: usize,
    value: *const u8,
    value_length: usize,
) -> Gate0Result {
    let key = match unsafe { copy_bytes(key, key_length, "key") } {
        Ok(key) => key,
        Err(error) => return Gate0Result::error(error),
    };
    let value = match unsafe { copy_bytes(value, value_length, "value") } {
        Ok(value) => value,
        Err(error) => return Gate0Result::error(error),
    };
    let client = match unsafe { as_client(client) } {
        Ok(client) => client,
        Err(error) => return error,
    };
    let client = match lock_client(client) {
        Ok(client) => client,
        Err(error) => return error,
    };
    if let Err(error) = ensure_gate0_namespace(&client) {
        return error;
    }
    match client
        .runtime
        .block_on(
            client
                .client
                .set_structured_canonical_key_cbor(key, value, SetOptions::new()),
        ) {
        Ok(SetOutcome::Created) => Gate0Result::success(FFI_RESULT_CREATED, []),
        Ok(SetOutcome::Replaced) => Gate0Result::success(FFI_RESULT_REPLACED, []),
        Ok(SetOutcome::NotStored) => Gate0Result::success(FFI_RESULT_NOT_STORED, []),
        Err(error) => Gate0Result::core_error(error),
    }
}

unsafe fn run_delete(client: *mut Gate0Client, key: *const u8, key_length: usize) -> Gate0Result {
    let key = match unsafe { copy_bytes(key, key_length, "key") } {
        Ok(key) => key,
        Err(error) => return Gate0Result::error(error),
    };
    let client = match unsafe { as_client(client) } {
        Ok(client) => client,
        Err(error) => return error,
    };
    let client = match lock_client(client) {
        Ok(client) => client,
        Err(error) => return error,
    };
    if let Err(error) = ensure_gate0_namespace(&client) {
        return error;
    }
    match client
        .runtime
        .block_on(client.client.delete_canonical_key(key))
    {
        Ok(DeleteOutcome::Deleted) => Gate0Result::success(FFI_RESULT_DELETED, []),
        Ok(DeleteOutcome::NotFound) => Gate0Result::success(FFI_RESULT_NOT_DELETED, []),
        Err(error) => Gate0Result::core_error(error),
    }
}

fn gate0_compression() -> Compression {
    match CLIENT_GATE0_COMPRESSION {
        VALUE_FORMAT_COMPRESSION_NONE => Compression::Disabled,
        selector => panic!("generated Gate 0 compression selector {selector} is unsupported"),
    }
}

fn gate0_encryption() -> Encryption {
    match CLIENT_GATE0_ENCRYPTION {
        VALUE_FORMAT_ENCRYPTION_NONE => Encryption::Unprotected,
        selector => panic!("generated Gate 0 encryption selector {selector} is unsupported"),
    }
}

fn validate_gate0_selector() {
    let expected = CLIENT_GATE0_ENCRYPTION
        | (CLIENT_GATE0_COMPRESSION << 2)
        | (VALUE_FORMAT_SERIALIZATION_STRUCTURED << VALUE_FORMAT_ENCRYPTION_SHIFT);
    assert_eq!(
        CLIENT_GATE0_VALUE_SELECTOR, expected,
        "generated Gate 0 value selector does not match its profile fields",
    );
}

/// Opens the fixed Gate 0 development profile.
///
/// The endpoint is the already-resolved `host:port` authority supplied by the
/// Python facade.  Trust, ALPN, namespace, key mapping, compression, and value
/// protection are deliberately not represented in this ABI.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_python_connect(
    address: *const u8,
    address_length: usize,
) -> *mut Gate0Result {
    let address = match unsafe { copy_utf8(address, address_length, "address") } {
        Ok(address) => address,
        Err(error) => return boxed_result(Gate0Result::error(error)),
    };
    boxed_result(connect_gate0(address))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_python_get(
    client: *mut Gate0Client,
    key: *const u8,
    key_length: usize,
) -> *mut Gate0Result {
    boxed_result(unsafe { run_get(client, key, key_length) })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_python_set(
    client: *mut Gate0Client,
    key: *const u8,
    key_length: usize,
    value: *const u8,
    value_length: usize,
) -> *mut Gate0Result {
    boxed_result(unsafe { run_set(client, key, key_length, value, value_length) })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_python_delete(
    client: *mut Gate0Client,
    key: *const u8,
    key_length: usize,
) -> *mut Gate0Result {
    boxed_result(unsafe { run_delete(client, key, key_length) })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_python_result_kind(result: *const Gate0Result) -> u32 {
    if result.is_null() {
        return FFI_RESULT_ERROR;
    }
    unsafe { (*result).kind }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_python_result_data(result: *const Gate0Result) -> *const u8 {
    if result.is_null() {
        return ptr::null();
    }
    let payload = unsafe { &(*result).payload };
    if payload.is_empty() {
        ptr::null()
    } else {
        payload.as_ptr()
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_python_result_data_length(result: *const Gate0Result) -> usize {
    if result.is_null() {
        return 0;
    }
    unsafe { (*result).payload.len() }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_python_result_take_client(
    result: *mut Gate0Result,
) -> *mut Gate0Client {
    if result.is_null() {
        return ptr::null_mut();
    }
    let result = unsafe { &mut *result };
    std::mem::replace(&mut result.client, ptr::null_mut())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_python_result_free(result: *mut Gate0Result) {
    if !result.is_null() {
        drop(unsafe { Box::from_raw(result) });
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_python_client_free(client: *mut Gate0Client) {
    if !client.is_null() {
        drop(unsafe { Box::from_raw(client) });
    }
}
