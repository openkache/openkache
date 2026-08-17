//! Bounded network-to-storage request admission.
//!
//! Request payloads move into the worker queue before this module returns.
//! The resulting future retains only completion and telemetry state, keeping
//! server operation futures compact without a heap fallback.

use std::future::Future;

use crate::channel::TrySendError;
use crate::observability::{NetworkWorkerId, Operation};
use crate::{KvError, Result};

use super::completion::{CompletionDisconnected, CompletionReceiver};
use super::{ThreadedKvkache, WorkerRequest, WorkerResponse, WorkerResponseSender};

#[derive(Clone, Copy, Debug, thiserror::Error)]
pub(in crate::runtime) enum RequestAdmissionError {
    #[error("storage request queue is full")]
    QueueFull,
    #[error("storage completion capacity is exhausted")]
    CompletionFull,
    #[error("storage request queue is disconnected")]
    Disconnected,
}

impl From<RequestAdmissionError> for KvError {
    fn from(error: RequestAdmissionError) -> Self {
        match error {
            RequestAdmissionError::QueueFull => Self::CapacityExhausted {
                resource: "storage request queue",
            },
            RequestAdmissionError::CompletionFull => Self::CapacityExhausted {
                resource: "storage completion capacity",
            },
            RequestAdmissionError::Disconnected => {
                Self::Worker("request queue disconnected".into())
            }
        }
    }
}

pub(in crate::runtime) struct PendingWorkerResponse<'a> {
    response: CompletionReceiver<'a, Result<WorkerResponse>>,
    cache: &'a ThreadedKvkache,
    requester: NetworkWorkerId,
    worker: usize,
    operation: Operation,
    started: std::time::Instant,
    wait_recorded: bool,
}

impl PendingWorkerResponse<'_> {
    fn record_wait(&self) {
        if let Some(observability) = self.cache.observability.as_ref() {
            observability.record_storage_wait(
                self.requester.index(),
                self.worker,
                self.operation,
                self.started.elapsed(),
            );
        }
    }

    fn project(
        result: std::result::Result<Result<WorkerResponse>, CompletionDisconnected>,
    ) -> Result<WorkerResponse> {
        result
            .map_err(|_| KvError::Worker("worker response disconnected".into()))
            .and_then(|result| result)
    }
}

impl Future for PendingWorkerResponse<'_> {
    type Output = Result<WorkerResponse>;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        #[cfg(not(feature = "network-runtime-kimojio"))]
        let result = std::task::ready!(std::pin::Pin::new(&mut self.response).poll(context));

        #[cfg(feature = "network-runtime-kimojio")]
        let result = match self.response.try_recv() {
            Ok(Some(result)) => Ok(result),
            Err(error) => Err(error),
            Ok(None) => {
                let mut yielding = std::pin::pin!(kimojio::operations::yield_io());
                let yielded = yielding.as_mut().poll(context);
                debug_assert!(yielded.is_pending());
                return std::task::Poll::Pending;
            }
        };

        self.record_wait();
        self.wait_recorded = true;
        std::task::Poll::Ready(Self::project(result))
    }
}

impl Drop for PendingWorkerResponse<'_> {
    fn drop(&mut self) {
        if !self.wait_recorded {
            self.record_wait();
        }
    }
}

impl ThreadedKvkache {
    /// Submits bounded network work without retaining its payload or enqueue
    /// future across an await. The outer request deadline bounds completion.
    pub(in crate::runtime) fn try_network_request(
        &self,
        worker: usize,
        operation: Operation,
        requester: NetworkWorkerId,
        build: impl FnOnce(WorkerResponseSender) -> WorkerRequest,
    ) -> std::result::Result<PendingWorkerResponse<'_>, RequestAdmissionError> {
        let request_started = std::time::Instant::now();
        let Some((response_tx, response_rx)) = self.workers[worker].completions.try_register()
        else {
            if let Some(observability) = self.observability.as_ref() {
                observability.record_storage_wait(
                    requester.index(),
                    worker,
                    operation,
                    request_started.elapsed(),
                );
            }
            return Err(RequestAdmissionError::CompletionFull);
        };
        match self.workers[worker].sender.try_send(build(response_tx)) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                if let Some(observability) = self.observability.as_ref() {
                    observability.storage_queue_full(requester.index(), worker);
                    observability.record_storage_wait(
                        requester.index(),
                        worker,
                        operation,
                        request_started.elapsed(),
                    );
                }
                return Err(RequestAdmissionError::QueueFull);
            }
            Err(TrySendError::Disconnected(_)) => {
                if let Some(observability) = self.observability.as_ref() {
                    observability.record_storage_wait(
                        requester.index(),
                        worker,
                        operation,
                        request_started.elapsed(),
                    );
                }
                return Err(RequestAdmissionError::Disconnected);
            }
        }

        Ok(PendingWorkerResponse {
            response: response_rx,
            cache: self,
            requester,
            worker,
            operation,
            started: request_started,
            wait_recorded: false,
        })
    }
}
