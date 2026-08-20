use super::*;
use compio::BufResult;
use compio::io::AsyncWriteExt;

const NAME: &str = "noq";

pub(crate) struct Endpoint(comnoq::Endpoint);

impl Endpoint {
    pub(super) async fn bind(
        socket: std::net::UdpSocket,
        material: Arc<ServerTlsConfig>,
        max_concurrent_streams: usize,
    ) -> Result<Self, TransportError> {
        let tls = strict_server_config(&material)
            .map_err(|error| TransportError::backend(NAME, "TLS configuration", error))?;
        let crypto = comnoq::crypto::rustls::QuicServerConfig::try_from(tls)
            .map_err(|error| TransportError::backend(NAME, "TLS initialization", error))?;
        let socket = compio::net::UdpSocket::from_std(socket)
            .map_err(|error| TransportError::backend(NAME, "socket initialization", error))?;
        let max_concurrent_streams = u32::try_from(max_concurrent_streams)
            .map_err(|error| TransportError::backend(NAME, "stream limit configuration", error))?;
        let mut transport = comnoq::TransportConfig::default();
        transport
            .max_concurrent_bidi_streams(comnoq::VarInt::from_u32(max_concurrent_streams))
            // Keep a bounded receive credit for invalid client uni streams so
            // the server can reject them with STOP_SENDING.
            .max_concurrent_uni_streams(comnoq::VarInt::from_u32(max_concurrent_streams));
        let mut server_config = comnoq::ServerConfig::with_crypto(Arc::new(crypto));
        server_config.transport_config(Arc::new(transport));
        let endpoint = comnoq::Endpoint::new(
            socket,
            comnoq::EndpointConfig::default(),
            Some(server_config),
            None,
        )
        .map_err(|error| TransportError::backend(NAME, "endpoint initialization", error))?;
        Ok(Self(endpoint))
    }
}

impl super::Endpoint for Endpoint {
    type Incoming = Incoming;

    async fn wait_incoming(&self) -> Option<Self::Incoming> {
        self.0.accept().await.map(Incoming)
    }

    fn close(&self, reason: &[u8]) {
        self.0.close(comnoq::VarInt::from_u32(0), reason);
    }

    async fn shutdown(self) -> Result<(), TransportError> {
        self.0
            .shutdown()
            .await
            .map_err(|error| TransportError::backend(NAME, "shutdown", error))
    }
}

pub(crate) struct Incoming(comnoq::Incoming);

impl super::Incoming for Incoming {
    type Connection = Connection;

    async fn connect(self) -> Result<Self::Connection, TransportError> {
        self.0
            .await
            .map(Connection)
            .map_err(|error| TransportError::backend(NAME, "handshake", error))
    }
}

pub(crate) struct Connection(comnoq::Connection);

impl super::Connection for Connection {
    type SendStream = SendStream;
    type ReceiveStream = ReceiveStream;

    fn take_peer_certificate(&mut self) -> Option<CertificateDer<'static>> {
        self.0
            .peer_identity()
            .and_then(|certificates| (*certificates).into_iter().next())
    }

    async fn accept_bi(&self) -> Result<(Self::SendStream, Self::ReceiveStream), TransportError> {
        self.0
            .accept_bi()
            .await
            .map(|(send, receive)| (SendStream(send), ReceiveStream(receive)))
            .map_err(|error| TransportError::backend(NAME, "stream accept", error))
    }

    async fn accept_uni(&self) -> Result<Self::ReceiveStream, TransportError> {
        self.0
            .accept_uni()
            .await
            .map(ReceiveStream)
            .map_err(|error| TransportError::backend(NAME, "unidirectional stream accept", error))
    }

    fn close(&self, error_code: u64, reason: &[u8]) {
        let code =
            comnoq::VarInt::from_u64(error_code).unwrap_or_else(|_| comnoq::VarInt::from_u32(1));
        self.0.close(code, reason);
    }
}

pub(crate) struct ReceiveStream(comnoq::RecvStream);

impl super::RequestByteStream for ReceiveStream {
    async fn append_chunk(
        &mut self,
        mut frame: Vec<u8>,
        capacity: usize,
        backend: &'static str,
    ) -> Result<super::ChunkRead, TransportError> {
        let capacity = capacity.max(1);
        match self.0.read_chunk(capacity).await {
            Ok(Some(chunk)) => {
                frame.try_reserve(chunk.len()).map_err(|error| {
                    TransportError::backend(backend, "request buffer reserve", error)
                })?;
                frame.extend_from_slice(&chunk);
                Ok(super::ChunkRead::Bytes(frame))
            }
            Ok(None) => Ok(super::ChunkRead::Finished),
            Err(comnoq::ReadError::Reset(_)) => Ok(super::ChunkRead::Cancelled),
            Err(error) => Err(TransportError::backend(backend, "stream read", error)),
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
        let _ = self.0.stop(comnoq::VarInt::from_u32(0));
    }
}

pub(crate) struct SendStream(comnoq::SendStream);

impl super::SendStream for SendStream {
    async fn write_response(
        &mut self,
        parts: ResponseParts,
        timeout: Duration,
    ) -> Result<(), TransportError> {
        let result = network_runtime::timeout(timeout, async {
            let BufResult(result, _) = self
                .0
                .write_vectored_all(response_write_segments(parts))
                .await;
            result.map_err(|error| TransportError::backend(NAME, "stream write", error))?;
            Ok(())
        })
        .await
        .map_err(|_| TransportError::backend(NAME, "stream write timeout", "timed out"))?;
        result?;
        Ok(())
    }

    fn finish(&mut self) -> Result<(), TransportError> {
        self.0
            .finish()
            .map_err(|error| TransportError::backend(NAME, "stream finish", error))
    }

    fn reset(&mut self) {
        let _ = self.0.reset(comnoq::VarInt::from_u32(0));
    }
}
