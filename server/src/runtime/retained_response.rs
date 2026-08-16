use std::iter::FusedIterator;
use std::mem::{replace, size_of};
use std::sync::atomic::{AtomicU64, Ordering};

const NO_SLOT: u32 = u32::MAX;

static NEXT_ARENA_ID: AtomicU64 = AtomicU64::new(1);

struct Slot<R> {
    next: u32,
    value: Option<R>,
}

/// Fixed-capacity storage for values retained across worker-loop iterations.
pub(super) struct RetainedResponseArena<R> {
    slots: Box<[Slot<R>]>,
    free_head: u32,
    available: usize,
    id: u64,
}

impl<R> RetainedResponseArena<R> {
    pub(super) fn new(capacity: usize) -> Self {
        assert!(
            capacity <= u32::MAX as usize,
            "retained-response capacity exceeds the slot index range"
        );

        let id = NEXT_ARENA_ID
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .expect("retained-response arena identity space exhausted");
        let mut slots = Vec::with_capacity(capacity);
        for index in 0..capacity {
            let next = if index + 1 == capacity {
                NO_SLOT
            } else {
                u32::try_from(index + 1).expect("capacity was checked")
            };
            slots.push(Slot { next, value: None });
        }

        Self {
            slots: slots.into_boxed_slice(),
            free_head: if capacity == 0 { NO_SLOT } else { 0 },
            available: capacity,
            id,
        }
    }

    pub(super) fn capacity(&self) -> usize {
        self.slots.len()
    }

    pub(super) fn available(&self) -> usize {
        self.available
    }

    pub(super) const fn allocation_bytes(capacity: usize) -> Option<usize> {
        capacity.checked_mul(size_of::<Slot<R>>())
    }

    pub(super) fn reserve(&mut self) -> Option<ResponseReservation> {
        self.take_free_slot().map(|slot| ResponseReservation {
            arena_id: self.id,
            slot,
        })
    }

    pub(super) fn release(&mut self, reservation: ResponseReservation) {
        self.assert_owner(reservation.arena_id);
        let slot = self.slot_mut(reservation.slot);
        assert!(
            slot.value.is_none(),
            "retained-response reservation is not empty"
        );
        self.recycle_slot(reservation.slot);
    }

    pub(super) fn complete(&mut self, reservation: ResponseReservation, value: R) -> ResponseBatch {
        self.assert_owner(reservation.arena_id);
        let slot = self.slot_mut(reservation.slot);
        assert!(
            slot.value.is_none(),
            "retained-response reservation is not empty"
        );
        slot.value = Some(value);
        slot.next = NO_SLOT;
        ResponseBatch {
            arena_id: self.id,
            head: reservation.slot,
            tail: reservation.slot,
            len: 1,
        }
    }

    pub(super) fn batch(&mut self) -> ResponseBatchBuilder<'_, R> {
        let arena_id = self.id;
        ResponseBatchBuilder {
            arena: self,
            batch: ResponseBatch::empty(arena_id),
        }
    }

    pub(super) fn get_mut(&mut self, batch: &ResponseBatch, index: usize) -> Option<&mut R> {
        self.assert_owner(batch.arena_id);
        let slot = self.batch_slot(batch, index)?;
        self.slot_mut(slot).value.as_mut()
    }

    pub(super) fn drain(&mut self, batch: ResponseBatch) -> ResponseDrain<'_, R> {
        self.assert_owner(batch.arena_id);
        ResponseDrain {
            arena: self,
            head: batch.head,
            remaining: batch.len,
        }
    }

    fn take_free_slot(&mut self) -> Option<u32> {
        let slot = self.free_head;
        if slot == NO_SLOT {
            return None;
        }

        let next = self.slot(slot).next;
        self.free_head = next;
        self.available -= 1;
        self.slot_mut(slot).next = NO_SLOT;
        Some(slot)
    }

    fn append(&mut self, batch: &mut ResponseBatch, value: R) -> Result<usize, R> {
        let Some(slot) = self.take_free_slot() else {
            return Err(value);
        };

        self.slot_mut(slot).value = Some(value);
        if batch.tail == NO_SLOT {
            batch.head = slot;
        } else {
            self.slot_mut(batch.tail).next = slot;
        }
        batch.tail = slot;
        let index = batch.len;
        batch.len += 1;
        Ok(index)
    }

    fn batch_slot(&self, batch: &ResponseBatch, index: usize) -> Option<u32> {
        if index >= batch.len {
            return None;
        }

        let mut slot = batch.head;
        for _ in 0..index {
            slot = self.slot(slot).next;
        }
        Some(slot)
    }

    fn reclaim(&mut self, mut head: u32, mut remaining: usize) {
        while remaining != 0 {
            assert_ne!(head, NO_SLOT, "retained-response batch chain is truncated");
            let next = self.slot(head).next;
            self.slot_mut(head).value.take();
            self.recycle_slot(head);
            head = next;
            remaining -= 1;
        }
        assert_eq!(
            head, NO_SLOT,
            "retained-response batch chain exceeds its length"
        );
    }

    fn recycle_slot(&mut self, slot: u32) {
        let free_head = self.free_head;
        let recycled = self.slot_mut(slot);
        debug_assert!(recycled.value.is_none());
        recycled.next = free_head;
        self.free_head = slot;
        self.available += 1;
        debug_assert!(self.available <= self.capacity());
    }

    fn assert_owner(&self, arena_id: u64) {
        assert_eq!(
            arena_id, self.id,
            "retained-response handle belongs to another arena"
        );
    }

    fn slot(&self, index: u32) -> &Slot<R> {
        &self.slots[index as usize]
    }

    fn slot_mut(&mut self, index: u32) -> &mut Slot<R> {
        &mut self.slots[index as usize]
    }
}

/// One empty slot reserved before starting work that may produce a value.
#[must_use = "the reservation must be released or completed"]
pub(super) struct ResponseReservation {
    arena_id: u64,
    slot: u32,
}

/// Move-only ownership of one FIFO-linked sequence in an arena.
#[must_use = "the batch must be drained"]
pub(super) struct ResponseBatch {
    arena_id: u64,
    head: u32,
    tail: u32,
    len: usize,
}

impl ResponseBatch {
    fn empty(arena_id: u64) -> Self {
        Self {
            arena_id,
            head: NO_SLOT,
            tail: NO_SLOT,
            len: 0,
        }
    }

    pub(super) fn len(&self) -> usize {
        self.len
    }

}

/// In-progress batch whose uncommitted slots are reclaimed on drop.
#[must_use = "the builder must be committed or dropped"]
pub(super) struct ResponseBatchBuilder<'a, R> {
    arena: &'a mut RetainedResponseArena<R>,
    batch: ResponseBatch,
}

impl<R> ResponseBatchBuilder<'_, R> {
    pub(super) fn push(&mut self, value: R) -> Result<usize, R> {
        self.arena.append(&mut self.batch, value)
    }

    pub(super) fn len(&self) -> usize {
        self.batch.len()
    }

    pub(super) fn commit(mut self) -> ResponseBatch {
        let arena_id = self.arena.id;
        replace(&mut self.batch, ResponseBatch::empty(arena_id))
    }
}

impl<R> Drop for ResponseBatchBuilder<'_, R> {
    fn drop(&mut self) {
        self.arena.reclaim(self.batch.head, self.batch.len);
    }
}

/// FIFO iterator that recycles each consumed slot and reclaims unread values.
#[must_use = "the drain must be consumed or dropped"]
pub(super) struct ResponseDrain<'a, R> {
    arena: &'a mut RetainedResponseArena<R>,
    head: u32,
    remaining: usize,
}

impl<R> Iterator for ResponseDrain<'_, R> {
    type Item = R;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }

        assert_ne!(
            self.head, NO_SLOT,
            "retained-response batch chain is truncated"
        );
        let slot = self.head;
        self.head = self.arena.slot(slot).next;
        self.remaining -= 1;
        let value = self
            .arena
            .slot_mut(slot)
            .value
            .take()
            .expect("retained-response batch slot is empty");
        self.arena.recycle_slot(slot);
        if self.remaining == 0 {
            assert_eq!(
                self.head, NO_SLOT,
                "retained-response batch chain exceeds its length"
            );
        }
        Some(value)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<R> ExactSizeIterator for ResponseDrain<'_, R> {
    fn len(&self) -> usize {
        self.remaining
    }
}

impl<R> FusedIterator for ResponseDrain<'_, R> {}

impl<R> Drop for ResponseDrain<'_, R> {
    fn drop(&mut self) {
        self.arena.reclaim(self.head, self.remaining);
        self.head = NO_SLOT;
        self.remaining = 0;
    }
}
