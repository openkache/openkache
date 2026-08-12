//! Transport-neutral outcomes returned by modeled server operations.
//!
//! The shared operation handler maps these outcomes through the generated
//! Smithy status contract and owns all wire response framing. API
//! implementations therefore do not need to construct a `Response`, encode a
//! wire frame, or select a layout-specific sentinel.

use smallvec::SmallVec;

use openkache_protocol::{OwnedRange, SegmentedValue};

const INLINE_OPERATION_VALUE_BYTES: usize = 32;

/// Opaque status token carried from an API binding to the response adapter.
///
/// The token deliberately does not embed the generated wire-status enum.
/// API modules own the constants they use and the response adapter validates
/// the resulting code against each operation's generated contract. Keeping
/// only the discriminant here means behavior code does not import a wire
/// response type merely to report a domain outcome.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct StatusToken(u8);

impl StatusToken {
    pub(super) const fn new(code: u8) -> Self {
        Self(code)
    }

    pub(super) const fn code(self) -> u8 {
        self.0
    }
}

/// An owned wire value returned by a modeled server operation.
///
/// The generic boundary intentionally uses bytes. Shape-specific decoding and
/// encoding belongs to the generated contract and API-owned binding. Small
/// scalar and token values remain inline, while existing application/storage
/// allocations retain their ownership and logical range.
pub(super) enum OperationValue {
    Inline(SmallVec<[u8; INLINE_OPERATION_VALUE_BYTES]>),
    Owned(OwnedRange),
    Segmented(SegmentedValue),
}

impl OperationValue {
    /// Copies a small API value into allocation-free inline storage.
    ///
    /// Values larger than the inline capacity remain valid but spill. Callers
    /// that already own a large payload should pass its `Vec` or `OwnedRange`
    /// instead so the existing allocation is preserved.
    pub(super) fn inline(value: &[u8]) -> Self {
        Self::Inline(SmallVec::from_slice(value))
    }

    pub(super) fn len(&self) -> usize {
        match self {
            Self::Inline(value) => value.len(),
            Self::Owned(value) => value.len(),
            Self::Segmented(value) => value.len(),
        }
    }

    pub(super) fn contiguous(&self) -> Option<&[u8]> {
        match self {
            Self::Inline(value) => Some(value),
            Self::Owned(value) => Some(value.as_ref()),
            Self::Segmented(_) => None,
        }
    }
}

impl openkache_protocol::LayoutValue for OperationValue {
    fn encoded_len(&self) -> usize {
        self.len()
    }
}

impl From<Vec<u8>> for OperationValue {
    fn from(value: Vec<u8>) -> Self {
        Self::Owned(value.into())
    }
}

impl From<OwnedRange> for OperationValue {
    fn from(value: OwnedRange) -> Self {
        Self::Owned(value)
    }
}

impl From<SegmentedValue> for OperationValue {
    fn from(value: SegmentedValue) -> Self {
        Self::Segmented(value)
    }
}

type OperationFieldValues = SmallVec<[Option<OperationValue>; 8]>;

/// Domain-level failures understood by the generated contract adapter.
///
/// None of the variants names a wire byte or owns a protocol response.  The
/// adapter in `operation_handlers` maps these stable meanings to the status
/// allowed by the operation's Smithy contract.
#[derive(Debug)]
pub(super) enum OperationError {
    /// Input bytes do not satisfy the operation's modeled domain.
    InvalidRequest(&'static [u8]),
    /// An API-defined status token resolved by the generated contract.
    Status {
        status: StatusToken,
        message: &'static [u8],
    },
    /// A domain-owned status with an allocated diagnostic.
    ///
    /// The allocation keeps backend error types out of the shared outcome
    /// boundary while preserving their useful diagnostic text.
    OwnedStatus {
        status: StatusToken,
        message: Vec<u8>,
    },
}

impl OperationError {
    /// Creates a contract-resolved error without adding an infrastructure enum
    /// variant for one API's status vocabulary.
    pub(super) const fn status(status: StatusToken, message: &'static [u8]) -> Self {
        Self::Status { status, message }
    }

    /// Creates a contract-resolved status with an owned diagnostic.
    pub(super) fn owned_status(status: StatusToken, message: Vec<u8>) -> Self {
        Self::OwnedStatus { status, message }
    }
}
/// Domain payload returned by a successful operation.
pub(super) enum OperationBody {
    /// A status-only response with no payload.
    Empty,
    /// One opaque response payload.
    Opaque(OperationValue),
    /// One ordered output field sequence.
    Fields(OperationFieldValues),
}

impl OperationBody {
    /// Creates an opaque payload from an already-owned byte vector.
    pub(super) fn opaque(value: impl Into<OperationValue>) -> Self {
        Self::Opaque(value.into())
    }
}

/// A response produced by an API-owned behavior implementation.
///
/// The variants deliberately describe domain output only.  In particular,
/// errors are not pre-encoded responses, which keeps wire status selection in
/// one generated adapter.
pub(super) enum OperationOutcome {
    /// A successful domain result and its transport-neutral status.
    Success {
        status: StatusToken,
        body: OperationBody,
    },
    /// A domain-level failure to be mapped by the shared contract adapter.
    Error(OperationError),
    /// The operation may have crossed its commit point, so the caller must
    /// not receive a replayable error response.
    Abandoned,
}

impl OperationOutcome {
    /// Creates a successful opaque response with an API-selected status.
    ///
    /// The payload may represent any API-owned value (for example a token,
    /// receipt, or encoded structure).  The shared response adapter decides
    /// how the operation's declared opaque framing is written to the wire.
    pub(super) fn opaque(
        status: StatusToken,
        value: impl Into<OperationValue>,
    ) -> Self {
        Self::Success {
            status,
            body: OperationBody::opaque(value),
        }
    }

    /// Creates a successful ordered field body.
    ///
    /// The generated response descriptor chooses whether these fields are
    /// dense, variable-length, or projected by an API adapter. The outcome
    /// boundary therefore does not expose a wire layout name.
    pub(super) fn fields<I, V>(status: StatusToken, values: I) -> Self
    where
        I: IntoIterator<Item = Option<V>>,
        V: Into<OperationValue>,
    {
        Self::Success {
            status,
            body: OperationBody::Fields(
                values
                    .into_iter()
                    .map(|value| value.map(Into::into))
                    .collect(),
            ),
        }
    }

    /// Creates a successful result with an explicit domain status.
    pub(super) fn success(status: StatusToken, body: OperationBody) -> Self {
        Self::Success { status, body }
    }

    /// Creates a domain validation failure.
    pub(super) fn invalid_request(message: &'static [u8]) -> Self {
        Self::error(OperationError::InvalidRequest(message))
    }

    pub(super) fn error(error: OperationError) -> Self {
        Self::Error(error)
    }

    /// Suppresses a response when a mutation's commit state is unknowable.
    pub(super) fn abandoned() -> Self {
        Self::Abandoned
    }
}
