//! Operation-neutral worker request, response, and completion envelopes.

use super::completion::CompletionSender;
use super::storage_port::{StorageError, StorageTaskOutput};
use crate::Result;

/// Work routed through either the keyed scheduler or quiescent control path.
pub(super) enum Request<K, C, X> {
    /// Keyed data-plane work routed through the per-key scheduler.
    Keyed { storage_key: K, command: C },
    /// Control work that requires a quiescent worker.
    Control(X),
}

/// Worker result envelope with an API-owned data-plane projection.
pub(super) enum Response<D> {
    Data(D),
    Stats(String),
    Synced,
    StorageResult(StorageTaskOutput),
    StorageFailure(StorageError),
}

impl<D> std::fmt::Debug for Response<D> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Data(_) => formatter.write_str("Data(..)"),
            Self::Stats(stats) => formatter.debug_tuple("Stats").field(stats).finish(),
            Self::Synced => formatter.write_str("Synced"),
            Self::StorageResult(_) => formatter.write_str("StorageResult(..)"),
            Self::StorageFailure(error) => formatter
                .debug_tuple("StorageFailure")
                .field(error)
                .finish(),
        }
    }
}

pub(super) type ResponseSender<R> = CompletionSender<Result<R>>;

pub(super) struct DeferredResponse<R> {
    pub(super) sender: ResponseSender<R>,
    pub(super) value: R,
}
