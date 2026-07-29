//! Stable C ABI used by the TypeScript client.

use std::net::SocketAddr;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{SyncSender, sync_channel};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::value::{Compression, ENCRYPTION_KEY_BYTES, ValueCodec, ZstandardOptions};
use crate::{
    Client, ClientOptions, ClientTimeouts, QuicBackend, QuicOptions, SetCondition, SetOptions,
    SetOutcome,
};

const RESULT_ERROR: u32 = 0;
const RESULT_OK: u32 = 1;
const RESULT_VALUE: u32 = 2;
const RESULT_NOT_FOUND: u32 = 3;
const RESULT_CREATED: u32 = 4;
const RESULT_REPLACED: u32 = 5;
const RESULT_DELETED: u32 = 6;
const RESULT_NOT_DELETED: u32 = 7;
const RESULT_CONNECTED: u32 = 8;
const RESULT_NOT_STORED: u32 = 9;

const OPERATION_PING: u32 = 1;
const OPERATION_GET: u32 = 2;
const OPERATION_SET: u32 = 3;
const OPERATION_DELETE: u32 = 4;
const OPERATION_STATS: u32 = 5;
const OPERATION_SYNC: u32 = 6;

const COMMAND_QUEUE_CAPACITY: usize = 64;

/// Opaque result allocated by the FFI boundary.
pub struct FfiResult {
    kind: u32,
    payload: Vec<u8>,
    client: Option<Box<FfiClient>>,
}

/// Opaque TypeScript-owned handle to a dedicated Rust client worker.
pub struct FfiClient {
    commands: flume::Sender<Command>,
    request_timeout: Duration,
    shutdown: Arc<AtomicBool>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

enum Command {
    Execute {
        operation: u32,
        key: Vec<u8>,
        value: Vec<u8>,
        set_options: SetOptions,
        response: SyncSender<FfiResult>,
    },
    Shutdown,
}

struct WorkerOptions {
    address: SocketAddr,
    server_name: String,
    certificate: Vec<u8>,
    value_codec: ValueCodec,
    timeouts: ClientTimeouts,
}

impl FfiResult {
    fn error(message: impl Into<String>) -> Self {
        Self {
            kind: RESULT_ERROR,
            payload: message.into().into_bytes(),
            client: None,
        }
    }

    fn success(kind: u32, payload: Vec<u8>) -> Self {
        Self {
            kind,
            payload,
            client: None,
        }
    }

    fn connected(client: FfiClient) -> Self {
        Self {
            kind: RESULT_CONNECTED,
            payload: Vec::new(),
            client: Some(Box::new(client)),
        }
    }
}

impl FfiClient {
    fn connect(
        address: SocketAddr,
        server_name: String,
        certificate: Vec<u8>,
        value_codec: ValueCodec,
        timeouts: ClientTimeouts,
    ) -> std::result::Result<Self, String> {
        let (commands, receiver) = flume::bounded(COMMAND_QUEUE_CAPACITY);
        let (ready_sender, ready_receiver) = sync_channel(1);
        let shutdown = Arc::new(AtomicBool::new(false));
        let worker_shutdown = Arc::clone(&shutdown);
        let options = WorkerOptions {
            address,
            server_name,
            certificate,
            value_codec,
            timeouts,
        };
        let worker = thread::Builder::new()
            .name("openkache-client".to_string())
            .spawn(move || {
                run_worker(receiver, ready_sender, options, worker_shutdown);
            })
            .map_err(|error| format!("failed to start client worker: {error}"))?;

        match ready_receiver.recv() {
            Ok(Ok(())) => Ok(Self {
                commands,
                request_timeout: timeouts.request,
                shutdown,
                worker: Mutex::new(Some(worker)),
            }),
            Ok(Err(error)) => {
                let _ = worker.join();
                Err(error)
            }
            Err(error) => {
                let _ = worker.join();
                Err(format!(
                    "client worker stopped before connection completed: {error}"
                ))
            }
        }
    }

    fn execute(
        &self,
        operation: u32,
        key: Vec<u8>,
        value: Vec<u8>,
        set_options: SetOptions,
    ) -> FfiResult {
        let (response, receiver) = sync_channel(1);
        let Some(deadline) = Instant::now().checked_add(self.request_timeout) else {
            return FfiResult::error("client request timeout exceeds the platform clock range");
        };
        if let Err(error) = self.commands.send_deadline(
            Command::Execute {
                operation,
                key,
                value,
                set_options,
                response,
            },
            deadline,
        ) {
            return FfiResult::error(format!("client worker queue deadline exceeded: {error}"));
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        receiver.recv_timeout(remaining).unwrap_or_else(|error| {
            FfiResult::error(format!("client operation timed out: {error}"))
        })
    }
}

impl Drop for FfiClient {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        let _ = self.commands.try_send(Command::Shutdown);
        if let Ok(worker) = self.worker.get_mut()
            && let Some(worker) = worker.take()
        {
            let _ = worker.join();
        }
    }
}

fn run_worker(
    commands: flume::Receiver<Command>,
    ready: SyncSender<std::result::Result<(), String>>,
    options: WorkerOptions,
    shutdown: Arc<AtomicBool>,
) {
    let WorkerOptions {
        address,
        server_name,
        certificate,
        value_codec,
        timeouts,
    } = options;
    let runtime = match compio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(error) => {
            let _ = ready.send(Err(format!("failed to create Compio runtime: {error}")));
            return;
        }
    };
    if !runtime.driver_type().is_iouring() {
        let _ = ready.send(Err(
            "OpenKache client requires the Compio io_uring driver".to_string()
        ));
        return;
    }
    let client = match runtime.block_on(Client::connect_with_options(
        address,
        &server_name,
        &certificate,
        ClientOptions {
            value_codec,
            identity: None,
            quic: QuicOptions {
                backend: Some(QuicBackend::Compio),
            },
            timeouts,
            ..ClientOptions::default()
        },
    )) {
        Ok(client) => client,
        Err(error) => {
            let _ = ready.send(Err(error.to_string()));
            return;
        }
    };
    if ready.send(Ok(())).is_err() {
        return;
    }

    while !shutdown.load(Ordering::Acquire) {
        let Ok(command) = commands.recv() else {
            break;
        };
        match command {
            Command::Execute {
                operation,
                key,
                value,
                set_options,
                response,
            } => {
                let result = runtime.block_on(execute(&client, operation, key, value, set_options));
                let _ = response.send(result);
            }
            Command::Shutdown => break,
        }
    }
}

async fn execute(
    client: &Client,
    operation: u32,
    key: Vec<u8>,
    value: Vec<u8>,
    set_options: SetOptions,
) -> FfiResult {
    let result = match operation {
        OPERATION_PING => client
            .ping()
            .await
            .map(|()| FfiResult::success(RESULT_OK, Vec::new())),
        OPERATION_GET => client.get(&key).await.map(|value| match value {
            Some(value) => FfiResult::success(RESULT_VALUE, value),
            None => FfiResult::success(RESULT_NOT_FOUND, Vec::new()),
        }),
        OPERATION_SET => client
            .set_owned_with_options(&key, value, set_options)
            .await
            .map(|outcome| match outcome {
                SetOutcome::Created => FfiResult::success(RESULT_CREATED, Vec::new()),
                SetOutcome::Replaced => FfiResult::success(RESULT_REPLACED, Vec::new()),
                SetOutcome::NotStored => FfiResult::success(RESULT_NOT_STORED, Vec::new()),
            }),
        OPERATION_DELETE => client.delete(&key).await.map(|deleted| {
            FfiResult::success(
                if deleted {
                    RESULT_DELETED
                } else {
                    RESULT_NOT_DELETED
                },
                Vec::new(),
            )
        }),
        OPERATION_STATS => client
            .stats()
            .await
            .map(|stats| FfiResult::success(RESULT_VALUE, stats.into_bytes())),
        OPERATION_SYNC => client
            .sync()
            .await
            .map(|()| FfiResult::success(RESULT_OK, Vec::new())),
        _ => return FfiResult::error(format!("unsupported operation {operation}")),
    };
    result.unwrap_or_else(|error| FfiResult::error(error.to_string()))
}

/// Returns the native ABI version implemented by this library.
#[unsafe(no_mangle)]
pub extern "C" fn openkache_client_abi_version() -> u32 {
    3
}

/// Connects a native client and returns an opaque result.
///
/// # Safety
///
/// Every non-empty pointer/length pair must identify readable memory for the duration of this
/// call. `encryption_key` must contain either zero bytes or exactly 32 bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_client_connect(
    address: *const u8,
    address_length: usize,
    server_name: *const u8,
    server_name_length: usize,
    certificate: *const u8,
    certificate_length: usize,
    encryption_key: *const u8,
    encryption_key_length: usize,
    compression_enabled: u8,
    compression_level: i32,
    minimum_input_size: usize,
    minimum_savings: usize,
    connect_timeout_ms: u64,
    request_timeout_ms: u64,
) -> *mut FfiResult {
    boxed_result(catch_result(|| {
        let address = copy_utf8(address, address_length, "address")?;
        let address = address
            .parse()
            .map_err(|error| format!("invalid server address: {error}"))?;
        let server_name = copy_utf8(server_name, server_name_length, "server name")?;
        let certificate = copy_bytes(certificate, certificate_length, "certificate")?;
        if certificate.is_empty() {
            return Err("certificate must not be empty".to_string());
        }
        let encryption_key = copy_bytes(encryption_key, encryption_key_length, "encryption key")?;
        let compression = if compression_enabled == 0 {
            Compression::Disabled
        } else {
            Compression::Zstandard(ZstandardOptions {
                level: compression_level,
                minimum_input_size,
                minimum_savings,
            })
        };
        let value_codec = match encryption_key.len() {
            0 => ValueCodec::compressed(compression),
            ENCRYPTION_KEY_BYTES => ValueCodec::encrypted(
                encryption_key
                    .try_into()
                    .expect("validated encryption key length"),
                compression,
            ),
            actual => {
                return Err(format!(
                    "encryption key must contain {ENCRYPTION_KEY_BYTES} bytes, got {actual}"
                ));
            }
        }
        .map_err(|error| error.to_string())?;
        if connect_timeout_ms == 0 || request_timeout_ms == 0 {
            return Err("client timeouts must be greater than zero milliseconds".to_string());
        }
        let timeouts = ClientTimeouts {
            connect: Duration::from_millis(connect_timeout_ms),
            request: Duration::from_millis(request_timeout_ms),
        };
        FfiClient::connect(address, server_name, certificate, value_codec, timeouts)
            .map(FfiResult::connected)
    }))
}

/// Executes one operation through an opaque native client.
///
/// # Safety
///
/// `client` must be a live pointer returned by [`openkache_client_result_take_client`].
/// Every non-empty key/value pointer pair must identify readable memory for this call. The client
/// must not be freed until this call returns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_client_execute(
    client: *const FfiClient,
    operation: u32,
    key: *const u8,
    key_length: usize,
    value: *const u8,
    value_length: usize,
    set_condition: u32,
    ttl_ms: u64,
) -> *mut FfiResult {
    boxed_result(catch_result(|| {
        let client = unsafe {
            client
                .as_ref()
                .ok_or_else(|| "client pointer must not be null".to_string())?
        };
        let key = copy_bytes(key, key_length, "key")?;
        let value = copy_bytes(value, value_length, "value")?;
        let condition = match set_condition {
            0 => SetCondition::None,
            1 => SetCondition::Nx,
            2 => SetCondition::Xx,
            _ => return Err(format!("unsupported SET condition {set_condition}")),
        };
        let set_options = SetOptions::new(condition, (ttl_ms != 0).then_some(ttl_ms));
        match operation {
            OPERATION_GET | OPERATION_SET | OPERATION_DELETE if key.is_empty() => {
                Err("key must not be empty".to_string())
            }
            OPERATION_PING | OPERATION_STATS | OPERATION_SYNC if !key.is_empty() => {
                Err("operation does not accept a key".to_string())
            }
            OPERATION_SET if value.is_empty() => Err("SET value must not be empty".to_string()),
            OPERATION_PING | OPERATION_GET | OPERATION_DELETE | OPERATION_STATS
            | OPERATION_SYNC
                if !value.is_empty() =>
            {
                Err("operation does not accept a value".to_string())
            }
            operation
                if operation != OPERATION_SET
                    && (set_options.condition != SetCondition::None
                        || set_options.ttl_ms.is_some()) =>
            {
                Err("SET options require a SET operation".to_string())
            }
            _ => Ok(client.execute(operation, key, value, set_options)),
        }
    }))
}

/// Returns an FFI result discriminator.
///
/// # Safety
///
/// `result` must be a live pointer returned by this library.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_client_result_kind(result: *const FfiResult) -> u32 {
    unsafe { result.as_ref() }.map_or(RESULT_ERROR, |result| result.kind)
}

/// Returns a borrowed pointer to an FFI result payload.
///
/// # Safety
///
/// `result` must be live, and the returned pointer must not be used after freeing that result.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_client_result_data(result: *const FfiResult) -> *const u8 {
    let Some(result) = (unsafe { result.as_ref() }) else {
        return ptr::null();
    };
    if result.payload.is_empty() {
        ptr::null()
    } else {
        result.payload.as_ptr()
    }
}

/// Returns the byte length of an FFI result payload.
///
/// # Safety
///
/// `result` must be a live pointer returned by this library.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_client_result_data_length(result: *const FfiResult) -> usize {
    unsafe { result.as_ref() }.map_or(0, |result| result.payload.len())
}

/// Moves a connected client handle out of an FFI result.
///
/// # Safety
///
/// `result` must be a unique, live pointer returned by [`openkache_client_connect`]. This function
/// may be called at most once for a connected result.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_client_result_take_client(
    result: *mut FfiResult,
) -> *mut FfiClient {
    let Some(result) = (unsafe { result.as_mut() }) else {
        return ptr::null_mut();
    };
    result.client.take().map_or(ptr::null_mut(), Box::into_raw)
}

/// Frees an FFI result.
///
/// # Safety
///
/// `result` must be null or a live pointer returned by this library, and it may be freed once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_client_result_free(result: *mut FfiResult) {
    if !result.is_null() {
        drop(unsafe { Box::from_raw(result) });
    }
}

/// Closes and frees a native client.
///
/// # Safety
///
/// `client` must be null or a live pointer returned by
/// [`openkache_client_result_take_client`]. No operation may use it concurrently, and it may be
/// freed once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_client_free(client: *mut FfiClient) {
    if !client.is_null() {
        drop(unsafe { Box::from_raw(client) });
    }
}

fn boxed_result(result: FfiResult) -> *mut FfiResult {
    Box::into_raw(Box::new(result))
}

fn catch_result(operation: impl FnOnce() -> std::result::Result<FfiResult, String>) -> FfiResult {
    match catch_unwind(AssertUnwindSafe(operation)) {
        Ok(Ok(result)) => result,
        Ok(Err(error)) => FfiResult::error(error),
        Err(_) => FfiResult::error("native client panicked"),
    }
}

fn copy_utf8(pointer: *const u8, length: usize, name: &str) -> std::result::Result<String, String> {
    let bytes = copy_bytes(pointer, length, name)?;
    String::from_utf8(bytes).map_err(|error| format!("{name} is not valid UTF-8: {error}"))
}

fn copy_bytes(
    pointer: *const u8,
    length: usize,
    name: &str,
) -> std::result::Result<Vec<u8>, String> {
    if length == 0 {
        return Ok(Vec::new());
    }
    if pointer.is_null() {
        return Err(format!("{name} pointer is null for {length} bytes"));
    }
    Ok(unsafe { std::slice::from_raw_parts(pointer, length) }.to_vec())
}
