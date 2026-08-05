//! Command-line entry point for the SSD-backed OpenKache QUIC server.

use std::{io, net::SocketAddr, path::PathBuf};

use clap::Parser;
use openkache::{AppConfig, QuicBackend};

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
    runtime.block_on(serve::run(arguments, config))
}

#[cfg(target_os = "linux")]
fn require_native_driver(driver: compio::driver::DriverType) -> std::io::Result<()> {
    if driver.is_iouring() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "openkache-server cannot start: the network runtime fell back \
                 to polling because Linux io_uring could not be initialized. Required \
                 syscalls: io_uring_setup, io_uring_enter, io_uring_register. {}",
            io_uring_kernel_hint()
        )))
    }
}

#[cfg(target_os = "macos")]
fn require_native_driver(driver: compio::driver::DriverType) -> std::io::Result<()> {
    if driver.is_polling() {
        Ok(())
    } else {
        Err(std::io::Error::other(
            "openkache-server requires the network runtime's polling driver \
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
            "openkache-server cannot start: the network runtime requires \
             Linux io_uring ({error}); {cause}. \
             Required syscalls: io_uring_setup, io_uring_enter, io_uring_register. \
             {}",
            io_uring_kernel_hint()
        ),
    )
}

#[cfg(target_os = "linux")]
fn io_uring_kernel_hint() -> String {
    let release = linux_kernel_release();
    let recommendation = match linux_kernel_version(&release) {
        Some((major, minor)) if (major, minor) < (5, 1) => {
            "Upgrade to Linux 5.1 or newer before retrying."
        }
        Some(_) => {
            "This release meets the Linux 5.1 version baseline; kernel version alone \
             does not guarantee io_uring access."
        }
        None => {
            "Linux 5.1 is the version baseline; kernel version could not be parsed, \
             so verify the release manually."
        }
    };
    format!(
        "Detected Linux kernel {release}; io_uring requires Linux 5.1 or newer. \
         {recommendation} Also check the container seccomp policy, CONFIG_IO_URING, \
         and /proc/sys/kernel/io_uring_disabled (0 enables io_uring)."
    )
}

#[cfg(target_os = "linux")]
fn linux_kernel_release() -> String {
    let mut system = unsafe { std::mem::zeroed::<libc::utsname>() };
    if unsafe { libc::uname(&mut system) } != 0 {
        return "unknown".to_owned();
    }

    let release = unsafe { std::ffi::CStr::from_ptr(system.release.as_ptr()) };
    let release = release.to_string_lossy().trim().to_owned();
    if release.is_empty() {
        "unknown".to_owned()
    } else {
        release
    }
}

#[cfg(target_os = "linux")]
fn linux_kernel_version(release: &str) -> Option<(u32, u32)> {
    let mut components = release.split('.');
    let major = components.next()?.parse().ok()?;
    let minor = components
        .next()?
        .split(|character: char| !character.is_ascii_digit())
        .next()?
        .parse()
        .ok()?;
    Some((major, minor))
}

#[cfg(target_os = "macos")]
fn runtime_initialization_error(error: io::Error) -> io::Error {
    io::Error::new(
        error.kind(),
        format!(
            "openkache-server cannot start: the network runtime requires \
             the polling driver on Apple Silicon macOS ({error})"
        ),
    )
}

/// Loads the cache configuration from TOML or returns the default configuration.
fn load_config(path: Option<&std::path::Path>) -> Result<AppConfig, Box<dyn std::error::Error>> {
    let Some(path) = path else {
        return Ok(AppConfig::default());
    };
    let text = std::fs::read_to_string(path)?;
    Ok(toml::from_str(&text)?)
}
