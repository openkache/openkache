use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use futures_util::{FutureExt, pin_mut, select};
use openkache_protocol::ResponseSegment;
use rustls::pki_types::CertificateDer;
use smallvec::SmallVec;

use super::{Connection, Incoming, NAME, Stream, StreamChunk, StreamKind, TransportError};
use crate::channel::{self, AsyncReceiver, Sender, TrySendError};
use crate::network_runtime;

const MAX_DATAGRAM_BYTES: usize = 65_535;
const REQUEST_CANCELLED_ERROR_CODE: u64 = 0;
const STREAM_CHUNK_BYTES: usize = 16 * 1024;
const STREAM_CHUNK_BACKLOG: usize = 1;

pub(super) type ConnectionId = Arc<[u8]>;

pub(super) enum Command {
    Close {
        error_code: u64,
        reason: Vec<u8>,
    },
    CloseConnection {
        connection_id: ConnectionId,
        error_code: u64,
        reason: Vec<u8>,
    },
    StopRequest {
        connection_id: ConnectionId,
        stream_id: u64,
    },
    ResumeRequest {
        connection_id: ConnectionId,
        stream_id: u64,
    },
    FinishResponse {
        connection_id: ConnectionId,
        stream_id: u64,
    },
    ResetResponse {
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
    chunks: Sender<StreamChunk>,
    pending: Option<StreamChunk>,
    finished: bool,
}

struct Client {
    connection: quiche::Connection,
    announced: bool,
    streams: Option<Sender<Stream>>,
    requests: HashMap<u64, RequestChunks>,
    responses: HashMap<u64, PendingResponse>,
}

pub(super) struct Driver {
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
    pub(super) fn new(
        socket: network_runtime::UdpSocket,
        local_address: SocketAddr,
        config: quiche::Config,
        incoming: Sender<Incoming>,
        command_sender: Sender<Command>,
        commands: AsyncReceiver<Command>,
    ) -> Self {
        Self {
            socket,
            local_address,
            config,
            incoming,
            command_sender,
            commands: Some(commands),
            routes: HashMap::new(),
            clients: HashMap::new(),
            output_buffer: vec![0_u8; MAX_DATAGRAM_BYTES],
        }
    }

    pub(super) async fn run(mut self) -> Result<(), TransportError> {
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
            Command::Close { error_code, reason } => {
                for client in self.clients.values_mut() {
                    let _ = client.connection.close(true, error_code, &reason);
                }
                false
            }
            Command::CloseConnection {
                connection_id,
                error_code,
                reason,
            } => {
                if let Some(client) = self.clients.get_mut(&connection_id) {
                    let _ = client.connection.close(true, error_code, &reason);
                }
                false
            }
            Command::StopRequest {
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
                    if client.requests.contains_key(&stream_id) {
                        receive_stream(client, stream_id);
                    }
                }
                false
            }
            Command::FinishResponse {
                connection_id,
                stream_id,
            } => {
                if let Some(client) = self.clients.get_mut(&connection_id) {
                    let _ = client.connection.stream_send(stream_id, &[], true);
                }
                false
            }
            Command::ResetResponse {
                connection_id,
                stream_id,
            } => {
                if let Some(client) = self.clients.get_mut(&connection_id) {
                    let _ =
                        client
                            .connection
                            .stream_shutdown(stream_id, quiche::Shutdown::Write, 0);
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
                    // Only client-initiated bidirectional lanes carry the
                    // application protocol. Reject every unidirectional
                    // stream with STOP_SENDING before consuming its body.
                    let _ = client.connection.stream_shutdown(
                        stream_id,
                        quiche::Shutdown::Read,
                        REQUEST_CANCELLED_ERROR_CODE,
                    );
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
                            .map_err(|error| TransportError::backend(NAME, "packet send", error))?;
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
            kind: StreamKind::ClientBidirectional,
            chunks: chunk_receiver,
        });
        RequestChunks {
            chunks,
            pending: None,
            finished: false,
        }
    });
    if let Some(chunk) = request.pending.take() {
        let pending_finished = matches!(chunk, StreamChunk::Finished);
        let pending_cancelled = matches!(chunk, StreamChunk::Cancelled);
        match request.chunks.try_send(chunk) {
            Ok(()) if request.finished && !pending_finished => {
                match request.chunks.try_send(StreamChunk::Finished) {
                    Ok(()) => {
                        client.requests.remove(&stream_id);
                        return;
                    }
                    Err(TrySendError::Full(chunk)) => request.pending = Some(chunk),
                    Err(TrySendError::Disconnected(_)) => {
                        client.requests.remove(&stream_id);
                        return;
                    }
                }
            }
            Ok(()) if pending_finished => {
                client.requests.remove(&stream_id);
                return;
            }
            Ok(()) if pending_cancelled => {
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
                buffer.truncate(read);
                if read > 0 {
                    if let Err(error) = request.chunks.try_send(StreamChunk::Bytes(buffer)) {
                        match error {
                            TrySendError::Full(chunk) => {
                                request.pending = Some(chunk);
                                request.finished = finished;
                                break;
                            }
                            TrySendError::Disconnected(_) => {
                                client.requests.remove(&stream_id);
                                break;
                            }
                        }
                    }
                }
                if finished {
                    request.finished = true;
                    match request.chunks.try_send(StreamChunk::Finished) {
                        Ok(()) => {}
                        Err(TrySendError::Full(chunk)) => {
                            request.pending = Some(chunk);
                        }
                        Err(TrySendError::Disconnected(_)) => {
                            client.requests.remove(&stream_id);
                            break;
                        }
                    }
                    if request.pending.is_none() {
                        client.requests.remove(&stream_id);
                        break;
                    }
                }
            }
            Err(quiche::Error::Done) => break,
            Err(_) => {
                match request.chunks.try_send(StreamChunk::Cancelled) {
                    Ok(()) => {
                        client.requests.remove(&stream_id);
                    }
                    Err(TrySendError::Full(chunk)) => request.pending = Some(chunk),
                    Err(TrySendError::Disconnected(_)) => {
                        client.requests.remove(&stream_id);
                    }
                }
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
