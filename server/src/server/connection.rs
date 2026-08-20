use super::*;
use std::collections::VecDeque;
use std::fmt::Write as _;
use futures_util::{select_biased};
use crate::protocol::Response;

#[derive(Clone)]
pub(super) struct NetworkWorkerLimits {
    pub(super) worker_id: usize,
    pub(super) request_timeout: Duration,
    pub(super) max_stream_lanes: usize,
    pub(super) request_budget: RequestBudget,
    pub(super) namespaces: Arc<Mutex<NamespaceRegistry>>,
    pub(super) observability: Arc<ObservabilityState>,
    pub(super) capabilities: Arc<dyn CapabilityCatalog>,
    pub(super) experimental_api_enabled: bool,
    pub(super) experimental_api_revision: Option<String>,
}

pub(super) fn prepare_network_worker(
    cache: Arc<ThreadedKvkache>,
    limits: &NetworkWorkerLimits,
) -> std::result::Result<Arc<operation_execution_state::OperationRuntime>, &'static str> {
    let network_shard = limits
        .observability
        .network_shard(NetworkWorkerId(limits.worker_id));
    let cache = Arc::new(NetworkWorkerCache::new(cache, network_shard.worker_id()));
    let runtime = operation_registrations::build_operation_runtime(
        limits.capabilities.as_ref(),
        Arc::clone(&cache),
        Arc::clone(&limits.namespaces),
        Arc::clone(&limits.observability),
        operation_execution_state::ExperimentalApiGate::new(
            limits.experimental_api_enabled,
            limits.experimental_api_revision.clone(),
        ),
    )?;
    Ok(runtime)
}

pub(super) async fn run_selected_endpoint(
    endpoint: ServerEndpoint,
    access_policy: &AccessPolicy,
    limits: NetworkWorkerLimits,
    runtime: Arc<operation_execution_state::OperationRuntime>,
    stop: AsyncReceiver<()>,
) -> std::result::Result<(), TransportError> {
    match endpoint {
        #[cfg(feature = "quic-quinn")]
        ServerEndpoint::Quinn(endpoint) => {
            run_network_worker(endpoint, access_policy, limits, runtime, stop).await
        }
        #[cfg(feature = "quic-noq")]
        ServerEndpoint::Noq(endpoint) => {
            run_network_worker(endpoint, access_policy, limits, runtime, stop).await
        }
        #[cfg(feature = "quic-quiche")]
        ServerEndpoint::Quiche(endpoint) => {
            run_network_worker(endpoint, access_policy, limits, runtime, stop).await
        }
    }
}

async fn run_network_worker<E: TransportEndpoint>(
    endpoint: E,
    access_policy: &AccessPolicy,
    limits: NetworkWorkerLimits,
    runtime: Arc<operation_execution_state::OperationRuntime>,
    stop: AsyncReceiver<()>,
) -> std::result::Result<(), TransportError> {
    let NetworkWorkerLimits {
        worker_id,
        request_timeout,
        max_stream_lanes,
        request_budget,
        namespaces: _,
        observability,
        capabilities: _,
        ..
    } = limits;
    let network_shard = observability.network_shard(NetworkWorkerId(worker_id));
    let mut connections = FuturesUnordered::new();
    loop {
        if connections.is_empty() {
            let incoming = endpoint.wait_incoming().fuse();
            let stopping = stop.recv_async_network().fuse();
            pin_mut!(incoming, stopping);
            select! {
                incoming = incoming => {
                    let Some(incoming) = incoming else { break };
                    connections.push(serve_incoming(
                        incoming, network_shard, access_policy, request_timeout,
                        max_stream_lanes, request_budget.clone(),
                        Arc::clone(&runtime),
                    ));
                }
                _ = stopping => break,
            }
        } else {
            let incoming = endpoint.wait_incoming().fuse();
            let completed = connections.next().fuse();
            let stopping = stop.recv_async_network().fuse();
            pin_mut!(incoming, completed, stopping);
            select! {
                incoming = incoming => {
                    let Some(incoming) = incoming else { break };
                    connections.push(serve_incoming(
                        incoming, network_shard, access_policy, request_timeout,
                        max_stream_lanes, request_budget.clone(),
                        Arc::clone(&runtime),
                    ));
                }
                _ = completed => {}
                _ = stopping => break,
            }
        }
    }
    endpoint.close(b"server shutting down");
    while connections.next().await.is_some() {}
    endpoint.shutdown().await
}

/// Completes one QUIC handshake and serves the accepted connection.
async fn serve_incoming<I: TransportIncoming>(
    incoming: I,
    network_shard: NetworkShard<'_>,
    access_policy: &AccessPolicy,
    request_timeout: Duration,
    max_stream_lanes: usize,
    request_budget: RequestBudget,
    runtime: Arc<operation_execution_state::OperationRuntime>,
) {
    match incoming.connect().await {
        Ok(mut connection) => {
            network_shard.connection_started();
            network_shard.handshake_succeeded();
            let peer_certificate = connection.take_peer_certificate();
            let authorization = if access_policy.permits_administration(peer_certificate.as_ref()) {
                operation_authorization::AuthorizationContext::administrator()
            } else {
                operation_authorization::AuthorizationContext::public()
            };
            serve_connection(
                connection,
                network_shard,
                authorization,
                request_timeout,
                max_stream_lanes,
                request_budget,
                runtime,
            )
            .await;
            network_shard.connection_finished();
        }
        Err(_) => network_shard.handshake_failed(),
    }
}

/// Multiplexes bounded reusable request lanes for one QUIC connection.
async fn serve_connection<C: TransportConnection>(
    connection: C,
    network_shard: NetworkShard<'_>,
    authorization: operation_authorization::AuthorizationContext,
    request_timeout: Duration,
    max_stream_lanes: usize,
    request_budget: RequestBudget,
    runtime: Arc<operation_execution_state::OperationRuntime>,
) {
    let mut streams = FuturesUnordered::new();
    let mut accept_uni = true;
    loop {
        if streams.is_empty() {
            let incoming_bi = connection.accept_bi().fuse();
            let incoming_uni = if accept_uni {
                Some(connection.accept_uni().fuse())
            } else {
                None
            };
            pin_mut!(incoming_bi);
            if let Some(incoming_uni) = incoming_uni {
                pin_mut!(incoming_uni);
                select! {
                    incoming = incoming_bi => match incoming {
                        Ok((send, receive)) => {
                            network_shard.stream_started();
                            streams.push(serve_stream(
                                send,
                                receive,
                                network_shard,
                                authorization.clone(),
                                request_timeout,
                                request_budget.clone(),
                                Arc::clone(&runtime),
                            ));
                        }
                        Err(_) => break,
                    },
                    uni = incoming_uni => match uni {
                        Ok(mut receive) => receive.stop(),
                        Err(_) => accept_uni = false,
                    },
                }
            } else {
                match incoming_bi.await {
                    Ok((send, receive)) => {
                        network_shard.stream_started();
                        streams.push(serve_stream(
                            send,
                            receive,
                            network_shard,
                            authorization.clone(),
                            request_timeout,
                            request_budget.clone(),
                            Arc::clone(&runtime),
                        ));
                    }
                    Err(_) => break,
                }
            }
            continue;
        }
        if streams.len() >= max_stream_lanes {
            let completed = streams.next().fuse();
            if accept_uni {
                let incoming_uni = connection.accept_uni().fuse();
                pin_mut!(completed, incoming_uni);
                select! {
                    outcome = completed => {
                        if matches!(outcome, Some(LaneOutcome::Malformed | LaneOutcome::Unknown)) {
                            connection.close(1, b"lane failure before a mutation outcome was known");
                            break;
                        }
                    }
                    uni = incoming_uni => match uni {
                        Ok(mut receive) => receive.stop(),
                        Err(_) => accept_uni = false,
                    },
                }
            } else if let Some(outcome) = completed.await {
                if matches!(outcome, LaneOutcome::Malformed | LaneOutcome::Unknown) {
                    connection.close(1, b"lane failure before a mutation outcome was known");
                    break;
                }
            }
            continue;
        }
        let incoming_bi = connection.accept_bi().fuse();
        let incoming_uni = if accept_uni {
            Some(connection.accept_uni().fuse())
        } else {
            None
        };
        let completed = streams.next().fuse();
        pin_mut!(incoming_bi, completed);
        if let Some(incoming_uni) = incoming_uni {
            pin_mut!(incoming_uni);
            select! {
                incoming = incoming_bi => match incoming {
                    Ok((send, receive)) => {
                        network_shard.stream_started();
                        streams.push(serve_stream(
                            send,
                            receive,
                            network_shard,
                            authorization.clone(),
                            request_timeout,
                            request_budget.clone(),
                            Arc::clone(&runtime),
                        ));
                    }
                    Err(_) => break,
                },
                uni = incoming_uni => match uni {
                    Ok(mut receive) => receive.stop(),
                    Err(_) => accept_uni = false,
                },
                completed = completed => {
                    if let Some(outcome) = completed {
                        if matches!(outcome, LaneOutcome::Malformed | LaneOutcome::Unknown) {
                            connection.close(1, b"lane failure before a mutation outcome was known");
                            break;
                        }
                    }
                },
            }
        } else {
            select! {
                incoming = incoming_bi => match incoming {
                    Ok((send, receive)) => {
                        network_shard.stream_started();
                        streams.push(serve_stream(
                            send,
                            receive,
                            network_shard,
                            authorization.clone(),
                            request_timeout,
                            request_budget.clone(),
                            Arc::clone(&runtime),
                        ));
                    }
                    Err(_) => break,
                },
                completed = completed => {
                    if let Some(outcome) = completed {
                        if matches!(outcome, LaneOutcome::Malformed | LaneOutcome::Unknown) {
                            connection.close(1, b"lane failure before a mutation outcome was known");
                            break;
                        }
                    }
                },
            }
        }
    }
    while let Some(outcome) = streams.next().await {
        if matches!(outcome, LaneOutcome::Malformed | LaneOutcome::Unknown) {
            connection.close(1, b"lane failure before a mutation outcome was known");
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum LaneOutcome {
    Finished,
    Cancelled,
    Malformed,
    Transport,
    Unknown,
}

fn response_write_failure_outcome(unknown_on_write: bool) -> LaneOutcome {
    if unknown_on_write {
        LaneOutcome::Unknown
    } else {
        LaneOutcome::Cancelled
    }
}

enum LaneRequest {
    Frame(RequestFrame),
    Rejected {
        header: openkache_protocol::RequestFrameHeader,
        rejection: operation_dispatch::HeaderAdmissionRejection,
    },
    Overloaded {
        header: openkache_protocol::RequestFrameHeader,
        timed_out: bool,
    },
}

/// Advances one QUIC lane with bounded read-ahead and ordered effects.
///
/// The receive future and the operation future are polled together. This
/// allows a peer to pipeline complete frames while an earlier operation is
/// waiting on storage, while the queue bound keeps body permits and frame
/// allocations finite. Only the queue head executes and writes, preserving
/// effect and response order.
pub(super) async fn serve_stream<S: SendStream, R: ReceiveStream>(
    mut send: S,
    mut receive: R,
    network_shard: NetworkShard<'_>,
    authorization: operation_authorization::AuthorizationContext,
    request_timeout: Duration,
    request_budget: RequestBudget,
    runtime: Arc<operation_execution_state::OperationRuntime>,
) -> LaneOutcome {
    let _stream_guard = ActiveStream { network_shard };
    let mut task_storage = operation_registry::OperationTaskStorage::new();
    const MAX_ADMITTED_REQUESTS: usize = 8;
    let mut queue = VecDeque::with_capacity(MAX_ADMITTED_REQUESTS);
    let mut request_direction_open = true;
    let mut stop_receive = false;

    loop {
        if stop_receive {
            receive.stop();
            stop_receive = false;
        }
        let Some(request) = queue.pop_front() else {
            if !request_direction_open {
                let _ = send.finish();
                return LaneOutcome::Finished;
            }
            let progress = std::sync::atomic::AtomicBool::new(false);
            let result = receive
                .read_request(
                    crate::protocol::max_request_frame_bytes(),
                    request_timeout,
                    &request_budget,
                    &progress,
                    |header, prefix| {
                        operation_dispatch::admit_request_header(header, prefix, runtime.as_ref())
                    },
                )
                .await;
            match enqueue_read_result(
                result,
                &mut queue,
                &mut request_direction_open,
                network_shard,
            ) {
                ReadDisposition::Malformed => return LaneOutcome::Malformed,
                ReadDisposition::Transport => return LaneOutcome::Transport,
                ReadDisposition::Stop => stop_receive = true,
                ReadDisposition::Continue => {}
            }
            continue;
        };

        // The operation owns the dequeued item while reads continue in the
        // same lane. A bounded queue absorbs completed pipelined frames
        // without allowing unbounded body permits or allocations.
        let execute = execute_queued_request(
            request,
            &authorization,
            runtime.as_ref(),
            &mut task_storage,
            request_budget.clone(),
            request_timeout,
            network_shard,
        )
        .fuse();
        pin_mut!(execute);
        loop {
            if request_direction_open && queue.len() < MAX_ADMITTED_REQUESTS {
                // Once a read has delivered bytes, its local frame buffer owns
                // the only copy. Finish that read before writing the response
                // if execution wins the race; otherwise dropping it would
                // lose a partial pipelined frame and desynchronize framing.
                let progress = std::sync::atomic::AtomicBool::new(false);
                let read = receive
                    .read_request(
                        crate::protocol::max_request_frame_bytes(),
                        request_timeout,
                        &request_budget,
                        &progress,
                        |header, prefix| {
                            operation_dispatch::admit_request_header(
                                header,
                                prefix,
                                runtime.as_ref(),
                            )
                        },
                    )
                    .fuse();
                pin_mut!(read);
                select_biased! {
                    result = read => {
                        match enqueue_read_result(
                            result,
                            &mut queue,
                            &mut request_direction_open,
                            network_shard,
                        ) {
                            ReadDisposition::Malformed => return LaneOutcome::Malformed,
                            ReadDisposition::Transport => return LaneOutcome::Transport,
                            ReadDisposition::Stop => stop_receive = true,
                            ReadDisposition::Continue => {}
                        }
                    }
                    result = execute => {
                        if progress.load(std::sync::atomic::Ordering::Relaxed) {
                            match read.await {
                                result => match enqueue_read_result(
                                    result,
                                    &mut queue,
                                    &mut request_direction_open,
                                    network_shard,
                                ) {
                                    ReadDisposition::Malformed => return LaneOutcome::Malformed,
                                    ReadDisposition::Transport => return LaneOutcome::Transport,
                                    ReadDisposition::Stop => stop_receive = true,
                                    ReadDisposition::Continue => {}
                                },
                            }
                        }
                        match result {
                            ExecutionResult::Unknown => return LaneOutcome::Unknown,
                            ExecutionResult::Response(response) => {
                                let QueuedResponse {
                                    request_id,
                                    response,
                                    permit,
                                    request_permit,
                                    unknown_on_write,
                                    terminal,
                                } = response;
                                if !write_response(
                                    &mut send,
                                    request_id,
                                    response,
                                    request_timeout,
                                )
                                .await
                                {
                                    network_shard.response_write_failure();
                                    send.reset();
                                    drop(permit);
                                    drop(request_permit);
                                    return response_write_failure_outcome(unknown_on_write);
                                }
                                drop(permit);
                                drop(request_permit);
                                if terminal {
                                    return LaneOutcome::Finished;
                                }
                            }
                        }
                        break;
                    }
                }
            } else {
                match execute.await {
                    ExecutionResult::Unknown => return LaneOutcome::Unknown,
                    ExecutionResult::Response(response) => {
                        let QueuedResponse {
                            request_id,
                            response,
                            permit,
                            request_permit,
                            unknown_on_write,
                            terminal,
                        } = response;
                        if !write_response(&mut send, request_id, response, request_timeout).await {
                            network_shard.response_write_failure();
                            send.reset();
                            drop(permit);
                            drop(request_permit);
                            return response_write_failure_outcome(unknown_on_write);
                        }
                        drop(permit);
                        drop(request_permit);
                        if terminal {
                            return LaneOutcome::Finished;
                        }
                    }
                }
                break;
            }
        }
    }
}

enum ReadDisposition {
    Continue,
    Stop,
    Malformed,
    Transport,
}

fn enqueue_read_result(
    result: std::result::Result<
        RequestRead<operation_dispatch::HeaderAdmissionRejection>,
        StreamReadError,
    >,
    queue: &mut VecDeque<LaneRequest>,
    request_direction_open: &mut bool,
    network_shard: NetworkShard<'_>,
) -> ReadDisposition {
    match result {
        Ok(RequestRead::Frame(frame)) => queue.push_back(LaneRequest::Frame(frame)),
        Ok(RequestRead::Rejected { header, rejection }) => {
            network_shard.record_request(
                operation_contract::telemetry_operation(rejection.opcode()),
                rejection.status(),
                rejection.elapsed(),
            );
            if rejection.silently_close() {
                *request_direction_open = false;
                return ReadDisposition::Malformed;
            } else {
                queue.push_back(LaneRequest::Rejected { header, rejection });
            }
        }
        Ok(RequestRead::Overloaded { header, timed_out }) => {
            queue.push_back(LaneRequest::Overloaded { header, timed_out });
        }
        Ok(RequestRead::Finished) => *request_direction_open = false,
        Ok(RequestRead::Cancelled) => {
            *request_direction_open = false;
            return ReadDisposition::Stop;
        }
        Err(StreamReadError::Timeout) => {
            network_shard.request_read_timeout();
            network_shard.protocol_error();
            return ReadDisposition::Malformed;
        }
        Err(StreamReadError::TooLarge | StreamReadError::Malformed(_)) => {
            network_shard.protocol_error();
            return ReadDisposition::Malformed;
        }
        Err(StreamReadError::Transport(_)) => return ReadDisposition::Transport,
    }
    ReadDisposition::Continue
}

struct QueuedResponse {
    request_id: u64,
    response: operation_transport::OperationResponse,
    permit: Option<RequestBudgetPermit>,
    request_permit: Option<RequestBudgetPermit>,
    unknown_on_write: bool,
    terminal: bool,
}

enum ExecutionResult {
    Response(QueuedResponse),
    Unknown,
}

async fn execute_queued_request(
    request: LaneRequest,
    authorization: &operation_authorization::AuthorizationContext,
    runtime: &operation_execution_state::OperationRuntime,
    task_storage: &mut operation_registry::OperationTaskStorage,
    request_budget: RequestBudget,
    request_timeout: Duration,
    network_shard: NetworkShard<'_>,
) -> ExecutionResult {
    match request {
        LaneRequest::Rejected { header, rejection } => ExecutionResult::Response(QueuedResponse {
            request_id: header.request_id(),
            response: rejection.into_response(),
            permit: None,
            request_permit: None,
            unknown_on_write: false,
            terminal: false,
        }),
        LaneRequest::Overloaded { header, timed_out } => {
            let operation_id = operation_contract::operation_id_for_opcode(header.opcode());
            let response = if timed_out {
                operation_dispatch::timeout_response(
                    operation_id,
                    b"request memory budget timed out",
                )
            } else {
                operation_transport::contract_error_response_for_operation(
                    operation_id,
                    Status::Overloaded,
                    b"request exceeds the server memory budget",
                )
            };
            ExecutionResult::Response(QueuedResponse {
                request_id: header.request_id(),
                response,
                permit: None,
                request_permit: None,
                unknown_on_write: false,
                terminal: false,
            })
        }
        LaneRequest::Frame(frame) => {
            let RequestFrame {
                header,
                bytes: request_bytes,
                _permit: request_permit,
            } = frame;
            let request_id = header.request_id();
            let input = match request_projection::project_owned_request(request_bytes) {
                Ok(input) => input,
                Err(error) => {
                    return ExecutionResult::Response(QueuedResponse {
                        request_id,
                        response: wire_protocol_error_response(error).into(),
                        permit: None,
                        request_permit: Some(request_permit),
                        unknown_on_write: false,
                        terminal: false,
                    });
                }
            };
            let operation_id = input.operation_id();
            let operation = operation_contract::telemetry_operation_id(operation_id);
            let request_started = std::time::Instant::now();
            let may_mutate = operation_dispatch::may_mutate(runtime, operation_id);
            let response_permit = if let Some(bytes) =
                operation_dispatch::response_budget_bytes(runtime, operation_id)
            {
                match request_budget.acquire(bytes, request_timeout).await {
                    Ok(permit) => Some(permit),
                    Err(StreamReadError::Timeout) => {
                        return ExecutionResult::Response(QueuedResponse {
                            request_id,
                            response: operation_dispatch::timeout_response(
                                operation_id,
                                b"response memory budget timed out",
                            ),
                            permit: None,
                            request_permit: Some(request_permit),
                            unknown_on_write: false,
                            terminal: false,
                        });
                    }
                    Err(_) => {
                        return ExecutionResult::Response(QueuedResponse {
                            request_id,
                            response: operation_transport::contract_error_response_for_operation(
                                operation_id,
                                Status::Overloaded,
                                b"response exceeds the server memory budget",
                            ),
                            permit: None,
                            request_permit: Some(request_permit),
                            unknown_on_write: false,
                            terminal: false,
                        });
                    }
                }
            } else {
                None
            };
            match network_runtime::timeout(
                request_timeout,
                operation_dispatch::execute_request(input, authorization, runtime, task_storage),
            )
            .await
            {
                Ok(Some(response)) => {
                    network_shard.record_request(
                        operation,
                        response.status(),
                        request_started.elapsed(),
                    );
                    ExecutionResult::Response(QueuedResponse {
                        request_id,
                        response,
                        permit: response_permit,
                        request_permit: Some(request_permit),
                        unknown_on_write: may_mutate,
                        terminal: false,
                    })
                }
                Ok(None) => {
                    network_shard.abandoned_request();
                    ExecutionResult::Unknown
                }
                Err(_) if may_mutate => {
                    network_shard.abandoned_request();
                    ExecutionResult::Unknown
                }
                Err(_) => ExecutionResult::Response(QueuedResponse {
                    request_id,
                    response: operation_dispatch::timeout_response(
                        operation_id,
                        b"request execution timed out",
                    ),
                    permit: response_permit,
                    request_permit: Some(request_permit),
                    unknown_on_write: false,
                    terminal: false,
                }),
            }
        }
    }
}

struct ActiveStream<'a> {
    network_shard: NetworkShard<'a>,
}

impl Drop for ActiveStream<'_> {
    fn drop(&mut self) {
        self.network_shard.stream_finished();
    }
}

async fn write_response<S: SendStream>(
    send: &mut S,
    request_id: u64,
    response: impl Into<operation_transport::OperationResponse>,
    request_timeout: Duration,
) -> bool {
    let parts = match response.into().into_parts().with_request_id(request_id) {
        Ok(parts) => parts,
        Err(_) => return false,
    };
    send.write_response(parts, request_timeout).await.is_ok()
}

fn wire_protocol_error_response(error: openkache_protocol::ProtocolError) -> Response {
    let status = match error {
        openkache_protocol::ProtocolError::UnknownOpcode(_) => Status::UnsupportedOpcode,
        openkache_protocol::ProtocolError::ValueTooLarge { .. } => Status::TooLarge,
        _ => Status::InvalidRequest,
    };
    response_display(status, error)
}

fn response_display(status: Status, value: impl std::fmt::Display) -> Response {
    let mut payload = String::with_capacity(
        openkache_protocol::RESPONSE_FIXED_BYTES + openkache_protocol::MAX_VARUINT_BYTES + 64,
    );
    write!(payload, "{value}").expect("writing to a String cannot fail");
    response(status, payload.into_bytes())
}

/// Constructs a protocol response whose payload is known to fit protocol limits.
fn response(status: Status, payload: Vec<u8>) -> Response {
    Response::new(status, payload).expect("server responses stay within protocol limits")
}
