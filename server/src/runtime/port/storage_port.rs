//! Runtime-neutral storage capability port.
//!
//! The network/runtime implementation owns routing and worker lifecycle;
//! API modules depend only on opaque address and storage operation contracts.

use std::future::Future;
use std::hash::{Hash, Hasher};
use std::pin::Pin;

use openkache_protocol::{OwnedRange, StableBytes};

pub(crate) use crate::types::StorageWriteOptions;

/// Opaque address accepted by the generic storage capability.
///
/// API bindings only need a stable byte identity. The server-derived
/// [`StorageKey`] remains an implementation detail of the runtime adapter.
///
/// Callers may use fixed- or variable-length identities. Both are normalized
/// by the runtime adapter, so API contracts do not need to adopt the storage
/// engine's internal key width or namespace.
#[derive(Debug)]
pub(crate) struct StorageAddress {
    owner: OwnedRange,
}

impl StorageAddress {
    pub(crate) fn new(bytes: impl AsRef<[u8]>) -> Self {
        Self::from_owned(bytes.as_ref().to_vec())
    }

    /// Takes ownership of an existing byte buffer without copying it.
    ///
    /// The address may outlive the request frame while work is queued, so the
    /// buffer is moved into the address instead of borrowing transport memory.
    pub(crate) fn from_owned(bytes: Vec<u8>) -> Self {
        Self {
            owner: OwnedRange::whole(bytes),
        }
    }

    /// Takes ownership of a byte buffer while retaining a logical sub-range.
    ///
    /// This lets queued storage work keep an opaque request's original
    /// frame allocation alive without memmoving a payload out of its wire
    /// prefix. The returned address still compares and hashes by the visible
    /// range rather than by the hidden frame bytes.
    pub(crate) fn from_owned_range(bytes: OwnedRange) -> Self {
        Self { owner: bytes }
    }

    /// Creates an address from a borrowed key without exposing ownership
    /// details to the API binding.
    pub(crate) fn from_bytes(bytes: impl AsRef<[u8]>) -> Self {
        Self::new(bytes)
    }

    /// Builds a collision-resistant composite address from caller-owned
    /// segments.
    ///
    /// Each segment is length-delimited before it is concatenated. This gives
    /// API modules an inexpensive way to add an API/type/tenant prefix without
    /// asking the storage infrastructure to know what that prefix means, and
    /// avoids ambiguous concatenations such as `["ab", "c"]` and `["a", "bc"]`.
    #[allow(dead_code)]
    pub(crate) fn from_segments<I, B>(segments: I) -> Self
    where
        I: IntoIterator<Item = B>,
        B: AsRef<[u8]>,
    {
        let mut encoded = Vec::new();
        for segment in segments {
            let bytes = segment.as_ref();
            let length =
                u64::try_from(bytes.len()).expect("a platform slice length always fits in u64");
            encoded.extend_from_slice(&length.to_be_bytes());
            encoded.extend_from_slice(bytes);
        }
        Self::from_owned(encoded)
    }

    pub(crate) fn as_bytes(&self) -> &[u8] {
        self.owner.as_slice()
    }
}

impl PartialEq for StorageAddress {
    fn eq(&self, other: &Self) -> bool {
        self.as_ref() == other.as_ref()
    }
}

impl Eq for StorageAddress {}

impl Hash for StorageAddress {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_ref().hash(state);
    }
}

impl Ord for StorageAddress {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.as_ref().cmp(other.as_ref())
    }
}

impl PartialOrd for StorageAddress {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<const N: usize> From<[u8; N]> for StorageAddress {
    fn from(bytes: [u8; N]) -> Self {
        Self::new(bytes)
    }
}

impl From<&[u8]> for StorageAddress {
    fn from(bytes: &[u8]) -> Self {
        Self::from_bytes(bytes)
    }
}

impl AsRef<[u8]> for StorageAddress {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

/// Opaque value accepted by the generic storage capability.
///
/// The value retains its original allocation and logical byte range so an API
/// binding can move a large request payload into storage without shifting it
/// out of its wire prefix.
#[derive(Debug)]
#[allow(dead_code)]
pub(crate) struct StorageValue {
    owner: OwnedRange,
}

#[allow(dead_code)]
impl StorageValue {
    /// Takes ownership of a complete value buffer.
    pub(crate) fn from_owned(bytes: Vec<u8>) -> Self {
        Self {
            owner: OwnedRange::whole(bytes),
        }
    }

    /// Takes ownership of a value buffer while retaining its logical range.
    pub(crate) fn from_owned_range(bytes: OwnedRange) -> Self {
        Self {
            owner: if bytes.is_empty() {
                OwnedRange::whole(Vec::new())
            } else {
                bytes
            },
        }
    }

    /// Returns the visible value bytes.
    pub(crate) fn as_bytes(&self) -> &[u8] {
        self.owner.as_slice()
    }

    /// Transfers the owned allocation and logical range to the runtime.
    pub(crate) fn into_owned_range(self) -> OwnedRange {
        self.owner
    }
}

impl From<Vec<u8>> for StorageValue {
    fn from(bytes: Vec<u8>) -> Self {
        Self::from_owned(bytes)
    }
}

impl AsRef<[u8]> for StorageValue {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

/// Stable ownership for one value returned through the neutral storage port.
///
/// Implementations may retain memory segments, pooled read leases, or another
/// backend owner. The visible byte length must remain unchanged while the
/// owner is held by a [`StorageReadValue`].
pub(crate) use openkache_protocol::StableByteOwner as StorageReadOwner;

/// One storage-read value with backend-independent byte ownership.
///
/// API bindings can inspect or transfer this value without learning which
/// storage representation keeps its bytes alive.
pub(crate) struct StorageReadValue {
    owner: StableBytes,
}

impl StorageReadValue {
    pub(crate) fn from_owner(owner: impl StorageReadOwner) -> Self {
        Self {
            owner: StableBytes::new(owner),
        }
    }

    pub(crate) fn as_bytes(&self) -> &[u8] {
        self.owner.as_slice()
    }

    pub(crate) fn into_stable_bytes(self) -> StableBytes {
        self.owner
    }
}

impl AsRef<[u8]> for StorageReadValue {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl std::fmt::Debug for StorageReadValue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_tuple("StorageReadValue")
            .field(&self.as_bytes())
            .finish()
    }
}

impl<T: AsRef<[u8]>> PartialEq<T> for StorageReadValue {
    fn eq(&self, other: &T) -> bool {
        self.as_bytes() == other.as_ref()
    }
}

impl Eq for StorageReadValue {}

/// Runtime-neutral storage failure.
///
/// The storage contract deliberately does not expose the server's concrete
/// `KvError` enum. Runtime adapters may preserve richer backend diagnostics
/// internally, but API modules only need a stable category and message.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum StorageError {
    InvalidRequest(String),
    Worker(String),
    Unavailable(String),
    Timeout(String),
    Backend(String),
}

impl StorageError {
    pub(crate) fn message(&self) -> &str {
        match self {
            Self::InvalidRequest(message)
            | Self::Worker(message)
            | Self::Unavailable(message)
            | Self::Timeout(message)
            | Self::Backend(message) => message,
        }
    }
}

pub(crate) type StorageResult<T> = std::result::Result<T, StorageError>;

/// Future returned by a neutral storage read.
pub(crate) type StorageReadFuture<'a> =
    Pin<Box<dyn Future<Output = StorageResult<Option<StorageReadValue>>> + 'a>>;

/// Future returned by a neutral storage mutation.
#[allow(dead_code)]
pub(crate) type StorageMutationFuture<'a> =
    Pin<Box<dyn Future<Output = StorageResult<StorageMutation>> + 'a>>;

/// Future returned by a neutral storage write.
#[allow(dead_code)]
pub(crate) type StorageWriteFuture<'a> =
    Pin<Box<dyn Future<Output = StorageResult<StorageWriteOutcome>> + 'a>>;

/// The result of a storage mutation.
///
/// The distinction is intentionally smaller than the concrete store outcome:
/// callers only need to know whether the requested mutation was applied.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StorageMutation {
    Applied,
    Unchanged,
}

/// Result of storing one value.
///
/// The neutral port preserves whether storage inserted or replaced a value so
/// APIs that need create-vs-update semantics do not need a custom task.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StorageWriteOutcome {
    Created,
    Replaced,
    Unchanged,
}

/// Runtime implementation contract for the API-facing storage capability.
///
/// The bridge re-exports this under the neutral [`StoragePort`] name. Keeping
/// the implementation here avoids making the runtime depend on the server
/// composition module while still hiding worker details from API bindings.
pub(crate) trait StoragePort: Send + Sync {
    /// Retrieves the value stored at one opaque address.
    fn get<'a>(&'a self, storage_address: StorageAddress) -> StorageReadFuture<'a>;

    /// Stores one opaque value at one opaque address.
    #[allow(dead_code)]
    fn set<'a>(
        &'a self,
        storage_address: StorageAddress,
        value: StorageValue,
        options: StorageWriteOptions,
    ) -> StorageWriteFuture<'a>;

    /// Deletes the value at one opaque address.
    #[allow(dead_code)]
    fn delete<'a>(&'a self, storage_address: StorageAddress) -> StorageMutationFuture<'a>;
}
