use std::collections::{HashMap, VecDeque, hash_map::Entry};

use crate::storage_message::{Command, StorageKey};

use super::RoutedRequest;

pub(super) struct Scheduler {
    key_queues: HashMap<StorageKey, KeyQueue>,
    ready_keys: VecDeque<StorageKey>,
    pending_flushes: VecDeque<RoutedRequest>,
    deferred_requests: VecDeque<RoutedRequest>,
    flush_in_flight: bool,
}

struct KeyQueue {
    active: bool,
    pending: VecDeque<RoutedRequest>,
}

/*
RoutedRequest : struct RoutedRequest {
    channel_id: ChannelId,
    request: StorageRequest,
}
key queue -> 지금 내 key 들의 직렬화된 요청 큐들.
*/

impl Scheduler {
    pub(super) fn new() -> Self {
        Self {
            key_queues: HashMap::new(),
            ready_keys: VecDeque::new(),
            pending_flushes: VecDeque::new(),
            deferred_requests: VecDeque::new(),
            flush_in_flight: false,
        }
    }

    pub(super) fn enqueue(&mut self, request: RoutedRequest) {
        match &request.request.command {
            Command::Flush => self.pending_flushes.push_back(request),

            Command::Get { .. } | Command::Set { .. } | Command::Delete { .. }
                if self.flush_in_flight || !self.pending_flushes.is_empty() =>
            {
                // Once the first Flush has arrived, later keyed requests wait
                // behind the whole Flush batch. Additional Flush requests may
                // therefore move ahead of these deferred requests.
                self.deferred_requests.push_back(request);
            }

            Command::Get { key } | Command::Set { key, .. } | Command::Delete { key } => {
                self.enqueue_key(*key, request);
            }
        }
    }

    fn enqueue_key(&mut self, key: StorageKey, request: RoutedRequest) {
        match self.key_queues.entry(key) {
            Entry::Vacant(entry) => {
                entry.insert(KeyQueue {
                    active: false,
                    pending: VecDeque::from([request]),
                });
                self.ready_keys.push_back(key);
            }
            Entry::Occupied(mut entry) => entry.get_mut().pending.push_back(request),
        }
    }

    pub(super) fn take_ready(&mut self) -> Option<(Option<StorageKey>, RoutedRequest)> {
        if self.flush_in_flight {
            return None;
        }

        if let Some(key) = self.ready_keys.pop_front() {
            let key_queue = self
                .key_queues
                .get_mut(&key)
                .expect("a ready key must have a queue");
            assert!(!key_queue.active, "a ready key cannot already be active");

            key_queue.active = true;
            return Some((Some(key), key_queue.pending.pop_front().unwrap()));
        }

        // A Flush starts only after every keyed request accepted before the
        // first Flush has completed, including same-key requests still queued
        // behind an active request.
        if !self.key_queues.is_empty() {
            return None;
        }

        let flush = self.pending_flushes.pop_front()?;
        self.flush_in_flight = true;
        Some((None, flush))
    }

    pub(super) fn finish(&mut self, key: Option<StorageKey>) {
        let Some(key) = key else {
            assert!(self.flush_in_flight, "finished a Flush that was not active");
            self.flush_in_flight = false;

            if self.pending_flushes.is_empty() {
                while let Some(request) = self.deferred_requests.pop_front() {
                    let key = match &request.request.command {
                        Command::Get { key }
                        | Command::Set { key, .. }
                        | Command::Delete { key } => *key,
                        Command::Flush => unreachable!("Flush requests have their own queue"),
                    };
                    self.enqueue_key(key, request);
                }
            }
            return;
        };

        let key_queue = self
            .key_queues
            .get_mut(&key)
            .expect("finished a key with no queue");
        assert!(key_queue.active, "finished a key that was not active");

        key_queue.active = false;
        if key_queue.pending.is_empty() {
            self.key_queues.remove(&key);
        } else {
            self.ready_keys.push_back(key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Scheduler;
    use crate::storage::{ChannelId, RoutedRequest};
    use crate::storage_message::{ClientId, Command, StorageKey, StorageRequest};

    #[test]
    fn serializes_the_same_key_without_blocking_other_keys() {
        let mut scheduler = Scheduler::new();
        scheduler.enqueue(keyed_request(1, b"first"));
        scheduler.enqueue(keyed_request(2, b"first"));
        scheduler.enqueue(keyed_request(3, b"second"));

        let (first_key, first) = scheduler.take_ready().unwrap();
        assert_eq!(first.request.sequence, 1);
        let (second_key, second) = scheduler.take_ready().unwrap();
        assert_eq!(second.request.sequence, 3);
        assert!(scheduler.take_ready().is_none());

        scheduler.finish(first_key);
        let (same_key, same) = scheduler.take_ready().unwrap();
        assert_eq!(same.request.sequence, 2);

        scheduler.finish(second_key);
        scheduler.finish(same_key);
    }

    #[test]
    fn batches_additional_flushes_ahead_of_later_keyed_requests() {
        let mut scheduler = Scheduler::new();
        scheduler.enqueue(keyed_request(1, b"before"));
        scheduler.enqueue(flush_request(2));
        scheduler.enqueue(keyed_request(3, b"after-first-flush"));
        scheduler.enqueue(flush_request(4));
        scheduler.enqueue(keyed_request(5, b"after-second-flush"));

        let (before_key, before) = scheduler.take_ready().unwrap();
        assert_eq!(before.request.sequence, 1);
        assert!(scheduler.take_ready().is_none());

        scheduler.finish(before_key);

        let (first_flush_key, first_flush) = scheduler.take_ready().unwrap();
        assert_eq!(first_flush_key, None);
        assert_eq!(first_flush.request.sequence, 2);
        assert!(scheduler.take_ready().is_none());

        scheduler.finish(first_flush_key);

        let (second_flush_key, second_flush) = scheduler.take_ready().unwrap();
        assert_eq!(second_flush_key, None);
        assert_eq!(second_flush.request.sequence, 4);
        assert!(scheduler.take_ready().is_none());

        scheduler.finish(second_flush_key);

        let (_, after_first_flush) = scheduler.take_ready().unwrap();
        assert_eq!(after_first_flush.request.sequence, 3);
        let (_, after_second_flush) = scheduler.take_ready().unwrap();
        assert_eq!(after_second_flush.request.sequence, 5);
    }

    #[test]
    fn drains_same_key_requests_accepted_before_a_flush() {
        let mut scheduler = Scheduler::new();
        scheduler.enqueue(keyed_request(1, b"key"));
        scheduler.enqueue(keyed_request(2, b"key"));
        scheduler.enqueue(flush_request(3));

        let (first_key, first) = scheduler.take_ready().unwrap();
        assert_eq!(first.request.sequence, 1);
        scheduler.finish(first_key);

        let (second_key, second) = scheduler.take_ready().unwrap();
        assert_eq!(second.request.sequence, 2);
        scheduler.finish(second_key);

        let (flush_key, flush) = scheduler.take_ready().unwrap();
        assert_eq!(flush_key, None);
        assert_eq!(flush.request.sequence, 3);
    }

    fn keyed_request(sequence: u64, key: &[u8]) -> RoutedRequest {
        RoutedRequest {
            channel_id: ChannelId(0),
            request: StorageRequest {
                client_id: ClientId(0),
                sequence,
                command: Command::Get {
                    key: StorageKey::from_client_key(key),
                },
            },
        }
    }

    fn flush_request(sequence: u64) -> RoutedRequest {
        RoutedRequest {
            channel_id: ChannelId(0),
            request: StorageRequest {
                client_id: ClientId(0),
                sequence,
                command: Command::Flush,
            },
        }
    }
}
