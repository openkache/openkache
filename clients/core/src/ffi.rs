//! Runtime-neutral native ABI shared by non-Rust language bindings.
//!
//! The public C symbols are emitted by the thin `openkache-client` crate.  This
//! module owns the worker, validation, and operation mapping so every native
//! binding gets the same behavior without reimplementing protocol or
//! protection logic.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{SyncSender, sync_channel};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use openkache_protocol::{
    FFI_ABI_VERSION, FFI_CONNECTION_STATE_UNKNOWN, FFI_OPERATION_CONNECTION_STATE,
    FFI_OPERATION_RECONNECT, FFI_RESULT_CONNECTED, FFI_RESULT_CONNECTION_STATE, FFI_RESULT_CREATED,
    FFI_RESULT_DELETED, FFI_RESULT_ERROR, FFI_RESULT_NOT_DELETED, FFI_RESULT_NOT_FOUND,
    FFI_RESULT_NOT_STORED, FFI_RESULT_OK, FFI_RESULT_REPLACED, FFI_RESULT_VALUE,
    FFI_SET_CONDITION_IF_ABSENT, FFI_SET_CONDITION_IF_PRESENT, FFI_SET_CONDITION_NONE, Opcode,
    VALUE_FORMAT_ENCRYPTION_COMPACT, VALUE_FORMAT_ENCRYPTION_ROBUST,
};

use crate::value::{Compression, Encryption, ZstandardOptions};
use crate::{
    Certificate, ClientIdentity, ClientTimeouts, DataProtectionKey, DeleteOutcome, Endpoint,
    GetOutcome, ItemId, ItemValue, PrivateKey, ProtectedClient, RetryPolicy, ServerTrust,
    SetCondition, SetOptions, SetOutcome,
};

const COMMAND_QUEUE_CAPACITY: usize = 64;

/// Discriminator returned by an operation result.
#[derive(Clone, Copy)]
#[repr(u32)]
pub enum FfiResultKind {
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
    ConnectionState = FFI_RESULT_CONNECTION_STATE,
}

#[derive(Clone, Copy, Debug)]
#[repr(u32)]
enum FfiOperation {
    Ping = Opcode::Ping as u32,
    Get = Opcode::Get as u32,
    Set = Opcode::Set as u32,
    Delete = Opcode::Delete as u32,
    Stats = Opcode::Stats as u32,
    Sync = Opcode::Sync as u32,
    // Keep client-lifecycle commands outside the Smithy service opcode range.
    Reconnect = FFI_OPERATION_RECONNECT,
    ConnectionState = FFI_OPERATION_CONNECTION_STATE,
}

#[derive(Clone, Copy)]
#[repr(u32)]
enum FfiSetCondition {
    None = FFI_SET_CONDITION_NONE,
    IfAbsent = FFI_SET_CONDITION_IF_ABSENT,
    IfPresent = FFI_SET_CONDITION_IF_PRESENT,
}

impl TryFrom<u32> for FfiOperation {
    type Error = u32;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            value if value == Self::Ping as u32 => Ok(Self::Ping),
            value if value == Self::Get as u32 => Ok(Self::Get),
            value if value == Self::Set as u32 => Ok(Self::Set),
            value if value == Self::Delete as u32 => Ok(Self::Delete),
            value if value == Self::Stats as u32 => Ok(Self::Stats),
            value if value == Self::Sync as u32 => Ok(Self::Sync),
            value if value == Self::Reconnect as u32 => Ok(Self::Reconnect),
            value if value == Self::ConnectionState as u32 => Ok(Self::ConnectionState),
            _ => Err(value),
        }
    }
}

impl TryFrom<u32> for FfiSetCondition {
    type Error = u32;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            value if value == Self::None as u32 => Ok(Self::None),
            value if value == Self::IfAbsent as u32 => Ok(Self::IfAbsent),
            value if value == Self::IfPresent as u32 => Ok(Self::IfPresent),
            _ => Err(value),
        }
    }
}

/// Opaque result allocated by the native boundary.
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
        item_id: Option<ItemId>,
        application_key: Vec<u8>,
        value: Vec<u8>,
        set_options: SetOptions,
        response: SyncSender<FfiResult>,
    },
    Shutdown,
}

type CommandSender = crossfire::MTx<crossfire::mpsc::Array<Command>>;
type CommandReceiver = crossfire::Rx<crossfire::mpsc::Array<Command>>;

struct WorkerOptions {
    endpoint: Endpoint,
    trust: ServerTrust,
    identity: Option<ClientIdentity>,
    data_protection_key: DataProtectionKey,
    compression: Compression,
    encryption: Encryption,
    timeouts: ClientTimeouts,
    retry: RetryPolicy,
    max_in_flight: usize,
}

struct ConnectOptions {
    address: String,
    server_name: String,
    certificate: Vec<u8>,
    client_certificate: Vec<u8>,
    client_private_key: Vec<u8>,
    data_protection_key: DataProtectionKey,
    compression: Compression,
    encryption: Encryption,
    timeouts: ClientTimeouts,
    retry: RetryPolicy,
    max_in_flight: usize,
    require_certificate: bool,
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
        let timeouts = options.timeouts;
        let worker = thread::Builder::new()
            .name("openkache-client".to_string())
            .spawn(move || run_worker(receiver, ready_sender, options, worker_shutdown))
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
        operation: FfiOperation,
        item_id: Option<ItemId>,
        application_key: Vec<u8>,
        value: Vec<u8>,
        set_options: SetOptions,
    ) -> FfiResult {
        if self.shutdown.load(Ordering::Acquire) {
            return FfiResult::error("client is closed");
        }
        let (response, receiver) = sync_channel(1);
        let Some(deadline) = Instant::now().checked_add(self.request_timeout) else {
            return FfiResult::error("client request timeout exceeds the platform clock range");
        };
        let command = Command::Execute {
            operation,
            item_id,
            application_key,
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
        let _ = self.commands.try_send(Command::Shutdown);
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
        endpoint,
        trust,
        identity,
        data_protection_key,
        compression,
        encryption,
        timeouts,
        retry,
        max_in_flight,
    } = options;
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            let _ = ready.send(Err(format!("failed to create Tokio runtime: {error}")));
            return;
        }
    };
    let mut builder = ProtectedClient::builder(endpoint, data_protection_key)
        .server_trust(trust)
        .compression(compression)
        .encryption(encryption)
        .timeouts(timeouts)
        .retry_policy(retry)
        .max_in_flight(max_in_flight);
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
                item_id,
                application_key,
                value,
                set_options,
                response,
            } => {
                let result = runtime.block_on(execute(
                    &client,
                    operation,
                    item_id,
                    application_key,
                    value,
                    set_options,
                ));
                let _ = response.send(result);
            }
            Command::Shutdown => break,
        }
    }
}

async fn execute(
    client: &ProtectedClient,
    operation: FfiOperation,
    item_id: Option<ItemId>,
    application_key: Vec<u8>,
    value: Vec<u8>,
    set_options: SetOptions,
) -> FfiResult {
    if let Some(item_id) = item_id {
        return execute_raw(client, operation, item_id, value, set_options).await;
    }
    let result =
        match operation {
            FfiOperation::Ping => client
                .ping()
                .await
                .map(|_| FfiResult::success(FfiResultKind::Ok, Vec::new())),
            FfiOperation::Get => client.get(&application_key).await.map(|value| match value {
                GetOutcome::Found(value) => FfiResult::success(FfiResultKind::Value, value),
                GetOutcome::NotFound => FfiResult::success(FfiResultKind::NotFound, Vec::new()),
            }),
            FfiOperation::Set => client.set(&application_key, value, set_options).await.map(
                |outcome| match outcome {
                    SetOutcome::Created => FfiResult::success(FfiResultKind::Created, Vec::new()),
                    SetOutcome::Replaced => FfiResult::success(FfiResultKind::Replaced, Vec::new()),
                    SetOutcome::NotStored => {
                        FfiResult::success(FfiResultKind::NotStored, Vec::new())
                    }
                },
            ),
            FfiOperation::Delete => client.delete(&application_key).await.map(|deleted| {
                FfiResult::success(
                    match deleted {
                        DeleteOutcome::Deleted => FfiResultKind::Deleted,
                        DeleteOutcome::NotFound => FfiResultKind::NotDeleted,
                    },
                    Vec::new(),
                )
            }),
            FfiOperation::Stats => client
                .stats()
                .await
                .map(|stats| FfiResult::success(FfiResultKind::Value, stats.into_bytes())),
            FfiOperation::Sync => client
                .sync()
                .await
                .map(|()| FfiResult::success(FfiResultKind::Ok, Vec::new())),
            FfiOperation::Reconnect => client
                .reconnect()
                .await
                .map(|()| FfiResult::success(FfiResultKind::Ok, Vec::new())),
            FfiOperation::ConnectionState => {
                let state = client.connection_state() as u8;
                Ok(FfiResult::success(
                    FfiResultKind::ConnectionState,
                    vec![state],
                ))
            }
        };
    result.unwrap_or_else(|error| FfiResult::error(error.to_string()))
}

async fn execute_raw(
    client: &ProtectedClient,
    operation: FfiOperation,
    item_id: ItemId,
    value: Vec<u8>,
    set_options: SetOptions,
) -> FfiResult {
    let result = match operation {
        FfiOperation::Get => client.raw().get(item_id).await.map(|value| match value {
            GetOutcome::Found(value) => {
                FfiResult::success(FfiResultKind::Value, value.into_bytes())
            }
            GetOutcome::NotFound => FfiResult::success(FfiResultKind::NotFound, Vec::new()),
        }),
        FfiOperation::Set => client
            .raw()
            .set(item_id, ItemValue::new(value), set_options)
            .await
            .map(|outcome| match outcome {
                SetOutcome::Created => FfiResult::success(FfiResultKind::Created, Vec::new()),
                SetOutcome::Replaced => FfiResult::success(FfiResultKind::Replaced, Vec::new()),
                SetOutcome::NotStored => FfiResult::success(FfiResultKind::NotStored, Vec::new()),
            }),
        FfiOperation::Delete => client.raw().delete(item_id).await.map(|deleted| {
            FfiResult::success(
                match deleted {
                    DeleteOutcome::Deleted => FfiResultKind::Deleted,
                    DeleteOutcome::NotFound => FfiResultKind::NotDeleted,
                },
                Vec::new(),
            )
        }),
        operation => Err(crate::Error::Configuration {
            field: "operation",
            message: format!("raw operation does not support {operation:?}"),
        }),
    };
    result.unwrap_or_else(|error| FfiResult::error(error.to_string()))
}

/// Returns the native ABI version implemented by this core.
pub fn openkache_client_abi_version() -> u32 {
    FFI_ABI_VERSION
}

/// Connects a native client with the original ABI v1 configuration.
///
/// The legacy entry point requires a non-empty DER or PEM trust certificate and
/// uses the shared default Robust encryption, retry, and in-flight settings.
///
/// # Safety
///
/// Every non-empty pointer/length pair must identify readable memory for the
/// duration of this call. `data_protection_key` must contain exactly 32 bytes.
#[allow(clippy::too_many_arguments)]
pub unsafe fn openkache_client_connect(
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
        let certificate = copy_bytes(certificate, certificate_length, "certificate")?;
        if certificate.is_empty() {
            return Err("certificate must not be empty".to_string());
        }
        connect_owned(ConnectOptions {
            address: copy_utf8(address, address_length, "address")?,
            server_name: copy_utf8(server_name, server_name_length, "server name")?,
            certificate,
            client_certificate: Vec::new(),
            client_private_key: Vec::new(),
            data_protection_key: copy_data_protection_key(
                data_protection_key,
                data_protection_key_length,
            )?,
            compression: compression_from_inputs(
                compression_enabled,
                compression_level,
                minimum_input_size,
                minimum_savings,
            )?,
            encryption: Encryption::Robust,
            timeouts: timeouts_from_millis(connect_timeout_ms, request_timeout_ms)?,
            retry: RetryPolicy::default(),
            max_in_flight: crate::DEFAULT_MAX_IN_FLIGHT,
            require_certificate: true,
        })
    }))
}

/// Connects a native client with the complete shared-core configuration.
///
/// `certificate` may be one DER certificate or a PEM certificate chain. An
/// empty trust bundle selects system roots. The optional client certificate and
/// key may likewise be DER or PEM; both must be supplied together.
///
/// # Safety
///
/// Every non-empty pointer/length pair must identify readable memory for the
/// duration of this call. The data-protection key must contain exactly 32
/// bytes.
#[allow(clippy::too_many_arguments)]
pub unsafe fn openkache_client_connect_ex(
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
    boxed_result(catch_result(|| {
        connect_owned(ConnectOptions {
            address: copy_utf8(address, address_length, "address")?,
            server_name: copy_utf8(server_name, server_name_length, "server name")?,
            certificate: copy_bytes(certificate, certificate_length, "certificate")?,
            client_certificate: copy_bytes(
                client_certificate,
                client_certificate_length,
                "client certificate",
            )?,
            client_private_key: copy_bytes(
                client_private_key,
                client_private_key_length,
                "client private key",
            )?,
            data_protection_key: copy_data_protection_key(
                data_protection_key,
                data_protection_key_length,
            )?,
            compression: compression_from_inputs(
                compression_enabled,
                compression_level,
                minimum_input_size,
                minimum_savings,
            )?,
            encryption: match encryption {
                value if value == VALUE_FORMAT_ENCRYPTION_COMPACT as u32 => Encryption::Compact,
                value if value == VALUE_FORMAT_ENCRYPTION_ROBUST as u32 => Encryption::Robust,
                value => return Err(format!("unsupported encryption profile {value}")),
            },
            timeouts: timeouts_from_millis(connect_timeout_ms, request_timeout_ms)?,
            retry: RetryPolicy {
                max_attempts: retry_max_attempts,
            },
            max_in_flight,
            require_certificate: false,
        })
    }))
}

/// Executes one operation through an opaque native client.
///
/// # Safety
///
/// `client` must be live for the duration of this call. Every non-empty
/// application-key/value pointer pair must identify readable memory.
#[allow(clippy::too_many_arguments)]
pub unsafe fn openkache_client_execute(
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
            FfiOperation::Get | FfiOperation::Set | FfiOperation::Delete
                if application_key.is_empty() =>
            {
                Err("application key must not be empty".to_string())
            }
            FfiOperation::Ping
            | FfiOperation::Stats
            | FfiOperation::Sync
            | FfiOperation::Reconnect
            | FfiOperation::ConnectionState
                if !application_key.is_empty() =>
            {
                Err("operation does not accept an application key".to_string())
            }
            FfiOperation::Set if set_options.time_to_live_millis() == Some(0) => {
                Err("SET TTL must be greater than zero milliseconds".to_string())
            }
            FfiOperation::Ping
            | FfiOperation::Get
            | FfiOperation::Delete
            | FfiOperation::Stats
            | FfiOperation::Sync
            | FfiOperation::Reconnect
            | FfiOperation::ConnectionState
                if !value.is_empty() =>
            {
                Err("operation does not accept a value".to_string())
            }
            operation
                if !matches!(operation, FfiOperation::Set)
                    && (set_options.condition() != SetCondition::None
                        || set_options.time_to_live_millis().is_some()) =>
            {
                Err("SET options require a SET operation".to_string())
            }
            _ => Ok(client.execute(operation, None, application_key, value, set_options)),
        }
    }))
}

/// Executes an exact-item-ID operation without application-key derivation.
///
/// Raw `GET` and `SET` values bypass the protected value transformation and
/// are returned or stored exactly as supplied. Only `GET`, `SET`, and `DELETE`
/// accept an item ID.
///
/// # Safety
///
/// `client` must be live for the duration of this call. Every non-empty
/// pointer/length pair must identify readable memory.
#[allow(clippy::too_many_arguments)]
pub unsafe fn openkache_client_execute_raw(
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
    boxed_result(catch_result(|| {
        let client = unsafe {
            client
                .as_ref()
                .ok_or_else(|| "client pointer must not be null".to_string())?
        };
        let item_id_bytes = copy_bytes(item_id, item_id_length, "item_id")?;
        let item_id = ItemId::from_slice(&item_id_bytes).map_err(|error| error.to_string())?;
        let value = copy_bytes(value, value_length, "value")?;
        let operation = FfiOperation::try_from(operation)
            .map_err(|operation| format!("unsupported operation {operation}"))?;
        if !matches!(
            operation,
            FfiOperation::Get | FfiOperation::Set | FfiOperation::Delete
        ) {
            return Err("raw operation must be GET, SET, or DELETE".to_string());
        }
        let condition = FfiSetCondition::try_from(set_condition)
            .map_err(|condition| format!("unsupported SET condition {condition}"))?;
        let mut set_options = match condition {
            FfiSetCondition::None => SetOptions::new(),
            FfiSetCondition::IfAbsent => SetOptions::new().if_absent(),
            FfiSetCondition::IfPresent => SetOptions::new().if_present(),
        };
        if ttl_enabled != 0 {
            if ttl_ms == 0 {
                return Err("SET TTL must be greater than zero milliseconds".to_string());
            }
            set_options = set_options.expires_after_millis(ttl_ms);
        }
        if !matches!(operation, FfiOperation::Set)
            && (set_options.condition() != SetCondition::None
                || set_options.time_to_live_millis().is_some())
        {
            return Err("SET options require a SET operation".to_string());
        }
        if !matches!(operation, FfiOperation::Set) && !value.is_empty() {
            return Err("operation does not accept a value".to_string());
        }
        Ok(client.execute(operation, Some(item_id), Vec::new(), value, set_options))
    }))
}

/// Returns the current connection-state discriminator, or
/// `FFI_CONNECTION_STATE_UNKNOWN` for an invalid
/// handle/result.
///
/// # Safety
///
/// `client` must be null or a live handle returned by this library.
pub unsafe fn openkache_client_connection_state(client: *const FfiClient) -> u32 {
    let result = unsafe {
        openkache_client_execute(
            client,
            FfiOperation::ConnectionState as u32,
            std::ptr::null(),
            0,
            std::ptr::null(),
            0,
            FfiSetCondition::None as u32,
            0,
            0,
        )
    };
    if result.is_null() {
        return FFI_CONNECTION_STATE_UNKNOWN as u32;
    }
    let state = if unsafe { openkache_client_result_kind(result) }
        == FfiResultKind::ConnectionState as u32
    {
        let length = unsafe { openkache_client_result_data_length(result) };
        if length == 1 {
            let pointer = unsafe { openkache_client_result_data(result) };
            if pointer.is_null() {
                FFI_CONNECTION_STATE_UNKNOWN as u32
            } else {
                unsafe { *pointer as u32 }
            }
        } else {
            FFI_CONNECTION_STATE_UNKNOWN as u32
        }
    } else {
        FFI_CONNECTION_STATE_UNKNOWN as u32
    };
    unsafe { openkache_client_result_free(result) };
    state
}

/// Returns a result discriminator.
///
/// # Safety
///
/// `result` must be null or a live result returned by this library.
pub unsafe fn openkache_client_result_kind(result: *const FfiResult) -> u32 {
    unsafe { result.as_ref() }.map_or(FfiResultKind::Error as u32, |result| result.kind as u32)
}

/// Returns a borrowed result payload pointer.
///
/// # Safety
///
/// `result` must be null or a live result returned by this library. The
/// returned pointer is borrowed and must not outlive that result.
pub unsafe fn openkache_client_result_data(result: *const FfiResult) -> *const u8 {
    let Some(result) = (unsafe { result.as_ref() }) else {
        return std::ptr::null();
    };
    if result.payload.is_empty() {
        std::ptr::null()
    } else {
        result.payload.as_ptr()
    }
}

/// Returns a result payload length.
///
/// # Safety
///
/// `result` must be null or a live result returned by this library.
pub unsafe fn openkache_client_result_data_length(result: *const FfiResult) -> usize {
    unsafe { result.as_ref() }.map_or(0, |result| result.payload.len())
}

/// Moves a connected client handle out of a connected result.
///
/// # Safety
///
/// `result` must be null or a live result returned by this library. The
/// returned handle is owned by the caller and must be freed exactly once.
pub unsafe fn openkache_client_result_take_client(result: *mut FfiResult) -> *mut FfiClient {
    let Some(result) = (unsafe { result.as_mut() }) else {
        return std::ptr::null_mut();
    };
    result
        .client
        .take()
        .map_or(std::ptr::null_mut(), Box::into_raw)
}

/// Frees a result. Null is accepted.
///
/// # Safety
///
/// `result` must be null or a pointer returned by this library that has not
/// already been freed.
pub unsafe fn openkache_client_result_free(result: *mut FfiResult) {
    if !result.is_null() {
        drop(unsafe { Box::from_raw(result) });
    }
}

/// Closes and frees a client. Null is accepted.
///
/// # Safety
///
/// `client` must be null or a pointer returned by
/// [`openkache_client_result_take_client`] that has not already been freed.
pub unsafe fn openkache_client_free(client: *mut FfiClient) {
    if !client.is_null() {
        drop(unsafe { Box::from_raw(client) });
    }
}

fn connect_owned(options: ConnectOptions) -> std::result::Result<FfiResult, String> {
    let ConnectOptions {
        address,
        server_name,
        certificate,
        client_certificate,
        client_private_key,
        data_protection_key,
        compression,
        encryption,
        timeouts,
        retry,
        max_in_flight,
        require_certificate,
    } = options;
    if require_certificate && certificate.is_empty() {
        return Err("certificate must not be empty".to_string());
    }
    let endpoint = endpoint_from_strings(&address, &server_name)?;
    let trust = if certificate.is_empty() {
        ServerTrust::System
    } else {
        ServerTrust::Custom(
            Certificate::from_der_or_pem_chain(&certificate).map_err(|error| error.to_string())?,
        )
    };
    let identity = match (client_certificate.is_empty(), client_private_key.is_empty()) {
        (true, true) => None,
        (false, false) => {
            let chain = Certificate::from_der_or_pem_chain(&client_certificate)
                .map_err(|error| error.to_string())?;
            let private_key = PrivateKey::from_der_or_pem(&client_private_key)
                .map_err(|error| error.to_string())?;
            Some(ClientIdentity::new(chain, private_key).map_err(|error| error.to_string())?)
        }
        _ => {
            return Err(
                "client certificate and client private key must be supplied together".to_string(),
            );
        }
    };
    FfiClient::connect(WorkerOptions {
        endpoint,
        trust,
        identity,
        data_protection_key,
        compression,
        encryption,
        timeouts,
        retry,
        max_in_flight,
    })
    .map(FfiResult::connected)
}

fn endpoint_from_strings(
    address: &str,
    server_name: &str,
) -> std::result::Result<Endpoint, String> {
    if let Ok(socket_address) = address.parse::<std::net::SocketAddr>() {
        let name = if server_name.is_empty() {
            socket_address.ip().to_string()
        } else {
            server_name.to_string()
        };
        return Endpoint::from_socket_addr(socket_address, name).map_err(|error| error.to_string());
    }
    let endpoint = address
        .parse::<Endpoint>()
        .map_err(|error| format!("invalid server address: {error}"))?;
    if server_name.is_empty() || server_name == endpoint.server_name() {
        Ok(endpoint)
    } else {
        Err(
            "server_name overrides are supported only with a numeric server address; use an address hostname as its TLS name"
                .to_string(),
        )
    }
}

fn compression_from_inputs(
    enabled: u8,
    level: i32,
    minimum_input_size: usize,
    minimum_savings: usize,
) -> std::result::Result<Compression, String> {
    if enabled == 0 {
        Ok(Compression::Disabled)
    } else {
        if !(1..=22).contains(&level) {
            return Err("compression level must be from 1 through 22".to_string());
        }
        Ok(Compression::Zstandard(ZstandardOptions {
            level,
            minimum_input_size,
            minimum_savings,
        }))
    }
}

fn timeouts_from_millis(
    connect_timeout_ms: u64,
    request_timeout_ms: u64,
) -> std::result::Result<ClientTimeouts, String> {
    if connect_timeout_ms == 0 || request_timeout_ms == 0 {
        return Err("client timeouts must be greater than zero milliseconds".to_string());
    }
    Ok(ClientTimeouts {
        connect: Duration::from_millis(connect_timeout_ms),
        request: Duration::from_millis(request_timeout_ms),
    })
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
    let bytes = copy_bytes(pointer, length, "data_protection_key")?;
    DataProtectionKey::from_slice(&bytes).map_err(|error| error.to_string())
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
