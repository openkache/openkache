//! Isolated development server with encrypted TLS and no client certificates.
//!
//! The server still generates and presents an ephemeral TLS 1.3 certificate;
//! "certificate-free" here means that clients do not need a configured
//! identity or trust bundle. Do not expose this mode to an untrusted network.

use std::net::SocketAddr;

use openkache::server::KacheServer;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    openkache::block_on(async {
        let address = SocketAddr::from(([127, 0, 0, 1], 4433));
        let server = KacheServer::bind_insecure_for_development(
            address,
            openkache::AppConfig::default(),
        )
        .await?;
        println!(
            "TLS 1.3 development server listening on {} (client certificates omitted)",
            server.local_addr()?
        );
        server
            .serve(std::future::pending::<()>())
            .await
            .map_err(Into::into)
    })?
}
