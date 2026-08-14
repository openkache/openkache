use std::collections::HashMap;

use boring::pkey::PKey;
use boring::ssl::{SslContextBuilder, SslMethod, SslVerifyMode};
use boring::x509::X509;
use boring::x509::store::X509StoreBuilder;
use futures_util::{FutureExt, pin_mut, select};
use smallvec::SmallVec;

use super::*;
use crate::channel::{self, AsyncReceiver, Sender, TrySendError};

const NAME: &str = "quiche";
const MAX_DATAGRAM_BYTES: usize = 65_535;
const MAX_BUFFERED_REQUEST_BYTES: usize = crate::protocol::max_request_frame_bytes() + 1;
const REQUEST_CANCELLED_ERROR_CODE: u64 = 0;
const STREAM_CHUNK_BYTES: usize = 16 * 1024;
const STREAM_CHUNK_BACKLOG: usize = 1;

type ConnectionId = Arc<[u8]>;

pub(crate) struct Endpoint {
    incoming: AsyncReceiver<Incoming>,
    commands: Sender<Command>,
    driver: AsyncReceiver<Result<(), TransportError>>,
}

impl Endpoint {
    pub(super) async fn bind(
        socket: std::net::UdpSocket,
        material: Arc<ServerTlsConfig>,
        max_concurrent_streams: usize,
    ) -> Result<Self, TransportError> {
        let socket = network_runtime::UdpSocket::from_std(socket)
            .map_err(|error| TransportError::backend(NAME, "socket initialization", error))?;
        let local_address = socket
            .local_addr()
            .map_err(|error| TransportError::backend(NAME, "local address", error))?;
        let config = config(&material, max_concurrent_streams)?;
        let (incoming_sender, incoming) = channel::unbounded_async();
        let (commands, command_receiver) = channel::unbounded_async();
        let driver_commands = commands.clone();
        let (driver_done_sender, driver_done) =
            channel::bounded_sync_async::<Result<(), TransportError>>(1);
        network_runtime::spawn_detached(async move {
            let result = std::panic::AssertUnwindSafe(
                Driver {
                    socket,
                    local_address,
                    config,
                    incoming: incoming_sender,
                    command_sender: driver_commands,
                    commands: Some(command_receiver),
                    routes: HashMap::new(),
                    clients: HashMap::new(),
                    output_buffer: vec![0_u8; MAX_DATAGRAM_BYTES],
                }
                .run(),
            )
            .catch_unwind()
            .await
            .map_err(|panic| {
                TransportError::backend(NAME, "driver task", format!("panic: {panic:?}"))
            })
            .and_then(|result| result);
            let _ = driver_done_sender.try_send(result);
        });
        Ok(Self {
            incoming,
            commands,
            driver: driver_done,
        })
    }
}

impl super::Endpoint for Endpoint {
    type Incoming = Incoming;

    async fn wait_incoming(&self) -> Option<Self::Incoming> {
        self.incoming.recv_async_network().await.ok()
    }

    fn close(&self, reason: &[u8]) {
        let _ = self.commands.try_send(Command::Close(reason.to_vec()));
    }

    async fn shutdown(self) -> Result<(), TransportError> {
        let _ = self.commands.send_async_network(Command::Shutdown).await;
        network_runtime::timeout(Duration::from_secs(5), self.driver.recv_async_network())
            .await
            .map_err(|_| TransportError::backend(NAME, "driver task", "shutdown timed out"))?
            .map_err(|error| TransportError::backend(NAME, "driver task", error))
            .and_then(|result| result)
    }
}

pub(crate) struct Incoming(Connection);

impl super::Incoming for Incoming {
    type Connection = Connection;

    async fn connect(self) -> Result<Self::Connection, TransportError> {
        Ok(self.0)
    }
}

pub(crate) struct Connection {
    connection_id: ConnectionId,
    peer_certificate: Option<CertificateDer<'static>>,
    streams: AsyncReceiver<Stream>,
    commands: Sender<Command>,
}

impl super::Connection for Connection {
    type SendStream = SendStream;
    type ReceiveStream = ReceiveStream;

    fn take_peer_certificate(&mut self) -> Option<CertificateDer<'static>> {
        self.peer_certificate.take()
    }

    async fn accept_bi(
        &self,
    ) -> Result<(Self::SendStream, Self::ReceiveStream), TransportError> {
        let stream = self
            .streams
            .recv_async_network()
            .await
            .map_err(|error| TransportError::backend(NAME, "stream accept", error))?;
        Ok((
            SendStream {
                connection_id: self.connection_id.clone(),
                stream_id: stream.stream_id,
                commands: self.commands.clone(),
            },
            ReceiveStream {
                connection_id: self.connection_id.clone(),
                stream_id: stream.stream_id,
                commands: self.commands.clone(),
                chunks: stream.chunks,
                buffered: Vec::new(),
            },
        ))
    }
}

struct Stream {
    stream_id: u64,
    chunks: AsyncReceiver<Vec<u8>>,
}

pub(crate) struct ReceiveStream {
    connection_id: ConnectionId,
    stream_id: u64,
    commands: Sender<Command>,
    chunks: AsyncReceiver<Vec<u8>>,
    buffered: Vec<u8>,
}

impl ReceiveStream {
    async fn next_chunk(&self, operation: &'static str) -> Result<Vec<u8>, TransportError> {
        let chunk = self
            .chunks
            .recv_async_network()
            .await
            .map_err(|error| TransportError::backend(NAME, operation, error))?;
        let _ = self.commands.try_send(Command::ResumeRequest {
            connection_id: self.connection_id.clone(),
            stream_id: self.stream_id,
        });
        Ok(chunk)
    }
}

impl super::ReceiveStream for ReceiveStream {
    async fn read_request(
        &mut self,
        maximum: usize,
        maximum_value: usize,
        timeout: Duration,
        budget: &RequestBudget,
    ) -> Result<RequestFrame, StreamReadError> {
        network_runtime::timeout(timeout, async {
            while self.buffered.is_empty() {
                self.buffered = self.next_chunk("stream header read").await?;
            }
            Ok::<(), TransportError>(())
        })
        .await
        .map_err(|_| StreamReadError::Timeout)?
        .map_err(StreamReadError::Transport)?;
        let header = network_runtime::timeout(timeout, async {
            loop {
                if let Some(header) = ProtocolRequestFrame::decode_header(&self.buffered)? {
                    break Ok::<_, StreamReadError>(header);
                }
                if self.buffered.len() >= maximum {
                    return Err(StreamReadError::TooLarge);
                }
                let chunk = self.next_chunk("stream header read").await?;
                self.buffered.extend_from_slice(&chunk);
            }
        })
        .await
        .map_err(|_| StreamReadError::Timeout)??;
        if header.value_len() > maximum_value {
            return Err(StreamReadError::TooLarge);
        }
        let frame_len = network_runtime::timeout(timeout, async {
            let frame_len = header.frame_len()?;
            Ok::<_, StreamReadError>(frame_len)
        })
        .await
        .map_err(|_| StreamReadError::Timeout)??;
        if frame_len > maximum {
            return Err(StreamReadError::TooLarge);
        }
        let permit = budget.acquire(header.value_len(), timeout).await?;
        network_runtime::timeout(timeout, async {
            while self.buffered.len() < frame_len {
                let chunk = self.next_chunk("stream body read").await?;
                self.buffered.extend_from_slice(&chunk);
            }
            Ok::<(), TransportError>(())
        })
        .await
        .map_err(|_| StreamReadError::Timeout)?
        .map_err(StreamReadError::Transport)?;
        let has_trailing_bytes = self.buffered.len() > frame_len;
        let frame = if !has_trailing_bytes {
            std::mem::take(&mut self.buffered)
        } else {
            self.buffered.drain(..frame_len).collect()
        };
        Ok(RequestFrame::with_trailing_bytes(
            frame,
            permit,
            has_trailing_bytes,
        ))
    }
}

impl Drop for ReceiveStream {
    fn drop(&mut self) {
        let _ = self.commands.try_send(Command::CancelRequest {
            connection_id: self.connection_id.clone(),
            stream_id: self.stream_id,
        });
    }
}

pub(crate) struct SendStream {
    connection_id: ConnectionId,
    stream_id: u64,
    commands: Sender<Command>,
}

impl super::SendStream for SendStream {
    async fn write_response(
        &mut self,
        parts: ResponseParts,
        timeout: Duration,
    ) -> Result<(), TransportError> {
        let (reply, response) = channel::bounded_sync_async(1);
        self.commands
            .send_async_network(Command::SendResponse {
                connection_id: self.connection_id.clone(),
                stream_id: self.stream_id,
                segments: parts.into_segments(),
                reply,
            })
            .await
            .map_err(|error| TransportError::backend(NAME, "stream write", error))?;
        let result = network_runtime::timeout(timeout, response.recv_async_network())
            .await
            .map_err(|_| TransportError::backend(NAME, "stream write timeout", "timed out"))?
            .map_err(|error| TransportError::backend(NAME, "stream write", error))?;
        result.map_err(|message| TransportError::backend(NAME, "stream write", message))
    }
}

enum Command {
    Close(Vec<u8>),
    CancelRequest {
        connection_id: ConnectionId,
        stream_id: u64,
    },
    ResumeRequest {
        connection_id: ConnectionId,
        stream_id: u64,
    },
    SendResponse {
        connection_id: ConnectionId,
        stream_id: u64,
        segments: SmallVec<[ResponseSegment; 8]>,
        reply: Sender<Result<(), String>>,
    },
    Shutdown,
}

struct PendingResponse {
    segments: SmallVec<[ResponseSegment; 8]>,
    segment_index: usize,
    segment_written: usize,
    reply: Sender<Result<(), String>>,
}

struct RequestChunks {
    chunks: Sender<Vec<u8>>,
    pending: Option<Vec<u8>>,
    finished: bool,
}

struct Client {
    connection: quiche::Connection,
    announced: bool,
    streams: Option<Sender<Stream>>,
    requests: HashMap<u64, RequestChunks>,
    responses: HashMap<u64, PendingResponse>,
}

struct Driver {
    socket: network_runtime::UdpSocket,
    local_address: SocketAddr,
    config: quiche::Config,
    incoming: Sender<Incoming>,
    command_sender: Sender<Command>,
    commands: Option<AsyncReceiver<Command>>,
    routes: HashMap<Vec<u8>, ConnectionId>,
    clients: HashMap<ConnectionId, Client>,
    output_buffer: Vec<u8>,
}

impl Driver {
    async fn run(mut self) -> Result<(), TransportError> {
        let mut packets = self.socket.receiver();
        let datagram_capability = packets.capability();
        let commands = self
            .commands
            .take()
            .expect("QUIC driver command receiver is present");
        loop {
            let packet_batch = packets.recv_batch().fuse();
            let command = commands.recv_async_network().fuse();
            let timeout = network_runtime::sleep(
                self.next_timeout()
                    .unwrap_or_else(|| Duration::from_secs(86_400)),
            )
            .fuse();
            pin_mut!(packet_batch, command, timeout);

            let mut shutting_down = false;
            select! {
                packet_batch = packet_batch => {
                    let packet_batch = packet_batch
                        .map_err(|error| TransportError::backend(NAME, "receive", error))?;
                    if datagram_capability
                        == network_runtime::DatagramCapability::Single
                    {
                        debug_assert!(
                            packet_batch.len() <= 1,
                            "single-datagram runtime returned a batch"
                        );
                    }
                    for packet in packet_batch {
                        self.receive_packet(packet)?;
                    }
                },
                command = command => {
                    let Ok(command) = command else {
                        break;
                    };
                    shutting_down = self.handle_command(command);
                }
                () = timeout => {
                    for client in self.clients.values_mut() {
                        client.connection.on_timeout();
                    }
                }
            }

            self.process_connections();
            self.flush_packets().await?;
            self.collect_closed();
            if shutting_down {
                break;
            }
        }
        Ok(())
    }

    fn next_timeout(&self) -> Option<Duration> {
        self.clients
            .values()
            .filter_map(|client| client.connection.timeout())
            .min()
    }

    fn receive_packet(
        &mut self,
        mut packet: network_runtime::Datagram,
    ) -> Result<(), TransportError> {
        let peer_address = packet.address();
        let datagram = packet.payload_mut();
        let header = match quiche::Header::from_slice(datagram, quiche::MAX_CONN_ID_LEN) {
            Ok(header) => header,
            Err(_) => return Ok(()),
        };
        let connection_id = match self.routes.get(header.dcid.as_ref()) {
            Some(connection_id) => connection_id.clone(),
            None => {
                if header.ty != quiche::Type::Initial
                    || !quiche::version_is_supported(header.version)
                {
                    return Ok(());
                }
                self.accept_connection(header.dcid.as_ref().to_vec(), peer_address)?
            }
        };
        let Some(client) = self.clients.get_mut(&connection_id) else {
            return Ok(());
        };
        let receive_info = quiche::RecvInfo {
            from: peer_address,
            to: self.local_address,
        };
        if client.connection.recv(datagram, receive_info).is_err() {
            return Ok(());
        }
        Ok(())
    }

    fn accept_connection(
        &mut self,
        original_destination_id: Vec<u8>,
        peer_address: SocketAddr,
    ) -> Result<ConnectionId, TransportError> {
        let connection_id = ConnectionId::from(rand::random::<[u8; quiche::MAX_CONN_ID_LEN]>());
        let source_id = quiche::ConnectionId::from_ref(&connection_id);
        let connection = quiche::accept(
            &source_id,
            None,
            self.local_address,
            peer_address,
            &mut self.config,
        )
        .map_err(|error| TransportError::backend(NAME, "connection accept", error))?;
        self.routes
            .insert(original_destination_id, connection_id.clone());
        self.routes
            .insert(connection_id.as_ref().to_vec(), connection_id.clone());
        self.clients.insert(
            connection_id.clone(),
            Client {
                connection,
                announced: false,
                streams: None,
                requests: HashMap::new(),
                responses: HashMap::new(),
            },
        );
        Ok(connection_id)
    }

    fn handle_command(&mut self, command: Command) -> bool {
        match command {
            Command::Close(reason) => {
                for client in self.clients.values_mut() {
                    let _ = client.connection.close(true, 0, &reason);
                }
                false
            }
            Command::CancelRequest {
                connection_id,
                stream_id,
            } => {
                if let Some(client) = self.clients.get_mut(&connection_id) {
                    client.requests.remove(&stream_id);
                    let _ = client.connection.stream_shutdown(
                        stream_id,
                        quiche::Shutdown::Read,
                        REQUEST_CANCELLED_ERROR_CODE,
                    );
                }
                false
            }
            Command::ResumeRequest {
                connection_id,
                stream_id,
            } => {
                if let Some(client) = self.clients.get_mut(&connection_id) {
                    receive_stream(client, stream_id);
                }
                false
            }
            Command::SendResponse {
                connection_id,
                stream_id,
                segments,
                reply,
            } => {
                let Some(client) = self.clients.get_mut(&connection_id) else {
                    let _ = reply.try_send(Err("connection is closed".to_string()));
                    return false;
                };
                client.responses.insert(
                    stream_id,
                    PendingResponse {
                        segments,
                        segment_index: 0,
                        segment_written: 0,
                        reply,
                    },
                );
                progress_response(client, stream_id);
                false
            }
            Command::Shutdown => {
                for client in self.clients.values_mut() {
                    let _ = client.connection.close(true, 0, b"server shutting down");
                }
                true
            }
        }
    }

    fn process_connections(&mut self) {
        let incoming = &self.incoming;
        let command_sender = &self.command_sender;
        for (connection_id, client) in &mut self.clients {
            if !client.announced && client.connection.is_established() {
                let (streams, stream_receiver) = channel::unbounded_async();
                client.streams = Some(streams);
                client.announced = true;
                let _ = incoming.try_send(Incoming(Connection {
                    connection_id: connection_id.clone(),
                    peer_certificate: client
                        .connection
                        .peer_cert()
                        .map(|certificate| CertificateDer::from(certificate.to_vec())),
                    streams: stream_receiver,
                    commands: command_sender.clone(),
                }));
            }

            let writable: SmallVec<[_; 8]> = client.connection.writable().collect();
            for stream_id in writable {
                progress_response(client, stream_id);
            }

            let readable: SmallVec<[_; 8]> = client.connection.readable().collect();
            for stream_id in readable {
                if stream_id & 0x3 != 0 {
                    continue;
                }
                receive_stream(client, stream_id);
            }
        }
    }

    async fn flush_packets(&mut self) -> Result<(), TransportError> {
        let mut output = std::mem::take(&mut self.output_buffer);
        for client in self.clients.values_mut() {
            loop {
                match client.connection.send(&mut output) {
                    Ok((written, information)) => {
                        let (written_count, returned) = self
                            .socket
                            .send_to(output, written, information.to)
                            .await
                            .map_err(|error| {
                                TransportError::backend(NAME, "packet send", error)
                            })?;
                        output = returned;
                        if output.len() < MAX_DATAGRAM_BYTES {
                            output.resize(MAX_DATAGRAM_BYTES, 0);
                        }
                        if written_count != written {
                            return Err(TransportError::backend(
                                NAME,
                                "packet send",
                                format!(
                                    "UDP send was partial: wrote {written_count} of {written} bytes"
                                ),
                            ));
                        }
                    }
                    Err(quiche::Error::Done) => break,
                    Err(_) => {
                        let _ = client
                            .connection
                            .close(false, 1, b"packet generation failed");
                        break;
                    }
                }
            }
        }
        self.output_buffer = output;
        Ok(())
    }

    fn collect_closed(&mut self) {
        self.clients
            .retain(|_, client| !client.connection.is_closed());
        self.routes
            .retain(|_, connection_id| self.clients.contains_key(connection_id.as_ref()));
    }
}

fn receive_stream(client: &mut Client, stream_id: u64) {
    let Some(stream_sender) = client.streams.clone() else {
        return;
    };
    let request = client.requests.entry(stream_id).or_insert_with(|| {
        let (chunks, chunk_receiver) = channel::bounded_sync_async(STREAM_CHUNK_BACKLOG);
        let _ = stream_sender.try_send(Stream {
            stream_id,
            chunks: chunk_receiver,
        });
        RequestChunks {
            chunks,
            pending: None,
            finished: false,
        }
    });
    if let Some(chunk) = request.pending.take() {
        match request.chunks.try_send(chunk) {
            Ok(()) if request.finished => {
                client.requests.remove(&stream_id);
                return;
            }
            Ok(()) => {}
            Err(TrySendError::Full(chunk)) => {
                request.pending = Some(chunk);
                return;
            }
            Err(TrySendError::Disconnected(_)) => {
                client.requests.remove(&stream_id);
                return;
            }
        }
    }
    loop {
        if request.chunks.is_full() {
            break;
        }
        let mut buffer = vec![0_u8; STREAM_CHUNK_BYTES];
        match client
            .connection
            .stream_recv(stream_id, buffer.as_mut_slice())
        {
            Ok((read, finished)) => {
                request.finished = finished;
                buffer.truncate(read);
                match request.chunks.try_send(buffer) {
                    Ok(()) => {}
                    Err(TrySendError::Full(chunk)) => {
                        request.pending = Some(chunk);
                        break;
                    }
                    Err(TrySendError::Disconnected(_)) => {
                        client.requests.remove(&stream_id);
                        break;
                    }
                }
                if finished {
                    client.requests.remove(&stream_id);
                    break;
                }
            }
            Err(quiche::Error::Done) => break,
            Err(_) => {
                client.requests.remove(&stream_id);
                break;
            }
        }
    }
}

fn progress_response(client: &mut Client, stream_id: u64) {
    let Some(mut response) = client.responses.remove(&stream_id) else {
        return;
    };
    while response.segment_index < response.segments.len()
        && response.segment_written == response.segments[response.segment_index].len()
    {
        response.segment_index += 1;
        response.segment_written = 0;
    }
    let remaining = if let Some(segment) = response.segments.get(response.segment_index) {
        &segment.as_slice()[response.segment_written..]
    } else {
        let _ = response.reply.try_send(Ok(()));
        return;
    };
    match client.connection.stream_send(stream_id, remaining, false) {
        Ok(written) => {
            response.segment_written += written;
            while response.segment_index < response.segments.len()
                && response.segment_written == response.segments[response.segment_index].len()
            {
                response.segment_index += 1;
                response.segment_written = 0;
            }
            if response.segment_index == response.segments.len() {
                let _ = response.reply.try_send(Ok(()));
            } else {
                client.responses.insert(stream_id, response);
            }
        }
        Err(quiche::Error::Done) => {
            client.responses.insert(stream_id, response);
        }
        Err(error) => {
            let _ = response.reply.try_send(Err(error.to_string()));
        }
    }
}

fn config(
    material: &ServerTlsConfig,
    max_concurrent_streams: usize,
) -> Result<quiche::Config, TransportError> {
    let certificate = X509::from_der(
        material
            .certificate_chain
            .first()
            .expect("validated TLS certificate chain"),
    )
    .map_err(|error| TransportError::backend(NAME, "certificate parsing", error))?;
    let private_key = PKey::private_key_from_der(material.private_key.secret_der())
        .map_err(|error| TransportError::backend(NAME, "private key parsing", error))?;
    let mut tls = SslContextBuilder::new(SslMethod::tls())
        .map_err(|error| TransportError::backend(NAME, "TLS configuration", error))?;
    tls.set_certificate(&certificate)
        .map_err(|error| TransportError::backend(NAME, "TLS certificate", error))?;
    for certificate in material.certificate_chain.iter().skip(1) {
        let certificate = X509::from_der(certificate)
            .map_err(|error| TransportError::backend(NAME, "certificate parsing", error))?;
        tls.add_extra_chain_cert(certificate)
            .map_err(|error| TransportError::backend(NAME, "TLS certificate chain", error))?;
    }
    tls.set_private_key(&private_key)
        .map_err(|error| TransportError::backend(NAME, "TLS private key", error))?;
    if !material.client_ca.is_empty() {
        let mut roots = X509StoreBuilder::new()
            .map_err(|error| TransportError::backend(NAME, "client CA store", error))?;
        for certificate in &material.client_ca {
            let certificate = X509::from_der(certificate).map_err(|error| {
                TransportError::backend(NAME, "client CA certificate parsing", error)
            })?;
            tls.add_client_ca(&certificate)
                .map_err(|error| TransportError::backend(NAME, "client CA names", error))?;
            roots
                .add_cert(certificate)
                .map_err(|error| TransportError::backend(NAME, "client CA store", error))?;
        }
        tls.set_verify_cert_store(roots.build())
            .map_err(|error| TransportError::backend(NAME, "client CA store", error))?;
        tls.set_verify(SslVerifyMode::PEER | SslVerifyMode::FAIL_IF_NO_PEER_CERT);
        tls.set_session_id_context(b"openkache-mtls")
            .map_err(|error| TransportError::backend(NAME, "TLS session identity", error))?;
    }
    let mut config = quiche::Config::with_boring_ssl_ctx_builder(quiche::PROTOCOL_VERSION, tls)
        .map_err(|error| TransportError::backend(NAME, "configuration", error))?;
    config
        .set_application_protos(&[openkache_protocol::ALPN])
        .map_err(|error| TransportError::backend(NAME, "ALPN", error))?;
    config.set_max_idle_timeout(30_000);
    config.set_max_recv_udp_payload_size(MAX_DATAGRAM_BYTES);
    config.set_max_send_udp_payload_size(1_350);
    config.set_initial_max_data(64 * 1024 * 1024);
    config.set_initial_max_stream_data_bidi_remote(MAX_BUFFERED_REQUEST_BYTES as u64);
    config.set_initial_max_stream_data_bidi_local(64 * 1024);
    config.set_initial_max_streams_bidi(max_concurrent_streams as u64);
    config.set_initial_max_streams_uni(0);
    config.set_disable_active_migration(true);
    Ok(config)
}
