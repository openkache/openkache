//! Transport-neutral outcomes returned by modeled server operations.
//!
//! The shared operation handler maps these outcomes through the generated
//! Smithy status contract and owns all wire response framing. API
//! implementations therefore do not need to construct a `Response`, select a
//! wire `Status`, or encode a sentinel.

use smallvec::SmallVec;

use super::operation_contract::OperationStatus;
use super::storage_port::{StorageReadBytes, StorageReadValue};
use crate::openkache_protocol::{OwnedRange, ResponseSegment};

/// An owned wire value returned by a modeled server operation.
///
/// The generic boundary intentionally uses bytes. Shape-specific decoding and
/// encoding belongs to the generated contract and API-owned binding. Small
/// scalar and token values remain inline, while existing application/storage
/// allocations retain their ownership and logical range.
pub(super) struct OperationValue(ResponseSegment);

impl OperationValue {
    /// Copies a small API value into allocation-free inline storage.
    ///
    /// Values larger than the inline capacity remain valid but spill. Callers
    /// that already own a large payload should pass its `Vec` or `OwnedRange`
    /// instead so the existing allocation is preserved.
    pub(super) fn inline(value: &[u8]) -> Self {
        Self(ResponseSegment::inline(value))
    }

    pub(super) fn len(&self) -> usize {
        self.as_ref().len()
    }

    pub(super) fn into_segment(self) -> ResponseSegment {
        self.0
    }
}

impl AsRef<[u8]> for OperationValue {
    fn as_ref(&self) -> &[u8] {
        self.0.as_slice()
    }
}

impl From<Vec<u8>> for OperationValue {
    fn from(value: Vec<u8>) -> Self {
        Self(ResponseSegment::owned(value))
    }
}

impl From<OwnedRange> for OperationValue {
    fn from(value: OwnedRange) -> Self {
        Self(ResponseSegment::Owned(value))
    }
}

impl From<ResponseSegment> for OperationValue {
    fn from(value: ResponseSegment) -> Self {
        Self(value)
    }
}

impl From<StorageReadValue> for OperationValue {
    fn from(value: StorageReadValue) -> Self {
        match value.into_bytes() {
            StorageReadBytes::Owned(value) => Self(ResponseSegment::Owned(value)),
            StorageReadBytes::Stable(value) => Self(value.into()),
        }
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
        status: OperationStatus,
        message: &'static [u8],
    },
    /// A domain-owned status with an allocated diagnostic.
    ///
    /// The allocation keeps backend error types out of the shared outcome
    /// boundary while preserving their useful diagnostic text.
    OwnedStatus {
        status: OperationStatus,
        message: Vec<u8>,
    },
}

impl OperationError {
    /// Creates a contract-resolved error without adding an infrastructure enum
    /// variant for one API's status vocabulary.
    pub(super) const fn status(status: OperationStatus, message: &'static [u8]) -> Self {
        Self::Status { status, message }
    }

    /// Creates a contract-resolved status with an owned diagnostic.
    pub(super) fn owned_status(status: OperationStatus, message: Vec<u8>) -> Self {
        Self::OwnedStatus { status, message }
    }
}
/// Generated semantic status understood by the contract adapter.
///
/// The shared response adapter validates the value against the operation's
/// generated status table and rejects values outside that contract.
pub(super) type OperationSuccessStatus = OperationStatus;

/// Domain payload returned by a successful operation.
pub(super) enum OperationBody {
    /// A status-only response with no payload.
    Empty,
    /// One opaque response payload.
    Opaque(OperationValue),
    /// One ordered output field sequence.
    #[allow(dead_code)]
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
        status: OperationSuccessStatus,
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
        status: OperationSuccessStatus,
        value: impl Into<OperationValue>,
    ) -> Self {
        Self::Success {
            status,
            body: OperationBody::opaque(value),
        }
    }

    /// Creates a successful ordered field-sequence response.
    ///
    /// This generic response path remains available for future generated APIs;
    /// the currently registered generic operation returns an opaque response.
    #[allow(dead_code)]
    pub(super) fn field_sequence<I, V>(status: OperationSuccessStatus, values: I) -> Self
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
    pub(super) fn success(status: OperationSuccessStatus, body: OperationBody) -> Self {
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
