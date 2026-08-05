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
