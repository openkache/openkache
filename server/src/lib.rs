//! Reusable data structures for the OpenKache server.

#![cfg_attr(
    all(target_arch = "aarch64", not(feature = "force-scalar")),
    feature(stdarch_aarch64_sve)
)]

/// Approximate membership filtering based on the BCF53 breadcrumb filter.
pub mod allocators;

pub mod breadcrumb;

/// Shared key, value, and fixed-size hashed-key types.
pub mod types;

/// Re-exports the common KV types at the crate root for convenient reuse.
pub use types::{HashedKey, Key, Value};
