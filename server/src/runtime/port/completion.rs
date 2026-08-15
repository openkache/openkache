//! Reusable cross-thread completion slots for storage worker requests.

use std::cell::UnsafeCell;
use std::future::Future;
use std::mem::MaybeUninit;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Context, Poll};
#[cfg(feature = "network-runtime-kimojio")]
use std::time::Duration;

use futures_util::task::AtomicWaker;

const DEFAULT_RETAINED_CAPACITY: usize = 256;
const STATE_BITS: u32 = 3;
const STATE_MASK: u64 = (1 << STATE_BITS) - 1;
const FREE: u64 = 0;
const WAITING: u64 = 1;
const WRITING: u64 = 2;
const READY: u64 = 3;
const DISCONNECTED: u64 = 4;
const WRITING_CANCELLED: u64 = 5;
const READING: u64 = 6;
const MAX_GENERATION: u64 = u64::MAX >> STATE_BITS;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct CompletionDisconnected;

#[derive(Clone, Copy)]
struct SlotId {
    index: u32,
    generation: u64,
}

struct AtomicSlot<T> {
    generation_and_state: AtomicU64,
    value: UnsafeCell<MaybeUninit<T>>,
    waker: AtomicWaker,
}

// SAFETY: the state machine grants exclusive value access to one completing sender or one
// receiver. `T` crosses threads only after a release publication and an acquire observation.
unsafe impl<T: Send> Sync for AtomicSlot<T> {}

impl<T> Default for AtomicSlot<T> {
    fn default() -> Self {
        Self {
            generation_and_state: AtomicU64::new(tag(0, FREE)),
            value: UnsafeCell::new(MaybeUninit::uninit()),
            waker: AtomicWaker::new(),
        }
    }
}

impl<T> AtomicSlot<T> {
    fn activate(&self) -> Option<u64> {
        let current = self.generation_and_state.load(Ordering::Acquire);
        debug_assert_eq!(state(current), FREE);
        let generation = generation(current).checked_add(1)?;
        if generation > MAX_GENERATION {
            return None;
        }
        let _ = self.waker.take();
        self.generation_and_state
            .compare_exchange(
                current,
                tag(generation, WAITING),
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .ok()
            .map(|_| generation)
    }

    fn complete(&self, generation: u64, value: T) -> CompleteResult<T> {
        if self
            .generation_and_state
            .compare_exchange(
                tag(generation, WAITING),
                tag(generation, WRITING),
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_err()
        {
            return CompleteResult::Rejected(value);
        }

        // SAFETY: the WAITING-to-WRITING transition grants this sender exclusive write access.
        unsafe { (*self.value.get()).write(value) };

        match self.generation_and_state.compare_exchange(
            tag(generation, WRITING),
            tag(generation, READY),
            Ordering::Release,
            Ordering::Acquire,
        ) {
            Ok(_) => {
                self.waker.wake();
                CompleteResult::Delivered
            }
            Err(current) if current == tag(generation, WRITING_CANCELLED) => {
                // SAFETY: this sender initialized the value and cancellation transferred the
                // responsibility for reading it back to the sender.
                let value = unsafe { (*self.value.get()).assume_init_read() };
                let _ = self.waker.take();
                self.generation_and_state
                    .store(tag(generation, FREE), Ordering::Release);
                CompleteResult::RejectedAndFreed(value)
            }
            Err(_) => unreachable!("completion slot has a valid writer transition"),
        }
    }

    fn disconnect(&self, generation: u64) {
        if self
            .generation_and_state
            .compare_exchange(
                tag(generation, WAITING),
                tag(generation, DISCONNECTED),
                Ordering::Release,
                Ordering::Acquire,
            )
            .is_ok()
        {
            self.waker.wake();
        }
    }

    fn poll(&self, expected_generation: u64, cx: &Context<'_>) -> Poll<SlotResult<T>> {
        loop {
            if let Some(result) = self.try_take(expected_generation) {
                return Poll::Ready(result);
            }
            self.waker.register(cx.waker());
            if let Some(result) = self.try_take(expected_generation) {
                return Poll::Ready(result);
            }
            let current = self.generation_and_state.load(Ordering::Acquire);
            if generation(current) == expected_generation
                && matches!(state(current), WAITING | WRITING)
            {
                return Poll::Pending;
            }
        }
    }

    fn try_take(&self, expected_generation: u64) -> Option<SlotResult<T>> {
        loop {
            let current = self.generation_and_state.load(Ordering::Acquire);
            if generation(current) != expected_generation {
                return Some(SlotResult::Disconnected { recycle: false });
            }
            match state(current) {
                WAITING | WRITING => return None,
                READY => {
                    if self
                        .generation_and_state
                        .compare_exchange(
                            current,
                            tag(expected_generation, READING),
                            Ordering::Acquire,
                            Ordering::Acquire,
                        )
                        .is_err()
                    {
                        continue;
                    }
                    // SAFETY: acquiring READY observes the sender's initialized value and the
                    // READY-to-READING transition grants this receiver exclusive read access.
                    let value = unsafe { (*self.value.get()).assume_init_read() };
                    let _ = self.waker.take();
                    self.generation_and_state
                        .store(tag(expected_generation, FREE), Ordering::Release);
                    return Some(SlotResult::Value(value));
                }
                DISCONNECTED => {
                    if self
                        .generation_and_state
                        .compare_exchange(
                            current,
                            tag(expected_generation, FREE),
                            Ordering::AcqRel,
                            Ordering::Acquire,
                        )
                        .is_err()
                    {
                        continue;
                    }
                    let _ = self.waker.take();
                    return Some(SlotResult::Disconnected { recycle: true });
                }
                FREE => return Some(SlotResult::Disconnected { recycle: false }),
                WRITING_CANCELLED | READING => {
                    return Some(SlotResult::Disconnected { recycle: false });
                }
                _ => unreachable!("completion slot state is encoded in three bits"),
            }
        }
    }

    fn deactivate(&self, expected_generation: u64) -> DeactivateResult {
        loop {
            let current = self.generation_and_state.load(Ordering::Acquire);
            if generation(current) != expected_generation {
                return DeactivateResult::Stale;
            }
            match state(current) {
                WAITING | DISCONNECTED => {
                    if self
                        .generation_and_state
                        .compare_exchange(
                            current,
                            tag(expected_generation, FREE),
                            Ordering::AcqRel,
                            Ordering::Acquire,
                        )
                        .is_err()
                    {
                        continue;
                    }
                    let _ = self.waker.take();
                    return DeactivateResult::Freed;
                }
                WRITING => {
                    if self
                        .generation_and_state
                        .compare_exchange(
                            current,
                            tag(expected_generation, WRITING_CANCELLED),
                            Ordering::AcqRel,
                            Ordering::Acquire,
                        )
                        .is_err()
                    {
                        continue;
                    }
                    let _ = self.waker.take();
                    return DeactivateResult::DeferredToWriter;
                }
                READY => {
                    if self
                        .generation_and_state
                        .compare_exchange(
                            current,
                            tag(expected_generation, READING),
                            Ordering::Acquire,
                            Ordering::Acquire,
                        )
                        .is_err()
                    {
                        continue;
                    }
                    // SAFETY: acquiring READY grants exclusive read access to the initialized
                    // value, which must be dropped because the receiver was cancelled.
                    unsafe { (*self.value.get()).assume_init_drop() };
                    let _ = self.waker.take();
                    self.generation_and_state
                        .store(tag(expected_generation, FREE), Ordering::Release);
                    return DeactivateResult::Freed;
                }
                FREE | WRITING_CANCELLED | READING => return DeactivateResult::Stale,
                _ => unreachable!("completion slot state is encoded in three bits"),
            }
        }
    }
}

impl<T> Drop for AtomicSlot<T> {
    fn drop(&mut self) {
        if state(*self.generation_and_state.get_mut()) == READY {
            // SAFETY: dropping the final slot owner means no sender or receiver can access the
            // initialized READY value again.
            unsafe { self.value.get_mut().assume_init_drop() };
        }
    }
}

enum CompleteResult<T> {
    Delivered,
    Rejected(T),
    RejectedAndFreed(T),
}

enum SlotResult<T> {
    Value(T),
    Disconnected { recycle: bool },
}

enum DeactivateResult {
    Freed,
    DeferredToWriter,
    Stale,
}

struct CompletionPool<T> {
    slots: Box<[AtomicSlot<T>]>,
    free: Mutex<Vec<u32>>,
}

impl<T> CompletionPool<T> {
    fn with_capacity(capacity: usize) -> Self {
        let slots = std::iter::repeat_with(AtomicSlot::default)
            .take(capacity)
            .collect::<Box<[_]>>();
        let free = (0..capacity)
            .rev()
            .map(|index| u32::try_from(index).expect("completion capacity fits in u32"))
            .collect();
        Self {
            slots,
            free: Mutex::new(free),
        }
    }

    fn activate(&self) -> Option<SlotId> {
        loop {
            let index = lock(&self.free).pop()?;
            if let Some(generation) = self.slots[index as usize].activate() {
                return Some(SlotId { index, generation });
            }
        }
    }

    fn slot(&self, id: SlotId) -> &AtomicSlot<T> {
        &self.slots[id.index as usize]
    }

    fn recycle(&self, index: u32) {
        lock(&self.free).push(index);
    }
}

pub(in crate::runtime) struct CompletionSlab<T> {
    pool: Arc<CompletionPool<T>>,
}

impl<T> Default for CompletionSlab<T> {
    fn default() -> Self {
        Self::with_retained_capacity(DEFAULT_RETAINED_CAPACITY)
    }
}

impl<T> CompletionSlab<T> {
    pub(in crate::runtime) fn with_retained_capacity(retained_capacity: usize) -> Self {
        assert!(
            u32::try_from(retained_capacity).is_ok(),
            "completion capacity fits in u32"
        );
        Self {
            pool: Arc::new(CompletionPool::with_capacity(retained_capacity)),
        }
    }

    pub(in crate::runtime) fn register(&self) -> (CompletionSender<T>, CompletionReceiver<'_, T>) {
        if let Some(id) = self.pool.activate() {
            return (
                CompletionSender {
                    storage: SenderStorage::Indexed {
                        pool: self.pool.clone(),
                        id,
                    },
                    finished: false,
                },
                CompletionReceiver {
                    slab: self,
                    storage: Some(ReceiverStorage::Indexed(id)),
                },
            );
        }

        let slot = Arc::new(AtomicSlot::default());
        let generation = slot
            .activate()
            .expect("a new completion slot has an available generation");
        (
            CompletionSender {
                storage: SenderStorage::Overflow {
                    slot: slot.clone(),
                    generation,
                },
                finished: false,
            },
            CompletionReceiver {
                slab: self,
                storage: Some(ReceiverStorage::Overflow { slot, generation }),
            },
        )
    }
}

enum SenderStorage<T> {
    Indexed {
        pool: Arc<CompletionPool<T>>,
        id: SlotId,
    },
    Overflow {
        slot: Arc<AtomicSlot<T>>,
        generation: u64,
    },
}

impl<T> SenderStorage<T> {
    fn complete(&self, value: T) -> Result<(), T> {
        let (slot, generation) = match self {
            Self::Indexed { pool, id } => (pool.slot(*id), id.generation),
            Self::Overflow { slot, generation } => (slot.as_ref(), *generation),
        };
        match slot.complete(generation, value) {
            CompleteResult::Delivered => Ok(()),
            CompleteResult::Rejected(value) => Err(value),
            CompleteResult::RejectedAndFreed(value) => {
                if let Self::Indexed { pool, id } = self {
                    pool.recycle(id.index);
                }
                Err(value)
            }
        }
    }

    fn disconnect(&self) {
        match self {
            Self::Indexed { pool, id } => pool.slot(*id).disconnect(id.generation),
            Self::Overflow { slot, generation } => slot.disconnect(*generation),
        }
    }
}

pub(in crate::runtime) struct CompletionSender<T> {
    storage: SenderStorage<T>,
    finished: bool,
}

impl<T> CompletionSender<T> {
    pub(in crate::runtime) fn send(mut self, value: T) -> Result<(), T> {
        let result = self.storage.complete(value);
        self.finished = true;
        result
    }
}

impl<T> Drop for CompletionSender<T> {
    fn drop(&mut self) {
        if !self.finished {
            self.storage.disconnect();
        }
    }
}

enum ReceiverStorage<T> {
    Indexed(SlotId),
    Overflow {
        slot: Arc<AtomicSlot<T>>,
        generation: u64,
    },
}

pub(in crate::runtime) struct CompletionReceiver<'a, T> {
    slab: &'a CompletionSlab<T>,
    storage: Option<ReceiverStorage<T>>,
}

impl<T> CompletionReceiver<'_, T> {
    fn slot(&self) -> (&AtomicSlot<T>, u64) {
        match self
            .storage
            .as_ref()
            .expect("completion receiver is not polled after completion")
        {
            ReceiverStorage::Indexed(id) => (self.slab.pool.slot(*id), id.generation),
            ReceiverStorage::Overflow { slot, generation } => (slot, *generation),
        }
    }

    fn finish(&mut self, recycle: bool) {
        let Some(storage) = self.storage.take() else {
            return;
        };
        if recycle && let ReceiverStorage::Indexed(id) = storage {
            self.slab.pool.recycle(id.index);
        }
    }

    fn release(&mut self) {
        let Some(storage) = self.storage.take() else {
            return;
        };
        let (result, index) = match &storage {
            ReceiverStorage::Indexed(id) => (
                self.slab.pool.slot(*id).deactivate(id.generation),
                Some(id.index),
            ),
            ReceiverStorage::Overflow { slot, generation } => (slot.deactivate(*generation), None),
        };
        if matches!(result, DeactivateResult::Freed)
            && let Some(index) = index
        {
            self.slab.pool.recycle(index);
        }
    }

    fn consume_result(
        &mut self,
        result: SlotResult<T>,
    ) -> Result<Option<T>, CompletionDisconnected> {
        match result {
            SlotResult::Value(value) => {
                self.finish(true);
                Ok(Some(value))
            }
            SlotResult::Disconnected { recycle } => {
                self.finish(recycle);
                Err(CompletionDisconnected)
            }
        }
    }

    #[cfg(feature = "network-runtime-kimojio")]
    fn try_recv(&mut self) -> Result<Option<T>, CompletionDisconnected> {
        let (slot, generation) = self.slot();
        let Some(result) = slot.try_take(generation) else {
            return Ok(None);
        };
        self.consume_result(result)
    }

    pub(in crate::runtime) async fn recv_async_network(self) -> Result<T, CompletionDisconnected>
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

impl<T: Unpin> Future for CompletionReceiver<'_, T> {
    type Output = Result<T, CompletionDisconnected>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let (slot, generation) = self.slot();
        let result = std::task::ready!(slot.poll(generation, cx));
        Poll::Ready(
            self.consume_result(result)
                .map(|value| value.expect("a completed future contains one completion value")),
        )
    }
}

impl<T> Drop for CompletionReceiver<'_, T> {
    fn drop(&mut self) {
        self.release();
    }
}

fn tag(generation: u64, state: u64) -> u64 {
    (generation << STATE_BITS) | state
}

fn generation(tag: u64) -> u64 {
    tag >> STATE_BITS
}

fn state(tag: u64) -> u64 {
    tag & STATE_MASK
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
