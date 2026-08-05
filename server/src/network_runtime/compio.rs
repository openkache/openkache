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
