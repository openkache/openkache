//! Command-line entry point for the minimal in-memory OpenKache QUIC server.

use std::net::SocketAddr;
use std::path::PathBuf;

use clap::Parser;
use openkache::server::KacheServer;

#[derive(Parser)]
#[command(name = "openkache-server")]
struct Arguments {
    #[arg(long, default_value = "127.0.0.1:4433")]
    listen: SocketAddr,

    #[arg(long, default_value = "target/openkache-local/certificate.local.der")]
    certificate_out: PathBuf,
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
    let server = KacheServer::bind(arguments.listen).await?;
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
    println!("Storage: in-memory HashMap (SYNC is a no-op)");
    println!("Runtime: Compio (io_uring)");
    println!("Press Ctrl-C to stop");

    server
        .serve(async {
            let _ = compio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
