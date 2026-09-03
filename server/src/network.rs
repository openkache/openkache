use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

#[path = "network_linux.rs"]
mod platform;

#[cfg(not(target_os = "linux"))]
compile_error!("openkache-server supports Linux only");

#[path = "network/provided_buffer_ring.rs"]
pub(crate) mod provided_buffer_ring;

#[allow(unused_imports)]
pub(crate) use platform::{ACCEPT_CQE, READ_CQE, WRITE_CQE, make_cqe_user_data};

static SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);

extern "C" fn signal_handler(_signal: libc::c_int) {
    // Signal handlers may only perform async-signal-safe operations. The
    // atomic flag lets the network loop perform the actual shutdown.
    SHUTDOWN_REQUESTED.store(true, Ordering::Relaxed);
}

pub(crate) fn install_signal_handlers() -> io::Result<()> {
    SHUTDOWN_REQUESTED.store(false, Ordering::Relaxed);

    for signal in [libc::SIGINT, libc::SIGTERM] {
        let handler = signal_handler as *const () as libc::sighandler_t;
        let previous_handler = unsafe { libc::signal(signal, handler) };
        if previous_handler == libc::SIG_ERR {
            return Err(io::Error::last_os_error());
        }
    }

    Ok(())
}

pub(crate) fn shutdown_requested() -> bool {
    SHUTDOWN_REQUESTED.load(Ordering::Relaxed)
}

pub(crate) use platform::Network;
