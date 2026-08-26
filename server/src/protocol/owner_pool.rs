//! Bounded, allocation-free ownership for stable byte values.

use std::cell::UnsafeCell;
use std::mem::MaybeUninit;
use std::num::NonZeroU64;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crossbeam_queue::ArrayQueue;

use crate::protocol::segments::StableByteOwner;

const ACTIVE: u64 = 1;
const RELEASING: u64 = 2;
const STATE_GENERATION_SHIFT: u64 = 2;
const TOKEN_INDEX_BITS: u32 = 16;
const TOKEN_INDEX_MASK: u64 = u16::MAX as u64;
const MAX_GENERATION: u64 = (1 << (u64::BITS - TOKEN_INDEX_BITS)) - 1;

/// Opaque generation-checked handle for one preallocated owner slot.
#[derive(Debug, Eq, Hash, PartialEq)]
pub(crate) struct StableOwnerToken(NonZeroU64);

impl StableOwnerToken {
    fn new(index: usize, generation: u64) -> Self {
        let index = u16::try_from(index).expect("stable owner pool has at most u16 slots");
        assert!(generation <= MAX_GENERATION);
        Self(
            NonZeroU64::new((generation << TOKEN_INDEX_BITS) | u64::from(index))
                .expect("stable owner generation is nonzero"),
        )
    }

    fn index(&self) -> usize {
        (self.0.get() & TOKEN_INDEX_MASK) as usize
    }

    fn generation(&self) -> u64 {
        self.0.get() >> TOKEN_INDEX_BITS
    }
}

/// Operation-neutral type-erased access to a bounded owner pool.
pub(crate) trait StableByteOwnerPool: Send + Sync + 'static {
    /// Returns the bytes for a live token.
    ///
    /// A token is live until the [`StableBytes`](crate::protocol::StableBytes) carrying it
    /// is dropped. Callers must not retain this slice after that owner is
    /// dropped.
    ///
    /// # Safety
    ///
    /// The caller must keep the token live and must not call [`release`](Self::release)
    /// until the returned slice is no longer used.
    unsafe fn as_bytes(&self, token: &StableOwnerToken) -> &[u8];

    /// Releases a live token exactly once.
    ///
    /// # Safety
    ///
    /// The token must be uniquely owned by the caller and no slice returned by
    /// [`as_bytes`](Self::as_bytes) may still be in use.
    unsafe fn release(&self, token: StableOwnerToken);
}

/// A uniquely owned, pool-bound slot lease.
pub struct StableOwnerLease {
    pool: Arc<dyn StableByteOwnerPool>,
    token: Option<StableOwnerToken>,
}

impl std::fmt::Debug for StableOwnerLease {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StableOwnerLease")
            .field("token", &self.token)
            .finish_non_exhaustive()
    }
}

impl StableOwnerLease {
    fn new(pool: Arc<dyn StableByteOwnerPool>, token: StableOwnerToken) -> Self {
        Self {
            pool,
            token: Some(token),
        }
    }

    pub(crate) fn as_bytes(&self) -> &[u8] {
        // SAFETY: the lease owns the live token for its entire borrow.
        unsafe {
            self.pool.as_bytes(
                self.token
                    .as_ref()
                    .expect("stable owner lease retains a live token"),
            )
        }
    }
}

impl Drop for StableOwnerLease {
    fn drop(&mut self) {
        // SAFETY: dropping the lease consumes its unique token after all
        // borrows through the lease have ended.
        if let Some(token) = self.token.take() {
            unsafe { self.pool.release(token) };
        }
    }
}

struct Slot<T> {
    state: AtomicU64,
    value: UnsafeCell<MaybeUninit<T>>,
}

impl<T> Slot<T> {
    fn new() -> Self {
        Self {
            state: AtomicU64::new(0),
            value: UnsafeCell::new(MaybeUninit::uninit()),
        }
    }
}

unsafe impl<T: Send> Send for Slot<T> {}
unsafe impl<T: Send> Sync for Slot<T> {}

/// A fixed-capacity owner pool.
///
/// Slot storage is allocated once at construction. Insertion, lookup, and
/// release perform no heap allocation; generation-bearing tokens reject stale
/// releases after a slot has been recycled.
pub struct StableOwnerPool<T> {
    slots: Box<[Slot<T>]>,
    free_slots: Option<ArrayQueue<u16>>,
}

impl<T> StableOwnerPool<T> {
    /// Returns the fixed slot and free-queue storage for `capacity` owners.
    pub fn allocation_bytes(capacity: usize) -> Option<usize> {
        let queue_slot_bytes = std::mem::size_of::<usize>()
            .checked_add(std::mem::size_of::<u16>())?
            .next_multiple_of(std::mem::align_of::<usize>());
        std::mem::size_of::<Self>()
            .checked_add(std::mem::size_of::<Slot<T>>().checked_mul(capacity)?)?
            .checked_add(queue_slot_bytes.checked_mul(capacity)?)
    }

    /// Creates a pool with `capacity` preallocated slots.
    pub fn new(capacity: usize) -> Arc<Self> {
        assert!(
            capacity <= TOKEN_INDEX_MASK as usize,
            "stable owner pool capacity is too large"
        );
        let slots = (0..capacity)
            .map(|_| Slot::new())
            .collect::<Vec<_>>()
            .into_boxed_slice();
        let free_slots = (capacity != 0).then(|| {
            let free_slots = ArrayQueue::new(capacity);
            for index in 0..capacity {
                free_slots
                    .push(index as u16)
                    .expect("stable owner free queue has fixed capacity");
            }
            free_slots
        });
        Arc::new(Self { slots, free_slots })
    }

    /// Inserts an owner without allocating.
    pub fn try_insert(self: &Arc<Self>, owner: T) -> Result<StableOwnerLease, T>
    where
        T: StableByteOwner,
    {
        let Some(index) = self.pop_free() else {
            return Err(owner);
        };
        let slot = &self.slots[index];
        let generation = (slot.state.load(Ordering::Relaxed) >> STATE_GENERATION_SHIFT)
            .checked_add(1)
            .filter(|generation| *generation <= MAX_GENERATION)
            .expect("stable owner slot generation exhausted");
        let active = (generation << STATE_GENERATION_SHIFT) | ACTIVE;

        // SAFETY: the free-list pop exclusively claims this slot.
        unsafe {
            (*slot.value.get()).write(owner);
        }
        slot.state.store(active, Ordering::Release);
        Ok(StableOwnerLease::new(
            Arc::clone(self) as Arc<dyn StableByteOwnerPool>,
            StableOwnerToken::new(index, generation),
        ))
    }

    fn pop_free(&self) -> Option<usize> {
        self.free_slots.as_ref()?.pop().map(usize::from)
    }

    fn push_free(&self, index: usize) {
        self.free_slots
            .as_ref()
            .expect("stable owner free queue exists for a live owner")
            .push(u16::try_from(index).expect("stable owner pool index fits in u16"))
            .expect("stable owner free queue exceeded its fixed capacity");
    }

    fn slot(&self, token: &StableOwnerToken) -> &Slot<T> {
        self.slots
            .get(token.index())
            .expect("stable owner token index is valid")
    }
}

impl<T> StableByteOwnerPool for StableOwnerPool<T>
where
    T: StableByteOwner,
{
    unsafe fn as_bytes(&self, token: &StableOwnerToken) -> &[u8] {
        let slot = self.slot(token);
        let expected = (token.generation() << STATE_GENERATION_SHIFT) | ACTIVE;
        assert_eq!(
            slot.state.load(Ordering::Acquire),
            expected,
            "stable owner token is not live"
        );

        // SAFETY: the token validates that the claimed slot contains a live T
        // and the caller upholds the trait's token lifetime contract.
        unsafe { (*slot.value.get()).assume_init_ref().as_bytes() }
    }

    unsafe fn release(&self, token: StableOwnerToken) {
        let slot = self.slot(&token);
        let expected = (token.generation() << STATE_GENERATION_SHIFT) | ACTIVE;
        let releasing = (token.generation() << STATE_GENERATION_SHIFT) | RELEASING;
        assert!(
            slot.state
                .compare_exchange(expected, releasing, Ordering::AcqRel, Ordering::Acquire)
                .is_ok(),
            "stable owner token is not live"
        );

        // SAFETY: the token validates exclusive ownership of the live value.
        unsafe { (*slot.value.get()).assume_init_drop() };
        slot.state.store(
            token.generation() << STATE_GENERATION_SHIFT,
            Ordering::Release,
        );
        self.push_free(token.index());
    }
}

impl<T> Drop for StableOwnerPool<T> {
    fn drop(&mut self) {
        for slot in self.slots.iter_mut() {
            if slot.state.load(Ordering::Acquire) & ACTIVE == 0 {
                continue;
            }
            // SAFETY: an active slot contains an initialized value and the pool
            // owns the final reference when its destructor runs.
            unsafe { (*slot.value.get()).assume_init_drop() };
        }
    }
}
