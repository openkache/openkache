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
            connection_id: None,
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

    fn close(&self, error_code: u64, reason: &[u8]) {
        let _ = self.commands.try_send(Command::Close {
            connection_id: Some(self.connection_id.clone()),
            error_code,
            reason: reason.to_vec(),
        });
    }

    async fn accept_bi(&self) -> Result<(Self::SendStream, Self::ReceiveStream), TransportError> {
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
                ended: false,
            },
        ))
    }
}

struct Stream {
    stream_id: u64,
    chunks: AsyncReceiver<StreamChunk>,
}

pub(crate) struct ReceiveStream {
    connection_id: ConnectionId,
    stream_id: u64,
    commands: Sender<Command>,
    chunks: AsyncReceiver<StreamChunk>,
    buffered: Vec<u8>,
    ended: bool,
}

impl ReceiveStream {
    async fn next_chunk(&mut self, operation: &'static str) -> Result<StreamChunk, TransportError> {
        if self.ended {
            return Ok(StreamChunk::Fin);
        }
        let chunk = self
            .chunks
            .recv_async_network()
            .await
            .map_err(|error| TransportError::backend(NAME, operation, error))?;
        if matches!(&chunk, StreamChunk::Data(_)) {
            let _ = self.commands.try_send(Command::ResumeRequest {
                connection_id: self.connection_id.clone(),
                stream_id: self.stream_id,
            });
        }
        if matches!(&chunk, StreamChunk::Fin) {
            self.ended = true;
        }
        Ok(chunk)
    }
}

impl super::ReceiveStream for ReceiveStream {
    async fn read_request<T>(
        &mut self,
        maximum: usize,
        timeout: Duration,
        budget: &RequestBudget,
        admit: impl FnOnce(RequestFrameHeader, &[u8]) -> Result<(), T>,
    ) -> Result<Result<RequestFrame, T>, StreamReadError> {
        network_runtime::timeout(timeout, async {
            while self.buffered.is_empty() {
                match self.next_chunk("stream header read").await? {
                    StreamChunk::Data(chunk) => self.buffered.extend_from_slice(&chunk),
                    StreamChunk::Fin => return Err(StreamReadError::EndOfStream),
                    StreamChunk::Reset(error_code) => {
                        return Err(StreamReadError::Transport(TransportError::backend(
                            NAME,
                            "stream read",
                            format!("stream reset with error code {error_code}"),
                        )));
                    }
                }
            }
            Ok::<(), StreamReadError>(())
        })
        .await
        .map_err(|_| StreamReadError::Timeout)??;
        let header = network_runtime::timeout(timeout, async {
            loop {
                if let Some(header) = ProtocolRequestFrame::decode_header(&self.buffered)? {
                    break Ok::<_, StreamReadError>(header);
                }
                if self.buffered.len() >= maximum {
                    return Err(StreamReadError::TooLarge);
                }
                match self.next_chunk("stream header read").await? {
                    StreamChunk::Data(chunk) => self.buffered.extend_from_slice(&chunk),
                    StreamChunk::Fin => return Err(StreamReadError::Truncated),
                    StreamChunk::Reset(error_code) => {
                        return Err(StreamReadError::Transport(TransportError::backend(
                            NAME,
                            "stream read",
                            format!("stream reset with error code {error_code}"),
                        )));
                    }
                }
            }
        })
        .await
        .map_err(|_| StreamReadError::Timeout)??;
        let frame_len = network_runtime::timeout(timeout, async {
            let frame_len = header.frame_len()?;
            Ok::<_, StreamReadError>(frame_len)
        })
        .await
        .map_err(|_| StreamReadError::Timeout)??;
        if frame_len > maximum {
            return Err(StreamReadError::TooLarge);
        }
        // Consume the declared body before returning an admission rejection.
        // This preserves the next frame boundary and keeps the response
        // correlated with the request that was admitted by its header.
        let rejection = admit(header, &self.buffered[..header.encoded_len()]).err();
        let permit = budget.acquire(header.body_len(), timeout).await?;
        network_runtime::timeout(timeout, async {
            while self.buffered.len() < frame_len {
                match self.next_chunk("stream body read").await? {
                    StreamChunk::Data(chunk) => self.buffered.extend_from_slice(&chunk),
                    StreamChunk::Fin => return Err(StreamReadError::Truncated),
                    StreamChunk::Reset(error_code) => {
                        return Err(StreamReadError::Transport(TransportError::backend(
                            NAME,
                            "stream read",
                            format!("stream reset with error code {error_code}"),
                        )));
                    }
                }
            }
            Ok::<(), StreamReadError>(())
        })
        .await
        .map_err(|_| StreamReadError::Timeout)??;
        let has_trailing_bytes = self.buffered.len() > frame_len;
        let frame = if self.buffered.len() == frame_len {
            std::mem::take(&mut self.buffered)
        } else {
            self.buffered.drain(..frame_len).collect()
        };
        if let Some(rejection) = rejection {
            drop(permit);
            return Ok(Err(rejection));
        }
        Ok(Ok(RequestFrame::with_trailing_bytes(
            frame,
            permit,
            header.request_id(),
            has_trailing_bytes,
        )))
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

#[path = "quiche_backend/configuration.rs"]
mod configuration;
#[path = "quiche_backend/driver.rs"]
mod driver;
