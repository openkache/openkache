//! Quiescent worker control and neutral storage-task execution.
//!
//! Keyed scheduling stays in `worker.rs`; this module owns only the control
//! barrier and the runtime-neutral storage context construction used when a
//! worker is quiescent.

use super::storage_backend::RuntimeStorageBackend;
use super::storage_context::StorageWorkerContext;
use super::{Kvkache, StorageTask, StorageTaskCancellation};
use super::{WorkerControlRequest, WorkerRequest, WorkerResponse};
use crate::Result;

/// Runs one API-owned task against the worker-local backend.
///
/// Both keyed extension tasks and quiescent control tasks use the same
/// execution path. Keeping backend/context construction here prevents a new
/// storage capability from adding another worker-specific command branch.
pub(super) async fn execute_storage_task(cache: &mut Kvkache, task: StorageTask) -> WorkerResponse {
    let metadata = task.metadata();
    let mut backend = RuntimeStorageBackend::new(cache);
    let mut context = StorageWorkerContext::new(&mut backend, metadata);
    match task.execute(&mut context).await {
        Ok(output) => WorkerResponse::StorageResult(output),
        Err(error) => WorkerResponse::StorageFailure(error),
    }
}

pub(super) async fn process_worker_barrier(
    cache: &mut Kvkache,
    request: WorkerRequest,
    affinity_id: usize,
) -> Result<bool> {
    match request {
        WorkerRequest::Control(WorkerControlRequest::Stats { response }) => {
            let _ = response.send(Ok(WorkerResponse::Stats(format!(
                "{} {}",
                crate::platform::cpu_diagnostic(affinity_id),
                cache.stats()
            ))));
            Ok(false)
        }
        WorkerRequest::Control(WorkerControlRequest::Sync { response }) => {
            let result = cache.sync().await.map(|()| WorkerResponse::Synced);
            let _ = response.send(result);
            Ok(false)
        }
        WorkerRequest::Control(WorkerControlRequest::StorageTask { task, response }) => {
            if task.metadata().cancellation() == StorageTaskCancellation::CancelIfDisconnected
                && response.is_disconnected()
            {
                return Ok(false);
            }
            let result = execute_storage_task(cache, task).await;
            let _ = response.send(Ok(result));
            Ok(false)
        }
        WorkerRequest::Control(WorkerControlRequest::Shutdown) => {
            cache.sync().await?;
            cache.checkpoint().await?;
            Ok(true)
        }
        WorkerRequest::Keyed { .. } => Err(crate::KvError::Worker(
            "keyed request reached the worker barrier path".into(),
        )),
    }
}
