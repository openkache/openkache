//! Runtime-neutral storage task port.
//!
//! The network/runtime implementation owns routing and worker lifecycle;
//! API modules depend only on these task and context contracts.

use std::any::Any;
use std::future::Future;
use std::hash::{Hash, Hasher};
use std::pin::Pin;
use std::sync::Arc;

use openkache_protocol::OwnedRange;

use super::storage_task::{StorageTask, StorageTaskMetadata};

/// Opaque address accepted by the generic storage capability.
///
/// API bindings only need a stable byte identity. The server-derived
/// [`StorageKey`] remains an implementation detail of the runtime adapter.
///
/// Callers may use fixed- or variable-length identities. Both are normalized
/// by the runtime adapter, so API contracts do not need to adopt the storage
/// engine's internal key width or namespace.
#[derive(Clone, Debug)]
pub(crate) struct StorageAddress {
    owner: Arc<Vec<u8>>,
    start: usize,
    end: usize,
}

impl StorageAddress {
    pub(crate) fn new(bytes: impl AsRef<[u8]>) -> Self {
        let owner = Arc::new(bytes.as_ref().to_vec());
        let end = owner.len();
        Self {
            owner,
            start: 0,
            end,
        }
    }

    /// Takes ownership of an existing byte buffer without copying it.
    ///
    /// The address may outlive the request frame while a storage task is
    /// queued, so the buffer is moved into the shared address owner instead
    /// of borrowing transport memory.
    pub(crate) fn from_owned(bytes: Vec<u8>) -> Self {
        let end = bytes.len();
        Self {
            owner: Arc::new(bytes),
            start: 0,
            end,
        }
    }

    /// Takes ownership of a byte buffer while retaining a logical sub-range.
    ///
    /// This lets a queued storage task keep an opaque request's original
    /// frame allocation alive without memmoving a payload out of its wire
    /// prefix. The returned address still compares and hashes by the visible
    /// range rather than by the hidden frame bytes.
    pub(crate) fn from_owned_range(bytes: OwnedRange) -> Self {
        let (bytes, range) = bytes.into_parts();
        let start = range.start;
        let end = range.end;
        if start == 0 && end == bytes.len() {
            return Self::from_owned(bytes);
        }
        let owner = Arc::new(bytes);
        Self { owner, start, end }
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
        self.owner
            .get(self.start..self.end)
            .expect("storage address range must remain within its owner")
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

/// Runtime-neutral storage failure.
///
/// The storage contract deliberately does not expose the server's concrete
/// `KvError` enum. Runtime adapters may preserve richer backend diagnostics
/// internally, but API-owned tasks only need a stable category and message.
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

/// Result owned by an API-specific storage task.
pub(crate) type StorageTaskOutput = Box<dyn Any + Send>;

/// Future returned by an API-owned storage task.
pub(crate) type StorageTaskFuture<'a> =
    Pin<Box<dyn Future<Output = StorageResult<StorageTaskOutput>> + 'a>>;

/// Future returned by a neutral storage context operation.
pub(crate) type StorageContextFuture<'a, T> = Pin<Box<dyn Future<Output = StorageResult<T>> + 'a>>;

/// Typed completion future for the storage-port convenience methods.
pub(crate) type StorageTypedTaskFuture<'a, T> =
    Pin<Box<dyn Future<Output = StorageResult<T>> + 'a>>;

/// Recovers an API-owned result from the erased worker completion boundary.
///
/// Type erasure is confined to the runtime submission channel. API bindings
/// can keep their task result typed at the call site and receive a structured
/// worker error if the composition boundary is wired incorrectly.
pub(crate) fn downcast_storage_output<T: Any + Send>(
    output: StorageTaskOutput,
) -> StorageResult<T> {
    output
        .downcast::<T>()
        .map(|value| *value)
        .map_err(|_| StorageError::Worker("storage task returned an unexpected result type".into()))
}

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

/// One operation in a generic storage batch.
///
/// The batch vocabulary describes storage capabilities rather than a
/// protocol operation. An API can compose reads, writes, deletes, and
/// conditional mutations without adding another worker command variant.
#[allow(dead_code)]
#[derive(Debug)]
pub(crate) enum StorageBatchOperation {
    Get {
        address: StorageAddress,
    },
    Set {
        address: StorageAddress,
        value: Vec<u8>,
        options: StorageWriteOptions,
    },
    Delete {
        address: StorageAddress,
    },
    CompareAndSet {
        address: StorageAddress,
        expected: Option<Vec<u8>>,
        replacement: Option<Vec<u8>>,
        options: StorageWriteOptions,
    },
}

/// Result of one operation in a generic storage batch.
///
/// Mutation and batch variants are intentionally retained as an extension
/// capability even while the first generic example is read-only. Keeping this
/// vocabulary here lets future APIs opt into compare-and-set or grouped work
/// without adding operation-specific worker commands.
#[allow(dead_code)]
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum StorageBatchResult {
    Value(Option<Vec<u8>>),
    Mutation(StorageMutation),
    CompareAndSet(bool),
}

/// Existence condition for a storage mutation.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum StorageWriteCondition {
    #[default]
    Any,
    IfAbsent,
    IfPresent,
}

/// Expiration selection independent of the protocol-v1 flag byte.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum StorageWriteExpiration {
    #[default]
    Inherit,
    NoExpiry,
    Ttl(u64),
}

/// Capacity selection independent of the protocol-v1 flag byte.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum StorageWriteEviction {
    #[default]
    Inherit,
    Evictable,
    Protected,
}

/// Neutral write options supplied by API-owned storage tasks.
///
/// This type is deliberately not a wire or client type. Runtime adapters
/// translate it to the active backend's concrete write options.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct StorageWriteOptions {
    pub(crate) condition: StorageWriteCondition,
    pub(crate) expiration: StorageWriteExpiration,
    pub(crate) eviction: StorageWriteEviction,
}

/// Operations that can safely share a worker's read lane.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StorageTaskScheduling {
    Exclusive,
    ReadOnly,
}

/// The key ownership promised by a storage task.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StorageTaskScope {
    /// The task is routed by one key.
    SingleKey,
    /// The task has an explicit key set and must stay on one worker.
    KeySet,
    /// The task does not use keyed storage routing.
    Unbound,
}

/// Whether an in-flight task may be abandoned after its caller disappears.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StorageTaskCancellation {
    CompleteOnceSubmitted,
    CancelIfDisconnected,
}

/// Commit behavior declared by an API-owned task.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StorageTaskIsolation {
    None,
    /// Linearizes the task inside one worker-owned mutable storage lane.
    ///
    /// This is a runtime serialization guarantee, not a crash-safe
    /// transaction or a cross-worker commit protocol. Backends that need
    /// rollback semantics must add an explicit transaction primitive instead
    /// of relying on this marker.
    WorkerSerialized,
}

/// API-owned execution context.
///
/// The object-safe context is the only storage surface visible to generic API
/// tasks. Concrete cache objects, protocol policy structs, and worker-local
/// state remain behind the runtime implementation.
#[allow(dead_code)]
pub(crate) trait StorageContext {
    fn get<'a>(
        &'a mut self,
        storage_address: StorageAddress,
    ) -> StorageContextFuture<'a, Option<Vec<u8>>>;

    fn set<'a>(
        &'a mut self,
        storage_address: StorageAddress,
        value: Vec<u8>,
        options: StorageWriteOptions,
    ) -> StorageContextFuture<'a, StorageMutation>;

    fn delete<'a>(
        &'a mut self,
        storage_address: StorageAddress,
    ) -> StorageContextFuture<'a, StorageMutation>;

    /// Compares and conditionally replaces one value.
    ///
    /// API tasks that require worker-local linearization must be submitted through
    /// `StoragePort::execute_for_key` or `execute_for_keys`; those routes
    /// serialize the task with the addressed worker lane.
    fn compare_and_set<'a>(
        &'a mut self,
        storage_address: StorageAddress,
        expected: Option<&'a [u8]>,
        replacement: Option<Vec<u8>>,
        options: StorageWriteOptions,
    ) -> StorageContextFuture<'a, bool>;

    /// Executes a batch against the current worker lane.
    ///
    /// A task declaring [`StorageTaskIsolation::WorkerSerialized`] must be submitted
    /// through a keyed scope that covers every address in the batch. The
    /// runtime then keeps the batch on one worker while this method executes
    /// each operation in order. It does not promise rollback after a process
    /// failure.
    fn batch<'a>(
        &'a mut self,
        operations: Vec<StorageBatchOperation>,
    ) -> StorageContextFuture<'a, Vec<StorageBatchResult>>;
}

/// Runtime implementation contract for the API-facing storage capability.
///
/// The bridge re-exports this under the neutral [`StoragePort`] name. Keeping
/// the implementation here avoids making the runtime depend on the server
/// composition module while still hiding worker details from API bindings.
pub(crate) trait StoragePort: Any + Send + Sync {
    /// Submits keyed work to the owner of one storage key.
    fn execute_for_key<'a>(
        &'a self,
        storage_address: StorageAddress,
        task: StorageTask,
    ) -> StorageTaskFuture<'a>;

    /// Submits work for an explicit key set.
    ///
    /// Implementations reject a set that crosses worker boundaries instead of
    /// silently weakening the task's worker-serialization declaration.
    fn execute_for_keys<'a>(
        &'a self,
        storage_addresses: &'a [StorageAddress],
        task: StorageTask,
    ) -> StorageTaskFuture<'a>;

    /// Submits work that is not associated with a storage key.
    fn execute_unbound<'a>(&'a self, task: StorageTask) -> StorageTaskFuture<'a>;
}

/// Ergonomic typed facade over the object-safe storage submission port.
///
/// The underlying trait remains object-safe and transports one erased result
/// envelope. API bindings use these helpers to keep their own result type
/// without repeating task construction or downcast plumbing.
#[allow(dead_code)]
pub(crate) trait StoragePortExt: StoragePort {
    /// Executes one typed read task on the worker owning `storage_address`.
    fn execute_typed_for_key<'a, T, F>(
        &'a self,
        storage_address: StorageAddress,
        run: F,
    ) -> StorageTypedTaskFuture<'a, T>
    where
        T: Any + Send,
        F: for<'b> FnOnce(&'b mut dyn StorageContext) -> StorageContextFuture<'b, T>
            + Send
            + 'static;

    /// Executes one typed, worker-serialized mutation on a single key.
    ///
    /// API bindings should use this helper for compare-and-set or other
    /// read/modify/write work. The runtime keeps the task on the same keyed
    /// lane as compatibility mutations without exposing a backend-specific command.
    fn execute_typed_mutation_for_key<'a, T, F>(
        &'a self,
        storage_address: StorageAddress,
        run: F,
    ) -> StorageTypedTaskFuture<'a, T>
    where
        T: Any + Send,
        F: for<'b> FnOnce(&'b mut dyn StorageContext) -> StorageContextFuture<'b, T>
            + Send
            + 'static;

    fn execute_typed_for_key_with_metadata<'a, T, F>(
        &'a self,
        storage_address: StorageAddress,
        metadata: StorageTaskMetadata,
        run: F,
    ) -> StorageTypedTaskFuture<'a, T>
    where
        T: Any + Send,
        F: for<'b> FnOnce(&'b mut dyn StorageContext) -> StorageContextFuture<'b, T>
            + Send
            + 'static;

    fn execute_typed_for_keys<'a, T, F>(
        &'a self,
        storage_addresses: &'a [StorageAddress],
        run: F,
    ) -> StorageTypedTaskFuture<'a, T>
    where
        T: Any + Send,
        F: for<'b> FnOnce(&'b mut dyn StorageContext) -> StorageContextFuture<'b, T>
            + Send
            + 'static;

    /// Executes one typed mutation for a set of addresses that must share a
    /// worker. The runtime rejects a set that would cross worker boundaries.
    fn execute_typed_mutation_for_keys<'a, T, F>(
        &'a self,
        storage_addresses: &'a [StorageAddress],
        run: F,
    ) -> StorageTypedTaskFuture<'a, T>
    where
        T: Any + Send,
        F: for<'b> FnOnce(&'b mut dyn StorageContext) -> StorageContextFuture<'b, T>
            + Send
            + 'static;

    fn execute_typed_for_keys_with_metadata<'a, T, F>(
        &'a self,
        storage_addresses: &'a [StorageAddress],
        metadata: StorageTaskMetadata,
        run: F,
    ) -> StorageTypedTaskFuture<'a, T>
    where
        T: Any + Send,
        F: for<'b> FnOnce(&'b mut dyn StorageContext) -> StorageContextFuture<'b, T>
            + Send
            + 'static;

    fn execute_typed_unbound<'a, T, F>(&'a self, run: F) -> StorageTypedTaskFuture<'a, T>
    where
        T: Any + Send,
        F: for<'b> FnOnce(&'b mut dyn StorageContext) -> StorageContextFuture<'b, T>
            + Send
            + 'static;

    fn execute_typed_unbound_with_metadata<'a, T, F>(
        &'a self,
        metadata: StorageTaskMetadata,
        run: F,
    ) -> StorageTypedTaskFuture<'a, T>
    where
        T: Any + Send,
        F: for<'b> FnOnce(&'b mut dyn StorageContext) -> StorageContextFuture<'b, T>
            + Send
            + 'static;
}

impl<P: StoragePort + ?Sized> StoragePortExt for P {
    fn execute_typed_for_key<'a, T, F>(
        &'a self,
        storage_address: StorageAddress,
        run: F,
    ) -> StorageTypedTaskFuture<'a, T>
    where
        T: Any + Send,
        F: for<'b> FnOnce(&'b mut dyn StorageContext) -> StorageContextFuture<'b, T>
            + Send
            + 'static,
    {
        self.execute_typed_for_key_with_metadata(
            storage_address,
            StorageTaskMetadata::keyed_read(),
            run,
        )
    }

    fn execute_typed_mutation_for_key<'a, T, F>(
        &'a self,
        storage_address: StorageAddress,
        run: F,
    ) -> StorageTypedTaskFuture<'a, T>
    where
        T: Any + Send,
        F: for<'b> FnOnce(&'b mut dyn StorageContext) -> StorageContextFuture<'b, T>
            + Send
            + 'static,
    {
        self.execute_typed_for_key_with_metadata(
            storage_address,
            StorageTaskMetadata::worker_serialized_key_mutation(),
            run,
        )
    }

    fn execute_typed_for_key_with_metadata<'a, T, F>(
        &'a self,
        storage_address: StorageAddress,
        metadata: StorageTaskMetadata,
        run: F,
    ) -> StorageTypedTaskFuture<'a, T>
    where
        T: Any + Send,
        F: for<'b> FnOnce(&'b mut dyn StorageContext) -> StorageContextFuture<'b, T>
            + Send
            + 'static,
    {
        Box::pin(async move {
            let output = self
                .execute_for_key(
                    storage_address,
                    StorageTask::typed_with_metadata(metadata, run),
                )
                .await?;
            downcast_storage_output(output)
        })
    }

    fn execute_typed_for_keys<'a, T, F>(
        &'a self,
        storage_addresses: &'a [StorageAddress],
        run: F,
    ) -> StorageTypedTaskFuture<'a, T>
    where
        T: Any + Send,
        F: for<'b> FnOnce(&'b mut dyn StorageContext) -> StorageContextFuture<'b, T>
            + Send
            + 'static,
    {
        self.execute_typed_for_keys_with_metadata(
            storage_addresses,
            StorageTaskMetadata::key_set_read(),
            run,
        )
    }

    fn execute_typed_mutation_for_keys<'a, T, F>(
        &'a self,
        storage_addresses: &'a [StorageAddress],
        run: F,
    ) -> StorageTypedTaskFuture<'a, T>
    where
        T: Any + Send,
        F: for<'b> FnOnce(&'b mut dyn StorageContext) -> StorageContextFuture<'b, T>
            + Send
            + 'static,
    {
        self.execute_typed_for_keys_with_metadata(
            storage_addresses,
            StorageTaskMetadata::worker_serialized_mutation(),
            run,
        )
    }

    fn execute_typed_for_keys_with_metadata<'a, T, F>(
        &'a self,
        storage_addresses: &'a [StorageAddress],
        metadata: StorageTaskMetadata,
        run: F,
    ) -> StorageTypedTaskFuture<'a, T>
    where
        T: Any + Send,
        F: for<'b> FnOnce(&'b mut dyn StorageContext) -> StorageContextFuture<'b, T>
            + Send
            + 'static,
    {
        Box::pin(async move {
            let output = self
                .execute_for_keys(
                    storage_addresses,
                    StorageTask::typed_with_metadata(metadata, run),
                )
                .await?;
            downcast_storage_output(output)
        })
    }

    fn execute_typed_unbound<'a, T, F>(&'a self, run: F) -> StorageTypedTaskFuture<'a, T>
    where
        T: Any + Send,
        F: for<'b> FnOnce(&'b mut dyn StorageContext) -> StorageContextFuture<'b, T>
            + Send
            + 'static,
    {
        self.execute_typed_unbound_with_metadata(StorageTaskMetadata::read_only(), run)
    }

    fn execute_typed_unbound_with_metadata<'a, T, F>(
        &'a self,
        metadata: StorageTaskMetadata,
        run: F,
    ) -> StorageTypedTaskFuture<'a, T>
    where
        T: Any + Send,
        F: for<'b> FnOnce(&'b mut dyn StorageContext) -> StorageContextFuture<'b, T>
            + Send
            + 'static,
    {
        Box::pin(async move {
            let output = self
                .execute_unbound(StorageTask::typed_with_metadata(metadata, run))
                .await?;
            downcast_storage_output(output)
        })
    }
}
