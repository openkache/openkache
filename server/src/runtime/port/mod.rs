//! Runtime-neutral execution ports.
//!
//! These contracts are grouped separately from the concrete cache adapter and
//! keyed compatibility implementation. They describe worker completion slots
//! and storage capabilities without naming a wire operation.

pub(crate) mod completion;
pub(crate) mod storage_port;
