//! Command-line entry point for the SSD-backed OpenKache QUIC server.

use std::net::SocketAddr;
use std::path::PathBuf;

use clap::Parser;
use openkache::server::KacheServer;
use openkache::{AppConfig, QuicBackend};

#[path = "openkache_server/allocator.rs"]
mod allocator;

/// Command-line arguments controlling the network endpoint and cache configuration.
#[derive(Parser)]
#[command(name = "openkache-server")]
struct Arguments {
    /// UDP address on which the QUIC endpoint listens.
    #[arg(long, default_value = "127.0.0.1:4433")]
    listen: SocketAddr,

    /// File receiving the generated self-signed server certificate.
    #[arg(long, default_value = "target/openkache-local/certificate.local.der")]
    certificate_out: PathBuf,

    /// Optional TOML cache configuration file.
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,

    /// QUIC protocol implementation, overriding the configuration file.
    #[arg(long, value_enum)]
    quic_backend: Option<QuicBackend>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let runtime = compio::runtime::Runtime::new()?;
    if !runtime.driver_type().is_iouring() {
        return Err(std::io::Error::other("openkache-server requires the io_uring driver").into());
    }
    runtime.block_on(run())
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let arguments = Arguments::parse();
    let mut config = load_config(arguments.config.as_deref())?;
    if let Some(backend) = arguments.quic_backend {
        config.quic.backend = Some(backend);
    }
    let storage_directory = config.storage.directory.clone();
    let quic_backend = config.quic.selected_backend()?;
    let server = KacheServer::bind_with_config(arguments.listen, config).await?;
    let address = server.local_addr()?;
    if let Some(parent) = arguments.certificate_out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&arguments.certificate_out, server.certificate_der())?;

    println!("OpenKache listening on {address}");
    println!(
        "Client certificate: {}",
        arguments.certificate_out.display()
    );
    println!("Storage: SSD-backed ({})", storage_directory.display());
    println!("Runtime: Compio (io_uring)");
    println!("QUIC backend: {}", quic_backend.as_str());
    println!("Allocator: {}", allocator::NAME);
    println!("Press Ctrl-C to stop");

    server
        .serve(async {
            let _ = compio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

/// Loads the cache configuration from TOML or returns the default configuration.
fn load_config(path: Option<&std::path::Path>) -> Result<AppConfig, Box<dyn std::error::Error>> {
    let Some(path) = path else {
        return Ok(AppConfig::default());
    };
    let text = std::fs::read_to_string(path)?;
    Ok(toml::from_str(&text)?)
}
