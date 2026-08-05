//! Command-line entry point for the SSD-backed OpenKache QUIC server.

use std::{io, net::SocketAddr, path::PathBuf};

use clap::Parser;
use openkache::{platform, AppConfig, QuicBackend};

const DEFAULT_PORT: u16 = 4433;

#[path = "openkache_server/allocator.rs"]
mod allocator;
#[path = "openkache_server/pki.rs"]
mod pki;
#[path = "openkache_server/security.rs"]
mod security;
#[path = "openkache_server/serve.rs"]
mod serve;
#[path = "openkache_server/sizing.rs"]
mod sizing;

/// Command-line arguments controlling the network endpoint and cache configuration.
#[derive(Parser)]
#[command(name = "openkache-server")]
struct Arguments {
    #[command(subcommand)]
    command: Option<pki::Command>,

    /// UDP address on which the QUIC endpoint listens.
    #[arg(long, value_name = "ADDRESS", conflicts_with = "port")]
    listen: Option<SocketAddr>,

    /// UDP port on localhost; defaults to 4433.
    #[arg(long, value_name = "PORT", conflicts_with = "listen")]
    port: Option<u16>,

    /// Client protocol accepted by this server process.
    #[arg(long, value_enum, default_value_t = serve::Protocol::Quic)]
    protocol: serve::Protocol,

    #[command(flatten)]
    security: security::SecurityArguments,

    /// Optional TOML cache configuration file.
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,

    /// QUIC protocol implementation, overriding the configuration file.
    #[arg(long, value_enum)]
    quic_backend: Option<QuicBackend>,

    #[command(flatten)]
    sizing: sizing::SizingArguments,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments = Arguments::parse();
    if let Some(command) = &arguments.command {
        return Ok(command.run()?);
    }
    let sizing_plan = arguments.sizing.build_plan(arguments.config.is_some())?;
    if arguments.sizing.plan_only() {
        sizing::print_plan(
            sizing_plan
                .as_ref()
                .expect("clap requires sizing arguments with --plan"),
        );
        return Ok(());
    }
    let mut config = match &sizing_plan {
        Some(plan) => plan.config.clone(),
        None => load_config(arguments.config.as_deref())?,
    };
    if let Some(backend) = arguments.quic_backend {
        config.quic.backend = Some(backend);
    }
    arguments.security.apply(&mut config)?;
    if let Some(plan) = &sizing_plan {
        sizing::print_plan(plan);
    }
    let runtime = compio::runtime::Runtime::new().map_err(runtime_initialization_error)?;
    require_native_driver(runtime.driver_type())?;
    report_storage_device(&config.storage.directory);
    runtime.block_on(serve::run(arguments, config))
}

#[cfg(target_os = "linux")]
fn require_native_driver(driver: compio::driver::DriverType) -> std::io::Result<()> {
    if driver.is_iouring() {
        Ok(())
    } else {
        Err(std::io::Error::other(
            "openkache-server cannot start: the selected network runtime fell back \
             to polling because Linux io_uring could not be initialized. Required syscalls: \
             io_uring_setup, io_uring_enter, io_uring_register. Check the \
             container seccomp policy and /proc/sys/kernel/io_uring_disabled \
             (0 enables io_uring).",
        ))
    }
}

#[cfg(target_os = "macos")]
fn require_native_driver(driver: compio::driver::DriverType) -> std::io::Result<()> {
    if driver.is_polling() {
        Ok(())
    } else {
        Err(std::io::Error::other(
            "openkache-server requires the selected network runtime's polling driver \
             on Apple Silicon macOS",
        ))
    }
}

#[cfg(target_os = "linux")]
fn runtime_initialization_error(error: io::Error) -> io::Error {
    let cause = match error.raw_os_error() {
        Some(libc::ENOSYS) => {
            "the kernel may not provide io_uring, or a container seccomp profile may \
             be denying the io_uring syscalls"
        }
        Some(libc::EPERM | libc::EACCES) => {
            "the kernel or container security policy denied io_uring; check seccomp \
             and io_uring permissions"
        }
        _ => "the kernel or container security policy rejected io_uring initialization",
    };
    io::Error::new(
        error.kind(),
        format!(
            "openkache-server cannot start: the selected network runtime requires \
             Linux io_uring ({error}); {cause}. \
             Required syscalls: io_uring_setup, io_uring_enter, io_uring_register. \
             Also check /proc/sys/kernel/io_uring_disabled (0 enables io_uring)."
        ),
    )
}

#[cfg(target_os = "macos")]
fn runtime_initialization_error(error: io::Error) -> io::Error {
    io::Error::new(
        error.kind(),
        format!(
            "openkache-server cannot start: the selected network runtime requires \
             the polling driver on Apple Silicon macOS ({error})"
        ),
    )
}

fn report_storage_device(path: &std::path::Path) {
    match platform::storage_device_kind(path) {
        platform::StorageDeviceKind::Nvme => {
            println!("Storage device: NVMe (detected)");
        }
        platform::StorageDeviceKind::NonNvme => {
            eprintln!(
                "WARNING: storage directory {} is on a non-NVMe block device. \
                 OpenKache will continue, but NVMe SSD is the intended production \
                 medium for predictable latency.",
                path.display()
            );
        }
        platform::StorageDeviceKind::Unknown => {
            eprintln!(
                "WARNING: could not verify that storage directory {} is on NVMe. \
                 OpenKache will continue, but NVMe SSD is the intended production \
                 medium for predictable latency.",
                path.display()
            );
        }
    }
}

/// Loads the cache configuration from TOML or returns the default configuration.
fn load_config(path: Option<&std::path::Path>) -> Result<AppConfig, Box<dyn std::error::Error>> {
    let Some(path) = path else {
        return Ok(AppConfig::default());
    };
    let text = std::fs::read_to_string(path)?;
    Ok(toml::from_str(&text)?)
}
