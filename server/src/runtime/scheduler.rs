//! Generic bounded keyed scheduler.
//!
//! This module deliberately knows nothing about operations, protocol values, or
//! storage.  An API supplies a command implementing [`ScheduledTask`] and owns
//! all preparation, reduction, and completion semantics.

use std::collections::{HashMap, VecDeque};
use std::hash::Hash;

/// Identity for a reducer family.  The token must be non-zero-sized because
/// pointers to distinct zero-sized statics are allowed to compare equal.
#[derive(Debug, Eq, PartialEq)]
pub(super) struct CollapseGroup(pub(super) u8);

/// Metadata needed by the scheduler to preserve lane ordering.
pub(super) trait ScheduledTask {
    fn collapse_group(&self) -> &'static CollapseGroup;
    fn is_exclusive(&self) -> bool;
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
    next: Option<SlotId>,
}

/// Bounded intrusive FIFO storage.  Requests are moved exactly once into and
/// out of the slab; no per-request linked-list allocation is required.
pub(super) struct WaitingSlab<T> {
    slots: Vec<WaitingSlot<T>>,
    free: Vec<u32>,
    capacity: usize,
}

impl<T> WaitingSlab<T> {
    pub(super) fn with_capacity(capacity: usize) -> Self {
        Self {
            slots: Vec::with_capacity(capacity),
            free: Vec::new(),
            capacity,
        }
    }

    pub(super) fn has_capacity(&self) -> bool {
        self.slots.len() < self.capacity || !self.free.is_empty()
    }

    pub(super) fn insert(&mut self, value: T) -> Option<SlotId> {
        let index = if let Some(index) = self.free.pop() {
            index
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
            index
        };
        let slot = &mut self.slots[index as usize];
        debug_assert!(slot.value.is_none());
        slot.value = Some(value);
        slot.next = None;
        Some(SlotId {
            index,
            generation: slot.generation,
        })
    }

    pub(super) fn link(&mut self, tail: SlotId, next: SlotId) {
        let slot = self.slot_mut(tail);
        debug_assert!(slot.next.is_none());
        slot.next = Some(next);
    }

    pub(super) fn take(&mut self, id: SlotId) -> (T, Option<SlotId>) {
        let slot = self.slot_mut(id);
        let value = slot.value.take().expect("waiting slot contains a command");
        let next = slot.next.take();
        slot.generation = slot.generation.wrapping_add(1);
        self.free.push(id.index);
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
enum LaneState {
    Ready,
    Running {
        collapse_group: &'static CollapseGroup,
    },
}

struct KeyLane {
    state: LaneState,
    waiting_head: Option<SlotId>,
    waiting_tail: Option<SlotId>,
}

/// Fair, bounded scheduler for one worker's keyed requests.
pub(super) struct KeyScheduler<K, T: ScheduledTask> {
    lanes: HashMap<K, KeyLane>,
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

    pub(super) fn ready_is_exclusive(&self) -> bool {
        let Some(storage_key) = self.ready.front() else {
            return false;
        };
        let lane = self.lanes.get(storage_key).expect("ready key has a lane");
        let head = lane.waiting_head.expect("ready lane has a waiting command");
        self.waiting.get(head).is_exclusive()
    }

    /// Removes the next fair command and records its reducer family.
    pub(super) fn take_ready(&mut self) -> Option<(K, T)> {
        let storage_key = self.ready.pop_front()?;
        let head = self
            .lanes
            .get(&storage_key)
            .expect("ready key has a lane")
            .waiting_head
            .expect("ready lane has a waiting command");
        let collapse_group = self.waiting.get(head).collapse_group();
        let (command, next) = self.waiting.take(head);
        let lane = self
            .lanes
            .get_mut(&storage_key)
            .expect("ready key has a lane");
        debug_assert_eq!(lane.state, LaneState::Ready);
        lane.waiting_head = next;
        if next.is_none() {
            lane.waiting_tail = None;
        }
        lane.state = LaneState::Running { collapse_group };
        Some((storage_key, command))
    }

    pub(super) fn take_ready_exclusive(&mut self) -> Option<(K, T)> {
        if !self.ready_is_exclusive() {
            return None;
        }
        self.take_ready()
    }

    pub(super) fn enqueue(
        &mut self,
        storage_key: K,
        command: T,
    ) -> std::result::Result<(), SchedulerError> {
        let slot = self.waiting.insert(command).ok_or(SchedulerError::Full)?;
        if let Some(lane) = self.lanes.get_mut(&storage_key) {
            if let Some(tail) = lane.waiting_tail {
                self.waiting.link(tail, slot);
            } else {
                debug_assert!(lane.waiting_head.is_none());
                lane.waiting_head = Some(slot);
            }
            lane.waiting_tail = Some(slot);
            return Ok(());
        }
        self.lanes.insert(
            storage_key.clone(),
            KeyLane {
                state: LaneState::Ready,
                waiting_head: Some(slot),
                waiting_tail: Some(slot),
            },
        );
        self.ready.push_back(storage_key);
        Ok(())
    }

    /// Takes the contiguous prefix compatible with the running reducer.
    pub(super) fn take_collapsible(
        &mut self,
        storage_key: K,
        mut can_collapse: impl FnMut(&T) -> bool,
    ) -> Vec<T> {
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
            return Vec::new();
        };

        let mut commands = Vec::new();
        loop {
            let command = self.waiting.get(head);
            if !std::ptr::eq(command.collapse_group(), collapse_group) || !can_collapse(command) {
                break;
            }
            let (command, next) = self.waiting.take(head);
            let lane = self
                .lanes
                .get_mut(&storage_key)
                .expect("completed key has a lane");
            lane.waiting_head = next;
            if next.is_none() {
                lane.waiting_tail = None;
            }
            commands.push(command);
            let Some(next_head) = next else {
                break;
            };
            head = next_head;
        }
        commands
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
