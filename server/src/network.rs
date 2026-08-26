#[cfg(target_os = "linux")]
#[path = "network_linux.rs"]
mod platform;

#[cfg(target_os = "macos")]
#[path = "network_macos.rs"]
mod platform;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
compile_error!("openkache-server supports Linux and Apple Silicon macOS only");

#[cfg(target_os = "linux")]
#[path = "network/provided_buffer_ring.rs"]
pub(crate) mod provided_buffer_ring;

#[cfg(target_os = "linux")]
#[allow(unused_imports)]
pub(crate) use platform::{ACCEPT_CQE, READ_CQE, WRITE_CQE, make_cqe_user_data};

pub(crate) use platform::Network;
