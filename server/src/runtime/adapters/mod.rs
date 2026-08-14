//! Runtime adapters for protocol- or API-specific keyed behavior.
//!
//! The worker and scheduler depend on opaque adapter contracts. Concrete wire
//! compatibility behavior lives in this namespace so adding another API does
//! not add operation branches to the generic runtime.

pub(in crate::runtime) mod draft_v1_keyed;
