//! Reusable cross-thread completion slots for storage worker requests.

use std::any::Any;
use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Context, Poll, Waker};
#[cfg(feature = "network-runtime-kimojio")]
use std::time::Duration;

const DEFAULT_RETAINED_CAPACITY: usize = 256;
const LOCAL_RETAINED_CAPACITY: usize = 8;

struct LocalCompletionPool {
    slab: *const (),
    slots: Vec<Arc<dyn Any + Send + Sync>>,
}

thread_local! {
    static LOCAL_COMPLETIONS: RefCell<Vec<LocalCompletionPool>> = const { RefCell::new(Vec::new()) };
}

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

    #[cfg(feature = "network-runtime-kimojio")]
    fn try_take(&self, generation: u64) -> Result<Option<T>, CompletionDisconnected> {
        let mut state = lock(&self.state);
        if !state.active || state.generation != generation || state.disconnected {
            return Err(CompletionDisconnected);
        }
        Ok(state.value.take())
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

pub(super) struct CompletionSlab<T: Send + Sync + 'static> {
    state: Mutex<CompletionSlabState<T>>,
}

impl<T: Send + Sync + 'static> Default for CompletionSlab<T> {
    fn default() -> Self {
        Self::with_retained_capacity(DEFAULT_RETAINED_CAPACITY)
    }
}

impl<T: Send + Sync + 'static> CompletionSlab<T> {
    pub(super) fn with_retained_capacity(retained_capacity: usize) -> Self {
        Self {
            state: Mutex::new(CompletionSlabState::with_retained_capacity(
                retained_capacity,
            )),
        }
    }

    pub(super) fn register(&self) -> (CompletionSender<T>, CompletionReceiver<'_, T>) {
        let slab = self as *const Self as *const ();
        let mut slot = take_local_slot::<T>(slab)
            .or_else(|| lock(&self.state).free.pop())
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
        if retain_local_slot(self as *const Self as *const (), slot.clone()) {
            return;
        }
        let mut state = lock(&self.state);
        if state.free.len() < state.retained_capacity {
            state.free.push(slot);
        }
    }
}

pub(super) struct CompletionSender<T: Send + Sync + 'static> {
    slot: Arc<CompletionSlot<T>>,
    generation: u64,
    finished: bool,
}

impl<T: Send + Sync + 'static> CompletionSender<T> {
    pub(super) fn send(mut self, value: T) -> Result<(), T> {
        let result = self.slot.complete(self.generation, value);
        self.finished = true;
        result
    }
}

impl<T: Send + Sync + 'static> Drop for CompletionSender<T> {
    fn drop(&mut self) {
        if !self.finished {
            self.slot.disconnect(self.generation);
        }
    }
}

pub(super) struct CompletionReceiver<'a, T: Send + Sync + 'static> {
    slab: &'a CompletionSlab<T>,
    slot: Option<Arc<CompletionSlot<T>>>,
    generation: u64,
}

impl<T: Send + Sync + 'static> CompletionReceiver<'_, T> {
    fn release(&mut self) {
        if let Some(slot) = self.slot.take() {
            self.slab.recycle(slot, self.generation);
        }
    }

    #[cfg(feature = "network-runtime-kimojio")]
    fn try_recv(&mut self) -> Result<Option<T>, CompletionDisconnected> {
        let result = self
            .slot
            .as_ref()
            .expect("completion receiver is not polled after completion")
            .try_take(self.generation);
        if result.is_err() || result.as_ref().is_ok_and(Option::is_some) {
            self.release();
        }
        result
    }

    pub(super) async fn recv_async_network(self) -> Result<T, CompletionDisconnected>
    where
        T: Unpin,
    {
        #[cfg(not(feature = "network-runtime-kimojio"))]
        return self.await;

        #[cfg(feature = "network-runtime-kimojio")]
        {
            let mut receiver = self;
            loop {
                match receiver.try_recv()? {
                    Some(value) => return Ok(value),
                    None => crate::network_runtime::sleep(Duration::from_micros(10)).await,
                }
            }
        }
    }
}

impl<T: Unpin + Send + Sync + 'static> Future for CompletionReceiver<'_, T> {
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

impl<T: Send + Sync + 'static> Drop for CompletionReceiver<'_, T> {
    fn drop(&mut self) {
        self.release();
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn take_local_slot<T: Send + Sync + 'static>(
    slab: *const (),
) -> Option<Arc<CompletionSlot<T>>> {
    LOCAL_COMPLETIONS.with(|pools| {
        let mut pools = pools.borrow_mut();
        let pool = pools.iter_mut().find(|pool| pool.slab == slab)?;
        let slot = pool.slots.pop()?;
        slot.downcast::<CompletionSlot<T>>().ok()
    })
}

fn retain_local_slot<T: Send + Sync + 'static>(
    slab: *const (),
    slot: Arc<CompletionSlot<T>>,
) -> bool {
    LOCAL_COMPLETIONS.with(|pools| {
        let mut pools = pools.borrow_mut();
        let pool = pools.iter_mut().find(|pool| pool.slab == slab);
        let pool = match pool {
            Some(pool) => pool,
            None => {
                pools.push(LocalCompletionPool {
                    slab,
                    slots: Vec::with_capacity(LOCAL_RETAINED_CAPACITY),
                });
                pools.last_mut().expect("new local completion pool exists")
            }
        };
        if pool.slots.len() >= LOCAL_RETAINED_CAPACITY {
            return false;
        }
        let slot: Arc<dyn Any + Send + Sync> = slot;
        pool.slots.push(slot);
        true
    })
}
