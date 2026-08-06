//! Runtime-neutral network I/O used by the protocol servers.
//!
//! The protocol and framing layers deliberately depend only on the small
//! buffer API in this module. Runtime-specific sockets, buffers, timers, task
//! scheduling, and executor construction stay in the selected backend below.

use std::future::Future;
use std::io;
#[cfg(feature = "quic-quiche")]
use std::net::UdpSocket as StdUdpSocket;
use std::net::{SocketAddr, TcpListener as StdTcpListener};
use std::time::Duration;

#[cfg(feature = "network-runtime-compio")]
#[path = "compio.rs"]
mod compio_backend;
#[cfg(feature = "network-runtime-glommio")]
#[path = "glommio.rs"]
mod glommio_backend;
#[cfg(feature = "network-runtime-kimojio")]
#[path = "kimojio.rs"]
mod kimojio_backend;
#[cfg(feature = "network-runtime-monoio")]
#[path = "monoio.rs"]
mod monoio_backend;

#[cfg(feature = "network-runtime-compio")]
use compio_backend as backend;
#[cfg(feature = "network-runtime-glommio")]
use glommio_backend as backend;
#[cfg(feature = "network-runtime-kimojio")]
use kimojio_backend as backend;
#[cfg(feature = "network-runtime-monoio")]
use monoio_backend as backend;

#[cfg(feature = "network-runtime-kimojio")]
use kimojio_backend::{KimojioTcpListener, KimojioTcpStream, KimojioUdpSocket};

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct RuntimeConfig {
    pub(crate) entries: u32,
    pub(crate) event_interval: usize,
    pub(crate) cpu_id: Option<usize>,
    pub(crate) worker_index: Option<usize>,
}

#[derive(Debug)]
pub(crate) struct Timeout;

#[cfg(feature = "quic-quiche")]
#[derive(Debug)]
pub(crate) struct Datagram {
    payload: DatagramPayload,
    address: SocketAddr,
}

#[cfg(feature = "quic-quiche")]
#[derive(Debug)]
enum DatagramPayload {
    #[cfg(not(feature = "network-runtime-compio"))]
    Owned(Vec<u8>),
    #[cfg(feature = "network-runtime-compio")]
    Compio {
        buffer: compio::driver::BufferRef,
        payload: std::ops::Range<usize>,
    },
}

#[cfg(feature = "quic-quiche")]
impl Datagram {
    pub(crate) const fn address(&self) -> SocketAddr {
        self.address
    }

    pub(crate) fn payload_mut(&mut self) -> &mut [u8] {
        match &mut self.payload {
            #[cfg(not(feature = "network-runtime-compio"))]
            DatagramPayload::Owned(buffer) => buffer,
            #[cfg(feature = "network-runtime-compio")]
            DatagramPayload::Compio { buffer, payload } => &mut buffer[payload.clone()],
        }
    }
}

/// Receive behavior exposed by a runtime-neutral datagram adapter.
///
/// `Batched` means that [`DatagramReceiver::recv_batch`] may return more than
/// one packet for a single readiness operation.  Callers must continue to work
/// with both variants: a runtime without a native batching primitive returns a
/// one-element batch through the same API.
#[cfg(feature = "quic-quiche")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DatagramCapability {
    Single,
    #[allow(dead_code)]
    Batched,
}

/// Runs a network task on a dedicated executor for the selected backend.
pub(crate) fn run<F>(config: RuntimeConfig, future: F) -> io::Result<F::Output>
where
    F: Future + 'static,
{
    backend::run(config, future)
}

/// Runs a top-level future on the selected network executor.
pub fn block_on<F>(future: F) -> io::Result<F::Output>
where
    F: Future + 'static,
{
    backend::block_on(future)
}

/// Spawns a task on the currently running selected network executor.
pub(crate) fn spawn_detached<F>(future: F)
where
    F: Future<Output = ()> + 'static,
{
    backend::spawn_detached(future);
}

/// Applies the selected runtime's timeout primitive to a future.
pub(crate) async fn timeout<F>(duration: Duration, future: F) -> Result<F::Output, Timeout>
where
    F: Future,
{
    backend::timeout(duration, future).await
}

/// Sleeps using the selected runtime's timer.
///
/// Compio-only builds without a timer consumer do not carry an unused shim.
#[cfg(any(
    feature = "quic-quiche",
    feature = "network-runtime-kimojio",
    not(feature = "network-runtime-compio")
))]
pub(crate) async fn sleep(duration: Duration) {
    backend::sleep(duration).await;
}

/// Waits for SIGINT or SIGTERM without exposing a runtime-specific signal API.
pub async fn shutdown_signal() {
    backend::shutdown_signal().await;
}

/// The compile-time-selected network runtime name used in diagnostics.
pub const fn name() -> &'static str {
    backend::NAME
}

/// Returns whether this network runtime can execute a network role on an
/// already-running instance of the same runtime.
pub(crate) const fn supports_combined_network_role() -> bool {
    backend::SUPPORTS_COMBINED_NETWORK_ROLE
}

/// Owned-buffer stream operations shared by RESP and protocol framing.
pub(crate) enum TcpStream {
    #[cfg(feature = "network-runtime-compio")]
    Compio(compio::net::TcpStream),
    #[cfg(feature = "network-runtime-monoio")]
    Monoio(monoio::net::TcpStream),
    #[cfg(feature = "network-runtime-glommio")]
    Glommio(glommio::net::TcpStream),
    #[cfg(feature = "network-runtime-kimojio")]
    Kimojio(KimojioTcpStream),
}

impl TcpStream {
    pub(crate) fn set_nodelay(&self, nodelay: bool) -> io::Result<()> {
        match self {
            #[cfg(feature = "network-runtime-compio")]
            Self::Compio(stream) => stream.set_nodelay(nodelay),
            #[cfg(feature = "network-runtime-monoio")]
            Self::Monoio(stream) => stream.set_nodelay(nodelay),
            #[cfg(feature = "network-runtime-glommio")]
            Self::Glommio(stream) => stream
                .set_nodelay(nodelay)
                .map_err(|error| io::Error::other(error.to_string())),
            #[cfg(feature = "network-runtime-kimojio")]
            Self::Kimojio(stream) => stream.set_nodelay(nodelay),
        }
    }

    pub(crate) async fn read(&mut self, buffer: Vec<u8>) -> io::Result<(usize, Vec<u8>)> {
        match self {
            #[cfg(feature = "network-runtime-compio")]
            Self::Compio(stream) => {
                use compio::buf::{IntoInner, IoBuf};
                use compio::io::AsyncRead;

                let result = stream.read(buffer.slice(..)).await;
                let compio::BufResult(result, buffer) = result;
                result.map(|read| (read, buffer.into_inner()))
            }
            #[cfg(feature = "network-runtime-monoio")]
            Self::Monoio(stream) => {
                use monoio::io::AsyncReadRent;

                let (result, buffer) = stream.read(buffer).await;
                result.map(|read| (read, buffer))
            }
            #[cfg(feature = "network-runtime-glommio")]
            Self::Glommio(stream) => {
                use futures_util::io::AsyncReadExt;

                let mut buffer = buffer;
                let read = stream.read(&mut buffer).await?;
                Ok((read, buffer))
            }
            #[cfg(feature = "network-runtime-kimojio")]
            Self::Kimojio(stream) => stream.read(buffer).await,
        }
    }

    pub(crate) async fn write_all(&mut self, buffer: Vec<u8>) -> io::Result<Vec<u8>> {
        match self {
            #[cfg(feature = "network-runtime-compio")]
            Self::Compio(stream) => {
                use compio::io::AsyncWriteExt;

                let compio::BufResult(result, buffer) = stream.write_all(buffer).await;
                result.map(|_| buffer)
            }
            #[cfg(feature = "network-runtime-monoio")]
            Self::Monoio(stream) => {
                use monoio::io::AsyncWriteRentExt;

                let (result, buffer) = stream.write_all(buffer).await;
                result.map(|_| buffer)
            }
            #[cfg(feature = "network-runtime-glommio")]
            Self::Glommio(stream) => {
                use futures_util::io::AsyncWriteExt;

                stream.write_all(&buffer).await?;
                Ok(buffer)
            }
            #[cfg(feature = "network-runtime-kimojio")]
            Self::Kimojio(stream) => stream.write_all(buffer).await,
        }
    }
}

/// A runtime-neutral listener that converts accepted sockets into owned-buffer
/// streams.
pub(crate) enum TcpListener {
    #[cfg(feature = "network-runtime-compio")]
    Compio(compio::net::TcpListener),
    #[cfg(feature = "network-runtime-monoio")]
    Monoio(monoio::net::TcpListener),
    #[cfg(feature = "network-runtime-glommio")]
    Glommio(glommio::net::TcpListener),
    #[cfg(feature = "network-runtime-kimojio")]
    Kimojio(KimojioTcpListener),
}

impl TcpListener {
    pub(crate) fn from_std(listener: StdTcpListener) -> io::Result<Self> {
        backend::tcp_listener_from_std(listener)
    }

    pub(crate) async fn accept(&self) -> io::Result<(TcpStream, SocketAddr)> {
        backend::tcp_accept(self).await
    }
}

/// A runtime-neutral datagram socket.
#[cfg(feature = "quic-quiche")]
pub(crate) enum UdpSocket {
    #[cfg(feature = "network-runtime-compio")]
    Compio(std::sync::Arc<compio::net::UdpSocket>),
    #[cfg(feature = "network-runtime-monoio")]
    Monoio(std::sync::Arc<monoio::net::udp::UdpSocket>),
    #[cfg(feature = "network-runtime-glommio")]
    Glommio(std::sync::Arc<glommio::net::UdpSocket>),
    #[cfg(feature = "network-runtime-kimojio")]
    Kimojio(std::sync::Arc<KimojioUdpSocket>),
}

#[cfg(feature = "quic-quiche")]
impl UdpSocket {
    pub(crate) fn from_std(socket: StdUdpSocket) -> io::Result<Self> {
        backend::udp_socket_from_std(socket)
    }

    pub(crate) fn local_addr(&self) -> io::Result<SocketAddr> {
        backend::udp_local_addr(self)
    }

    pub(crate) fn receiver(&self) -> DatagramReceiver {
        backend::receiver(self)
    }

    pub(crate) async fn send_to(
        &self,
        buffer: Vec<u8>,
        length: usize,
        address: SocketAddr,
    ) -> io::Result<(usize, Vec<u8>)> {
        backend::udp_send_to(self, buffer, length, address).await
    }
}

/// Receives datagrams through the selected runtime without exposing its socket
/// or buffer types.
#[cfg(feature = "quic-quiche")]
pub(crate) enum DatagramReceiver {
    #[cfg(feature = "network-runtime-compio")]
    Compio(CompioDatagramReceiver),
    #[cfg(not(feature = "network-runtime-compio"))]
    Generic(UdpSocket),
}

#[cfg(all(feature = "quic-quiche", feature = "network-runtime-compio"))]
pub(crate) struct CompioDatagramReceiver {
    socket: std::sync::Arc<compio::net::UdpSocket>,
}

#[cfg(feature = "quic-quiche")]
impl DatagramReceiver {
    /// Reports whether this receiver can return multiple datagrams per poll.
    pub(crate) const fn capability(&self) -> DatagramCapability {
        match self {
            #[cfg(feature = "network-runtime-compio")]
            Self::Compio(_) => DatagramCapability::Batched,
            #[cfg(not(feature = "network-runtime-compio"))]
            Self::Generic(_) => DatagramCapability::Single,
        }
    }

    /// Receives one or more datagrams.
    ///
    /// Backends without a native batching operation return exactly one
    /// datagram.  The Compio path retains its `recv_from_multi` operation so
    /// ready packets are delivered together and its runtime-owned buffers
    /// remain inside the adapter.
    pub(crate) async fn recv_batch(&mut self) -> io::Result<Vec<Datagram>> {
        match self {
            #[cfg(feature = "network-runtime-compio")]
            Self::Compio(receiver) => {
                use futures_util::{FutureExt, StreamExt};

                let mut stream = receiver.socket.recv_from_multi();
                let packet = stream.next().await.ok_or_else(|| {
                    io::Error::new(io::ErrorKind::UnexpectedEof, "UDP stream ended")
                })??;
                let mut packets = Vec::with_capacity(4);
                packets.push(compio_datagram(packet)?);

                // `recv_from_multi` may have completed additional receives before
                // the first item was observed. Drain only immediately-ready
                // items before dropping the temporary stream so an already
                // batched datagram is not discarded.
                while let Some(Some(result)) = stream.next().now_or_never() {
                    packets.push(compio_datagram(result?)?);
                }
                Ok(packets)
            }
            #[cfg(not(feature = "network-runtime-compio"))]
            Self::Generic(socket) => {
                let buffer = vec![0_u8; 65_535];
                let (read, address, mut buffer) = backend::udp_recv_from(socket, buffer).await?;
                buffer.truncate(read);
                Ok(vec![Datagram {
                    payload: DatagramPayload::Owned(buffer),
                    address,
                }])
            }
        }
    }
}

#[cfg(all(feature = "quic-quiche", feature = "network-runtime-compio"))]
fn compio_datagram(packet: compio::driver::op::RecvFromMultiResult) -> io::Result<Datagram> {
    use compio::buf::IntoInner;

    let address = packet
        .addr()
        .and_then(|address| address.as_socket())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "UDP peer address missing"))?;
    let payload_start = packet.data().as_ptr() as usize;
    let payload_len = packet.data().len();
    let buffer = packet.into_inner();
    let buffer_start = buffer.as_ptr() as usize;
    let payload_start = payload_start.checked_sub(buffer_start).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "Compio UDP payload precedes its managed buffer",
        )
    })?;
    let payload_end = payload_start.checked_add(payload_len).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "Compio UDP payload range overflow",
        )
    })?;
    if payload_end > buffer.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Compio UDP payload exceeds its managed buffer",
        ));
    }
    Ok(Datagram {
        payload: DatagramPayload::Compio {
            buffer,
            payload: payload_start..payload_end,
        },
        address,
    })
}

#[cfg(not(feature = "network-runtime-compio"))]
async fn portable_shutdown_signal() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let interrupted = Arc::new(AtomicBool::new(false));
    let terminated = Arc::new(AtomicBool::new(false));
    let interrupt_id = signal_hook::flag::register(libc::SIGINT, Arc::clone(&interrupted));
    let terminate_id = signal_hook::flag::register(libc::SIGTERM, Arc::clone(&terminated));
    if interrupt_id.is_err() || terminate_id.is_err() {
        eprintln!("failed to install portable shutdown signal handlers");
        return;
    }
    while !interrupted.load(Ordering::Relaxed) && !terminated.load(Ordering::Relaxed) {
        sleep(Duration::from_millis(50)).await;
    }
}
