//! Task metadata and typed worker closures for the runtime-neutral storage port.
//!
//! The lower-level [`super::storage_port`] module owns the object-safe byte
//! and context capabilities. Keeping task construction here makes the
//! submission contract easier to scan and leaves the runtime with one
//! explicit task type.

use std::any::Any;

use super::storage_port::{
    StorageContext, StorageContextFuture, StorageTaskCancellation, StorageTaskFuture,
    StorageTaskIsolation, StorageTaskOutput, StorageTaskScheduling, StorageTaskScope,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code)]
pub(crate) struct StorageTaskMetadata {
    scheduling: StorageTaskScheduling,
    scope: StorageTaskScope,
    cancellation: StorageTaskCancellation,
    isolation: StorageTaskIsolation,
}

#[allow(dead_code)]
impl StorageTaskMetadata {
    /// Metadata for an API-owned task that does not need a compatibility
    /// observability bucket.
    pub(crate) const fn generic() -> Self {
        Self {
            scheduling: StorageTaskScheduling::Exclusive,
            scope: StorageTaskScope::Unbound,
            cancellation: StorageTaskCancellation::CompleteOnceSubmitted,
            isolation: StorageTaskIsolation::None,
        }
    }

    /// Metadata for a read-only API-owned task.
    ///
    /// The task API deliberately does not accept the server's closed
    /// observability enum. Runtime-owned adapters may attach a metric bucket
    /// directly to the task when they need one.
    pub(crate) const fn read_only() -> Self {
        Self {
            scheduling: StorageTaskScheduling::ReadOnly,
            scope: StorageTaskScope::Unbound,
            cancellation: StorageTaskCancellation::CompleteOnceSubmitted,
            isolation: StorageTaskIsolation::None,
        }
    }

    /// Metadata for a keyed read that may share the worker read lane.
    pub(crate) const fn keyed_read() -> Self {
        Self::read_only().with_scope(StorageTaskScope::SingleKey)
    }

    /// Metadata for a read-only batch whose keys are routed as one set.
    pub(crate) const fn key_set_read() -> Self {
        Self::read_only().with_scope(StorageTaskScope::KeySet)
    }

    /// Metadata for a mutation that must be serialized for its
    /// worker scope. This does not imply crash-safe rollback.
    pub(crate) const fn worker_serialized(scope: StorageTaskScope) -> Self {
        Self::generic()
            .with_scope(scope)
            .with_isolation(StorageTaskIsolation::WorkerSerialized)
    }

    /// Metadata for a worker-serialized mutation routed by a set of storage keys.
    pub(crate) const fn worker_serialized_mutation() -> Self {
        Self::worker_serialized(StorageTaskScope::KeySet)
    }

    /// Metadata for a worker-serialized mutation routed by one storage key.
    pub(crate) const fn worker_serialized_key_mutation() -> Self {
        Self::worker_serialized(StorageTaskScope::SingleKey)
    }

    /// Validates that a submission path preserves the task's declared scope
    /// and worker-serialization guarantee.
    ///
    /// Worker-serialized work must be routed through a keyed worker lane. An
    /// unbound task has no stable serialization domain, and a read-only task
    /// cannot truthfully declare worker-local serialization.
    pub(crate) fn validate_submission(
        self,
        submitted_scope: StorageTaskScope,
    ) -> std::result::Result<(), &'static str> {
        if self.scope != submitted_scope {
            return Err("storage task has incompatible submission scope");
        }
        if self.isolation == StorageTaskIsolation::WorkerSerialized
            && (submitted_scope == StorageTaskScope::Unbound
                || self.scheduling == StorageTaskScheduling::ReadOnly)
        {
            return Err("worker-serialized storage task must use an exclusive keyed worker lane");
        }
        Ok(())
    }

    pub(crate) const fn with_scope(self, scope: StorageTaskScope) -> Self {
        Self { scope, ..self }
    }

    pub(crate) const fn with_cancellation(self, cancellation: StorageTaskCancellation) -> Self {
        Self {
            cancellation,
            ..self
        }
    }

    pub(crate) const fn with_isolation(self, isolation: StorageTaskIsolation) -> Self {
        Self { isolation, ..self }
    }

    pub(crate) const fn scheduling(self) -> StorageTaskScheduling {
        self.scheduling
    }

    pub(crate) const fn scope(self) -> StorageTaskScope {
        self.scope
    }

    pub(crate) const fn cancellation(self) -> StorageTaskCancellation {
        self.cancellation
    }

    pub(crate) const fn isolation(self) -> StorageTaskIsolation {
        self.isolation
    }
}

/// API-owned storage work submitted to a worker.
pub(crate) struct StorageTask {
    metadata: StorageTaskMetadata,
    run: Box<dyn for<'a> FnOnce(&'a mut dyn StorageContext) -> StorageTaskFuture<'a> + Send>,
}

#[allow(dead_code)]
impl StorageTask {
    /// Builds a task while preserving its API-owned result type.
    ///
    /// The worker still transports one erased result envelope, but an API
    /// handler does not need to write the boxing/downcast boilerplate for
    /// every storage-backed operation.
    pub(crate) fn typed<T: Any + Send>(
        run: impl for<'a> FnOnce(&'a mut dyn StorageContext) -> StorageContextFuture<'a, T>
        + Send
        + 'static,
    ) -> Self {
        Self::typed_with_metadata(StorageTaskMetadata::generic(), run)
    }

    /// Builds a typed task with an explicit scheduling/scope contract.
    pub(crate) fn typed_with_metadata<T: Any + Send>(
        metadata: StorageTaskMetadata,
        run: impl for<'a> FnOnce(&'a mut dyn StorageContext) -> StorageContextFuture<'a, T>
        + Send
        + 'static,
    ) -> Self {
        Self::new(move |context| {
            Box::pin(async move {
                run(context)
                    .await
                    .map(|value| Box::new(value) as StorageTaskOutput)
            })
        })
        .with_metadata(metadata)
    }

    pub(crate) fn new(
        run: impl for<'a> FnOnce(&'a mut dyn StorageContext) -> StorageTaskFuture<'a> + Send + 'static,
    ) -> Self {
        Self {
            metadata: StorageTaskMetadata::generic(),
            run: Box::new(run),
        }
    }

    pub(crate) fn with_metadata(mut self, metadata: StorageTaskMetadata) -> Self {
        self.metadata = metadata;
        self
    }

    pub(in crate::runtime) const fn metadata(&self) -> StorageTaskMetadata {
        self.metadata
    }

    pub(in crate::runtime) fn execute(
        self,
        context: &mut dyn StorageContext,
    ) -> StorageTaskFuture<'_> {
        (self.run)(context)
    }
}
