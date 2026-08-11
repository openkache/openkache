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

use openkache_protocol::Opcode;

pub use crate::contract::FFI_ABI_VERSION as ABI_VERSION;
pub use crate::contract::FfiNamespaceDescriptor;
pub use crate::contract::{
    FFI_NAMESPACE_DEFAULT_EVICTION_EVICTABLE, FFI_NAMESPACE_DEFAULT_EVICTION_PROTECTED,
    FFI_NAMESPACE_DEFAULT_EXPIRATION_FIXED_TTL, FFI_NAMESPACE_DEFAULT_EXPIRATION_NO_EXPIRY,
    FFI_NAMESPACE_DESCRIPTOR_DECODE_INVALID, FFI_NAMESPACE_DESCRIPTOR_DECODE_OK,
    FFI_NAMESPACE_DESCRIPTOR_DEFAULT_EVICTION_OFFSET,
    FFI_NAMESPACE_DESCRIPTOR_DEFAULT_EXPIRATION_OFFSET,
    FFI_NAMESPACE_DESCRIPTOR_DEFAULT_TTL_MS_OFFSET,
    FFI_NAMESPACE_DESCRIPTOR_EVICTION_OVERRIDE_OFFSET,
    FFI_NAMESPACE_DESCRIPTOR_EXPIRATION_OVERRIDE_OFFSET,
    FFI_NAMESPACE_DESCRIPTOR_NAMESPACE_ID_OFFSET, FFI_NAMESPACE_DESCRIPTOR_REVISION_OFFSET,
    FFI_NAMESPACE_DESCRIPTOR_SIZE_BYTES, FFI_NAMESPACE_OVERRIDE_ALLOWED,
    FFI_NAMESPACE_OVERRIDE_DISALLOWED,
};
pub use crate::contract::{
    FfiInputKind, FfiKeySpec, FfiOperation, FfiOperationContract, FfiResultKind, FfiSetCondition,
};
use crate::contract::{
    NAMESPACE_NAME_MAX_BYTES, VALUE_FORMAT_ENCRYPTION_COMPACT, VALUE_FORMAT_ENCRYPTION_NONE,
    VALUE_FORMAT_ENCRYPTION_ROBUST,
};
use crate::key::KeyInput;
use crate::value::{Compression, Encryption, JsonValue, Value, ZstandardOptions};
use crate::{
    Certificate, ClientIdentity, ClientTimeouts, ConnectionState, DataProtectionKey, Endpoint,
    EvictionDefault, ExpirationDefault, GetOutcome, LocalProtectedClient, NamespacePolicy,
    OverridePolicy, PrivateKey, RetryPolicy, ServerTrust, SetCondition, SetOptions, SetOutcome,
};
const COMMAND_QUEUE_CAPACITY: usize = 64;

/// Opaque result allocated by the native ABI.
pub struct FfiResult {
    kind: FfiResultKind,
    status: u32,
    payload: Vec<u8>,
    client: Option<Box<FfiClient>>,
}

/// Sentinel returned by the native result-status accessor when a result was produced locally
/// without a protocol response.
pub const FFI_RESULT_STATUS_NONE: u32 = crate::OPERATION_STATUS_NONE;

/// Native connection options passed by C and C++ bindings.
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
    /// Optional exact 32-byte application data-protection key. Empty selects unprotected values.
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

/// One borrowed field span for the generic ordered-field native call.
///
/// `present == 0` encodes a missing optional field; a present field with
/// `length == 0` is a valid empty value. The call encodes the spans into one
/// owned request body before enqueueing it on the client worker, so the caller
/// may release its buffers once the function returns.
#[repr(C)]
pub struct FfiOperationField {
    pub data: *const u8,
    pub length: usize,
    pub present: u8,
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
        input: Option<FfiOperationInput>,
        value: Vec<u8>,
        set_options: SetOptions,
        raw: bool,
        response: SyncSender<FfiResult>,
    },
    ExecuteUnary {
        operation: Opcode,
        body: Vec<u8>,
        response: SyncSender<FfiResult>,
    },
    ExecuteScoped {
        operation: FfiOperation,
        namespace_id: u64,
        item_id: Vec<u8>,
        value: Vec<u8>,
        set_options: SetOptions,
        response: SyncSender<FfiResult>,
    },
    NamespaceOpen {
        name: Vec<u8>,
        create_if_missing: bool,
        policy: Option<NamespacePolicy>,
        response: SyncSender<FfiResult>,
    },
    NamespaceUpdatePolicy {
        namespace_id: u64,
        expected_revision: u64,
        policy: NamespacePolicy,
        response: SyncSender<FfiResult>,
    },
    NamespaceDelete {
        namespace_id: u64,
        expected_revision: u64,
        response: SyncSender<FfiResult>,
    },
    Shutdown,
}

/// Key material carried by one native operation before it reaches the protected
/// client.
///
/// Logical keys stay inside [`KeyInput`], where canonicalization and key-space
/// validation live. Exact item IDs are a different raw-transport concern and
/// therefore never enter the logical key resolver.
enum FfiOperationInput {
    Logical(KeyInput),
    ExactItemId(Vec<u8>),
}

type CommandSender = crossfire::MTx<crossfire::mpsc::Array<Command>>;
type CommandReceiver = crossfire::Rx<crossfire::mpsc::Array<Command>>;

struct WorkerOptions {
    endpoint: Endpoint,
    certificate: Vec<u8>,
    data_protection_key: Option<DataProtectionKey>,
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
            status: FFI_RESULT_STATUS_NONE,
            payload: message.into().into_bytes(),
            client: None,
        }
    }

    fn success(kind: FfiResultKind, payload: Vec<u8>) -> Self {
        Self {
            kind,
            status: FFI_RESULT_STATUS_NONE,
            payload,
            client: None,
        }
    }

    fn success_with_status(kind: FfiResultKind, status: u32, payload: Vec<u8>) -> Self {
        Self {
            kind,
            status,
            payload,
            client: None,
        }
    }

    fn connected(client: FfiClient) -> Self {
        Self {
            kind: FfiResultKind::Connected,
            status: FFI_RESULT_STATUS_NONE,
            payload: Vec::new(),
            client: Some(Box::new(client)),
        }
    }
}

fn parse_operation(operation: u32) -> std::result::Result<Opcode, String> {
    let operation_byte = u8::try_from(operation)
        .map_err(|_| format!("unsupported protocol operation {operation}"))?;
    Opcode::try_from(operation_byte)
        .map_err(|_| format!("unsupported protocol operation {operation}"))
}

impl FfiClient {
    // The argument list mirrors the stable native connection contract.
    #[allow(clippy::too_many_arguments)]
    fn connect(
        endpoint: Endpoint,
        certificate: Vec<u8>,
        data_protection_key: Option<DataProtectionKey>,
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
        self.execute_with_input(
            operation,
            Some(if raw {
                FfiOperationInput::ExactItemId(application_key)
            } else {
                FfiOperationInput::Logical(KeyInput::canonical_key(application_key))
            }),
            value,
            set_options,
            raw,
        )
    }

    fn execute_typed(
        &self,
        operation: FfiOperation,
        key_spec: FfiKeySpec,
        application_key: Vec<u8>,
        value: Vec<u8>,
        set_options: SetOptions,
        raw: bool,
    ) -> FfiResult {
        self.execute_with_input(
            operation,
            Some(FfiOperationInput::Logical(KeyInput::from_ffi(
                key_spec,
                application_key,
            ))),
            value,
            set_options,
            raw,
        )
    }

    fn execute_with_input(
        &self,
        operation: FfiOperation,
        input: Option<FfiOperationInput>,
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
            input,
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

    fn execute_unary(&self, operation: Opcode, body: Vec<u8>) -> FfiResult {
        self.send_command_with_response(|response| Command::ExecuteUnary {
            operation,
            body,
            response,
        })
    }

    fn execute_scoped(
        &self,
        operation: FfiOperation,
        namespace_id: u64,
        item_id: Vec<u8>,
        value: Vec<u8>,
        set_options: SetOptions,
    ) -> FfiResult {
        self.send_command_with_response(|response| Command::ExecuteScoped {
            operation,
            namespace_id,
            item_id,
            value,
            set_options,
            response,
        })
    }

    fn namespace_open(
        &self,
        name: Vec<u8>,
        create_if_missing: bool,
        policy: Option<NamespacePolicy>,
    ) -> FfiResult {
        self.send_command_with_response(|response| Command::NamespaceOpen {
            name,
            create_if_missing,
            policy,
            response,
        })
    }

    fn namespace_update_policy(
        &self,
        namespace_id: u64,
        expected_revision: u64,
        policy: NamespacePolicy,
    ) -> FfiResult {
        self.send_command_with_response(|response| Command::NamespaceUpdatePolicy {
            namespace_id,
            expected_revision,
            policy,
            response,
        })
    }

    fn namespace_delete(&self, namespace_id: u64, expected_revision: u64) -> FfiResult {
        self.send_command_with_response(|response| Command::NamespaceDelete {
            namespace_id,
            expected_revision,
            response,
        })
    }

    fn send_command_with_response(
        &self,
        build: impl FnOnce(SyncSender<FfiResult>) -> Command,
    ) -> FfiResult {
        let (response, receiver) = sync_channel(1);
        let Some(deadline) = Instant::now().checked_add(self.request_timeout) else {
            return FfiResult::error("client request timeout exceeds the platform clock range");
        };
        let command = build(response);
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
    let protected = data_protection_key.is_some();
    let mut builder = match data_protection_key {
        Some(key) => LocalProtectedClient::builder(endpoint, key),
        None => LocalProtectedClient::builder_unprotected(endpoint),
    }
    .compression(compression)
    .timeouts(timeouts)
    .retry_policy(retry)
    .max_in_flight(max_in_flight);
    if protected {
        builder = builder.encryption(encryption);
    }
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
                input,
                value,
                set_options,
                raw,
                response,
            } => {
                let result = catch_unwind(AssertUnwindSafe(|| {
                    runtime.block_on(execute(&client, operation, input, value, set_options, raw))
                }))
                .unwrap_or_else(|_| FfiResult::error("native client worker panicked"));
                state.store(
                    connection_state_value(client.connection_state()),
                    Ordering::Release,
                );
                let _ = response.send(result);
            }
            Command::ExecuteUnary {
                operation,
                body,
                response,
            } => {
                let result = catch_unwind(AssertUnwindSafe(|| {
                    runtime
                        .block_on(client.raw().execute_unary(operation, body))
                        .and_then(operation_result)
                }))
                .unwrap_or_else(|_| {
                    Err(crate::Error::configuration(
                        "operation",
                        "native client worker panicked",
                    ))
                })
                .unwrap_or_else(|error| FfiResult::error(error.to_string()));
                state.store(
                    connection_state_value(client.connection_state()),
                    Ordering::Release,
                );
                let _ = response.send(result);
            }
            Command::ExecuteScoped {
                operation,
                namespace_id,
                item_id,
                value,
                set_options,
                response,
            } => {
                let result = catch_unwind(AssertUnwindSafe(|| {
                    runtime.block_on(execute_scoped(
                        &client,
                        operation,
                        namespace_id,
                        item_id,
                        value,
                        set_options,
                    ))
                }))
                .unwrap_or_else(|_| {
                    Err(crate::Error::configuration(
                        "operation",
                        "native client worker panicked",
                    ))
                })
                .unwrap_or_else(|error| FfiResult::error(error.to_string()));
                state.store(
                    connection_state_value(client.connection_state()),
                    Ordering::Release,
                );
                let _ = response.send(result);
            }
            Command::NamespaceOpen {
                name,
                create_if_missing,
                policy,
                response,
            } => {
                let result = catch_unwind(AssertUnwindSafe(|| {
                    runtime.block_on(namespace_open(&client, name, create_if_missing, policy))
                }))
                .unwrap_or_else(|_| FfiResult::error("native client worker panicked"));
                state.store(
                    connection_state_value(client.connection_state()),
                    Ordering::Release,
                );
                let _ = response.send(result);
            }
            Command::NamespaceUpdatePolicy {
                namespace_id,
                expected_revision,
                policy,
                response,
            } => {
                let result = catch_unwind(AssertUnwindSafe(|| {
                    runtime.block_on(namespace_update_policy(
                        &client,
                        namespace_id,
                        expected_revision,
                        policy,
                    ))
                }))
                .unwrap_or_else(|_| FfiResult::error("native client worker panicked"));
                state.store(
                    connection_state_value(client.connection_state()),
                    Ordering::Release,
                );
                let _ = response.send(result);
            }
            Command::NamespaceDelete {
                namespace_id,
                expected_revision,
                response,
            } => {
                let result = catch_unwind(AssertUnwindSafe(|| {
                    runtime.block_on(namespace_delete(&client, namespace_id, expected_revision))
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
    input: Option<FfiOperationInput>,
    value: Vec<u8>,
    set_options: SetOptions,
    raw: bool,
) -> FfiResult {
    let result = if raw {
        match input {
            Some(FfiOperationInput::ExactItemId(item_id)) => {
                execute_raw(client, operation, item_id, value, set_options).await
            }
            Some(FfiOperationInput::Logical(_)) => Err(crate::Error::configuration(
                "item_id",
                "exact-item-ID operations require exact item-ID bytes",
            )),
            None => execute_raw(client, operation, Vec::new(), value, set_options).await,
        }
    } else if let Some(opcode) = protocol_global_opcode(operation) {
        execute_protocol_global(client, opcode, value).await
    } else {
        match input {
            Some(FfiOperationInput::Logical(key)) => {
                execute_protected(client, operation, key, value, set_options).await
            }
            Some(FfiOperationInput::ExactItemId(_)) => Err(crate::Error::configuration(
                "application_key",
                "protected operations require a logical application key",
            )),
            None => Err(crate::Error::configuration(
                "application_key",
                "protected operation requires a key",
            )),
        }
    };
    result.unwrap_or_else(|error| FfiResult::error(error.to_string()))
}

async fn execute_protected(
    client: &LocalProtectedClient,
    operation: FfiOperation,
    input: KeyInput,
    value: Vec<u8>,
    set_options: SetOptions,
) -> std::result::Result<FfiResult, crate::Error> {
    if operation == FfiOperation::Reconnect {
        return client.reconnect().await.map(|()| ok_result());
    }
    if operation == FfiOperation::GetJson {
        return client
            .get_value_key_input(input)
            .await
            .and_then(json_result);
    }
    if operation == FfiOperation::SetJson {
        return match parse_json(&value) {
            Ok(json) => client
                .set_value_key_input(input, Value::Json(json), set_options)
                .await
                .map(set_result),
            Err(error) => Err(crate::value::Error::InvalidJson(error).into()),
        };
    }
    execute_protected_data_plane(client, operation, input, value, set_options).await
}

async fn execute_protocol_global(
    client: &LocalProtectedClient,
    opcode: Opcode,
    value: Vec<u8>,
) -> std::result::Result<FfiResult, crate::Error> {
    client
        .raw()
        .execute_unary(opcode, value)
        .await
        .and_then(operation_result)
}

fn protocol_global_opcode(operation: FfiOperation) -> Option<Opcode> {
    let opcode = crate::contract::protocol_opcode(operation)?;
    openkache_protocol::compat_v1::request_projection(opcode)
        .is_none()
        .then_some(opcode)
}

async fn execute_protected_data_plane(
    client: &LocalProtectedClient,
    operation: FfiOperation,
    input: KeyInput,
    value: Vec<u8>,
    set_options: SetOptions,
) -> std::result::Result<FfiResult, crate::Error> {
    let Some(opcode) = crate::contract::protocol_opcode(operation) else {
        return Err(crate::Error::configuration(
            "operation",
            "operation is not available through the protected ABI",
        ));
    };
    if crate::protocol::uses_compact_item_route(opcode) {
        client
            .execute_operation_key_input(opcode, input, value, set_options)
            .await
            .and_then(operation_result)
    } else {
        Err(crate::Error::configuration(
            "operation",
            "protocol operation is not available through the protected ABI",
        ))
    }
}

async fn execute_raw(
    client: &LocalProtectedClient,
    operation: FfiOperation,
    item_id: Vec<u8>,
    value: Vec<u8>,
    set_options: SetOptions,
) -> std::result::Result<FfiResult, crate::Error> {
    if let Some(opcode) = protocol_global_opcode(operation) {
        return execute_protocol_global(client, opcode, value).await;
    }
    if operation == FfiOperation::Reconnect {
        return client.raw().reconnect().await.map(|()| ok_result());
    }
    let Some(opcode) = crate::contract::protocol_opcode(operation) else {
        return Err(crate::Error::configuration(
            "operation",
            "operation is not available through the exact item-ID ABI",
        ));
    };
    client
        .raw()
        .execute_raw(opcode, item_id, value, set_options)
        .await
        .and_then(operation_result)
}

async fn execute_scoped(
    client: &LocalProtectedClient,
    operation: FfiOperation,
    namespace_id: u64,
    item_id: Vec<u8>,
    value: Vec<u8>,
    set_options: SetOptions,
) -> std::result::Result<FfiResult, crate::Error> {
    let Some(opcode) = crate::contract::protocol_opcode(operation) else {
        return Err(crate::Error::configuration(
            "operation",
            "operation is not available through the namespace-scoped exact-ID ABI",
        ));
    };
    client
        .raw()
        .execute_scoped(opcode, namespace_id, item_id, value, set_options)
        .await
        .and_then(operation_result)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FfiInvocation {
    Protected,
    Raw,
    Scoped,
}

fn validate_ffi_operation(
    operation_contract: FfiOperationContract,
    invocation: FfiInvocation,
    typed_key: bool,
    input: &[u8],
    value: &[u8],
    set_options: &SetOptions,
) -> std::result::Result<(), String> {
    if operation_contract.dedicated_abi {
        return Err("namespace management uses dedicated native ABI calls".to_owned());
    }
    let supported = match invocation {
        FfiInvocation::Protected => operation_contract.supports_protected,
        FfiInvocation::Raw => operation_contract.supports_raw,
        FfiInvocation::Scoped => operation_contract.supports_scoped,
    };
    if !supported {
        return Err(match invocation {
            FfiInvocation::Protected => {
                "operation is not available through the protected ABI".to_owned()
            }
            FfiInvocation::Raw => {
                "operation is not available through the exact item-ID ABI".to_owned()
            }
            FfiInvocation::Scoped => {
                "operation is not available through the namespace-scoped exact-ID ABI".to_owned()
            }
        });
    }
    match (invocation, operation_contract.input_kind) {
        (FfiInvocation::Protected, FfiInputKind::None) if !input.is_empty() => {
            return Err("operation does not accept an application key".to_owned());
        }
        (FfiInvocation::Protected, FfiInputKind::ApplicationKey)
        | (FfiInvocation::Protected, FfiInputKind::ItemId)
            if input.is_empty() && !typed_key =>
        {
            return Err("application key must not be empty".to_owned());
        }
        (FfiInvocation::Raw, FfiInputKind::None) if !input.is_empty() => {
            return Err("operation does not accept an item_id".to_owned());
        }
        (FfiInvocation::Raw, FfiInputKind::ItemId)
        | (FfiInvocation::Scoped, FfiInputKind::ItemId)
            if input.len()
                != crate::ITEM_ID_BYTES * operation_contract.request_item_count as usize =>
        {
            return Err(format!(
                "item_id must contain exactly {} bytes for {} item IDs, got {}",
                crate::ITEM_ID_BYTES * operation_contract.request_item_count as usize,
                operation_contract.request_item_count,
                input.len()
            ));
        }
        (FfiInvocation::Scoped, FfiInputKind::None) if !input.is_empty() => {
            return Err("operation does not accept an item_id".to_owned());
        }
        _ => {}
    }
    if !operation_contract.accepts_value && !value.is_empty() {
        return Err("operation does not accept a value".to_owned());
    }
    if !operation_contract.accepts_set_options
        && (set_options.condition() != SetCondition::Any
            || set_options.time_to_live_millis().is_some())
    {
        return Err("SET options require a SET operation".to_owned());
    }
    Ok(())
}

async fn namespace_open(
    client: &LocalProtectedClient,
    name: Vec<u8>,
    create_if_missing: bool,
    policy: Option<NamespacePolicy>,
) -> FfiResult {
    match client
        .raw()
        .namespace_open_with_outcome(name, create_if_missing, policy)
        .await
    {
        Ok((descriptor, created)) => match ffi_namespace_descriptor(descriptor) {
            Ok(payload) => FfiResult::success_with_status(
                if created {
                    FfiResultKind::Created
                } else {
                    FfiResultKind::Ok
                },
                u32::from(if created {
                    openkache_protocol::Status::Created
                } else {
                    openkache_protocol::Status::Ok
                } as u8),
                payload,
            ),
            Err(error) => FfiResult::error(error.to_string()),
        },
        Err(error) => FfiResult::error(error.to_string()),
    }
}

async fn namespace_update_policy(
    client: &LocalProtectedClient,
    namespace_id: u64,
    expected_revision: u64,
    policy: NamespacePolicy,
) -> FfiResult {
    match client
        .raw()
        .namespace_update_policy(namespace_id, expected_revision, policy)
        .await
    {
        Ok(descriptor) => match ffi_namespace_descriptor(descriptor) {
            Ok(payload) => FfiResult::success_with_status(
                FfiResultKind::Value,
                u32::from(openkache_protocol::Status::Ok as u8),
                payload,
            ),
            Err(error) => FfiResult::error(error.to_string()),
        },
        Err(error) => FfiResult::error(error.to_string()),
    }
}

async fn namespace_delete(
    client: &LocalProtectedClient,
    namespace_id: u64,
    expected_revision: u64,
) -> FfiResult {
    match client
        .raw()
        .namespace_delete(namespace_id, expected_revision)
        .await
    {
        Ok(()) => FfiResult::success_with_status(
            FfiResultKind::Ok,
            u32::from(openkache_protocol::Status::Deleted as u8),
            Vec::new(),
        ),
        Err(error) => FfiResult::error(error.to_string()),
    }
}

fn set_options_from_flags(flags: u8, ttl_ms: u64) -> std::result::Result<SetOptions, String> {
    let ttl_ms = (ttl_ms != 0).then_some(ttl_ms);
    crate::protocol::SetWireOptions::from_wire_parts(flags, ttl_ms)
        .map_err(|error| error.to_string())
        .and_then(|options| SetOptions::from_protocol(options).map_err(|error| error.to_string()))
}

fn namespace_policy_from_flags(
    flags: u8,
    ttl_ms: u64,
) -> std::result::Result<NamespacePolicy, String> {
    let ttl_ms = (ttl_ms != 0).then_some(ttl_ms);
    NamespacePolicy::from_wire_parts(flags, ttl_ms).map_err(|error| error.to_string())
}

fn ffi_namespace_descriptor(
    descriptor: crate::NamespaceDescriptor,
) -> std::result::Result<Vec<u8>, String> {
    descriptor
        .encode()
        .map_err(|error| format!("namespace descriptor encoding failed: {error}"))
}

fn connection_state_value(state: ConnectionState) -> u32 {
    state.code()
}

fn ok_result() -> FfiResult {
    FfiResult::success(FfiResultKind::Ok, Vec::new())
}

fn operation_result(
    result: crate::OperationResult,
) -> std::result::Result<FfiResult, crate::Error> {
    let kind = FfiResultKind::try_from(result.kind).map_err(|kind| {
        crate::Error::configuration(
            "operation result",
            format!("unsupported native result kind {kind}"),
        )
    })?;
    Ok(FfiResult::success_with_status(
        kind,
        result.status,
        result.payload,
    ))
}

fn set_result(outcome: SetOutcome) -> FfiResult {
    let (status, kind) = match outcome {
        SetOutcome::Created => (openkache_protocol::Status::Created, FfiResultKind::Created),
        SetOutcome::Replaced => (
            openkache_protocol::Status::Replaced,
            FfiResultKind::Replaced,
        ),
        SetOutcome::NotStored => (
            openkache_protocol::Status::NotStored,
            FfiResultKind::NotStored,
        ),
    };
    FfiResult::success_with_status(kind, u32::from(status as u8), Vec::new())
}

fn json_result(outcome: GetOutcome<Value>) -> std::result::Result<FfiResult, crate::Error> {
    match outcome {
        GetOutcome::Found(Value::Json(value)) => serde_json_canonicalizer::to_vec(&value)
            .map(|payload| {
                FfiResult::success_with_status(
                    FfiResultKind::Value,
                    u32::from(openkache_protocol::Status::Ok as u8),
                    payload,
                )
            })
            .map_err(|error| crate::value::Error::InvalidJson(error.to_string()).into()),
        GetOutcome::Found(Value::Raw(_)) => Err(crate::value::Error::ExpectedRawValue.into()),
        GetOutcome::NotFound => Ok(FfiResult::success_with_status(
            FfiResultKind::NotFound,
            u32::from(openkache_protocol::Status::NotFound as u8),
            Vec::new(),
        )),
    }
}

fn parse_json(bytes: &[u8]) -> std::result::Result<JsonValue, String> {
    crate::value::parse_json_input(bytes).map_err(|error| error.to_string())
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
/// PEM chain, or empty to use system trust roots. An empty data-protection key
/// selects the unprotected formatted-value profile; a non-empty key must be
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

/// Connects a native client with the complete shared-core configuration.
///
/// Zero retry and lane limits select shared-core defaults. The Smithy None and
/// Robust encryption values select Robust; Compact selects Compact.
///
/// # Safety
///
/// Every non-empty pointer/length pair must identify readable memory for the duration of this
/// call. `data_protection_key` must contain exactly 32 bytes.
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
/// The structure is copied before this function returns.
///
/// # Safety
///
/// `options` must be either null or a valid, properly aligned pointer to an initialized
/// [`FfiConnectOptions`] for the duration of this call. Every non-empty pointer/length pair in a
/// non-null options structure must identify readable memory for the duration of this call.
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
/// This compatibility entry point accepts one canonical v1 key item.
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

/// Executes a generic operation from an already encoded request body.
///
/// This ABI is intentionally independent from application keys, item IDs,
/// namespace scope, and SET options. The operation's generated wire contract
/// validates the body and selects empty, opaque, or ordered-field framing.
/// Compact protocol-v1 operations are rejected here and remain available
/// through the compatibility entry points below.
///
/// # Safety
///
/// `client` must be a live pointer returned by
/// [`openkache_client_result_take_client`]. `body` must identify readable
/// memory for `body_length` bytes (unless the length is zero), and the client
/// must not be freed until the call returns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_client_execute_unary(
    client: *const FfiClient,
    operation: u32,
    body: *const u8,
    body_length: usize,
) -> *mut FfiResult {
    boxed_result(catch_result(|| {
        let client = unsafe {
            client
                .as_ref()
                .ok_or_else(|| "client pointer must not be null".to_owned())?
        };
        let operation = parse_operation(operation)?;
        let body = copy_bytes(body, body_length, "body")?;
        if openkache_protocol::compat_v1::request_projection(operation).is_some() {
            return Err("compact protocol-v1 operations require a compatibility ABI".to_owned());
        }
        Ok(client.execute_unary(operation, body))
    }))
}

/// Executes a generic ordered-field operation from borrowed field spans.
///
/// `present == 0` represents a missing optional field. A present field with a
/// zero length is valid and is not treated as missing. The generated operation
/// contract validates field count, requiredness, and codecs before sending.
///
/// # Safety
///
/// `client` must be a live pointer returned by
/// [`openkache_client_result_take_client`]. When `field_count` is non-zero,
/// `fields` must point to an array of [`FfiOperationField`] values that remain
/// readable for the duration of this call, and every present span must point
/// to readable memory for its declared length.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_client_execute_fields(
    client: *const FfiClient,
    operation: u32,
    fields: *const FfiOperationField,
    field_count: usize,
) -> *mut FfiResult {
    boxed_result(catch_result(|| {
        let client = unsafe {
            client
                .as_ref()
                .ok_or_else(|| "client pointer must not be null".to_owned())?
        };
        let operation = parse_operation(operation)?;
        if !matches!(
            crate::contract::operation_wire_spec(operation)
                .request
                .framing,
            crate::contract::OperationLayoutFraming::OrderedFields
                | crate::contract::OperationLayoutFraming::FieldSequence
        ) {
            return Err("operation does not use ordered-field request framing".to_owned());
        }
        if field_count > crate::contract::MAX_OPERATION_REQUEST_FIELDS
            || field_count > crate::contract::MAX_OPERATION_FIELDS
        {
            return Err("field count exceeds the generated operation bound".to_owned());
        }
        let fields = if field_count == 0 {
            &[][..]
        } else if fields.is_null() {
            return Err("fields pointer must not be null for a non-empty field list".to_owned());
        } else {
            unsafe { std::slice::from_raw_parts(fields, field_count) }
        };
        let mut borrowed = [None; crate::contract::MAX_OPERATION_FIELDS];
        for (index, field) in fields.iter().enumerate() {
            match field.present {
                0 => {
                    if field.length != 0 {
                        return Err("missing field must have zero length".to_owned());
                    }
                }
                1 => {
                    let value = if field.length == 0 {
                        &[][..]
                    } else {
                        if field.data.is_null() {
                            return Err(format!(
                                "field pointer is null for {} bytes",
                                field.length
                            ));
                        }
                        unsafe { std::slice::from_raw_parts(field.data, field.length) }
                    };
                    borrowed[index] = Some(value);
                }
                _ => return Err("field presence must be zero or one".to_owned()),
            }
        }
        let contract = crate::contract::operation_wire_spec(operation);
        let body = openkache_protocol::encode_planned_fields(
            &borrowed[..fields.len()],
            contract.request.fields,
            contract.request.layout,
        )
        .map_err(|error| error.to_string())?;
        Ok(client.execute_unary(operation, body))
    }))
}

/// Executes one protected operation from a logical key and a generated key specification.
///
/// `key_spec` selects Text, Bytes, or Integer. Text and Bytes receive their
/// exact UTF-8/byte payload; Integer receives canonical signed decimal UTF-8.
/// The shared Rust core performs PortableKey conversion, deterministic CBOR,
/// namespace-bound Item ID derivation, and value protection in one worker
/// operation. Global operations ignore the key input and require an empty
/// value when their operation contract says so.
///
/// # Safety
///
/// `client` must be a live pointer returned by [`openkache_client_result_take_client`]. Every
/// non-empty input pointer must identify readable memory for this call, and the client must not
/// be freed until this call returns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_client_execute_typed(
    client: *const FfiClient,
    operation: u32,
    key_spec: u32,
    application_key: *const u8,
    application_key_length: usize,
    value: *const u8,
    value_length: usize,
    set_condition: u32,
    ttl_enabled: u8,
    ttl_ms: u64,
) -> *mut FfiResult {
    let key_spec = match parse_ffi_key_spec(key_spec) {
        Ok(key_spec) => key_spec,
        Err(error) => return boxed_result(FfiResult::error(error)),
    };
    execute_entry_inner(
        client,
        operation,
        application_key,
        application_key_length,
        value,
        value_length,
        false,
        Some(key_spec),
        None,
        set_condition,
        ttl_enabled,
        ttl_ms,
    )
}

/// Executes one exact-item-ID operation without application-key derivation or
/// value protection.
///
/// `GET`, `SET`, and `DELETE` use the exact item ID supplied by the caller.
///
/// # Safety
///
/// `client` must be a live pointer returned by [`openkache_client_result_take_client`].
/// Every non-empty item-ID/value pointer pair must identify readable memory for this call, and
/// the client must not be freed until this call returns.
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

/// Executes one protected operation with the complete wire SET policy byte.
///
/// This entry point preserves application-key derivation and value protection while allowing
/// callers to select expiration and eviction policy in addition to the existence condition.
///
/// # Safety
///
/// `client` must be a live pointer returned by [`openkache_client_result_take_client`]. Every
/// non-empty application-key/value pointer pair must identify readable memory for this call, and
/// the client must not be freed until the call returns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_client_execute_with_options(
    client: *const FfiClient,
    operation: u32,
    application_key: *const u8,
    application_key_length: usize,
    value: *const u8,
    value_length: usize,
    set_flags: u8,
    ttl_ms: u64,
) -> *mut FfiResult {
    execute_entry_with_flags(
        client,
        operation,
        application_key,
        application_key_length,
        value,
        value_length,
        set_flags,
        ttl_ms,
        false,
    )
}

/// Executes one typed protected operation with the complete wire SET policy byte.
///
/// This is the options-bearing counterpart to [`openkache_client_execute_typed`].
///
/// # Safety
///
/// `client` must be a live pointer returned by [`openkache_client_result_take_client`]. Every
/// non-empty input pointer must identify readable memory for this call, and the client must not
/// be freed until this call returns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_client_execute_typed_with_options(
    client: *const FfiClient,
    operation: u32,
    key_spec: u32,
    application_key: *const u8,
    application_key_length: usize,
    value: *const u8,
    value_length: usize,
    set_flags: u8,
    ttl_ms: u64,
) -> *mut FfiResult {
    let key_spec = match parse_ffi_key_spec(key_spec) {
        Ok(key_spec) => key_spec,
        Err(error) => return boxed_result(FfiResult::error(error)),
    };
    execute_entry_inner(
        client,
        operation,
        application_key,
        application_key_length,
        value,
        value_length,
        false,
        Some(key_spec),
        Some((set_flags, ttl_ms)),
        FfiSetCondition::Any as u32,
        0,
        0,
    )
}

/// Executes one exact-item-ID operation with the complete wire SET policy byte.
///
/// # Safety
///
/// `client` must be a live pointer returned by [`openkache_client_result_take_client`]. Every
/// non-empty item-ID/value pointer pair must identify readable memory for this call, and the
/// client must not be freed until the call returns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_client_execute_raw_with_options(
    client: *const FfiClient,
    operation: u32,
    item_id: *const u8,
    item_id_length: usize,
    value: *const u8,
    value_length: usize,
    set_flags: u8,
    ttl_ms: u64,
) -> *mut FfiResult {
    execute_entry_with_flags(
        client,
        operation,
        item_id,
        item_id_length,
        value,
        value_length,
        set_flags,
        ttl_ms,
        true,
    )
}

/// Executes one exact-item-ID operation in an explicitly supplied namespace.
///
/// `set_flags` is the complete wire SET flag byte. It is ignored for operations other than SET,
/// which must pass zero for both `set_flags` and `ttl_ms`.
///
/// # Safety
///
/// `client` must be a live pointer returned by [`openkache_client_result_take_client`]. Every
/// non-empty item-ID/value pointer pair must identify readable memory for this call, and the
/// client must not be freed until the call returns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_client_execute_scoped(
    client: *const FfiClient,
    operation: u32,
    namespace_id: u64,
    item_id: *const u8,
    item_id_length: usize,
    value: *const u8,
    value_length: usize,
    set_flags: u8,
    ttl_ms: u64,
) -> *mut FfiResult {
    boxed_result(catch_result(|| {
        let client = unsafe {
            client
                .as_ref()
                .ok_or_else(|| "client pointer must not be null".to_owned())?
        };
        if namespace_id == 0 {
            return Err("namespace_id must be a positive server-assigned ID".to_owned());
        }
        let item_id = copy_bytes(item_id, item_id_length, "item_id")?;
        let value = copy_bytes(value, value_length, "value")?;
        let operation = FfiOperation::try_from(operation)
            .map_err(|operation| format!("unsupported operation {operation}"))?;
        let operation_contract = crate::contract::ffi_operation_contract(operation);
        let set_options = if operation_contract.accepts_set_options {
            set_options_from_flags(set_flags, ttl_ms)?
        } else {
            if set_flags != 0 || ttl_ms != 0 {
                return Err("SET flags and TTL require a SET operation".to_owned());
            }
            SetOptions::new()
        };
        validate_ffi_operation(
            operation_contract,
            FfiInvocation::Scoped,
            false,
            &item_id,
            &value,
            &set_options,
        )?;
        Ok(client.execute_scoped(operation, namespace_id, item_id, value, set_options))
    }))
}

/// Opens or resolves a namespace and returns its encoded descriptor payload.
///
/// The result kind is `Created` when a namespace was created and `Ok` when it already existed.
/// `policy_flags` and `ttl_ms` are used only when `create_if_missing` is non-zero.
///
/// # Safety
///
/// `client` must be a live pointer returned by [`openkache_client_result_take_client`].
/// Every non-empty name pointer must identify readable memory for this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_client_namespace_open(
    client: *const FfiClient,
    name: *const u8,
    name_length: usize,
    create_if_missing: u8,
    policy_flags: u8,
    ttl_ms: u64,
) -> *mut FfiResult {
    boxed_result(catch_result(|| {
        let client = unsafe {
            client
                .as_ref()
                .ok_or_else(|| "client pointer must not be null".to_owned())?
        };
        let name = copy_bytes(name, name_length, "namespace name")?;
        if name.len() > NAMESPACE_NAME_MAX_BYTES {
            return Err(format!(
                "namespace name exceeds {} octets",
                NAMESPACE_NAME_MAX_BYTES
            ));
        }
        let create_if_missing = create_if_missing != 0;
        let policy = if create_if_missing {
            Some(namespace_policy_from_flags(policy_flags, ttl_ms)?)
        } else {
            if policy_flags != 0 || ttl_ms != 0 {
                return Err("namespace policy requires create_if_missing".to_owned());
            }
            None
        };
        Ok(client.namespace_open(name, create_if_missing, policy))
    }))
}

/// Replaces a namespace policy using its optimistic revision.
///
/// A successful result has kind `Value` and contains the encoded namespace descriptor.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_client_namespace_update_policy(
    client: *const FfiClient,
    namespace_id: u64,
    expected_revision: u64,
    policy_flags: u8,
    ttl_ms: u64,
) -> *mut FfiResult {
    boxed_result(catch_result(|| {
        let client = unsafe {
            client
                .as_ref()
                .ok_or_else(|| "client pointer must not be null".to_owned())?
        };
        let policy = namespace_policy_from_flags(policy_flags, ttl_ms)?;
        Ok(client.namespace_update_policy(namespace_id, expected_revision, policy))
    }))
}

/// Deletes an empty namespace using its optimistic revision.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_client_namespace_delete(
    client: *const FfiClient,
    namespace_id: u64,
    expected_revision: u64,
) -> *mut FfiResult {
    boxed_result(catch_result(|| {
        let client = unsafe {
            client
                .as_ref()
                .ok_or_else(|| "client pointer must not be null".to_owned())?
        };
        Ok(client.namespace_delete(namespace_id, expected_revision))
    }))
}

/// Decodes one complete namespace descriptor payload using the canonical
/// protocol implementation.
///
/// Returns [`FFI_NAMESPACE_DESCRIPTOR_DECODE_OK`] on success and
/// [`FFI_NAMESPACE_DESCRIPTOR_DECODE_INVALID`] when the output pointer is
/// invalid or the payload is not a valid descriptor.
///
/// # Safety
///
/// `payload` must be readable for `payload_length` bytes (unless the length is
/// zero), and `output` must point to writable storage for one
/// [`FfiNamespaceDescriptor`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_client_namespace_descriptor_decode(
    payload: *const u8,
    payload_length: usize,
    output: *mut FfiNamespaceDescriptor,
) -> u32 {
    if output.is_null() {
        return FFI_NAMESPACE_DESCRIPTOR_DECODE_INVALID;
    }
    let payload = if payload_length == 0 {
        &[][..]
    } else if payload.is_null() {
        return FFI_NAMESPACE_DESCRIPTOR_DECODE_INVALID;
    } else {
        unsafe { std::slice::from_raw_parts(payload, payload_length) }
    };
    let Ok(descriptor) = crate::protocol::NamespaceDescriptor::decode(payload) else {
        return FFI_NAMESPACE_DESCRIPTOR_DECODE_INVALID;
    };
    let (default_expiration, default_ttl_ms) = match descriptor.policy.default_expiration {
        ExpirationDefault::NoExpiry => (FFI_NAMESPACE_DEFAULT_EXPIRATION_NO_EXPIRY, 0),
        ExpirationDefault::FixedTtl { ttl_ms } => {
            (FFI_NAMESPACE_DEFAULT_EXPIRATION_FIXED_TTL, ttl_ms)
        }
    };
    let decoded = FfiNamespaceDescriptor {
        namespace_id: descriptor.namespace_id,
        revision: descriptor.revision,
        default_ttl_ms,
        default_expiration,
        expiration_override: if descriptor.policy.expiration_override == OverridePolicy::Allowed {
            FFI_NAMESPACE_OVERRIDE_ALLOWED
        } else {
            FFI_NAMESPACE_OVERRIDE_DISALLOWED
        },
        default_eviction: if descriptor.policy.default_eviction
            == EvictionDefault::EvictionProtected
        {
            FFI_NAMESPACE_DEFAULT_EVICTION_PROTECTED
        } else {
            FFI_NAMESPACE_DEFAULT_EVICTION_EVICTABLE
        },
        eviction_override: if descriptor.policy.eviction_override == OverridePolicy::Allowed {
            FFI_NAMESPACE_OVERRIDE_ALLOWED
        } else {
            FFI_NAMESPACE_OVERRIDE_DISALLOWED
        },
    };
    unsafe { output.write(decoded) };
    FFI_NAMESPACE_DESCRIPTOR_DECODE_OK
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
    execute_entry_inner(
        client,
        operation,
        application_key,
        application_key_length,
        value,
        value_length,
        raw,
        None,
        None,
        set_condition,
        ttl_enabled,
        ttl_ms,
    )
}

fn execute_entry_with_flags(
    client: *const FfiClient,
    operation: u32,
    application_key: *const u8,
    application_key_length: usize,
    value: *const u8,
    value_length: usize,
    set_flags: u8,
    ttl_ms: u64,
    raw: bool,
) -> *mut FfiResult {
    execute_entry_inner(
        client,
        operation,
        application_key,
        application_key_length,
        value,
        value_length,
        raw,
        None,
        Some((set_flags, ttl_ms)),
        FfiSetCondition::Any as u32,
        0,
        0,
    )
}

#[allow(clippy::too_many_arguments)]
fn execute_entry_inner(
    client: *const FfiClient,
    operation: u32,
    application_key: *const u8,
    application_key_length: usize,
    value: *const u8,
    value_length: usize,
    raw: bool,
    key_spec: Option<FfiKeySpec>,
    complete_flags: Option<(u8, u64)>,
    set_condition: u32,
    ttl_enabled: u8,
    ttl_ms: u64,
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
        let operation_contract = crate::contract::ffi_operation_contract(operation);
        let set_options = if let Some((flags, ttl_ms)) = complete_flags {
            if operation_contract.accepts_set_options {
                set_options_from_flags(flags, ttl_ms)?
            } else {
                if flags != 0 || ttl_ms != 0 {
                    return Err("SET flags and TTL require a SET operation".to_owned());
                }
                SetOptions::new()
            }
        } else {
            let condition = match FfiSetCondition::try_from(set_condition)
                .map_err(|condition| format!("unsupported SET condition {condition}"))?
            {
                FfiSetCondition::Any => SetCondition::Any,
                FfiSetCondition::IfAbsent => SetCondition::IfAbsent,
                FfiSetCondition::IfPresent => SetCondition::IfPresent,
            };
            let mut set_options = match condition {
                SetCondition::Any => SetOptions::new(),
                SetCondition::IfAbsent => SetOptions::new().if_absent(),
                SetCondition::IfPresent => SetOptions::new().if_present(),
            };
            if ttl_enabled != 0 {
                if ttl_ms == 0 {
                    return Err("SET TTL must be greater than zero milliseconds".to_owned());
                }
                set_options = set_options.expires_after_millis(ttl_ms);
            }
            set_options
        };
        validate_ffi_operation(
            operation_contract,
            if raw {
                FfiInvocation::Raw
            } else {
                FfiInvocation::Protected
            },
            key_spec.is_some(),
            &application_key,
            &value,
            &set_options,
        )?;
        Ok(match key_spec {
            Some(key_spec) => client.execute_typed(
                operation,
                key_spec,
                application_key,
                value,
                set_options,
                raw,
            ),
            None => client.execute(operation, application_key, value, set_options, raw),
        })
    }))
}

fn parse_ffi_key_spec(value: u32) -> std::result::Result<FfiKeySpec, String> {
    FfiKeySpec::try_from(value).map_err(|value| format!("unsupported key spec {value}"))
}

/// Returns a best-effort connection-state discriminator:
/// `0` connected, `1` reconnecting, `2` disconnected, `3` closed, and `4`
/// for a null handle.
///
/// # Safety
///
/// If `client` is non-null, it must be a live pointer returned by
/// [`openkache_client_result_take_client`] and remain valid until this call returns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_client_connection_state(client: *const FfiClient) -> u32 {
    unsafe { client.as_ref() }.map_or(ConnectionState::Unknown.code(), FfiClient::connection_state)
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
    unsafe { result.as_ref() }.map_or(FfiResultKind::Error.code(), |result| result.kind.code())
}

/// Returns the protocol status discriminator carried by an FFI result.
///
/// The value is the wire status byte widened to `u32`.  Results created locally, such as
/// connection errors and reconnect acknowledgements, return [`FFI_RESULT_STATUS_NONE`].
///
/// # Safety
///
/// `result` must be null or a live pointer returned by this library.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_client_result_status(result: *const FfiResult) -> u32 {
    unsafe { result.as_ref() }.map_or(FFI_RESULT_STATUS_NONE, |result| result.status)
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
) -> std::result::Result<Option<DataProtectionKey>, String> {
    if length == 0 {
        return Ok(None);
    }
    if pointer.is_null() {
        return Err(format!(
            "data protection key pointer is null for {length} bytes"
        ));
    }
    let bytes = unsafe { std::slice::from_raw_parts(pointer, length) };
    DataProtectionKey::from_slice(bytes)
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
