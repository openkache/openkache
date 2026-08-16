//! Generic bounded keyed scheduler.
//!
//! This module deliberately knows nothing about operations, protocol values, or
//! storage.  An API supplies a command implementing [`ScheduledTask`] and owns
//! all preparation, reduction, and completion semantics.

use std::collections::hash_map::Entry;
use std::collections::{HashMap, VecDeque};
use std::hash::Hash;

/// Metadata needed by the scheduler to preserve lane ordering.
pub(super) trait ScheduledTask {
    type CollapseGroup: Copy + Eq;

    fn collapse_group(&self) -> Self::CollapseGroup;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SchedulerError {
    Full,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct SlotId {
    index: u32,
    generation: u32,
}

struct WaitingSlot<T> {
    generation: u32,
    value: Option<T>,
    /// Next FIFO command while occupied, or next free slot while vacant.
    next: Option<SlotId>,
}

/// Bounded intrusive FIFO storage.  Requests are moved exactly once into and
/// out of the slab; no per-request linked-list allocation is required.
pub(super) struct WaitingSlab<T> {
    slots: Vec<WaitingSlot<T>>,
    free: Option<SlotId>,
    capacity: usize,
}

impl<T> WaitingSlab<T> {
    pub(super) fn with_capacity(capacity: usize) -> Self {
        Self {
            slots: Vec::with_capacity(capacity),
            free: None,
            capacity,
        }
    }

    pub(super) fn has_capacity(&self) -> bool {
        self.slots.len() < self.capacity || self.free.is_some()
    }

    pub(super) fn insert(&mut self, value: T) -> Option<SlotId> {
        let id = if let Some(id) = self.free {
            self.free = self.slot_mut(id).next.take();
            id
        } else {
            if self.slots.len() == self.capacity {
                return None;
            }
            let index = u32::try_from(self.slots.len()).ok()?;
            self.slots.push(WaitingSlot {
                generation: 0,
                value: None,
                next: None,
            });
            SlotId {
                index,
                generation: 0,
            }
        };
        let slot = self.slot_mut(id);
        debug_assert!(slot.value.is_none());
        slot.value = Some(value);
        slot.next = None;
        Some(id)
    }

    pub(super) fn link(&mut self, tail: SlotId, next: SlotId) {
        let slot = self.slot_mut(tail);
        debug_assert!(slot.next.is_none());
        slot.next = Some(next);
    }

    pub(super) fn take(&mut self, id: SlotId) -> (T, Option<SlotId>) {
        let free = self.free;
        let slot = self.slot_mut(id);
        let value = slot.value.take().expect("waiting slot contains a command");
        let next = slot.next.take();
        slot.generation = slot.generation.wrapping_add(1);
        slot.next = free;
        self.free = Some(SlotId {
            index: id.index,
            generation: slot.generation,
        });
        (value, next)
    }

    pub(super) fn get(&self, id: SlotId) -> &T {
        let slot = self
            .slots
            .get(id.index as usize)
            .expect("waiting SlotId index is valid");
        assert_eq!(
            slot.generation, id.generation,
            "waiting SlotId generation is current"
        );
        slot.value
            .as_ref()
            .expect("waiting slot contains a command")
    }

    fn next(&self, id: SlotId) -> Option<SlotId> {
        let slot = self
            .slots
            .get(id.index as usize)
            .expect("waiting SlotId index is valid");
        assert_eq!(
            slot.generation, id.generation,
            "waiting SlotId generation is current"
        );
        slot.next
    }

    fn slot_mut(&mut self, id: SlotId) -> &mut WaitingSlot<T> {
        let slot = self
            .slots
            .get_mut(id.index as usize)
            .expect("waiting SlotId index is valid");
        assert_eq!(
            slot.generation, id.generation,
            "waiting SlotId generation is current"
        );
        slot
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LaneState<G> {
    Ready,
    Running { collapse_group: G },
}

struct KeyLane<G> {
    state: LaneState<G>,
    waiting_head: Option<SlotId>,
    waiting_tail: Option<SlotId>,
}

/// Fair, bounded scheduler for one worker's keyed requests.
pub(super) struct KeyScheduler<K, T: ScheduledTask> {
    lanes: HashMap<K, KeyLane<T::CollapseGroup>>,
    ready: VecDeque<K>,
    waiting: WaitingSlab<T>,
}

impl<K, T> KeyScheduler<K, T>
where
    K: Eq + Hash + Clone,
    T: ScheduledTask,
{
    pub(super) fn with_waiting_capacity(capacity: usize) -> Self {
        Self {
            lanes: HashMap::with_capacity(capacity.saturating_mul(2)),
            ready: VecDeque::with_capacity(capacity),
            waiting: WaitingSlab::with_capacity(capacity),
        }
    }

    pub(super) fn has_waiting_capacity(&self) -> bool {
        self.waiting.has_capacity()
    }

    pub(super) fn is_idle(&self) -> bool {
        self.lanes.is_empty()
    }

    pub(super) fn has_waiting(&self, storage_key: &K) -> bool {
        self.lanes
            .get(storage_key)
            .is_some_and(|lane| lane.waiting_head.is_some())
    }

    /// Removes the next fair command and records its reducer family.
    pub(super) fn take_ready(&mut self) -> Option<(K, T)> {
        let key = self.ready.pop_front()?;
        let head = self
            .lanes
            .get(&key)
            .expect("ready key has a lane")
            .waiting_head
            .expect("ready lane has a waiting command");
        let collapse_group = self.waiting.get(head).collapse_group();
        let (command, next) = self.waiting.take(head);
        let lane = self.lanes.get_mut(&key).expect("ready key has a lane");
        debug_assert!(matches!(lane.state, LaneState::Ready));
        lane.waiting_head = next;
        if next.is_none() {
            lane.waiting_tail = None;
        }
        lane.state = LaneState::Running { collapse_group };
        Some((key, command))
    }

    pub(super) fn enqueue(
        &mut self,
        storage_key: K,
        command: T,
    ) -> std::result::Result<(), SchedulerError> {
        let slot = self.waiting.insert(command).ok_or(SchedulerError::Full)?;
        match self.lanes.entry(storage_key) {
            Entry::Occupied(mut entry) => {
                let lane = entry.get_mut();
                if let Some(tail) = lane.waiting_tail {
                    self.waiting.link(tail, slot);
                } else {
                    debug_assert!(lane.waiting_head.is_none());
                    lane.waiting_head = Some(slot);
                }
                lane.waiting_tail = Some(slot);
            }
            Entry::Vacant(entry) => {
                let ready_key = entry.key().clone();
                entry.insert(KeyLane {
                    state: LaneState::Ready,
                    waiting_head: Some(slot),
                    waiting_tail: Some(slot),
                });
                self.ready.push_back(ready_key);
            }
        }
        Ok(())
    }

    /// Moves the contiguous prefix compatible with the running reducer.
    pub(super) fn drain_collapsible(
        &mut self,
        storage_key: K,
        can_collapse: impl FnMut(&T) -> bool,
    ) -> CollapsibleDrain<'_, K, T> {
        self.drain_collapsible_up_to(storage_key, usize::MAX, can_collapse)
    }

    /// Moves at most `limit` commands from the compatible reducer prefix.
    pub(super) fn drain_collapsible_up_to(
        &mut self,
        storage_key: K,
        limit: usize,
        mut can_collapse: impl FnMut(&T) -> bool,
    ) -> CollapsibleDrain<'_, K, T> {
        if limit == 0 {
            return CollapsibleDrain {
                scheduler: self,
                storage_key,
                remaining: 0,
            };
        }
        let Some((collapse_group, mut head)) =
            self.lanes
                .get(&storage_key)
                .and_then(|lane| match (lane.state, lane.waiting_head) {
                    (LaneState::Running { collapse_group }, Some(head)) => {
                        Some((collapse_group, head))
                    }
                    _ => None,
                })
        else {
            return CollapsibleDrain {
                scheduler: self,
                storage_key,
                remaining: 0,
            };
        };

        let mut remaining = 0;
        loop {
            let command = self.waiting.get(head);
            if command.collapse_group() != collapse_group || !can_collapse(command) {
                break;
            }
            remaining += 1;
            if remaining == limit {
                break;
            }
            let Some(next) = self.waiting.next(head) else {
                break;
            };
            head = next;
        }

        CollapsibleDrain {
            scheduler: self,
            storage_key,
            remaining,
        }
    }

    fn take_collapsible_head(&mut self, storage_key: &K) -> T {
        let (collapse_group, head) =
            self.lanes
                .get(storage_key)
                .and_then(|lane| match (lane.state, lane.waiting_head) {
                    (LaneState::Running { collapse_group }, Some(head)) => {
                        Some((collapse_group, head))
                    }
                    _ => None,
                })
                .expect("collapsible drain retains a running command");
        debug_assert!(
            self.waiting.get(head).collapse_group() == collapse_group,
            "collapsible drain remains in its reducer family"
        );
        let (command, next) = self.waiting.take(head);
        let lane = self
            .lanes
            .get_mut(storage_key)
            .expect("completed key has a lane");
        lane.waiting_head = next;
        if next.is_none() {
            lane.waiting_tail = None;
        }
        command
    }

    pub(super) fn finish_running_lane(&mut self, storage_key: K) {
        let ready_again = {
            let lane = self
                .lanes
                .get_mut(&storage_key)
                .expect("completed key has a lane");
            debug_assert!(matches!(lane.state, LaneState::Running { .. }));
            if lane.waiting_head.is_some() {
                lane.state = LaneState::Ready;
                true
            } else {
                false
            }
        };
        if ready_again {
            self.ready.push_back(storage_key);
        } else {
            self.lanes.remove(&storage_key);
        }
    }
}

/// Exact-size move iterator over one compatible scheduler prefix.
///
/// Each command remains scheduler-owned until it is consumed, avoiding an
/// intermediate batch. Dropping the iterator leaves its unconsumed suffix
/// queued.
pub(super) struct CollapsibleDrain<'a, K, T>
where
    K: Eq + Hash + Clone,
    T: ScheduledTask,
{
    scheduler: &'a mut KeyScheduler<K, T>,
    storage_key: K,
    remaining: usize,
}

impl<K, T> Iterator for CollapsibleDrain<'_, K, T>
where
    K: Eq + Hash + Clone,
    T: ScheduledTask,
{
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        self.remaining -= 1;
        Some(self.scheduler.take_collapsible_head(&self.storage_key))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<K, T> ExactSizeIterator for CollapsibleDrain<'_, K, T>
where
    K: Eq + Hash + Clone,
    T: ScheduledTask,
{
}

impl<K, T> std::iter::FusedIterator for CollapsibleDrain<'_, K, T>
where
    K: Eq + Hash + Clone,
    T: ScheduledTask,
{
}
