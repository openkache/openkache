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

#[cfg(not(any(
    feature = "network-runtime-compio",
    feature = "network-runtime-monoio",
    feature = "network-runtime-glommio",
    feature = "network-runtime-kimojio"
)))]
compile_error!(
    "enable exactly one network runtime feature: `network-runtime-compio`, `network-runtime-monoio`, `network-runtime-glommio`, or `network-runtime-kimojio`"
);

#[cfg(any(
    all(feature = "network-runtime-compio", feature = "network-runtime-monoio"),
    all(
        feature = "network-runtime-compio",
        feature = "network-runtime-glommio"
    ),
    all(
        feature = "network-runtime-compio",
        feature = "network-runtime-kimojio"
    ),
    all(
        feature = "network-runtime-monoio",
        feature = "network-runtime-glommio"
    ),
    all(
        feature = "network-runtime-monoio",
        feature = "network-runtime-kimojio"
    ),
    all(
        feature = "network-runtime-glommio",
        feature = "network-runtime-kimojio"
    )
))]
compile_error!(
    "enable exactly one network runtime feature: `network-runtime-compio`, `network-runtime-monoio`, `network-runtime-glommio`, or `network-runtime-kimojio`"
);

#[cfg(any(
    all(feature = "quic-quinn", not(feature = "network-runtime-compio")),
    all(feature = "quic-noq", not(feature = "network-runtime-compio"))
))]
compile_error!(
    "`quic-quinn` and `quic-noq` require `network-runtime-compio`; use `quic-quiche` with another network runtime"
);

#[cfg(all(
    any(
        feature = "network-runtime-monoio",
        feature = "network-runtime-glommio",
        feature = "network-runtime-kimojio"
    ),
    not(target_os = "linux")
))]
compile_error!("Monoio, Glommio, and Kimojio network runtimes require Linux io_uring support");

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
// The generated contract contains compatibility constants used by adapters
// and client build targets that are not all referenced by the server crate.
// Keep those generated details out of server lint noise without weakening
// warnings for handwritten production code.
#[allow(dead_code)]
pub(crate) mod contract {
    include!(concat!(env!("OUT_DIR"), "/server_contract.rs"));
}
pub(crate) mod network_runtime;
pub(crate) mod operation_compatibility_contract;
pub(crate) mod operation_contract;
pub mod platform;
pub mod protocol;
pub mod resp;
pub mod server;
mod transport;
pub mod types;

pub(crate) mod channel;
pub(crate) mod observability;
pub(crate) mod storage_backend;
pub(crate) mod storage_runtime;

pub use protocol::{
    EvictionDefault, EvictionMode, ExpirationDefault, ExpirationMode, ItemId, NamespaceDescriptor,
    NamespacePolicy, OverridePolicy, SetCondition, SetOptions,
};
pub use types::{ItemValue, StorageKey};

mod error;
pub use error::{KvError, Result};

mod config;
pub use config::{
    AppConfig, BucketSelectionPolicy, Config, IoUringConfig, NetworkConfig, ObservabilityConfig,
    QuicBackend, QuicConfig, RuntimeConfig, StorageConfig, TableConfig, TimeoutConfig, TlsConfig,
    bits_for_count, expand_thread_pattern,
};
pub use platform::allowed_cpu_ids;

/// Returns the compile-time-selected storage-worker runtime backend.
pub const fn storage_runtime_name() -> &'static str {
    storage_runtime::NAME
}

/// Returns the compile-time-selected network runtime backend.
pub const fn network_runtime_name() -> &'static str {
    network_runtime::name()
}

/// Runs a future on the compile-time-selected network runtime.
pub fn block_on<F>(future: F) -> std::io::Result<F::Output>
where
    F: std::future::Future + 'static,
{
    network_runtime::block_on(future)
}

/// Waits for the process shutdown signal using the selected network runtime.
pub async fn shutdown_signal() {
    network_runtime::shutdown_signal().await;
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
    storage_backend::USES_PHYSICAL_STORAGE
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
