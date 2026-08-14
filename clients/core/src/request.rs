//! Operation-neutral request ownership and retry boundaries.

use std::sync::Arc;

use openkache_protocol::{OwnedFrame, WireSegment};

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

/// API-owned construction deferred until the request lifecycle needs wire parts.
pub(crate) trait RequestBuilder: Sized {
    fn retry_policy(&self) -> RequestRetryPolicy;

    fn into_parts(self) -> Result<RequestParts>;
}

/// Owned request pieces ready for a transport write.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct RequestParts {
    frame: OwnedFrame,
}

impl RequestParts {
    pub(crate) fn new<I, T>(
        segments: I,
    ) -> std::result::Result<Self, openkache_protocol::ProtocolError>
    where
        I: IntoIterator<Item = T>,
        T: Into<WireSegment>,
    {
        Ok(Self {
            frame: OwnedFrame::new(segments)?,
        })
    }

    pub(crate) fn segments(&self) -> &[WireSegment] {
        self.frame.segments()
    }
}

/// One transport write with ownership matched to its replay policy.
pub(crate) enum RequestAttempt {
    /// A non-replayable request moved directly into its only attempt.
    Once(RequestParts),
    /// A replayable request retained by the retry loop.
    Replay(Arc<RequestParts>),
}

impl RequestAttempt {
    fn parts(&self) -> &RequestParts {
        match self {
            Self::Once(parts) => parts,
            Self::Replay(parts) => parts,
        }
    }

    pub(crate) fn segments(&self) -> impl Iterator<Item = &[u8]> {
        self.parts()
            .segments()
            .iter()
            .map(WireSegment::as_slice)
            .filter(|segment| !segment.is_empty())
    }
}
