//! Operation-neutral request ownership and retry boundaries.

use std::sync::Arc;

use openkache_protocol::{OwnedFrame, WireSegment};

use crate::Result;

pub(crate) const INLINE_REQUEST_PREFIX_BYTES: usize = 64;

/// Operation-neutral ownership for one contiguous request prefix.
///
/// Common framing stays inline. Unusually large API prefixes promote once
/// without changing the ordered payload owners that follow them.
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum RequestPrefix {
    Inline {
        bytes: [u8; INLINE_REQUEST_PREFIX_BYTES],
        len: u8,
    },
    Owned(Vec<u8>),
}

impl RequestPrefix {
    pub(crate) const fn new() -> Self {
        Self::Inline {
            bytes: [0; INLINE_REQUEST_PREFIX_BYTES],
            len: 0,
        }
    }

    pub(crate) fn push(&mut self, byte: u8) {
        self.extend_from_slice(&[byte]);
    }

    pub(crate) fn extend_from_slice(&mut self, suffix: &[u8]) {
        match self {
            Self::Inline { bytes, len }
                if usize::from(*len) + suffix.len() <= INLINE_REQUEST_PREFIX_BYTES =>
            {
                let start = usize::from(*len);
                let end = start + suffix.len();
                bytes[start..end].copy_from_slice(suffix);
                *len = u8::try_from(end).expect("inline request prefix length fits in u8");
            }
            Self::Inline { bytes, len } => {
                let current_len = usize::from(*len);
                let required = current_len + suffix.len();
                let capacity = required.checked_next_power_of_two().unwrap_or(required);
                let mut owned = Vec::with_capacity(capacity);
                owned.extend_from_slice(&bytes[..current_len]);
                owned.extend_from_slice(suffix);
                *self = Self::Owned(owned);
            }
            Self::Owned(bytes) => bytes.extend_from_slice(suffix),
        }
    }

    pub(crate) fn append_varuint(&mut self, value: u64) {
        let (encoded, encoded_len) = openkache_protocol::encode_varuint(value);
        self.extend_from_slice(&encoded[..encoded_len]);
    }

    pub(crate) fn as_slice(&self) -> &[u8] {
        match self {
            Self::Inline { bytes, len } => &bytes[..usize::from(*len)],
            Self::Owned(bytes) => bytes,
        }
    }
}

const _: () = assert!(std::mem::size_of::<RequestPrefix>() <= 72);

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
    prefix: RequestPrefix,
    frame: OwnedFrame,
}

impl RequestParts {
    pub(crate) fn new<I, T>(
        prefix: RequestPrefix,
        segments: I,
    ) -> std::result::Result<Self, openkache_protocol::ProtocolError>
    where
        I: IntoIterator<Item = T>,
        T: Into<WireSegment>,
    {
        Ok(Self {
            prefix,
            frame: OwnedFrame::new(segments)?,
        })
    }

    pub(crate) fn segments(&self) -> impl Iterator<Item = &[u8]> {
        std::iter::once(self.prefix.as_slice())
            .chain(self.frame.segments().iter().map(WireSegment::as_slice))
            .filter(|segment| !segment.is_empty())
    }
}

const _: () = assert!(std::mem::size_of::<RequestParts>() <= 192);

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
        self.parts().segments()
    }
}
