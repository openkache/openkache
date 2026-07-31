//! Stable C ABI for native client integrations.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{SyncSender, sync_channel};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::value::{Compression, ZstandardOptions};
use crate::{
    Certificate, ClientIdentity, ClientTimeouts, DataProtectionKey, DeleteOutcome, Endpoint,
    GetOutcome, ItemId, ItemValue, LocalProtectedClient, PrivateKey, RetryPolicy, ServerTrust,
    SetCondition, SetOptions, SetOutcome,
};

const COMMAND_QUEUE_CAPACITY: usize = 64;

#[derive(Clone, Copy)]
#[repr(u32)]
enum FfiResultKind {
    Error = openkache_protocol::FFI_RESULT_ERROR,
    Ok = openkache_protocol::FFI_RESULT_OK,
    Value = openkache_protocol::FFI_RESULT_VALUE,
    NotFound = openkache_protocol::FFI_RESULT_NOT_FOUND,
    Created = openkache_protocol::FFI_RESULT_CREATED,
    Replaced = openkache_protocol::FFI_RESULT_REPLACED,
    Deleted = openkache_protocol::FFI_RESULT_DELETED,
    NotDeleted = openkache_protocol::FFI_RESULT_NOT_DELETED,
    Connected = openkache_protocol::FFI_RESULT_CONNECTED,
    NotStored = openkache_protocol::FFI_RESULT_NOT_STORED,
}

macro_rules! ffi_input_enum {
    (
        enum $name:ident {
            $($variant:ident = $value:expr),+ $(,)?
        }
    ) => {
        #[derive(Clone, Copy, Debug)]
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
        Ping = openkache_protocol::Opcode::Ping as u32,
        Get = openkache_protocol::Opcode::Get as u32,
        Set = openkache_protocol::Opcode::Set as u32,
        Delete = openkache_protocol::Opcode::Delete as u32,
        Stats = openkache_protocol::Opcode::Stats as u32,
        Sync = openkache_protocol::Opcode::Sync as u32,
    }
}

ffi_input_enum! {
    enum FfiSetCondition {
        None = openkache_protocol::FFI_SET_CONDITION_NONE,
        IfAbsent = openkache_protocol::FFI_SET_CONDITION_IF_ABSENT,
        IfPresent = openkache_protocol::FFI_SET_CONDITION_IF_PRESENT,
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
        item_id: Option<ItemId>,
        application_key: Vec<u8>,
        value: Vec<u8>,
        set_options: SetOptions,
        response: SyncSender<FfiResult>,
    },
    Shutdown,
}

type CommandSender = crossfire::MTx<crossfire::mpsc::Array<Command>>;
type CommandReceiver = crossfire::AsyncRx<crossfire::mpsc::Array<Command>>;

struct WorkerOptions {
    endpoint: Endpoint,
    certificate: Vec<u8>,
    identity: Option<ClientIdentity>,
    data_protection_key: DataProtectionKey,
    compression: Compression,
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
    fn connect(options: WorkerOptions) -> std::result::Result<Self, String> {
        let (commands, receiver) = crossfire::mpsc::bounded_blocking_async(COMMAND_QUEUE_CAPACITY);
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
        item_id: Option<ItemId>,
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
        endpoint,
        certificate,
        identity,
        data_protection_key,
        compression,
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
            "OpenKache client requires the Compio io_uring driver".to_string()
        ));
        return;
    }
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

    runtime.block_on(async move {
        let mut operations: Vec<compio::runtime::JoinHandle<()>> = Vec::new();
        while !shutdown.load(Ordering::Acquire) {
            let Ok(command) = commands.recv().await else {
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
                    operations.retain(|operation| !operation.is_finished());
                    if operations.len() >= COMMAND_QUEUE_CAPACITY
                        && let Some(operation) = operations.pop()
                    {
                        let _ = operation.await;
                    }
                    let client = client.clone();
                    operations.push(compio::runtime::spawn(async move {
                        let result = execute(
                            &client,
                            operation,
                            item_id,
                            application_key,
                            value,
                            set_options,
                        )
                        .await;
                        let _ = response.send(result);
                    }));
                }
                Command::Shutdown => break,
            }
        }
        for operation in operations {
            let _ = operation.await;
        }
    });
}

async fn execute(
    client: &LocalProtectedClient,
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
        };
    result.unwrap_or_else(|error| FfiResult::error(error.to_string()))
}

async fn execute_raw(
    client: &LocalProtectedClient,
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

/// Returns the native ABI version implemented by this library.
#[unsafe(no_mangle)]
pub extern "C" fn openkache_client_abi_version() -> u32 {
    openkache_protocol::FFI_ABI_VERSION
}

/// Connects a native client and returns an opaque result.
///
/// `address` accepts a host and port such as `127.0.0.1:4433` or
/// `cache.example.com:4433`; `server_name` is the TLS identity to verify.
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
        let address = copy_utf8(address, address_length, "address")?;
        let server_name = copy_utf8(server_name, server_name_length, "server name")?;
        let endpoint = address
            .parse::<Endpoint>()
            .map_err(|error| error.to_string())?
            .with_server_name(server_name)
            .map_err(|error| error.to_string())?;
        let certificate = copy_bytes(certificate, certificate_length, "certificate")?;
        if certificate.is_empty() {
            return Err("certificate must not be empty".to_string());
        }
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
        let timeouts = ClientTimeouts {
            connect: Duration::from_millis(connect_timeout_ms),
            request: Duration::from_millis(request_timeout_ms),
        };
        FfiClient::connect(WorkerOptions {
            endpoint,
            certificate,
            identity: None,
            data_protection_key,
            compression,
            timeouts,
            retry: RetryPolicy::default(),
            max_in_flight: 256,
        })
        .map(FfiResult::connected)
    }))
}

/// Connects a native client with the complete shared-core configuration.
///
/// The original [`openkache_client_connect`] entry point remains available for
/// ABI compatibility. This extension accepts PEM or DER trust material,
/// optional mutual-TLS identity material, retry policy, and stream limits
/// without changing the ABI version.
///
/// # Safety
///
/// Every non-empty pointer/length pair must identify readable memory for the
/// duration of this call. The data-protection key must contain exactly 32
/// bytes. The client certificate chain and private key must either both be
/// present or both be absent.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn openkache_client_connect_ex(
    address: *const u8,
    address_length: usize,
    server_name: *const u8,
    server_name_length: usize,
    certificate: *const u8,
    certificate_length: usize,
    identity_certificate_chain: *const u8,
    identity_certificate_chain_length: usize,
    identity_private_key: *const u8,
    identity_private_key_length: usize,
    data_protection_key: *const u8,
    data_protection_key_length: usize,
    compression_enabled: u8,
    compression_level: i32,
    minimum_input_size: usize,
    minimum_savings: usize,
    connect_timeout_ms: u64,
    request_timeout_ms: u64,
    retry_max_attempts: u64,
    max_in_flight: usize,
) -> *mut FfiResult {
    boxed_result(catch_result(|| {
        let address = copy_utf8(address, address_length, "address")?;
        let server_name = copy_utf8(server_name, server_name_length, "server name")?;
        let endpoint = address
            .parse::<Endpoint>()
            .map_err(|error| error.to_string())?
            .with_server_name(server_name)
            .map_err(|error| error.to_string())?;
        let certificate = copy_bytes(certificate, certificate_length, "certificate")?;
        if certificate.is_empty() {
            return Err("certificate must not be empty".to_string());
        }
        let identity_certificate_chain = copy_bytes(
            identity_certificate_chain,
            identity_certificate_chain_length,
            "identity certificate chain",
        )?;
        let identity_private_key = copy_bytes(
            identity_private_key,
            identity_private_key_length,
            "identity private key",
        )?;
        let identity = match (
            identity_certificate_chain.is_empty(),
            identity_private_key.is_empty(),
        ) {
            (true, true) => None,
            (false, false) => {
                let certificates = Certificate::from_der_or_pem_chain(&identity_certificate_chain)
                    .map_err(|error| error.to_string())?;
                let private_key = PrivateKey::from_der_or_pem(&identity_private_key)
                    .map_err(|error| error.to_string())?;
                Some(
                    ClientIdentity::new(certificates, private_key)
                        .map_err(|error| error.to_string())?,
                )
            }
            _ => {
                return Err(
                    "identity certificate chain and private key must be provided together"
                        .to_string(),
                );
            }
        };
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
        if retry_max_attempts == 0 {
            return Err("retry max attempts must be greater than zero".to_string());
        }
        let retry_max_attempts = usize::try_from(retry_max_attempts)
            .map_err(|_| "retry max attempts exceeds the platform limit".to_string())?;
        if max_in_flight == 0 {
            return Err("max in-flight streams must be greater than zero".to_string());
        }
        let timeouts = ClientTimeouts {
            connect: Duration::from_millis(connect_timeout_ms),
            request: Duration::from_millis(request_timeout_ms),
        };
        FfiClient::connect(WorkerOptions {
            endpoint,
            certificate,
            identity,
            data_protection_key,
            compression,
            timeouts,
            retry: RetryPolicy {
                max_attempts: retry_max_attempts,
            },
            max_in_flight,
        })
        .map(FfiResult::connected)
    }))
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
            FfiOperation::Get | FfiOperation::Set | FfiOperation::Delete
                if application_key.is_empty() =>
            {
                Err("application key must not be empty".to_string())
            }
            FfiOperation::Ping | FfiOperation::Stats | FfiOperation::Sync
                if !application_key.is_empty() =>
            {
                Err("operation does not accept an application key".to_string())
            }
            FfiOperation::Ping
            | FfiOperation::Get
            | FfiOperation::Delete
            | FfiOperation::Stats
            | FfiOperation::Sync
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
/// are returned or stored exactly as supplied. Only `GET`, `SET`, and
/// `DELETE` accept an item ID.
///
/// # Safety
///
/// `client` must be live until this call returns. Every non-empty pointer and
/// length pair must identify readable memory for the duration of this call.
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
