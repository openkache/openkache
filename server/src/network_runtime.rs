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

#[cfg(all(unix, feature = "network-runtime-kimojio"))]
use std::os::fd::AsRawFd;

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

#[cfg(feature = "network-runtime-compio")]
mod backend {
    use super::*;
    use std::collections::HashSet;

    use compio::driver::ProactorBuilder;
    use compio::runtime::{Runtime, RuntimeBuilder};

    pub(crate) const NAME: &str = "compio";
    pub(crate) const SUPPORTS_COMBINED_NETWORK_ROLE: bool = true;

    pub(crate) fn run<F>(config: RuntimeConfig, future: F) -> io::Result<F::Output>
    where
        F: Future + 'static,
    {
        let _ = config.event_interval;
        let _ = config.worker_index;
        let mut proactor = ProactorBuilder::new();
        proactor.capacity(config.entries);
        let mut builder = RuntimeBuilder::new();
        builder.with_proactor(proactor);
        if let Some(cpu_id) = config.cpu_id {
            builder.thread_affinity(HashSet::from([cpu_id]));
        }
        builder.event_interval(config.event_interval);
        let runtime = builder.build()?;
        require_native_driver(runtime.driver_type())?;
        Ok(runtime.block_on(future))
    }

    pub(crate) fn block_on<F>(future: F) -> io::Result<F::Output>
    where
        F: Future + 'static,
    {
        let runtime = Runtime::new()?;
        require_native_driver(runtime.driver_type())?;
        Ok(runtime.block_on(future))
    }

    fn require_native_driver(driver: compio::driver::DriverType) -> io::Result<()> {
        #[cfg(target_os = "linux")]
        {
            if driver.is_iouring() {
                return Ok(());
            }
            return Err(io::Error::other(
                "openkache-server requires the io_uring driver on Linux",
            ));
        }
        #[cfg(target_os = "macos")]
        {
            if driver.is_polling() {
                return Ok(());
            }
            return Err(io::Error::other(
                "openkache-server requires the polling driver on macOS",
            ));
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            let _ = driver;
            Ok(())
        }
    }

    pub(crate) fn spawn_detached<F>(future: F)
    where
        F: Future<Output = ()> + 'static,
    {
        compio::runtime::spawn(future).detach();
    }

    pub(crate) async fn timeout<F>(duration: Duration, future: F) -> Result<F::Output, Timeout>
    where
        F: Future,
    {
        compio::runtime::time::timeout(duration, future)
            .await
            .map_err(|_| Timeout)
    }

    #[cfg(feature = "quic-quiche")]
    pub(crate) async fn sleep(duration: Duration) {
        let _ = compio::runtime::time::sleep(duration).await;
    }

    pub(crate) async fn shutdown_signal() {
        let interrupt = compio::signal::ctrl_c();
        let terminate = compio::signal::unix::signal(libc::SIGTERM);
        futures_util::pin_mut!(interrupt, terminate);
        let _ = futures_util::future::select(interrupt, terminate).await;
    }

    pub(crate) fn tcp_listener_from_std(listener: StdTcpListener) -> io::Result<TcpListener> {
        Ok(TcpListener::Compio(compio::net::TcpListener::from_std(
            listener,
        )?))
    }

    pub(crate) async fn tcp_accept(listener: &TcpListener) -> io::Result<(TcpStream, SocketAddr)> {
        let listener = match listener {
            TcpListener::Compio(listener) => listener,
        };
        let (stream, address) = listener.accept().await?;
        Ok((TcpStream::Compio(stream), address))
    }

    #[cfg(feature = "quic-quiche")]
    pub(crate) fn udp_socket_from_std(socket: StdUdpSocket) -> io::Result<UdpSocket> {
        Ok(UdpSocket::Compio(std::sync::Arc::new(
            compio::net::UdpSocket::from_std(socket)?,
        )))
    }

    #[cfg(feature = "quic-quiche")]
    pub(crate) fn udp_local_addr(socket: &UdpSocket) -> io::Result<SocketAddr> {
        let socket = match socket {
            UdpSocket::Compio(socket) => socket,
        };
        socket.local_addr()
    }

    #[cfg(feature = "quic-quiche")]
    pub(crate) fn receiver(socket: &UdpSocket) -> DatagramReceiver {
        let socket = match socket {
            UdpSocket::Compio(socket) => socket,
        };
        DatagramReceiver::Compio(CompioDatagramReceiver {
            socket: std::sync::Arc::clone(socket),
        })
    }

    #[cfg(feature = "quic-quiche")]
    pub(crate) async fn udp_send_to(
        socket: &UdpSocket,
        buffer: Vec<u8>,
        buffer_length: usize,
        address: SocketAddr,
    ) -> io::Result<(usize, Vec<u8>)> {
        if buffer_length > buffer.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "UDP datagram length exceeds its buffer",
            ));
        }
        let socket = match socket {
            UdpSocket::Compio(socket) => socket,
        };
        use compio::buf::{IntoInner, IoBuf};

        let compio::BufResult(result, buffer) =
            socket.send_to(buffer.slice(..buffer_length), address).await;
        result
            .map(|written| (written, buffer))
            .map(|(written, buffer)| (written, buffer.into_inner()))
    }
}

#[cfg(feature = "network-runtime-monoio")]
mod backend {
    use super::*;
    use monoio::{IoUringDriver, RuntimeBuilder};

    pub(crate) const NAME: &str = "monoio";
    pub(crate) const SUPPORTS_COMBINED_NETWORK_ROLE: bool = true;

    pub(crate) fn run<F>(config: RuntimeConfig, future: F) -> io::Result<F::Output>
    where
        F: Future + 'static,
    {
        let _ = config.event_interval;
        let _ = config.worker_index;
        let cpu_id = config
            .cpu_id
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Monoio requires a CPU"))?;
        crate::platform::pin_current_thread(cpu_id)?;
        let mut runtime = RuntimeBuilder::<IoUringDriver>::new()
            .with_entries(config.entries)
            .enable_timer()
            .build()?;
        Ok(runtime.block_on(future))
    }

    pub(crate) fn block_on<F>(future: F) -> io::Result<F::Output>
    where
        F: Future + 'static,
    {
        let mut runtime = RuntimeBuilder::<IoUringDriver>::new()
            .with_entries(1024)
            .enable_timer()
            .build()?;
        Ok(runtime.block_on(future))
    }

    pub(crate) fn spawn_detached<F>(future: F)
    where
        F: Future<Output = ()> + 'static,
    {
        drop(monoio::spawn(future));
    }

    pub(crate) async fn timeout<F>(duration: Duration, future: F) -> Result<F::Output, Timeout>
    where
        F: Future,
    {
        monoio::time::timeout(duration, future)
            .await
            .map_err(|_| Timeout)
    }

    pub(crate) async fn sleep(duration: Duration) {
        monoio::time::sleep(duration).await;
    }

    pub(crate) async fn shutdown_signal() {
        portable_shutdown_signal().await;
    }

    pub(crate) fn tcp_listener_from_std(listener: StdTcpListener) -> io::Result<TcpListener> {
        Ok(TcpListener::Monoio(monoio::net::TcpListener::from_std(
            listener,
        )?))
    }

    pub(crate) async fn tcp_accept(listener: &TcpListener) -> io::Result<(TcpStream, SocketAddr)> {
        let listener = match listener {
            TcpListener::Monoio(listener) => listener,
        };
        let (stream, address) = listener.accept().await?;
        Ok((TcpStream::Monoio(stream), address))
    }

    #[cfg(feature = "quic-quiche")]
    pub(crate) fn udp_socket_from_std(socket: StdUdpSocket) -> io::Result<UdpSocket> {
        Ok(UdpSocket::Monoio(std::sync::Arc::new(
            monoio::net::udp::UdpSocket::from_std(socket)?,
        )))
    }

    #[cfg(feature = "quic-quiche")]
    pub(crate) fn udp_local_addr(socket: &UdpSocket) -> io::Result<SocketAddr> {
        let socket = match socket {
            UdpSocket::Monoio(socket) => socket,
        };
        socket.local_addr()
    }

    #[cfg(feature = "quic-quiche")]
    pub(crate) fn receiver(socket: &UdpSocket) -> DatagramReceiver {
        DatagramReceiver::Generic(match socket {
            UdpSocket::Monoio(socket) => UdpSocket::Monoio(socket.clone()),
        })
    }

    #[cfg(feature = "quic-quiche")]
    pub(crate) async fn udp_recv_from(
        socket: &UdpSocket,
        buffer: Vec<u8>,
    ) -> io::Result<(usize, SocketAddr, Vec<u8>)> {
        let socket = match socket {
            UdpSocket::Monoio(socket) => socket,
        };
        let (result, buffer) = socket.recv_from(buffer).await;
        result.map(|(read, address)| (read, address, buffer))
    }

    #[cfg(feature = "quic-quiche")]
    pub(crate) async fn udp_send_to(
        socket: &UdpSocket,
        buffer: Vec<u8>,
        length: usize,
        address: SocketAddr,
    ) -> io::Result<(usize, Vec<u8>)> {
        if length > buffer.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "UDP datagram length exceeds its buffer",
            ));
        }
        let socket = match socket {
            UdpSocket::Monoio(socket) => socket,
        };
        let mut buffer = buffer;
        buffer.truncate(length);
        let (result, buffer) = socket.send_to(buffer, address).await;
        result.map(|written| (written, buffer))
    }
}

#[cfg(feature = "network-runtime-glommio")]
mod backend {
    use super::*;
    use futures_util::FutureExt;
    use std::os::fd::{FromRawFd, IntoRawFd};

    pub(crate) const NAME: &str = "glommio";
    pub(crate) const SUPPORTS_COMBINED_NETWORK_ROLE: bool = true;

    pub(crate) fn run<F>(config: RuntimeConfig, future: F) -> io::Result<F::Output>
    where
        F: Future + 'static,
    {
        let _ = config.worker_index;
        let cpu_id = config
            .cpu_id
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Glommio requires a CPU"))?;
        let executor = glommio::LocalExecutorBuilder::new(glommio::Placement::Fixed(cpu_id))
            .ring_depth(config.entries as usize)
            .preempt_timer(Duration::from_millis(
                u64::try_from(config.event_interval.max(1)).unwrap_or(u64::MAX),
            ))
            .make()
            .map_err(|error| io::Error::other(error.to_string()))?;
        Ok(executor.run(future))
    }

    pub(crate) fn block_on<F>(future: F) -> io::Result<F::Output>
    where
        F: Future + 'static,
    {
        let executor = glommio::LocalExecutorBuilder::new(glommio::Placement::Unbound)
            .make()
            .map_err(|error| io::Error::other(error.to_string()))?;
        Ok(executor.run(future))
    }

    pub(crate) fn spawn_detached<F>(future: F)
    where
        F: Future<Output = ()> + 'static,
    {
        glommio::spawn_local(future).detach();
    }

    pub(crate) async fn timeout<F>(duration: Duration, future: F) -> Result<F::Output, Timeout>
    where
        F: Future,
    {
        futures_util::pin_mut!(future);
        let timer = glommio::timer::Timer::new(duration);
        futures_util::pin_mut!(timer);
        futures_util::select! {
            output = future.fuse() => Ok(output),
            _ = timer.fuse() => Err(Timeout),
        }
    }

    pub(crate) async fn sleep(duration: Duration) {
        glommio::timer::Timer::new(duration).await;
    }

    pub(crate) async fn shutdown_signal() {
        portable_shutdown_signal().await;
    }

    pub(crate) fn tcp_listener_from_std(listener: StdTcpListener) -> io::Result<TcpListener> {
        let fd = listener.into_raw_fd();
        // SAFETY: `fd` is owned by the returned Glommio listener.
        Ok(TcpListener::Glommio(unsafe {
            glommio::net::TcpListener::from_raw_fd(fd)
        }))
    }

    pub(crate) async fn tcp_accept(listener: &TcpListener) -> io::Result<(TcpStream, SocketAddr)> {
        let listener = match listener {
            TcpListener::Glommio(listener) => listener,
        };
        let stream = listener
            .accept()
            .await
            .map_err(|error| io::Error::other(error.to_string()))?;
        let address = stream
            .peer_addr()
            .map_err(|error| io::Error::other(error.to_string()))?;
        Ok((TcpStream::Glommio(stream), address))
    }

    #[cfg(feature = "quic-quiche")]
    pub(crate) fn udp_socket_from_std(socket: StdUdpSocket) -> io::Result<UdpSocket> {
        let fd = socket.into_raw_fd();
        // SAFETY: `fd` is owned by the returned Glommio socket.
        Ok(UdpSocket::Glommio(std::sync::Arc::new(unsafe {
            glommio::net::UdpSocket::from_raw_fd(fd)
        })))
    }

    #[cfg(feature = "quic-quiche")]
    pub(crate) fn udp_local_addr(socket: &UdpSocket) -> io::Result<SocketAddr> {
        let socket = match socket {
            UdpSocket::Glommio(socket) => socket,
        };
        socket
            .local_addr()
            .map_err(|error| io::Error::other(error.to_string()))
    }

    #[cfg(feature = "quic-quiche")]
    pub(crate) fn receiver(socket: &UdpSocket) -> DatagramReceiver {
        DatagramReceiver::Generic(UdpSocket::Glommio(std::sync::Arc::clone(match socket {
            UdpSocket::Glommio(socket) => socket,
        })))
    }

    #[cfg(feature = "quic-quiche")]
    pub(crate) async fn udp_recv_from(
        socket: &UdpSocket,
        mut buffer: Vec<u8>,
    ) -> io::Result<(usize, SocketAddr, Vec<u8>)> {
        let socket = match socket {
            UdpSocket::Glommio(socket) => socket,
        };
        let (read, address) = socket
            .recv_from(&mut buffer)
            .await
            .map_err(|error| io::Error::other(error.to_string()))?;
        Ok((read, address, buffer))
    }

    #[cfg(feature = "quic-quiche")]
    pub(crate) async fn udp_send_to(
        socket: &UdpSocket,
        buffer: Vec<u8>,
        length: usize,
        address: SocketAddr,
    ) -> io::Result<(usize, Vec<u8>)> {
        if length > buffer.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "UDP datagram length exceeds its buffer",
            ));
        }
        let socket = match socket {
            UdpSocket::Glommio(socket) => socket,
        };
        let mut buffer = buffer;
        buffer.truncate(length);
        let written = socket
            .send_to(&buffer, address)
            .await
            .map_err(|error| io::Error::other(error.to_string()))?;
        Ok((written, buffer))
    }
}

#[cfg(feature = "network-runtime-kimojio")]
mod backend {
    use super::*;
    use std::mem::{size_of, zeroed};
    use std::os::fd::{FromRawFd, IntoRawFd};

    use kimojio::operations::{self, RecvFlags, SendFlags};
    use rustix::io_uring::{MsgHdr, SocketAddrLen, iovec};

    pub(crate) const NAME: &str = "kimojio";
    pub(crate) const SUPPORTS_COMBINED_NETWORK_ROLE: bool = true;

    pub(crate) fn run<F>(config: RuntimeConfig, future: F) -> io::Result<F::Output>
    where
        F: Future + 'static,
    {
        let _ = (config.entries, config.event_interval);
        let cpu_id = config
            .cpu_id
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Kimojio requires a CPU"))?;
        crate::platform::pin_current_thread(cpu_id)?;
        let worker_index = config.worker_index.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "Kimojio requires a stable network worker index",
            )
        })?;
        let worker_index = u8::try_from(worker_index).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "Kimojio supports at most 256 network workers",
            )
        })?;
        let configuration = kimojio::configuration::Configuration::new();
        let mut runtime = kimojio::Runtime::new(worker_index, configuration);
        match runtime.block_on(future) {
            Some(Ok(output)) => Ok(output),
            Some(Err(_)) => Err(io::Error::other("Kimojio network task panicked")),
            None => Err(io::Error::other(
                "Kimojio network runtime shut down before its task completed",
            )),
        }
    }

    pub(crate) fn block_on<F>(future: F) -> io::Result<F::Output>
    where
        F: Future + 'static,
    {
        let mut runtime = kimojio::Runtime::new(0, kimojio::configuration::Configuration::new());
        match runtime.block_on(future) {
            Some(Ok(output)) => Ok(output),
            Some(Err(_)) => Err(io::Error::other("Kimojio network task panicked")),
            None => Err(io::Error::other(
                "Kimojio network runtime shut down before its task completed",
            )),
        }
    }

    pub(crate) fn spawn_detached<F>(future: F)
    where
        F: Future<Output = ()> + 'static,
    {
        drop(operations::spawn_task(future));
    }

    pub(crate) async fn timeout<F>(duration: Duration, future: F) -> Result<F::Output, Timeout>
    where
        F: Future,
    {
        operations::timeout_at(std::time::Instant::now() + duration, future)
            .await
            .map_err(|_| Timeout)
    }

    pub(crate) async fn sleep(duration: Duration) {
        let _ = operations::sleep(duration).await;
    }

    pub(crate) async fn shutdown_signal() {
        portable_shutdown_signal().await;
    }

    pub(crate) fn tcp_listener_from_std(listener: StdTcpListener) -> io::Result<TcpListener> {
        let fd = listener.into_raw_fd();
        // SAFETY: `fd` is owned by the returned Kimojio listener.
        Ok(TcpListener::Kimojio(KimojioTcpListener {
            fd: unsafe { kimojio::OwnedFd::from_raw_fd(fd) },
        }))
    }

    pub(crate) async fn tcp_accept(listener: &TcpListener) -> io::Result<(TcpStream, SocketAddr)> {
        let listener = match listener {
            TcpListener::Kimojio(listener) => listener,
        };
        let fd = operations::accept(&listener.fd)
            .await
            .map_err(errno_to_io)?;
        let address = peer_addr(fd.as_raw_fd())?;
        Ok((TcpStream::Kimojio(KimojioTcpStream { fd }), address))
    }

    #[cfg(feature = "quic-quiche")]
    pub(crate) fn udp_socket_from_std(socket: StdUdpSocket) -> io::Result<UdpSocket> {
        let local_addr = socket.local_addr()?;
        let fd = socket.into_raw_fd();
        // SAFETY: `fd` is owned by the returned Kimojio socket.
        Ok(UdpSocket::Kimojio(std::sync::Arc::new(KimojioUdpSocket {
            fd: unsafe { kimojio::OwnedFd::from_raw_fd(fd) },
            local_addr,
        })))
    }

    #[cfg(feature = "quic-quiche")]
    pub(crate) fn udp_local_addr(socket: &UdpSocket) -> io::Result<SocketAddr> {
        let socket = match socket {
            UdpSocket::Kimojio(socket) => socket,
        };
        Ok(socket.local_addr)
    }

    #[cfg(feature = "quic-quiche")]
    pub(crate) fn receiver(socket: &UdpSocket) -> DatagramReceiver {
        DatagramReceiver::Generic(UdpSocket::Kimojio(std::sync::Arc::clone(match socket {
            UdpSocket::Kimojio(socket) => socket,
        })))
    }

    #[cfg(feature = "quic-quiche")]
    pub(crate) async fn udp_recv_from(
        socket: &UdpSocket,
        mut buffer: Vec<u8>,
    ) -> io::Result<(usize, SocketAddr, Vec<u8>)> {
        let socket = match socket {
            UdpSocket::Kimojio(socket) => socket,
        };
        let mut storage = unsafe { zeroed::<libc::sockaddr_storage>() };
        let mut iovec = iovec {
            iov_base: buffer.as_mut_ptr().cast(),
            iov_len: buffer.len(),
        };
        let mut header = MsgHdr {
            msg_name: (&mut storage as *mut libc::sockaddr_storage).cast(),
            msg_namelen: size_of::<libc::sockaddr_storage>() as SocketAddrLen,
            msg_iov: &mut iovec,
            msg_iovlen: 1,
            msg_control: std::ptr::null_mut(),
            msg_controllen: 0,
            msg_flags: RecvFlags::empty(),
        };
        let read = operations::recvmsg(&socket.fd, &mut header, RecvFlags::empty(), None)
            .await
            .map_err(errno_to_io)?;
        let address = storage_to_socket_addr(&storage, header.msg_namelen)?;
        Ok((read, address, buffer))
    }

    #[cfg(feature = "quic-quiche")]
    pub(crate) async fn udp_send_to(
        socket: &UdpSocket,
        buffer: Vec<u8>,
        length: usize,
        address: SocketAddr,
    ) -> io::Result<(usize, Vec<u8>)> {
        if length > buffer.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "UDP datagram length exceeds its buffer",
            ));
        }
        let socket = match socket {
            UdpSocket::Kimojio(socket) => socket,
        };
        let (mut storage, address_length) = socket_addr_to_storage(address);
        let mut iovec = iovec {
            iov_base: buffer.as_ptr() as *mut _,
            iov_len: length,
        };
        let mut header = MsgHdr {
            msg_name: (&mut storage as *mut libc::sockaddr_storage).cast(),
            msg_namelen: address_length,
            msg_iov: &mut iovec,
            msg_iovlen: 1,
            msg_control: std::ptr::null_mut(),
            msg_controllen: 0,
            msg_flags: RecvFlags::empty(),
        };
        let written = operations::sendmsg(&socket.fd, &mut header, SendFlags::empty(), None)
            .await
            .map_err(errno_to_io)?;
        Ok((written, buffer))
    }

    pub(crate) struct KimojioTcpStream {
        fd: kimojio::OwnedFd,
    }

    impl KimojioTcpStream {
        pub(super) fn set_nodelay(&self, nodelay: bool) -> io::Result<()> {
            set_socket_bool(self.fd.as_raw_fd(), libc::TCP_NODELAY, nodelay)
        }

        pub(super) async fn read(&self, mut buffer: Vec<u8>) -> io::Result<(usize, Vec<u8>)> {
            let read = operations::read(&self.fd, &mut buffer)
                .await
                .map_err(errno_to_io)?;
            Ok((read, buffer))
        }

        pub(super) async fn write_all(&self, buffer: Vec<u8>) -> io::Result<Vec<u8>> {
            let mut written = 0;
            while written < buffer.len() {
                let count = operations::write(&self.fd, &buffer[written..])
                    .await
                    .map_err(errno_to_io)?;
                if count == 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "Kimojio TCP write returned zero",
                    ));
                }
                written += count;
            }
            Ok(buffer)
        }
    }

    pub(crate) struct KimojioTcpListener {
        fd: kimojio::OwnedFd,
    }

    pub(crate) struct KimojioUdpSocket {
        fd: kimojio::OwnedFd,
        local_addr: SocketAddr,
    }

    fn errno_to_io(error: kimojio::Errno) -> io::Error {
        io::Error::from_raw_os_error(error.raw_os_error())
    }

    fn set_socket_bool(fd: i32, option: i32, value: bool) -> io::Result<()> {
        let value: libc::c_int = i32::from(value);
        // SAFETY: `fd` is a live socket and the pointer/length describe `value`.
        let result = unsafe {
            libc::setsockopt(
                fd,
                libc::IPPROTO_TCP,
                option,
                (&value as *const libc::c_int).cast(),
                size_of::<libc::c_int>() as libc::socklen_t,
            )
        };
        if result == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    fn peer_addr(fd: i32) -> io::Result<SocketAddr> {
        let mut storage = unsafe { zeroed::<libc::sockaddr_storage>() };
        let mut length = size_of::<libc::sockaddr_storage>() as libc::socklen_t;
        // SAFETY: `fd` is a connected socket and the output buffer is valid.
        let result = unsafe {
            libc::getpeername(
                fd,
                (&mut storage as *mut libc::sockaddr_storage).cast(),
                &mut length,
            )
        };
        if result != 0 {
            return Err(io::Error::last_os_error());
        }
        storage_to_socket_addr(&storage, length as SocketAddrLen)
    }

    fn socket_addr_to_storage(address: SocketAddr) -> (libc::sockaddr_storage, SocketAddrLen) {
        let mut storage = unsafe { zeroed::<libc::sockaddr_storage>() };
        match address {
            SocketAddr::V4(address) => {
                let value = libc::sockaddr_in {
                    sin_family: libc::AF_INET as libc::sa_family_t,
                    sin_port: address.port().to_be(),
                    sin_addr: libc::in_addr {
                        s_addr: u32::from_ne_bytes(address.ip().octets()),
                    },
                    sin_zero: [0; 8],
                };
                // SAFETY: sockaddr_storage is large enough for sockaddr_in.
                unsafe {
                    std::ptr::write((&mut storage as *mut libc::sockaddr_storage).cast(), value);
                }
                (storage, size_of::<libc::sockaddr_in>() as SocketAddrLen)
            }
            SocketAddr::V6(address) => {
                let value = libc::sockaddr_in6 {
                    sin6_family: libc::AF_INET6 as libc::sa_family_t,
                    sin6_port: address.port().to_be(),
                    sin6_flowinfo: address.flowinfo(),
                    sin6_addr: libc::in6_addr {
                        s6_addr: address.ip().octets(),
                    },
                    sin6_scope_id: address.scope_id(),
                };
                // SAFETY: sockaddr_storage is large enough for sockaddr_in6.
                unsafe {
                    std::ptr::write((&mut storage as *mut libc::sockaddr_storage).cast(), value);
                }
                (storage, size_of::<libc::sockaddr_in6>() as SocketAddrLen)
            }
        }
    }

    fn storage_to_socket_addr(
        storage: &libc::sockaddr_storage,
        length: SocketAddrLen,
    ) -> io::Result<SocketAddr> {
        let length = usize::try_from(length).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "socket address length does not fit in usize",
            )
        })?;
        if length < size_of::<libc::sa_family_t>() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "socket address length is too small",
            ));
        }
        match i32::from(storage.ss_family) {
            libc::AF_INET => {
                if length < size_of::<libc::sockaddr_in>() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "IPv4 socket address is truncated",
                    ));
                }
                // SAFETY: family and length identify sockaddr_in.
                let address = unsafe {
                    *(storage as *const libc::sockaddr_storage as *const libc::sockaddr_in)
                };
                Ok(SocketAddr::from(std::net::SocketAddrV4::new(
                    std::net::Ipv4Addr::from(address.sin_addr.s_addr.to_ne_bytes()),
                    u16::from_be(address.sin_port),
                )))
            }
            libc::AF_INET6 => {
                if length < size_of::<libc::sockaddr_in6>() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "IPv6 socket address is truncated",
                    ));
                }
                // SAFETY: family and length identify sockaddr_in6.
                let address = unsafe {
                    *(storage as *const libc::sockaddr_storage as *const libc::sockaddr_in6)
                };
                Ok(SocketAddr::from(std::net::SocketAddrV6::new(
                    std::net::Ipv6Addr::from(address.sin6_addr.s6_addr),
                    u16::from_be(address.sin6_port),
                    address.sin6_flowinfo,
                    address.sin6_scope_id,
                )))
            }
            family => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unsupported socket address family {family}"),
            )),
        }
    }
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

#[cfg(feature = "network-runtime-kimojio")]
use backend::{KimojioTcpListener, KimojioTcpStream, KimojioUdpSocket};
