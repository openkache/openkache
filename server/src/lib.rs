//! Crate root for the `openkache` server library. Declares all public modules and
//! re-exports the public API types: [`StorageKey`], [`Value`], [`KvError`],
//! [`Result`], config types, [`Command`], and runtime items.

#![cfg_attr(
    all(target_arch = "aarch64", not(feature = "force-scalar")),
    feature(stdarch_aarch64_sve)
)]

pub mod allocators;
pub mod breadcrumb_filter;
pub mod server;
pub mod types;

pub use types::{StorageKey, Value};

mod error;
pub use error::{KvError, Result};

mod config;
pub use config::{
    AppConfig, BucketSelectionPolicy, Config, IoUringConfig, RuntimeConfig, StorageConfig,
    TableConfig, TimeoutConfig, allowed_cpu_ids, bits_for_count, expand_thread_pattern,
};

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
