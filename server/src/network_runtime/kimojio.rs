use super::*;
use std::mem::{size_of, zeroed};
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd};

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
            let address =
                unsafe { *(storage as *const libc::sockaddr_storage as *const libc::sockaddr_in) };
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
            let address =
                unsafe { *(storage as *const libc::sockaddr_storage as *const libc::sockaddr_in6) };
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
