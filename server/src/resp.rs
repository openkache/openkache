//! Plaintext RESP2 compatibility server for local development and native benchmarks.

use std::future::Future;
use std::io::Write as _;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use futures_util::stream::{FuturesUnordered, StreamExt};
use futures_util::{FutureExt, pin_mut, select};
use openkache_protocol::Opcode;
use sha2::{Digest, Sha256};
use smallvec::SmallVec;
use socket2::{Domain, Protocol, SockAddr, Socket, Type};

use crate::channel::{self, AsyncReceiver};
use crate::network_runtime::{self, TcpListener, TcpStream};
use crate::observability::{
    NetworkShard, NetworkWorkerId, ObservabilityService, ObservabilityState, Operation,
};
use crate::platform::StorageDeviceKind;
use crate::protocol::ItemId;
use crate::server::{
    NetworkRolePlacement, NetworkWorkerCompletion, NetworkWorkerReporter, Result, ServerError,
    launch_network_role, shutdown_network_workers_and_cache,
};
use crate::types::{StorageWriteOptions, StoredItemValue};
use crate::{AppConfig, NetworkWorkerCache, SetOutcome, ThreadedKvkache};

const MAX_ARRAY_ITEMS: usize = 64;
const MAX_BULK_BYTES: usize = 16 * 1024 * 1024;
const MAX_BUFFER_BYTES: usize = 32 * 1024 * 1024;
const READ_BUFFER_BYTES: usize = 64 * 1024;
type Command<'a> = SmallVec<[&'a [u8]; 4]>;

fn resp_item_id(application_key: &[u8]) -> ItemId {
    ItemId::new(Sha256::digest(application_key).into())
}

/// Plaintext RESP2 endpoint that dispatches directly to OpenKache storage workers.
pub struct RespServer {
    sockets: Vec<std::net::TcpListener>,
    local_addr: SocketAddr,
    cache: Arc<ThreadedKvkache>,
    network: crate::NetworkConfig,
    request_timeout: Duration,
    observability: ObservabilityService,
}

impl RespServer {
    /// Binds a loopback-only plaintext RESP2 server for development and benchmarks.
    ///
    /// # Arguments
    ///
    /// * `address` - TCP loopback address on which RESP2 clients connect.
    /// * `config` - Network, storage, table, and timeout configuration.
    ///
    /// # Returns
    ///
    /// A ready server containing reuse-port TCP listeners and cache workers.
    ///
    /// # Errors
    ///
    /// Returns an error when the address is not loopback, configuration validation fails,
    /// socket binding fails, or cache startup fails.
    pub async fn bind_plaintext_for_development(
        address: SocketAddr,
        config: AppConfig,
    ) -> Result<Self> {
        if !address.ip().is_loopback() {
            return Err(ServerError::PlaintextRespRequiresLoopback(address));
        }
        config.validate()?;
        let network = config.network.clone();
        let request_timeout = Duration::from_micros(config.timeouts.request_max_time_us);
        let observability = ObservabilityService::new(
            network.worker_count,
            config.runtime.thread_count,
            &config.observability,
        )?;
        let observability_state = observability.state();
        let mut cache = ThreadedKvkache::start_validated_for_server_with_observability(
            config,
            observability_state,
        )?;
        let sockets = match bind_reuse_port_tcp_listeners(address, network.worker_count) {
            Ok(sockets) => sockets,
            Err(error) => {
                cache.shutdown()?;
                return Err(error.into());
            }
        };
        let local_addr = sockets[0].local_addr()?;
        Ok(Self {
            sockets,
            local_addr,
            cache: Arc::new(cache),
            network,
            request_timeout,
            observability,
        })
    }

    /// Returns the TCP address selected by the operating system.
    ///
    /// # Returns
    ///
    /// The bound local address shared by all reuse-port listeners.
    ///
    /// # Errors
    ///
    /// This accessor returns the server result type for symmetry with the QUIC server.
    pub fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.local_addr)
    }

    /// Returns the bound observability management address, when enabled.
    ///
    /// # Returns
    ///
    /// The address bound for the management listener, or `None` when
    /// observability metrics are disabled.
    pub fn metrics_addr(&self) -> Option<SocketAddr> {
        self.observability.metrics_addr()
    }

    /// Returns the conservative classification of the files opened by the
    /// storage workers during bind.
    ///
    /// # Returns
    ///
    /// The aggregate classification of every data and large-value file opened
    /// by the storage workers.
    pub fn storage_device_kind(&self) -> StorageDeviceKind {
        self.cache.storage_device_kind()
    }

    /// Accepts RESP2 connections until `shutdown` resolves, then flushes cache workers.
    ///
    /// # Arguments
    ///
    /// * `shutdown` - Future whose completion initiates graceful server shutdown.
    ///
    /// # Returns
    ///
    /// `Ok(())` after network threads and cache workers stop.
    ///
    /// # Errors
    ///
    /// Returns an error when a network worker fails or cache shutdown fails.
    pub async fn serve(self, shutdown: impl Future<Output = ()>) -> Result<()> {
        let Self {
            sockets,
            cache,
            network,
            request_timeout,
            observability: observability_service,
            ..
        } = self;
        let observability = observability_service.state();
        let (started_tx, started_rx) =
            channel::bounded::<std::result::Result<(), String>>(network.worker_count);
        let (finished_tx, finished_rx) =
            channel::bounded_sync_async::<NetworkWorkerCompletion>(network.worker_count);
        let mut workers = Vec::with_capacity(network.worker_count);
        let mut launch_error = None;

        for (worker_id, socket) in sockets.into_iter().enumerate() {
            let (stop_tx, stop_rx) = channel::bounded_sync_async(1);
            let started_tx = started_tx.clone();
            let finished_tx = finished_tx.clone();
            let worker_cache = Arc::clone(&cache);
            let worker_observability = Arc::clone(&observability);
            let cpu_id = network.cpu_ids[worker_id];
            let entries = network.io_uring_entries_per_worker;
            let event_interval = network.event_interval;
            let reporter = NetworkWorkerReporter::new(worker_id, started_tx, finished_tx);
            match launch_network_role(
                &cache,
                NetworkRolePlacement::new(
                    worker_id,
                    cpu_id,
                    format!("openkache-resp-{worker_id}"),
                    entries,
                    event_interval,
                    stop_tx,
                ),
                reporter,
                move |reporter| {
                    run_resp_role(
                        worker_id,
                        cpu_id,
                        socket,
                        worker_cache,
                        worker_observability,
                        request_timeout,
                        stop_rx,
                        reporter,
                    )
                },
            ) {
                Ok(worker) => workers.push(worker),
                Err(error) => {
                    launch_error.get_or_insert_with(|| error.to_string());
                }
            }
        }
        drop(started_tx);
        drop(finished_tx);

        let mut startup_error = launch_error;
        for _ in 0..network.worker_count {
            match started_rx.recv() {
                Ok(Ok(())) => {}
                Ok(Err(message)) => {
                    startup_error.get_or_insert(message);
                }
                Err(_) => {
                    startup_error
                        .get_or_insert_with(|| "RESP worker startup channel closed".into());
                    break;
                }
            }
        }
        if let Some(message) = startup_error {
            observability.set_failed();
            let remaining_completions = workers.len();
            shutdown_network_workers_and_cache(workers, &finished_rx, remaining_completions, cache)
                .await?;
            return Err(ServerError::NetworkWorker(message));
        }

        let metrics_handle = observability_service.start();
        let shutdown = shutdown.fuse();
        let worker_finished = finished_rx.recv_async_network().fuse();
        pin_mut!(shutdown, worker_finished);
        let (worker_failure, completed_workers, failed_worker) = select! {
            () = shutdown => (None, 0, None),
            result = worker_finished => match result {
                Ok((worker_id, Ok(()))) => (
                    Some(format!("RESP worker {worker_id} exited unexpectedly")),
                    1,
                    Some(worker_id),
                ),
                Ok((worker_id, Err(message))) => (
                    Some(format!("RESP worker {worker_id} failed: {message}")),
                    1,
                    Some(worker_id),
                ),
                Err(_) => (Some("RESP worker completion channel closed".into()), 1, None),
            },
        };
        let remaining_completions = workers.len().saturating_sub(completed_workers);
        if let Some(worker_id) = failed_worker {
            observability.network_worker_failed(worker_id);
        } else if worker_failure.is_some() {
            observability.set_failed();
        } else {
            observability.set_draining();
        }
        metrics_handle.stop();
        shutdown_network_workers_and_cache(workers, &finished_rx, remaining_completions, cache)
            .await?;
        match worker_failure {
            Some(message) => Err(ServerError::NetworkWorker(message)),
            None => Ok(()),
        }
    }
}

async fn run_resp_role(
    worker_id: usize,
    cpu_id: usize,
    socket: std::net::TcpListener,
    cache: Arc<ThreadedKvkache>,
    observability: Arc<ObservabilityState>,
    request_timeout: Duration,
    stop: AsyncReceiver<()>,
    mut reporter: NetworkWorkerReporter,
) -> Option<std::result::Result<(), String>> {
    let listener = match TcpListener::from_std(socket) {
        Ok(listener) => listener,
        Err(error) => {
            observability.network_worker_failed(worker_id);
            reporter.startup_failed(error.to_string());
            return None;
        }
    };
    if let Some(error) =
        crate::platform::cpu_assignment_error(&format!("RESP worker {worker_id}"), cpu_id)
    {
        observability.network_worker_failed(worker_id);
        reporter.startup_failed(error);
        return None;
    }
    if !reporter.started() {
        return None;
    }
    observability.network_worker_started(worker_id);
    Some(
        run_resp_worker(
            listener,
            Arc::clone(&cache),
            worker_id,
            observability,
            request_timeout,
            stop,
        )
        .await
        .map_err(|error| error.to_string()),
    )
}

fn bind_reuse_port_tcp_listeners(
    address: SocketAddr,
    worker_count: usize,
) -> std::io::Result<Vec<std::net::TcpListener>> {
    let mut sockets = Vec::with_capacity(worker_count);
    let mut bind_address = address;
    for worker_id in 0..worker_count {
        let socket = Socket::new(
            Domain::for_address(bind_address),
            Type::STREAM,
            Some(Protocol::TCP),
        )?;
        socket.set_reuse_address(true)?;
        socket.set_reuse_port(true)?;
        socket.bind(&SockAddr::from(bind_address))?;
        socket.listen(1_024)?;
        let listener = std::net::TcpListener::from(socket);
        if worker_id == 0 {
            bind_address = listener.local_addr()?;
        }
        sockets.push(listener);
    }
    Ok(sockets)
}

async fn run_resp_worker(
    listener: TcpListener,
    cache: Arc<ThreadedKvkache>,
    worker_id: usize,
    observability: Arc<ObservabilityState>,
    request_timeout: Duration,
    stop: AsyncReceiver<()>,
) -> std::io::Result<()> {
    let network_shard = observability.network_shard(NetworkWorkerId(worker_id));
    let cache = NetworkWorkerCache::new(cache, network_shard.worker_id());
    let mut connections = FuturesUnordered::new();
    loop {
        if connections.is_empty() {
            let incoming = listener.accept().fuse();
            let stopping = stop.recv_async_network().fuse();
            pin_mut!(incoming, stopping);
            select! {
                incoming = incoming => {
                    let (stream, _) = incoming?;
                    stream.set_nodelay(true)?;
                    network_shard.connection_started();
                    connections.push(serve_resp_connection(
                        stream,
                        &cache,
                        network_shard,
                        request_timeout,
                    ));
                }
                _ = stopping => break,
            }
        } else {
            let incoming = listener.accept().fuse();
            let completed = connections.next().fuse();
            let stopping = stop.recv_async_network().fuse();
            pin_mut!(incoming, completed, stopping);
            select! {
                incoming = incoming => {
                    let (stream, _) = incoming?;
                    stream.set_nodelay(true)?;
                    network_shard.connection_started();
                    connections.push(serve_resp_connection(
                        stream,
                        &cache,
                        network_shard,
                        request_timeout,
                    ));
                }
                _ = completed => {}
                _ = stopping => break,
            }
        }
    }
    Ok(())
}

async fn serve_resp_connection(
    mut stream: TcpStream,
    cache: &NetworkWorkerCache,
    network_shard: NetworkShard<'_>,
    request_timeout: Duration,
) -> std::io::Result<()> {
    let _connection_guard = ActiveRespConnection { network_shard };
    let mut pending = Vec::with_capacity(READ_BUFFER_BYTES);
    let mut responses = Vec::new();
    loop {
        let input = vec![0_u8; READ_BUFFER_BYTES];
        let (bytes_read, input) = match stream.read(input).await {
            Ok(result) => result,
            Err(error) => {
                if error.kind() == std::io::ErrorKind::TimedOut {
                    network_shard.request_read_timeout();
                }
                return Err(error);
            }
        };
        if bytes_read == 0 {
            return Ok(());
        }
        pending.extend_from_slice(&input[..bytes_read]);
        if pending.len() > MAX_BUFFER_BYTES {
            network_shard.protocol_error();
            responses.clear();
            error(&mut responses, "request buffer exceeds RESP limit");
            if let Err(error) = write_with_timeout(&mut stream, responses, request_timeout).await {
                network_shard.response_write_failure();
                return Err(error);
            }
            return Ok(());
        }

        let mut consumed = 0;
        responses.clear();
        let mut close = false;
        while consumed < pending.len() {
            match parse_command(&pending[consumed..]) {
                ParseResult::Complete {
                    command,
                    consumed: command_bytes,
                } => {
                    consumed += command_bytes;
                    let response_start = responses.len();
                    let request_started = std::time::Instant::now();
                    let operation = operation_for_command(&command);
                    close = execute_command(cache, &command, &mut responses).await;
                    network_shard.record_request(
                        operation,
                        status_for_resp_response(&responses[response_start..], operation),
                        request_started.elapsed(),
                    );
                    if close {
                        break;
                    }
                }
                ParseResult::Incomplete => break,
                ParseResult::Invalid(message) => {
                    network_shard.protocol_error();
                    error(&mut responses, &message);
                    close = true;
                    consumed = pending.len();
                    break;
                }
            }
        }
        if consumed == pending.len() {
            pending.clear();
        } else if consumed > 0 {
            pending.drain(..consumed);
        }
        if !responses.is_empty() {
            responses = match write_with_timeout(&mut stream, responses, request_timeout).await {
                Ok(responses) => responses,
                Err(error) => {
                    network_shard.response_write_failure();
                    return Err(error);
                }
            };
        }
        if close {
            return Ok(());
        }
    }
}

struct ActiveRespConnection<'a> {
    network_shard: NetworkShard<'a>,
}

impl Drop for ActiveRespConnection<'_> {
    fn drop(&mut self) {
        self.network_shard.connection_finished();
    }
}

async fn write_with_timeout(
    stream: &mut TcpStream,
    response: Vec<u8>,
    timeout: Duration,
) -> std::io::Result<Vec<u8>> {
    match network_runtime::timeout(timeout, stream.write_all(response)).await {
        Ok(result) => result,
        Err(_) => Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "RESP response write timed out",
        )),
    }
}

pub(crate) fn operation_for_command(command: &[&[u8]]) -> Operation {
    match classify_command(command) {
        RespCommandKind::Ping => operation_for_opcode(Opcode::Ping),
        RespCommandKind::Get => operation_for_opcode(Opcode::Get),
        RespCommandKind::Set => operation_for_opcode(Opcode::Set),
        RespCommandKind::Delete => operation_for_opcode(Opcode::Delete),
        RespCommandKind::Stats => operation_for_opcode(Opcode::Stats),
        RespCommandKind::Sync => operation_for_opcode(Opcode::Sync),
        RespCommandKind::Select
        | RespCommandKind::Client
        | RespCommandKind::Quit
        | RespCommandKind::Unknown
        | RespCommandKind::Empty => Operation::unknown(),
    }
}

const fn operation_for_opcode(opcode: Opcode) -> Operation {
    crate::operation_contract::telemetry_operation(opcode)
}

/// RESP command names are a compatibility adapter concern. Classifying them
/// once keeps telemetry and execution on the same small registry instead of
/// repeating case-insensitive command branches in both paths.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RespCommandKind {
    Ping,
    Get,
    Set,
    Delete,
    Stats,
    Sync,
    Select,
    Client,
    Quit,
    Unknown,
    Empty,
}

fn classify_command(command: &[&[u8]]) -> RespCommandKind {
    let Some(name) = command.first() else {
        return RespCommandKind::Empty;
    };
    if name.eq_ignore_ascii_case(b"PING") {
        RespCommandKind::Ping
    } else if name.eq_ignore_ascii_case(b"GET") {
        RespCommandKind::Get
    } else if name.eq_ignore_ascii_case(b"SET") {
        RespCommandKind::Set
    } else if name.eq_ignore_ascii_case(b"DEL") {
        RespCommandKind::Delete
    } else if name.eq_ignore_ascii_case(b"OPENKACHE.STATS") {
        RespCommandKind::Stats
    } else if name.eq_ignore_ascii_case(b"OPENKACHE.SYNC") {
        RespCommandKind::Sync
    } else if name.eq_ignore_ascii_case(b"SELECT") {
        RespCommandKind::Select
    } else if name.eq_ignore_ascii_case(b"CLIENT") {
        RespCommandKind::Client
    } else if name.eq_ignore_ascii_case(b"QUIT") {
        RespCommandKind::Quit
    } else {
        RespCommandKind::Unknown
    }
}

pub(crate) fn status_for_resp_response(
    response: &[u8],
    operation: Operation,
) -> openkache_protocol::Status {
    if response.starts_with(b"+") {
        return openkache_protocol::Status::Ok;
    }
    if response.starts_with(b"$-1\r\n") {
        return if operation == operation_for_opcode(Opcode::Set) {
            openkache_protocol::Status::NotStored
        } else {
            openkache_protocol::Status::NotFound
        };
    }
    if response.starts_with(b"$") {
        return openkache_protocol::Status::Ok;
    }
    if response.starts_with(b":") {
        return if operation == operation_for_opcode(Opcode::Delete) {
            if response.starts_with(b":0\r\n") {
                openkache_protocol::Status::NotFound
            } else {
                openkache_protocol::Status::Deleted
            }
        } else {
            openkache_protocol::Status::Ok
        };
    }
    if response.starts_with(b"-ERR") {
        return if operation == Operation::unknown() {
            openkache_protocol::Status::UnsupportedOpcode
        } else {
            openkache_protocol::Status::InternalError
        };
    }
    openkache_protocol::Status::InternalError
}

async fn execute_command(
    cache: &NetworkWorkerCache,
    command: &[&[u8]],
    response: &mut Vec<u8>,
) -> bool {
    match classify_command(command) {
        RespCommandKind::Ping => simple(response, "PONG"),
        RespCommandKind::Get => match command {
            [_, application_key] => {
                let item_id = resp_item_id(application_key);
                let storage_key = cache.storage_key_for_identity(&item_id.storage_bytes());
                match cache
                    .get_storage_key(storage_key, operation_for_opcode(Opcode::Get))
                    .await
                {
                    Ok(Some(value)) => bulk(response, Some(&value.bytes)),
                    Ok(None) => bulk(response, None),
                    Err(cache_error) => resp_cache_error(response, cache_error),
                }
            }
            _ => error(response, "wrong number of arguments for GET"),
        },
        RespCommandKind::Set => match command {
            [_, application_key, value] => {
                let item_id = resp_item_id(application_key);
                let storage_key = cache.storage_key_for_identity(&item_id.storage_bytes());
                match cache
                    .set_storage_key(
                        storage_key,
                        StoredItemValue::new(value.to_vec()),
                        StorageWriteOptions::default(),
                        operation_for_opcode(Opcode::Set),
                    )
                    .await
                {
                    Ok(SetOutcome::Created | SetOutcome::Replaced) => simple(response, "OK"),
                    Ok(SetOutcome::NotStored) => bulk(response, None),
                    Err(cache_error) => resp_cache_error(response, cache_error),
                }
            }
            _ => error(response, "SET options are not supported"),
        },
        RespCommandKind::Delete => {
            if command.len() < 2 {
                error(response, "wrong number of arguments for DEL");
            } else {
                let mut deleted = 0;
                for application_key in &command[1..] {
                    let item_id = resp_item_id(application_key);
                    let storage_key = cache.storage_key_for_identity(&item_id.storage_bytes());
                    match cache
                        .delete_storage_key(storage_key, operation_for_opcode(Opcode::Delete))
                        .await
                    {
                        Ok(true) => deleted += 1,
                        Ok(false) => {}
                        Err(cache_error) => {
                            resp_cache_error(response, cache_error);
                            return false;
                        }
                    }
                }
                integer(response, deleted);
            }
        }
        RespCommandKind::Stats => match command {
            [_] => match cache.stats(operation_for_opcode(Opcode::Stats)).await {
                Ok(stats) => {
                    let stats = stats.join("\n");
                    bulk(response, Some(stats.as_bytes()));
                }
                Err(cache_error) => resp_cache_error(response, cache_error),
            },
            _ => error(response, "wrong number of arguments for OPENKACHE.STATS"),
        },
        RespCommandKind::Sync => match command {
            [_] => match cache.sync(operation_for_opcode(Opcode::Sync)).await {
                Ok(()) => simple(response, "OK"),
                Err(cache_error) => resp_cache_error(response, cache_error),
            },
            _ => error(response, "wrong number of arguments for OPENKACHE.SYNC"),
        },
        RespCommandKind::Select | RespCommandKind::Client => {
            simple(response, "OK");
        }
        RespCommandKind::Quit => {
            simple(response, "OK");
            return true;
        }
        RespCommandKind::Unknown => error(response, "unsupported command"),
        RespCommandKind::Empty => error(response, "empty command"),
    }
    false
}

fn simple(response: &mut Vec<u8>, message: &str) {
    response.push(b'+');
    response.extend_from_slice(message.as_bytes());
    response.extend_from_slice(b"\r\n");
}

fn error(response: &mut Vec<u8>, message: &str) {
    response.extend_from_slice(b"-ERR ");
    response.extend(message.bytes().map(|byte| {
        if matches!(byte, b'\r' | b'\n') {
            b' '
        } else {
            byte
        }
    }));
    response.extend_from_slice(b"\r\n");
}

fn integer(response: &mut Vec<u8>, value: u64) {
    write!(response, ":{value}\r\n").expect("writing to a Vec cannot fail");
}

fn bulk(response: &mut Vec<u8>, value: Option<&[u8]>) {
    match value {
        Some(value) => {
            write!(response, "${}\r\n", value.len()).expect("writing to a Vec cannot fail");
            response.extend_from_slice(value);
            response.extend_from_slice(b"\r\n");
        }
        None => response.extend_from_slice(b"$-1\r\n"),
    }
}

fn resp_cache_error(response: &mut Vec<u8>, cache_error: crate::KvError) {
    response.extend_from_slice(b"-ERR ");
    let message_start = response.len();
    write!(response, "{cache_error}").expect("writing to a Vec cannot fail");
    for byte in &mut response[message_start..] {
        if matches!(*byte, b'\r' | b'\n') {
            *byte = b' ';
        }
    }
    response.extend_from_slice(b"\r\n");
}

pub(crate) enum ParseResult<'a> {
    Complete {
        command: Command<'a>,
        consumed: usize,
    },
    Incomplete,
    Invalid(String),
}

pub(crate) fn parse_command<'a>(input: &'a [u8]) -> ParseResult<'a> {
    if input.is_empty() {
        return ParseResult::Incomplete;
    }
    if input[0] != b'*' {
        return parse_inline_command(input);
    }

    let Some((array_line, mut offset)) = line_at(input, 0) else {
        return ParseResult::Incomplete;
    };
    let item_count = match parse_decimal(&array_line[1..]) {
        Ok(value) if value <= MAX_ARRAY_ITEMS => value,
        Ok(value) => {
            return ParseResult::Invalid(format!(
                "array has {value} items; maximum is {MAX_ARRAY_ITEMS}"
            ));
        }
        Err(message) => return ParseResult::Invalid(message),
    };

    let mut items = Command::with_capacity(item_count);
    for _ in 0..item_count {
        let Some((bulk_line, next_offset)) = line_at(input, offset) else {
            return ParseResult::Incomplete;
        };
        if bulk_line.first() != Some(&b'$') {
            return ParseResult::Invalid("expected RESP bulk string".into());
        }
        let bulk_length = match parse_decimal(&bulk_line[1..]) {
            Ok(value) if value <= MAX_BULK_BYTES => value,
            Ok(value) => {
                return ParseResult::Invalid(format!(
                    "bulk string has {value} bytes; maximum is {MAX_BULK_BYTES}"
                ));
            }
            Err(message) => return ParseResult::Invalid(message),
        };
        let Some(data_end) = next_offset.checked_add(bulk_length) else {
            return ParseResult::Invalid("bulk length overflow".into());
        };
        let Some(frame_end) = data_end.checked_add(2) else {
            return ParseResult::Invalid("bulk frame overflow".into());
        };
        if input.len() < frame_end {
            return ParseResult::Incomplete;
        }
        if &input[data_end..frame_end] != b"\r\n" {
            return ParseResult::Invalid("bulk string is missing CRLF".into());
        }
        items.push(&input[next_offset..data_end]);
        offset = frame_end;
    }

    ParseResult::Complete {
        command: items,
        consumed: offset,
    }
}

fn parse_inline_command<'a>(input: &'a [u8]) -> ParseResult<'a> {
    let Some((line, consumed)) = line_at(input, 0) else {
        return ParseResult::Incomplete;
    };
    let items: Command<'a> = line
        .split(|byte| byte.is_ascii_whitespace())
        .filter(|item| !<[u8]>::is_empty(item))
        .collect();
    if items.is_empty() {
        return ParseResult::Invalid("empty inline command".into());
    }
    ParseResult::Complete {
        command: items,
        consumed,
    }
}

fn line_at(input: &[u8], offset: usize) -> Option<(&[u8], usize)> {
    let relative_end = input
        .get(offset..)?
        .windows(2)
        .position(|pair| pair == b"\r\n")?;
    let end = offset + relative_end;
    Some((&input[offset..end], end + 2))
}

fn parse_decimal(bytes: &[u8]) -> std::result::Result<usize, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "RESP length is not UTF-8".to_string())?;
    text.parse::<usize>()
        .map_err(|_| format!("invalid RESP length {text:?}"))
}
