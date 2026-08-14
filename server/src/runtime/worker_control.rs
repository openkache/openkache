//! Quiescent worker control and neutral storage-task execution.
//!
//! Keyed scheduling stays in `worker.rs`; this module owns only the control
//! barrier and the runtime-neutral storage context construction used when a
//! worker is quiescent.

use super::storage_backend::RuntimeStorageBackend;
use super::storage_context::StorageWorkerContext;
use super::worker::{ControlFlow, ControlPort, ResponseSender};
use super::{Kvkache, StorageTask, StorageTaskCancellation, WorkerResponse};
use crate::Result;

pub(super) enum ControlRequest<R> {
    Stats {
        response: ResponseSender<R>,
    },
    Sync {
        response: ResponseSender<R>,
    },
    /// Executes an API-owned storage task after all keyed work is quiescent.
    ///
    /// Keyed data-plane work keeps its scheduler and collapse optimizations.
    /// Extensions use this escape hatch for storage shapes that do not fit one
    /// keyed lane without adding another worker-owned command variant.
    StorageTask {
        task: StorageTask,
        response: ResponseSender<R>,
    },
    Shutdown,
}

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

impl ControlPort<ControlRequest<WorkerResponse>> for Kvkache {
    fn execute_control(
        &mut self,
        command: ControlRequest<WorkerResponse>,
        affinity_id: usize,
    ) -> impl std::future::Future<Output = Result<ControlFlow>> + '_ {
        async move {
            match command {
                ControlRequest::Stats { response } => {
                    let stats = format!(
                        "{} {}",
                        crate::platform::cpu_diagnostic(affinity_id),
                        self.stats()
                    );
                    let _ = response.send(Ok(WorkerResponse::Stats(stats)));
                    Ok(ControlFlow::Continue)
                }
                ControlRequest::Sync { response } => {
                    let result = self.sync().await.map(|()| WorkerResponse::Synced);
                    let _ = response.send(result);
                    Ok(ControlFlow::Continue)
                }
                ControlRequest::StorageTask { task, response } => {
                    if task.metadata().cancellation()
                        == StorageTaskCancellation::CancelIfDisconnected
                        && response.is_disconnected()
                    {
                        return Ok(ControlFlow::Continue);
                    }
                    let result = execute_storage_task(self, task).await;
                    let _ = response.send(Ok(result));
                    Ok(ControlFlow::Continue)
                }
                ControlRequest::Shutdown => {
                    self.sync().await?;
                    self.checkpoint().await?;
                    Ok(ControlFlow::Stop)
                }
            }
        }
    }
}
