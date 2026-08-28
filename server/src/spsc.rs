//! Lock-free single-producer, single-consumer (SPSC) ring buffer.
//!
//! This is the only channel between the network thread and the storage thread.
//! With exactly one producer and one consumer, no locks or CAS loops are needed:
//! ownership of a slot transfers via a single Release store of `tail` (producer)
//! paired with an Acquire load (consumer), and vice versa for `head`. The head
//! and tail counters sit on separate cache lines, and each endpoint caches the
//! other's counter, so the common case touches no shared atomic at all.

use std::cell::UnsafeCell;
use std::mem::MaybeUninit;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[repr(align(64))]
/** Places a frequently updated atomic on its own cache line to reduce false sharing. */
struct CacheLine<T>(/** The value isolated from adjacent cache lines. */ T);

/** A fixed-capacity lock-free ring buffer shared by one producer and one consumer. */
struct Ring<T, const N: usize> {
    /** Storage slots; only slots in the logical [head, tail) range are initialized. */
    slots: [UnsafeCell<MaybeUninit<T>>; N],
    /** Index of the next slot the consumer will read. */
    head: CacheLine<AtomicUsize>,
    /** Index of the next slot the producer will write. */
    tail: CacheLine<AtomicUsize>,
}

// SAFETY: Each slot has exactly one producer and one consumer. Publishing tail
// with Release and observing it with Acquire transfers ownership.
unsafe impl<T: Send, const N: usize> Sync for Ring<T, N> {}

/** The ring's sole writer, which publishes values and caches consumer progress. */
pub(crate) struct Producer<T, const N: usize> {
    /** The ring shared by the producer and consumer. */
    ring: Arc<Ring<T, N>>,
    /** Last observed consumer head, cached to reduce atomic loads. */
    head_cache: usize,
}

/** The ring's sole reader, which retrieves values and caches producer progress. */
pub(crate) struct Consumer<T, const N: usize> {
    /** The ring shared by the producer and consumer. */
    ring: Arc<Ring<T, N>>,
    /** Last observed producer tail, cached to reduce atomic loads. */
    tail_cache: usize,
}

/** Creates an SPSC ring with at least two slots and returns its unique endpoints. */
pub(crate) fn channel<T, const N: usize>() -> (Producer<T, N>, Consumer<T, N>) {
    assert!(N >= 2, "an SPSC ring needs at least two slots");
    let ring = Arc::new(Ring {
        slots: std::array::from_fn(|_| UnsafeCell::new(MaybeUninit::uninit())),
        head: CacheLine(AtomicUsize::new(0)),
        tail: CacheLine(AtomicUsize::new(0)),
    });
    (
        Producer {
            ring: Arc::clone(&ring),
            head_cache: 0,
        },
        Consumer {
            ring,
            tail_cache: 0,
        },
    )
}

impl<T, const N: usize> Producer<T, N> {
    /** Checks for a free slot, refreshing the cached head only when necessary. */
    pub(crate) fn has_capacity(&mut self) -> bool {
        // `tail` is owned by this producer, so a Relaxed load suffices. Only when
        // the cached head says we look full do we pay for an Acquire load to see
        // if the consumer has since advanced — avoiding a shared read per call.
        let tail = self.ring.tail.0.load(Ordering::Relaxed);
        let next = (tail + 1) % N;
        if next != self.head_cache {
            return true;
        }
        self.head_cache = self.ring.head.0.load(Ordering::Acquire);
        next != self.head_cache
    }

    /** Publishes a value to the next slot, or returns it when the ring is full. */
    pub(crate) fn push(&mut self, value: T) -> Result<(), T> {
        let tail = self.ring.tail.0.load(Ordering::Relaxed);
        let next = (tail + 1) % N;
        // Re-check the real head only when the cache says full, so a full ring is
        // the only case that reads the shared atomic.
        if next == self.head_cache {
            self.head_cache = self.ring.head.0.load(Ordering::Acquire);
            if next == self.head_cache {
                return Err(value);
            }
        }

        // SAFETY: Only this producer writes the slot at `tail`; the consumer cannot
        // observe it until the Release store below.
        unsafe { (*self.ring.slots[tail].get()).write(value) };
        self.ring.tail.0.store(next, Ordering::Release);
        Ok(())
    }
}

impl<T, const N: usize> Consumer<T, N> {
    /** Retrieves the next value and advances head, or returns `None` when empty. */
    pub(crate) fn pop(&mut self) -> Option<T> {
        // Symmetric to the producer: `head` is ours (Relaxed), and we only pay for
        // an Acquire load of `tail` — which synchronizes with the producer's
        // Release store and makes the written value visible — when the ring looks
        // empty against the cached tail.
        let head = self.ring.head.0.load(Ordering::Relaxed);
        if head == self.tail_cache {
            self.tail_cache = self.ring.tail.0.load(Ordering::Acquire);
            if head == self.tail_cache {
                return None;
            }
        }

        // SAFETY: The Acquire load observed a slot initialized by the producer, and
        // this consumer is the only owner that reads and advances `head`.
        let value = unsafe { (*self.ring.slots[head].get()).assume_init_read() };
        self.ring.head.0.store((head + 1) % N, Ordering::Release);
        Some(value)
    }
}

impl<T, const N: usize> Drop for Ring<T, N> {
    /** Drops every initialized value remaining when the final shared owner is released. */
    fn drop(&mut self) {
        let mut head = self.head.0.load(Ordering::Relaxed);
        let tail = self.tail.0.load(Ordering::Relaxed);
        while head != tail {
            // SAFETY: Once the final Arc is released, neither endpoint exists and
            // every slot in [head, tail) is initialized.
            unsafe { (*self.slots[head].get()).assume_init_drop() };
            head = (head + 1) % N;
        }
    }
}
