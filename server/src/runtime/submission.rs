//! Requester-free keyed storage submission.
//!
//! Callers own workload construction, windowing, and result aggregation. This
//! module only routes one already-derived storage key through the production
//! keyed scheduler and separates bounded enqueue from completion.

use std::time::{Duration, Instant};

use super::completion::CompletionReceiver;
use super::{ThreadedKvkache, WorkerRequest, WorkerResponse, WorkerResponseSender, keyed_storage};
use crate::network_runtime;
use crate::observability::Operation;
use crate::types::{StorageWriteOptions, StoredItemValue};
use crate::{KvError, Result, SetOutcome, StorageKey};

/// One storage-read value retaining its backend allocation.
///
/// The value borrows the backend-owned bytes for its lifetime. Dropping the
/// wrapper releases that ownership; it never materializes a second buffer.
pub struct SubmittedStorageValue(StoredItemValue);

impl AsRef<[u8]> for SubmittedStorageValue {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl std::fmt::Debug for SubmittedStorageValue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_tuple("SubmittedStorageValue")
            .field(&self.as_ref())
            .finish()
    }
}

/// One completed storage submission and its enqueue-to-completion latency.
#[derive(Debug)]
pub struct StorageSubmission<T> {
    /// Typed storage result.
    pub output: T,
    /// Nanoseconds from the start of bounded enqueue through response receipt.
    pub latency_ns: u64,
}

/// In-flight keyed storage work whose enqueue has completed.
///
/// Keeping completion separate lets callers apply their own bounded admission
/// policy without exposing worker requests, completion slots, or scheduler
/// internals.
///
/// Dropping this handle stops observation, not execution: once enqueue
/// succeeds, a write or delete may still commit after the handle is dropped or
/// its output timeout elapses. Callers that require a mutation result must
/// retain the handle and call [`Self::complete`].
pub struct PendingStorageSubmission<'a, T> {
    response: CompletionReceiver<'a, Result<WorkerResponse>>,
    decode: fn(WorkerResponse) -> Result<T>,
    started: Instant,
    request_limit: Duration,
    output_limit: Duration,
}

impl<T> PendingStorageSubmission<'_, T> {
    /// Waits for the typed result within the remaining request/output budget.
    pub async fn complete(self) -> Result<StorageSubmission<T>> {
        let remaining = self
            .request_limit
            .saturating_sub(self.started.elapsed())
            .min(self.output_limit);
        let response = network_runtime::timeout(remaining, self.response.recv_async_network())
            .await
            .map_err(|_| KvError::Timeout("storage submission output"))?
            .map_err(|_| KvError::Worker("storage submission response disconnected".into()))??;
        let output = (self.decode)(response)?;
        Ok(StorageSubmission {
            output,
            latency_ns: self.started.elapsed().as_nanos() as u64,
        })
    }
}

impl ThreadedKvkache {
    async fn submit_storage<T>(
        &self,
        storage_key: StorageKey,
        build: impl FnOnce(WorkerResponseSender) -> keyed_storage::Command,
        decode: fn(WorkerResponse) -> Result<T>,
    ) -> Result<PendingStorageSubmission<'_, T>> {
        let worker = self.owner(&storage_key);
        let (response_tx, response_rx) = self.workers[worker].completions.register();
        let started = Instant::now();
        let request = WorkerRequest::Keyed {
            storage_key,
            command: build(response_tx),
        };
        network_runtime::timeout(
            Duration::from_micros(self.config.timeouts.input_max_time_us),
            self.workers[worker].sender.send_async_network(request),
        )
        .await
        .map_err(|_| KvError::Timeout("storage submission input"))?
        .map_err(|_| KvError::Worker("storage submission queue disconnected".into()))?;
        Ok(PendingStorageSubmission {
            response: response_rx,
            decode,
            started,
            request_limit: Duration::from_micros(self.config.timeouts.request_max_time_us),
            output_limit: Duration::from_micros(self.config.timeouts.output_max_time_us),
        })
    }

    /// Submits a read through the production keyed scheduler.
    ///
    /// The returned handle owns the completion observation and can be polled
    /// independently of other submissions.
    pub async fn submit_storage_read(
        &self,
        storage_key: StorageKey,
    ) -> Result<PendingStorageSubmission<'_, Option<SubmittedStorageValue>>> {
        self.submit_storage(
            storage_key,
            |response| keyed_storage::get(Operation::unknown(), response),
            |response| {
                keyed_storage::value_response(response, "storage read")
                    .map(|value| value.map(SubmittedStorageValue))
            },
        )
        .await
    }

    /// Submits a write while moving the value allocation into storage.
    ///
    /// `value` is moved into the storage pipeline exactly once. If completion
    /// is abandoned after enqueue, the mutation may still be applied.
    pub async fn submit_storage_write(
        &self,
        storage_key: StorageKey,
        value: Vec<u8>,
        options: StorageWriteOptions,
    ) -> Result<PendingStorageSubmission<'_, SetOutcome>> {
        self.submit_storage(
            storage_key,
            |response| {
                keyed_storage::set(
                    Operation::unknown(),
                    StoredItemValue::new(value),
                    options,
                    response,
                )
            },
            |response| keyed_storage::set_response(response, "storage write"),
        )
        .await
    }

    /// Submits a deletion through the production keyed scheduler.
    ///
    /// If completion is abandoned after enqueue, the deletion may still be
    /// applied even though its outcome is not observed.
    pub async fn submit_storage_remove(
        &self,
        storage_key: StorageKey,
    ) -> Result<PendingStorageSubmission<'_, bool>> {
        self.submit_storage(
            storage_key,
            |response| keyed_storage::delete(Operation::unknown(), response),
            |response| keyed_storage::delete_response(response, "storage delete"),
        )
        .await
    }
}
