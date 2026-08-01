//! Bounded server-side replay protection for mutating requests.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use openkache_protocol::MutationId;

/// Default number of completed mutation results retained for replay.
pub const DEFAULT_CAPACITY: usize = 65_536;
/// Default replay lifetime for a mutation token.
pub const DEFAULT_TTL: Duration = Duration::from_secs(10 * 60);

#[derive(Clone, Debug)]
struct Entry {
    generation: u64,
    fingerprint: [u8; 32],
    result: Option<(u8, Vec<u8>)>,
    expires_at: Instant,
}

/// Result of checking a mutation token before executing a request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MutationDecision {
    /// The request has not been seen and may be executed.
    New,
    /// The exact request was seen and its response can be replayed.
    Replay { status: u8, payload: Vec<u8> },
    /// The exact request is currently being executed by another stream.
    Pending,
    /// The token was previously used for different request bytes.
    Conflict,
    /// The bounded store cannot reserve another in-flight mutation.
    Capacity,
}

/// Fixed-capacity, time-bounded mutation result store.
#[derive(Debug)]
pub struct MutationDedupeStore {
    capacity: usize,
    ttl: Duration,
    entries: HashMap<MutationId, Entry>,
    order: VecDeque<MutationId>,
    next_generation: u64,
}

impl Default for MutationDedupeStore {
    fn default() -> Self {
        Self::new(DEFAULT_CAPACITY, DEFAULT_TTL)
    }
}

impl MutationDedupeStore {
    /// Creates a bounded replay store.
    pub fn new(capacity: usize, ttl: Duration) -> Self {
        assert!(capacity > 0, "mutation dedupe capacity must be positive");
        assert!(!ttl.is_zero(), "mutation dedupe TTL must be positive");
        Self {
            capacity,
            ttl,
            entries: HashMap::with_capacity(capacity),
            order: VecDeque::with_capacity(capacity),
            next_generation: 1,
        }
    }

    /// Removes expired entries and checks one token/fingerprint pair.
    #[allow(dead_code)]
    pub fn check(
        &mut self,
        mutation_id: MutationId,
        fingerprint: [u8; 32],
        now: Instant,
    ) -> MutationDecision {
        self.check_with_reservation(mutation_id, fingerprint, now).0
    }

    /// Checks one mutation and returns its reservation generation when new.
    ///
    /// The generation identifies one particular asynchronous execution. It
    /// prevents a completion from an expired reservation from overwriting a
    /// later reservation that reused the same token and request bytes.
    pub fn check_with_reservation(
        &mut self,
        mutation_id: MutationId,
        fingerprint: [u8; 32],
        now: Instant,
    ) -> (MutationDecision, Option<u64>) {
        match self.lookup(mutation_id, fingerprint, now) {
            MutationDecision::New => match self.reserve(mutation_id, fingerprint, now) {
                Some(generation) => (MutationDecision::New, Some(generation)),
                None => (MutationDecision::Capacity, None),
            },
            decision => (decision, None),
        }
    }

    /// Checks a token without reserving a new entry.
    ///
    /// This is used by a concurrent retry while the original request owns the
    /// reservation. A missing entry is reported as [`MutationDecision::New`]
    /// so callers can decide whether it is safe to execute or should fail.
    pub fn lookup(
        &mut self,
        mutation_id: MutationId,
        fingerprint: [u8; 32],
        now: Instant,
    ) -> MutationDecision {
        self.purge(now);
        let Some(entry) = self.entries.get(&mutation_id) else {
            return MutationDecision::New;
        };
        if entry.fingerprint != fingerprint {
            return MutationDecision::Conflict;
        }
        match &entry.result {
            Some((status, payload)) => MutationDecision::Replay {
                status: *status,
                payload: payload.clone(),
            },
            None => MutationDecision::Pending,
        }
    }

    /// Records a completed response for future replay.
    pub fn record(
        &mut self,
        mutation_id: MutationId,
        fingerprint: [u8; 32],
        status: u8,
        payload: Vec<u8>,
        now: Instant,
    ) {
        self.purge(now);
        if self
            .entries
            .get(&mutation_id)
            .is_some_and(|entry| entry.fingerprint != fingerprint)
        {
            // The token may have expired and been reserved again while an
            // earlier request was still finishing. Never let that stale
            // completion overwrite the newer request's reservation.
            return;
        }
        let generation = if let Some(entry) = self.entries.get(&mutation_id) {
            self.order.retain(|candidate| candidate != &mutation_id);
            self.order.push_back(mutation_id);
            entry.generation
        } else {
            let Some(generation) = self.reserve(mutation_id, fingerprint, now) else {
                // A completed result is useful only while it is retained. If
                // all slots are occupied by active reservations, leave the
                // response untracked rather than evicting an in-flight
                // mutation.
                return;
            };
            generation
        };
        self.entries.insert(
            mutation_id,
            Entry {
                generation,
                fingerprint,
                result: Some((status, payload)),
                expires_at: now + self.ttl,
            },
        );
    }

    /// Records a response only when it belongs to the active reservation.
    ///
    /// Returns `true` when the response was stored. A `false` result means the
    /// reservation expired, was released, or has already been replaced.
    pub fn record_with_reservation(
        &mut self,
        mutation_id: MutationId,
        fingerprint: [u8; 32],
        generation: u64,
        status: u8,
        payload: Vec<u8>,
        now: Instant,
    ) -> bool {
        self.purge(now);
        let Some(entry) = self.entries.get(&mutation_id) else {
            return false;
        };
        if entry.generation != generation
            || entry.fingerprint != fingerprint
            || entry.result.is_some()
        {
            return false;
        }
        self.order.retain(|candidate| candidate != &mutation_id);
        self.order.push_back(mutation_id);
        self.entries.insert(
            mutation_id,
            Entry {
                generation,
                fingerprint,
                result: Some((status, payload)),
                expires_at: now + self.ttl,
            },
        );
        true
    }

    /// Releases an in-flight reservation when request execution is abandoned.
    ///
    /// Completed replay entries are never removed by this method. The
    /// fingerprint check also prevents a stale guard from deleting a newer
    /// reservation after the token's TTL has elapsed.
    #[allow(dead_code)]
    pub fn release_pending(
        &mut self,
        mutation_id: MutationId,
        fingerprint: [u8; 32],
        now: Instant,
    ) -> bool {
        self.purge(now);
        let Some(generation) = self.entries.get(&mutation_id).and_then(|entry| {
            (entry.fingerprint == fingerprint && entry.result.is_none()).then_some(entry.generation)
        }) else {
            return false;
        };
        self.release_pending_with_reservation(mutation_id, fingerprint, generation, now)
    }

    /// Releases an in-flight reservation only when its generation is current.
    pub fn release_pending_with_reservation(
        &mut self,
        mutation_id: MutationId,
        fingerprint: [u8; 32],
        generation: u64,
        now: Instant,
    ) -> bool {
        self.purge(now);
        let is_pending = self.entries.get(&mutation_id).is_some_and(|entry| {
            entry.generation == generation
                && entry.fingerprint == fingerprint
                && entry.result.is_none()
        });
        if !is_pending {
            return false;
        }
        self.entries.remove(&mutation_id);
        self.order.retain(|candidate| candidate != &mutation_id);
        true
    }

    /// Returns the configured capacity.
    #[allow(dead_code)]
    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    /// Returns the configured replay TTL.
    #[allow(dead_code)]
    pub const fn ttl(&self) -> Duration {
        self.ttl
    }

    fn purge(&mut self, now: Instant) {
        self.order.retain(|mutation_id| {
            let keep = self
                .entries
                .get(mutation_id)
                .is_some_and(|entry| entry.expires_at > now);
            if !keep {
                self.entries.remove(mutation_id);
            }
            keep
        });
    }

    fn reserve(
        &mut self,
        mutation_id: MutationId,
        fingerprint: [u8; 32],
        now: Instant,
    ) -> Option<u64> {
        self.purge(now);
        while self.entries.len() >= self.capacity {
            let index = self.order.iter().position(|candidate| {
                self.entries
                    .get(candidate)
                    .is_some_and(|entry| entry.result.is_some())
            })?;
            let oldest = self.order.remove(index)?;
            self.entries.remove(&oldest);
        }
        let generation = self.next_generation;
        self.next_generation = if generation == u64::MAX {
            1
        } else {
            generation + 1
        };
        self.order.push_back(mutation_id);
        self.entries.insert(
            mutation_id,
            Entry {
                generation,
                fingerprint,
                result: None,
                expires_at: now + self.ttl,
            },
        );
        Some(generation)
    }
}
