//! Crate root for the `openkache` server library. Declares all public modules and
//! re-exports the public API types: [`Key`], [`Value`], [`HashedKey`], [`KvError`],
//! [`Result`], config types, [`Command`], and runtime items.

#![cfg_attr(
    all(target_arch = "aarch64", not(feature = "force-scalar")),
    feature(stdarch_aarch64_sve)
)]

pub mod allocators;
pub mod breadcrumb_filter;
pub mod types;

pub use types::{HashedKey, Key, Value};

mod error;
pub use error::{KvError, Result};

mod config;
pub use config::{
    AppConfig, Config, DurabilityConfig, IndexConfig, IoUringConfig, RecoveryConfig, RuntimeConfig,
    StorageConfig, TimeoutConfig, bits_for_count, expand_thread_pattern,
};

mod cli;
pub use cli::Command;

pub(crate) mod store;
pub(crate) use store::*;

pub mod runtime;
pub use runtime::*;

pub(crate) mod index;
pub(crate) use index::*;

pub(crate) const BUCKET_BYTES: usize = 64;
