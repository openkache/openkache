//! Stable C ABI shared by native language bindings.
//!
//! The ABI owns one Tokio runtime and one protected client per native handle. C, C++, and other
//! native bindings only marshal buffers and interpret result discriminators; connection management,
//! retries, protocol framing, and value protection remain in this crate.

use std::collections::{HashMap, VecDeque};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{SyncSender, sync_channel};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use openkache_protocol::{MUTATION_ID_BYTES, MutationId};

pub use crate::contract::FFI_ABI_VERSION as ABI_VERSION;
use crate::contract::{
    FFI_ERROR_AMBIGUOUS, FFI_ERROR_CANCELLED, FFI_ERROR_CLOSED, FFI_ERROR_CONFIGURATION,
    FFI_ERROR_CONNECTION, FFI_ERROR_IO, FFI_ERROR_PROTOCOL, FFI_ERROR_RESPONSE_TOO_LARGE,
    FFI_ERROR_RUNTIME, FFI_ERROR_SERVER, FFI_ERROR_TIMEOUT, FFI_ERROR_TLS, FFI_ERROR_TRANSPORT,
    FFI_ERROR_UNEXPECTED_RESPONSE, FFI_ERROR_VALUE, VALUE_FORMAT_ENCRYPTION_COMPACT,
    VALUE_FORMAT_ENCRYPTION_NONE, VALUE_FORMAT_ENCRYPTION_ROBUST,
};
pub use crate::contract::{FfiOperation, FfiResultKind, FfiSetCondition};
use crate::value::{
    Compression, Encryption, JsonValue, Value, ZstandardOptions, canonical_json_bytes,
};
use crate::{
    Certificate, ClientIdentity, ClientTimeouts, ConnectionState, CoreMetricsSnapshot,
    DataProtectionKey, DataProtectionKeyRing, DeleteOutcome, Endpoint, GetOutcome, ItemId,
    ItemValue, MAX_PREVIOUS_DATA_PROTECTION_KEYS, PrivateKey, ProtectedClient, RetryPolicy,
    ServerTrust, SetCondition, SetOptions, SetOutcome,
};
use futures_util::{FutureExt, StreamExt, pin_mut};
use zeroize::{Zeroize, Zeroizing};

/// Opaque result allocated by the native ABI.
pub struct FfiResult {
    kind: FfiResultKind,
    payload: Vec<u8>,
    metadata: FfiErrorMetadata,
    client: Option<Box<FfiClient>>,
}

/// Structured metadata for a failed native operation.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct FfiErrorMetadata {
    /// Stable client error category.
    pub code: u32,
    /// Stable operation discriminator, or zero when no operation was started.
    pub operation: u32,
    /// Stable phase discriminator, or zero when no phase was identified.
    pub phase: u32,
    /// Backend discriminator: one for Quinn, two for Compio, zero otherwise.
    pub backend: u32,
    /// Non-zero when retrying the operation is safe.
    pub retryable: u8,
    /// Non-zero when the server may have applied the operation.
    pub ambiguous: u8,
    /// Number of meaningful bytes in [`Self::mutation_id`].
    pub mutation_id_length: u8,
    /// Reserved for ABI alignment and future metadata fields.
    pub reserved: u8,
    /// Mutation token that can be reused after an ambiguous outcome.
    pub mutation_id: [u8; MUTATION_ID_BYTES],
}

/// Point-in-time counters collected by one native client handle.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct FfiMetricsSnapshot {
    /// Number of operations submitted to the worker.
    pub requests: u64,
    /// Number of successful GET responses containing a value.
    pub hits: u64,
    /// Number of successful GET responses without a value.
    pub misses: u64,
    /// Number of retry attempts after the initial attempt.
    pub retries: u64,
    /// Number of replacement connections established.
    pub reconnects: u64,
    /// Number of requests canceled by the caller.
    pub cancellations: u64,
    /// Number of transport-level failures.
    pub transport_errors: u64,
    /// Number of protocol-level failures.
    pub protocol_errors: u64,
    /// Bytes submitted in request values and keys.
    pub bytes_sent: u64,
    /// Bytes returned in response payloads.
    pub bytes_received: u64,
    /// Requests currently executing on the worker.
    pub active_lanes: u64,
}

const _: () = {
    assert!(core::mem::size_of::<FfiErrorMetadata>() == crate::contract::FFI_ERROR_METADATA_BYTES);
    assert!(
        core::mem::size_of::<FfiMetricsSnapshot>() == crate::contract::FFI_METRICS_SNAPSHOT_BYTES
    );
};

#[derive(Default)]
struct FfiMetrics {
    requests: AtomicU64,
    hits: AtomicU64,
    misses: AtomicU64,
    retries: AtomicU64,
    reconnects: AtomicU64,
    cancellations: AtomicU64,
    transport_errors: AtomicU64,
    protocol_errors: AtomicU64,
    bytes_sent: AtomicU64,
    bytes_received: AtomicU64,
    active_lanes: AtomicUsize,
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
    /// Exact 32-byte application data-protection key.
    pub data_protection_key: *const u8,
    /// Byte length of [`Self::data_protection_key`].
    pub data_protection_key_length: usize,
    /// Concatenated retired data-protection keys, newest first.
    pub previous_data_protection_keys: *const u8,
    /// Byte length of [`Self::previous_data_protection_keys`].
    pub previous_data_protection_keys_length: usize,
    /// Number of retired keys in [`Self::previous_data_protection_keys`].
    pub previous_data_protection_key_count: usize,
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

const _: () = {
    assert!(
        core::mem::size_of::<FfiConnectOptions>() == crate::contract::FFI_CONNECT_OPTIONS_BYTES
    );
};

/// Opaque native handle to a dedicated Rust client worker.
pub struct FfiClient {
    commands: CommandSender,
    request_timeout: Duration,
    max_in_flight: usize,
    in_flight: Arc<AtomicUsize>,
    shutdown: Arc<AtomicBool>,
    state: Arc<AtomicU32>,
    worker: Mutex<Option<JoinHandle<()>>>,
    next_request_id: AtomicU64,
    metrics: Arc<FfiMetrics>,
    requests: RequestRegistry,
}

enum Command {
    Execute {
        request_id: u64,
        operation: FfiOperation,
        application_key: Vec<u8>,
        value: Vec<u8>,
        set_options: SetOptions,
        raw: bool,
        transmission: Arc<AtomicBool>,
        response: SyncSender<FfiResult>,
    },
    Cancel {
        request_id: u64,
    },
    Shutdown,
}

type CommandSender = crossfire::MTx<crossfire::mpsc::Array<Command>>;
type CommandReceiver = crossfire::AsyncRx<crossfire::mpsc::Array<Command>>;
type ActiveRequest = (
    tokio::task::AbortHandle,
    SyncSender<FfiResult>,
    FfiOperation,
    Option<MutationId>,
    Arc<AtomicBool>,
);
type PendingRequest = (
    SyncSender<FfiResult>,
    FfiOperation,
    Option<MutationId>,
    Arc<AtomicBool>,
);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RequestState {
    /// The caller has reserved a request ID, but the worker has not started it.
    Queued,
    /// The worker owns a running native future for this request.
    Active,
    /// A cancellation was requested before the worker observed the request.
    CancelRequested,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RequestClaim {
    Claimed,
    CancelRequested,
    Invalid,
}

type RequestRegistry = Arc<Mutex<HashMap<u64, RequestState>>>;

struct WorkerOptions {
    endpoint: Endpoint,
    certificate: Vec<u8>,
    key_ring: DataProtectionKeyRing,
    client_certificate_chain: Vec<u8>,
    client_private_key: Zeroizing<Vec<u8>>,
    compression: Compression,
    encryption: Encryption,
    timeouts: ClientTimeouts,
    retry: RetryPolicy,
    max_in_flight: usize,
}

impl FfiResult {
    fn error_with_operation(
        message: impl Into<String>,
        code: u32,
        operation: Option<FfiOperation>,
    ) -> Self {
        Self {
            kind: FfiResultKind::Error,
            payload: message.into().into_bytes(),
            metadata: FfiErrorMetadata {
                code,
                operation: operation.map_or(0, ffi_operation_code),
                ..FfiErrorMetadata::default()
            },
            client: None,
        }
    }

    fn success(kind: FfiResultKind, payload: Vec<u8>) -> Self {
        Self {
            kind,
            payload,
            metadata: FfiErrorMetadata::default(),
            client: None,
        }
    }

    fn error_from(error: &crate::Error, operation: FfiOperation) -> Self {
        Self {
            kind: FfiResultKind::Error,
            payload: error.to_string().into_bytes(),
            metadata: error_metadata(error, operation),
            client: None,
        }
    }

    fn cancelled(
        operation: Option<FfiOperation>,
        mutation_id: Option<MutationId>,
        ambiguous: bool,
    ) -> Self {
        let mut metadata = FfiErrorMetadata {
            code: FFI_ERROR_CANCELLED,
            operation: operation.map_or(0, ffi_operation_code),
            ..FfiErrorMetadata::default()
        };
        attach_mutation_metadata(&mut metadata, mutation_id, ambiguous);
        Self {
            kind: FfiResultKind::Error,
            payload: b"client operation canceled".to_vec(),
            metadata,
            client: None,
        }
    }

    fn timed_out(
        operation: FfiOperation,
        mutation_id: Option<MutationId>,
        ambiguous: bool,
    ) -> Self {
        let mut metadata = FfiErrorMetadata {
            code: FFI_ERROR_TIMEOUT,
            operation: ffi_operation_code(operation),
            ..FfiErrorMetadata::default()
        };
        attach_mutation_metadata(&mut metadata, mutation_id, ambiguous);
        Self {
            kind: FfiResultKind::Error,
            payload: format!("client operation timed out during {operation:?}").into_bytes(),
            metadata,
            client: None,
        }
    }

    fn queue_full(operation: FfiOperation) -> Self {
        let mut result = Self::error_with_operation(
            "client worker queue is full",
            crate::contract::FFI_ERROR_RUNTIME,
            Some(operation),
        );
        result.metadata.retryable = 1;
        result
    }

    fn connected(client: FfiClient) -> Self {
        Self {
            kind: FfiResultKind::Connected,
            payload: Vec::new(),
            metadata: FfiErrorMetadata::default(),
            client: Some(Box::new(client)),
        }
    }
}

fn attach_mutation_metadata(
    metadata: &mut FfiErrorMetadata,
    mutation_id: Option<MutationId>,
    ambiguous: bool,
) {
    if let Some(mutation_id) = mutation_id {
        metadata.retryable = 1;
        metadata.ambiguous = u8::from(ambiguous);
        metadata.mutation_id_length = MUTATION_ID_BYTES as u8;
        metadata.mutation_id = mutation_id.into_bytes();
    }
}

fn ffi_operation_code(operation: FfiOperation) -> u32 {
    operation.code()
}

impl FfiClient {
    // The argument list mirrors the stable native connection contract.
    #[allow(clippy::too_many_arguments)]
    fn connect(
        endpoint: Endpoint,
        certificate: Vec<u8>,
        key_ring: DataProtectionKeyRing,
        client_certificate_chain: Vec<u8>,
        client_private_key: Zeroizing<Vec<u8>>,
        compression: Compression,
        encryption: Encryption,
        timeouts: ClientTimeouts,
        retry: RetryPolicy,
        max_in_flight: usize,
    ) -> std::result::Result<Self, String> {
        // Reserve one command slot per possible in-flight request for
        // cancellation/shutdown control messages. Execute requests still
        // reserve their capacity through `in_flight`, so this remains
        // bounded by the caller's `max_in_flight` setting without allowing a
        // full execute queue to make cancellation impossible.
        let command_capacity = max_in_flight.max(1).saturating_mul(2).max(1);
        let (commands, receiver) = crossfire::mpsc::bounded_blocking_async(command_capacity);
        let (ready_sender, ready_receiver) = sync_channel(1);
        let shutdown = Arc::new(AtomicBool::new(false));
        let in_flight = Arc::new(AtomicUsize::new(0));
        let metrics = Arc::new(FfiMetrics::default());
        let worker_shutdown = Arc::clone(&shutdown);
        let worker_in_flight = Arc::clone(&in_flight);
        let requests = Arc::new(Mutex::new(HashMap::new()));
        let worker_requests = Arc::clone(&requests);
        let state = Arc::new(AtomicU32::new(connection_state_value(
            ConnectionState::Reconnecting,
        )));
        let worker_state = Arc::clone(&state);
        let options = WorkerOptions {
            endpoint,
            certificate,
            key_ring,
            client_certificate_chain,
            client_private_key,
            compression,
            encryption,
            timeouts,
            retry,
            max_in_flight,
        };
        let worker_metrics = Arc::clone(&metrics);
        let worker = thread::Builder::new()
            .name("openkache-client".to_owned())
            .spawn(move || {
                run_worker(
                    receiver,
                    ready_sender,
                    options,
                    worker_shutdown,
                    worker_state,
                    worker_metrics,
                    worker_in_flight,
                    worker_requests,
                )
            })
            .map_err(|error| format!("failed to start client worker: {error}"))?;

        match ready_receiver.recv() {
            Ok(Ok(())) => Ok(Self {
                commands,
                request_timeout: timeouts.request,
                max_in_flight: max_in_flight.max(1),
                in_flight,
                shutdown,
                state,
                worker: Mutex::new(Some(worker)),
                next_request_id: AtomicU64::new(1),
                metrics,
                requests,
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
        let request_id = self.allocate_request_id();
        self.execute_with_request_id(
            request_id,
            operation,
            application_key,
            value,
            set_options,
            raw,
        )
    }

    fn allocate_request_id(&self) -> u64 {
        self.next_request_id
            .try_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                Some(if current == u64::MAX {
                    1
                } else {
                    current.saturating_add(1).max(1)
                })
            })
            .unwrap_or(1)
    }

    fn execute_with_request_id(
        &self,
        request_id: u64,
        operation: FfiOperation,
        application_key: Vec<u8>,
        value: Vec<u8>,
        mut set_options: SetOptions,
        raw: bool,
    ) -> FfiResult {
        if set_options.mutation_id().is_none()
            && matches!(
                operation,
                FfiOperation::Set | FfiOperation::SetJson | FfiOperation::Delete
            )
        {
            let mutation_id = match crate::key::random_mutation_id() {
                Ok(mutation_id) => mutation_id,
                Err(error) => {
                    return FfiResult::error_with_operation(
                        error.to_string(),
                        crate::contract::FFI_ERROR_RUNTIME,
                        Some(operation),
                    );
                }
            };
            set_options = set_options.with_mutation_id(mutation_id);
        }
        if !self.register_request(request_id) {
            return FfiResult::error_with_operation(
                format!("request ID {request_id} is already active"),
                crate::contract::FFI_ERROR_RUNTIME,
                Some(operation),
            );
        }
        self.metrics.requests.fetch_add(1, Ordering::Relaxed);
        self.metrics.bytes_sent.fetch_add(
            (application_key.len() + value.len()) as u64,
            Ordering::Relaxed,
        );
        let mutation_id = set_options.mutation_id();
        let (response, receiver) = sync_channel(1);
        let Some(deadline) = Instant::now().checked_add(self.request_timeout) else {
            self.remove_request(request_id);
            return FfiResult::error_with_operation(
                "client request timeout exceeds the platform clock range",
                crate::contract::FFI_ERROR_RUNTIME,
                Some(operation),
            );
        };
        if !self.reserve_slot_until(deadline) {
            self.remove_request(request_id);
            return FfiResult::queue_full(operation);
        }
        let transmission = Arc::new(AtomicBool::new(false));
        let command = Command::Execute {
            request_id,
            operation,
            application_key,
            value,
            set_options,
            raw,
            transmission: Arc::clone(&transmission),
            response,
        };
        let remaining = deadline.saturating_duration_since(Instant::now());
        if let Err(error) = self.commands.send_timeout(command, remaining) {
            self.release_slot();
            self.remove_request(request_id);
            return FfiResult::error_with_operation(
                format!("client worker queue deadline exceeded: {error}"),
                crate::contract::FFI_ERROR_RUNTIME,
                Some(operation),
            );
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        let result = match receiver.recv_timeout(remaining) {
            Ok(result) => result,
            Err(_) => {
                // The request deadline has already elapsed. Do not spend
                // another request-timeout waiting for cancellation. The
                // worker retains the request registry entry until it has
                // observed the cancellation intent and cleaned up its lane.
                let _ = self.cancel_with_timeout(request_id, Duration::ZERO);
                return FfiResult::timed_out(
                    operation,
                    mutation_id,
                    transmission.load(Ordering::Acquire),
                );
            }
        };
        match result.kind {
            FfiResultKind::Value => {
                self.metrics.hits.fetch_add(1, Ordering::Relaxed);
                self.metrics
                    .bytes_received
                    .fetch_add(result.payload.len() as u64, Ordering::Relaxed);
            }
            FfiResultKind::NotFound => {
                self.metrics.misses.fetch_add(1, Ordering::Relaxed);
            }
            _ => {}
        }
        result
    }

    fn try_reserve_slot(&self) -> bool {
        try_reserve_slot(&self.in_flight, self.max_in_flight)
    }

    fn reserve_slot_until(&self, deadline: Instant) -> bool {
        loop {
            if self.try_reserve_slot() {
                return true;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return false;
            }
            // Native adapters invoke this synchronous ABI from their own
            // worker/task threads. A short bounded wait provides backpressure
            // without turning max_in_flight into an immediate reject gate.
            thread::sleep(remaining.min(Duration::from_millis(1)));
        }
    }

    fn release_slot(&self) {
        release_slot(&self.in_flight);
    }

    fn cancel(&self, request_id: u64) -> bool {
        self.cancel_with_timeout(request_id, self.request_timeout)
    }

    fn cancel_with_timeout(&self, request_id: u64, timeout: Duration) -> bool {
        if !self.request_cancel(request_id) {
            return false;
        }
        // Record the intent before enqueueing the control command. If the
        // worker observes Execute first, it will still turn that request into
        // a cancellation result; if it observes Cancel first, the intent is
        // retained until Execute arrives.
        self.commands
            .send_timeout(Command::Cancel { request_id }, timeout)
            .is_ok()
    }

    fn register_request(&self, request_id: u64) -> bool {
        let mut requests = self
            .requests
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if requests.contains_key(&request_id) {
            return false;
        }
        requests.insert(request_id, RequestState::Queued);
        true
    }

    fn remove_request(&self, request_id: u64) {
        self.requests
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&request_id);
    }

    fn request_cancel(&self, request_id: u64) -> bool {
        let mut requests = self
            .requests
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(state) = requests.get_mut(&request_id) else {
            return false;
        };
        if *state == RequestState::CancelRequested {
            return false;
        }
        *state = RequestState::CancelRequested;
        true
    }

    fn metrics_snapshot(&self) -> FfiMetricsSnapshot {
        FfiMetricsSnapshot {
            requests: self.metrics.requests.load(Ordering::Relaxed),
            hits: self.metrics.hits.load(Ordering::Relaxed),
            misses: self.metrics.misses.load(Ordering::Relaxed),
            retries: self.metrics.retries.load(Ordering::Relaxed),
            reconnects: self.metrics.reconnects.load(Ordering::Relaxed),
            cancellations: self.metrics.cancellations.load(Ordering::Relaxed),
            transport_errors: self.metrics.transport_errors.load(Ordering::Relaxed),
            protocol_errors: self.metrics.protocol_errors.load(Ordering::Relaxed),
            bytes_sent: self.metrics.bytes_sent.load(Ordering::Relaxed),
            bytes_received: self.metrics.bytes_received.load(Ordering::Relaxed),
            active_lanes: self.metrics.active_lanes.load(Ordering::Relaxed) as u64,
        }
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
    metrics: Arc<FfiMetrics>,
    in_flight: Arc<AtomicUsize>,
    requests: RequestRegistry,
) {
    let WorkerOptions {
        endpoint,
        certificate,
        key_ring,
        client_certificate_chain,
        mut client_private_key,
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
    let mut builder = ProtectedClient::builder_with_key_ring(endpoint, key_ring)
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
        let private_key_result = PrivateKey::from_der_or_pem(&client_private_key);
        client_private_key.zeroize();
        let private_key = match private_key_result {
            Ok(private_key) => private_key,
            Err(error) => {
                let _ = ready.send(Err(error.to_string()));
                return;
            }
        };
        let certificate_chain = match Certificate::from_der_or_pem_chain(&client_certificate_chain)
        {
            Ok(certificate_chain) => certificate_chain,
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

    runtime.block_on(run_worker_loop(
        commands,
        client,
        shutdown,
        state,
        metrics,
        max_in_flight,
        in_flight,
        requests,
    ));
}

async fn run_worker_loop(
    commands: CommandReceiver,
    client: ProtectedClient,
    shutdown: Arc<AtomicBool>,
    state: Arc<AtomicU32>,
    metrics: Arc<FfiMetrics>,
    max_in_flight: usize,
    in_flight: Arc<AtomicUsize>,
    requests: RequestRegistry,
) {
    let mut pending = VecDeque::new();
    let mut core_metrics = CoreMetricsSnapshot::default();
    let mut active = futures_util::stream::FuturesUnordered::new();
    let mut active_requests = HashMap::new();

    loop {
        while active_requests.len() < max_in_flight {
            let Some(command) = pending.pop_front() else {
                break;
            };
            spawn_request(
                command,
                &client,
                &mut active,
                &mut active_requests,
                &requests,
                &in_flight,
                &metrics,
            );
        }
        metrics
            .active_lanes
            .store(active_requests.len(), Ordering::Release);
        if shutdown.load(Ordering::Acquire) {
            for (request_id, (abort, response, operation, mutation_id, transmission)) in
                active_requests.drain()
            {
                remove_request(&requests, request_id);
                abort.abort();
                release_slot(&in_flight);
                let _ = response.send(FfiResult::cancelled(
                    Some(operation),
                    mutation_id,
                    transmission.load(Ordering::Acquire),
                ));
            }
            drain_pending(&mut pending, &in_flight, &requests);
            drain_commands(&commands, &in_flight, &requests);
            return;
        }
        let has_active = !active.is_empty();
        let completed = active.next().fuse();
        let command = commands.recv().fuse();
        pin_mut!(completed, command);
        tokio::select! {
            completed = completed, if has_active => {
                match completed {
                    Some(Ok((request_id, result))) => {
                        if let Some((_, response, _, _, _)) = active_requests.remove(&request_id) {
                            remove_request(&requests, request_id);
                            release_slot(&in_flight);
                            sync_core_metrics(&client, &metrics, &mut core_metrics);
                            state.store(
                                connection_state_value(client.connection_state()),
                                Ordering::Release,
                            );
                            let _ = response.send(result);
                        }
                    }
                    Some(Err(error)) if error.is_cancelled() => {
                        // An explicit request cancellation aborts only that
                        // task. Its response was already completed by the
                        // cancellation command, so keep serving the other
                        // queued and active requests.
                    }
                    Some(Err(error)) => {
                        let message = format!("native client worker task failed: {error}");
                        for (request_id, (abort, response, operation, mutation_id, transmission)) in
                            active_requests.drain()
                        {
                            remove_request(&requests, request_id);
                            abort.abort();
                            release_slot(&in_flight);
                            let mut result = FfiResult::error_with_operation(
                                message.clone(),
                                crate::contract::FFI_ERROR_RUNTIME,
                                Some(operation),
                            );
                            attach_mutation_metadata(
                                &mut result.metadata,
                                mutation_id,
                                transmission.load(Ordering::Acquire),
                            );
                            let _ = response.send(result);
                        }
                        drain_pending(&mut pending, &in_flight, &requests);
                        drain_commands(&commands, &in_flight, &requests);
                        return;
                    }
                    None => {}
                }
            }
            command = command => {
                match command {
                    Ok(Command::Execute {
                        request_id,
                        operation,
                        application_key,
                        value,
                        set_options,
                        raw,
                        transmission,
                        response,
                    }) => {
                        match request_state(&requests, request_id) {
                            Some(RequestState::CancelRequested) => {
                                remove_request(&requests, request_id);
                                release_slot(&in_flight);
                                metrics.cancellations.fetch_add(1, Ordering::Relaxed);
                                let _ = response.send(FfiResult::cancelled(
                                    Some(operation),
                                    set_options.mutation_id(),
                                    transmission.load(Ordering::Acquire),
                                ));
                            }
                            Some(RequestState::Queued) if pending.len() < max_in_flight => {
                                pending.push_back(Command::Execute {
                                    request_id,
                                    operation,
                                    application_key,
                                    value,
                                    set_options,
                                    raw,
                                    transmission,
                                    response,
                                });
                            }
                            Some(RequestState::Queued) => {
                                remove_request(&requests, request_id);
                                release_slot(&in_flight);
                                let _ = response.send(FfiResult::queue_full(operation));
                            }
                            Some(RequestState::Active) => {
                                remove_request(&requests, request_id);
                                release_slot(&in_flight);
                                let _ = response.send(FfiResult::error_with_operation(
                                    format!("request ID {request_id} is already active"),
                                    crate::contract::FFI_ERROR_RUNTIME,
                                    Some(operation),
                                ));
                            }
                            None => {
                                release_slot(&in_flight);
                                let _ = response.send(FfiResult::error_with_operation(
                                    format!("request ID {request_id} is no longer active"),
                                    crate::contract::FFI_ERROR_RUNTIME,
                                    Some(operation),
                                ));
                            }
                        }
                    }
                    Ok(Command::Cancel { request_id }) => {
                        if let Some((
                            abort,
                            original_response,
                            operation,
                            mutation_id,
                            transmission,
                        )) =
                            active_requests.remove(&request_id)
                        {
                            remove_request(&requests, request_id);
                            abort.abort();
                            release_slot(&in_flight);
                            metrics.cancellations.fetch_add(1, Ordering::Relaxed);
                            let _ = original_response.send(FfiResult::cancelled(
                                Some(operation),
                                mutation_id,
                                transmission.load(Ordering::Acquire),
                            ));
                        } else if let Some((
                            original_response,
                            operation,
                            mutation_id,
                            transmission,
                        )) =
                            cancel_pending(&mut pending, request_id)
                        {
                            remove_request(&requests, request_id);
                            release_slot(&in_flight);
                            metrics.cancellations.fetch_add(1, Ordering::Relaxed);
                            let _ = original_response.send(FfiResult::cancelled(
                                Some(operation),
                                mutation_id,
                                transmission.load(Ordering::Acquire),
                            ));
                        } else if request_state(&requests, request_id)
                            == Some(RequestState::CancelRequested)
                        {
                            // The Execute command has not reached the worker
                            // yet. Leave the intent in the registry so the
                            // eventual Execute is completed as canceled.
                        }
                    }
                    Ok(Command::Shutdown) | Err(_) => {
                        shutdown.store(true, Ordering::Release);
                    }
                }
            }
        }
    }
}

fn spawn_request(
    command: Command,
    client: &ProtectedClient,
    active: &mut futures_util::stream::FuturesUnordered<tokio::task::JoinHandle<(u64, FfiResult)>>,
    active_requests: &mut HashMap<u64, ActiveRequest>,
    requests: &RequestRegistry,
    in_flight: &AtomicUsize,
    metrics: &FfiMetrics,
) {
    let Command::Execute {
        request_id,
        operation,
        application_key,
        value,
        set_options,
        raw,
        transmission,
        response,
    } = command
    else {
        return;
    };
    match claim_request(requests, request_id) {
        RequestClaim::Claimed => {}
        RequestClaim::CancelRequested => {
            remove_request(requests, request_id);
            release_slot(in_flight);
            metrics.cancellations.fetch_add(1, Ordering::Relaxed);
            let _ = response.send(FfiResult::cancelled(
                Some(operation),
                set_options.mutation_id(),
                transmission.load(Ordering::Acquire),
            ));
            return;
        }
        RequestClaim::Invalid => {
            remove_request(requests, request_id);
            release_slot(in_flight);
            let _ = response.send(FfiResult::error_with_operation(
                format!("request ID {request_id} is already active"),
                crate::contract::FFI_ERROR_RUNTIME,
                Some(operation),
            ));
            return;
        }
    }
    let task_client = client.clone();
    let mutation_id = set_options.mutation_id();
    let task_transmission = Arc::clone(&transmission);
    let task = tokio::spawn(async move {
        let result = AssertUnwindSafe(execute(
            &task_client,
            operation,
            application_key,
            value,
            set_options,
            raw,
            task_transmission.clone(),
        ))
        .catch_unwind()
        .await
        .unwrap_or_else(|_| {
            let mut result = FfiResult::error_with_operation(
                "native client worker panicked",
                crate::contract::FFI_ERROR_RUNTIME,
                Some(operation),
            );
            attach_mutation_metadata(
                &mut result.metadata,
                mutation_id,
                task_transmission.load(Ordering::Acquire),
            );
            result
        });
        (request_id, result)
    });
    active_requests.insert(
        request_id,
        (
            task.abort_handle(),
            response,
            operation,
            mutation_id,
            transmission,
        ),
    );
    active.push(task);
}

fn cancel_pending(pending: &mut VecDeque<Command>, request_id: u64) -> Option<PendingRequest> {
    let index = pending.iter().position(|command| {
        matches!(command, Command::Execute { request_id: queued_id, .. } if *queued_id == request_id)
    })?;
    let Some(Command::Execute {
        response,
        operation,
        set_options,
        transmission,
        ..
    }) = pending.remove(index)
    else {
        return None;
    };
    Some((response, operation, set_options.mutation_id(), transmission))
}

fn request_state(requests: &RequestRegistry, request_id: u64) -> Option<RequestState> {
    requests
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&request_id)
        .copied()
}

fn claim_request(requests: &RequestRegistry, request_id: u64) -> RequestClaim {
    let mut requests = requests
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    match requests.get_mut(&request_id) {
        Some(state @ RequestState::Queued) => {
            *state = RequestState::Active;
            RequestClaim::Claimed
        }
        Some(RequestState::CancelRequested) => RequestClaim::CancelRequested,
        Some(RequestState::Active) | None => RequestClaim::Invalid,
    }
}

fn remove_request(requests: &RequestRegistry, request_id: u64) {
    requests
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(&request_id);
}

fn drain_pending(
    pending: &mut VecDeque<Command>,
    in_flight: &AtomicUsize,
    requests: &RequestRegistry,
) {
    while let Some(command) = pending.pop_front() {
        match command {
            Command::Execute {
                request_id,
                response,
                operation,
                set_options,
                ..
            } => {
                remove_request(requests, request_id);
                release_slot(in_flight);
                let _ = response.send(FfiResult::cancelled(
                    Some(operation),
                    set_options.mutation_id(),
                    false,
                ));
            }
            Command::Cancel { .. } => {}
            Command::Shutdown => {}
        }
    }
}

fn drain_commands(commands: &CommandReceiver, in_flight: &AtomicUsize, requests: &RequestRegistry) {
    while let Ok(command) = commands.try_recv() {
        match command {
            Command::Execute {
                request_id,
                response,
                operation,
                set_options,
                ..
            } => {
                remove_request(requests, request_id);
                release_slot(in_flight);
                let _ = response.send(FfiResult::cancelled(
                    Some(operation),
                    set_options.mutation_id(),
                    false,
                ));
            }
            Command::Cancel { .. } => {}
            Command::Shutdown => {}
        }
    }
}

fn try_reserve_slot(in_flight: &AtomicUsize, max_in_flight: usize) -> bool {
    let max_in_flight = max_in_flight.max(1);
    let mut current = in_flight.load(Ordering::Acquire);
    loop {
        if current >= max_in_flight {
            return false;
        }
        match in_flight.compare_exchange_weak(
            current,
            current + 1,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => return true,
            Err(observed) => current = observed,
        }
    }
}

fn release_slot(in_flight: &AtomicUsize) {
    let previous = in_flight.fetch_sub(1, Ordering::Release);
    debug_assert!(previous > 0, "in-flight reservation released twice");
}

fn sync_core_metrics(
    client: &ProtectedClient,
    metrics: &FfiMetrics,
    previous: &mut CoreMetricsSnapshot,
) {
    let current = client.metrics_snapshot();
    metrics.retries.fetch_add(
        current.retries.saturating_sub(previous.retries),
        Ordering::Relaxed,
    );
    metrics.reconnects.fetch_add(
        current.reconnects.saturating_sub(previous.reconnects),
        Ordering::Relaxed,
    );
    metrics.transport_errors.fetch_add(
        current
            .transport_errors
            .saturating_sub(previous.transport_errors),
        Ordering::Relaxed,
    );
    metrics.protocol_errors.fetch_add(
        current
            .protocol_errors
            .saturating_sub(previous.protocol_errors),
        Ordering::Relaxed,
    );
    *previous = current;
}

async fn execute(
    client: &ProtectedClient,
    operation: FfiOperation,
    application_key: Vec<u8>,
    value: Vec<u8>,
    set_options: SetOptions,
    raw: bool,
    transmission: Arc<AtomicBool>,
) -> FfiResult {
    let mutation_id = set_options.mutation_id();
    let result = if raw {
        execute_raw(
            client,
            operation,
            application_key,
            value,
            set_options,
            &transmission,
        )
        .await
    } else {
        execute_protected(
            client,
            operation,
            application_key,
            value,
            set_options,
            &transmission,
        )
        .await
    };
    result.unwrap_or_else(|error| {
        let mut result = FfiResult::error_from(&error, operation);
        attach_mutation_metadata(
            &mut result.metadata,
            mutation_id,
            transmission.load(Ordering::Acquire),
        );
        result
    })
}

async fn execute_protected(
    client: &ProtectedClient,
    operation: FfiOperation,
    application_key: Vec<u8>,
    value: Vec<u8>,
    set_options: SetOptions,
    transmission: &AtomicBool,
) -> std::result::Result<FfiResult, crate::Error> {
    match operation {
        FfiOperation::Ping => client.ping().await.map(|_| ok_result()),
        FfiOperation::Get => client
            .get(&application_key)
            .await
            .map(|value| get_result(value, bytes_result)),
        FfiOperation::GetJson => client
            .get_value(&application_key)
            .await
            .and_then(json_result),
        FfiOperation::Set => client
            .set_with_transmission(&application_key, value, set_options, transmission)
            .await
            .map(set_result),
        FfiOperation::SetJson => match parse_json(&value) {
            Ok(json) => client
                .set_value_with_transmission(
                    &application_key,
                    Value::Json(json),
                    set_options,
                    transmission,
                )
                .await
                .map(set_result),
            Err(error) => Err(crate::value::Error::InvalidJson(error).into()),
        },
        FfiOperation::Delete => match set_options.mutation_id() {
            Some(mutation_id) => client
                .delete_with_mutation_id_with_transmission(
                    &application_key,
                    mutation_id,
                    Some(transmission),
                )
                .await
                .map(delete_result),
            None => client.delete(&application_key).await.map(delete_result),
        },
        FfiOperation::Stats => client
            .stats()
            .await
            .map(|stats| FfiResult::success(FfiResultKind::Value, stats.into_bytes())),
        FfiOperation::Sync => client.sync().await.map(|()| ok_result()),
        FfiOperation::Reconnect => client.reconnect().await.map(|()| ok_result()),
    }
}

async fn execute_raw(
    client: &ProtectedClient,
    operation: FfiOperation,
    item_id: Vec<u8>,
    value: Vec<u8>,
    set_options: SetOptions,
    transmission: &AtomicBool,
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
                .set_with_transmission(item_id, ItemValue::new(value), set_options, transmission)
                .await
                .map(set_result)
        }
        FfiOperation::Delete => {
            let item_id = ItemId::from_slice(&item_id)?;
            match set_options.mutation_id() {
                Some(mutation_id) => client
                    .raw()
                    .delete_with_mutation_id_with_transmission(item_id, mutation_id, transmission)
                    .await
                    .map(delete_result),
                None => client.raw().delete(item_id).await.map(delete_result),
            }
        }
        FfiOperation::Stats => client
            .raw()
            .stats()
            .await
            .map(|stats| FfiResult::success(FfiResultKind::Value, stats.into_bytes())),
        FfiOperation::Sync => client.raw().sync().await.map(|()| ok_result()),
        FfiOperation::Reconnect => client.raw().reconnect().await.map(|()| ok_result()),
        FfiOperation::GetJson | FfiOperation::SetJson => Err(crate::Error::configuration(
            "operation",
            "exact item-ID calls do not support formatted JSON operations",
        )),
    }
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
        GetOutcome::Found(Value::Json(value)) => canonical_json_bytes(&value)
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
    boxed_result(catch_result(None, || {
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
    let key_ring = copy_data_protection_key_ring(
        options.data_protection_key,
        options.data_protection_key_length,
        options.previous_data_protection_keys,
        options.previous_data_protection_keys_length,
        options.previous_data_protection_key_count,
    )?;
    let client_certificate_chain = copy_bytes(
        options.client_certificate_chain,
        options.client_certificate_chain_length,
        "client certificate chain",
    )?;
    let client_private_key = copy_secret_bytes(
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
        RetryPolicy::with_max_attempts(options.retry_max_attempts)
    };
    let max_in_flight = if options.max_in_flight == 0 {
        crate::DEFAULT_MAX_IN_FLIGHT
    } else {
        options.max_in_flight
    };
    FfiClient::connect(
        endpoint,
        certificate,
        key_ring,
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

/// Starts one protected operation with a caller-assigned request identifier.
///
/// The identifier is returned only through the caller's bookkeeping; use
/// [`openkache_client_cancel`] from another thread to abort the operation.
///
/// # Safety
///
/// `client` must be a live pointer returned by [`openkache_client_result_take_client`]. The
/// client must remain valid until this call returns, and every non-empty buffer pointer must
/// identify readable memory for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_client_execute_with_request_id(
    client: *const FfiClient,
    request_id: u64,
    operation: u32,
    application_key: *const u8,
    application_key_length: usize,
    value: *const u8,
    value_length: usize,
    set_condition: u32,
    ttl_enabled: u8,
    ttl_ms: u64,
) -> *mut FfiResult {
    execute_entry_with_request_id(
        client,
        operation,
        application_key,
        application_key_length,
        value,
        value_length,
        set_condition,
        ttl_enabled,
        ttl_ms,
        Some(request_id),
        ptr::null(),
        0,
        false,
    )
}

/// Starts one protected operation with both a caller-assigned request ID and
/// a fixed-width mutation token.
///
/// # Safety
///
/// `client` must be a live pointer returned by [`openkache_client_result_take_client`]. The
/// client must remain valid until this call returns. Every non-empty buffer pointer, including
/// `mutation_id`, must identify readable memory for the duration of the call.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn openkache_client_execute_with_request_id_and_mutation_id(
    client: *const FfiClient,
    request_id: u64,
    operation: u32,
    application_key: *const u8,
    application_key_length: usize,
    value: *const u8,
    value_length: usize,
    set_condition: u32,
    ttl_enabled: u8,
    ttl_ms: u64,
    mutation_id: *const u8,
    mutation_id_length: usize,
) -> *mut FfiResult {
    execute_entry_with_request_id(
        client,
        operation,
        application_key,
        application_key_length,
        value,
        value_length,
        set_condition,
        ttl_enabled,
        ttl_ms,
        Some(request_id),
        mutation_id,
        mutation_id_length,
        false,
    )
}

/// Starts one exact-item-ID operation with a caller-assigned request identifier.
///
/// # Safety
///
/// `client` must be a live pointer returned by [`openkache_client_result_take_client`]. The
/// client must remain valid until this call returns, and every non-empty buffer pointer must
/// identify readable memory for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_client_execute_raw_with_request_id(
    client: *const FfiClient,
    request_id: u64,
    operation: u32,
    item_id: *const u8,
    item_id_length: usize,
    value: *const u8,
    value_length: usize,
    set_condition: u32,
    ttl_enabled: u8,
    ttl_ms: u64,
) -> *mut FfiResult {
    execute_entry_with_request_id(
        client,
        operation,
        item_id,
        item_id_length,
        value,
        value_length,
        set_condition,
        ttl_enabled,
        ttl_ms,
        Some(request_id),
        ptr::null(),
        0,
        true,
    )
}

/// Starts one exact-item-ID operation with both a caller-assigned request ID
/// and a fixed-width mutation token.
///
/// # Safety
///
/// `client` must be a live pointer returned by [`openkache_client_result_take_client`]. The
/// client must remain valid until this call returns. Every non-empty buffer pointer, including
/// `mutation_id`, must identify readable memory for the duration of the call.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn openkache_client_execute_raw_with_request_id_and_mutation_id(
    client: *const FfiClient,
    request_id: u64,
    operation: u32,
    item_id: *const u8,
    item_id_length: usize,
    value: *const u8,
    value_length: usize,
    set_condition: u32,
    ttl_enabled: u8,
    ttl_ms: u64,
    mutation_id: *const u8,
    mutation_id_length: usize,
) -> *mut FfiResult {
    execute_entry_with_request_id(
        client,
        operation,
        item_id,
        item_id_length,
        value,
        value_length,
        set_condition,
        ttl_enabled,
        ttl_ms,
        Some(request_id),
        mutation_id,
        mutation_id_length,
        true,
    )
}

// The argument list mirrors the stable native operation contract.
#[allow(clippy::too_many_arguments)]
fn execute_entry_with_request_id(
    client: *const FfiClient,
    operation: u32,
    application_key: *const u8,
    application_key_length: usize,
    value: *const u8,
    value_length: usize,
    set_condition: u32,
    ttl_enabled: u8,
    ttl_ms: u64,
    request_id: Option<u64>,
    mutation_id: *const u8,
    mutation_id_length: usize,
    raw: bool,
) -> *mut FfiResult {
    let caller_operation = FfiOperation::try_from(operation).ok();
    boxed_result(catch_result(caller_operation, || {
        let client = unsafe {
            client
                .as_ref()
                .ok_or_else(|| "client pointer must not be null".to_owned())?
        };
        if request_id == Some(0) {
            return Err("request ID must be greater than zero".to_owned());
        }
        let application_key =
            copy_bytes(application_key, application_key_length, "application_key")?;
        let value = copy_bytes(value, value_length, "value")?;
        let mutation_id = copy_mutation_id(mutation_id, mutation_id_length)?;
        let operation = FfiOperation::try_from(operation)
            .map_err(|operation| format!("unsupported operation {operation}"))?;
        if raw && matches!(operation, FfiOperation::GetJson | FfiOperation::SetJson) {
            return Err("exact item-ID calls do not support formatted JSON operations".to_owned());
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
        if let Some(mutation_id) = mutation_id {
            if !matches!(
                operation,
                FfiOperation::Set | FfiOperation::SetJson | FfiOperation::Delete
            ) {
                return Err("mutation IDs are valid only for SET, SET_JSON, and DELETE".to_owned());
            }
            set_options = set_options.with_mutation_id(mutation_id);
        }
        match operation {
            FfiOperation::Get
            | FfiOperation::Set
            | FfiOperation::GetJson
            | FfiOperation::SetJson
            | FfiOperation::Delete
                if !raw && application_key.is_empty() =>
            {
                Err("application key must not be empty".to_owned())
            }
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
            | FfiOperation::Delete
            | FfiOperation::Stats
            | FfiOperation::Sync
            | FfiOperation::Reconnect
                if !value.is_empty() =>
            {
                Err("operation does not accept a value".to_owned())
            }
            operation
                if !matches!(operation, FfiOperation::Set | FfiOperation::SetJson)
                    && (set_options.condition() != SetCondition::None
                        || set_options.time_to_live_millis().is_some()) =>
            {
                Err("SET options require a SET operation".to_owned())
            }
            _ => Ok(match request_id {
                Some(request_id) => client.execute_with_request_id(
                    request_id,
                    operation,
                    application_key,
                    value,
                    set_options,
                    raw,
                ),
                None => client.execute(operation, application_key, value, set_options, raw),
            }),
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

/// Requests cancellation of a queued or active operation.
///
/// Returns one when the request was found and cancellation was recorded, and zero when it had
/// already completed or a cancellation was already requested. The canceled operation's result is
/// delivered through its normal result pointer with structured cancellation metadata.
///
/// # Safety
///
/// `client` must be null or a live pointer returned by [`openkache_client_result_take_client`]
/// and must remain valid until this call returns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_client_cancel(client: *const FfiClient, request_id: u64) -> u8 {
    unsafe { client.as_ref() }.map_or(0, |client| u8::from(client.cancel(request_id)))
}

/// Copies a metrics snapshot into caller-owned storage.
///
/// Returns one on success and zero for null pointers.
///
/// # Safety
///
/// `client` must be null or a live pointer returned by [`openkache_client_result_take_client`].
/// When non-null, `snapshot` must point to writable storage for one [`FfiMetricsSnapshot`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_client_metrics_snapshot(
    client: *const FfiClient,
    snapshot: *mut FfiMetricsSnapshot,
) -> u8 {
    let (Some(client), Some(snapshot)) = (unsafe { client.as_ref() }, unsafe { snapshot.as_mut() })
    else {
        return 0;
    };
    *snapshot = client.metrics_snapshot();
    1
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

/// Copies structured error metadata from a result.
///
/// Returns one when `result` is an error, zero for null pointers or successful results.
///
/// # Safety
///
/// `result` must be null or a live pointer returned by this library. When non-null, `metadata`
/// must point to writable storage for one [`FfiErrorMetadata`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_client_result_error_metadata(
    result: *const FfiResult,
    metadata: *mut FfiErrorMetadata,
) -> u8 {
    let (Some(result), Some(metadata)) = (unsafe { result.as_ref() }, unsafe { metadata.as_mut() })
    else {
        return 0;
    };
    if result.kind != FfiResultKind::Error {
        return 0;
    }
    *metadata = result.metadata;
    1
}

/// Moves a connected client handle out of an FFI result.
///
/// The result remains valid and may be freed after this function returns. Calling this function
/// more than once returns null.
///
/// # Safety
///
/// `result` must be null or a unique, live pointer returned by
/// [`openkache_client_connect_with_options`].
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

fn catch_result(
    caller_operation: Option<FfiOperation>,
    operation: impl FnOnce() -> std::result::Result<FfiResult, String>,
) -> FfiResult {
    match catch_unwind(AssertUnwindSafe(operation)) {
        Ok(Ok(result)) => result,
        Ok(Err(error)) => {
            FfiResult::error_with_operation(error, FFI_ERROR_CONFIGURATION, caller_operation)
        }
        Err(_) => FfiResult::error_with_operation(
            "native client panicked",
            crate::contract::FFI_ERROR_RUNTIME,
            caller_operation,
        ),
    }
}

fn error_metadata(error: &crate::Error, caller_operation: FfiOperation) -> FfiErrorMetadata {
    let mut metadata = FfiErrorMetadata {
        operation: ffi_operation_code(caller_operation),
        ..FfiErrorMetadata::default()
    };
    match error {
        crate::Error::Configuration { .. } => metadata.code = FFI_ERROR_CONFIGURATION,
        crate::Error::Connection(_) => {
            metadata.code = FFI_ERROR_CONNECTION;
            metadata.retryable = 1;
        }
        crate::Error::Timeout { operation } => {
            metadata.code = FFI_ERROR_TIMEOUT;
            metadata.phase = operation.ffi_phase_code();
            metadata.retryable = 1;
        }
        crate::Error::Runtime { backend, .. } => {
            metadata.code = FFI_ERROR_RUNTIME;
            metadata.backend = backend.ffi_code();
        }
        crate::Error::Transport {
            backend, operation, ..
        } => {
            metadata.code = FFI_ERROR_TRANSPORT;
            metadata.backend = backend.ffi_code();
            metadata.phase = operation.ffi_phase_code();
            metadata.retryable = 1;
        }
        crate::Error::Server { code, .. } => {
            metadata.code = FFI_ERROR_SERVER;
            metadata.retryable = u8::from(matches!(
                code,
                crate::ServerErrorCode::Overloaded | crate::ServerErrorCode::Timeout
            ));
        }
        crate::Error::UnexpectedResponse { operation, .. } => {
            metadata.code = FFI_ERROR_UNEXPECTED_RESPONSE;
            metadata.phase = operation.ffi_phase_code();
        }
        crate::Error::ResponseTooLarge { .. } => metadata.code = FFI_ERROR_RESPONSE_TOO_LARGE,
        crate::Error::Tls(_) => metadata.code = FFI_ERROR_TLS,
        crate::Error::Protocol(_) => metadata.code = FFI_ERROR_PROTOCOL,
        crate::Error::Io(_) => {
            metadata.code = FFI_ERROR_IO;
            metadata.retryable = 1;
        }
        crate::Error::Value(_) => metadata.code = FFI_ERROR_VALUE,
        crate::Error::ClientClosed => metadata.code = FFI_ERROR_CLOSED,
        crate::Error::AmbiguousOutcome {
            operation: _,
            mutation_id,
            cause,
        } => {
            metadata.code = FFI_ERROR_AMBIGUOUS;
            metadata.ambiguous = 1;
            metadata.retryable = 1;
            if let Some(mutation_id) = mutation_id {
                metadata.mutation_id_length = MUTATION_ID_BYTES as u8;
                metadata.mutation_id = mutation_id.into_bytes();
            }
            let nested = error_metadata(cause, caller_operation);
            metadata.phase = nested.phase;
            metadata.backend = nested.backend;
        }
    }
    metadata
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
    let owned = Zeroizing::new(bytes.to_vec());
    DataProtectionKey::from_slice(&owned).map_err(|error| error.to_string())
}

fn copy_data_protection_key_ring(
    active_pointer: *const u8,
    active_length: usize,
    previous_pointer: *const u8,
    previous_length: usize,
    previous_count: usize,
) -> std::result::Result<DataProtectionKeyRing, String> {
    let active = copy_data_protection_key(active_pointer, active_length)?;
    if previous_count > MAX_PREVIOUS_DATA_PROTECTION_KEYS {
        return Err(format!(
            "data protection key ring retains at most {MAX_PREVIOUS_DATA_PROTECTION_KEYS} previous keys"
        ));
    }
    let key_width = crate::DATA_PROTECTION_KEY_BYTES;
    let expected_length = previous_count
        .checked_mul(key_width)
        .ok_or_else(|| "previous data protection key length overflows usize".to_owned())?;
    if previous_length != expected_length {
        return Err(format!(
            "previous data protection keys must contain {expected_length} bytes for {previous_count} keys, got {previous_length}"
        ));
    }
    if previous_length != 0 && previous_pointer.is_null() {
        return Err(format!(
            "previous data protection key pointer is null for {previous_length} bytes"
        ));
    }
    let owned = if previous_length == 0 {
        Zeroizing::new(Vec::new())
    } else {
        Zeroizing::new(
            unsafe { std::slice::from_raw_parts(previous_pointer, previous_length) }.to_vec(),
        )
    };
    let mut previous = Vec::with_capacity(previous_count);
    for chunk in owned.chunks_exact(key_width) {
        previous.push(DataProtectionKey::from_slice(chunk).map_err(|error| error.to_string())?);
    }
    DataProtectionKeyRing::with_previous(active, previous).map_err(|error| error.to_string())
}

fn copy_secret_bytes(
    pointer: *const u8,
    length: usize,
    name: &str,
) -> std::result::Result<Zeroizing<Vec<u8>>, String> {
    if length == 0 {
        return Ok(Zeroizing::new(Vec::new()));
    }
    if pointer.is_null() {
        return Err(format!("{name} pointer is null for {length} bytes"));
    }
    Ok(Zeroizing::new(
        unsafe { std::slice::from_raw_parts(pointer, length) }.to_vec(),
    ))
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

fn copy_mutation_id(
    pointer: *const u8,
    length: usize,
) -> std::result::Result<Option<MutationId>, String> {
    if length == 0 {
        return Ok(None);
    }
    if length != MUTATION_ID_BYTES {
        return Err(format!(
            "mutation ID must contain exactly {MUTATION_ID_BYTES} bytes, got {length}"
        ));
    }
    if pointer.is_null() {
        return Err(format!(
            "mutation ID pointer is null for {MUTATION_ID_BYTES} bytes"
        ));
    }
    let bytes = unsafe { std::slice::from_raw_parts(pointer, length) };
    let exact: [u8; MUTATION_ID_BYTES] = bytes
        .try_into()
        .map_err(|_| "mutation ID length validation failed".to_owned())?;
    Ok(Some(MutationId::new(exact)))
}
