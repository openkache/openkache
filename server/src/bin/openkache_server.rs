//! Command-line entry point for the SSD-backed OpenKache QUIC server.

use std::{net::SocketAddr, path::PathBuf};

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
    let runtime = compio::runtime::Runtime::new()?;
    if !runtime.driver_type().is_iouring() {
        return Err(std::io::Error::other("openkache-server requires the io_uring driver").into());
    }
    runtime.block_on(serve::run(arguments, config))
}

/// Loads the cache configuration from TOML or returns the default configuration.
fn load_config(path: Option<&std::path::Path>) -> Result<AppConfig, Box<dyn std::error::Error>> {
    let Some(path) = path else {
        return Ok(AppConfig::default());
    };
    let text = std::fs::read_to_string(path)?;
    Ok(toml::from_str(&text)?)
}
