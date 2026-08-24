//! Quiescent worker control.
//!
//! Keyed scheduling stays in `worker.rs`; this module owns the control barrier
//! used when a worker is quiescent.

use super::worker::{ControlFlow, ControlPort};
use super::worker_contract::ResponseSender;
use super::{Kvkache, WorkerControlResponse, WorkerResponse};
use crate::Result;

pub(super) enum ControlRequest<R> {
    ExperimentalStats {
        response: ResponseSender<R>,
    },
    ExperimentalSync {
        response: ResponseSender<R>,
    },
    Shutdown,
}

impl ControlPort<ControlRequest<WorkerResponse>> for Kvkache {
    fn execute_control(
        &mut self,
        command: ControlRequest<WorkerResponse>,
        affinity_id: usize,
    ) -> impl std::future::Future<Output = Result<ControlFlow>> + '_ {
        async move {
            match command {
                ControlRequest::ExperimentalStats { response } => {
                    let stats = format!(
                        "{} {}",
                        crate::platform::cpu_diagnostic(affinity_id),
                        self.stats()
                    );
                    let _ = response.send(Ok(WorkerResponse::Control(
                        WorkerControlResponse::ExperimentalStats(stats),
                    )));
                    Ok(ControlFlow::Continue)
                }
                ControlRequest::ExperimentalSync { response } => {
                    let result = self
                        .sync()
                        .await
                        .map(|()| WorkerResponse::Control(WorkerControlResponse::ExperimentalSynced));
                    let _ = response.send(result);
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
