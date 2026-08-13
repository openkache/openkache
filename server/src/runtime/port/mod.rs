//! Runtime-neutral execution ports.
//!
//! These contracts are grouped separately from the concrete cache adapter and
//! keyed compatibility implementation. They describe task submission,
//! worker-local storage capabilities, and completion slots without naming a
//! wire operation.

pub(crate) mod completion;
pub(crate) mod storage_context;
pub(crate) mod storage_port;
pub(crate) mod storage_task;
