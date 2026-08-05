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
