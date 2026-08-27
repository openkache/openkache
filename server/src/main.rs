#[cfg(all(feature = "alloc-mimalloc", feature = "alloc-jemalloc"))]
compile_error!("only one allocator feature may be enabled");

#[cfg(all(feature = "alloc-mimalloc", not(feature = "alloc-jemalloc")))]
#[global_allocator]
static GLOBAL_ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[cfg(all(feature = "alloc-jemalloc", not(feature = "alloc-mimalloc")))]
#[global_allocator]
static GLOBAL_ALLOCATOR: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

mod client;
mod config;
mod network;
mod resp;
mod resp_proxy;
mod spsc;
mod storage;
mod storage_message;

use std::net::TcpListener;
use std::path::PathBuf;
use std::{env, io, thread};

#[cfg(target_os = "linux")]
use std::mem;

use config::{Config, StorageConfig};
use storage_message::{STORAGE_QUEUE_SLOTS, StorageRequest, StorageResponse};
// Create the network layer.
// Create storage.
// Create SPSC queues between the network and storage layers.

// Everything below is handled by network.run().
// An accept SQE creates a new client.
// Parse each completed read SQE as RESP, box the key or value, create a request,
// and push it to the SPSC queue.
// Stop reading when no input remains or a response SPSC queue is full.
// Move responses from the SPSC queue into the client's VecDeque without copying.
// Values currently use Arc-backed ownership.
// Submit queued writes and reads, starting at the buffer write position and
// shifting up to the last read position when the buffer is full.

#[cfg(target_os = "linux")]
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

#[cfg(target_os = "linux")]
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

fn usage_error() -> io::Error {
    #[cfg(target_os = "linux")]
    let usage = "usage: openkache-server [--config <path>] [address] [network-cpu storage-cpu]";
    #[cfg(target_os = "macos")]
    let usage = "usage: openkache-server [--config <path>] [address]";
    io::Error::new(io::ErrorKind::InvalidInput, usage)
}

#[derive(Clone, Copy)]
enum ThreadPlacement {
    #[cfg(target_os = "linux")]
    Pinned {
        network_cpu: usize,
        storage_cpu: usize,
    },
    #[cfg(target_os = "macos")]
    Scheduler,
}

struct ServerConfig {
    address: String,
    storage: StorageConfig,
    placement: ThreadPlacement,
}

impl ServerConfig {
    fn parse(mut arguments: impl Iterator<Item = String>) -> io::Result<Self> {
        let mut config_path: Option<PathBuf> = None;
        let mut positional = Vec::new();
        while let Some(argument) = arguments.next() {
            if argument == "--config" {
                let path = arguments.next().ok_or_else(usage_error)?;
                config_path = Some(PathBuf::from(path));
            } else {
                positional.push(argument);
            }
        }

        let storage = Config::load(config_path.as_deref())?;
        let mut positional = positional.into_iter();
        let address = positional
            .next()
            .unwrap_or_else(|| "127.0.0.1:4433".to_owned());

        Ok(Self {
            address,
            storage,
            placement: ThreadPlacement::parse(&mut positional)?,
        })
    }

    fn spawn_storage_thread(
        &self,
        request_consumer: spsc::Consumer<StorageRequest, STORAGE_QUEUE_SLOTS>,
        response_producer: spsc::Producer<StorageResponse, STORAGE_QUEUE_SLOTS>,
    ) -> io::Result<thread::JoinHandle<io::Result<()>>> {
        self.placement.spawn_storage_thread(
            self.storage.clone(),
            request_consumer,
            response_producer,
        )
    }

    fn log_listening(&self, tcp_address: std::net::SocketAddr) {
        self.placement.log_listening(tcp_address);
    }

    fn run_network(self, mut network: network::Network) -> io::Result<()> {
        self.placement.run_network(&mut network)
    }
}

impl ThreadPlacement {
    #[cfg(target_os = "linux")]
    fn parse(arguments: &mut impl Iterator<Item = String>) -> io::Result<Self> {
        let network_cpu = parse_cpu(arguments.next(), 0, "network")?;
        let storage_cpu = parse_cpu(arguments.next(), 1, "storage")?;

        if arguments.next().is_some() || network_cpu == storage_cpu {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "usage: openkache-server [address] [network-cpu storage-cpu]",
            ));
        }

        Ok(Self::Pinned {
            network_cpu,
            storage_cpu,
        })
    }

    #[cfg(target_os = "macos")]
    fn parse(arguments: &mut impl Iterator<Item = String>) -> io::Result<Self> {
        if arguments.next().is_some() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "usage: openkache-server [address]",
            ));
        }

        Ok(Self::Scheduler)
    }

    #[cfg(target_os = "linux")]
    fn spawn_storage_thread(
        self,
        config: StorageConfig,
        request_consumer: spsc::Consumer<StorageRequest, STORAGE_QUEUE_SLOTS>,
        response_producer: spsc::Producer<StorageResponse, STORAGE_QUEUE_SLOTS>,
    ) -> io::Result<thread::JoinHandle<io::Result<()>>> {
        let Self::Pinned { storage_cpu, .. } = self;

        thread::Builder::new()
            .name("storage".into())
            .spawn(move || {
                pin_current_thread(storage_cpu)?;
                storage::run(config, request_consumer, response_producer)
            })
    }

    #[cfg(target_os = "macos")]
    fn spawn_storage_thread(
        self,
        config: StorageConfig,
        request_consumer: spsc::Consumer<StorageRequest, STORAGE_QUEUE_SLOTS>,
        response_producer: spsc::Producer<StorageResponse, STORAGE_QUEUE_SLOTS>,
    ) -> io::Result<thread::JoinHandle<io::Result<()>>> {
        let Self::Scheduler = self;

        thread::Builder::new()
            .name("storage".into())
            .spawn(move || storage::run(config, request_consumer, response_producer))
    }

    #[cfg(target_os = "linux")]
    fn run_network(self, network: &mut network::Network) -> io::Result<()> {
        let Self::Pinned { network_cpu, .. } = self;
        pin_current_thread(network_cpu)?;
        network.run()
    }

    #[cfg(target_os = "macos")]
    fn run_network(self, network: &mut network::Network) -> io::Result<()> {
        let Self::Scheduler = self;
        network.run()
    }

    #[cfg(target_os = "linux")]
    fn log_listening(self, tcp_address: std::net::SocketAddr) {
        let Self::Pinned {
            network_cpu,
            storage_cpu,
        } = self;
        eprintln!(
            "openkache-server listening on {tcp_address} over RESP/TCP and native QUIC/UDP; network CPU={network_cpu}, storage CPU={storage_cpu}"
        );
    }

    #[cfg(target_os = "macos")]
    fn log_listening(self, tcp_address: std::net::SocketAddr) {
        let Self::Scheduler = self;
        eprintln!(
            "openkache-server listening on {tcp_address} over RESP/TCP and native QUIC/UDP; thread placement delegated to the macOS scheduler"
        );
    }
}

fn main() -> io::Result<()> {
    let config = ServerConfig::parse(env::args().skip(1))?;

    network::install_signal_handlers()?;

    let tcp_listener = TcpListener::bind(&config.address)?;
    let tcp_address = tcp_listener.local_addr()?;
    let udp_socket = std::net::UdpSocket::bind(tcp_address)?;
    let _resp_proxy_thread = resp_proxy::spawn(udp_socket, resp_backend_address(tcp_address))?;
    let (request_producer, request_consumer) =
        spsc::channel::<StorageRequest, STORAGE_QUEUE_SLOTS>();
    let (response_producer, response_consumer) =
        spsc::channel::<StorageResponse, STORAGE_QUEUE_SLOTS>();

    let _storage_thread = config.spawn_storage_thread(request_consumer, response_producer)?;
    config.log_listening(tcp_address);

    let network = network::Network::new(tcp_listener, request_producer, response_consumer)?;
    config.run_network(network)
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
