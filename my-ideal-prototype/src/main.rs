#[cfg(all(feature = "alloc-mimalloc", feature = "alloc-jemalloc"))]
compile_error!("allocator feature는 하나만 선택해야 합니다");

#[cfg(all(feature = "alloc-mimalloc", not(feature = "alloc-jemalloc")))]
#[global_allocator]
static GLOBAL_ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[cfg(all(feature = "alloc-jemalloc", not(feature = "alloc-mimalloc")))]
#[global_allocator]
static GLOBAL_ALLOCATOR: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

mod client;
mod aa;
mod compio_example;
mod network;
mod resp;
mod resp_proxy;
mod spsc;
mod storage;
mod storage_example;
mod storage_message;

use std::net::TcpListener;
use std::{env, io, mem, thread};

use storage_message::{STORAGE_QUEUE_SLOTS, StorageRequest, StorageResponse};
// 네트워크 만들고
// storage 만들고
// 네트워크랑 storage 사이에 spsc 만들고

// 이 아래는 network.run()
// sqe - > accept 새로 client
// sqe 읽은거 보고서 차례대로 하나식 resp 파싱 -> box 로 key or value 파싱 -> request -> spsc 에  push 하기
// 어떤 조건이 맞춰지면 그만 읽고 (ex 더 읽을게 없음. or 아무거나 response spsc 꽉참 등등)
// 그때 response spsc -> client vecdeque 에 넣어주기. (여기 모두 zero copy) + value 를 Arc 로 할지 말지 고민. 우선은 Arc로..
// sqe 에 새로 내보낼거 write_state client vecdeque +  새로 read 할거 보내야 하는데 bucket write_pos 부터로 잡고 만약 꽉찼으면 마지막 read_pos 까지 shift

fn pin_current_thread(cpu: usize) -> io::Result<()> {
    let mut cpu_set: libc::cpu_set_t = unsafe { mem::zeroed() };

    unsafe {
        libc::CPU_ZERO(&mut cpu_set);
        libc::CPU_SET(cpu, &mut cpu_set);
    }

    let result = unsafe {
        libc::pthread_setaffinity_np(
            libc::pthread_self(),
            mem::size_of::<libc::cpu_set_t>(),
            &cpu_set,
        )
    };

    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(result))
    }
}

fn parse_cpu(value: Option<String>, default: usize, role: &str) -> io::Result<usize> {
    value.map_or(Ok(default), |value| {
        value.parse().map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("invalid {role} CPU: {value}"),
            )
        })
    })
}

fn main() -> io::Result<()> {
    let mut arguments = env::args().skip(1);
    let address = arguments
        .next()
        .unwrap_or_else(|| "127.0.0.1:80".to_owned());
    let network_cpu = parse_cpu(arguments.next(), 0, "network")?;
    let storage_cpu = parse_cpu(arguments.next(), 1, "storage")?;

    if arguments.next().is_some() || network_cpu == storage_cpu {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "usage: my-ideal-prototype [address] [network-cpu storage-cpu]",
        ));
    }

    let tcp_listener = TcpListener::bind(&address)?;
    let tcp_address = tcp_listener.local_addr()?;
    let udp_socket = std::net::UdpSocket::bind(tcp_address)?;
    let _resp_proxy_thread = resp_proxy::spawn(udp_socket, resp_backend_address(tcp_address))?;
    let (request_producer, request_consumer) =
        spsc::channel::<StorageRequest, STORAGE_QUEUE_SLOTS>();
    let (response_producer, response_consumer) =
        spsc::channel::<StorageResponse, STORAGE_QUEUE_SLOTS>();

    let _storage_thread =
        thread::Builder::new()
            .name("storage".into())
            .spawn(move || -> io::Result<()> {
                pin_current_thread(storage_cpu)?;
                storage::run(request_consumer, response_producer)
            })?;

    pin_current_thread(network_cpu)?;

    eprintln!(
        "my-ideal-prototype listening on {tcp_address} over RESP/TCP and native QUIC/UDP; network CPU={network_cpu}, storage CPU={storage_cpu}"
    );

    let mut network = network::Network::new(tcp_listener, request_producer, response_consumer)?;
    network.run()
}

fn resp_backend_address(public_address: std::net::SocketAddr) -> std::net::SocketAddr {
    if public_address.ip().is_unspecified() {
        std::net::SocketAddr::new(
            if public_address.is_ipv4() {
                std::net::Ipv4Addr::LOCALHOST.into()
            } else {
                std::net::Ipv6Addr::LOCALHOST.into()
            },
            public_address.port(),
        )
    } else {
        public_address
    }
}
