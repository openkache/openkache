//! Reusable cross-thread completion slots for storage worker requests.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Context, Poll, Waker};

const DEFAULT_RETAINED_CAPACITY: usize = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct CompletionDisconnected;

struct CompletionSlotState<T> {
    generation: u64,
    active: bool,
    disconnected: bool,
    value: Option<T>,
    waker: Option<Waker>,
}

impl<T> Default for CompletionSlotState<T> {
    fn default() -> Self {
        Self {
            generation: 0,
            active: false,
            disconnected: false,
            value: None,
            waker: None,
        }
    }
}

struct CompletionSlot<T> {
    state: Mutex<CompletionSlotState<T>>,
}

impl<T> Default for CompletionSlot<T> {
    fn default() -> Self {
        Self {
            state: Mutex::new(CompletionSlotState::default()),
        }
    }
}

impl<T> CompletionSlot<T> {
    fn activate(&self) -> Option<u64> {
        let mut state = lock(&self.state);
        debug_assert!(!state.active);
        state.generation = state.generation.checked_add(1)?;
        state.active = true;
        state.disconnected = false;
        state.value = None;
        state.waker = None;
        Some(state.generation)
    }

    fn complete(&self, generation: u64, value: T) -> Result<(), T> {
        let waker = {
            let mut state = lock(&self.state);
            if !state.active || state.generation != generation {
                return Err(value);
            }
            state.value = Some(value);
            state.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
        Ok(())
    }

    fn disconnect(&self, generation: u64) {
        let waker = {
            let mut state = lock(&self.state);
            if !state.active || state.generation != generation || state.value.is_some() {
                return;
            }
            state.disconnected = true;
            state.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }

    fn poll(&self, generation: u64, cx: &Context<'_>) -> Poll<Result<T, CompletionDisconnected>> {
        let mut state = lock(&self.state);
        if !state.active || state.generation != generation || state.disconnected {
            return Poll::Ready(Err(CompletionDisconnected));
        }
        if let Some(value) = state.value.take() {
            return Poll::Ready(Ok(value));
        }
        if state
            .waker
            .as_ref()
            .is_none_or(|waker| !waker.will_wake(cx.waker()))
        {
            state.waker = Some(cx.waker().clone());
        }
        Poll::Pending
    }

    fn deactivate(&self, generation: u64) -> bool {
        let mut state = lock(&self.state);
        if !state.active || state.generation != generation {
            return false;
        }
        state.active = false;
        state.disconnected = false;
        state.value = None;
        state.waker = None;
        true
    }
}

struct CompletionSlabState<T> {
    free: Vec<Arc<CompletionSlot<T>>>,
    retained_capacity: usize,
}

impl<T> CompletionSlabState<T> {
    fn with_retained_capacity(retained_capacity: usize) -> Self {
        Self {
            free: Vec::new(),
            retained_capacity,
        }
    }
}

pub(super) struct CompletionSlab<T> {
    state: Mutex<CompletionSlabState<T>>,
}

impl<T> Default for CompletionSlab<T> {
    fn default() -> Self {
        Self::with_retained_capacity(DEFAULT_RETAINED_CAPACITY)
    }
}

impl<T> CompletionSlab<T> {
    pub(super) fn with_retained_capacity(retained_capacity: usize) -> Self {
        Self {
            state: Mutex::new(CompletionSlabState::with_retained_capacity(
                retained_capacity,
            )),
        }
    }

    pub(super) fn register(&self) -> (CompletionSender<T>, CompletionReceiver<'_, T>) {
        let mut slot = lock(&self.state)
            .free
            .pop()
            .unwrap_or_else(|| Arc::new(CompletionSlot::default()));
        let generation = slot.activate().unwrap_or_else(|| {
            slot = Arc::new(CompletionSlot::default());
            slot.activate()
                .expect("a new completion slot has an available generation")
        });
        (
            CompletionSender {
                slot: slot.clone(),
                generation,
                finished: false,
            },
            CompletionReceiver {
                slab: self,
                slot: Some(slot),
                generation,
            },
        )
    }

    fn recycle(&self, slot: Arc<CompletionSlot<T>>, generation: u64) {
        if !slot.deactivate(generation) {
            return;
        }
        let mut state = lock(&self.state);
        if state.free.len() < state.retained_capacity {
            state.free.push(slot);
        }
    }
}

pub(super) struct CompletionSender<T> {
    slot: Arc<CompletionSlot<T>>,
    generation: u64,
    finished: bool,
}

impl<T> CompletionSender<T> {
    pub(super) fn send(mut self, value: T) -> Result<(), T> {
        let result = self.slot.complete(self.generation, value);
        self.finished = true;
        result
    }
}

impl<T> Drop for CompletionSender<T> {
    fn drop(&mut self) {
        if !self.finished {
            self.slot.disconnect(self.generation);
        }
    }
}

pub(super) struct CompletionReceiver<'a, T> {
    slab: &'a CompletionSlab<T>,
    slot: Option<Arc<CompletionSlot<T>>>,
    generation: u64,
}

impl<T> CompletionReceiver<'_, T> {
    fn release(&mut self) {
        if let Some(slot) = self.slot.take() {
            self.slab.recycle(slot, self.generation);
        }
    }
}

impl<T: Unpin> Future for CompletionReceiver<'_, T> {
    type Output = Result<T, CompletionDisconnected>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let result = self
            .slot
            .as_ref()
            .expect("completion receiver is not polled after completion")
            .poll(self.generation, cx);
        if result.is_ready() {
            self.release();
        }
        result
    }
}

impl<T> Drop for CompletionReceiver<'_, T> {
    fn drop(&mut self) {
        self.release();
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
