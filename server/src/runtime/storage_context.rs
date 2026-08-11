//! Runtime-neutral execution context for generic storage tasks.
//!
//! The public storage task contract lives in [`super::storage_port`]. This
//! module owns task metadata validation and delegates byte-oriented operations
//! to a backend supplied by the runtime composition boundary.

use super::storage_port::{
    StorageAddress, StorageBatchOperation, StorageBatchResult, StorageContext,
    StorageContextFuture, StorageError, StorageMutation, StorageResult, StorageTaskIsolation,
    StorageTaskScheduling, StorageWriteOptions,
};
use super::storage_task::StorageTaskMetadata;

/// Backend contract consumed by the generic storage task context.
///
/// The task context deliberately knows only this runtime-neutral byte
/// contract. The composition layer supplies the concrete cache adapter,
/// including any protocol-policy conversion required by the active backend.
#[allow(dead_code)]
pub(super) trait StorageBackend {
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

    fn compare_and_set<'a>(
        &'a mut self,
        storage_address: StorageAddress,
        expected: Option<&'a [u8]>,
        replacement: Option<Vec<u8>>,
        options: StorageWriteOptions,
    ) -> StorageContextFuture<'a, bool>;
}

/// Runtime-neutral implementation of [`StorageContext`].
///
/// This layer owns task scheduling policy and batch/CAS validation. Concrete
/// cache objects and wire policy types stay behind [`StorageBackend`].
#[allow(dead_code)]
pub(super) struct StorageWorkerContext<'a> {
    backend: &'a mut dyn StorageBackend,
    metadata: StorageTaskMetadata,
}

#[allow(dead_code)]
impl<'a> StorageWorkerContext<'a> {
    pub(super) const fn new(
        backend: &'a mut dyn StorageBackend,
        metadata: StorageTaskMetadata,
    ) -> Self {
        Self { backend, metadata }
    }

    fn mutation_allowed(&self) -> StorageResult<()> {
        if self.metadata.scheduling() == StorageTaskScheduling::ReadOnly {
            return Err(StorageError::InvalidRequest(
                "mutation requires an exclusive storage task".into(),
            ));
        }
        Ok(())
    }

    async fn get_value(
        &mut self,
        storage_address: StorageAddress,
    ) -> StorageResult<Option<Vec<u8>>> {
        self.backend.get(storage_address).await
    }

    async fn set_value(
        &mut self,
        storage_address: StorageAddress,
        value: Vec<u8>,
        options: StorageWriteOptions,
    ) -> StorageResult<StorageMutation> {
        self.backend.set(storage_address, value, options).await
    }

    async fn delete_value(
        &mut self,
        storage_address: StorageAddress,
    ) -> StorageResult<StorageMutation> {
        self.backend.delete(storage_address).await
    }

    async fn compare_and_set_value(
        &mut self,
        storage_address: StorageAddress,
        expected: Option<&[u8]>,
        replacement: Option<Vec<u8>>,
        options: StorageWriteOptions,
    ) -> StorageResult<bool> {
        self.backend
            .compare_and_set(storage_address, expected, replacement, options)
            .await
    }

    async fn batch_values(
        &mut self,
        operations: Vec<StorageBatchOperation>,
    ) -> StorageResult<Vec<StorageBatchResult>> {
        let mut results = Vec::with_capacity(operations.len());
        for operation in operations {
            let result = match operation {
                StorageBatchOperation::Get { address } => {
                    StorageBatchResult::Value(self.get_value(address).await?)
                }
                StorageBatchOperation::Set {
                    address,
                    value,
                    options,
                } => StorageBatchResult::Mutation(self.set_value(address, value, options).await?),
                StorageBatchOperation::Delete { address } => {
                    StorageBatchResult::Mutation(self.delete_value(address).await?)
                }
                StorageBatchOperation::CompareAndSet {
                    address,
                    expected,
                    replacement,
                    options,
                } => StorageBatchResult::CompareAndSet(
                    self.compare_and_set_value(address, expected.as_deref(), replacement, options)
                        .await?,
                ),
            };
            results.push(result);
        }
        Ok(results)
    }
}

#[allow(dead_code)]
impl StorageContext for StorageWorkerContext<'_> {
    fn get<'a>(
        &'a mut self,
        storage_address: StorageAddress,
    ) -> StorageContextFuture<'a, Option<Vec<u8>>> {
        Box::pin(self.get_value(storage_address))
    }

    fn set<'a>(
        &'a mut self,
        storage_address: StorageAddress,
        value: Vec<u8>,
        options: StorageWriteOptions,
    ) -> StorageContextFuture<'a, StorageMutation> {
        if let Err(error) = self.mutation_allowed() {
            return Box::pin(async { Err(error) });
        }
        Box::pin(self.set_value(storage_address, value, options))
    }

    fn delete<'a>(
        &'a mut self,
        storage_address: StorageAddress,
    ) -> StorageContextFuture<'a, StorageMutation> {
        if let Err(error) = self.mutation_allowed() {
            return Box::pin(async { Err(error) });
        }
        Box::pin(self.delete_value(storage_address))
    }

    fn compare_and_set<'a>(
        &'a mut self,
        storage_address: StorageAddress,
        expected: Option<&'a [u8]>,
        replacement: Option<Vec<u8>>,
        options: StorageWriteOptions,
    ) -> StorageContextFuture<'a, bool> {
        if let Err(error) = self.mutation_allowed() {
            return Box::pin(async { Err(error) });
        }
        if self.metadata.isolation() != StorageTaskIsolation::WorkerSerialized {
            return Box::pin(async {
                Err(StorageError::InvalidRequest(
                    "compare_and_set requires a worker-serialized storage task".into(),
                ))
            });
        }
        Box::pin(self.compare_and_set_value(storage_address, expected, replacement, options))
    }

    fn batch<'a>(
        &'a mut self,
        operations: Vec<StorageBatchOperation>,
    ) -> StorageContextFuture<'a, Vec<StorageBatchResult>> {
        let contains_mutation = operations.iter().any(|operation| {
            matches!(
                operation,
                StorageBatchOperation::Set { .. }
                    | StorageBatchOperation::Delete { .. }
                    | StorageBatchOperation::CompareAndSet { .. }
            )
        });
        if contains_mutation {
            if let Err(error) = self.mutation_allowed() {
                return Box::pin(async { Err(error) });
            }
        }
        if operations
            .iter()
            .any(|operation| matches!(operation, StorageBatchOperation::CompareAndSet { .. }))
            && self.metadata.isolation() != StorageTaskIsolation::WorkerSerialized
        {
            return Box::pin(async {
                Err(StorageError::InvalidRequest(
                    "compare_and_set requires a worker-serialized storage task".into(),
                ))
            });
        }
        Box::pin(self.batch_values(operations))
    }
}
