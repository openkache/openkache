//! Reusable cross-thread completion slots for storage worker requests.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Context, Poll, Waker};

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
    fn activate(&self) -> u64 {
        let mut state = lock(&self.state);
        debug_assert!(!state.active);
        state.generation = state.generation.wrapping_add(1);
        state.active = true;
        state.disconnected = false;
        state.value = None;
        state.waker = None;
        state.generation
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
    slots: Vec<Arc<CompletionSlot<T>>>,
    free: Vec<usize>,
}

impl<T> Default for CompletionSlabState<T> {
    fn default() -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
        }
    }
}

pub(super) struct CompletionSlab<T> {
    state: Mutex<CompletionSlabState<T>>,
}

impl<T> Default for CompletionSlab<T> {
    fn default() -> Self {
        Self {
            state: Mutex::new(CompletionSlabState::default()),
        }
    }
}

impl<T> CompletionSlab<T> {
    pub(super) fn register(&self) -> (CompletionSender<T>, CompletionReceiver<'_, T>) {
        let (index, slot) = {
            let mut state = lock(&self.state);
            if let Some(index) = state.free.pop() {
                (index, state.slots[index].clone())
            } else {
                let index = state.slots.len();
                let slot = Arc::new(CompletionSlot::default());
                state.slots.push(slot.clone());
                (index, slot)
            }
        };
        let generation = slot.activate();
        (
            CompletionSender {
                slot: slot.clone(),
                generation,
                finished: false,
            },
            CompletionReceiver {
                slab: self,
                slot,
                index,
                generation,
                released: false,
            },
        )
    }

    fn recycle(&self, index: usize, slot: &CompletionSlot<T>, generation: u64) {
        if slot.deactivate(generation) {
            lock(&self.state).free.push(index);
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
    slot: Arc<CompletionSlot<T>>,
    index: usize,
    generation: u64,
    released: bool,
}

impl<T> CompletionReceiver<'_, T> {
    fn release(&mut self) {
        if !self.released {
            self.slab.recycle(self.index, &self.slot, self.generation);
            self.released = true;
        }
    }
}

impl<T: Unpin> Future for CompletionReceiver<'_, T> {
    type Output = Result<T, CompletionDisconnected>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let result = self.slot.poll(self.generation, cx);
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
