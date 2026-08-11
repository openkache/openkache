//! Quiche connection driver and demand-driven stream progression.

use std::collections::HashMap;

use futures_util::{FutureExt, pin_mut, select};
use smallvec::SmallVec;

use super::*;
use crate::channel::TrySendError;

struct PendingResponse {
    segments: SmallVec<[ResponseSegment; 8]>,
    segment_index: usize,
    segment_written: usize,
    reply: Sender<Result<(), String>>,
}

struct RequestState {
    chunks: Sender<RequestChunk>,
    pending: Option<RequestChunk>,
    read_capacity: usize,
    finished: bool,
}

struct Client {
    connection: quiche::Connection,
    announced: bool,
    streams: Option<Sender<Stream>>,
    requests: HashMap<u64, RequestState>,
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
            Command::ReadRequest {
                connection_id,
                stream_id,
                capacity,
            } => {
                if let Some(client) = self.clients.get_mut(&connection_id) {
                    if let Some(request) = client.requests.get_mut(&stream_id) {
                        request.read_capacity = capacity.max(1);
                    }
                    receive_stream(client, stream_id);
                }
                false
            }
            Command::ProbeRequest {
                connection_id,
                stream_id,
                reply,
            } => {
                let result = self
                    .clients
                    .get_mut(&connection_id)
                    .map(|client| probe_stream(client, stream_id))
                    .unwrap_or(Ok(false));
                let _ = reply.try_send(result);
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
    let is_new = !client.requests.contains_key(&stream_id);
    let request = client.requests.entry(stream_id).or_insert_with(|| {
        let (chunks, chunk_receiver) = channel::bounded_sync_async(STREAM_CHUNK_BACKLOG);
        let _ = stream_sender.try_send(Stream {
            stream_id,
            chunks: chunk_receiver,
        });
        RequestState {
            chunks,
            pending: None,
            read_capacity: 0,
            finished: false,
        }
    });
    if is_new {
        return;
    }
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
    if request.read_capacity == 0 || request.chunks.is_full() {
        return;
    }
    let capacity = std::mem::take(&mut request.read_capacity);
    let mut buffer = vec![0_u8; capacity];
    match client
        .connection
        .stream_recv(stream_id, buffer.as_mut_slice())
    {
        Ok((read, finished)) => {
            request.finished = finished;
            buffer.truncate(read);
            let chunk = RequestChunk {
                bytes: buffer,
                finished,
            };
            let disconnected = match request.chunks.try_send(chunk) {
                Ok(()) => false,
                Err(TrySendError::Full(chunk)) => {
                    request.pending = Some(chunk);
                    false
                }
                Err(TrySendError::Disconnected(_)) => true,
            };
            if disconnected {
                client.requests.remove(&stream_id);
                return;
            }
            if finished && request.pending.is_none() {
                client.requests.remove(&stream_id);
            }
        }
        Err(quiche::Error::Done) => {
            request.read_capacity = capacity;
        }
        Err(_) => {
            client.requests.remove(&stream_id);
        }
    }
}

fn probe_stream(client: &mut Client, stream_id: u64) -> Result<bool, String> {
    let mut byte = [0_u8; 1];
    match client.connection.stream_recv(stream_id, &mut byte) {
        Ok((read, finished)) => {
            if let Some(request) = client.requests.get_mut(&stream_id) {
                request.finished = finished;
            }
            Ok(read != 0)
        }
        Err(quiche::Error::Done) => Ok(false),
        Err(error) => Err(error.to_string()),
    }
}

fn progress_response(client: &mut Client, stream_id: u64) {
    let Some(mut response) = client.responses.remove(&stream_id) else {
        return;
    };
    loop {
        while response.segment_index < response.segments.len()
            && response.segment_written == response.segments[response.segment_index].len()
        {
            response.segment_index += 1;
            response.segment_written = 0;
        }
        let Some(segment) = response.segments.get(response.segment_index) else {
            let _ = response.reply.try_send(Ok(()));
            return;
        };
        let remaining = &segment.as_slice()[response.segment_written..];
        match client.connection.stream_send(stream_id, remaining, false) {
            Ok(0) | Err(quiche::Error::Done) => {
                client.responses.insert(stream_id, response);
                return;
            }
            Ok(written) => {
                response.segment_written += written;
            }
            Err(error) => {
                let _ = response.reply.try_send(Err(error.to_string()));
                return;
            }
        }
    }
}
