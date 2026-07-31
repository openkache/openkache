//! Re-export of the shared native client ABI.
//!
//! The ABI implementation lives in `openkache-client-core` so every native
//! language adapter uses the same transport, protection, validation, and
//! ownership contract. This module keeps the historical Rust crate path
//! (`openkache_client::ffi`) source-compatible.

pub use openkache_client_core::ffi::*;
