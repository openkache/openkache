//! Generic server-owned behavior types.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use super::operation_handlers::OperationContext;
use super::operation_outcome::OperationOutcome;

/// Future returned by an operation binding.
///
/// Immediate API handlers use the inline `Ready` variant and therefore do not
/// allocate a boxed future. Storage-backed compatibility handlers use
/// `Pending`, which keeps the same erased async boundary for operations that
/// genuinely suspend.
pub(super) enum OperationFuture<'a> {
    Ready(Option<OperationOutcome>),
    Pending(Pin<Box<dyn Future<Output = OperationOutcome> + 'a>>),
}

impl<'a> OperationFuture<'a> {
    pub(super) fn ready(outcome: OperationOutcome) -> Self {
        Self::Ready(Some(outcome))
    }

    pub(super) fn pending(future: Pin<Box<dyn Future<Output = OperationOutcome> + 'a>>) -> Self {
        Self::Pending(future)
    }
}

impl Future for OperationFuture<'_> {
    type Output = OperationOutcome;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        match self.get_mut() {
            Self::Ready(outcome) => Poll::Ready(
                outcome
                    .take()
                    .expect("operation ready future was polled after completion"),
            ),
            Self::Pending(future) => future.as_mut().poll(context),
        }
    }
}

/// One operation behavior slot.
pub(super) type OperationHandler = for<'a> fn(OperationContext<'a>) -> OperationFuture<'a>;
