//! Protocol-specific server startup and reporting.

use std::net::SocketAddr;

use clap::ValueEnum;
use openkache::AppConfig;
use openkache::resp::RespServer;

use super::{Arguments, DEFAULT_PORT, allocator};

/// Client protocol accepted by the server process.
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(super) enum Protocol {
    /// OpenKache's authenticated QUIC protocol.
    Quic,
    /// Loopback-only plaintext Redis Serialization Protocol version 2.
    Resp,
}

pub(super) async fn run(
    arguments: Arguments,
    config: AppConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let storage_directory = config.storage.directory.clone();
    let listen = arguments.listen.unwrap_or_else(|| {
        SocketAddr::from(([127, 0, 0, 1], arguments.port.unwrap_or(DEFAULT_PORT)))
    });

    match arguments.protocol {
        Protocol::Quic => {
            let quic_backend = config.quic.selected_backend()?;
            let (server, security_mode) = arguments.security.bind(listen, config).await?;
            let address = server.local_addr()?;
            arguments
                .security
                .write_development_certificate(&server, security_mode)?;
            report_common(address, &storage_directory);
            arguments.security.report(security_mode);
            println!("Protocol: QUIC ({})", quic_backend.as_str());
            server.serve(shutdown_signal()).await?;
        }
        Protocol::Resp => {
            if config.tls.is_configured() {
                return Err(
                    "native RESP is currently loopback-only plaintext; remove TLS configuration or use --protocol quic"
                        .into(),
                );
            }
            let server = RespServer::bind_plaintext_for_development(listen, config).await?;
            report_common(server.local_addr()?, &storage_directory);
            println!("Security: loopback-only plaintext development mode");
            println!("Protocol: RESP2 (native direct dispatch)");
            server.serve(shutdown_signal()).await?;
        }
    }
    Ok(())
}

fn report_common(address: SocketAddr, storage_directory: &std::path::Path) {
    println!("OpenKache listening on {address}");
    println!("Storage directory: {}", storage_directory.display());
    println!("Storage runtime: {}", openkache::storage_runtime_name());
    println!("Allocator: {}", allocator::NAME);
    println!("Press Ctrl-C or send SIGTERM to stop");
}

async fn shutdown_signal() {
    let interrupt = compio::signal::ctrl_c();
    let terminate = compio::signal::unix::signal(libc::SIGTERM);
    futures_util::pin_mut!(interrupt, terminate);
    let _ = futures_util::future::select(interrupt, terminate).await;
}
