//! Stable C ABI for native client integrations.

use std::net::SocketAddr;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{SyncSender, sync_channel};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use openkache_protocol::{
    FFI_ABI_VERSION, FFI_OPERATION_GET_JSON, FFI_OPERATION_RAW_DELETE, FFI_OPERATION_RAW_GET,
    FFI_OPERATION_RAW_SET, FFI_OPERATION_RECONNECT, FFI_OPERATION_SET_JSON, FFI_OPERATION_STATE,
    FFI_RESULT_CONNECTED, FFI_RESULT_CREATED, FFI_RESULT_DELETED, FFI_RESULT_ERROR,
    FFI_RESULT_NOT_DELETED, FFI_RESULT_NOT_FOUND, FFI_RESULT_NOT_STORED, FFI_RESULT_OK,
    FFI_RESULT_REPLACED, FFI_RESULT_STATE, FFI_RESULT_VALUE, FFI_SET_CONDITION_IF_ABSENT,
    FFI_SET_CONDITION_IF_PRESENT, FFI_SET_CONDITION_NONE, Opcode,
};
use serde::Deserialize;

use crate::value::{Compression, JsonValue, Value, ZstandardOptions};
use crate::{
    Certificate, ClientIdentity, ClientTimeouts, DataProtectionKey, DeleteOutcome, Endpoint,
    GetOutcome, ItemId, ItemValue, LocalProtectedClient, PrivateKey, ServerTrust, SetCondition,
    SetOptions, SetOutcome,
};

const COMMAND_QUEUE_CAPACITY: usize = 64;

#[derive(Clone, Copy)]
#[repr(u32)]
enum FfiResultKind {
    Error = FFI_RESULT_ERROR,
    Ok = FFI_RESULT_OK,
    Value = FFI_RESULT_VALUE,
    NotFound = FFI_RESULT_NOT_FOUND,
    Created = FFI_RESULT_CREATED,
    Replaced = FFI_RESULT_REPLACED,
    Deleted = FFI_RESULT_DELETED,
    NotDeleted = FFI_RESULT_NOT_DELETED,
    Connected = FFI_RESULT_CONNECTED,
    NotStored = FFI_RESULT_NOT_STORED,
    State = FFI_RESULT_STATE,
}

macro_rules! ffi_input_enum {
    (
        enum $name:ident {
            $($variant:ident = $value:expr),+ $(,)?
        }
    ) => {
        #[derive(Clone, Copy)]
        #[repr(u32)]
        enum $name {
            $($variant = $value),+
        }

        impl TryFrom<u32> for $name {
            type Error = u32;

            fn try_from(value: u32) -> std::result::Result<Self, Self::Error> {
                match value {
                    $(value if value == Self::$variant as u32 => Ok(Self::$variant),)+
                    _ => Err(value),
                }
            }
        }
    };
}

ffi_input_enum! {
    enum FfiOperation {
        Ping = Opcode::Ping as u32,
        Get = Opcode::Get as u32,
        Set = Opcode::Set as u32,
        Delete = Opcode::Delete as u32,
        Stats = Opcode::Stats as u32,
        Sync = Opcode::Sync as u32,
        GetJson = FFI_OPERATION_GET_JSON,
        SetJson = FFI_OPERATION_SET_JSON,
        Reconnect = FFI_OPERATION_RECONNECT,
        State = FFI_OPERATION_STATE,
        RawGet = FFI_OPERATION_RAW_GET,
        RawSet = FFI_OPERATION_RAW_SET,
        RawDelete = FFI_OPERATION_RAW_DELETE,
    }
}

ffi_input_enum! {
enum FfiSetCondition {
    None = FFI_SET_CONDITION_NONE,
    IfAbsent = FFI_SET_CONDITION_IF_ABSENT,
    IfPresent = FFI_SET_CONDITION_IF_PRESENT,
    }
}

/// Opaque result allocated by the FFI boundary.
pub struct FfiResult {
    kind: FfiResultKind,
    payload: Vec<u8>,
    client: Option<Box<FfiClient>>,
}

/// Opaque native handle to a dedicated Rust client worker.
pub struct FfiClient {
    commands: CommandSender,
    request_timeout: Duration,
    shutdown: Arc<AtomicBool>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

enum Command {
    Execute {
        operation: FfiOperation,
        key: Vec<u8>,
        value: Vec<u8>,
        set_options: SetOptions,
        response: SyncSender<FfiResult>,
    },
    Shutdown,
}

type CommandSender = crossfire::MTx<crossfire::mpsc::Array<Command>>;
type CommandReceiver = crossfire::Rx<crossfire::mpsc::Array<Command>>;

struct WorkerOptions {
    address: SocketAddr,
    server_name: String,
    certificate: Vec<u8>,
    identity: Option<ClientIdentity>,
    data_protection_key: DataProtectionKey,
    compression: Compression,
    timeouts: ClientTimeouts,
    max_in_flight: usize,
    retry_max_attempts: usize,
}

impl FfiResult {
    fn error(message: impl Into<String>) -> Self {
        Self {
            kind: FfiResultKind::Error,
            payload: message.into().into_bytes(),
            client: None,
        }
    }

    fn success(kind: FfiResultKind, payload: Vec<u8>) -> Self {
        Self {
            kind,
            payload,
            client: None,
        }
    }

    fn connected(client: FfiClient) -> Self {
        Self {
            kind: FfiResultKind::Connected,
            payload: Vec::new(),
            client: Some(Box::new(client)),
        }
    }
}

impl FfiClient {
    fn connect(options: WorkerOptions) -> std::result::Result<Self, String> {
        let (commands, receiver) = crossfire::mpsc::bounded_blocking(COMMAND_QUEUE_CAPACITY);
        let (ready_sender, ready_receiver) = sync_channel(1);
        let shutdown = Arc::new(AtomicBool::new(false));
        let worker_shutdown = Arc::clone(&shutdown);
        let request_timeout = options.timeouts.request;
        let worker = thread::Builder::new()
            .name("openkache-client".to_string())
            .spawn(move || {
                run_worker(receiver, ready_sender, options, worker_shutdown);
            })
            .map_err(|error| format!("failed to start client worker: {error}"))?;

        match ready_receiver.recv() {
            Ok(Ok(())) => Ok(Self {
                commands,
                request_timeout,
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
        operation: FfiOperation,
        application_key: Vec<u8>,
        value: Vec<u8>,
        set_options: SetOptions,
    ) -> FfiResult {
        let (response, receiver) = sync_channel(1);
        let Some(deadline) = Instant::now().checked_add(self.request_timeout) else {
            return FfiResult::error("client request timeout exceeds the platform clock range");
        };
        let command = Command::Execute {
            operation,
            key: application_key,
            value,
            set_options,
            response,
        };
        let remaining = deadline.saturating_duration_since(Instant::now());
        if let Err(error) = self.commands.send_timeout(command, remaining) {
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
        // A non-blocking send can lose the shutdown marker when the bounded
        // queue is full. In that case the worker would drain the queue and
        // wait for another command while this destructor joins it forever.
        // Wait for one slot so the marker is guaranteed to reach the worker.
        let _ = self.commands.send(Command::Shutdown);
        if let Ok(worker) = self.worker.get_mut()
            && let Some(worker) = worker.take()
        {
            let _ = worker.join();
        }
    }
}

fn run_worker(
    commands: CommandReceiver,
    ready: SyncSender<std::result::Result<(), String>>,
    options: WorkerOptions,
    shutdown: Arc<AtomicBool>,
) {
    let WorkerOptions {
        address,
        server_name,
        certificate,
        identity,
        data_protection_key,
        compression,
        timeouts,
        max_in_flight,
        retry_max_attempts,
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
    let endpoint = match Endpoint::from_socket_addr(address, server_name) {
        Ok(endpoint) => endpoint,
        Err(error) => {
            let _ = ready.send(Err(error.to_string()));
            return;
        }
    };
    let certificates = match Certificate::from_der_or_pem_chain(&certificate) {
        Ok(certificates) => certificates,
        Err(error) => {
            let _ = ready.send(Err(error.to_string()));
            return;
        }
    };
    let mut builder = LocalProtectedClient::builder(endpoint, data_protection_key)
        .server_trust(ServerTrust::Custom(certificates))
        .compression(compression)
        .timeouts(timeouts)
        .max_in_flight(max_in_flight)
        .retry_policy(crate::RetryPolicy {
            max_attempts: retry_max_attempts,
        });
    if let Some(identity) = identity {
        builder = builder.client_identity(identity);
    }
    let client = match runtime.block_on(builder.connect()) {
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
    client: &LocalProtectedClient,
    operation: FfiOperation,
    key: Vec<u8>,
    value: Vec<u8>,
    set_options: SetOptions,
) -> FfiResult {
    let result: std::result::Result<FfiResult, String> = match operation {
        FfiOperation::Ping => client
            .ping()
            .await
            .map(|_| FfiResult::success(FfiResultKind::Ok, Vec::new()))
            .map_err(|error| error.to_string()),
        FfiOperation::Get => client
            .get(&key)
            .await
            .map(|value| match value {
                GetOutcome::Found(value) => FfiResult::success(FfiResultKind::Value, value),
                GetOutcome::NotFound => FfiResult::success(FfiResultKind::NotFound, Vec::new()),
            })
            .map_err(|error| error.to_string()),
        FfiOperation::Set => client
            .set(&key, value, set_options)
            .await
            .map(set_result)
            .map_err(|error| error.to_string()),
        FfiOperation::GetJson => client
            .get_value(&key)
            .await
            .map_err(|error| error.to_string())
            .and_then(json_result),
        FfiOperation::SetJson => match parse_json(&value) {
            Ok(json) => client
                .set_value(&key, Value::Json(json), set_options)
                .await
                .map(set_result)
                .map_err(|error| error.to_string()),
            Err(error) => Err(error),
        },
        FfiOperation::RawGet => match ItemId::from_slice(&key) {
            Ok(item_id) => client
                .raw()
                .get(item_id)
                .await
                .map(|value| match value {
                    GetOutcome::Found(value) => {
                        FfiResult::success(FfiResultKind::Value, value.into_bytes())
                    }
                    GetOutcome::NotFound => FfiResult::success(FfiResultKind::NotFound, Vec::new()),
                })
                .map_err(|error| error.to_string()),
            Err(error) => Err(error.to_string()),
        },
        FfiOperation::RawSet => match ItemId::from_slice(&key) {
            Ok(item_id) => client
                .raw()
                .set(item_id, ItemValue::new(value), set_options)
                .await
                .map(set_result)
                .map_err(|error| error.to_string()),
            Err(error) => Err(error.to_string()),
        },
        FfiOperation::RawDelete => match ItemId::from_slice(&key) {
            Ok(item_id) => client
                .raw()
                .delete(item_id)
                .await
                .map(|outcome| {
                    FfiResult::success(
                        match outcome {
                            DeleteOutcome::Deleted => FfiResultKind::Deleted,
                            DeleteOutcome::NotFound => FfiResultKind::NotDeleted,
                        },
                        Vec::new(),
                    )
                })
                .map_err(|error| error.to_string()),
            Err(error) => Err(error.to_string()),
        },
        FfiOperation::Delete => client
            .delete(&key)
            .await
            .map(|outcome| {
                FfiResult::success(
                    match outcome {
                        DeleteOutcome::Deleted => FfiResultKind::Deleted,
                        DeleteOutcome::NotFound => FfiResultKind::NotDeleted,
                    },
                    Vec::new(),
                )
            })
            .map_err(|error| error.to_string()),
        FfiOperation::Stats => client
            .stats()
            .await
            .map(|stats| FfiResult::success(FfiResultKind::Value, stats.into_bytes()))
            .map_err(|error| error.to_string()),
        FfiOperation::Sync => client
            .sync()
            .await
            .map(|()| FfiResult::success(FfiResultKind::Ok, Vec::new()))
            .map_err(|error| error.to_string()),
        FfiOperation::Reconnect => client
            .reconnect()
            .await
            .map(|()| FfiResult::success(FfiResultKind::Ok, Vec::new()))
            .map_err(|error| error.to_string()),
        FfiOperation::State => Ok(FfiResult::success(
            FfiResultKind::State,
            format!("{:?}", client.connection_state()).into_bytes(),
        )),
    };
    match result {
        Ok(result) => result,
        Err(error) => FfiResult::error(error),
    }
}

fn set_result(outcome: SetOutcome) -> FfiResult {
    FfiResult::success(
        match outcome {
            SetOutcome::Created => FfiResultKind::Created,
            SetOutcome::Replaced => FfiResultKind::Replaced,
            SetOutcome::NotStored => FfiResultKind::NotStored,
        },
        Vec::new(),
    )
}

fn json_result(outcome: GetOutcome<Value>) -> std::result::Result<FfiResult, String> {
    match outcome {
        GetOutcome::Found(Value::Json(value)) => serde_json_canonicalizer::to_vec(&value)
            .map(|payload| FfiResult::success(FfiResultKind::Value, payload))
            .map_err(|error| error.to_string()),
        GetOutcome::Found(Value::Raw(_)) => Err("formatted value is not JSON".to_string()),
        GetOutcome::NotFound => Ok(FfiResult::success(FfiResultKind::NotFound, Vec::new())),
    }
}

fn parse_json(bytes: &[u8]) -> std::result::Result<JsonValue, String> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let value = JsonValue::deserialize(&mut deserializer).map_err(|error| error.to_string())?;
    deserializer.end().map_err(|error| error.to_string())?;
    Ok(value)
}

/// Returns the native ABI version implemented by this library.
#[unsafe(no_mangle)]
pub extern "C" fn openkache_client_abi_version() -> u32 {
    FFI_ABI_VERSION
}

/// Connects a native client and returns an opaque result.
///
/// # Safety
///
/// Every non-empty pointer/length pair must identify readable memory for the duration of this
/// call. `data_protection_key` must contain exactly 32 bytes.
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
    boxed_result(catch_result(|| {
        connect_impl(
            address,
            address_length,
            server_name,
            server_name_length,
            certificate,
            certificate_length,
            ptr::null(),
            0,
            ptr::null(),
            0,
            data_protection_key,
            data_protection_key_length,
            compression_enabled,
            compression_level,
            minimum_input_size,
            minimum_savings,
            connect_timeout_ms,
            request_timeout_ms,
            256,
            2,
        )
    }))
}

/// Connects a native client with optional mutual-TLS identity and explicit core settings.
///
/// `client_certificate_chain` may contain one DER certificate or one or more PEM certificates.
/// `client_private_key` accepts a DER or PEM PKCS#1, SEC1, or PKCS#8 private key.
///
/// # Safety
///
/// Every non-empty pointer/length pair must identify readable memory for the duration of this
/// call. `data_protection_key` must contain exactly 32 bytes. `max_in_flight` and
/// `retry_max_attempts` must be positive.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_client_connect_v2(
    address: *const u8,
    address_length: usize,
    server_name: *const u8,
    server_name_length: usize,
    certificate: *const u8,
    certificate_length: usize,
    client_certificate_chain: *const u8,
    client_certificate_chain_length: usize,
    client_private_key: *const u8,
    client_private_key_length: usize,
    data_protection_key: *const u8,
    data_protection_key_length: usize,
    compression_enabled: u8,
    compression_level: i32,
    minimum_input_size: usize,
    minimum_savings: usize,
    connect_timeout_ms: u64,
    request_timeout_ms: u64,
    max_in_flight: u64,
    retry_max_attempts: u64,
) -> *mut FfiResult {
    boxed_result(catch_result(|| {
        connect_impl(
            address,
            address_length,
            server_name,
            server_name_length,
            certificate,
            certificate_length,
            client_certificate_chain,
            client_certificate_chain_length,
            client_private_key,
            client_private_key_length,
            data_protection_key,
            data_protection_key_length,
            compression_enabled,
            compression_level,
            minimum_input_size,
            minimum_savings,
            connect_timeout_ms,
            request_timeout_ms,
            usize::try_from(max_in_flight)
                .map_err(|_| "max_in_flight exceeds the native platform limit".to_string())?,
            usize::try_from(retry_max_attempts)
                .map_err(|_| "retry_max_attempts exceeds the native platform limit".to_string())?,
        )
    }))
}

#[allow(clippy::too_many_arguments)]
fn connect_impl(
    address: *const u8,
    address_length: usize,
    server_name: *const u8,
    server_name_length: usize,
    certificate: *const u8,
    certificate_length: usize,
    client_certificate_chain: *const u8,
    client_certificate_chain_length: usize,
    client_private_key: *const u8,
    client_private_key_length: usize,
    data_protection_key: *const u8,
    data_protection_key_length: usize,
    compression_enabled: u8,
    compression_level: i32,
    minimum_input_size: usize,
    minimum_savings: usize,
    connect_timeout_ms: u64,
    request_timeout_ms: u64,
    max_in_flight: usize,
    retry_max_attempts: usize,
) -> std::result::Result<FfiResult, String> {
    let address = copy_utf8(address, address_length, "address")?;
    let address = address
        .parse()
        .map_err(|error| format!("invalid server address: {error}"))?;
    let server_name = copy_utf8(server_name, server_name_length, "server name")?;
    let certificate = copy_bytes(certificate, certificate_length, "certificate")?;
    if certificate.is_empty() {
        return Err("certificate must not be empty".to_string());
    }
    let identity = copy_identity(
        client_certificate_chain,
        client_certificate_chain_length,
        client_private_key,
        client_private_key_length,
    )?;
    let data_protection_key =
        copy_data_protection_key(data_protection_key, data_protection_key_length)?;
    let compression = if compression_enabled == 0 {
        Compression::Disabled
    } else {
        Compression::Zstandard(ZstandardOptions {
            level: compression_level,
            minimum_input_size,
            minimum_savings,
        })
    };
    if connect_timeout_ms == 0 || request_timeout_ms == 0 {
        return Err("client timeouts must be greater than zero milliseconds".to_string());
    }
    if max_in_flight == 0 {
        return Err("max_in_flight must be greater than zero".to_string());
    }
    if retry_max_attempts == 0 {
        return Err("retry_max_attempts must be greater than zero".to_string());
    }
    let timeouts = ClientTimeouts {
        connect: Duration::from_millis(connect_timeout_ms),
        request: Duration::from_millis(request_timeout_ms),
    };
    FfiClient::connect(WorkerOptions {
        address,
        server_name,
        certificate,
        identity,
        data_protection_key,
        compression,
        timeouts,
        max_in_flight,
        retry_max_attempts,
    })
    .map(FfiResult::connected)
}

/// Executes one operation through an opaque native client.
///
/// # Safety
///
/// `client` must be a live pointer returned by [`openkache_client_result_take_client`].
/// Every non-empty application-key/value pointer pair must identify readable memory for this call,
/// and the client must not be freed until this call returns.
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
    boxed_result(catch_result(|| {
        let client = unsafe {
            client
                .as_ref()
                .ok_or_else(|| "client pointer must not be null".to_string())?
        };
        let application_key =
            copy_bytes(application_key, application_key_length, "application_key")?;
        let value = copy_bytes(value, value_length, "value")?;
        let operation = FfiOperation::try_from(operation)
            .map_err(|operation| format!("unsupported operation {operation}"))?;
        let condition = match FfiSetCondition::try_from(set_condition)
            .map_err(|condition| format!("unsupported SET condition {condition}"))?
        {
            FfiSetCondition::None => SetCondition::None,
            FfiSetCondition::IfAbsent => SetCondition::IfAbsent,
            FfiSetCondition::IfPresent => SetCondition::IfPresent,
        };
        let mut set_options = match condition {
            SetCondition::None => SetOptions::new(),
            SetCondition::IfAbsent => SetOptions::new().if_absent(),
            SetCondition::IfPresent => SetOptions::new().if_present(),
        };
        if ttl_enabled != 0 {
            if ttl_ms == 0 {
                return Err("SET TTL must be greater than zero milliseconds".to_string());
            }
            set_options = set_options.expires_after_millis(ttl_ms);
        }
        match operation {
            FfiOperation::Get
            | FfiOperation::Set
            | FfiOperation::GetJson
            | FfiOperation::SetJson
            | FfiOperation::Delete
                if application_key.is_empty() =>
            {
                Err("application key must not be empty".to_string())
            }
            FfiOperation::RawGet | FfiOperation::RawSet | FfiOperation::RawDelete
                if application_key.len() != crate::ITEM_ID_BYTES =>
            {
                Err(format!(
                    "raw item ID must contain exactly {} bytes, got {}",
                    crate::ITEM_ID_BYTES,
                    application_key.len()
                ))
            }
            FfiOperation::Ping
            | FfiOperation::Stats
            | FfiOperation::Sync
            | FfiOperation::Reconnect
            | FfiOperation::State
                if !application_key.is_empty() =>
            {
                Err("operation does not accept an application key".to_string())
            }
            FfiOperation::Ping
            | FfiOperation::Get
            | FfiOperation::Set
            | FfiOperation::GetJson
            | FfiOperation::SetJson
            | FfiOperation::Delete
            | FfiOperation::Stats
            | FfiOperation::Sync
            | FfiOperation::Reconnect
            | FfiOperation::State
            | FfiOperation::RawGet
            | FfiOperation::RawSet
            | FfiOperation::RawDelete
                if !value.is_empty()
                    && !matches!(
                        operation,
                        FfiOperation::Set | FfiOperation::SetJson | FfiOperation::RawSet
                    ) =>
            {
                Err("operation does not accept a value".to_string())
            }
            operation
                if !matches!(
                    operation,
                    FfiOperation::Set | FfiOperation::SetJson | FfiOperation::RawSet
                ) && (set_options.condition() != SetCondition::None
                    || set_options.time_to_live_millis().is_some()) =>
            {
                Err("SET options require a SET operation".to_string())
            }
            _ => Ok(client.execute(operation, application_key, value, set_options)),
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
    unsafe { result.as_ref() }.map_or(FfiResultKind::Error as u32, |result| result.kind as u32)
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

fn copy_data_protection_key(
    pointer: *const u8,
    length: usize,
) -> std::result::Result<DataProtectionKey, String> {
    if length == 0 {
        return DataProtectionKey::from_slice(&[]).map_err(|error| error.to_string());
    }
    if pointer.is_null() {
        return Err(format!(
            "data protection key pointer is null for {length} bytes"
        ));
    }
    let bytes = unsafe { std::slice::from_raw_parts(pointer, length) };
    DataProtectionKey::from_slice(bytes).map_err(|error| error.to_string())
}

fn copy_identity(
    certificate_chain: *const u8,
    certificate_chain_length: usize,
    private_key: *const u8,
    private_key_length: usize,
) -> std::result::Result<Option<ClientIdentity>, String> {
    if certificate_chain_length == 0 && private_key_length == 0 {
        return Ok(None);
    }
    if certificate_chain_length == 0 || private_key_length == 0 {
        return Err(
            "client certificate chain and private key must be supplied together".to_string(),
        );
    }
    let certificate_bytes = copy_bytes(
        certificate_chain,
        certificate_chain_length,
        "client certificate chain",
    )?;
    let certificates = Certificate::from_der_or_pem_chain(&certificate_bytes)
        .map_err(|error| error.to_string())?;
    let private_key_bytes = copy_bytes(private_key, private_key_length, "client private key")?;
    let private_key =
        PrivateKey::from_der_or_pem(&private_key_bytes).map_err(|error| error.to_string())?;
    ClientIdentity::new(certificates, private_key)
        .map(Some)
        .map_err(|error| error.to_string())
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
