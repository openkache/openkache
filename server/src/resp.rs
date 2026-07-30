//! Plaintext RESP2 compatibility server for local development and native benchmarks.

use std::future::Future;
use std::io::Write as _;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use compio::BufResult;
use compio::buf::{IntoInner, IoBuf};
use compio::io::{AsyncRead, AsyncWriteExt};
use compio::net::{TcpListener, TcpStream};
use futures_util::stream::{FuturesUnordered, StreamExt};
use futures_util::{FutureExt, pin_mut, select};
use openkache_protocol::{ItemKey, SetOptions, ValueFlags};
use sha2::{Digest, Sha256};
use smallvec::SmallVec;
use socket2::{Domain, Protocol, SockAddr, Socket, Type};

use crate::channel::{self, AsyncReceiver};
use crate::server::{
    NetworkRolePlacement, NetworkWorkerCompletion, NetworkWorkerReporter, Result, ServerError,
    launch_network_role, shutdown_network_workers_and_cache,
};
use crate::types::StoredItemValue;
use crate::{AppConfig, SetOutcome, ThreadedKvkache};

const MAX_ARRAY_ITEMS: usize = 64;
const MAX_BULK_BYTES: usize = 16 * 1024 * 1024;
const MAX_BUFFER_BYTES: usize = 32 * 1024 * 1024;
const READ_BUFFER_BYTES: usize = 64 * 1024;
type Command<'a> = SmallVec<[&'a [u8]; 4]>;

fn resp_item_key(key: &[u8]) -> ItemKey {
    ItemKey::new(Sha256::digest(key).into())
}

/// Plaintext RESP2 endpoint that dispatches directly to OpenKache storage workers.
pub struct RespServer {
    sockets: Vec<std::net::TcpListener>,
    local_addr: SocketAddr,
    cache: Arc<ThreadedKvkache>,
    network: crate::NetworkConfig,
    request_timeout: Duration,
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
        let mut cache = ThreadedKvkache::start_validated_for_server(config)?;
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
            ..
        } = self;
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
            let cpu_id = network.cpu_ids[worker_id];
            let entries = network.io_uring_entries_per_worker;
            let event_interval = network.event_interval;
            let reporter = NetworkWorkerReporter::new(worker_id, started_tx, finished_tx);
            match launch_network_role(
                &cache,
                NetworkRolePlacement::new(
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
            let remaining_completions = workers.len();
            shutdown_network_workers_and_cache(workers, &finished_rx, remaining_completions, cache)
                .await?;
            return Err(ServerError::NetworkWorker(message));
        }

        let shutdown = shutdown.fuse();
        let worker_finished = finished_rx.recv_async().fuse();
        pin_mut!(shutdown, worker_finished);
        let (worker_failure, completed_workers) = select! {
            () = shutdown => (None, 0),
            result = worker_finished => (Some(match result {
                Ok((worker_id, Ok(()))) => format!("RESP worker {worker_id} exited unexpectedly"),
                Ok((worker_id, Err(message))) => {
                    format!("RESP worker {worker_id} failed: {message}")
                }
                Err(_) => "RESP worker completion channel closed".into(),
            }), 1),
        };
        let remaining_completions = workers.len().saturating_sub(completed_workers);
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
    request_timeout: Duration,
    stop: AsyncReceiver<()>,
    mut reporter: NetworkWorkerReporter,
) -> Option<std::result::Result<(), String>> {
    let listener = match TcpListener::from_std(socket) {
        Ok(listener) => listener,
        Err(error) => {
            reporter.startup_failed(error.to_string());
            return None;
        }
    };
    let actual_cpu = unsafe { libc::sched_getcpu() };
    if actual_cpu < 0 || actual_cpu as usize != cpu_id {
        reporter.startup_failed(format!(
            "RESP worker {worker_id} expected CPU {cpu_id}, running on CPU {actual_cpu}"
        ));
        return None;
    }
    if !reporter.started() {
        return None;
    }
    Some(
        run_resp_worker(listener, &cache, request_timeout, stop)
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
    cache: &ThreadedKvkache,
    request_timeout: Duration,
    stop: AsyncReceiver<()>,
) -> std::io::Result<()> {
    let mut connections = FuturesUnordered::new();
    loop {
        if connections.is_empty() {
            let incoming = listener.accept().fuse();
            let stopping = stop.recv_async().fuse();
            pin_mut!(incoming, stopping);
            select! {
                incoming = incoming => {
                    let (stream, _) = incoming?;
                    stream.set_nodelay(true)?;
                    connections.push(serve_resp_connection(stream, cache, request_timeout));
                }
                _ = stopping => break,
            }
        } else {
            let incoming = listener.accept().fuse();
            let completed = connections.next().fuse();
            let stopping = stop.recv_async().fuse();
            pin_mut!(incoming, completed, stopping);
            select! {
                incoming = incoming => {
                    let (stream, _) = incoming?;
                    stream.set_nodelay(true)?;
                    connections.push(serve_resp_connection(stream, cache, request_timeout));
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
    cache: &ThreadedKvkache,
    request_timeout: Duration,
) -> std::io::Result<()> {
    let mut pending = Vec::with_capacity(READ_BUFFER_BYTES);
    let mut responses = Vec::new();
    loop {
        let read_start = pending.len();
        pending.reserve(READ_BUFFER_BYTES);
        let read = compio::runtime::time::timeout(
            request_timeout,
            stream.read(pending.slice(read_start..read_start + READ_BUFFER_BYTES)),
        )
        .await;
        let BufResult(result, input) = match read {
            Ok(read) => read,
            Err(_) => return Ok(()),
        };
        let bytes_read = result?;
        pending = input.into_inner();
        if bytes_read == 0 {
            return Ok(());
        }
        if pending.len() > MAX_BUFFER_BYTES {
            responses.clear();
            error(&mut responses, "request buffer exceeds RESP limit");
            write_with_timeout(&mut stream, responses, request_timeout).await?;
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
                    close = execute_command(cache, &command, &mut responses).await;
                    if close {
                        break;
                    }
                }
                ParseResult::Incomplete => break,
                ParseResult::Invalid(message) => {
                    error(&mut responses, &message);
                    close = true;
                    consumed = pending.len();
                    break;
                }
            }
        }
        if consumed > 0 {
            pending.drain(..consumed);
        }
        if !responses.is_empty() {
            responses = write_with_timeout(&mut stream, responses, request_timeout).await?;
        }
        if close {
            return Ok(());
        }
    }
}

async fn write_with_timeout(
    stream: &mut TcpStream,
    response: Vec<u8>,
    timeout: Duration,
) -> std::io::Result<Vec<u8>> {
    match compio::runtime::time::timeout(timeout, stream.write_all(response)).await {
        Ok(BufResult(result, response)) => {
            result?;
            Ok(response)
        }
        Err(_) => Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "RESP response write timed out",
        )),
    }
}

async fn execute_command(
    cache: &ThreadedKvkache,
    command: &[&[u8]],
    response: &mut Vec<u8>,
) -> bool {
    match command.first() {
        Some(name) if name.eq_ignore_ascii_case(b"PING") => simple(response, "PONG"),
        Some(name) if name.eq_ignore_ascii_case(b"GET") => match command {
            [_, key] => match cache.get_async(resp_item_key(key)).await {
                Ok(Some(value)) if value.flags == ValueFlags::NONE => {
                    bulk(response, Some(&value.bytes));
                }
                Ok(Some(_)) => error(response, "RESP cannot decode transformed client values"),
                Ok(None) => bulk(response, None),
                Err(cache_error) => resp_cache_error(response, cache_error),
            },
            _ => error(response, "wrong number of arguments for GET"),
        },
        Some(name) if name.eq_ignore_ascii_case(b"SET") => match command {
            [_, key, value] => match cache
                .set_async_with_options(
                    resp_item_key(key),
                    StoredItemValue::plain(value.to_vec()),
                    SetOptions::NONE,
                )
                .await
            {
                Ok(SetOutcome::Created | SetOutcome::Replaced) => simple(response, "OK"),
                Ok(SetOutcome::NotStored) => bulk(response, None),
                Err(cache_error) => resp_cache_error(response, cache_error),
            },
            _ => error(response, "SET options are not supported"),
        },
        Some(name) if name.eq_ignore_ascii_case(b"DEL") => {
            if command.len() < 2 {
                error(response, "wrong number of arguments for DEL");
            } else {
                let mut deleted = 0;
                for key in &command[1..] {
                    match cache.delete_async(resp_item_key(key)).await {
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
        Some(name)
            if name.eq_ignore_ascii_case(b"SELECT") || name.eq_ignore_ascii_case(b"CLIENT") =>
        {
            simple(response, "OK");
        }
        Some(name) if name.eq_ignore_ascii_case(b"QUIT") => {
            simple(response, "OK");
            return true;
        }
        Some(_) => error(response, "unsupported command"),
        None => error(response, "empty command"),
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
