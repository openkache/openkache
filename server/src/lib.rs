//! Crate root for the `openkache` server library. Declares all public modules and
//! re-exports the public API types: [`StorageKey`], [`ItemValue`], [`KvError`],
//! [`Result`], config types, [`Command`], and runtime items.

#![cfg_attr(
    all(
        target_arch = "aarch64",
        target_os = "linux",
        not(feature = "force-scalar")
    ),
    feature(stdarch_aarch64_sve)
)]

#[cfg(not(any(
    all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    ),
    all(target_os = "macos", target_arch = "aarch64")
)))]
compile_error!(
    "OpenKache server supports only Linux x86_64/aarch64 and Apple Silicon macOS; \
     use one of these supported targets"
);

#[cfg(not(any(feature = "quic-quinn", feature = "quic-noq", feature = "quic-quiche")))]
compile_error!(
    "enable at least one QUIC backend feature: `quic-quinn`, `quic-noq`, or `quic-quiche`"
);

#[cfg(not(any(
    feature = "channel-crossfire",
    feature = "channel-flume",
    feature = "channel-kanal"
)))]
compile_error!(
    "enable exactly one channel backend feature: `channel-crossfire`, `channel-flume`, or `channel-kanal`"
);

#[cfg(any(
    all(feature = "channel-crossfire", feature = "channel-flume"),
    all(feature = "channel-crossfire", feature = "channel-kanal"),
    all(feature = "channel-flume", feature = "channel-kanal")
))]
compile_error!(
    "enable exactly one channel backend feature: `channel-crossfire`, `channel-flume`, or `channel-kanal`"
);

#[cfg(not(any(
    feature = "storage-runtime-compio",
    feature = "storage-runtime-kimojio",
    feature = "storage-runtime-monoio",
    feature = "storage-runtime-simulated"
)))]
compile_error!(
    "enable exactly one storage runtime feature: `storage-runtime-compio`, `storage-runtime-kimojio`, `storage-runtime-monoio`, or `storage-runtime-simulated`"
);

#[cfg(any(
    all(
        feature = "storage-runtime-compio",
        feature = "storage-runtime-kimojio"
    ),
    all(feature = "storage-runtime-compio", feature = "storage-runtime-monoio"),
    all(
        feature = "storage-runtime-compio",
        feature = "storage-runtime-simulated"
    ),
    all(
        feature = "storage-runtime-kimojio",
        feature = "storage-runtime-monoio"
    ),
    all(
        feature = "storage-runtime-kimojio",
        feature = "storage-runtime-simulated"
    ),
    all(
        feature = "storage-runtime-monoio",
        feature = "storage-runtime-simulated"
    )
))]
compile_error!(
    "enable exactly one storage runtime feature: `storage-runtime-compio`, `storage-runtime-kimojio`, `storage-runtime-monoio`, or `storage-runtime-simulated`"
);

#[cfg(all(
    any(
        feature = "storage-runtime-kimojio",
        feature = "storage-runtime-monoio"
    ),
    not(target_os = "linux")
))]
compile_error!(
    "`storage-runtime-kimojio` and `storage-runtime-monoio` require Linux io_uring support"
);

pub mod allocators;
pub mod breadcrumb_filter;
pub mod platform;
pub mod resp;
pub mod server;
mod transport;
pub mod types;

pub(crate) mod channel;
pub(crate) mod storage_runtime;

pub use types::{ItemValue, StorageKey};

mod error;
pub use error::{KvError, Result};

mod config;
pub use config::{
    AppConfig, BucketSelectionPolicy, Config, IoUringConfig, NetworkConfig, QuicBackend,
    QuicConfig, RuntimeConfig, StorageConfig, TableConfig, TimeoutConfig, TlsConfig,
    bits_for_count, expand_thread_pattern,
};
pub use platform::allowed_cpu_ids;

/// Returns the compile-time-selected storage-worker runtime backend.
pub const fn storage_runtime_name() -> &'static str {
    storage_runtime::NAME
}

/// Returns the io_uring submission entry count used by the selected storage runtime.
pub const fn storage_runtime_effective_ring_entries(configured: u32) -> u32 {
    storage_runtime::effective_ring_entries(configured)
}

/// Returns whether the selected storage runtime uses physical storage files.
///
/// # Returns
///
/// `true` for native storage runtimes that open Segment/Blob files, and `false`
/// for completion-only runtimes that do not use physical storage.
pub const fn storage_runtime_uses_physical_storage() -> bool {
    storage_runtime::USES_PHYSICAL_STORAGE
}

/// Builds the Compio runtime used by the server's top-level task.
///
/// The returned runtime uses the same production builder and native-driver
/// policy as dedicated network workers. On Linux this requires io_uring; on
/// Apple Silicon macOS it requires Compio's polling driver.
pub fn build_server_runtime() -> std::io::Result<compio::runtime::Runtime> {
    storage_runtime::build(storage_runtime::CompioRuntimeConfig::server_host())
}

pub(crate) mod sizing;
pub use sizing::{SizingPlan, SizingProfile, SizingRequest};

mod cli;
pub use cli::Command;

pub(crate) mod store;
pub(crate) use store::*;

pub mod runtime;
pub use runtime::*;

pub(crate) mod table;
pub(crate) use table::*;

pub(crate) const BUCKET_BYTES: usize = 4 * 1024;
pub(crate) const SUBTABLE_BYTES: usize = 64;
