//! QUIC backend boundary used by the OpenKache protocol server.

use std::future::Future;
use std::net::SocketAddr;
#[cfg(any(feature = "quic-quinn", feature = "quic-noq"))]
use std::sync::Arc;
use std::time::Duration;

use compio::BufResult;
#[cfg(any(feature = "quic-quinn", feature = "quic-noq"))]
use compio::io::{AsyncReadExt, AsyncWriteExt};
#[cfg(any(feature = "quic-quinn", feature = "quic-noq"))]
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

use crate::QuicBackend;

/// Backend-independent endpoint selected when the server binds.
pub(super) enum ServerEndpoint {
    #[cfg(feature = "quic-quinn")]
    Quinn(quinn_backend::Endpoint),
    #[cfg(feature = "quic-noq")]
    Noq(noq_backend::Endpoint),
    #[cfg(feature = "quic-quiche")]
    Quiche(quiche_backend::Endpoint),
}

impl ServerEndpoint {
    /// Binds the selected implementation to a Compio UDP socket.
    pub(super) async fn bind(
        backend: QuicBackend,
        address: SocketAddr,
        certificate_der: &[u8],
        private_key_der: &[u8],
    ) -> Result<Self, TransportError> {
        match backend {
            QuicBackend::Quinn => {
                #[cfg(feature = "quic-quinn")]
                {
                    Ok(Self::Quinn(
                        quinn_backend::Endpoint::bind(address, certificate_der, private_key_der)
                            .await?,
                    ))
                }
                #[cfg(not(feature = "quic-quinn"))]
                {
                    Err(TransportError::not_compiled(backend, "quic-quinn"))
                }
            }
            QuicBackend::Noq => {
                #[cfg(feature = "quic-noq")]
                {
                    Ok(Self::Noq(
                        noq_backend::Endpoint::bind(address, certificate_der, private_key_der)
                            .await?,
                    ))
                }
                #[cfg(not(feature = "quic-noq"))]
                {
                    Err(TransportError::not_compiled(backend, "quic-noq"))
                }
            }
            QuicBackend::Quiche => {
                #[cfg(feature = "quic-quiche")]
                {
                    Ok(Self::Quiche(
                        quiche_backend::Endpoint::bind(address, certificate_der, private_key_der)
                            .await?,
                    ))
                }
                #[cfg(not(feature = "quic-quiche"))]
                {
                    Err(TransportError::not_compiled(backend, "quic-quiche"))
                }
            }
            QuicBackend::Neqo => Err(TransportError::unavailable(
                backend,
                "the official neqo transport is not published as a standalone crate and requires NSS certificate-database integration",
            )),
        }
    }

    /// Returns the UDP address selected by the operating system.
    pub(super) fn local_addr(&self) -> Result<SocketAddr, TransportError> {
        match self {
            #[cfg(feature = "quic-quinn")]
            Self::Quinn(endpoint) => endpoint.local_addr(),
            #[cfg(feature = "quic-noq")]
            Self::Noq(endpoint) => endpoint.local_addr(),
            #[cfg(feature = "quic-quiche")]
            Self::Quiche(endpoint) => endpoint.local_addr(),
        }
    }
}

/// Endpoint behavior needed by the generic connection serving loop.
pub(super) trait Endpoint {
    type Incoming: Incoming;

    fn wait_incoming(&self) -> impl Future<Output = Option<Self::Incoming>>;

    fn close(&self, reason: &[u8]);

    fn shutdown(self) -> impl Future<Output = Result<(), TransportError>>;
}

/// A QUIC handshake accepted by an endpoint.
pub(super) trait Incoming {
    type Connection: Connection;

    fn connect(self) -> impl Future<Output = Result<Self::Connection, TransportError>>;
}

/// A connected peer capable of accepting bidirectional streams.
pub(super) trait Connection {
    type SendStream: SendStream;
    type ReceiveStream: ReceiveStream;

    fn accept_bi(
        &self,
    ) -> impl Future<Output = Result<(Self::SendStream, Self::ReceiveStream), TransportError>>;
}

/// Receive half of one request stream.
pub(super) trait ReceiveStream {
    fn read_request(
        self,
        maximum: usize,
        timeout: Duration,
    ) -> impl Future<Output = Result<Vec<u8>, StreamReadError>>;
}

/// Send half of one response stream.
pub(super) trait SendStream {
    fn write_response(
        self,
        frame: Vec<u8>,
        timeout: Duration,
    ) -> impl Future<Output = Result<(), TransportError>>;
}

/// Failure while receiving a request frame.
#[derive(Debug, thiserror::Error)]
pub(super) enum StreamReadError {
    #[error("request read timed out")]
    Timeout,
    #[error(transparent)]
    Transport(#[from] TransportError),
}

/// Stable transport failure with backend and operation context.
#[derive(Debug, thiserror::Error)]
#[error("{backend} QUIC {operation} failed: {message}")]
pub struct TransportError {
    backend: &'static str,
    operation: &'static str,
    message: String,
}

impl TransportError {
    fn backend(
        backend: &'static str,
        operation: &'static str,
        error: impl std::fmt::Display,
    ) -> Self {
        Self {
            backend,
            operation,
            message: error.to_string(),
        }
    }

    fn not_compiled(backend: QuicBackend, feature: &'static str) -> Self {
        Self {
            backend: backend.as_str(),
            operation: "selection",
            message: format!("backend was not compiled; enable Cargo feature `{feature}`"),
        }
    }

    fn unavailable(backend: QuicBackend, reason: &'static str) -> Self {
        Self {
            backend: backend.as_str(),
            operation: "selection",
            message: format!("backend is unavailable: {reason}"),
        }
    }
}

#[cfg(any(feature = "quic-quinn", feature = "quic-noq"))]
fn tls_config(
    certificate_der: &[u8],
    private_key_der: &[u8],
) -> Result<rustls::ServerConfig, rustls::Error> {
    let mut tls = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from(certificate_der.to_vec())],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(private_key_der.to_vec())),
        )?;
    tls.alpn_protocols = vec![openkache_protocol::ALPN.to_vec()];
    Ok(tls)
}

#[cfg(feature = "quic-quinn")]
mod quinn_backend {
    use super::*;

    const NAME: &str = "quinn";

    pub(crate) struct Endpoint(compio_quic::Endpoint);

    impl Endpoint {
        pub(super) async fn bind(
            address: SocketAddr,
            certificate_der: &[u8],
            private_key_der: &[u8],
        ) -> Result<Self, TransportError> {
            let tls = tls_config(certificate_der, private_key_der)
                .map_err(|error| TransportError::backend(NAME, "TLS configuration", error))?;
            let crypto = compio_quic::crypto::rustls::QuicServerConfig::try_from(tls)
                .map_err(|error| TransportError::backend(NAME, "TLS initialization", error))?;
            let socket = compio::net::UdpSocket::bind(address)
                .await
                .map_err(|error| TransportError::backend(NAME, "bind", error))?;
            let endpoint = compio_quic::Endpoint::new(
                socket,
                compio_quic::EndpointConfig::default(),
                Some(compio_quic::ServerConfig::with_crypto(Arc::new(crypto))),
                None,
            )
            .map_err(|error| TransportError::backend(NAME, "endpoint initialization", error))?;
            Ok(Self(endpoint))
        }

        pub(super) fn local_addr(&self) -> Result<SocketAddr, TransportError> {
            self.0
                .local_addr()
                .map_err(|error| TransportError::backend(NAME, "local address", error))
        }
    }

    impl super::Endpoint for Endpoint {
        type Incoming = Incoming;

        async fn wait_incoming(&self) -> Option<Self::Incoming> {
            self.0.wait_incoming().await.map(Incoming)
        }

        fn close(&self, reason: &[u8]) {
            self.0.close(compio_quic::VarInt::from_u32(0), reason);
        }

        async fn shutdown(self) -> Result<(), TransportError> {
            self.0
                .shutdown()
                .await
                .map_err(|error| TransportError::backend(NAME, "shutdown", error))
        }
    }

    pub(crate) struct Incoming(compio_quic::Incoming);

    impl super::Incoming for Incoming {
        type Connection = Connection;

        async fn connect(self) -> Result<Self::Connection, TransportError> {
            self.0
                .await
                .map(Connection)
                .map_err(|error| TransportError::backend(NAME, "handshake", error))
        }
    }

    pub(crate) struct Connection(compio_quic::Connection);

    impl super::Connection for Connection {
        type SendStream = SendStream;
        type ReceiveStream = ReceiveStream;

        async fn accept_bi(
            &self,
        ) -> Result<(Self::SendStream, Self::ReceiveStream), TransportError> {
            self.0
                .accept_bi()
                .await
                .map(|(send, receive)| (SendStream(send), ReceiveStream(receive)))
                .map_err(|error| TransportError::backend(NAME, "stream accept", error))
        }
    }

    pub(crate) struct ReceiveStream(compio_quic::RecvStream);

    impl super::ReceiveStream for ReceiveStream {
        async fn read_request(
            self,
            maximum: usize,
            timeout: Duration,
        ) -> Result<Vec<u8>, StreamReadError> {
            let mut receive = self.0.take(maximum as u64);
            match compio::runtime::time::timeout(timeout, receive.read_to_end(Vec::new())).await {
                Err(_) => Err(StreamReadError::Timeout),
                Ok(BufResult(Err(error), _)) => {
                    Err(TransportError::backend(NAME, "stream read", error).into())
                }
                Ok(BufResult(Ok(_), frame)) => Ok(frame),
            }
        }
    }

    pub(crate) struct SendStream(compio_quic::SendStream);

    impl super::SendStream for SendStream {
        async fn write_response(
            mut self,
            frame: Vec<u8>,
            timeout: Duration,
        ) -> Result<(), TransportError> {
            let BufResult(result, _) =
                compio::runtime::time::timeout(timeout, self.0.write_all(frame))
                    .await
                    .map_err(|error| {
                        TransportError::backend(NAME, "stream write timeout", error)
                    })?;
            result.map_err(|error| TransportError::backend(NAME, "stream write", error))?;
            self.0
                .finish()
                .map_err(|error| TransportError::backend(NAME, "stream finish", error))
        }
    }
}

#[cfg(feature = "quic-noq")]
mod noq_backend {
    use super::*;

    const NAME: &str = "noq";

    pub(crate) struct Endpoint(comnoq::Endpoint);

    impl Endpoint {
        pub(super) async fn bind(
            address: SocketAddr,
            certificate_der: &[u8],
            private_key_der: &[u8],
        ) -> Result<Self, TransportError> {
            let tls = tls_config(certificate_der, private_key_der)
                .map_err(|error| TransportError::backend(NAME, "TLS configuration", error))?;
            let crypto = comnoq::crypto::rustls::QuicServerConfig::try_from(tls)
                .map_err(|error| TransportError::backend(NAME, "TLS initialization", error))?;
            let socket = compio::net::UdpSocket::bind(address)
                .await
                .map_err(|error| TransportError::backend(NAME, "bind", error))?;
            let endpoint = comnoq::Endpoint::new(
                socket,
                comnoq::EndpointConfig::default(),
                Some(comnoq::ServerConfig::with_crypto(Arc::new(crypto))),
                None,
            )
            .map_err(|error| TransportError::backend(NAME, "endpoint initialization", error))?;
            Ok(Self(endpoint))
        }

        pub(super) fn local_addr(&self) -> Result<SocketAddr, TransportError> {
            self.0
                .local_addr()
                .map_err(|error| TransportError::backend(NAME, "local address", error))
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

        async fn accept_bi(
            &self,
        ) -> Result<(Self::SendStream, Self::ReceiveStream), TransportError> {
            self.0
                .accept_bi()
                .await
                .map(|(send, receive)| (SendStream(send), ReceiveStream(receive)))
                .map_err(|error| TransportError::backend(NAME, "stream accept", error))
        }
    }

    pub(crate) struct ReceiveStream(comnoq::RecvStream);

    impl super::ReceiveStream for ReceiveStream {
        async fn read_request(
            self,
            maximum: usize,
            timeout: Duration,
        ) -> Result<Vec<u8>, StreamReadError> {
            let mut receive = self.0.take(maximum as u64);
            match compio::runtime::time::timeout(timeout, receive.read_to_end(Vec::new())).await {
                Err(_) => Err(StreamReadError::Timeout),
                Ok(BufResult(Err(error), _)) => {
                    Err(TransportError::backend(NAME, "stream read", error).into())
                }
                Ok(BufResult(Ok(_), frame)) => Ok(frame),
            }
        }
    }

    pub(crate) struct SendStream(comnoq::SendStream);

    impl super::SendStream for SendStream {
        async fn write_response(
            mut self,
            frame: Vec<u8>,
            timeout: Duration,
        ) -> Result<(), TransportError> {
            let BufResult(result, _) =
                compio::runtime::time::timeout(timeout, self.0.write_all(frame))
                    .await
                    .map_err(|error| {
                        TransportError::backend(NAME, "stream write timeout", error)
                    })?;
            result.map_err(|error| TransportError::backend(NAME, "stream write", error))?;
            self.0
                .finish()
                .map_err(|error| TransportError::backend(NAME, "stream finish", error))
        }
    }
}

#[cfg(feature = "quic-quiche")]
mod quiche_backend {
    use std::collections::HashMap;

    use boring::pkey::PKey;
    use boring::ssl::{SslContextBuilder, SslMethod};
    use boring::x509::X509;
    use compio::runtime::JoinHandle;
    use futures_util::{FutureExt, StreamExt, pin_mut, select};

    use super::*;

    const NAME: &str = "quiche";
    const MAX_DATAGRAM_BYTES: usize = 65_535;
    const MAX_BUFFERED_REQUEST_BYTES: usize = openkache_protocol::MAX_REQUEST_FRAME_BYTES + 1;

    pub(crate) struct Endpoint {
        local_address: SocketAddr,
        incoming: flume::Receiver<Incoming>,
        commands: flume::Sender<Command>,
        driver: JoinHandle<Result<(), TransportError>>,
    }

    impl Endpoint {
        pub(super) async fn bind(
            address: SocketAddr,
            certificate_der: &[u8],
            private_key_der: &[u8],
        ) -> Result<Self, TransportError> {
            let socket = compio::net::UdpSocket::bind(address)
                .await
                .map_err(|error| TransportError::backend(NAME, "bind", error))?;
            let local_address = socket
                .local_addr()
                .map_err(|error| TransportError::backend(NAME, "local address", error))?;
            let config = config(certificate_der, private_key_der)?;
            let (incoming_sender, incoming) = flume::unbounded();
            let (commands, command_receiver) = flume::unbounded();
            let driver_commands = commands.clone();
            let driver = compio::runtime::spawn(async move {
                Driver {
                    socket,
                    local_address,
                    config,
                    incoming: incoming_sender,
                    command_sender: driver_commands,
                    commands: command_receiver,
                    routes: HashMap::new(),
                    clients: HashMap::new(),
                }
                .run()
                .await
            });
            Ok(Self {
                local_address,
                incoming,
                commands,
                driver,
            })
        }

        pub(super) fn local_addr(&self) -> Result<SocketAddr, TransportError> {
            Ok(self.local_address)
        }
    }

    impl super::Endpoint for Endpoint {
        type Incoming = Incoming;

        async fn wait_incoming(&self) -> Option<Self::Incoming> {
            self.incoming.recv_async().await.ok()
        }

        fn close(&self, reason: &[u8]) {
            let _ = self.commands.try_send(Command::Close(reason.to_vec()));
        }

        async fn shutdown(self) -> Result<(), TransportError> {
            let _ = self.commands.send_async(Command::Shutdown).await;
            match self.driver.await {
                Ok(result) => result,
                Err(error) => Err(TransportError::backend(
                    NAME,
                    "driver task",
                    format!("{error:?}"),
                )),
            }
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
        connection_id: Vec<u8>,
        streams: flume::Receiver<Stream>,
        commands: flume::Sender<Command>,
    }

    impl super::Connection for Connection {
        type SendStream = SendStream;
        type ReceiveStream = ReceiveStream;

        async fn accept_bi(
            &self,
        ) -> Result<(Self::SendStream, Self::ReceiveStream), TransportError> {
            let stream = self
                .streams
                .recv_async()
                .await
                .map_err(|error| TransportError::backend(NAME, "stream accept", error))?;
            Ok((
                SendStream {
                    connection_id: self.connection_id.clone(),
                    stream_id: stream.stream_id,
                    commands: self.commands.clone(),
                },
                ReceiveStream(stream.request),
            ))
        }
    }

    struct Stream {
        stream_id: u64,
        request: flume::Receiver<Vec<u8>>,
    }

    pub(crate) struct ReceiveStream(flume::Receiver<Vec<u8>>);

    impl super::ReceiveStream for ReceiveStream {
        async fn read_request(
            self,
            maximum: usize,
            timeout: Duration,
        ) -> Result<Vec<u8>, StreamReadError> {
            let receive = self.0.recv_async();
            let mut frame = compio::runtime::time::timeout(timeout, receive)
                .await
                .map_err(|_| StreamReadError::Timeout)?
                .map_err(|error| TransportError::backend(NAME, "stream read", error))?;
            frame.truncate(maximum);
            Ok(frame)
        }
    }

    pub(crate) struct SendStream {
        connection_id: Vec<u8>,
        stream_id: u64,
        commands: flume::Sender<Command>,
    }

    impl super::SendStream for SendStream {
        async fn write_response(
            self,
            frame: Vec<u8>,
            timeout: Duration,
        ) -> Result<(), TransportError> {
            let (reply, response) = flume::bounded(1);
            self.commands
                .send_async(Command::SendResponse {
                    connection_id: self.connection_id,
                    stream_id: self.stream_id,
                    frame,
                    reply,
                })
                .await
                .map_err(|error| TransportError::backend(NAME, "stream write", error))?;
            let result = compio::runtime::time::timeout(timeout, response.recv_async())
                .await
                .map_err(|error| TransportError::backend(NAME, "stream write timeout", error))?
                .map_err(|error| TransportError::backend(NAME, "stream write", error))?;
            result.map_err(|message| TransportError::backend(NAME, "stream write", message))
        }
    }

    enum Command {
        Close(Vec<u8>),
        SendResponse {
            connection_id: Vec<u8>,
            stream_id: u64,
            frame: Vec<u8>,
            reply: flume::Sender<Result<(), String>>,
        },
        Shutdown,
    }

    struct PendingResponse {
        frame: Vec<u8>,
        written: usize,
        reply: flume::Sender<Result<(), String>>,
    }

    struct Request {
        frame: Vec<u8>,
        completed: flume::Sender<Vec<u8>>,
    }

    struct Client {
        connection: quiche::Connection,
        announced: bool,
        streams: Option<flume::Sender<Stream>>,
        requests: HashMap<u64, Request>,
        responses: HashMap<u64, PendingResponse>,
    }

    struct Driver {
        socket: compio::net::UdpSocket,
        local_address: SocketAddr,
        config: quiche::Config,
        incoming: flume::Sender<Incoming>,
        command_sender: flume::Sender<Command>,
        commands: flume::Receiver<Command>,
        routes: HashMap<Vec<u8>, Vec<u8>>,
        clients: HashMap<Vec<u8>, Client>,
    }

    impl Driver {
        async fn run(mut self) -> Result<(), TransportError> {
            let receive_socket = self.socket.clone();
            let mut packets = Box::pin(receive_socket.recv_from_multi());
            let commands = self.commands.clone();
            loop {
                let packet = packets.next().fuse();
                let command = commands.recv_async().fuse();
                let timeout = compio::runtime::time::sleep(
                    self.next_timeout()
                        .unwrap_or_else(|| Duration::from_secs(86_400)),
                )
                .fuse();
                pin_mut!(packet, command, timeout);

                let mut shutting_down = false;
                select! {
                    packet = packet => self.receive_packet(packet)?,
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
            packet: Option<std::io::Result<compio::driver::op::RecvFromMultiResult>>,
        ) -> Result<(), TransportError> {
            let packet = packet
                .ok_or_else(|| {
                    TransportError::backend(NAME, "receive", "UDP receive stream ended")
                })?
                .map_err(|error| TransportError::backend(NAME, "receive", error))?;
            let Some(peer_address) = packet.addr().and_then(|address| address.as_socket()) else {
                return Ok(());
            };
            let mut datagram = packet.data().to_vec();
            let header = match quiche::Header::from_slice(&mut datagram, quiche::MAX_CONN_ID_LEN) {
                Ok(header) => header,
                Err(_) => return Ok(()),
            };
            let destination_id = header.dcid.as_ref().to_vec();
            let connection_id = match self.routes.get(&destination_id) {
                Some(connection_id) => connection_id.clone(),
                None => {
                    if header.ty != quiche::Type::Initial
                        || !quiche::version_is_supported(header.version)
                    {
                        return Ok(());
                    }
                    self.accept_connection(destination_id, peer_address)?
                }
            };
            let Some(client) = self.clients.get_mut(&connection_id) else {
                return Ok(());
            };
            let receive_info = quiche::RecvInfo {
                from: peer_address,
                to: self.local_address,
            };
            if client.connection.recv(&mut datagram, receive_info).is_err() {
                return Ok(());
            }
            Ok(())
        }

        fn accept_connection(
            &mut self,
            original_destination_id: Vec<u8>,
            peer_address: SocketAddr,
        ) -> Result<Vec<u8>, TransportError> {
            let connection_id = rand::random::<[u8; quiche::MAX_CONN_ID_LEN]>().to_vec();
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
                .insert(connection_id.clone(), connection_id.clone());
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
                Command::SendResponse {
                    connection_id,
                    stream_id,
                    frame,
                    reply,
                } => {
                    let Some(client) = self.clients.get_mut(&connection_id) else {
                        let _ = reply.try_send(Err("connection is closed".to_string()));
                        return false;
                    };
                    client.responses.insert(
                        stream_id,
                        PendingResponse {
                            frame,
                            written: 0,
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
            let connection_ids: Vec<_> = self.clients.keys().cloned().collect();
            for connection_id in connection_ids {
                let Some(client) = self.clients.get_mut(&connection_id) else {
                    continue;
                };
                if !client.announced && client.connection.is_established() {
                    let (streams, stream_receiver) = flume::unbounded();
                    client.streams = Some(streams);
                    client.announced = true;
                    let _ = self.incoming.try_send(Incoming(Connection {
                        connection_id: connection_id.clone(),
                        streams: stream_receiver,
                        commands: self.command_sender.clone(),
                    }));
                }

                let writable: Vec<_> = client.connection.writable().collect();
                for stream_id in writable {
                    progress_response(client, stream_id);
                }

                let readable: Vec<_> = client.connection.readable().collect();
                for stream_id in readable {
                    if stream_id & 0x3 != 0 {
                        continue;
                    }
                    receive_stream(client, stream_id);
                }
            }
        }

        async fn flush_packets(&mut self) -> Result<(), TransportError> {
            let mut datagrams = Vec::new();
            let mut output = vec![0_u8; MAX_DATAGRAM_BYTES];
            for client in self.clients.values_mut() {
                loop {
                    match client.connection.send(&mut output) {
                        Ok((written, information)) => {
                            datagrams.push((output[..written].to_vec(), information.to));
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
            for (datagram, address) in datagrams {
                let BufResult(result, _) = self.socket.send_to(datagram, address).await;
                result.map_err(|error| TransportError::backend(NAME, "packet send", error))?;
            }
            Ok(())
        }

        fn collect_closed(&mut self) {
            self.clients
                .retain(|_, client| !client.connection.is_closed());
            self.routes
                .retain(|_, connection_id| self.clients.contains_key(connection_id));
        }
    }

    fn receive_stream(client: &mut Client, stream_id: u64) {
        let Some(stream_sender) = client.streams.clone() else {
            return;
        };
        let request = client.requests.entry(stream_id).or_insert_with(|| {
            let (completed, request) = flume::bounded(1);
            let _ = stream_sender.try_send(Stream { stream_id, request });
            Request {
                frame: Vec::new(),
                completed,
            }
        });
        let mut buffer = [0_u8; 16_384];
        loop {
            match client.connection.stream_recv(stream_id, &mut buffer) {
                Ok((read, finished)) => {
                    let remaining = MAX_BUFFERED_REQUEST_BYTES.saturating_sub(request.frame.len());
                    request
                        .frame
                        .extend_from_slice(&buffer[..read.min(remaining)]);
                    if finished {
                        if let Some(request) = client.requests.remove(&stream_id) {
                            let _ = request.completed.try_send(request.frame);
                        }
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
        let remaining = &response.frame[response.written..];
        match client.connection.stream_send(stream_id, remaining, true) {
            Ok(written) => {
                response.written += written;
                if response.written == response.frame.len() {
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
        certificate_der: &[u8],
        private_key_der: &[u8],
    ) -> Result<quiche::Config, TransportError> {
        let certificate = X509::from_der(certificate_der)
            .map_err(|error| TransportError::backend(NAME, "certificate parsing", error))?;
        let private_key = PKey::private_key_from_der(private_key_der)
            .map_err(|error| TransportError::backend(NAME, "private key parsing", error))?;
        let mut tls = SslContextBuilder::new(SslMethod::tls())
            .map_err(|error| TransportError::backend(NAME, "TLS configuration", error))?;
        tls.set_certificate(&certificate)
            .map_err(|error| TransportError::backend(NAME, "TLS certificate", error))?;
        tls.set_private_key(&private_key)
            .map_err(|error| TransportError::backend(NAME, "TLS private key", error))?;
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
        config.set_initial_max_streams_bidi(1_024);
        config.set_disable_active_migration(true);
        Ok(config)
    }
}
