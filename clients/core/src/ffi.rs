//! Stable C ABI shared by native language bindings.
//!
//! The ABI owns one Compio runtime and one protected client per native handle.  C, C++, and
//! other native bindings only marshal buffers and interpret result discriminators; connection
//! management, retries, protocol framing, and value protection remain in this crate.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{SyncSender, sync_channel};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use openkache_protocol::{
    FFI_ABI_VERSION, FFI_ADAPTER_OPERATION_GET_JSON, FFI_ADAPTER_OPERATION_RAW_DELETE,
    FFI_ADAPTER_OPERATION_RAW_GET, FFI_ADAPTER_OPERATION_RAW_SET, FFI_ADAPTER_OPERATION_RECONNECT,
    FFI_ADAPTER_OPERATION_SET_JSON, FFI_ADAPTER_OPERATION_STATE, FFI_CONNECTION_STATE_CLOSED,
    FFI_CONNECTION_STATE_CONNECTED, FFI_CONNECTION_STATE_DISCONNECTED,
    FFI_CONNECTION_STATE_RECONNECTING, FFI_CONNECTION_STATE_UNKNOWN, FFI_OPERATION_RECONNECT,
    FFI_RESULT_CONNECTED, FFI_RESULT_CREATED, FFI_RESULT_DELETED, FFI_RESULT_ERROR,
    FFI_RESULT_NOT_DELETED, FFI_RESULT_NOT_FOUND, FFI_RESULT_NOT_STORED, FFI_RESULT_OK,
    FFI_RESULT_REPLACED, FFI_RESULT_STATE, FFI_RESULT_VALUE, FFI_SET_CONDITION_IF_ABSENT,
    FFI_SET_CONDITION_IF_PRESENT, FFI_SET_CONDITION_NONE, Opcode, VALUE_FORMAT_ENCRYPTION_COMPACT,
    VALUE_FORMAT_ENCRYPTION_NONE, VALUE_FORMAT_ENCRYPTION_ROBUST,
};
use serde::Deserialize;

use crate::value::{Compression, Encryption, JsonValue, Value, ZstandardOptions};
use crate::{
    Certificate, ClientIdentity, ClientTimeouts, ConnectionState, DataProtectionKey, DeleteOutcome,
    Endpoint, GetOutcome, ItemId, ItemValue, LocalProtectedClient, PrivateKey, RetryPolicy,
    ServerTrust, SetCondition, SetOptions, SetOutcome,
};

/// Version of the native ABI represented by these declarations.
pub const ABI_VERSION: u32 = FFI_ABI_VERSION;

const COMMAND_QUEUE_CAPACITY: usize = 64;

/// Discriminator returned by an operation result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
#[non_exhaustive]
pub enum FfiResultKind {
    /// The operation failed; the payload is UTF-8 diagnostic text.
    Error = FFI_RESULT_ERROR,
    /// The operation completed without a value.
    Ok = FFI_RESULT_OK,
    /// The payload contains an owned operation value.
    Value = FFI_RESULT_VALUE,
    /// A lookup found no value.
    NotFound = FFI_RESULT_NOT_FOUND,
    /// A conditional or unconditional set created a value.
    Created = FFI_RESULT_CREATED,
    /// An unconditional or conditional set replaced a value.
    Replaced = FFI_RESULT_REPLACED,
    /// A delete removed a value.
    Deleted = FFI_RESULT_DELETED,
    /// A delete found no value.
    NotDeleted = FFI_RESULT_NOT_DELETED,
    /// A connect operation returned a client handle in the result.
    Connected = FFI_RESULT_CONNECTED,
    /// A conditional set did not change a value.
    NotStored = FFI_RESULT_NOT_STORED,
    /// The payload contains a connection-state string.
    State = FFI_RESULT_STATE,
}

/// Operation accepted by [`openkache_client_execute`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
#[non_exhaustive]
pub enum FfiOperation {
    /// Verify the connection.
    Ping = Opcode::Ping as u32,
    /// Retrieve one protected value.
    Get = Opcode::Get as u32,
    /// Store one protected value.
    Set = Opcode::Set as u32,
    /// Remove one protected value.
    Delete = Opcode::Delete as u32,
    /// Return server statistics.
    Stats = Opcode::Stats as u32,
    /// Wait for the server durability barrier.
    Sync = Opcode::Sync as u32,
    /// Explicitly reconnect without replaying a request.
    Reconnect = FFI_OPERATION_RECONNECT,
    /// Retrieve a canonical JSON value.
    GetJson = FFI_ADAPTER_OPERATION_GET_JSON,
    /// Store a canonical JSON value.
    SetJson = FFI_ADAPTER_OPERATION_SET_JSON,
    /// Reconnect using the language-adapter operation range.
    AdapterReconnect = FFI_ADAPTER_OPERATION_RECONNECT,
    /// Return the current connection state.
    State = FFI_ADAPTER_OPERATION_STATE,
    /// Retrieve a raw value by exact item ID.
    RawGet = FFI_ADAPTER_OPERATION_RAW_GET,
    /// Store a raw value by exact item ID.
    RawSet = FFI_ADAPTER_OPERATION_RAW_SET,
    /// Delete a raw value by exact item ID.
    RawDelete = FFI_ADAPTER_OPERATION_RAW_DELETE,
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
            value if value == Self::GetJson as u32 => Ok(Self::GetJson),
            value if value == Self::SetJson as u32 => Ok(Self::SetJson),
            value if value == Self::AdapterReconnect as u32 => Ok(Self::AdapterReconnect),
            value if value == Self::State as u32 => Ok(Self::State),
            value if value == Self::RawGet as u32 => Ok(Self::RawGet),
            value if value == Self::RawSet as u32 => Ok(Self::RawSet),
            value if value == Self::RawDelete as u32 => Ok(Self::RawDelete),
            value => Err(value),
        }
    }
}

/// Existence condition accepted by [`openkache_client_execute`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
#[non_exhaustive]
pub enum FfiSetCondition {
    /// Always store the supplied value.
    None = FFI_SET_CONDITION_NONE,
    /// Store only when the key is absent.
    IfAbsent = FFI_SET_CONDITION_IF_ABSENT,
    /// Store only when the key is present.
    IfPresent = FFI_SET_CONDITION_IF_PRESENT,
}

impl TryFrom<u32> for FfiSetCondition {
    type Error = u32;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            value if value == Self::None as u32 => Ok(Self::None),
            value if value == Self::IfAbsent as u32 => Ok(Self::IfAbsent),
            value if value == Self::IfPresent as u32 => Ok(Self::IfPresent),
            value => Err(value),
        }
    }
}

/// Opaque result allocated by the native ABI.
pub struct FfiResult {
    kind: FfiResultKind,
    payload: Vec<u8>,
    client: Option<Box<FfiClient>>,
}

/// Native connection options passed to [`openkache_client_connect_ex`].
///
/// Every pointer/length pair is copied before the function returns. A zero
/// length selects the default for optional numeric settings. `encryption` is
/// zero selects Robust for compatibility, while the generated value-format
/// identifiers select Compact or Robust explicitly.
#[repr(C)]
pub struct FfiConnectOptions {
    /// UTF-8 host and UDP port such as `127.0.0.1:4433` or `cache.example.com:4433`.
    pub address: *const u8,
    /// Byte length of [`Self::address`].
    pub address_length: usize,
    /// UTF-8 TLS server name.
    pub server_name: *const u8,
    /// Byte length of [`Self::server_name`].
    pub server_name_length: usize,
    /// One DER certificate or a PEM certificate chain used as server trust. An empty buffer uses
    /// the platform/system trust roots.
    pub certificate: *const u8,
    /// Byte length of [`Self::certificate`].
    pub certificate_length: usize,
    /// Optional PEM/DER client certificate chain for mutual TLS.
    pub client_certificate_chain: *const u8,
    /// Byte length of [`Self::client_certificate_chain`].
    pub client_certificate_chain_length: usize,
    /// Optional DER/PEM client private key for mutual TLS.
    pub client_private_key: *const u8,
    /// Byte length of [`Self::client_private_key`].
    pub client_private_key_length: usize,
    /// Exact 32-byte application data-protection key.
    pub data_protection_key: *const u8,
    /// Byte length of [`Self::data_protection_key`].
    pub data_protection_key_length: usize,
    /// Non-zero to enable Zstandard compression.
    pub compression_enabled: u8,
    /// Zstandard level, validated by the shared value codec.
    pub compression_level: i32,
    /// Minimum serialized input size eligible for compression.
    pub minimum_input_size: usize,
    /// Minimum compressed-byte savings required.
    pub minimum_savings: usize,
    /// Value encryption profile: zero/default or two for Robust, one for Compact.
    pub encryption: u32,
    /// Connection establishment timeout in milliseconds.
    pub connect_timeout_ms: u64,
    /// Complete request timeout in milliseconds.
    pub request_timeout_ms: u64,
    /// Maximum response-safe retry attempts; zero selects the core default.
    pub retry_max_attempts: usize,
    /// Maximum in-flight lanes; zero selects the core default.
    pub max_in_flight: usize,
}

/// Opaque native handle to a dedicated Rust client worker.
pub struct FfiClient {
    commands: CommandSender,
    request_timeout: Duration,
    shutdown: Arc<AtomicBool>,
    state: Arc<AtomicU32>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

enum Command {
    Execute {
        operation: FfiOperation,
        application_key: Vec<u8>,
        value: Vec<u8>,
        set_options: SetOptions,
        raw: bool,
        response: SyncSender<FfiResult>,
    },
    Shutdown,
}

type CommandSender = crossfire::MTx<crossfire::mpsc::Array<Command>>;
type CommandReceiver = crossfire::Rx<crossfire::mpsc::Array<Command>>;

struct WorkerOptions {
    endpoint: Endpoint,
    certificate: Vec<u8>,
    data_protection_key: DataProtectionKey,
    client_certificate_chain: Vec<u8>,
    client_private_key: Vec<u8>,
    compression: Compression,
    encryption: Encryption,
    timeouts: ClientTimeouts,
    retry: RetryPolicy,
    max_in_flight: usize,
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
    // The argument list mirrors the stable native connection contract.
    #[allow(clippy::too_many_arguments)]
    fn connect(
        endpoint: Endpoint,
        certificate: Vec<u8>,
        data_protection_key: DataProtectionKey,
        client_certificate_chain: Vec<u8>,
        client_private_key: Vec<u8>,
        compression: Compression,
        encryption: Encryption,
        timeouts: ClientTimeouts,
        retry: RetryPolicy,
        max_in_flight: usize,
    ) -> std::result::Result<Self, String> {
        let (commands, receiver) = crossfire::mpsc::bounded_blocking(COMMAND_QUEUE_CAPACITY);
        let (ready_sender, ready_receiver) = sync_channel(1);
        let shutdown = Arc::new(AtomicBool::new(false));
        let worker_shutdown = Arc::clone(&shutdown);
        let state = Arc::new(AtomicU32::new(connection_state_value(
            ConnectionState::Reconnecting,
        )));
        let worker_state = Arc::clone(&state);
        let options = WorkerOptions {
            endpoint,
            certificate,
            data_protection_key,
            client_certificate_chain,
            client_private_key,
            compression,
            encryption,
            timeouts,
            retry,
            max_in_flight,
        };
        let worker = thread::Builder::new()
            .name("openkache-client".to_owned())
            .spawn(move || {
                run_worker(
                    receiver,
                    ready_sender,
                    options,
                    worker_shutdown,
                    worker_state,
                )
            })
            .map_err(|error| format!("failed to start client worker: {error}"))?;

        match ready_receiver.recv() {
            Ok(Ok(())) => Ok(Self {
                commands,
                request_timeout: timeouts.request,
                shutdown,
                state,
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
        raw: bool,
    ) -> FfiResult {
        let (response, receiver) = sync_channel(1);
        let Some(deadline) = Instant::now().checked_add(self.request_timeout) else {
            return FfiResult::error("client request timeout exceeds the platform clock range");
        };
        let command = Command::Execute {
            operation,
            application_key,
            value,
            set_options,
            raw,
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

    fn connection_state(&self) -> u32 {
        self.state.load(Ordering::Acquire)
    }
}

impl Drop for FfiClient {
    fn drop(&mut self) {
        self.state.store(
            connection_state_value(ConnectionState::Closed),
            Ordering::Release,
        );
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
    state: Arc<AtomicU32>,
) {
    let WorkerOptions {
        endpoint,
        certificate,
        data_protection_key,
        client_certificate_chain,
        client_private_key,
        compression,
        encryption,
        timeouts,
        retry,
        max_in_flight,
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
            "OpenKache native client requires the Compio io_uring driver".to_owned(),
        ));
        return;
    }
    let mut builder = LocalProtectedClient::builder(endpoint, data_protection_key)
        .compression(compression)
        .encryption(encryption)
        .timeouts(timeouts)
        .retry_policy(retry)
        .max_in_flight(max_in_flight);
    if !certificate.is_empty() {
        let certificates = match Certificate::from_der_or_pem_chain(&certificate) {
            Ok(certificates) => certificates,
            Err(error) => {
                let _ = ready.send(Err(error.to_string()));
                return;
            }
        };
        builder = builder.server_trust(ServerTrust::Custom(certificates));
    }
    if !client_certificate_chain.is_empty() || !client_private_key.is_empty() {
        let certificate_chain = match Certificate::from_der_or_pem_chain(&client_certificate_chain)
        {
            Ok(certificate_chain) => certificate_chain,
            Err(error) => {
                let _ = ready.send(Err(error.to_string()));
                return;
            }
        };
        let private_key = match PrivateKey::from_der_or_pem(&client_private_key) {
            Ok(private_key) => private_key,
            Err(error) => {
                let _ = ready.send(Err(error.to_string()));
                return;
            }
        };
        let identity = match ClientIdentity::new(certificate_chain, private_key) {
            Ok(identity) => identity,
            Err(error) => {
                let _ = ready.send(Err(error.to_string()));
                return;
            }
        };
        builder = builder.client_identity(identity);
    }
    let client = match runtime.block_on(builder.connect()) {
        Ok(client) => client,
        Err(error) => {
            let _ = ready.send(Err(error.to_string()));
            return;
        }
    };
    state.store(
        connection_state_value(client.connection_state()),
        Ordering::Release,
    );
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
                application_key,
                value,
                set_options,
                raw,
                response,
            } => {
                let result = catch_unwind(AssertUnwindSafe(|| {
                    runtime.block_on(execute(
                        &client,
                        operation,
                        application_key,
                        value,
                        set_options,
                        raw,
                    ))
                }))
                .unwrap_or_else(|_| FfiResult::error("native client worker panicked"));
                state.store(
                    connection_state_value(client.connection_state()),
                    Ordering::Release,
                );
                let _ = response.send(result);
            }
            Command::Shutdown => break,
        }
    }
}

async fn execute(
    client: &LocalProtectedClient,
    operation: FfiOperation,
    application_key: Vec<u8>,
    value: Vec<u8>,
    set_options: SetOptions,
    raw: bool,
) -> FfiResult {
    let result = if raw {
        execute_raw(client, operation, application_key, value, set_options).await
    } else if matches!(
        operation,
        FfiOperation::RawGet | FfiOperation::RawSet | FfiOperation::RawDelete
    ) {
        let raw_operation = match operation {
            FfiOperation::RawGet => FfiOperation::Get,
            FfiOperation::RawSet => FfiOperation::Set,
            FfiOperation::RawDelete => FfiOperation::Delete,
            _ => unreachable!(),
        };
        execute_raw(client, raw_operation, application_key, value, set_options).await
    } else {
        execute_protected(client, operation, application_key, value, set_options).await
    };
    result.unwrap_or_else(|error| FfiResult::error(error.to_string()))
}

async fn execute_protected(
    client: &LocalProtectedClient,
    operation: FfiOperation,
    application_key: Vec<u8>,
    value: Vec<u8>,
    set_options: SetOptions,
) -> std::result::Result<FfiResult, crate::Error> {
    match operation {
        FfiOperation::Ping => client
            .ping()
            .await
            .map(|_| FfiResult::success(FfiResultKind::Ok, Vec::new())),
        FfiOperation::Get => client.get(&application_key).await.map(|value| match value {
            GetOutcome::Found(value) => FfiResult::success(FfiResultKind::Value, value),
            GetOutcome::NotFound => FfiResult::success(FfiResultKind::NotFound, Vec::new()),
        }),
        FfiOperation::GetJson => client
            .get_value(&application_key)
            .await
            .map_err(crate::Error::from)
            .and_then(json_result),
        FfiOperation::Set => client
            .set(&application_key, value, set_options)
            .await
            .map(set_result),
        FfiOperation::SetJson => match parse_json(&value) {
            Ok(json) => client
                .set_value(&application_key, Value::Json(json), set_options)
                .await
                .map(set_result),
            Err(error) => Err(crate::value::Error::InvalidJson(error).into()),
        },
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
        FfiOperation::AdapterReconnect => client
            .reconnect()
            .await
            .map(|()| FfiResult::success(FfiResultKind::Ok, Vec::new())),
        FfiOperation::State => Ok(FfiResult::success(
            FfiResultKind::State,
            format!("{:?}", client.connection_state()).into_bytes(),
        )),
        FfiOperation::RawGet | FfiOperation::RawSet | FfiOperation::RawDelete => {
            unreachable!("raw operations are dispatched by execute")
        }
    }
}

async fn execute_raw(
    client: &LocalProtectedClient,
    operation: FfiOperation,
    item_id: Vec<u8>,
    value: Vec<u8>,
    set_options: SetOptions,
) -> std::result::Result<FfiResult, crate::Error> {
    match operation {
        FfiOperation::Ping => client
            .raw()
            .ping()
            .await
            .map(|_| FfiResult::success(FfiResultKind::Ok, Vec::new())),
        FfiOperation::Get => {
            let item_id = ItemId::from_slice(&item_id)?;
            client.raw().get(item_id).await.map(|value| match value {
                GetOutcome::Found(value) => {
                    FfiResult::success(FfiResultKind::Value, value.into_bytes())
                }
                GetOutcome::NotFound => FfiResult::success(FfiResultKind::NotFound, Vec::new()),
            })
        }
        FfiOperation::Set => {
            let item_id = ItemId::from_slice(&item_id)?;
            client
                .raw()
                .set(item_id, ItemValue::new(value), set_options)
                .await
                .map(|outcome| match outcome {
                    SetOutcome::Created => FfiResult::success(FfiResultKind::Created, Vec::new()),
                    SetOutcome::Replaced => FfiResult::success(FfiResultKind::Replaced, Vec::new()),
                    SetOutcome::NotStored => {
                        FfiResult::success(FfiResultKind::NotStored, Vec::new())
                    }
                })
        }
        FfiOperation::Delete => {
            let item_id = ItemId::from_slice(&item_id)?;
            client.raw().delete(item_id).await.map(|deleted| {
                FfiResult::success(
                    match deleted {
                        DeleteOutcome::Deleted => FfiResultKind::Deleted,
                        DeleteOutcome::NotFound => FfiResultKind::NotDeleted,
                    },
                    Vec::new(),
                )
            })
        }
        FfiOperation::Stats => client
            .raw()
            .stats()
            .await
            .map(|stats| FfiResult::success(FfiResultKind::Value, stats.into_bytes())),
        FfiOperation::Sync => client
            .raw()
            .sync()
            .await
            .map(|()| FfiResult::success(FfiResultKind::Ok, Vec::new())),
        FfiOperation::Reconnect => client
            .raw()
            .reconnect()
            .await
            .map(|()| FfiResult::success(FfiResultKind::Ok, Vec::new())),
        FfiOperation::GetJson
        | FfiOperation::SetJson
        | FfiOperation::AdapterReconnect
        | FfiOperation::State
        | FfiOperation::RawGet
        | FfiOperation::RawSet
        | FfiOperation::RawDelete => {
            unreachable!("formatted operations are not valid in raw mode")
        }
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

fn json_result(outcome: GetOutcome<Value>) -> std::result::Result<FfiResult, crate::Error> {
    match outcome {
        GetOutcome::Found(Value::Json(value)) => serde_json_canonicalizer::to_vec(&value)
            .map(|payload| FfiResult::success(FfiResultKind::Value, payload))
            .map_err(|error| crate::value::Error::InvalidJson(error.to_string()).into()),
        GetOutcome::Found(Value::Raw(_)) => Err(crate::value::Error::ExpectedRawValue.into()),
        GetOutcome::NotFound => Ok(FfiResult::success(FfiResultKind::NotFound, Vec::new())),
    }
}

fn parse_json(bytes: &[u8]) -> std::result::Result<JsonValue, String> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let value = JsonValue::deserialize(&mut deserializer).map_err(|error| error.to_string())?;
    deserializer.end().map_err(|error| error.to_string())?;
    Ok(value)
}

fn connection_state_value(state: ConnectionState) -> u32 {
    match state {
        ConnectionState::Connected => FFI_CONNECTION_STATE_CONNECTED,
        ConnectionState::Reconnecting => FFI_CONNECTION_STATE_RECONNECTING,
        ConnectionState::Disconnected => FFI_CONNECTION_STATE_DISCONNECTED,
        ConnectionState::Closed => FFI_CONNECTION_STATE_CLOSED,
    }
}

/// Returns the native ABI version implemented by this library.
#[unsafe(no_mangle)]
pub extern "C" fn openkache_client_abi_version() -> u32 {
    ABI_VERSION
}

/// Connects a protected native client and returns an opaque result.
///
/// The address is a UTF-8 host/port authority such as `127.0.0.1:4433` or
/// `cache.example.com:4433`. The certificate may be one DER certificate, a
/// PEM chain, or empty to use system trust roots. The data-protection key is
/// exactly 32 bytes. All input buffers are copied before this function returns.
///
/// # Safety
///
/// Every non-empty pointer/length pair must identify readable memory for the duration of this
/// call.
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
    let options = FfiConnectOptions {
        address,
        address_length,
        server_name,
        server_name_length,
        certificate,
        certificate_length,
        data_protection_key,
        data_protection_key_length,
        client_certificate_chain: ptr::null(),
        client_certificate_chain_length: 0,
        client_private_key: ptr::null(),
        client_private_key_length: 0,
        compression_enabled,
        compression_level,
        minimum_input_size,
        minimum_savings,
        encryption: VALUE_FORMAT_ENCRYPTION_NONE as u32,
        connect_timeout_ms,
        request_timeout_ms,
        retry_max_attempts: 0,
        max_in_flight: 0,
    };
    boxed_result(catch_result(|| connect_options(&options)))
}

/// Connects a protected native client using the complete shared-core configuration.
///
/// This flat entry point is the canonical ABI used by C, C++, Swift, and
/// other native adapters. It accepts PEM or DER trust material, optional
/// mutual TLS identity, compression and encryption profiles, retries, and
/// stream-lane limits. All referenced buffers are copied before this function
/// returns.
///
/// # Safety
///
/// Every non-empty pointer/length pair must identify readable memory for the
/// duration of this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_client_connect_ex(
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
    encryption: u32,
    retry_max_attempts: usize,
    max_in_flight: usize,
    connect_timeout_ms: u64,
    request_timeout_ms: u64,
) -> *mut FfiResult {
    let options = FfiConnectOptions {
        address,
        address_length,
        server_name,
        server_name_length,
        certificate,
        certificate_length,
        data_protection_key,
        data_protection_key_length,
        client_certificate_chain,
        client_certificate_chain_length,
        client_private_key,
        client_private_key_length,
        compression_enabled,
        compression_level,
        minimum_input_size,
        minimum_savings,
        encryption,
        retry_max_attempts,
        max_in_flight,
        connect_timeout_ms,
        request_timeout_ms,
    };
    boxed_result(catch_result(|| connect_options(&options)))
}

/// Connects using a caller-owned options structure.
///
/// This helper is equivalent to [`openkache_client_connect_ex`] and is
/// separate so the flat function remains compatible with generated bindings.
///
/// # Safety
///
/// `options` must be null or point to a readable [`FfiConnectOptions`] whose
/// non-empty pointer/length pairs identify readable memory for the duration
/// of this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_client_connect_with_options(
    options: *const FfiConnectOptions,
) -> *mut FfiResult {
    boxed_result(catch_result(|| {
        let options = unsafe {
            options
                .as_ref()
                .ok_or_else(|| "connect options pointer must not be null".to_owned())?
        };
        connect_options(options)
    }))
}

fn connect_options(options: &FfiConnectOptions) -> std::result::Result<FfiResult, String> {
    let address = copy_utf8(options.address, options.address_length, "address")?;
    let mut endpoint: Endpoint = address
        .parse()
        .map_err(|error| format!("invalid server address: {error}"))?;
    let server_name = copy_utf8(
        options.server_name,
        options.server_name_length,
        "server name",
    )?;
    if !server_name.is_empty() {
        endpoint = endpoint
            .with_server_name(server_name)
            .map_err(|error| error.to_string())?;
    }
    let certificate = copy_bytes(
        options.certificate,
        options.certificate_length,
        "certificate",
    )?;
    let data_protection_key = copy_data_protection_key(
        options.data_protection_key,
        options.data_protection_key_length,
    )?;
    let client_certificate_chain = copy_bytes(
        options.client_certificate_chain,
        options.client_certificate_chain_length,
        "client certificate chain",
    )?;
    let client_private_key = copy_bytes(
        options.client_private_key,
        options.client_private_key_length,
        "client private key",
    )?;
    if client_certificate_chain.is_empty() != client_private_key.is_empty() {
        return Err(
            "client certificate chain and private key must be supplied together".to_owned(),
        );
    }
    let compression = if options.compression_enabled == 0 {
        Compression::Disabled
    } else {
        let defaults = ZstandardOptions::default();
        Compression::Zstandard(ZstandardOptions {
            level: if options.compression_level == 0 {
                defaults.level
            } else {
                options.compression_level
            },
            minimum_input_size: if options.minimum_input_size == 0 {
                defaults.minimum_input_size
            } else {
                options.minimum_input_size
            },
            minimum_savings: if options.minimum_savings == 0 {
                defaults.minimum_savings
            } else {
                options.minimum_savings
            },
        })
    };
    let encryption = match options.encryption {
        encryption
            if encryption == VALUE_FORMAT_ENCRYPTION_NONE as u32
                || encryption == VALUE_FORMAT_ENCRYPTION_ROBUST as u32 =>
        {
            Encryption::Robust
        }
        encryption if encryption == VALUE_FORMAT_ENCRYPTION_COMPACT as u32 => Encryption::Compact,
        encryption => return Err(format!("unsupported encryption profile {encryption}")),
    };
    if options.connect_timeout_ms == 0 || options.request_timeout_ms == 0 {
        return Err("client timeouts must be greater than zero milliseconds".to_owned());
    }
    let timeouts = ClientTimeouts {
        connect: Duration::from_millis(options.connect_timeout_ms),
        request: Duration::from_millis(options.request_timeout_ms),
    };
    let retry = if options.retry_max_attempts == 0 {
        RetryPolicy::default()
    } else {
        RetryPolicy {
            max_attempts: options.retry_max_attempts,
        }
    };
    let max_in_flight = if options.max_in_flight == 0 {
        crate::DEFAULT_MAX_IN_FLIGHT
    } else {
        options.max_in_flight
    };
    FfiClient::connect(
        endpoint,
        certificate,
        data_protection_key,
        client_certificate_chain,
        client_private_key,
        compression,
        encryption,
        timeouts,
        retry,
        max_in_flight,
    )
    .map(FfiResult::connected)
}

/// Executes one protected operation through an opaque native client.
///
/// For `GET`, `SET`, and `DELETE`, `application_key` is the exact application key used by the
/// shared data-protection layer. `SET` accepts an empty value and optional existence/TTL options.
/// `PING`, `STATS`, and `SYNC` require empty key and value buffers.
///
/// # Safety
///
/// `client` must be a live pointer returned by [`openkache_client_result_take_client`]. Every
/// non-empty application-key/value pointer pair must identify readable memory for this call, and
/// the client must not be freed until this call returns.
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
    execute_entry(
        client,
        operation,
        application_key,
        application_key_length,
        value,
        value_length,
        set_condition,
        ttl_enabled,
        ttl_ms,
        false,
    )
}

/// Executes one exact-item-ID operation without application-key derivation or
/// value protection.
///
/// `item_id` must contain exactly `OPENKACHE_SMITHY_ITEM_ID_BYTES` bytes for
/// `GET`, `SET`, and `DELETE`. The payload is sent and returned exactly as
/// supplied.
///
/// # Safety
///
/// `client` must be a live pointer returned by
/// [`openkache_client_result_take_client`]. Every non-empty pointer/length
/// pair must identify readable memory for the duration of this call, and the
/// client must not be freed until this call returns.
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
    execute_entry(
        client,
        operation,
        item_id,
        item_id_length,
        value,
        value_length,
        set_condition,
        ttl_enabled,
        ttl_ms,
        true,
    )
}

// The argument list mirrors the stable native operation contract.
#[allow(clippy::too_many_arguments)]
fn execute_entry(
    client: *const FfiClient,
    operation: u32,
    application_key: *const u8,
    application_key_length: usize,
    value: *const u8,
    value_length: usize,
    set_condition: u32,
    ttl_enabled: u8,
    ttl_ms: u64,
    raw: bool,
) -> *mut FfiResult {
    boxed_result(catch_result(|| {
        let client = unsafe {
            client
                .as_ref()
                .ok_or_else(|| "client pointer must not be null".to_owned())?
        };
        let application_key =
            copy_bytes(application_key, application_key_length, "application_key")?;
        let value = copy_bytes(value, value_length, "value")?;
        let operation = FfiOperation::try_from(operation)
            .map_err(|operation| format!("unsupported operation {operation}"))?;
        if (raw
            || matches!(
                operation,
                FfiOperation::RawGet | FfiOperation::RawSet | FfiOperation::RawDelete
            ))
            && matches!(
                operation,
                FfiOperation::Get
                    | FfiOperation::Set
                    | FfiOperation::Delete
                    | FfiOperation::RawGet
                    | FfiOperation::RawSet
                    | FfiOperation::RawDelete
            )
            && application_key.len() != crate::ITEM_ID_BYTES
        {
            return Err(format!(
                "item_id must contain exactly {} bytes, got {}",
                crate::ITEM_ID_BYTES,
                application_key.len()
            ));
        }
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
                return Err("SET TTL must be greater than zero milliseconds".to_owned());
            }
            set_options = set_options.expires_after_millis(ttl_ms);
        }
        match operation {
            FfiOperation::Ping
            | FfiOperation::Stats
            | FfiOperation::Sync
            | FfiOperation::Reconnect
            | FfiOperation::AdapterReconnect
            | FfiOperation::State
                if !application_key.is_empty() =>
            {
                Err("operation does not accept an application key".to_owned())
            }
            FfiOperation::Ping
            | FfiOperation::Get
            | FfiOperation::GetJson
            | FfiOperation::Delete
            | FfiOperation::RawGet
            | FfiOperation::RawDelete
            | FfiOperation::Stats
            | FfiOperation::Sync
            | FfiOperation::Reconnect
            | FfiOperation::AdapterReconnect
            | FfiOperation::State
                if !value.is_empty() =>
            {
                Err("operation does not accept a value".to_owned())
            }
            operation
                if !matches!(
                    operation,
                    FfiOperation::Set | FfiOperation::SetJson | FfiOperation::RawSet
                ) && (set_options.condition() != SetCondition::None
                    || set_options.time_to_live_millis().is_some()) =>
            {
                Err("SET options require a SET operation".to_owned())
            }
            _ => Ok(client.execute(operation, application_key, value, set_options, raw)),
        }
    }))
}

/// Returns a best-effort connection-state discriminator:
/// The returned value uses the generated native connection-state identifiers;
/// the unknown identifier is returned for a null or invalid handle.
///
/// # Safety
///
/// `client` must be null or a live pointer returned by
/// [`openkache_client_result_take_client`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_client_connection_state(client: *const FfiClient) -> u32 {
    unsafe { client.as_ref() }.map_or(FFI_CONNECTION_STATE_UNKNOWN, FfiClient::connection_state)
}

/// Returns an FFI result discriminator.
///
/// A null result is treated as [`FfiResultKind::Error`].
///
/// # Safety
///
/// `result` must be null or a live pointer returned by this library.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_client_result_kind(result: *const FfiResult) -> u32 {
    unsafe { result.as_ref() }.map_or(FfiResultKind::Error as u32, |result| result.kind as u32)
}

/// Returns a borrowed pointer to an FFI result payload.
///
/// The pointer remains valid until `result` is freed.
///
/// # Safety
///
/// `result` must be null or a live pointer returned by this library.
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
/// `result` must be null or a live pointer returned by this library.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_client_result_data_length(result: *const FfiResult) -> usize {
    unsafe { result.as_ref() }.map_or(0, |result| result.payload.len())
}

/// Moves a connected client handle out of an FFI result.
///
/// The result remains valid and may be freed after this function returns. Calling this function
/// more than once returns null.
///
/// # Safety
///
/// `result` must be null or a unique, live pointer returned by [`openkache_client_connect`].
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
