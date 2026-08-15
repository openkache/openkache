//! Operation-neutral request ownership and retry boundaries.

use std::sync::Arc;

use openkache_protocol::OwnedRequestFrame;

use crate::Result;

/// Closed replay decision attached by an API adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RequestRetryPolicy {
    /// The operation is safe to replay.
    Always,
    /// The operation must not be replayed automatically.
    Never,
}

impl RequestRetryPolicy {
    pub(crate) const fn is_safe(self) -> bool {
        matches!(self, Self::Always)
    }
}

/// API-owned construction deferred until the request lifecycle needs a wire frame.
pub(crate) trait RequestBuilder: Sized {
    fn retry_policy(&self) -> RequestRetryPolicy;

    fn into_frame(self) -> Result<OwnedRequestFrame>;
}

/// Retry state retaining one encoded frame only when another attempt is possible.
pub(crate) enum RequestAttempts<R> {
    Once(Option<R>),
    Replay(Option<Arc<OwnedRequestFrame>>),
}

impl<R: RequestBuilder> RequestAttempts<R> {
    pub(crate) fn new(request: R, replayable: bool) -> Result<Self> {
        if replayable {
            request
                .into_frame()
                .map(|frame| Self::Replay(Some(Arc::new(frame))))
        } else {
            Ok(Self::Once(Some(request)))
        }
    }

    pub(crate) fn next(&mut self, final_attempt: bool) -> Option<PendingRequest<R>> {
        match self {
            Self::Once(request) => request.take().map(PendingRequest::Once),
            Self::Replay(frame) if final_attempt => frame.take().map(PendingRequest::Replay),
            Self::Replay(frame) => frame
                .as_ref()
                .map(|frame| PendingRequest::Replay(Arc::clone(frame))),
        }
    }
}

/// One request selected from the retry state for a connection attempt.
pub(crate) enum PendingRequest<R> {
    Once(R),
    Replay(Arc<OwnedRequestFrame>),
}

/// One transport write with ownership matched to its replay policy.
pub(crate) enum RequestAttempt {
    /// A non-replayable request moved directly into its only attempt.
    Once(OwnedRequestFrame),
    /// A replayable request retained by the retry loop.
    Replay(Arc<OwnedRequestFrame>),
}

impl RequestAttempt {
    fn frame(&self) -> &OwnedRequestFrame {
        match self {
            Self::Once(frame) => frame,
            Self::Replay(frame) => frame,
        }
    }

    pub(crate) fn segments(&self) -> impl Iterator<Item = &[u8]> {
        self.frame().segments()
    }
}
