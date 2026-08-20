use futures_util::FutureExt;

use super::*;
use crate::channel::{self, AsyncReceiver, Sender};

const NAME: &str = "quiche";

use configuration::config;
use driver::{Command, ConnectionId, Driver, StreamChunk};

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
        let _ = self.commands.try_send(Command::Close {
            error_code: 0,
            reason: reason.to_vec(),
        });
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

    async fn accept_bi(&self) -> Result<(Self::SendStream, Self::ReceiveStream), TransportError> {
        loop {
            let stream = self
                .streams
                .recv_async_network()
                .await
                .map_err(|error| TransportError::backend(NAME, "stream accept", error))?;
            if stream.kind != StreamKind::ClientBidirectional {
                let _ = self.commands.try_send(Command::StopRequest {
                    connection_id: self.connection_id.clone(),
                    stream_id: stream.stream_id,
                });
                continue;
            }
            return Ok((
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
                    finished: false,
                    cancelled: false,
                },
            ));
        }
    }

    async fn accept_uni(&self) -> Result<Self::ReceiveStream, TransportError> {
        Err(TransportError::backend(
            NAME,
            "unidirectional stream accept",
            "quiche rejects invalid unidirectional streams in its driver",
        ))
    }

    fn close(&self, error_code: u64, reason: &[u8]) {
        let _ = self.commands.try_send(Command::CloseConnection {
            connection_id: self.connection_id.clone(),
            error_code,
            reason: reason.to_vec(),
        });
    }
}

struct Stream {
    stream_id: u64,
    kind: StreamKind,
    chunks: AsyncReceiver<StreamChunk>,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum StreamKind {
    ClientBidirectional,
}

enum StreamChunk {
    Bytes(Vec<u8>),
    Finished,
    Cancelled,
}

pub(crate) struct ReceiveStream {
    connection_id: ConnectionId,
    stream_id: u64,
    commands: Sender<Command>,
    chunks: AsyncReceiver<StreamChunk>,
    buffered: Vec<u8>,
    finished: bool,
    cancelled: bool,
}

impl super::RequestByteStream for ReceiveStream {
    async fn append_chunk(
        &mut self,
        mut frame: Vec<u8>,
        capacity: usize,
        backend: &'static str,
    ) -> Result<super::ChunkRead, TransportError> {
        let capacity = capacity.max(1);
        if !self.buffered.is_empty() {
            let take = capacity.min(self.buffered.len());
            frame.try_reserve(take).map_err(|error| {
                TransportError::backend(backend, "request buffer reserve", error)
            })?;
            frame.extend(self.buffered.drain(..take));
            return Ok(super::ChunkRead::Bytes(frame));
        }
        if self.cancelled {
            return Ok(super::ChunkRead::Cancelled);
        }
        if self.finished {
            return Ok(super::ChunkRead::Finished);
        }
        let chunk = self
            .chunks
            .recv_async_network()
            .await
            .map_err(|error| TransportError::backend(backend, "stream read", error))?;
        match chunk {
            StreamChunk::Bytes(bytes) => {
                self.buffered = bytes;
                let _ = self.commands.try_send(Command::ResumeRequest {
                    connection_id: self.connection_id.clone(),
                    stream_id: self.stream_id,
                });
                let take = capacity.min(self.buffered.len());
                frame.try_reserve(take).map_err(|error| {
                    TransportError::backend(backend, "request buffer reserve", error)
                })?;
                frame.extend(self.buffered.drain(..take));
                Ok(super::ChunkRead::Bytes(frame))
            }
            StreamChunk::Finished => {
                self.finished = true;
                Ok(super::ChunkRead::Finished)
            }
            StreamChunk::Cancelled => {
                self.cancelled = true;
                Ok(super::ChunkRead::Cancelled)
            }
        }
    }
}

impl super::ReceiveStream for ReceiveStream {
    async fn read_request<T>(
        &mut self,
        maximum: usize,
        timeout: Duration,
        budget: &RequestBudget,
        admit: impl FnOnce(RequestFrameHeader, &[u8]) -> Result<(), T>,
    ) -> Result<RequestRead<T>, StreamReadError> {
        read_buffered_request(self, NAME, maximum, timeout, budget, admit).await
    }

    fn stop(&mut self) {
        let _ = self.commands.try_send(Command::StopRequest {
            connection_id: self.connection_id.clone(),
            stream_id: self.stream_id,
        });
    }
}

impl Drop for ReceiveStream {
    fn drop(&mut self) {
        let _ = self.commands.try_send(Command::StopRequest {
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

    fn finish(&mut self) -> Result<(), TransportError> {
        self.commands
            .try_send(Command::FinishResponse {
                connection_id: self.connection_id.clone(),
                stream_id: self.stream_id,
            })
            .map_err(|error| TransportError::backend(NAME, "stream finish", error))
    }

    fn reset(&mut self) {
        let _ = self.commands.try_send(Command::ResetResponse {
            connection_id: self.connection_id.clone(),
            stream_id: self.stream_id,
        });
    }
}

#[path = "quiche_backend/configuration.rs"]
mod configuration;
#[path = "quiche_backend/driver.rs"]
mod driver;
