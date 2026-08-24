//! Stable C ABI shared by native language bindings.
//!
//! The ABI owns one Compio runtime and one protected client per native handle.  C, C++, and
//! other native bindings only marshal buffers and interpret result discriminators; connection
//! management, retries, protocol framing, and value protection remain in this crate.

use std::future::Future;
use std::io::{self, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TryRecvError, sync_channel};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[cfg(feature = "tls-tcp")]
use crate::TlsTcpProtectedClient;
pub use crate::contract::FFI_ABI_VERSION as ABI_VERSION;
pub use crate::contract::FfiNamespaceDescriptor;
pub use crate::contract::FfiOperationField;
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
    FfiErrorCategory, FfiKeySpec, FfiOperation, FfiRequestState, FfiResultKind, FfiSetCondition,
    FfiStatusCategory, FfiTransport, FfiValueMode, FfiValueRepresentation,
};
use crate::contract::{
    VALUE_FORMAT_ENCRYPTION_COMPACT, VALUE_FORMAT_ENCRYPTION_NONE, VALUE_FORMAT_ENCRYPTION_ROBUST,
};
use crate::ffi_admission::{AdmissionState, FfiAdmission};
use crate::transport::BytePermit;
use crate::value::{
    Compression, Encryption, JsonValue, Resource, Value, ValueKeyring, ValueLimits,
    ZstandardOptions,
};
use crate::{
    Certificate, ClientIdentity, ClientRootKey, ClientTimeouts, ConnectionState, DataProtectionKey,
    DeleteOutcome, Endpoint, EvictionDefault, ExpirationDefault, GetOutcome, ItemId, ItemValue,
    KeySpace, KeyType, LocalProtectedClient, NamespacePolicy, OverridePolicy, PrivateKey,
    RetryPolicy, ServerTrust, SetCondition, SetOptions, SetOutcome,
};
const COMMAND_QUEUE_CAPACITY: usize = 64;

/// Opaque result allocated by the native ABI.
pub struct FfiResult {
    kind: FfiResultKind,
    status: FfiStatusCategory,
    error_category: FfiErrorCategory,
    payload: Vec<u8>,
    _payload_permits: Vec<BytePermit>,
    client: Option<Box<FfiClient>>,
}

/// Owned asynchronous operation handle allocated by the native ABI.
///
/// A request owns copied inputs and exactly one completion receiver. It does
/// not borrow the client after submission; closing the client completes an
/// outstanding request with a structured closed error.
pub struct FfiRequest {
    control: Arc<FfiRequestControl>,
    receiver: Mutex<Option<Receiver<FfiResult>>>,
    ready: Mutex<Option<FfiResult>>,
    state: AtomicU32,
}

/// Shared admission and cancellation state observed by the one-engine worker.
struct FfiRequestControl {
    admission: FfiAdmission,
    mutating: bool,
}

/// Native connection options passed by C and C++ bindings.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FfiConnectOptions {
    /// UTF-8 host and transport port such as `127.0.0.1:4433` or `cache.example.com:4433`.
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
    /// Non-zero to enable automatic level-1 Zstandard compression. Zero is an
    /// explicit uncompressed opt-out.
    pub compression_enabled: u8,
    /// Zstandard level, validated by the shared value codec.
    pub compression_level: i32,
    /// Optional minimum serialized input size; zero selects the maintained
    /// no-threshold default.
    pub minimum_input_size: usize,
    /// Optional minimum compressed-byte savings; zero selects the maintained
    /// no-threshold default.
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

#[repr(C)]
#[derive(Clone, Copy)]
pub struct FfiValueKey {
    pub id: u64,
    pub key: *const u8,
    pub key_length: usize,
}

/// ABI v1 options with independent Item-ID root and value keyring.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FfiConnectOptionsWithKeyring {
    pub abi_version: u32,
    pub base: *const FfiConnectOptions,
    pub item_id_root_key: *const u8,
    pub item_id_root_key_length: usize,
    pub value_keys: *const FfiValueKey,
    pub value_key_count: usize,
    pub active_value_key_id: u64,
    pub value_encryption: u32,
}

/// Opaque native handle to a dedicated Rust client worker.
pub struct FfiClient {
    commands: CommandSender,
    request_timeout: Duration,
    request_budget: crate::RequestBudget,
    shutdown: Arc<AtomicBool>,
    state: Arc<AtomicU32>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

enum Command {
    Execute {
        operation: FfiOperation,
        application_key: Vec<u8>,
        value: Vec<u8>,
        input_permit: BytePermit,
        set_options: SetOptions,
        raw: bool,
        response: SyncSender<FfiResult>,
    },
    ExecuteStructured {
        operation: FfiOperation,
        canonical_key: Vec<u8>,
        value: Vec<u8>,
        set_options: SetOptions,
        response: SyncSender<FfiResult>,
    },
    ExecuteScoped {
        operation: FfiOperation,
        namespace_id: u64,
        item_id: Vec<u8>,
        value: Vec<u8>,
        input_permit: BytePermit,
        set_options: SetOptions,
        response: SyncSender<FfiResult>,
    },
    ExecuteAsync {
        operation: FfiOperation,
        application_key: Vec<u8>,
        value: Vec<u8>,
        input_permit: BytePermit,
        set_options: SetOptions,
        raw: bool,
        request: Arc<FfiRequestControl>,
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
type CommandReceiver = crossfire::AsyncRx<crossfire::mpsc::Array<Command>>;

struct WorkerOptions {
    transport: TransportSelection,
    endpoint: Endpoint,
    certificate: Vec<u8>,
    item_id_root: Option<DataProtectionKey>,
    value_keyring: Option<ValueKeyring>,
    client_certificate_chain: Vec<u8>,
    client_private_key: Vec<u8>,
    compression: Compression,
    encryption: Encryption,
    timeouts: ClientTimeouts,
    retry: RetryPolicy,
    max_in_flight: usize,
}

/// Internal transport selector used by the additive ABI entry point.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TransportSelection {
    Quic { verify_server: bool },
    TlsTcp { verify_server: bool },
}

impl TransportSelection {
    fn from_code(code: u32) -> std::result::Result<Self, String> {
        match FfiTransport::try_from(code) {
            Ok(FfiTransport::Quic) => Ok(Self::Quic {
                verify_server: true,
            }),
            Ok(FfiTransport::TlsTcp) => Ok(Self::TlsTcp {
                verify_server: true,
            }),
            Ok(FfiTransport::QuicInsecure) => Ok(Self::Quic {
                verify_server: false,
            }),
            Ok(FfiTransport::TlsTcpInsecure) => Ok(Self::TlsTcp {
                verify_server: false,
            }),
            Err(value) => Err(format!("unsupported transport selector {value}")),
        }
    }
}

impl FfiResult {
    fn error(message: impl Into<String>) -> Self {
        Self::error_with_category(message, FfiErrorCategory::Internal)
    }

    fn error_with_category(message: impl Into<String>, error_category: FfiErrorCategory) -> Self {
        Self {
            kind: FfiResultKind::Error,
            status: if error_category == FfiErrorCategory::ResourceExhausted {
                FfiStatusCategory::ResourceExhausted
            } else if error_category == FfiErrorCategory::Canceled {
                FfiStatusCategory::Canceled
            } else if error_category == FfiErrorCategory::UnknownMutation {
                FfiStatusCategory::UnknownMutation
            } else {
                FfiStatusCategory::Error
            },
            error_category,
            payload: message.into().into_bytes(),
            _payload_permits: Vec::new(),
            client: None,
        }
    }

    fn success(kind: FfiResultKind, payload: Vec<u8>) -> Self {
        let status = match kind {
            FfiResultKind::NotFound => FfiStatusCategory::NotFound,
            FfiResultKind::Created
            | FfiResultKind::Replaced
            | FfiResultKind::Deleted
            | FfiResultKind::NotDeleted
            | FfiResultKind::NotStored => FfiStatusCategory::Mutation,
            FfiResultKind::Canceled => FfiStatusCategory::Canceled,
            FfiResultKind::UnknownMutation => FfiStatusCategory::UnknownMutation,
            FfiResultKind::ResourceExhausted => FfiStatusCategory::ResourceExhausted,
            FfiResultKind::Error => FfiStatusCategory::Error,
            _ => FfiStatusCategory::Success,
        };
        Self {
            kind,
            status,
            error_category: FfiErrorCategory::None,
            payload,
            _payload_permits: Vec::new(),
            client: None,
        }
    }

    fn with_status(
        kind: FfiResultKind,
        status: FfiStatusCategory,
        error_category: FfiErrorCategory,
        payload: Vec<u8>,
    ) -> Self {
        Self {
            kind,
            status,
            error_category,
            payload,
            _payload_permits: Vec::new(),
            client: None,
        }
    }

    fn success_with_permits(
        kind: FfiResultKind,
        payload: Vec<u8>,
        payload_permits: Vec<BytePermit>,
    ) -> Self {
        let status = match kind {
            FfiResultKind::NotFound => FfiStatusCategory::NotFound,
            FfiResultKind::Created
            | FfiResultKind::Replaced
            | FfiResultKind::Deleted
            | FfiResultKind::NotDeleted
            | FfiResultKind::NotStored => FfiStatusCategory::Mutation,
            FfiResultKind::Canceled => FfiStatusCategory::Canceled,
            FfiResultKind::UnknownMutation => FfiStatusCategory::UnknownMutation,
            FfiResultKind::ResourceExhausted => FfiStatusCategory::ResourceExhausted,
            FfiResultKind::Error => FfiStatusCategory::Error,
            _ => FfiStatusCategory::Success,
        };
        Self {
            kind,
            status,
            error_category: FfiErrorCategory::None,
            payload,
            _payload_permits: payload_permits,
            client: None,
        }
    }

    fn from_error(error: crate::Error) -> Self {
        let category = match &error {
            crate::Error::Configuration { .. } => FfiErrorCategory::Configuration,
            crate::Error::Connection(_) | crate::Error::Runtime { .. } => {
                FfiErrorCategory::Transport
            }
            crate::Error::Timeout { .. } => FfiErrorCategory::Timeout,
            crate::Error::Transport { .. } | crate::Error::Io(_) => FfiErrorCategory::Transport,
            crate::Error::Server { code, .. } => {
                if matches!(
                    code.as_u8(),
                    value if value == openkache_protocol::Status::TooLarge as u8
                        || value == openkache_protocol::Status::Overloaded as u8
                        || value == openkache_protocol::Status::NoCapacity as u8
                ) {
                    FfiErrorCategory::ResourceExhausted
                } else {
                    FfiErrorCategory::Server
                }
            }
            crate::Error::UnexpectedResponse { .. } | crate::Error::Protocol(_) => {
                FfiErrorCategory::Protocol
            }
            crate::Error::ResponseTooLarge { .. } => FfiErrorCategory::ResourceExhausted,
            crate::Error::ResourceLimit { .. } => FfiErrorCategory::ResourceExhausted,
            crate::Error::Tls(_) => FfiErrorCategory::Transport,
            crate::Error::Value(_) => FfiErrorCategory::Value,
            crate::Error::Key(_) => FfiErrorCategory::Key,
            crate::Error::ClientClosed => FfiErrorCategory::Closed,
            crate::Error::AmbiguousOutcome { .. } => FfiErrorCategory::UnknownMutation,
        };
        let status = if category == FfiErrorCategory::UnknownMutation {
            FfiStatusCategory::UnknownMutation
        } else if category == FfiErrorCategory::ResourceExhausted {
            FfiStatusCategory::ResourceExhausted
        } else {
            FfiStatusCategory::Error
        };
        let kind = if category == FfiErrorCategory::UnknownMutation {
            FfiResultKind::UnknownMutation
        } else if category == FfiErrorCategory::ResourceExhausted {
            FfiResultKind::ResourceExhausted
        } else {
            FfiResultKind::Error
        };
        Self::with_status(kind, status, category, error.to_string().into_bytes())
    }

    fn connected(client: FfiClient) -> Self {
        Self {
            kind: FfiResultKind::Connected,
            status: FfiStatusCategory::Success,
            error_category: FfiErrorCategory::None,
            payload: Vec::new(),
            _payload_permits: Vec::new(),
            client: Some(Box::new(client)),
        }
    }
}

impl FfiClient {
    // The argument list mirrors the stable native connection contract.
    #[allow(clippy::too_many_arguments)]
    fn connect(
        transport: TransportSelection,
        endpoint: Endpoint,
        certificate: Vec<u8>,
        item_id_root: Option<DataProtectionKey>,
        value_keyring: Option<ValueKeyring>,
        client_certificate_chain: Vec<u8>,
        client_private_key: Vec<u8>,
        compression: Compression,
        encryption: Encryption,
        timeouts: ClientTimeouts,
        retry: RetryPolicy,
        max_in_flight: usize,
    ) -> std::result::Result<Self, String> {
        let (commands, receiver) =
            crossfire::mpsc::bounded_blocking_async(COMMAND_QUEUE_CAPACITY);
        let (ready_sender, ready_receiver) = sync_channel(1);
        let shutdown = Arc::new(AtomicBool::new(false));
        let worker_shutdown = Arc::clone(&shutdown);
        let state = Arc::new(AtomicU32::new(connection_state_value(
            ConnectionState::Reconnecting,
        )));
        let worker_state = Arc::clone(&state);
        let options = WorkerOptions {
            transport,
            endpoint,
            certificate,
            item_id_root,
            value_keyring,
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
            Ok(Ok(request_budget)) => Ok(Self {
                commands,
                request_timeout: timeouts.request,
                request_budget,
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
        input_permit: BytePermit,
        set_options: SetOptions,
        raw: bool,
    ) -> FfiResult {
        let (response, receiver) = sync_channel(1);
        let Some(deadline) = Instant::now().checked_add(self.request_timeout) else {
            return FfiResult::error_with_category(
                "client request timeout exceeds the platform clock range",
                FfiErrorCategory::Timeout,
            );
        };
        let command = Command::Execute {
            operation,
            application_key,
            value,
            input_permit,
            set_options,
            raw,
            response,
        };
        let remaining = deadline.saturating_duration_since(Instant::now());
        if let Err(error) = self.commands.send_timeout(command, remaining) {
            return FfiResult::error_with_category(
                format!("client worker queue deadline exceeded: {error}"),
                FfiErrorCategory::ResourceExhausted,
            );
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        receiver.recv_timeout(remaining).unwrap_or_else(|error| {
            FfiResult::error_with_category(
                format!("client operation timed out: {error}"),
                FfiErrorCategory::Timeout,
            )
        })
    }

    fn execute_structured(
        &self,
        operation: FfiOperation,
        canonical_key: Vec<u8>,
        value: Vec<u8>,
        set_options: SetOptions,
    ) -> FfiResult {
        self.send_command_with_response(|response| Command::ExecuteStructured {
            operation,
            canonical_key,
            value,
            set_options,
            response,
        })
    }

    fn execute_async(
        &self,
        operation: FfiOperation,
        application_key: Vec<u8>,
        value: Vec<u8>,
        input_permit: BytePermit,
        set_options: SetOptions,
        raw: bool,
    ) -> FfiRequest {
        let (response, receiver) = sync_channel(1);
        let control = Arc::new(FfiRequestControl {
            admission: FfiAdmission::new(),
            mutating: matches!(
                operation,
                FfiOperation::Set
                    | FfiOperation::SetJson
                    | FfiOperation::SetStructured
                    | FfiOperation::SetV0
                    | FfiOperation::Delete
            ),
        });
        let request = FfiRequest {
            control: Arc::clone(&control),
            receiver: Mutex::new(Some(receiver)),
            ready: Mutex::new(None),
            state: AtomicU32::new(FfiRequestState::Pending.code()),
        };
        let Some(deadline) = Instant::now().checked_add(self.request_timeout) else {
            *request
                .ready
                .lock()
                .expect("request ready lock is not poisoned") =
                Some(FfiResult::error_with_category(
                    "client request timeout exceeds the platform clock range",
                    FfiErrorCategory::Timeout,
                ));
            request
                .state
                .store(FfiRequestState::Ready.code(), Ordering::Release);
            return request;
        };
        let command = Command::ExecuteAsync {
            operation,
            application_key,
            value,
            input_permit,
            set_options,
            raw,
            request: control,
            response,
        };
        let remaining = deadline.saturating_duration_since(Instant::now());
        if let Err(error) = self.commands.send_timeout(command, remaining) {
            *request
                .ready
                .lock()
                .expect("request ready lock is not poisoned") =
                Some(FfiResult::error_with_category(
                    format!("client worker queue deadline exceeded: {error}"),
                    FfiErrorCategory::ResourceExhausted,
                ));
            request
                .state
                .store(FfiRequestState::Ready.code(), Ordering::Release);
        }
        request
    }

    fn execute_scoped(
        &self,
        operation: FfiOperation,
        namespace_id: u64,
        item_id: Vec<u8>,
        value: Vec<u8>,
        input_permit: BytePermit,
        set_options: SetOptions,
    ) -> FfiResult {
        self.send_command_with_response(|response| Command::ExecuteScoped {
            operation,
            namespace_id,
            item_id,
            value,
            input_permit,
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
            return FfiResult::error_with_category(
                "client request timeout exceeds the platform clock range",
                FfiErrorCategory::Timeout,
            );
        };
        let command = build(response);
        let remaining = deadline.saturating_duration_since(Instant::now());
        if let Err(error) = self.commands.send_timeout(command, remaining) {
            return FfiResult::error_with_category(
                format!("client worker queue deadline exceeded: {error}"),
                FfiErrorCategory::ResourceExhausted,
            );
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        receiver.recv_timeout(remaining).unwrap_or_else(|error| {
            FfiResult::error_with_category(
                format!("client operation timed out: {error}"),
                FfiErrorCategory::Timeout,
            )
        })
    }

    fn connection_state(&self) -> u32 {
        self.state.load(Ordering::Acquire)
    }

    fn request_budget(&self) -> crate::RequestBudget {
        self.request_budget.clone()
    }
}

impl FfiRequest {
    /// Builds a request which has already completed before it could be
    /// submitted to the shared worker.
    ///
    /// Validation errors are represented by a normal result handle so native
    /// callers can use one poll/wait/free lifecycle for both submitted and
    /// rejected requests.
    fn completed(result: FfiResult) -> Self {
        Self {
            control: Arc::new(FfiRequestControl {
                admission: FfiAdmission::new(),
                mutating: false,
            }),
            receiver: Mutex::new(None),
            ready: Mutex::new(Some(result)),
            state: AtomicU32::new(FfiRequestState::Ready.code()),
        }
    }

    fn poll(&self) -> FfiRequestState {
        let current = self.state.load(Ordering::Acquire);
        if current != FfiRequestState::Pending.code() {
            return FfiRequestState::try_from(current).unwrap_or(FfiRequestState::Freed);
        }
        if self
            .ready
            .lock()
            .expect("request ready lock is not poisoned")
            .is_some()
        {
            self.state
                .store(FfiRequestState::Ready.code(), Ordering::Release);
            return FfiRequestState::Ready;
        }
        let receiver_guard = self
            .receiver
            .lock()
            .expect("request receiver lock is not poisoned");
        let Some(receiver) = receiver_guard.as_ref() else {
            return FfiRequestState::Consumed;
        };
        match receiver.try_recv() {
            Ok(result) => {
                *self
                    .ready
                    .lock()
                    .expect("request ready lock is not poisoned") = Some(result);
                self.state
                    .store(FfiRequestState::Ready.code(), Ordering::Release);
                FfiRequestState::Ready
            }
            Err(TryRecvError::Empty) => FfiRequestState::Pending,
            Err(TryRecvError::Disconnected) => {
                *self
                    .ready
                    .lock()
                    .expect("request ready lock is not poisoned") =
                    Some(FfiResult::error_with_category(
                        "request completion channel closed",
                        FfiErrorCategory::Closed,
                    ));
                self.state
                    .store(FfiRequestState::Ready.code(), Ordering::Release);
                FfiRequestState::Ready
            }
        }
    }

    fn wait(&self, timeout: Duration) -> FfiResult {
        if let Some(result) = self
            .ready
            .lock()
            .expect("request ready lock is not poisoned")
            .take()
        {
            self.state
                .store(FfiRequestState::Consumed.code(), Ordering::Release);
            return result;
        }
        let receiver_guard = self
            .receiver
            .lock()
            .expect("request receiver lock is not poisoned");
        let Some(receiver) = receiver_guard.as_ref() else {
            return FfiResult::error_with_category(
                "request result was already consumed",
                FfiErrorCategory::Closed,
            );
        };
        match receiver.recv_timeout(timeout) {
            Ok(result) => {
                drop(receiver_guard);
                // Remove the receiver only after a result has been received.
                // A timeout must leave the request pending so callers can
                // poll or wait again without losing the completion channel.
                self.receiver
                    .lock()
                    .expect("request receiver lock is not poisoned")
                    .take();
                self.state
                    .store(FfiRequestState::Consumed.code(), Ordering::Release);
                result
            }
            Err(RecvTimeoutError::Timeout) => {
                FfiResult::error_with_category("request wait timed out", FfiErrorCategory::Timeout)
            }
            Err(RecvTimeoutError::Disconnected) => {
                drop(receiver_guard);
                self.receiver
                    .lock()
                    .expect("request receiver lock is not poisoned")
                    .take();
                self.state
                    .store(FfiRequestState::Consumed.code(), Ordering::Release);
                FfiResult::error_with_category(
                    "request completion channel closed",
                    FfiErrorCategory::Closed,
                )
            }
        }
    }

    fn cancel(&self) -> FfiRequestState {
        if self.state.load(Ordering::Acquire) != FfiRequestState::Pending.code() {
            return FfiRequestState::try_from(self.state.load(Ordering::Acquire))
                .unwrap_or(FfiRequestState::Freed);
        }

        // Publish cancellation against the worker's admission claim before
        // exposing Canceled through the public request state. If this CAS wins
        // while Pending, the worker cannot subsequently start the mutation.
        // If Started wins first, the state records StartedCanceled and the
        // documented UnknownMutation boundary is preserved.
        let admission = self.control.admission.cancel();
        // Completion is a terminal admission boundary. Do not publish a
        // cancellation result after the worker has produced a definitive
        // operation result; the completion sender owns that result.
        if admission == AdmissionState::Completed {
            return FfiRequestState::try_from(self.state.load(Ordering::Acquire))
                .unwrap_or(FfiRequestState::Freed);
        }
        if self
            .state
            .compare_exchange(
                FfiRequestState::Pending.code(),
                FfiRequestState::Canceled.code(),
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
        {
            *self
                .ready
                .lock()
                .expect("request ready lock is not poisoned") = Some(
                if self.control.mutating
                    && matches!(
                        admission,
                        AdmissionState::StartedCanceled | AdmissionState::CompletedCanceled
                    )
                {
                    FfiResult::with_status(
                        FfiResultKind::UnknownMutation,
                        FfiStatusCategory::UnknownMutation,
                        FfiErrorCategory::UnknownMutation,
                        b"mutation outcome is unknown after cancellation".to_vec(),
                    )
                } else {
                    FfiResult::with_status(
                        FfiResultKind::Canceled,
                        FfiStatusCategory::Canceled,
                        FfiErrorCategory::Canceled,
                        b"request canceled".to_vec(),
                    )
                },
            );
            return FfiRequestState::Canceled;
        }
        FfiRequestState::try_from(self.state.load(Ordering::Acquire))
            .unwrap_or(FfiRequestState::Freed)
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
    ready: SyncSender<std::result::Result<crate::RequestBudget, String>>,
    options: WorkerOptions,
    shutdown: Arc<AtomicBool>,
    state: Arc<AtomicU32>,
) {
    match options.transport {
        TransportSelection::Quic { .. } => {
            run_quic_worker(commands, ready, options, shutdown, state)
        }
        TransportSelection::TlsTcp { .. } => {
            run_tls_tcp_worker(commands, ready, options, shutdown, state)
        }
    }
}

fn run_quic_worker(
    commands: CommandReceiver,
    ready: SyncSender<std::result::Result<crate::RequestBudget, String>>,
    options: WorkerOptions,
    shutdown: Arc<AtomicBool>,
    state: Arc<AtomicU32>,
) {
    let WorkerOptions {
        transport,
        endpoint,
        certificate,
        item_id_root,
        value_keyring,
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
    let verify_server = match transport {
        TransportSelection::Quic { verify_server } => verify_server,
        TransportSelection::TlsTcp { .. } => unreachable!("transport dispatch selected QUIC"),
    };
    let protected = item_id_root.is_some();
    let mut builder = match item_id_root {
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
    if let Some(keyring) = value_keyring {
        builder = builder.value_keyring(keyring);
    }
    if !verify_server {
        // The explicit insecure selector takes precedence over any supplied
        // certificate bytes.  A caller that opts out of verification must not
        // accidentally regain certificate validation merely by leaving a
        // legacy trust buffer populated.
        builder = builder.server_trust(ServerTrust::Insecure);
    } else if !certificate.is_empty() {
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
    if ready.send(Ok(client.request_budget())).is_err() {
        return;
    }

    run_command_loop(&runtime, client, commands, shutdown, state, true);
}

fn run_tls_tcp_worker(
    commands: CommandReceiver,
    ready: SyncSender<std::result::Result<crate::RequestBudget, String>>,
    options: WorkerOptions,
    shutdown: Arc<AtomicBool>,
    state: Arc<AtomicU32>,
) {
    let WorkerOptions {
        transport,
        endpoint,
        certificate,
        item_id_root,
        value_keyring,
        client_certificate_chain,
        client_private_key,
        compression,
        encryption,
        timeouts,
        retry,
        max_in_flight,
    } = options;
    let verify_server = match transport {
        TransportSelection::TlsTcp { verify_server } => verify_server,
        TransportSelection::Quic { .. } => unreachable!("transport dispatch selected TLS/TCP"),
    };
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            let _ = ready.send(Err(format!("failed to create Tokio runtime: {error}")));
            return;
        }
    };
    let trust = if !verify_server {
        ServerTrust::Insecure
    } else if certificate.is_empty() {
        ServerTrust::System
    } else {
        match Certificate::from_der_or_pem_chain(&certificate) {
            Ok(certificates) => ServerTrust::Custom(certificates),
            Err(error) => {
                let _ = ready.send(Err(error.to_string()));
                return;
            }
        }
    };
    let mut identity = if !client_certificate_chain.is_empty() || !client_private_key.is_empty() {
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
        match ClientIdentity::new(certificate_chain, private_key) {
            Ok(identity) => Some(identity),
            Err(error) => {
                let _ = ready.send(Err(error.to_string()));
                return;
            }
        }
    } else {
        None
    };
    let protected = item_id_root.is_some();
    let mut value_keyring = value_keyring;
    macro_rules! connect_builder {
        ($builder:expr) => {{
            let mut builder = $builder
                .compression(compression)
                .timeouts(timeouts)
                .retry_policy(retry)
                .max_in_flight(max_in_flight)
                .server_trust(trust.clone());
            if let Some(identity) = identity.take() {
                builder = builder.client_identity(identity);
            }
            if protected {
                builder = builder.encryption(encryption);
            }
            if let Some(keyring) = value_keyring.take() {
                builder = builder.value_keyring(keyring);
            }
            runtime.block_on(builder.connect())
        }};
    }
    let client = match item_id_root {
        Some(key) => connect_builder!(TlsTcpProtectedClient::builder(endpoint, key)),
        None => connect_builder!(TlsTcpProtectedClient::builder_unprotected(endpoint)),
    };
    let client = match client {
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
    if ready.send(Ok(client.request_budget())).is_err() {
        return;
    }
    run_command_loop(&runtime, client, commands, shutdown, state, false);
}

trait WorkerRuntime {
    fn block_on<F: Future>(&self, future: F) -> F::Output;

    fn spawn<F>(&self, future: F)
    where
        F: Future<Output = ()> + 'static;
}

impl WorkerRuntime for compio::runtime::Runtime {
    fn block_on<F: Future>(&self, future: F) -> F::Output {
        compio::runtime::Runtime::block_on(self, future)
    }

    fn spawn<F>(&self, future: F)
    where
        F: Future<Output = ()> + 'static,
    {
        compio::runtime::Runtime::spawn(self, future).detach();
    }
}

impl WorkerRuntime for tokio::runtime::Runtime {
    fn block_on<F: Future>(&self, future: F) -> F::Output {
        tokio::runtime::Runtime::block_on(self, future)
    }

    fn spawn<F>(&self, future: F)
    where
        F: Future<Output = ()> + 'static,
    {
        // Tokio's current-thread worker uses the synchronous command loop,
        // so TLS-over-TCP dispatch never calls this method.
        drop(future);
    }
}

trait FfiRawClientApi {
    async fn ping(&self) -> crate::Result<Duration>;
    async fn get(&self, item_id: ItemId) -> crate::Result<GetOutcome<ItemValue>>;
    async fn set(
        &self,
        item_id: ItemId,
        value: ItemValue,
        options: SetOptions,
    ) -> crate::Result<SetOutcome>;
    async fn delete(&self, item_id: ItemId) -> crate::Result<DeleteOutcome>;
    async fn experimental_stats(&self) -> crate::Result<String>;
    async fn experimental_sync(&self) -> crate::Result<()>;
    async fn reconnect(&self) -> crate::Result<()>;
    async fn get_in_namespace(
        &self,
        namespace_id: u64,
        item_id: ItemId,
    ) -> crate::Result<GetOutcome<ItemValue>>;
    async fn set_in_namespace(
        &self,
        namespace_id: u64,
        item_id: ItemId,
        value: ItemValue,
        options: SetOptions,
    ) -> crate::Result<SetOutcome>;
    async fn delete_in_namespace(
        &self,
        namespace_id: u64,
        item_id: ItemId,
    ) -> crate::Result<DeleteOutcome>;
    async fn experimental_stats_in_namespace(&self, namespace_id: u64) -> crate::Result<String>;
    async fn experimental_sync_in_namespace(&self, namespace_id: u64) -> crate::Result<()>;
    async fn namespace_open_with_outcome(
        &self,
        name: Vec<u8>,
        create_if_missing: bool,
        policy: Option<NamespacePolicy>,
    ) -> crate::Result<(crate::NamespaceDescriptor, bool)>;
    async fn namespace_update_policy(
        &self,
        namespace_id: u64,
        expected_revision: u64,
        policy: NamespacePolicy,
    ) -> crate::Result<crate::NamespaceDescriptor>;
    async fn namespace_delete(
        &self,
        namespace_id: u64,
        expected_revision: u64,
    ) -> crate::Result<()>;
}

macro_rules! impl_ffi_raw_client {
    ($client:ty) => {
        impl FfiRawClientApi for $client {
            async fn ping(&self) -> crate::Result<Duration> {
                self.ping().await
            }
            async fn get(&self, item_id: ItemId) -> crate::Result<GetOutcome<ItemValue>> {
                self.get(item_id).await
            }
            async fn set(
                &self,
                item_id: ItemId,
                value: ItemValue,
                options: SetOptions,
            ) -> crate::Result<SetOutcome> {
                self.set(item_id, value, options).await
            }
            async fn delete(&self, item_id: ItemId) -> crate::Result<DeleteOutcome> {
                self.delete(item_id).await
            }
            async fn experimental_stats(&self) -> crate::Result<String> {
                self.experimental_stats().await
            }
            async fn experimental_sync(&self) -> crate::Result<()> {
                self.experimental_sync().await
            }
            async fn reconnect(&self) -> crate::Result<()> {
                self.reconnect().await
            }
            async fn get_in_namespace(
                &self,
                namespace_id: u64,
                item_id: ItemId,
            ) -> crate::Result<GetOutcome<ItemValue>> {
                self.get_in_namespace(namespace_id, item_id).await
            }
            async fn set_in_namespace(
                &self,
                namespace_id: u64,
                item_id: ItemId,
                value: ItemValue,
                options: SetOptions,
            ) -> crate::Result<SetOutcome> {
                self.set_in_namespace(namespace_id, item_id, value, options)
                    .await
            }
            async fn delete_in_namespace(
                &self,
                namespace_id: u64,
                item_id: ItemId,
            ) -> crate::Result<DeleteOutcome> {
                self.delete_in_namespace(namespace_id, item_id).await
            }
            async fn experimental_stats_in_namespace(&self, namespace_id: u64) -> crate::Result<String> {
                self.experimental_stats_in_namespace(namespace_id).await
            }
            async fn experimental_sync_in_namespace(&self, namespace_id: u64) -> crate::Result<()> {
                self.experimental_sync_in_namespace(namespace_id).await
            }
            async fn namespace_open_with_outcome(
                &self,
                name: Vec<u8>,
                create_if_missing: bool,
                policy: Option<NamespacePolicy>,
            ) -> crate::Result<(crate::NamespaceDescriptor, bool)> {
                self.namespace_open_with_outcome(name, create_if_missing, policy)
                    .await
            }
            async fn namespace_update_policy(
                &self,
                namespace_id: u64,
                expected_revision: u64,
                policy: NamespacePolicy,
            ) -> crate::Result<crate::NamespaceDescriptor> {
                self.namespace_update_policy(namespace_id, expected_revision, policy)
                    .await
            }
            async fn namespace_delete(
                &self,
                namespace_id: u64,
                expected_revision: u64,
            ) -> crate::Result<()> {
                self.namespace_delete(namespace_id, expected_revision).await
            }
        }
    };
}

#[cfg(feature = "quic-compio")]
impl_ffi_raw_client!(crate::LocalRawClient);
#[cfg(feature = "tls-tcp")]
impl_ffi_raw_client!(crate::TlsTcpRawClient);

trait FfiProtectedClientApi: Clone + 'static {
    type Raw: FfiRawClientApi;

    fn raw(&self) -> &Self::Raw;
    fn request_budget(&self) -> crate::RequestBudget;
    fn value_limits(&self) -> ValueLimits;
    fn connection_state(&self) -> ConnectionState;
    async fn ping(&self) -> crate::Result<Duration>;
    async fn get_canonical_key_unchecked(
        &self,
        canonical_key: &[u8],
    ) -> crate::Result<GetOutcome<Value>>;
    async fn get_structured_canonical_key_cbor(
        &self,
        canonical_key: Vec<u8>,
    ) -> crate::Result<GetOutcome<Vec<u8>>>;
    async fn get_json_canonical_key_unchecked(
        &self,
        canonical_key: &[u8],
    ) -> crate::Result<GetOutcome<JsonValue>>;
    async fn get_structured_canonical_key_cbor_unchecked(
        &self,
        canonical_key: &[u8],
    ) -> crate::Result<GetOutcome<Vec<u8>>>;
    async fn get_v0_canonical_key_unchecked(
        &self,
        canonical_key: &[u8],
    ) -> crate::Result<GetOutcome<Vec<u8>>>;
    async fn set_canonical_key_unchecked(
        &self,
        canonical_key: &[u8],
        value: Value,
        options: SetOptions,
    ) -> crate::Result<SetOutcome>;
    async fn set_structured_canonical_key_cbor(
        &self,
        canonical_key: Vec<u8>,
        value: Vec<u8>,
        options: SetOptions,
    ) -> crate::Result<SetOutcome>;
    async fn set_json_canonical_key_unchecked(
        &self,
        canonical_key: &[u8],
        value: JsonValue,
        options: SetOptions,
    ) -> crate::Result<SetOutcome>;
    async fn set_structured_canonical_key_cbor_unchecked(
        &self,
        canonical_key: &[u8],
        value: &[u8],
        options: SetOptions,
    ) -> crate::Result<SetOutcome>;
    async fn set_v0_canonical_key_unchecked(
        &self,
        canonical_key: &[u8],
        value: Vec<u8>,
        options: SetOptions,
    ) -> crate::Result<SetOutcome>;
    async fn get_json_exact_item_id(
        &self,
        namespace_id: u64,
        item_id: ItemId,
    ) -> crate::Result<GetOutcome<JsonValue>>;
    async fn set_json_exact_item_id(
        &self,
        namespace_id: u64,
        item_id: ItemId,
        value: JsonValue,
        options: SetOptions,
    ) -> crate::Result<SetOutcome>;
    async fn get_structured_exact_item_id_cbor(
        &self,
        namespace_id: u64,
        item_id: ItemId,
    ) -> crate::Result<GetOutcome<Vec<u8>>>;
    async fn set_structured_exact_item_id_cbor(
        &self,
        namespace_id: u64,
        item_id: ItemId,
        value: Vec<u8>,
        options: SetOptions,
    ) -> crate::Result<SetOutcome>;
    async fn get_v0_exact_item_id(
        &self,
        namespace_id: u64,
        item_id: ItemId,
    ) -> crate::Result<GetOutcome<Vec<u8>>>;
    async fn set_v0_exact_item_id(
        &self,
        namespace_id: u64,
        item_id: ItemId,
        value: Vec<u8>,
        options: SetOptions,
    ) -> crate::Result<SetOutcome>;
    async fn delete_canonical_key_unchecked(
        &self,
        canonical_key: &[u8],
    ) -> crate::Result<DeleteOutcome>;
    async fn experimental_stats(&self) -> crate::Result<String>;
    async fn experimental_sync(&self) -> crate::Result<()>;
    async fn reconnect(&self) -> crate::Result<()>;
}

macro_rules! impl_ffi_protected_client {
    ($client:ty, $raw:ty) => {
        impl FfiProtectedClientApi for $client {
            type Raw = $raw;

            fn raw(&self) -> &Self::Raw {
                self.raw()
            }
            fn request_budget(&self) -> crate::RequestBudget {
                self.request_budget()
            }
            fn value_limits(&self) -> ValueLimits {
                self.value_limits()
            }
            fn connection_state(&self) -> ConnectionState {
                self.connection_state()
            }
            async fn ping(&self) -> crate::Result<Duration> {
                self.ping().await
            }
            async fn get_canonical_key_unchecked(
                &self,
                canonical_key: &[u8],
            ) -> crate::Result<GetOutcome<Value>> {
                self.get_canonical_key_unchecked(canonical_key).await
            }
            async fn get_structured_canonical_key_cbor(
                &self,
                canonical_key: Vec<u8>,
            ) -> crate::Result<GetOutcome<Vec<u8>>> {
                self.get_structured_canonical_key_cbor(canonical_key).await
            }
            async fn get_json_canonical_key_unchecked(
                &self,
                canonical_key: &[u8],
            ) -> crate::Result<GetOutcome<JsonValue>> {
                self.get_json_canonical_key_unchecked(canonical_key).await
            }
            async fn get_structured_canonical_key_cbor_unchecked(
                &self,
                canonical_key: &[u8],
            ) -> crate::Result<GetOutcome<Vec<u8>>> {
                self.get_structured_canonical_key_cbor_unchecked(canonical_key)
                    .await
            }
            async fn get_v0_canonical_key_unchecked(
                &self,
                canonical_key: &[u8],
            ) -> crate::Result<GetOutcome<Vec<u8>>> {
                self.get_v0_canonical_key_unchecked(canonical_key).await
            }
            async fn set_canonical_key_unchecked(
                &self,
                canonical_key: &[u8],
                value: Value,
                options: SetOptions,
            ) -> crate::Result<SetOutcome> {
                self.set_canonical_key_unchecked(canonical_key, value, options)
                    .await
            }
            async fn set_structured_canonical_key_cbor(
                &self,
                canonical_key: Vec<u8>,
                value: Vec<u8>,
                options: SetOptions,
            ) -> crate::Result<SetOutcome> {
                self.set_structured_canonical_key_cbor(canonical_key, value, options)
                    .await
            }
            async fn set_json_canonical_key_unchecked(
                &self,
                canonical_key: &[u8],
                value: JsonValue,
                options: SetOptions,
            ) -> crate::Result<SetOutcome> {
                self.set_json_canonical_key_unchecked(canonical_key, value, options)
                    .await
            }
            async fn set_structured_canonical_key_cbor_unchecked(
                &self,
                canonical_key: &[u8],
                value: &[u8],
                options: SetOptions,
            ) -> crate::Result<SetOutcome> {
                self.set_structured_canonical_key_cbor_unchecked(
                    canonical_key,
                    value,
                    options,
                )
                .await
            }
            async fn set_v0_canonical_key_unchecked(
                &self,
                canonical_key: &[u8],
                value: Vec<u8>,
                options: SetOptions,
            ) -> crate::Result<SetOutcome> {
                self.set_v0_canonical_key_unchecked(canonical_key, value, options)
                    .await
            }
            async fn get_json_exact_item_id(
                &self,
                namespace_id: u64,
                item_id: ItemId,
            ) -> crate::Result<GetOutcome<JsonValue>> {
                self.get_json_exact_item_id(namespace_id, item_id).await
            }
            async fn set_json_exact_item_id(
                &self,
                namespace_id: u64,
                item_id: ItemId,
                value: JsonValue,
                options: SetOptions,
            ) -> crate::Result<SetOutcome> {
                self.set_json_exact_item_id(namespace_id, item_id, value, options)
                    .await
            }
            async fn get_structured_exact_item_id_cbor(
                &self,
                namespace_id: u64,
                item_id: ItemId,
            ) -> crate::Result<GetOutcome<Vec<u8>>> {
                self.get_structured_exact_item_id_cbor(namespace_id, item_id)
                    .await
            }
            async fn set_structured_exact_item_id_cbor(
                &self,
                namespace_id: u64,
                item_id: ItemId,
                value: Vec<u8>,
                options: SetOptions,
            ) -> crate::Result<SetOutcome> {
                self.set_structured_exact_item_id_cbor(namespace_id, item_id, value, options)
                    .await
            }
            async fn get_v0_exact_item_id(
                &self,
                namespace_id: u64,
                item_id: ItemId,
            ) -> crate::Result<GetOutcome<Vec<u8>>> {
                self.get_v0_exact_item_id(namespace_id, item_id).await
            }
            async fn set_v0_exact_item_id(
                &self,
                namespace_id: u64,
                item_id: ItemId,
                value: Vec<u8>,
                options: SetOptions,
            ) -> crate::Result<SetOutcome> {
                self.set_v0_exact_item_id(namespace_id, item_id, value, options)
                    .await
            }
            async fn delete_canonical_key_unchecked(
                &self,
                canonical_key: &[u8],
            ) -> crate::Result<DeleteOutcome> {
                self.delete_canonical_key_unchecked(canonical_key).await
            }
            async fn experimental_stats(&self) -> crate::Result<String> {
                self.experimental_stats().await
            }
            async fn experimental_sync(&self) -> crate::Result<()> {
                self.experimental_sync().await
            }
            async fn reconnect(&self) -> crate::Result<()> {
                self.reconnect().await
            }
        }
    };
}

#[cfg(feature = "quic-compio")]
impl_ffi_protected_client!(crate::LocalProtectedClient, crate::LocalRawClient);
#[cfg(feature = "tls-tcp")]
impl_ffi_protected_client!(crate::TlsTcpProtectedClient, crate::TlsTcpRawClient);

fn run_command_loop<R, C>(
    runtime: &R,
    client: C,
    commands: CommandReceiver,
    shutdown: Arc<AtomicBool>,
    state: Arc<AtomicU32>,
    async_dispatch: bool,
) where
    R: WorkerRuntime,
    C: FfiProtectedClientApi,
{
    while !shutdown.load(Ordering::Acquire) {
        let Ok(command) = runtime.block_on(commands.recv()) else {
            break;
        };
        match command {
            Command::Execute {
                operation,
                application_key,
                value,
                input_permit,
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
                        input_permit,
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
            Command::ExecuteStructured {
                operation,
                canonical_key,
                value,
                set_options,
                response,
            } => {
                let result = catch_unwind(AssertUnwindSafe(|| {
                    runtime.block_on(execute_structured(
                        &client,
                        operation,
                        canonical_key,
                        value,
                        set_options,
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
                input_permit,
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
                        input_permit,
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
            Command::ExecuteAsync {
                operation,
                application_key,
                value,
                input_permit,
                set_options,
                raw,
                request,
                response,
            } => {
                if !request.admission.try_start() {
                    drop(response);
                    continue;
                }
                let task = execute_async_request(
                    client.clone(),
                    operation,
                    application_key,
                    value,
                    input_permit,
                    set_options,
                    raw,
                    request,
                );
                if async_dispatch {
                    runtime.spawn(async move {
                        let _ = response.send(task.await);
                    });
                } else {
                    let _ = response.send(runtime.block_on(task));
                }
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

async fn execute_async_request<C>(
    client: C,
    operation: FfiOperation,
    application_key: Vec<u8>,
    value: Vec<u8>,
    input_permit: BytePermit,
    set_options: SetOptions,
    raw: bool,
    request: Arc<FfiRequestControl>,
) -> FfiResult
where
    C: FfiProtectedClientApi,
{
    let result = execute(
        &client,
        operation,
        application_key,
        value,
        input_permit,
        set_options,
        raw,
    )
    .await;
    let completion = request.admission.complete();
    if completion == AdmissionState::CompletedCanceled {
        if request.mutating {
            FfiResult::with_status(
                FfiResultKind::UnknownMutation,
                FfiStatusCategory::UnknownMutation,
                FfiErrorCategory::UnknownMutation,
                b"mutation outcome is unknown after cancellation".to_vec(),
            )
        } else {
            FfiResult::with_status(
                FfiResultKind::Canceled,
                FfiStatusCategory::Canceled,
                FfiErrorCategory::Canceled,
                b"request canceled".to_vec(),
            )
        }
    } else {
        result
    }
}

async fn execute(
    client: &impl FfiProtectedClientApi,
    operation: FfiOperation,
    application_key: Vec<u8>,
    value: Vec<u8>,
    input_permit: BytePermit,
    set_options: SetOptions,
    raw: bool,
) -> FfiResult {
    let _input_permit = input_permit;
    let result = if raw {
        execute_raw(client, operation, application_key, value, set_options).await
    } else {
        execute_protected(client, operation, application_key, value, set_options).await
    };
    result.unwrap_or_else(FfiResult::from_error)
}

/// Executes the canonical StructuredValue-CBOR-v1 native seam.
///
/// Unary requests carry one canonical application key and are currently used
/// for structured GET.  Structured SET carries the key and value as two
/// operation fields; keeping the value-model decode here ensures no Raw or
/// JSON compatibility fallback can silently change its semantics.
async fn execute_structured(
    client: &impl FfiProtectedClientApi,
    operation: FfiOperation,
    canonical_key: Vec<u8>,
    value: Vec<u8>,
    set_options: SetOptions,
) -> FfiResult {
    let result = match operation {
        FfiOperation::Get => client
            .get_structured_canonical_key_cbor(canonical_key)
            .await
            .and_then(structured_get_result),
        FfiOperation::Set => client
            .set_structured_canonical_key_cbor(canonical_key, value, set_options)
            .await
            .map(set_result),
        _ => Err(crate::Error::configuration(
            "operation",
            "structured native ABI supports only GET and SET",
        )),
    };
    result.unwrap_or_else(FfiResult::from_error)
}

fn structured_get_result(
    outcome: GetOutcome<Vec<u8>>,
) -> std::result::Result<FfiResult, crate::Error> {
    match outcome {
        GetOutcome::NotFound => Ok(not_found_result()),
        GetOutcome::Found(payload) => Ok(FfiResult::success(FfiResultKind::Value, payload)),
    }
}

async fn execute_protected(
    client: &impl FfiProtectedClientApi,
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
            .get_json_canonical_key_unchecked(canonical_key.as_slice())
            .await
            .and_then(|outcome| {
                json_value_result(outcome, client.request_budget(), client.value_limits())
            }),
        FfiOperation::GetStructured => client
            .get_structured_canonical_key_cbor_unchecked(canonical_key.as_slice())
            .await
            .and_then(structured_get_result),
        FfiOperation::GetV0 => client
            .get_v0_canonical_key_unchecked(canonical_key.as_slice())
            .await
            .map(|value| get_result(value, bytes_result)),
        FfiOperation::Set => client
            .set_canonical_key_unchecked(canonical_key.as_slice(), Value::Raw(value), set_options)
            .await
            .map(set_result),
        FfiOperation::SetJson => {
            let budget = client.request_budget();
            let limits = client.value_limits();
            let (json, _json_permits) =
                crate::value::parse_json_input_with_budget(&value, limits, &budget)?;
            client
                .set_json_canonical_key_unchecked(canonical_key.as_slice(), json, set_options)
                .await
                .map(set_result)
        }
        FfiOperation::SetStructured => client
            .set_structured_canonical_key_cbor_unchecked(
                canonical_key.as_slice(),
                value.as_slice(),
                set_options,
            )
            .await
            .map(set_result),
        FfiOperation::SetV0 => client
            .set_v0_canonical_key_unchecked(canonical_key.as_slice(), value, set_options)
            .await
            .map(set_result),
        FfiOperation::Delete => client
            .delete_canonical_key_unchecked(canonical_key.as_slice())
            .await
            .map(delete_result),
        FfiOperation::ExperimentalStats => client
            .experimental_stats()
            .await
            .map(|stats| FfiResult::success(FfiResultKind::Value, stats.into_bytes())),
        FfiOperation::ExperimentalSync => client.experimental_sync().await.map(|()| ok_result()),
        FfiOperation::Reconnect => client.reconnect().await.map(|()| ok_result()),
        _ => Err(crate::Error::configuration(
            "operation",
            "unsupported operation from the generated Smithy contract",
        )),
    }
}

async fn execute_raw(
    client: &impl FfiProtectedClientApi,
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
        FfiOperation::GetJson => {
            let item_id = ItemId::from_slice(&item_id)?;
            client
                .get_json_exact_item_id(1, item_id)
                .await
                .and_then(|outcome| {
                    json_value_result(outcome, client.request_budget(), client.value_limits())
                })
        }
        FfiOperation::SetJson => {
            let item_id = ItemId::from_slice(&item_id)?;
            let budget = client.request_budget();
            let limits = client.value_limits();
            let (json, _json_permits) =
                crate::value::parse_json_input_with_budget(&value, limits, &budget)?;
            client
                .set_json_exact_item_id(1, item_id, json, set_options)
                .await
                .map(set_result)
        }
        FfiOperation::GetStructured => {
            let item_id = ItemId::from_slice(&item_id)?;
            client
                .get_structured_exact_item_id_cbor(1, item_id)
                .await
                .and_then(structured_get_result)
        }
        FfiOperation::SetStructured => {
            let item_id = ItemId::from_slice(&item_id)?;
            client
                .set_structured_exact_item_id_cbor(1, item_id, value, set_options)
                .await
                .map(set_result)
        }
        FfiOperation::GetV0 => {
            let item_id = ItemId::from_slice(&item_id)?;
            client
                .get_v0_exact_item_id(1, item_id)
                .await
                .map(|value| get_result(value, bytes_result))
        }
        FfiOperation::SetV0 => {
            let item_id = ItemId::from_slice(&item_id)?;
            client
                .set_v0_exact_item_id(1, item_id, value, set_options)
                .await
                .map(set_result)
        }
        FfiOperation::Delete => {
            let item_id = ItemId::from_slice(&item_id)?;
            client.raw().delete(item_id).await.map(delete_result)
        }
        FfiOperation::ExperimentalStats => client
            .raw()
            .experimental_stats()
            .await
            .map(|stats| FfiResult::success(FfiResultKind::Value, stats.into_bytes())),
        FfiOperation::ExperimentalSync => client.raw().experimental_sync().await.map(|()| ok_result()),
        FfiOperation::Reconnect => client.raw().reconnect().await.map(|()| ok_result()),
        _ => Err(crate::Error::configuration(
            "operation",
            "unsupported operation from the generated Smithy contract",
        )),
    }
}

async fn execute_scoped(
    client: &impl FfiProtectedClientApi,
    operation: FfiOperation,
    namespace_id: u64,
    item_id: Vec<u8>,
    value: Vec<u8>,
    input_permit: BytePermit,
    set_options: SetOptions,
) -> std::result::Result<FfiResult, crate::Error> {
    let _input_permit = input_permit;
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
        FfiOperation::GetJson => {
            let item_id = ItemId::from_slice(&item_id)?;
            client
                .get_json_exact_item_id(namespace_id, item_id)
                .await
                .and_then(|outcome| {
                    json_value_result(outcome, client.request_budget(), client.value_limits())
                })
        }
        FfiOperation::SetJson => {
            let item_id = ItemId::from_slice(&item_id)?;
            let budget = client.request_budget();
            let limits = client.value_limits();
            let (json, _json_permits) =
                crate::value::parse_json_input_with_budget(&value, limits, &budget)?;
            client
                .set_json_exact_item_id(namespace_id, item_id, json, set_options)
                .await
                .map(set_result)
        }
        FfiOperation::GetStructured => {
            let item_id = ItemId::from_slice(&item_id)?;
            client
                .get_structured_exact_item_id_cbor(namespace_id, item_id)
                .await
                .and_then(structured_get_result)
        }
        FfiOperation::SetStructured => {
            let item_id = ItemId::from_slice(&item_id)?;
            client
                .set_structured_exact_item_id_cbor(namespace_id, item_id, value, set_options)
                .await
                .map(set_result)
        }
        FfiOperation::GetV0 => {
            let item_id = ItemId::from_slice(&item_id)?;
            client
                .get_v0_exact_item_id(namespace_id, item_id)
                .await
                .map(|value| get_result(value, bytes_result))
        }
        FfiOperation::SetV0 => {
            let item_id = ItemId::from_slice(&item_id)?;
            client
                .set_v0_exact_item_id(namespace_id, item_id, value, set_options)
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
        FfiOperation::ExperimentalStats => client
            .raw()
            .experimental_stats_in_namespace(namespace_id)
            .await
            .map(|stats| FfiResult::success(FfiResultKind::Value, stats.into_bytes())),
        FfiOperation::ExperimentalSync => client
            .raw()
            .experimental_sync_in_namespace(namespace_id)
            .await
            .map(|()| ok_result()),
        _ => Err(crate::Error::configuration(
            "operation",
            "unsupported namespace-scoped operation from the generated Smithy contract",
        )),
    }
}

async fn namespace_open(
    client: &impl FfiProtectedClientApi,
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
            Err(error) => {
                FfiResult::from_error(crate::Error::configuration("namespace_descriptor", error))
            }
        },
        Err(error) => FfiResult::from_error(error),
    }
}

async fn namespace_update_policy(
    client: &impl FfiProtectedClientApi,
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
            Err(error) => {
                FfiResult::from_error(crate::Error::configuration("namespace_descriptor", error))
            }
        },
        Err(error) => FfiResult::from_error(error),
    }
}

async fn namespace_delete(
    client: &impl FfiProtectedClientApi,
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

#[doc(hidden)]
pub fn json_result(
    outcome: GetOutcome<Value>,
    budget: crate::RequestBudget,
    limits: ValueLimits,
) -> std::result::Result<FfiResult, crate::Error> {
    match outcome {
        GetOutcome::Found(Value::Json(mut value)) => {
            let mut writer = BudgetJsonWriter::new(&budget, limits);
            if let Err(error) = write_json_canonical(&mut value, 0, limits, &mut writer) {
                return Err(writer.take_failure().unwrap_or(error).into());
            }
            let (payload, permits) = writer.finish();
            Ok(FfiResult::success_with_permits(
                FfiResultKind::Value,
                payload,
                permits,
            ))
        }
        GetOutcome::Found(Value::Raw(_)) => Err(crate::value::Error::ExpectedRawValue.into()),
        GetOutcome::NotFound => Ok(not_found_result()),
    }
}

fn json_value_result(
    outcome: GetOutcome<JsonValue>,
    budget: crate::RequestBudget,
    limits: ValueLimits,
) -> std::result::Result<FfiResult, crate::Error> {
    let outcome = match outcome {
        GetOutcome::Found(value) => GetOutcome::Found(Value::Json(value)),
        GetOutcome::NotFound => GetOutcome::NotFound,
    };
    json_result(outcome, budget, limits)
}

fn write_json_canonical(
    value: &mut JsonValue,
    depth: usize,
    limits: ValueLimits,
    writer: &mut BudgetJsonWriter<'_>,
) -> std::result::Result<(), crate::value::Error> {
    match value {
        JsonValue::Null | JsonValue::Boolean(_) | JsonValue::Number(_) | JsonValue::String(_) => {
            serde_json_canonicalizer::to_writer(value, writer).map_err(|error| {
                writer
                    .take_failure()
                    .unwrap_or_else(|| crate::value::Error::InvalidJson(error.to_string()))
            })
        }
        JsonValue::Array(values) => {
            if depth >= limits.max_depth {
                return Err(crate::value::Error::ResourceLimit {
                    resource: Resource::StructuredValue,
                    limit: limits.max_depth,
                    actual: depth + 1,
                });
            }
            writer.write_all(b"[").map_err(|error| {
                writer
                    .take_failure()
                    .unwrap_or_else(|| crate::value::Error::InvalidJson(error.to_string()))
            })?;
            for (index, value) in values.iter_mut().enumerate() {
                if index != 0 {
                    writer.write_all(b",").map_err(|error| {
                        writer
                            .take_failure()
                            .unwrap_or_else(|| crate::value::Error::InvalidJson(error.to_string()))
                    })?;
                }
                write_json_canonical(value, depth + 1, limits, writer)?;
            }
            writer.write_all(b"]").map_err(|error| {
                writer
                    .take_failure()
                    .unwrap_or_else(|| crate::value::Error::InvalidJson(error.to_string()))
            })
        }
        JsonValue::Object(entries) => {
            if depth >= limits.max_depth {
                return Err(crate::value::Error::ResourceLimit {
                    resource: Resource::StructuredValue,
                    limit: limits.max_depth,
                    actual: depth + 1,
                });
            }
            entries.sort_unstable_by(|(left, _), (right, _)| {
                left.encode_utf16().cmp(right.encode_utf16())
            });
            writer.write_all(b"{").map_err(|error| {
                writer
                    .take_failure()
                    .unwrap_or_else(|| crate::value::Error::InvalidJson(error.to_string()))
            })?;
            for (index, (key, value)) in entries.iter_mut().enumerate() {
                if index != 0 {
                    writer.write_all(b",").map_err(|error| {
                        writer
                            .take_failure()
                            .unwrap_or_else(|| crate::value::Error::InvalidJson(error.to_string()))
                    })?;
                }
                serde_json_canonicalizer::to_writer(key, writer).map_err(|error| {
                    writer
                        .take_failure()
                        .unwrap_or_else(|| crate::value::Error::InvalidJson(error.to_string()))
                })?;
                writer.write_all(b":").map_err(|error| {
                    writer
                        .take_failure()
                        .unwrap_or_else(|| crate::value::Error::InvalidJson(error.to_string()))
                })?;
                write_json_canonical(value, depth + 1, limits, writer)?;
            }
            writer.write_all(b"}").map_err(|error| {
                writer
                    .take_failure()
                    .unwrap_or_else(|| crate::value::Error::InvalidJson(error.to_string()))
            })
        }
    }
}

struct BudgetJsonWriter<'a> {
    budget: &'a crate::RequestBudget,
    limits: ValueLimits,
    payload: Vec<u8>,
    permits: Vec<BytePermit>,
    written: usize,
    failure: Option<crate::value::Error>,
}

impl<'a> BudgetJsonWriter<'a> {
    fn new(budget: &'a crate::RequestBudget, limits: ValueLimits) -> Self {
        Self {
            budget,
            limits,
            payload: Vec::new(),
            permits: Vec::new(),
            written: 0,
            failure: None,
        }
    }

    fn take_failure(&mut self) -> Option<crate::value::Error> {
        self.failure.take()
    }

    fn fail(&mut self, error: crate::value::Error) -> io::Error {
        if self.failure.is_none() {
            self.failure = Some(error);
        }
        io::Error::new(io::ErrorKind::Other, "bounded JSON output rejected")
    }

    fn finish(self) -> (Vec<u8>, Vec<BytePermit>) {
        (self.payload, self.permits)
    }
}

impl Write for BudgetJsonWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let next = self.written.checked_add(bytes.len()).ok_or_else(|| {
            self.fail(crate::value::Error::ResourceLimit {
                resource: Resource::ExpandedPayloadBytes,
                limit: self.limits.max_expanded_payload_bytes,
                actual: usize::MAX,
            })
        })?;
        if next > self.limits.max_expanded_payload_bytes {
            return Err(self.fail(crate::value::Error::ResourceLimit {
                resource: Resource::ExpandedPayloadBytes,
                limit: self.limits.max_expanded_payload_bytes,
                actual: next,
            }));
        }
        let permit = crate::value::reserve_budget(
            self.budget,
            bytes.len(),
            &self.limits,
            Resource::ExpandedPayloadBytes,
        )
        .map_err(|error| self.fail(error))?;
        self.payload
            .try_reserve_exact(bytes.len())
            .map_err(|_| self.fail(crate::value::Error::Allocation { size: bytes.len() }))?;
        self.payload.extend_from_slice(bytes);
        self.permits.push(permit);
        self.written = next;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
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
    boxed_result(catch_result(|| {
        connect_options(
            &options,
            TransportSelection::Quic {
                verify_server: true,
            },
        )
    }))
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
    boxed_result(catch_result(|| {
        connect_options(
            &options,
            TransportSelection::Quic {
                verify_server: true,
            },
        )
    }))
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
        connect_options(
            options,
            TransportSelection::Quic {
                verify_server: true,
            },
        )
    }))
}

/// Connects using the stable options structure and an explicit transport selector.
///
/// This additive symbol leaves the base options structure unchanged. Older
/// native libraries may omit it; callers must probe the symbol before use.
///
/// # Safety
///
/// `options` must be either null or a valid, initialized pointer. Every non-empty
/// pointer/length pair in the structure must identify readable memory for this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_client_connect_transport(
    options: *const FfiConnectOptions,
    transport: u32,
) -> *mut FfiResult {
    boxed_result(catch_result(|| {
        let options = unsafe {
            options
                .as_ref()
                .ok_or_else(|| "connect options pointer must not be null".to_owned())?
        };
        let transport = TransportSelection::from_code(transport)?;
        connect_options(options, transport)
    }))
}

/// Connects through the v1 keyring configuration path.
///
/// The keyring options keep Item-ID derivation independent from value
/// encryption keys. The base options must leave `data_protection_key` empty;
/// the caller supplies the Item-ID root and, for protected values, value keys.
/// Failures are encoded in the returned result pointer.
///
/// # Safety
///
/// `options` must be either null or a valid, properly aligned pointer to an
/// initialized [`FfiConnectOptionsWithKeyring`] for the duration of this call.
/// The nested `base` pointer and every non-empty pointer/length pair in the
/// options must identify readable memory for the duration of this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_client_connect_with_keyring_options(
    options: *const FfiConnectOptionsWithKeyring,
) -> *mut FfiResult {
    boxed_result(catch_result(|| {
        let options = unsafe {
            options
                .as_ref()
                .ok_or_else(|| "keyring connect options pointer must not be null".to_owned())?
        };
        connect_options_with_keyring(options)
    }))
}

fn connect_options_with_keyring(
    options: &FfiConnectOptionsWithKeyring,
) -> std::result::Result<FfiResult, String> {
    if options.abi_version != ABI_VERSION {
        return Err(format!(
            "unsupported native ABI options version {}, expected {}",
            options.abi_version, ABI_VERSION
        ));
    }
    let base = unsafe {
        options
            .base
            .as_ref()
            .ok_or_else(|| "keyring base options pointer must not be null".to_owned())?
    };
    if base.data_protection_key_length != 0 {
        return Err(
            "keyring base data_protection_key must be empty; configure item_id_root_key and value_keys"
                .to_owned(),
        );
    }
    let item_id_root = if options.item_id_root_key_length == 0 {
        ClientRootKey::public()
    } else {
        copy_data_protection_key(options.item_id_root_key, options.item_id_root_key_length)?
            .ok_or_else(|| "item_id_root_key must contain exactly 32 bytes".to_owned())?
    };
    let encryption = match options.value_encryption {
        value if value == VALUE_FORMAT_ENCRYPTION_NONE as u32 => Encryption::Unprotected,
        value if value == VALUE_FORMAT_ENCRYPTION_COMPACT as u32 => Encryption::Compact,
        value if value == VALUE_FORMAT_ENCRYPTION_ROBUST as u32 => Encryption::Robust,
        value => return Err(format!("unsupported keyring encryption profile {value}")),
    };
    if options.value_key_count == 0 && encryption != Encryption::Unprotected {
        return Err("protected keyring values require at least one value key".to_owned());
    }
    if options.value_key_count > 0 && encryption == Encryption::Unprotected {
        return Err("unprotected keyring values must not supply value keys".to_owned());
    }
    if options.value_key_count > 0 && options.value_keys.is_null() {
        return Err(format!(
            "value_keys pointer is null for {} entries",
            options.value_key_count
        ));
    }
    if options.value_key_count > (isize::MAX as usize) / std::mem::size_of::<FfiValueKey>() {
        return Err("value_keys array is too large".to_owned());
    }
    let value_keyring = if options.value_key_count == 0 {
        None
    } else {
        let entries =
            unsafe { std::slice::from_raw_parts(options.value_keys, options.value_key_count) };
        let mut keyring = ValueKeyring::new();
        for entry in entries {
            let key = copy_fixed_value_key(entry.key, entry.key_length)?;
            keyring
                .insert(entry.id, key)
                .map_err(|error| error.to_string())?;
        }
        if options.active_value_key_id != 0 {
            keyring
                .set_active_id(Some(options.active_value_key_id))
                .map_err(|error| error.to_string())?;
        }
        Some(keyring)
    };
    let address = copy_utf8(base.address, base.address_length, "address")?;
    let mut endpoint: Endpoint = address
        .parse()
        .map_err(|error| format!("invalid server address: {error}"))?;
    let server_name = copy_utf8(base.server_name, base.server_name_length, "server name")?;
    if !server_name.is_empty() {
        endpoint = endpoint
            .with_server_name(server_name)
            .map_err(|error| error.to_string())?;
    }
    let certificate = copy_bytes(base.certificate, base.certificate_length, "certificate")?;
    let client_certificate_chain = copy_bytes(
        base.client_certificate_chain,
        base.client_certificate_chain_length,
        "client certificate chain",
    )?;
    let client_private_key = copy_bytes(
        base.client_private_key,
        base.client_private_key_length,
        "client private key",
    )?;
    if client_certificate_chain.is_empty() != client_private_key.is_empty() {
        return Err(
            "client certificate chain and private key must be supplied together".to_owned(),
        );
    }
    let compression = if base.compression_enabled == 0 {
        Compression::Disabled
    } else {
        let defaults = ZstandardOptions::default();
        Compression::Zstandard(ZstandardOptions {
            level: if base.compression_level == 0 {
                defaults.level
            } else {
                base.compression_level
            },
            minimum_input_size: if base.minimum_input_size == 0 {
                defaults.minimum_input_size
            } else {
                base.minimum_input_size
            },
            minimum_savings: if base.minimum_savings == 0 {
                defaults.minimum_savings
            } else {
                base.minimum_savings
            },
        })
    };
    if base.connect_timeout_ms == 0 || base.request_timeout_ms == 0 {
        return Err("client timeouts must be greater than zero milliseconds".to_owned());
    }
    let timeouts = ClientTimeouts {
        connect: Duration::from_millis(base.connect_timeout_ms),
        request: Duration::from_millis(base.request_timeout_ms),
    };
    let retry = if base.retry_max_attempts == 0 {
        RetryPolicy::default()
    } else {
        RetryPolicy {
            max_attempts: base.retry_max_attempts,
        }
    };
    let max_in_flight = if base.max_in_flight == 0 {
        crate::DEFAULT_MAX_IN_FLIGHT
    } else {
        base.max_in_flight
    };
    FfiClient::connect(
        TransportSelection::Quic {
            verify_server: true,
        },
        endpoint,
        certificate,
        Some(item_id_root),
        value_keyring,
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

fn connect_options(
    options: &FfiConnectOptions,
    transport: TransportSelection,
) -> std::result::Result<FfiResult, String> {
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
        transport,
        endpoint,
        certificate,
        data_protection_key,
        None,
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
/// bytes and is not a wire Item ID. `SET` accepts an empty value and
/// optional existence/TTL options. `PING`, `EXPERIMENTAL_STATS`, and `EXPERIMENTAL_SYNC` require empty
/// key and value buffers.
///
/// # Safety
///
/// `client` must be a live pointer returned by [`openkache_client_result_take_client`]. Every
/// non-empty application-key/value pointer pair must identify readable memory for this call, and
/// the client must not be freed until this call returns.
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
    typed_execute_entry(
        client,
        operation,
        key_spec,
        application_key,
        application_key_length,
        value,
        value_length,
        Some((set_condition, ttl_enabled, ttl_ms)),
        None,
    )
}

/// Executes a typed protected operation with the complete wire SET policy.
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
    typed_execute_entry(
        client,
        operation,
        key_spec,
        application_key,
        application_key_length,
        value,
        value_length,
        None,
        Some((set_flags, ttl_ms)),
    )
}

/// Executes a typed operation asynchronously through the client's shared worker.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_client_execute_async(
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
) -> *mut FfiRequest {
    typed_async_entry(
        client,
        operation,
        key_spec,
        application_key,
        application_key_length,
        value,
        value_length,
        Some((set_condition, ttl_enabled, ttl_ms)),
        None,
        false,
    )
}

/// Executes a typed operation asynchronously with complete SET policy flags.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_client_execute_with_options_async(
    client: *const FfiClient,
    operation: u32,
    key_spec: u32,
    application_key: *const u8,
    application_key_length: usize,
    value: *const u8,
    value_length: usize,
    set_flags: u8,
    ttl_ms: u64,
) -> *mut FfiRequest {
    typed_async_entry(
        client,
        operation,
        key_spec,
        application_key,
        application_key_length,
        value,
        value_length,
        None,
        Some((set_flags, ttl_ms)),
        false,
    )
}

/// Executes an exact Item ID operation asynchronously without reinterpretation.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_client_execute_raw_async(
    client: *const FfiClient,
    operation: u32,
    item_id: *const u8,
    item_id_length: usize,
    value: *const u8,
    value_length: usize,
    set_condition: u32,
    ttl_enabled: u8,
    ttl_ms: u64,
) -> *mut FfiRequest {
    typed_async_entry(
        client,
        operation,
        FfiKeySpec::Bytes.code(),
        item_id,
        item_id_length,
        value,
        value_length,
        Some((set_condition, ttl_enabled, ttl_ms)),
        None,
        true,
    )
}

/// Executes the native StructuredValue-CBOR-v1 unary seam.
///
/// A unary GET body is one canonical key item.  Structured SET requests use
/// [`openkache_client_execute_fields`] so the key and value remain distinct.
///
/// # Safety
///
/// `client` must be live and `body` must be readable for `body_length` bytes.
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
        let operation = FfiOperation::try_from(operation)
            .map_err(|operation| format!("unsupported operation {operation}"))?;
        if operation != FfiOperation::Get {
            return Err(
                "structured unary ABI currently accepts only GET canonical-key bodies".to_owned(),
            );
        }
        let body = copy_bytes(body, body_length, "structured canonical key")?;
        Ok(client.execute_structured(operation, body, Vec::new(), SetOptions::new()))
    }))
}

/// Executes the native StructuredValue-CBOR-v1 field seam.
///
/// GET accepts one present field containing a canonical key.  SET accepts two
/// present fields: canonical key followed by one complete
/// StructuredValue-CBOR-v1 payload.  Optional/missing fields are rejected so
/// a caller cannot silently fall back to Raw or JSON behavior.
///
/// # Safety
///
/// `client` must be live.  `fields` must point to `field_count` initialized
/// [`FfiOperationField`] values, and every present field buffer must remain
/// readable for this call.
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
        let operation = FfiOperation::try_from(operation)
            .map_err(|operation| format!("unsupported operation {operation}"))?;
        let fields = if field_count == 0 {
            &[][..]
        } else if fields.is_null() {
            return Err("structured operation fields pointer must not be null".to_owned());
        } else {
            unsafe { std::slice::from_raw_parts(fields, field_count) }
        };
        let required = match operation {
            FfiOperation::Get => 1,
            FfiOperation::Set => 2,
            _ => {
                return Err("structured fields ABI currently accepts only GET and SET".to_owned());
            }
        };
        if fields.len() != required {
            return Err(format!(
                "structured {operation} requires exactly {required} fields, got {}",
                fields.len()
            ));
        }
        let copy_field =
            |index: usize, name: &'static str| -> std::result::Result<Vec<u8>, String> {
                let field = fields[index];
                if field.present == 0 {
                    return Err(format!("structured {name} field must be present"));
                }
                copy_bytes(field.data, field.length, name)
            };
        let canonical_key = copy_field(0, "canonical key")?;
        let value = if operation == FfiOperation::Set {
            copy_field(1, "structured value")?
        } else {
            Vec::new()
        };
        Ok(client.execute_structured(operation, canonical_key, value, SetOptions::new()))
    }))
}

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

/// Executes one exact-item-ID operation without application-key derivation.
///
/// `GET`, `SET`, and `DELETE` use the exact item ID supplied by the caller and
/// preserve their opaque value bytes. The generated `GET_JSON`/`SET_JSON`,
/// `GET_STRUCTURED`/`SET_STRUCTURED`, and `GET_V0`/`SET_V0` operations use the
/// same exact address while applying their documented value representation.
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
/// `set_flags` is the complete wire SET flag byte. It is ignored for operations other than
/// `SET`, `SET_JSON`, `SET_STRUCTURED`, and `SET_V0`, which must pass zero for both `set_flags`
/// and `ttl_ms`.
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
        let operation = FfiOperation::try_from(operation)
            .map_err(|operation| format!("unsupported operation {operation}"))?;
        validate_scoped_input_lengths(operation, item_id_length, value_length)?;
        let (item_id, value, input_permit) = copy_input_bytes(
            item_id,
            item_id_length,
            value,
            value_length,
            &client.request_budget(),
        )?;
        let set_options = if matches!(
            operation,
            FfiOperation::Set
                | FfiOperation::SetJson
                | FfiOperation::SetStructured
                | FfiOperation::SetV0
        ) {
            set_options_from_flags(set_flags, ttl_ms)?
        } else {
            if set_flags != 0 || ttl_ms != 0 {
                return Err("SET flags and TTL require a SET operation".to_owned());
            }
            SetOptions::new()
        };
        match operation {
            FfiOperation::Get
            | FfiOperation::Set
            | FfiOperation::GetJson
            | FfiOperation::SetJson
            | FfiOperation::GetStructured
            | FfiOperation::SetStructured
            | FfiOperation::GetV0
            | FfiOperation::SetV0
            | FfiOperation::Delete
                if item_id.len() > crate::MAX_ITEM_ID_BYTES =>
            {
                Err(format!(
                    "item_id must contain at most {} bytes, got {}",
                    crate::MAX_ITEM_ID_BYTES,
                    item_id.len()
                ))
            }
            FfiOperation::Get
            | FfiOperation::GetJson
            | FfiOperation::GetStructured
            | FfiOperation::GetV0
            | FfiOperation::Delete
                if !value.is_empty() =>
            {
                Err("operation does not accept a value".to_owned())
            }
            FfiOperation::ExperimentalStats | FfiOperation::ExperimentalSync if !item_id.is_empty() => {
                Err("operation does not accept an item_id".to_owned())
            }
            FfiOperation::ExperimentalStats | FfiOperation::ExperimentalSync if !value.is_empty() => {
                Err("operation does not accept a value".to_owned())
            }
            FfiOperation::GetJson
            | FfiOperation::SetJson
            | FfiOperation::GetStructured
            | FfiOperation::SetStructured
            | FfiOperation::GetV0
            | FfiOperation::SetV0 => Ok(client.execute_scoped(
                operation,
                namespace_id,
                item_id,
                value,
                input_permit,
                set_options,
            )),
            FfiOperation::Ping => Err(
                "operation is not available through the namespace-scoped exact-ID ABI".to_owned(),
            ),
            FfiOperation::NamespaceOpen
            | FfiOperation::NamespaceUpdatePolicy
            | FfiOperation::NamespaceDelete
            | FfiOperation::Reconnect => {
                Err("namespace management and reconnect use dedicated native ABI calls".to_owned())
            }
            _ => Ok(client.execute_scoped(
                operation,
                namespace_id,
                item_id,
                value,
                input_permit,
                set_options,
            )),
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

fn ffi_key_bytes(key_spec: u32, key: Vec<u8>, raw: bool) -> std::result::Result<Vec<u8>, String> {
    if raw {
        if key.len() > crate::MAX_ITEM_ID_BYTES {
            return Err(format!(
                "item_id must contain at most {} bytes, got {}",
                crate::MAX_ITEM_ID_BYTES,
                key.len()
            ));
        }
        return Ok(key);
    }
    let spec = FfiKeySpec::try_from(key_spec)
        .map_err(|value| format!("unsupported key specification {value}"))?;
    let key_type = match spec {
        FfiKeySpec::Text => KeyType::Text,
        FfiKeySpec::Bytes => KeyType::Bytes,
        FfiKeySpec::Integer => KeyType::Integer,
    };
    KeySpace::new(key_type)
        .resolve_logical_bytes(&key)
        .map(|key| key.into_canonical_bytes())
        .map_err(|error| error.to_string())
}

fn set_options_from_legacy(
    set_condition: u32,
    ttl_enabled: u8,
    ttl_ms: u64,
) -> std::result::Result<SetOptions, String> {
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
    Ok(set_options)
}

fn validated_execute(
    operation: u32,
    key: Vec<u8>,
    value: Vec<u8>,
    _raw: bool,
    legacy_options: Option<(u32, u8, u64)>,
    complete_flags: Option<(u8, u64)>,
) -> std::result::Result<(FfiOperation, Vec<u8>, Vec<u8>, SetOptions), String> {
    let operation = FfiOperation::try_from(operation)
        .map_err(|operation| format!("unsupported operation {operation}"))?;
    let set_options = if let Some((flags, ttl_ms)) = complete_flags {
        if matches!(
            operation,
            FfiOperation::Set
                | FfiOperation::SetJson
                | FfiOperation::SetStructured
                | FfiOperation::SetV0
        ) {
            set_options_from_flags(flags, ttl_ms)?
        } else {
            if flags != 0 || ttl_ms != 0 {
                return Err("SET flags and TTL require a SET operation".to_owned());
            }
            SetOptions::new()
        }
    } else if let Some((set_condition, ttl_enabled, ttl_ms)) = legacy_options {
        set_options_from_legacy(set_condition, ttl_enabled, ttl_ms)?
    } else {
        SetOptions::new()
    };
    match operation {
        FfiOperation::Ping | FfiOperation::ExperimentalStats | FfiOperation::ExperimentalSync | FfiOperation::Reconnect
            if !key.is_empty() =>
        {
            Err("operation does not accept an application key".to_owned())
        }
        FfiOperation::Ping
        | FfiOperation::Get
        | FfiOperation::GetJson
        | FfiOperation::GetStructured
        | FfiOperation::GetV0
        | FfiOperation::Delete
        | FfiOperation::ExperimentalStats
        | FfiOperation::ExperimentalSync
        | FfiOperation::Reconnect
            if !value.is_empty() =>
        {
            Err("operation does not accept a value".to_owned())
        }
        operation
            if !matches!(
                operation,
                FfiOperation::Set
                    | FfiOperation::SetJson
                    | FfiOperation::SetStructured
                    | FfiOperation::SetV0
            ) && (set_options.condition() != SetCondition::Any
                || set_options.time_to_live_millis().is_some()) =>
        {
            Err("SET options require a SET operation".to_owned())
        }
        _ => Ok((operation, key, value, set_options)),
    }
}

#[allow(clippy::too_many_arguments)]
fn typed_execute_entry(
    client: *const FfiClient,
    operation: u32,
    key_spec: u32,
    application_key: *const u8,
    application_key_length: usize,
    value: *const u8,
    value_length: usize,
    legacy_options: Option<(u32, u8, u64)>,
    complete_flags: Option<(u8, u64)>,
) -> *mut FfiResult {
    boxed_result(catch_result(|| {
        let client = unsafe {
            client
                .as_ref()
                .ok_or_else(|| "client pointer must not be null".to_owned())?
        };
        let operation = FfiOperation::try_from(operation)
            .map_err(|operation| format!("unsupported operation {operation}"))?;
        validate_input_lengths(operation, false, application_key_length, value_length)?;
        let (key_input, value, input_permit) = copy_input_bytes(
            application_key,
            application_key_length,
            value,
            value_length,
            &client.request_budget(),
        )?;
        let key = if matches!(
            operation,
            FfiOperation::Ping | FfiOperation::ExperimentalStats | FfiOperation::ExperimentalSync | FfiOperation::Reconnect
        ) {
            // Keyless operations use an empty application-key buffer.  Do
            // not turn that buffer into a canonical empty Bytes key before
            // `validated_execute` checks the operation's empty-key contract.
            key_input
        } else {
            ffi_key_bytes(key_spec, key_input, false)?
        };
        let (operation, key, value, set_options) = validated_execute(
            operation.code(),
            key,
            value,
            false,
            legacy_options,
            complete_flags,
        )?;
        Ok(client.execute(operation, key, value, input_permit, set_options, false))
    }))
}

#[allow(clippy::too_many_arguments)]
fn typed_async_entry(
    client: *const FfiClient,
    operation: u32,
    key_spec: u32,
    application_key: *const u8,
    application_key_length: usize,
    value: *const u8,
    value_length: usize,
    legacy_options: Option<(u32, u8, u64)>,
    complete_flags: Option<(u8, u64)>,
    raw: bool,
) -> *mut FfiRequest {
    let request = catch_unwind(AssertUnwindSafe(|| {
        let client = unsafe {
            client
                .as_ref()
                .ok_or_else(|| "client pointer must not be null".to_owned())?
        };
        let operation = FfiOperation::try_from(operation)
            .map_err(|operation| format!("unsupported operation {operation}"))?;
        validate_input_lengths(operation, raw, application_key_length, value_length)?;
        let (key_input, value, input_permit) = copy_input_bytes(
            application_key,
            application_key_length,
            value,
            value_length,
            &client.request_budget(),
        )?;
        let key = if !raw
            && matches!(
                operation,
                FfiOperation::Ping
                    | FfiOperation::ExperimentalStats
                    | FfiOperation::ExperimentalSync
                    | FfiOperation::Reconnect
            ) {
            // Keyless operations use an empty application-key buffer.  Do
            // not turn that buffer into a canonical empty Bytes key before
            // `validated_execute` checks the operation's empty-key contract.
            key_input
        } else {
            ffi_key_bytes(key_spec, key_input, raw)?
        };
        let (operation, key, value, set_options) = validated_execute(
            operation.code(),
            key,
            value,
            raw,
            legacy_options,
            complete_flags,
        )?;
        Ok::<FfiRequest, String>(client.execute_async(
            operation,
            key,
            value,
            input_permit,
            set_options,
            raw,
        ))
    }));
    match request {
        Ok(Ok(request)) => Box::into_raw(Box::new(request)),
        Ok(Err(error)) => Box::into_raw(Box::new(FfiRequest::completed(
            FfiResult::error_with_category(error, FfiErrorCategory::InvalidInput),
        ))),
        Err(_) => Box::into_raw(Box::new(FfiRequest::completed(
            FfiResult::error_with_category("native client panicked", FfiErrorCategory::Internal),
        ))),
    }
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
        let operation = FfiOperation::try_from(operation)
            .map_err(|operation| format!("unsupported operation {operation}"))?;
        validate_input_lengths(operation, raw, application_key_length, value_length)?;
        let (application_key, value, input_permit) = copy_input_bytes(
            application_key,
            application_key_length,
            value,
            value_length,
            &client.request_budget(),
        )?;
        if raw
            && matches!(
                operation,
                FfiOperation::Get
                    | FfiOperation::Set
                    | FfiOperation::GetJson
                    | FfiOperation::SetJson
                    | FfiOperation::GetStructured
                    | FfiOperation::SetStructured
                    | FfiOperation::GetV0
                    | FfiOperation::SetV0
                    | FfiOperation::Delete
            )
            && application_key.len() > crate::MAX_ITEM_ID_BYTES
        {
            return Err(format!(
                "item_id must contain at most {} bytes, got {}",
                crate::MAX_ITEM_ID_BYTES,
                application_key.len()
            ));
        }
        let set_options = if let Some((flags, ttl_ms)) = complete_flags {
            if matches!(
                operation,
                FfiOperation::Set
                    | FfiOperation::SetJson
                    | FfiOperation::SetStructured
                    | FfiOperation::SetV0
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
            | FfiOperation::ExperimentalStats
            | FfiOperation::ExperimentalSync
            | FfiOperation::Reconnect
                if !application_key.is_empty() =>
            {
                Err("operation does not accept an application key".to_owned())
            }
            FfiOperation::Ping
            | FfiOperation::Get
            | FfiOperation::GetJson
            | FfiOperation::GetStructured
            | FfiOperation::GetV0
            | FfiOperation::Delete
            | FfiOperation::ExperimentalStats
            | FfiOperation::ExperimentalSync
            | FfiOperation::Reconnect
                if !value.is_empty() =>
            {
                Err("operation does not accept a value".to_owned())
            }
            operation
                if !matches!(
                    operation,
                    FfiOperation::Set
                        | FfiOperation::SetJson
                        | FfiOperation::SetStructured
                        | FfiOperation::SetV0
                ) && (set_options.condition() != SetCondition::Any
                    || set_options.time_to_live_millis().is_some()) =>
            {
                Err("SET options require a SET operation".to_owned())
            }
            _ => Ok(client.execute(
                operation,
                application_key,
                value,
                input_permit,
                set_options,
                raw,
            )),
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

/// Returns the current state of an asynchronous request without consuming its
/// result.
///
/// A null request is reported as [`FfiRequestState::Freed`]. A ready request
/// remains ready until [`openkache_client_request_wait`] consumes its result.
///
/// # Safety
///
/// If `request` is non-null, it must be a live pointer returned by one of the
/// asynchronous execute functions and must remain valid until this call
/// returns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_client_request_poll(request: *const FfiRequest) -> u32 {
    let Some(request) = (unsafe { request.as_ref() }) else {
        return FfiRequestState::Freed.code();
    };
    request.poll().code()
}

/// Waits for an asynchronous request and returns its owned result handle.
///
/// The timeout is a maximum wait in milliseconds. A timeout returns a normal
/// timeout result but leaves the request pending, allowing a later poll or
/// wait. Once a result is returned, the request enters `Consumed` and must
/// still be released with [`openkache_client_request_free`].
///
/// # Safety
///
/// `request` must be a live, uniquely accessed pointer returned by an
/// asynchronous execute function. The request must not be freed concurrently
/// with this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_client_request_wait(
    request: *mut FfiRequest,
    timeout_ms: u64,
) -> *mut FfiResult {
    let Some(request) = (unsafe { request.as_ref() }) else {
        return boxed_result(FfiResult::error_with_category(
            "request pointer must not be null",
            FfiErrorCategory::InvalidInput,
        ));
    };
    boxed_result(request.wait(Duration::from_millis(timeout_ms)))
}

/// Requests cancellation of an asynchronous operation.
///
/// Cancellation is cooperative. A mutating request that has started may
/// complete as `UnknownMutation`, because the server outcome cannot be
/// inferred after cancellation. Read-only requests complete as `Canceled`.
///
/// # Safety
///
/// If `request` is non-null, it must be a live pointer returned by an
/// asynchronous execute function and remain valid until this call returns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_client_request_cancel(request: *const FfiRequest) -> u32 {
    let Some(request) = (unsafe { request.as_ref() }) else {
        return FfiRequestState::Freed.code();
    };
    request.cancel().code()
}

/// Frees an asynchronous request handle exactly once.
///
/// Any unconsumed result is discarded with the request. Result handles
/// returned by `request_wait` remain independently owned and must be freed
/// with [`openkache_client_result_free`].
///
/// # Safety
///
/// `request` must be null or a unique live pointer returned by an asynchronous
/// execute function. It must not be freed or accessed concurrently.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_client_request_free(request: *mut FfiRequest) {
    if !request.is_null() {
        drop(unsafe { Box::from_raw(request) });
    }
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

/// Returns the structured completion status category for a result.
///
/// A null result is treated as an error status.
///
/// # Safety
///
/// `result` must be null or a live pointer returned by this library.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_client_result_status(result: *const FfiResult) -> u32 {
    unsafe { result.as_ref() }.map_or(FfiStatusCategory::Error.code(), |result| {
        result.status.code()
    })
}

/// Returns the structured error category for a result.
///
/// A null result is treated as an internal error.
///
/// # Safety
///
/// `result` must be null or a live pointer returned by this library.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_client_result_error_category(result: *const FfiResult) -> u32 {
    unsafe { result.as_ref() }.map_or(FfiErrorCategory::Internal.code(), |result| {
        result.error_category.code()
    })
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
        Ok(Err(error)) => FfiResult::error_with_category(error, FfiErrorCategory::InvalidInput),
        Err(_) => {
            FfiResult::error_with_category("native client panicked", FfiErrorCategory::Internal)
        }
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

fn copy_fixed_value_key(
    pointer: *const u8,
    length: usize,
) -> std::result::Result<[u8; crate::DATA_PROTECTION_KEY_BYTES], String> {
    let bytes = copy_bytes(pointer, length, "value key")?;
    bytes.try_into().map_err(|bytes: Vec<u8>| {
        format!(
            "value key must contain exactly {} bytes, got {}",
            crate::DATA_PROTECTION_KEY_BYTES,
            bytes.len()
        )
    })
}

fn validate_input_lengths(
    operation: FfiOperation,
    raw: bool,
    application_key_length: usize,
    value_length: usize,
) -> std::result::Result<(), String> {
    if raw
        && matches!(
            operation,
            FfiOperation::Get
                | FfiOperation::Set
                | FfiOperation::GetJson
                | FfiOperation::SetJson
                | FfiOperation::GetStructured
                | FfiOperation::SetStructured
                | FfiOperation::GetV0
                | FfiOperation::SetV0
                | FfiOperation::Delete
        )
        && application_key_length > crate::MAX_ITEM_ID_BYTES
    {
        return Err(format!(
            "item_id must contain at most {} bytes, got {}",
            crate::MAX_ITEM_ID_BYTES,
            application_key_length
        ));
    }
    if matches!(
        operation,
        FfiOperation::Ping
            | FfiOperation::Get
            | FfiOperation::GetJson
            | FfiOperation::GetStructured
            | FfiOperation::GetV0
            | FfiOperation::Delete
            | FfiOperation::ExperimentalStats
            | FfiOperation::ExperimentalSync
            | FfiOperation::Reconnect
    ) && value_length != 0
    {
        return Err("operation does not accept a value".to_owned());
    }
    if matches!(
        operation,
        FfiOperation::Ping | FfiOperation::ExperimentalStats | FfiOperation::ExperimentalSync | FfiOperation::Reconnect
    ) && application_key_length != 0
    {
        return Err("operation does not accept an application key".to_owned());
    }
    Ok(())
}

fn validate_scoped_input_lengths(
    operation: FfiOperation,
    item_id_length: usize,
    value_length: usize,
) -> std::result::Result<(), String> {
    if matches!(
        operation,
        FfiOperation::Get
            | FfiOperation::Set
            | FfiOperation::GetJson
            | FfiOperation::SetJson
            | FfiOperation::GetStructured
            | FfiOperation::SetStructured
            | FfiOperation::GetV0
            | FfiOperation::SetV0
            | FfiOperation::Delete
    ) && item_id_length > crate::MAX_ITEM_ID_BYTES
    {
        return Err(format!(
            "item_id must contain at most {} bytes, got {}",
            crate::MAX_ITEM_ID_BYTES,
            item_id_length
        ));
    }
    if matches!(operation, FfiOperation::Get | FfiOperation::Delete) && value_length != 0 {
        return Err("operation does not accept a value".to_owned());
    }
    if matches!(operation, FfiOperation::ExperimentalStats | FfiOperation::ExperimentalSync)
        && (item_id_length != 0 || value_length != 0)
    {
        return Err("operation does not accept item_id or value".to_owned());
    }
    Ok(())
}

fn copy_input_bytes(
    key_pointer: *const u8,
    key_length: usize,
    value_pointer: *const u8,
    value_length: usize,
    budget: &crate::RequestBudget,
) -> std::result::Result<(Vec<u8>, Vec<u8>, BytePermit), String> {
    if key_length != 0 && key_pointer.is_null() {
        return Err(format!(
            "application_key pointer is null for {key_length} bytes"
        ));
    }
    if value_length != 0 && value_pointer.is_null() {
        return Err(format!("value pointer is null for {value_length} bytes"));
    }
    let total = key_length
        .checked_add(value_length)
        .ok_or_else(|| "FFI input length exceeds the platform address space".to_owned())?;
    let permit = budget
        .try_reserve(total)
        .map_err(|error| format!("FFI input admission failed: {error}"))?;
    let mut key = Vec::new();
    key.try_reserve_exact(key_length)
        .map_err(|_| format!("failed to allocate {key_length} bytes for application_key"))?;
    if key_length != 0 {
        key.extend_from_slice(unsafe { std::slice::from_raw_parts(key_pointer, key_length) });
    }
    let mut value = Vec::new();
    value
        .try_reserve_exact(value_length)
        .map_err(|_| format!("failed to allocate {value_length} bytes for value"))?;
    if value_length != 0 {
        value.extend_from_slice(unsafe { std::slice::from_raw_parts(value_pointer, value_length) });
    }
    Ok((key, value, permit))
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

/// Copies one FFI input buffer for white-box adapter regressions.
///
/// # Safety
///
/// When `length` is non-zero, `pointer` must identify readable memory for
/// `length` bytes for the duration of this call.
#[doc(hidden)]
pub unsafe fn copy_bytes_for_test(
    pointer: *const u8,
    length: usize,
    name: &str,
) -> std::result::Result<Vec<u8>, String> {
    copy_bytes(pointer, length, name)
}
