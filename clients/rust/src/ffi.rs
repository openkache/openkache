//! Compatibility re-export for the shared native ABI.
//!
//! The implementation lives in `openkache-client-core` so C, C++, and other native adapters use
//! one connection and value-protection boundary.

pub use openkache_client_core::ffi::*;
