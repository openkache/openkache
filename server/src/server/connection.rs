use super::*;

#[derive(Clone)]
pub(super) struct NetworkWorkerLimits {
    pub(super) worker_id: usize,
    pub(super) request_timeout: Duration,
    pub(super) max_stream_lanes: usize,
    pub(super) request_budget: RequestBudget,
    pub(super) namespaces: Arc<Mutex<NamespaceRegistry>>,
    pub(super) observability: Arc<ObservabilityState>,
    pub(super) capabilities: Arc<dyn CapabilityCatalog>,
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
    loop {
        if streams.len() >= max_stream_lanes {
            let _ = streams.next().await;
            continue;
        }
        if streams.is_empty() {
            match connection.accept_bi().await {
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
        } else {
            let incoming = connection.accept_bi().fuse();
            let completed = streams.next().fuse();
            pin_mut!(incoming, completed);
            select! {
                incoming = incoming => match incoming {
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
                _ = completed => {}
            }
        }
    }
    while streams.next().await.is_some() {}
}

/// Reuses one QUIC stream as a sequential request lane until either peer closes it.
async fn serve_stream<S: SendStream, R: ReceiveStream>(
    mut send: S,
    mut receive: R,
    network_shard: NetworkShard<'_>,
    authorization: operation_authorization::AuthorizationContext,
    request_timeout: Duration,
    request_budget: RequestBudget,
    runtime: Arc<operation_execution_state::OperationRuntime>,
) {
    let _stream_guard = ActiveStream { network_shard };
    loop {
        let mut frame = match receive
            .read_request(
                crate::protocol::max_request_frame_bytes(),
                request_timeout,
                &request_budget,
                |header, prefix| {
                    operation_dispatch::admit_request_header(
                        header,
                        prefix,
                        runtime.as_ref(),
                    )
                },
            )
            .await
        {
            Ok(Ok(frame)) => frame,
            Ok(Err(rejection)) => {
                network_shard.record_request(
                    operation_contract::telemetry_operation(rejection.opcode()),
                    rejection.status(),
                    rejection.elapsed(),
                );
                if !write_response(&mut send, rejection.into_response(), request_timeout).await {
                    network_shard.response_write_failure();
                }
                break;
            }
            Err(StreamReadError::Timeout) => {
                network_shard.request_read_timeout();
                if !write_response(
                    &mut send,
                    response_bytes(Status::Timeout, b"request read timed out"),
                    request_timeout,
                )
                .await
                {
                    network_shard.response_write_failure();
                }
                break;
            }
            Err(StreamReadError::TooLarge) => {
                network_shard.protocol_error();
                if !write_response(
                    &mut send,
                    response_bytes(Status::TooLarge, b"request exceeds the protocol limit"),
                    request_timeout,
                )
                .await
                {
                    network_shard.response_write_failure();
                }
                break;
            }
            Err(StreamReadError::Protocol(error)) => {
                network_shard.protocol_error();
                if !write_response(
                    &mut send,
                    wire_protocol_error_response(error),
                    request_timeout,
                )
                .await
                {
                    network_shard.response_write_failure();
                }
                break;
            }
            Err(StreamReadError::Transport(_)) => break,
        };
        let request_bytes = std::mem::take(&mut frame.bytes);
        let mut terminal_after_response = frame.has_trailing_bytes;
        let response_result = match request_projection::project_owned_request(request_bytes) {
            Ok(input) => {
                let request_opcode = input.opcode();
                let operation: Operation = operation_contract::telemetry_operation(request_opcode);
                let request_started = std::time::Instant::now();
                let may_mutate = operation_dispatch::may_mutate(runtime.as_ref(), request_opcode);
                let response_permit = if let Some(response_budget_bytes) =
                    operation_dispatch::response_budget_bytes(runtime.as_ref(), request_opcode)
                {
                    match request_budget
                        .acquire(response_budget_bytes, request_timeout)
                        .await
                    {
                        Ok(permit) => Some(permit),
                        Err(StreamReadError::Timeout) => {
                            let response = operation_dispatch::timeout_response(
                                request_opcode,
                                b"response memory budget timed out",
                            );
                            network_shard.record_request(
                                operation,
                                Status::Timeout,
                                request_started.elapsed(),
                            );
                            if !write_response(&mut send, response, request_timeout).await {
                                network_shard.response_write_failure();
                                break;
                            }
                            continue;
                        }
                        Err(_) => {
                            let response = operation_transport::contract_error_response(
                                request_opcode,
                                Status::Overloaded,
                                b"response exceeds the server memory budget",
                            );
                            network_shard.record_request(
                                operation,
                                Status::Overloaded,
                                request_started.elapsed(),
                            );
                            if !write_response(&mut send, response, request_timeout).await {
                                network_shard.response_write_failure();
                                break;
                            }
                            continue;
                        }
                    }
                } else {
                    None
                };
                match network_runtime::timeout(
                    request_timeout,
                    operation_dispatch::execute_request(
                        input,
                        &authorization,
                        runtime.as_ref(),
                    ),
                )
                .await
                {
                    Ok(Some(response)) => {
                        network_shard.record_request(
                            operation,
                            response.status(),
                            request_started.elapsed(),
                        );
                        (response, response_permit)
                    }
                    Ok(None) => {
                        // A mutating storage failure may have crossed its
                        // linearization point. Do not send an error response
                        // that would falsely guarantee that no mutation took
                        // effect.
                        network_shard.abandoned_request();
                        return;
                    }
                    Err(_) if may_mutate => {
                        // The worker request may already have crossed its mutation
                        // linearization point when this wait expires. An error response
                        // would falsely guarantee that it did not take effect.
                        network_shard.abandoned_request();
                        return;
                    }
                    Err(_) => {
                        network_shard.record_request(
                            operation,
                            Status::Timeout,
                            request_started.elapsed(),
                        );
                        (
                            operation_dispatch::timeout_response(
                                request_opcode,
                                b"request execution timed out",
                            ),
                            response_permit,
                        )
                    }
                }
            }
            Err(error) => {
                network_shard.protocol_error();
                terminal_after_response = true;
                (wire_protocol_error_response(error).into(), None)
            }
        };
        if !write_response(&mut send, response_result.0, request_timeout).await {
            network_shard.response_write_failure();
            break;
        }
        if terminal_after_response {
            break;
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
    response: impl Into<operation_transport::OperationResponse>,
    request_timeout: Duration,
) -> bool {
    send.write_response(response.into().into_parts(), request_timeout)
        .await
        .is_ok()
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

fn response_bytes(status: Status, payload: &[u8]) -> Response {
    let mut owned = Vec::with_capacity(
        openkache_protocol::RESPONSE_FIXED_BYTES
            + openkache_protocol::MAX_VARUINT_BYTES
            + payload.len(),
    );
    owned.extend_from_slice(payload);
    response(status, owned)
}
