//! Native OpenKache-over-QUIC frontend backed by the prototype's RESP port.
//!
//! TCP and UDP have independent port spaces. The existing server therefore
//! keeps exclusive ownership of the TCP address while this frontend binds the
//! same numeric UDP port for maintained SDK clients.

use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::thread::{self, JoinHandle};

mod mapping;
mod quic;
mod resp_backend;

/// Starts the UDP/QUIC compatibility frontend on its own runtime thread.
///
/// # Arguments
///
/// - `socket`: UDP socket already bound to the public server address.
/// - `resp_backend`: TCP address of the existing RESP listener.
///
/// # Returns
///
/// A handle for the frontend thread. Dropping the handle detaches the thread;
/// it does not stop the frontend.
///
/// # Errors
///
/// Returns an error when the socket cannot enter nonblocking mode, the
/// development TLS configuration cannot be created, or the frontend thread
/// cannot be spawned. Runtime errors after startup are written to stderr.
pub fn spawn(socket: UdpSocket, resp_backend: SocketAddr) -> io::Result<JoinHandle<()>> {
    socket.set_nonblocking(true)?;
    let server_config = quic::server_config()?;

    thread::Builder::new()
        .name("native-resp-proxy".into())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    eprintln!("native RESP proxy runtime failed: {error}");
                    return;
                }
            };

            if let Err(error) = runtime.block_on(quic::serve(socket, server_config, resp_backend)) {
                eprintln!("native RESP proxy stopped: {error}");
            }
        })
}
