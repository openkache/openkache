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
        }
    }

    /// Removes expired entries and checks one token/fingerprint pair.
    pub fn check(
        &mut self,
        mutation_id: MutationId,
        fingerprint: [u8; 32],
        now: Instant,
    ) -> MutationDecision {
        match self.lookup(mutation_id, fingerprint, now) {
            MutationDecision::New => {
                if self.reserve(mutation_id, fingerprint, now) {
                    MutationDecision::New
                } else {
                    MutationDecision::Capacity
                }
            }
            decision => decision,
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
        let existing = self.entries.contains_key(&mutation_id);
        if existing {
            self.order.retain(|candidate| candidate != &mutation_id);
            self.order.push_back(mutation_id);
        } else if !self.reserve(mutation_id, fingerprint, now) {
            // A completed result is useful only while it is retained. If all
            // slots are occupied by active reservations, leave the response
            // untracked rather than evicting an in-flight mutation.
            return;
        }
        self.entries.insert(
            mutation_id,
            Entry {
                fingerprint,
                result: Some((status, payload)),
                expires_at: now + self.ttl,
            },
        );
    }

    /// Releases an in-flight reservation when request execution is abandoned.
    ///
    /// Completed replay entries are never removed by this method. The
    /// fingerprint check also prevents a stale guard from deleting a newer
    /// reservation after the token's TTL has elapsed.
    pub fn release_pending(
        &mut self,
        mutation_id: MutationId,
        fingerprint: [u8; 32],
        now: Instant,
    ) -> bool {
        self.purge(now);
        let is_pending = self
            .entries
            .get(&mutation_id)
            .is_some_and(|entry| entry.fingerprint == fingerprint && entry.result.is_none());
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

    fn reserve(&mut self, mutation_id: MutationId, fingerprint: [u8; 32], now: Instant) -> bool {
        self.purge(now);
        while self.entries.len() >= self.capacity {
            let Some(index) = self.order.iter().position(|candidate| {
                self.entries
                    .get(candidate)
                    .is_some_and(|entry| entry.result.is_some())
            }) else {
                return false;
            };
            let Some(oldest) = self.order.remove(index) else {
                return false;
            };
            self.entries.remove(&oldest);
        }
        self.order.push_back(mutation_id);
        self.entries.insert(
            mutation_id,
            Entry {
                fingerprint,
                result: None,
                expires_at: now + self.ttl,
            },
        );
        true
    }
}
