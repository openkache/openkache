//! Worker-local lifecycle state and monotonic transitions.

use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub(super) enum Lifecycle {
    Starting = 0,
    Ready = 1,
    Degraded = 2,
    Draining = 3,
    Failed = 4,
}

impl Lifecycle {
    pub(super) fn from_raw(value: u8) -> Self {
        match value {
            1 => Self::Ready,
            2 => Self::Degraded,
            3 => Self::Draining,
            4 => Self::Failed,
            _ => Self::Starting,
        }
    }

    pub(super) fn is_up(self) -> bool {
        matches!(self, Self::Ready | Self::Degraded | Self::Draining)
    }

    #[cfg(feature = "opentelemetry")]
    pub(super) const fn is_ready(self) -> bool {
        matches!(self, Self::Ready)
    }

    #[cfg(feature = "opentelemetry")]
    pub(super) const fn is_degraded(self) -> bool {
        matches!(self, Self::Degraded)
    }

    #[cfg(feature = "opentelemetry")]
    pub(super) const fn is_draining(self) -> bool {
        matches!(self, Self::Draining)
    }

    #[cfg(feature = "opentelemetry")]
    pub(super) const fn is_failed(self) -> bool {
        matches!(self, Self::Failed)
    }
}

/// A lifecycle cell owned by one worker and read by the management plane.
///
/// Transitions are monotonic at terminal boundaries: a failed worker cannot
/// become ready again, and draining cannot be replaced by a non-failure state.
/// A failure discovered while draining is still allowed to win.
pub(super) struct LifecycleCell {
    state: AtomicU8,
}

impl LifecycleCell {
    pub(super) fn new() -> Self {
        Self {
            state: AtomicU8::new(Lifecycle::Starting as u8),
        }
    }

    pub(super) fn load(&self) -> Lifecycle {
        Lifecycle::from_raw(self.state.load(Ordering::Acquire))
    }

    pub(super) fn transition(&self, target: Lifecycle) {
        let mut current = self.state.load(Ordering::Acquire);
        loop {
            let current_state = Lifecycle::from_raw(current);
            if current_state == Lifecycle::Failed
                || (current_state == Lifecycle::Draining && target != Lifecycle::Failed)
            {
                return;
            }
            match self.state.compare_exchange(
                current,
                target as u8,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return,
                Err(next) => current = next,
            }
        }
    }
}
