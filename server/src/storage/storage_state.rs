use std::cell::RefCell;
use std::io;
use std::rc::Rc;
use std::sync::Arc;

use compio::buf::IoBuf;
use compio::fs::File;
use compio::io::AsyncWriteAtExt;
use synchrony::unsync::event::Event;

use crate::config::StorageConfig;
use crate::storage_message::{Reply, StorageKey};

mod sg;
mod table;

use sg::{CandidateLookup, MutableSg, SgState};
use table::{Table, TableLocation};

const MUTABLE_SG_COUNT: usize = 3;

#[derive(Clone)]
struct FlushBuffer(Rc<MutableSg>);

impl IoBuf for FlushBuffer {
    fn as_init(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

enum CandidateSource {
    Ram(CandidateLookup),
    Disk { start: u64, end: u64 },
    Unavailable,
}

pub(super) struct StorageState {
    storage_file: Rc<File>,
    table: Table,
    sgs: Box<[SgState]>,
    oldest_mutable_sg_index: usize,
    reusable_buffers: Vec<MutableSg>,
    reusable_buffer_returned: Event,
    storage_file_bytes: u64,
    next_record_start: u64,
    fatal_io_error: Option<io::Error>,
}

impl StorageState {
    pub(super) fn new(config: &StorageConfig, storage_file: Rc<File>) -> Self {
        assert!(
            config.storage_sg_count > MUTABLE_SG_COUNT,
            "storage needs three Mutable SGs plus at least one non-Mutable slot"
        );

        let mut sgs: Box<[SgState]> = (0..config.storage_sg_count)
            .map(|_| SgState::Unused)
            .collect();
        for state in &mut sgs[..MUTABLE_SG_COUNT] {
            *state = SgState::Mutable(MutableSg::new(config));
        }

        Self {
            storage_file,
            table: Table::new(config),
            sgs,
            oldest_mutable_sg_index: 0,
            reusable_buffers: vec![MutableSg::new(config)],
            reusable_buffer_returned: Event::new(),
            storage_file_bytes: config.storage_file_bytes,
            next_record_start: 0,
            fatal_io_error: None,
        }
    }

    pub(super) async fn get(
        storage_state: Rc<RefCell<Self>>,
        key: StorageKey,
    ) -> io::Result<Reply> {
        let value = Self::lookup(storage_state, &key)
            .await?
            .map(|(_, value)| value);
        Ok(Reply::Get(value))
    }

    pub(super) async fn set(
        storage_state: Rc<RefCell<Self>>,
        key: StorageKey,
        value: Arc<[u8]>,
    ) -> io::Result<Reply> {
        if !sg::value_fits_in_empty_sg(value.as_ref()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "item does not fit in an empty Segment+Blob SG",
            ));
        }

        let previous = Self::lookup(Rc::clone(&storage_state), &key).await?;
        let previous_table_location = previous.as_ref().map(|(table_location, _)| *table_location);
        {
            let mut storage_state = storage_state.borrow_mut();
            // sg 에 inplace replace 를 시도하기
            if let Some(previous_table_location) = previous_table_location
                && let Some(SgState::Mutable(sg)) =
                    storage_state.sgs.get_mut(previous_table_location.sg_index)
                && sg.replace(&key, previous_table_location.bucket_choice, value.as_ref())
            {
                return Ok(Reply::SetOk);
            }
        }

        let new_table_location = loop {
            if let Some(table_location) = storage_state.borrow_mut().append_to_mutable_sg(
                &key,
                value.as_ref(),
                previous_table_location,
            ) {
                break table_location;
            }

            let sealed_sg_index = {
                let mut state = storage_state.borrow_mut();

                match state.reusable_buffers.pop() {
                    Some(reusable_buffer) => {
                        state.seal_oldest_mutable_and_open_buffer(reusable_buffer)?
                    }
                    None => {
                        // Register while this borrow still proves that the pool is
                        // empty, then release the borrow before yielding.
                        let listener = state.reusable_buffer_returned.listen();
                        drop(state);
                        listener.await;
                        continue;
                    }
                }
            };

            let flush_storage_state = Rc::clone(&storage_state);
            compio::runtime::spawn(async move {
                if let Err(error) =
                    Self::flush_sealed_sg(Rc::clone(&flush_storage_state), sealed_sg_index).await
                {
                    flush_storage_state
                        .borrow_mut()
                        .record_fatal_io_error(error);
                }
            })
            .detach();
        };

        let mut storage_state = storage_state.borrow_mut();

        // The new route is published before the old route is removed, so a SET
        // never creates a moment in which its key has no Table candidate.
        storage_state.table.insert(&key, new_table_location);

        if let Some(previous_table_location) = previous_table_location {
            match storage_state.sgs.get_mut(previous_table_location.sg_index) {
                Some(SgState::Mutable(sg)) => {
                    let removed = sg.remove(&key, previous_table_location.bucket_choice);
                    debug_assert!(removed, "SET just found this key in the Mutable SG");

                    // Mutable bytes were removed by full key. Remove exactly one
                    // duplicate candidate, leaving colliding keys reachable.
                    storage_state
                        .table
                        .remove_one(&key, previous_table_location);
                }
                Some(
                    SgState::Sealed(_)
                    | SgState::Flushing { .. }
                    | SgState::Stable { .. }
                    | SgState::Evicting { .. },
                )
                | Some(SgState::Unused)
                | None => {
                    // Immutable bytes still contain the old value. Leaving even
                    // one duplicate route could expose that value again after the
                    // new value is evicted, so invalidate the entire old route.
                    storage_state
                        .table
                        .remove_all(&key, previous_table_location);
                }
            }
        }

        Ok(Reply::SetOk)
    }

    pub(super) async fn delete(
        storage_state: Rc<RefCell<Self>>,
        key: StorageKey,
    ) -> io::Result<Reply> {
        let Some((table_location, _)) = Self::lookup(Rc::clone(&storage_state), &key).await? else {
            return Ok(Reply::Delete(false));
        };

        let mut storage_state = storage_state.borrow_mut();
        match storage_state.sgs.get_mut(table_location.sg_index) {
            Some(SgState::Mutable(sg)) => {
                let removed = sg.remove(&key, table_location.bucket_choice);
                debug_assert!(removed, "DELETE just found this key in the Mutable SG");
                storage_state.table.remove_one(&key, table_location);
            }
            Some(
                SgState::Sealed(_)
                | SgState::Flushing { .. }
                | SgState::Stable { .. }
                | SgState::Evicting { .. },
            )
            | Some(SgState::Unused)
            | None => {
                // Immutable bytes cannot be edited. Removing every identical
                // candidate prevents the deleted value from being found through
                // a duplicate entry that belongs to a colliding key.
                storage_state.table.remove_all(&key, table_location);
            }
        }

        Ok(Reply::Delete(true))
    }

    /// Seals and durably writes every SG in the current Mutable window.
    pub(super) async fn flush(storage_state: Rc<RefCell<Self>>) -> io::Result<Reply> {
        for _ in 0..MUTABLE_SG_COUNT {
            let sealed_sg_index = loop {
                let listener = {
                    let mut storage_state = storage_state.borrow_mut();
                    match storage_state.reusable_buffers.pop() {
                        Some(reusable_buffer) => {
                            break storage_state
                                .seal_oldest_mutable_and_open_buffer(reusable_buffer)?;
                        }
                        None => storage_state.reusable_buffer_returned.listen(),
                    }
                };

                listener.await;
            };

            Self::flush_sealed_sg(Rc::clone(&storage_state), sealed_sg_index).await?;
        }

        Ok(Reply::Flush(Ok(())))
    }

    /// Looks up one full key for GET, SET, and DELETE.
    ///
    /// The candidate snapshot and every SSD result are owned, so neither a Table
    /// borrow nor an SG borrow crosses `await`. Scheduler serialization guarantees
    /// that this exact key cannot be changed by another request while lookup yields.
    async fn lookup(
        storage_state: Rc<RefCell<Self>>,
        key: &StorageKey,
    ) -> io::Result<Option<(TableLocation, Arc<[u8]>)>> {
        let (candidates, storage_file) = {
            let storage_state = storage_state.borrow();
            (
                storage_state.table.candidates(key),
                Rc::clone(&storage_state.storage_file),
            )
        };

        for table_location in candidates {
            let source = {
                let mut storage_state = storage_state.borrow_mut();
                match storage_state.sgs.get_mut(table_location.sg_index) {
                    Some(SgState::Mutable(sg) | SgState::Sealed(sg)) => {
                        CandidateSource::Ram(sg.lookup(key, table_location.bucket_choice))
                    }

                    Some(SgState::Flushing { sg, .. }) => {
                        CandidateSource::Ram(sg.lookup(key, table_location.bucket_choice))
                    }

                    Some(SgState::Stable {
                        start,
                        end,
                        pin_count,
                    }) => {
                        *pin_count = pin_count.checked_add(1).expect("SSD read pin overflow");
                        CandidateSource::Disk {
                            start: *start,
                            end: *end,
                        }
                    }

                    // Eviction owns Table-route cleanup. Lookup may observe this
                    // state before that cleanup finishes, but must not race it.
                    Some(SgState::Evicting { .. }) => CandidateSource::Unavailable,

                    Some(SgState::Unused) | None => {
                        unreachable!("a Table route must reference a live SG")
                    }
                }
            };

            let lookup = match source {
                CandidateSource::Ram(lookup) => lookup,
                CandidateSource::Unavailable => continue,
                CandidateSource::Disk { start, end } => {
                    let lookup = sg::read_candidate(
                        Rc::clone(&storage_file),
                        start,
                        end,
                        key,
                        table_location.bucket_choice,
                    )
                    .await;

                    // Unpin before propagating a read error. Otherwise one failed
                    // read would permanently prevent physical-range reuse.
                    {
                        let mut storage_state = storage_state.borrow_mut();
                        match storage_state.sgs.get_mut(table_location.sg_index) {
                            Some(SgState::Stable { pin_count, .. }) => {
                                assert!(*pin_count != 0, "SSD read pin underflow");
                                *pin_count -= 1;
                            }
                            Some(SgState::Evicting {
                                pin_count,
                                wake_flush,
                                ..
                            }) => {
                                assert!(*pin_count != 0, "SSD read pin underflow");
                                *pin_count -= 1;
                                if *pin_count == 0 {
                                    wake_flush.notify(1);
                                }
                            }
                            _ => unreachable!("a pinned physical record cannot be reused"),
                        }
                    }

                    lookup?
                }
            };

            match lookup {
                CandidateLookup::Value(value) => {
                    return Ok(Some((table_location, value)));
                }
                CandidateLookup::TableIdentityCollision => continue,
            }
        }

        Ok(None)
    }

    fn append_to_mutable_sg(
        &mut self,
        key: &StorageKey,
        value: &[u8],
        previous_table_location: Option<TableLocation>,
    ) -> Option<TableLocation> {
        for mutable_offset in 0..MUTABLE_SG_COUNT {
            let sg_index = (self.oldest_mutable_sg_index + mutable_offset) % self.sgs.len();
            let SgState::Mutable(sg) = &mut self.sgs[sg_index] else {
                break;
            };

            let previous_bucket_choice_in_this_sg = match previous_table_location {
                Some(table_location) if table_location.sg_index == sg_index => {
                    Some(table_location.bucket_choice)
                }
                Some(_) | None => None,
            };

            let Some(bucket_choice) =
                sg.try_insert_into_best_bucket(key, value, previous_bucket_choice_in_this_sg)
            else {
                continue;
            };

            return Some(TableLocation {
                sg_index,
                bucket_choice,
            });
        }

        None
    }

    /// Freezes the oldest Mutable SG and opens the next circular SG with the
    /// supplied reusable buffer. The next SG must already be Unused, and the
    /// three Mutable SGs must form the window starting at
    /// `oldest_mutable_sg_index`. This advances that cursor and returns the
    /// newly Sealed SG index; it performs no I/O and never yields.
    fn seal_oldest_mutable_and_open_buffer(
        &mut self,
        reusable_buffer: MutableSg,
    ) -> io::Result<usize> {
        let sealed_sg_index = self.oldest_mutable_sg_index;
        let opened_sg_index = (sealed_sg_index + MUTABLE_SG_COUNT) % self.sgs.len();

        assert_ne!(
            sealed_sg_index, opened_sg_index,
            "storage needs at least one non-Mutable SG slot",
        );
        assert!(
            matches!(&self.sgs[sealed_sg_index], SgState::Mutable(_)),
            "oldest SG must be Mutable",
        );
        assert!(
            matches!(&self.sgs[opened_sg_index], SgState::Unused),
            "the SG after the Mutable window must be Unused",
        );

        let old_state = std::mem::replace(&mut self.sgs[sealed_sg_index], SgState::Unused);
        let SgState::Mutable(sealed_sg) = old_state else {
            unreachable!("oldest SG was checked above");
        };

        self.sgs[sealed_sg_index] = SgState::Sealed(sealed_sg);
        self.sgs[opened_sg_index] = SgState::Mutable(reusable_buffer);
        self.oldest_mutable_sg_index = (sealed_sg_index + 1) % self.sgs.len();

        Ok(sealed_sg_index)
    }

    /// Reserves one contiguous Segment+Blob record and evicts every overlapping
    /// Stable SG, removing its Table routes and waiting for its SSD read pins.
    /// Once the range is exclusive, this changes Sealed to Flushing and awaits
    /// the write CQE. Success publishes Stable, resets and returns the RAM buffer,
    /// and wakes waiters; failure retains the buffer and records a fatal error.
    async fn flush_sealed_sg(
        storage_state: Rc<RefCell<Self>>,
        sealed_sg_index: usize,
    ) -> io::Result<()> {
        let wake_flush = Event::new();
        let (storage_file, start, end, victims) = {
            let mut storage_state = storage_state.borrow_mut();
            let record_len = match storage_state.sgs.get(sealed_sg_index) {
                Some(SgState::Sealed(sg)) => u64::try_from(sg.as_bytes().len())
                    .map_err(|_| io::Error::other("SG record length does not fit in u64"))?,
                _ => unreachable!("flush target must be Sealed"),
            };
            if record_len > storage_state.storage_file_bytes {
                return Err(io::Error::other("SG record does not fit in storage file"));
            }

            let start = storage_state
                .next_record_start
                .checked_add(record_len)
                .filter(|end| *end <= storage_state.storage_file_bytes)
                .map_or(0, |_| storage_state.next_record_start);
            let end = start + record_len;
            let mut victims = Vec::new();

            for sg_index in 0..storage_state.sgs.len() {
                let overlaps = match &storage_state.sgs[sg_index] {
                    SgState::Stable {
                        start: victim_start,
                        end: victim_end,
                        ..
                    } => *victim_start < end && start < *victim_end,
                    _ => false,
                };
                if !overlaps {
                    continue;
                }

                let old_state =
                    std::mem::replace(&mut storage_state.sgs[sg_index], SgState::Unused);
                let SgState::Stable {
                    start,
                    end,
                    pin_count,
                } = old_state
                else {
                    unreachable!("overlapping victim was checked as Stable");
                };

                storage_state.sgs[sg_index] = SgState::Evicting {
                    start,
                    end,
                    pin_count,
                    wake_flush: wake_flush.clone(),
                };
                victims.push((sg_index, start, end));
            }

            (Rc::clone(&storage_state.storage_file), start, end, victims)
        };

        for &(victim_sg_index, victim_start, victim_end) in &victims {
            let routes =
                sg::read_record_routes(Rc::clone(&storage_file), victim_start, victim_end).await?;
            let mut storage_state = storage_state.borrow_mut();
            for route in routes {
                storage_state.table.remove_one(
                    &route.key,
                    TableLocation {
                        sg_index: victim_sg_index,
                        bucket_choice: route.bucket_choice,
                    },
                );
            }
        }

        let listeners = {
            let storage_state = storage_state.borrow();
            victims
                .iter()
                .filter_map(|&(sg_index, _, _)| match &storage_state.sgs[sg_index] {
                    SgState::Evicting { pin_count, .. } => {
                        (*pin_count != 0).then(|| wake_flush.listen())
                    }
                    _ => unreachable!("victim must remain Evicting"),
                })
                .collect::<Vec<_>>()
        };

        for listener in listeners {
            listener.await;
        }

        let write_buffer = {
            let mut storage_state = storage_state.borrow_mut();
            for &(sg_index, _, _) in &victims {
                assert!(matches!(
                    &storage_state.sgs[sg_index],
                    SgState::Evicting { pin_count: 0, .. }
                ));
                storage_state.sgs[sg_index] = SgState::Unused;
            }

            let old_state =
                std::mem::replace(&mut storage_state.sgs[sealed_sg_index], SgState::Unused);
            let SgState::Sealed(sealed_sg) = old_state else {
                unreachable!("flush target must remain Sealed while evicting");
            };
            let sealed_sg = Rc::new(sealed_sg);
            storage_state.sgs[sealed_sg_index] = SgState::Flushing {
                sg: Rc::clone(&sealed_sg),
                start,
                end,
            };
            storage_state.next_record_start = if end == storage_state.storage_file_bytes {
                0
            } else {
                end
            };
            FlushBuffer(sealed_sg)
        };

        let mut file = &*storage_file;
        let (result, returned_buffer) = file.write_all_at(write_buffer, start).await.into_parts();
        if let Err(error) = result {
            drop(returned_buffer);
            return Err(error);
        }

        let mut storage_state = storage_state.borrow_mut();
        let old_state = std::mem::replace(
            &mut storage_state.sgs[sealed_sg_index],
            SgState::Stable {
                start,
                end,
                pin_count: 0,
            },
        );
        let SgState::Flushing { sg, .. } = old_state else {
            unreachable!("completed flush must still own a Flushing SG");
        };
        drop(returned_buffer);
        let mut reusable_buffer = Rc::try_unwrap(sg)
            .unwrap_or_else(|_| panic!("Flushing state must be the only SG buffer owner"));
        reusable_buffer.clear();
        storage_state.reusable_buffers.push(reusable_buffer);
        storage_state.reusable_buffer_returned.notify_all();
        Ok(())
    }

    fn record_fatal_io_error(&mut self, error: io::Error) {
        if self.fatal_io_error.is_none() {
            self.fatal_io_error = Some(error);
        }
    }

    pub(super) fn take_fatal_io_error(&mut self) -> Option<io::Error> {
        self.fatal_io_error.take()
    }
}
