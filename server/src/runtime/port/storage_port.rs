//! Runtime-neutral storage capability port.
//!
//! The network/runtime implementation owns routing and worker lifecycle;
//! API modules depend only on opaque address and storage operation contracts.

use crate::openkache_protocol::{OwnedRange, StableBytes, StableOwnerPool};

use crate::StorageKey;
pub(crate) use crate::types::StorageWriteOptions;

/// Borrowed API-owned storage scope whose bytes remain opaque to the port.
///
/// Address preparation is synchronous, so request-local scope bytes never
/// need an allocation or ownership transfer.
#[derive(Clone, Copy, Debug)]
pub(crate) struct StorageScope<'a>(&'a [u8]);

impl<'a> StorageScope<'a> {
    pub(crate) const fn from_borrowed(bytes: &'a [u8]) -> Self {
        Self(bytes)
    }

    pub(crate) const fn as_bytes(self) -> &'a [u8] {
        self.0
    }
}

/// Opaque storage partition selected during address preparation.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct StorageRoute(u64);

impl StorageRoute {
    pub(crate) const fn from_persisted(value: u64) -> Self {
        Self(value)
    }

    pub(crate) const fn persisted(self) -> u64 {
        self.0
    }

    pub(in crate::runtime) fn from_worker(worker: usize) -> Self {
        Self(u64::try_from(worker).expect("storage worker index must fit in u64"))
    }

    pub(in crate::runtime) fn worker(self) -> usize {
        usize::try_from(self.0).expect("storage route must fit in usize")
    }
}

/// Prepared backend address.
///
/// Preparation resolves the exact persisted key and route once. Request
/// operations consume this value without hashing or routing again.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct PreparedStorageAddress {
    key: [u8; crate::types::STORAGE_KEY_BYTES],
    route: StorageRoute,
}

impl PreparedStorageAddress {
    pub(crate) const fn new(
        key: [u8; crate::types::STORAGE_KEY_BYTES],
        route: StorageRoute,
    ) -> Self {
        Self { key, route }
    }

    pub(in crate::runtime) const fn as_bytes(&self) -> &[u8; crate::types::STORAGE_KEY_BYTES] {
        &self.key
    }

    pub(crate) const fn route(&self) -> StorageRoute {
        self.route
    }

    /// Returns the fixed storage identity resolved by address preparation.
    pub(crate) const fn storage_key(&self) -> StorageKey {
        StorageKey::new(self.key)
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
pub(crate) use crate::openkache_protocol::StableByteOwner as StorageReadOwner;

/// Preallocated type-erasure storage for backend read owners.
#[derive(Clone)]
pub(crate) struct StorageReadOwnerPool {
    inner: std::sync::Arc<StableOwnerPool<crate::types::StoredItemBytes>>,
}

impl StorageReadOwnerPool {
    pub(crate) fn allocation_bytes(capacity: usize) -> Option<usize> {
        StableOwnerPool::<crate::types::StoredItemBytes>::allocation_bytes(capacity)
    }

    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            inner: StableOwnerPool::new(capacity),
        }
    }

    pub(crate) fn try_retain(
        &self,
        owner: crate::types::StoredItemBytes,
    ) -> Result<StorageReadValue, crate::types::StoredItemBytes> {
        let lease = self.inner.try_insert(owner)?;
        Ok(StorageReadValue::from_pooled(StableBytes::from_pooled(
            lease,
        )))
    }
}

/// One storage-read value with backend-independent byte ownership.
///
/// API bindings can inspect or transfer this value without learning which
/// storage representation keeps its bytes alive.
pub(crate) struct StorageReadValue {
    owner: StorageReadBytes,
}

pub(crate) enum StorageReadBytes {
    Owned(OwnedRange),
    Stable(StableBytes),
}

impl StorageReadValue {
    pub(crate) fn from_owned(owner: Vec<u8>) -> Self {
        Self {
            owner: StorageReadBytes::Owned(OwnedRange::whole(owner)),
        }
    }

    pub(crate) fn from_owned_range(owner: Vec<u8>, range: std::ops::Range<usize>) -> Option<Self> {
        OwnedRange::new(owner, range).map(|owner| Self {
            owner: StorageReadBytes::Owned(owner),
        })
    }

    pub(crate) fn from_owner(owner: impl StorageReadOwner) -> Self {
        Self {
            owner: StorageReadBytes::Stable(StableBytes::new(owner)),
        }
    }

    pub(crate) fn from_pooled(owner: StableBytes) -> Self {
        Self {
            owner: StorageReadBytes::Stable(owner),
        }
    }

    pub(crate) fn from_shared_owner<T>(
        owner: std::sync::Arc<T>,
        range: std::ops::Range<usize>,
    ) -> Option<Self>
    where
        T: StorageReadOwner,
    {
        StableBytes::from_shared_range(owner, range).map(|owner| Self {
            owner: StorageReadBytes::Stable(owner),
        })
    }

    pub(crate) fn as_bytes(&self) -> &[u8] {
        match &self.owner {
            StorageReadBytes::Owned(owner) => owner.as_slice(),
            StorageReadBytes::Stable(owner) => owner.as_slice(),
        }
    }

    pub(crate) fn into_bytes(self) -> StorageReadBytes {
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
    NoCapacity(String),
    Overloaded(String),
    TooLarge(String),
    Timeout(String),
    Backend(String),
}

impl StorageError {
    #[allow(dead_code)]
    pub(crate) fn into_message(self) -> String {
        match self {
            Self::InvalidRequest(message)
            | Self::Worker(message)
            | Self::NoCapacity(message)
            | Self::Overloaded(message)
            | Self::TooLarge(message)
            | Self::Timeout(message)
            | Self::Backend(message) => message,
        }
    }
}

pub(crate) type StorageResult<T> = std::result::Result<T, StorageError>;

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
