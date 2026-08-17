//! Generic bounded request admission and completion polling.
//!
//! This module moves an already-built request into a bounded queue and retains
//! only its reusable completion slot. Callers own request construction,
//! telemetry, deadlines, and domain error projection.

use std::future::Future;

use crate::channel::{Sender, TrySendError};

use super::completion::{
    CompletionDisconnected, CompletionReceiver, CompletionSender, CompletionSlab,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(super) enum AdmissionError {
    #[error("request queue is full")]
    QueueFull,
    #[error("completion capacity is exhausted")]
    CompletionFull,
    #[error("request queue is disconnected")]
    Disconnected,
}

pub(super) struct PendingCompletion<'a, T> {
    response: CompletionReceiver<'a, T>,
}

impl<T: Unpin> Future for PendingCompletion<'_, T> {
    type Output = std::result::Result<T, CompletionDisconnected>;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        #[cfg(not(feature = "network-runtime-kimojio"))]
        return std::pin::Pin::new(&mut self.response).poll(context);

        #[cfg(feature = "network-runtime-kimojio")]
        {
            match self.response.try_recv() {
                Ok(Some(result)) => std::task::Poll::Ready(Ok(result)),
                Err(error) => std::task::Poll::Ready(Err(error)),
                Ok(None) => {
                    let mut yielding = std::pin::pin!(kimojio::operations::yield_io());
                    let yielded = yielding.as_mut().poll(context);
                    debug_assert!(yielded.is_pending());
                    std::task::Poll::Pending
                }
            }
        }
    }
}

pub(super) fn try_submit<'a, Q, T>(
    completions: &'a CompletionSlab<T>,
    sender: &Sender<Q>,
    build: impl FnOnce(CompletionSender<T>) -> Q,
) -> std::result::Result<PendingCompletion<'a, T>, AdmissionError>
where
    Q: Send + Unpin + 'static,
{
    let Some((response_tx, response_rx)) = completions.try_register() else {
        return Err(AdmissionError::CompletionFull);
    };
    match sender.try_send(build(response_tx)) {
        Ok(()) => Ok(PendingCompletion {
            response: response_rx,
        }),
        Err(TrySendError::Full(_)) => Err(AdmissionError::QueueFull),
        Err(TrySendError::Disconnected(_)) => Err(AdmissionError::Disconnected),
    }
}
