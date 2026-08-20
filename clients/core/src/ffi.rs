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

use openkache_value::{Value as StructuredValue, decode, encode};

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
pub use crate::contract::{FfiOperation, FfiResultKind, FfiSetCondition};
use crate::contract::{
    VALUE_FORMAT_ENCRYPTION_COMPACT, VALUE_FORMAT_ENCRYPTION_NONE, VALUE_FORMAT_ENCRYPTION_ROBUST,
};
use crate::value::{Compression, Encryption, JsonValue, Value, ZstandardOptions};
use crate::{
    Certificate, ClientIdentity, ClientTimeouts, ConnectionState, DataProtectionKey, DeleteOutcome,
    Endpoint, EvictionDefault, ExpirationDefault, GetOutcome, ItemId, ItemValue,
    LocalProtectedClient, NamespacePolicy, OverridePolicy, PrivateKey, RetryPolicy, ServerTrust,
    SetCondition, SetOptions, SetOutcome,
};
const COMMAND_QUEUE_CAPACITY: usize = 64;

/// Opaque result allocated by the native ABI.
pub struct FfiResult {
    kind: FfiResultKind,
    payload: Vec<u8>,
    client: Option<Box<FfiClient>>,
}

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
            payload: message.into().into_bytes(),
            client: None,
        }
    }

    fn cancelled(message: impl Into<String>) -> Self {
        Self {
            kind: FfiResultKind::Cancelled,
            payload: message.into().into_bytes(),
            client: None,
        }
    }

    fn from_error(error: crate::Error) -> Self {
        let kind = match error {
            crate::Error::AmbiguousOutcome { .. } => FfiResultKind::UnknownMutation,
            crate::Error::Timeout { .. } => FfiResultKind::Cancelled,
            _ => FfiResultKind::Error,
        };
        Self {
            kind,
            payload: error.to_string().into_bytes(),
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
            return FfiResult::cancelled(format!("client worker queue deadline exceeded: {error}"));
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        receiver.recv_timeout(remaining).unwrap_or_else(|error| {
            if matches!(
                operation,
                FfiOperation::Set
                    | FfiOperation::SetJson
                    | FfiOperation::SetStructured
                    | FfiOperation::Delete
            ) {
                FfiResult {
                    kind: FfiResultKind::UnknownMutation,
                    payload: format!(
                        "client mutation outcome is unknown after cancellation: {error}"
                    )
                    .into_bytes(),
                    client: None,
                }
            } else {
                FfiResult::cancelled(format!("client operation timed out: {error}"))
            }
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
        self.send_command_with_response(Some(operation), |response| Command::ExecuteScoped {
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
        self.send_command_with_response(
            create_if_missing.then_some(FfiOperation::NamespaceOpen),
            |response| Command::NamespaceOpen {
                name,
                create_if_missing,
                policy,
                response,
            },
        )
    }

    fn namespace_update_policy(
        &self,
        namespace_id: u64,
        expected_revision: u64,
        policy: NamespacePolicy,
    ) -> FfiResult {
        self.send_command_with_response(Some(FfiOperation::NamespaceUpdatePolicy), |response| {
            Command::NamespaceUpdatePolicy {
                namespace_id,
                expected_revision,
                policy,
                response,
            }
        })
    }

    fn namespace_delete(&self, namespace_id: u64, expected_revision: u64) -> FfiResult {
        self.send_command_with_response(Some(FfiOperation::NamespaceDelete), |response| {
            Command::NamespaceDelete {
                namespace_id,
                expected_revision,
                response,
            }
        })
    }

    fn send_command_with_response(
        &self,
        operation: Option<FfiOperation>,
        build: impl FnOnce(SyncSender<FfiResult>) -> Command,
    ) -> FfiResult {
        let (response, receiver) = sync_channel(1);
        let Some(deadline) = Instant::now().checked_add(self.request_timeout) else {
            return FfiResult::error("client request timeout exceeds the platform clock range");
        };
        let command = build(response);
        let remaining = deadline.saturating_duration_since(Instant::now());
        if let Err(error) = self.commands.send_timeout(command, remaining) {
            return FfiResult::cancelled(format!("client worker queue deadline exceeded: {error}"));
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        receiver.recv_timeout(remaining).unwrap_or_else(|error| {
            if operation.is_some_and(|operation| {
                matches!(
                    operation,
                    FfiOperation::Set
                        | FfiOperation::SetJson
                        | FfiOperation::SetStructured
                        | FfiOperation::Delete
                        | FfiOperation::NamespaceOpen
                        | FfiOperation::NamespaceUpdatePolicy
                        | FfiOperation::NamespaceDelete
                )
            }) {
                FfiResult {
                    kind: FfiResultKind::UnknownMutation,
                    payload: format!(
                        "client mutation outcome is unknown after cancellation: {error}"
                    )
                    .into_bytes(),
                    client: None,
                }
            } else {
                FfiResult::cancelled(format!("client operation timed out: {error}"))
            }
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
                .unwrap_or_else(FfiResult::from_error);
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
    application_key: Vec<u8>,
    value: Vec<u8>,
    set_options: SetOptions,
    raw: bool,
) -> FfiResult {
    let result = if raw {
        execute_raw(client, operation, application_key, value, set_options).await
    } else {
        execute_protected(client, operation, application_key, value, set_options).await
    };
    result.unwrap_or_else(FfiResult::from_error)
}

async fn execute_protected(
    client: &LocalProtectedClient,
    operation: FfiOperation,
    canonical_key: Vec<u8>,
    value: Vec<u8>,
    set_options: SetOptions,
) -> std::result::Result<FfiResult, crate::Error> {
    match operation {
        FfiOperation::Ping => client.ping().await.map(|_| ok_result()),
        FfiOperation::Get => client
            .get_canonical_key_unchecked(canonical_key.as_slice())
            .await
            .map(|value| get_result(value, raw_value_result)),
        FfiOperation::GetJson => client
            .get_canonical_key_unchecked(canonical_key.as_slice())
            .await
            .and_then(json_result),
        FfiOperation::GetStructured => client
            .get_structured_canonical_key_unchecked(canonical_key.as_slice())
            .await
            .and_then(structured_result),
        FfiOperation::Set => client
            .set_canonical_key_unchecked(canonical_key.as_slice(), Value::Raw(value), set_options)
            .await
            .map(set_result),
        FfiOperation::SetJson => match parse_json(&value) {
            Ok(json) => client
                .set_canonical_key_unchecked(
                    canonical_key.as_slice(),
                    Value::Json(json),
                    set_options,
                )
                .await
                .map(set_result),
            Err(error) => Err(crate::value::Error::InvalidJson(error).into()),
        },
        FfiOperation::SetStructured => {
            let structured = decode(&value).map_err(crate::value::Error::Structured)?;
            client
                .set_structured_canonical_key_unchecked(
                    canonical_key.as_slice(),
                    structured,
                    set_options,
                )
                .await
                .map(set_result)
        }
        FfiOperation::Delete => client
            .delete_canonical_key_unchecked(canonical_key.as_slice())
            .await
            .map(delete_result),
        FfiOperation::Stats => client
            .stats()
            .await
            .map(|stats| FfiResult::success(FfiResultKind::Value, stats.into_bytes())),
        FfiOperation::Sync => client.sync().await.map(|()| ok_result()),
        FfiOperation::Reconnect => client.reconnect().await.map(|()| ok_result()),
        _ => Err(crate::Error::configuration(
            "operation",
            "unsupported operation from the generated Smithy contract",
        )),
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
        FfiOperation::Ping => client.raw().ping().await.map(|_| ok_result()),
        FfiOperation::Get => {
            let item_id = ItemId::from_slice(&item_id)?;
            client
                .raw()
                .get(item_id)
                .await
                .map(|value| get_result(value, value_result))
        }
        FfiOperation::Set => {
            let item_id = ItemId::from_slice(&item_id)?;
            client
                .raw()
                .set(item_id, ItemValue::new(value), set_options)
                .await
                .map(set_result)
        }
        FfiOperation::Delete => {
            let item_id = ItemId::from_slice(&item_id)?;
            client.raw().delete(item_id).await.map(delete_result)
        }
        FfiOperation::Stats => client
            .raw()
            .stats()
            .await
            .map(|stats| FfiResult::success(FfiResultKind::Value, stats.into_bytes())),
        FfiOperation::Sync => client.raw().sync().await.map(|()| ok_result()),
        FfiOperation::Reconnect => client.raw().reconnect().await.map(|()| ok_result()),
        FfiOperation::GetJson
        | FfiOperation::SetJson
        | FfiOperation::GetStructured
        | FfiOperation::SetStructured => Err(crate::Error::configuration(
            "operation",
            "exact item-ID calls do not support formatted structured or JSON operations",
        )),
        _ => Err(crate::Error::configuration(
            "operation",
            "unsupported operation from the generated Smithy contract",
        )),
    }
}

async fn execute_scoped(
    client: &LocalProtectedClient,
    operation: FfiOperation,
    namespace_id: u64,
    item_id: Vec<u8>,
    value: Vec<u8>,
    set_options: SetOptions,
) -> std::result::Result<FfiResult, crate::Error> {
    match operation {
        FfiOperation::Get => {
            let item_id = ItemId::from_slice(&item_id)?;
            client
                .raw()
                .get_in_namespace(namespace_id, item_id)
                .await
                .map(|value| get_result(value, value_result))
        }
        FfiOperation::Set => {
            let item_id = ItemId::from_slice(&item_id)?;
            client
                .raw()
                .set_in_namespace(namespace_id, item_id, ItemValue::new(value), set_options)
                .await
                .map(set_result)
        }
        FfiOperation::Delete => {
            let item_id = ItemId::from_slice(&item_id)?;
            client
                .raw()
                .delete_in_namespace(namespace_id, item_id)
                .await
                .map(delete_result)
        }
        FfiOperation::Stats => client
            .raw()
            .stats_in_namespace(namespace_id)
            .await
            .map(|stats| FfiResult::success(FfiResultKind::Value, stats.into_bytes())),
        FfiOperation::Sync => client
            .raw()
            .sync_in_namespace(namespace_id)
            .await
            .map(|()| ok_result()),
        _ => Err(crate::Error::configuration(
            "operation",
            "unsupported namespace-scoped operation from the generated Smithy contract",
        )),
    }
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
            Ok(payload) => FfiResult::success(
                if created {
                    FfiResultKind::Created
                } else {
                    FfiResultKind::Ok
                },
                payload,
            ),
            Err(error) => FfiResult::error(error.to_string()),
        },
        Err(error) => FfiResult::from_error(error),
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
            Ok(payload) => FfiResult::success(FfiResultKind::Value, payload),
            Err(error) => FfiResult::error(error),
        },
        Err(error) => FfiResult::from_error(error),
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
        Ok(()) => ok_result(),
        Err(error) => FfiResult::from_error(error),
    }
}

fn set_options_from_flags(flags: u8, ttl_ms: u64) -> std::result::Result<SetOptions, String> {
    let ttl_ms = (ttl_ms != 0).then_some(ttl_ms);
    crate::protocol::SetWireOptions::from_wire_parts(flags, ttl_ms)
        .map_err(|error| error.to_string())
        .and_then(|options| {
            SetOptions::from_wire_options(options).map_err(|error| error.to_string())
        })
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

fn not_found_result() -> FfiResult {
    FfiResult::success(FfiResultKind::NotFound, Vec::new())
}

fn get_result<T>(outcome: GetOutcome<T>, found: impl FnOnce(T) -> FfiResult) -> FfiResult {
    match outcome {
        GetOutcome::Found(value) => found(value),
        GetOutcome::NotFound => not_found_result(),
    }
}

fn value_result(value: ItemValue) -> FfiResult {
    bytes_result(value.into_bytes())
}

fn bytes_result(payload: Vec<u8>) -> FfiResult {
    FfiResult::success(FfiResultKind::Value, payload)
}

fn raw_value_result(value: Value) -> FfiResult {
    match value {
        Value::Raw(payload) => bytes_result(payload),
        Value::Json(_) => FfiResult::error("formatted value is not Raw serialization"),
    }
}

fn structured_result(
    outcome: GetOutcome<StructuredValue>,
) -> std::result::Result<FfiResult, crate::Error> {
    match outcome {
        GetOutcome::Found(value) => encode(&value)
            .map(|payload| FfiResult::success(FfiResultKind::Value, payload))
            .map_err(|error| crate::value::Error::Structured(error).into()),
        GetOutcome::NotFound => Ok(not_found_result()),
    }
}

fn delete_result(outcome: DeleteOutcome) -> FfiResult {
    FfiResult::success(
        match outcome {
            DeleteOutcome::Deleted => FfiResultKind::Deleted,
            DeleteOutcome::NotFound => FfiResultKind::NotDeleted,
        },
        Vec::new(),
    )
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
        GetOutcome::NotFound => Ok(not_found_result()),
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
/// For `GET`, `SET`, and `DELETE`, `application_key` is exactly one canonical
/// v1 key item from `KEY_FORMAT.md`. The CBOR item is the ABI's type
/// discriminator (`Integer`, `Text`, or `Bytes`); it is not raw application
/// bytes and is not a 32-byte Item ID. `SET` accepts an empty value and
/// optional existence/TTL options. `PING`, `STATS`, and `SYNC` require empty
/// key and value buffers.
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

/// Retrieves one StructuredValue-CBOR-v1 payload through the protected native ABI.
///
/// The result payload is the canonical StructuredValue-CBOR-v1 bytes. It is
/// never converted through the compatibility JSON or Raw value paths.
///
/// # Safety
///
/// `client` must be a live pointer returned by
/// [`openkache_client_result_take_client`]. The canonical key pointer must
/// identify readable memory for the duration of this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_client_get_structured(
    client: *const FfiClient,
    canonical_key: *const u8,
    canonical_key_length: usize,
) -> *mut FfiResult {
    execute_entry(
        client,
        FfiOperation::GetStructured as u32,
        canonical_key,
        canonical_key_length,
        ptr::null(),
        0,
        FfiSetCondition::Any as u32,
        0,
        0,
        false,
    )
}

/// Stores one StructuredValue-CBOR-v1 payload through the protected native ABI.
///
/// The input value is decoded and validated by the shared StructuredValue
/// codec before protection. It is never routed through compatibility JSON or
/// Raw value operations.
///
/// # Safety
///
/// `client` must be a live pointer returned by
/// [`openkache_client_result_take_client`]. The canonical key and value
/// pointers must identify readable memory for the duration of this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_client_set_structured(
    client: *const FfiClient,
    canonical_key: *const u8,
    canonical_key_length: usize,
    value: *const u8,
    value_length: usize,
    set_flags: u8,
    ttl_ms: u64,
) -> *mut FfiResult {
    execute_entry_with_flags(
        client,
        FfiOperation::SetStructured as u32,
        canonical_key,
        canonical_key_length,
        value,
        value_length,
        set_flags,
        ttl_ms,
        false,
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
        let set_options = if operation == FfiOperation::Set {
            set_options_from_flags(set_flags, ttl_ms)?
        } else {
            if set_flags != 0 || ttl_ms != 0 {
                return Err("SET flags and TTL require a SET operation".to_owned());
            }
            SetOptions::new()
        };
        match operation {
            FfiOperation::Get | FfiOperation::Set | FfiOperation::Delete
                if item_id.len() != crate::ITEM_ID_BYTES =>
            {
                Err(format!(
                    "item_id must contain exactly {} bytes, got {}",
                    crate::ITEM_ID_BYTES,
                    item_id.len()
                ))
            }
            FfiOperation::Get | FfiOperation::Delete if !value.is_empty() => {
                Err("operation does not accept a value".to_owned())
            }
            FfiOperation::Stats | FfiOperation::Sync if !item_id.is_empty() => {
                Err("operation does not accept an item_id".to_owned())
            }
            FfiOperation::Stats | FfiOperation::Sync if !value.is_empty() => {
                Err("operation does not accept a value".to_owned())
            }
            FfiOperation::GetJson
            | FfiOperation::SetJson
            | FfiOperation::GetStructured
            | FfiOperation::SetStructured
            | FfiOperation::Ping => Err(
                "operation is not available through the namespace-scoped exact-ID ABI".to_owned(),
            ),
            FfiOperation::NamespaceOpen
            | FfiOperation::NamespaceUpdatePolicy
            | FfiOperation::NamespaceDelete
            | FfiOperation::Reconnect => {
                Err("namespace management and reconnect use dedicated native ABI calls".to_owned())
            }
            _ => Ok(client.execute_scoped(operation, namespace_id, item_id, value, set_options)),
        }
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
        if name.len() > openkache_protocol::compat_v1::NAMESPACE_NAME_MAX_BYTES {
            return Err(format!(
                "namespace name exceeds {} octets",
                openkache_protocol::compat_v1::NAMESPACE_NAME_MAX_BYTES
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
    let Ok(descriptor) = crate::NamespaceDescriptor::decode(payload) else {
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
        if raw
            && matches!(
                operation,
                FfiOperation::GetJson
                    | FfiOperation::SetJson
                    | FfiOperation::GetStructured
                    | FfiOperation::SetStructured
            )
        {
            return Err(
                "exact item-ID calls do not support formatted structured or JSON operations"
                    .to_owned(),
            );
        }
        if raw
            && matches!(
                operation,
                FfiOperation::Get | FfiOperation::Set | FfiOperation::Delete
            )
            && application_key.len() != crate::ITEM_ID_BYTES
        {
            return Err(format!(
                "item_id must contain exactly {} bytes, got {}",
                crate::ITEM_ID_BYTES,
                application_key.len()
            ));
        }
        let set_options = if let Some((flags, ttl_ms)) = complete_flags {
            if matches!(
                operation,
                FfiOperation::Set | FfiOperation::SetJson | FfiOperation::SetStructured
            ) {
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
        match operation {
            FfiOperation::Ping
            | FfiOperation::Stats
            | FfiOperation::Sync
            | FfiOperation::Reconnect
                if !application_key.is_empty() =>
            {
                Err("operation does not accept an application key".to_owned())
            }
            FfiOperation::Ping
            | FfiOperation::Get
            | FfiOperation::GetJson
            | FfiOperation::GetStructured
            | FfiOperation::Delete
            | FfiOperation::Stats
            | FfiOperation::Sync
            | FfiOperation::Reconnect
                if !value.is_empty() =>
            {
                Err("operation does not accept a value".to_owned())
            }
            operation
                if !matches!(
                    operation,
                    FfiOperation::Set | FfiOperation::SetJson | FfiOperation::SetStructured
                ) && (set_options.condition() != SetCondition::Any
                    || set_options.time_to_live_millis().is_some()) =>
            {
                Err("SET options require a SET operation".to_owned())
            }
            _ => Ok(client.execute(operation, application_key, value, set_options, raw)),
        }
    }))
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
