//! Protocol-specific server startup and reporting.

use std::net::SocketAddr;

use clap::ValueEnum;
use openkache::resp::RespServer;
use openkache::{AppConfig, platform::StorageDeviceKind};

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
            let storage_device = server.storage_device_kind();
            arguments
                .security
                .write_development_certificate(&server, security_mode)?;
            report_common(address, &storage_directory, storage_device);
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
            let storage_device = server.storage_device_kind();
            report_common(server.local_addr()?, &storage_directory, storage_device);
            println!("Security: loopback-only plaintext development mode");
            println!("Protocol: RESP2 (native direct dispatch)");
            server.serve(shutdown_signal()).await?;
        }
    }
    Ok(())
}

fn report_common(
    address: SocketAddr,
    storage_directory: &std::path::Path,
    storage_device: StorageDeviceKind,
) {
    println!("OpenKache listening on {address}");
    if storage_device == StorageDeviceKind::NotApplicable {
        println!(
            "Storage directory: {} (not used by simulated storage)",
            storage_directory.display()
        );
    } else {
        println!("Storage directory: {}", storage_directory.display());
    }
    println!("Storage runtime: {}", openkache::storage_runtime_name());
    println!("Network runtime: {}", openkache::network_runtime_name());
    report_storage_device(storage_device);
    println!("Allocator: {}", allocator::NAME);
    println!("Press Ctrl-C or send SIGTERM to stop");
}

fn report_storage_device(kind: StorageDeviceKind) {
    match kind {
        StorageDeviceKind::Nvme => {
            println!("Storage device: NVMe (detected from opened storage files)");
        }
        StorageDeviceKind::NonNvme => {
            eprintln!(
                "WARNING: at least one opened storage file is on a non-NVMe block \
                 device. OpenKache will continue, but NVMe SSD is the intended \
                 production medium for predictable latency."
            );
        }
        StorageDeviceKind::Unknown => {
            eprintln!(
                "WARNING: could not verify the devices used by the opened storage \
                 files. OpenKache will continue, but NVMe SSD is the intended \
                 production medium for predictable latency."
            );
        }
        StorageDeviceKind::NotApplicable => {
            println!(
                "Storage device: not applicable (simulated storage uses no physical files)"
            );
        }
    }
}

async fn shutdown_signal() {
    openkache::shutdown_signal().await;
}
