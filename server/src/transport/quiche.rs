//! Quiche backend adapter.

use futures_util::FutureExt;
use smallvec::SmallVec;

use super::*;
use crate::channel::{self, AsyncReceiver, Sender};

#[path = "quiche/driver.rs"]
mod driver;
#[path = "quiche/tls.rs"]
mod tls;

use driver::Driver;
use tls::config;

const NAME: &str = "quiche";
const MAX_DATAGRAM_BYTES: usize = 65_535;
const MAX_BUFFERED_REQUEST_BYTES: usize = crate::protocol::max_request_frame_bytes() + 1;
const REQUEST_CANCELLED_ERROR_CODE: u64 = 0;
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
                Driver::new(
                    socket,
                    local_address,
                    config,
                    incoming_sender,
                    driver_commands,
                    command_receiver,
                )
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
                finished: false,
            },
        ))
    }
}

struct Stream {
    stream_id: u64,
    chunks: AsyncReceiver<RequestChunk>,
}

struct RequestChunk {
    bytes: Vec<u8>,
    finished: bool,
}

pub(crate) struct ReceiveStream {
    connection_id: ConnectionId,
    stream_id: u64,
    commands: Sender<Command>,
    chunks: AsyncReceiver<RequestChunk>,
    finished: bool,
}

impl ReceiveStream {
    async fn next_chunk(
        &mut self,
        capacity: usize,
        operation: &'static str,
    ) -> Result<Vec<u8>, TransportError> {
        self.commands
            .send_async_network(Command::ReadRequest {
                connection_id: self.connection_id.clone(),
                stream_id: self.stream_id,
                capacity,
            })
            .await
            .map_err(|error| TransportError::backend(NAME, operation, error))?;
        let chunk = self
            .chunks
            .recv_async_network()
            .await
            .map_err(|error| TransportError::backend(NAME, operation, error))?;
        self.finished = chunk.finished;
        Ok(chunk.bytes)
    }

    async fn probe_readable_byte(&self) -> Result<bool, TransportError> {
        let (reply, response) = channel::bounded_sync_async(1);
        self.commands
            .send_async_network(Command::ProbeRequest {
                connection_id: self.connection_id.clone(),
                stream_id: self.stream_id,
                reply,
            })
            .await
            .map_err(|error| TransportError::backend(NAME, "stream trailing probe", error))?;
        response
            .recv_async_network()
            .await
            .map_err(|error| TransportError::backend(NAME, "stream trailing probe", error))?
            .map_err(|message| TransportError::backend(NAME, "stream trailing probe", message))
    }
}

impl super::RequestByteStream for ReceiveStream {
    async fn read_chunk(
        &mut self,
        capacity: usize,
        _backend: &'static str,
    ) -> Result<Option<OwnedRange>, TransportError> {
        self.next_chunk(capacity.max(1), "stream read")
            .await
            .map(|chunk| (!chunk.is_empty()).then(|| OwnedRange::whole(chunk)))
    }

    async fn has_readable_byte(
        &mut self,
        _backend: &'static str,
    ) -> Result<bool, TransportError> {
        if self.finished {
            Ok(false)
        } else {
            self.probe_readable_byte().await
        }
    }

    async fn try_has_readable_byte(
        &mut self,
        backend: &'static str,
    ) -> Result<bool, TransportError> {
        self.has_readable_byte(backend).await
    }
}

impl super::ReceiveStream for ReceiveStream {
    async fn read_request(
        &mut self,
        maximum: usize,
        maximum_value: usize,
        timeout: Duration,
        budget: &RequestBudget,
        frame_layout_provider: &dyn crate::protocol::FrameLayoutProvider,
    ) -> Result<RequestFrame, StreamReadError> {
        read_buffered_request(
            self,
            NAME,
            maximum,
            maximum_value,
            timeout,
            budget,
            frame_layout_provider,
        )
        .await
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
    ReadRequest {
        connection_id: ConnectionId,
        stream_id: u64,
        capacity: usize,
    },
    ProbeRequest {
        connection_id: ConnectionId,
        stream_id: u64,
        reply: Sender<Result<bool, String>>,
    },
    SendResponse {
        connection_id: ConnectionId,
        stream_id: u64,
        segments: SmallVec<[ResponseSegment; 8]>,
        reply: Sender<Result<(), String>>,
    },
    Shutdown,
}
