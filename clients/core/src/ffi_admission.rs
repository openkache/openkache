//! Atomic admission state for asynchronous native operations.
//!
//! This state machine is kept separate from the FFI worker so the cancellation
//! and writer admission transition has one compare-and-swap boundary. A
//! cancellation that wins while the request is pending prevents the worker
//! from claiming the request; a cancellation after the worker claims it keeps
//! the outcome ambiguous for mutations.

use std::sync::atomic::{AtomicU8, Ordering};

/// One request's worker-admission state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub(crate) enum AdmissionState {
    /// No worker has claimed the request and cancellation has not won.
    Pending = 0,
    /// The worker claimed the request and may transmit it.
    Started = 1,
    /// Cancellation won before worker admission.
    Canceled = 2,
    /// The worker claimed the request before cancellation won.
    StartedCanceled = 3,
}

impl TryFrom<u8> for AdmissionState {
    type Error = ();

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Pending),
            1 => Ok(Self::Started),
            2 => Ok(Self::Canceled),
            3 => Ok(Self::StartedCanceled),
            _ => Err(()),
        }
    }
}

/// Atomic request admission shared by the FFI caller and worker.
pub(crate) struct FfiAdmission {
    state: AtomicU8,
}

impl FfiAdmission {
    /// Creates an admission in the cancellable pending state.
    pub(crate) const fn new() -> Self {
        Self {
            state: AtomicU8::new(AdmissionState::Pending as u8),
        }
    }

    /// Claims the request for worker execution.
    ///
    /// A `false` result means cancellation won before admission and the worker
    /// must drop the queued command without invoking the mutation.
    pub(crate) fn try_start(&self) -> bool {
        self.state
            .compare_exchange(
                AdmissionState::Pending as u8,
                AdmissionState::Started as u8,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }

    /// Publishes cancellation and returns the resulting boundary.
    pub(crate) fn cancel(&self) -> AdmissionState {
        loop {
            let current = self.state.load(Ordering::Acquire);
            let current_state = AdmissionState::try_from(current)
                .expect("FFI request admission state must be a known discriminator");
            let next = match current_state {
                AdmissionState::Pending => AdmissionState::Canceled,
                AdmissionState::Started => AdmissionState::StartedCanceled,
                AdmissionState::Canceled | AdmissionState::StartedCanceled => return current_state,
            };
            if self
                .state
                .compare_exchange(current, next as u8, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return next;
            }
        }
    }

    /// Returns the currently published admission state.
    pub(crate) fn state(&self) -> AdmissionState {
        AdmissionState::try_from(self.state.load(Ordering::Acquire))
            .expect("FFI request admission state must be a known discriminator")
    }

    /// Returns whether cancellation has been published.
    pub(crate) fn is_canceled(&self) -> bool {
        matches!(
            self.state(),
            AdmissionState::Canceled | AdmissionState::StartedCanceled
        )
    }
}
