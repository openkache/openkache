//! One SG combines its fixed Bucket Segment and variable-length Blob.
//!
//! `StorageState` needs only these operations from this module:
//!
//! - create or reset a reusable [`MutableSg`];
//! - synchronously get, insert into the best Bucket, replace, or remove a full
//!   key in RAM;
//! - derive a Bucket from the full [`StorageKey`](crate::storage_message::StorageKey)
//!   and `bucket_choice` for both RAM access and SSD offsets;
//! - expose the combined Segment+Blob record length and immutable write view;
//! - validate and inspect Bucket/Blob bytes read back from SSD.
//!
//! This module does not select physical records, pin reads, submit I/O, publish
//! flushes, or evict data. Those transitions belong exclusively to
//! `StorageState`.

mod bucket;

use std::io;
use std::rc::Rc;
use std::sync::Arc;

use compio::fs::File;
use synchrony::unsync::event::Event;

use crate::config::StorageConfig;
use crate::storage_message::StorageKey;

pub(super) enum CandidateLookup {
    Value(Arc<[u8]>),
    TableIdentityCollision,
}

pub(super) struct StoredRoute {
    pub(super) key: StorageKey,
    pub(super) bucket_choice: u8,
}

/// Data lifecycle only. `StorageState` owns every transition between variants.
pub(super) enum SgState {
    /// No logical SG currently occupies this slot.
    Unused,
    /// Accepts writes and serves reads from its RAM buffer.
    Mutable(MutableSg),
    /// Frozen RAM buffer waiting for a writable physical record.
    Sealed(MutableSg),
    /// Immutable RAM buffer whose write SQE owns the claimed physical range.
    Flushing {
        sg: Rc<MutableSg>,
        start: u64,
        end: u64,
    },
    /// Published on SSD. Only asynchronous SSD reads modify `pin_count`.
    Stable {
        start: u64,
        end: u64,
        pin_count: usize,
    },
    /// Closed to new reads while its old readers drain and its routes are removed.
    Evicting {
        start: u64,
        end: u64,
        pin_count: usize,
        /// Every victim of one pending flush shares a clone of the same Event.
        wake_flush: Event,
    },
}

/// Reusable RAM representation of one combined Segment+Blob record.
pub(super) struct MutableSg;

impl MutableSg {
    pub(super) fn new(_config: &StorageConfig) -> Self {
        todo!("allocate the Segment+Blob SG buffer")
    }

    pub(super) fn lookup(&self, _key: &StorageKey, _bucket_choice: u8) -> CandidateLookup {
        todo!("look up the full key in the selected RAM Bucket")
    }

    pub(super) fn try_insert_into_best_bucket(
        &mut self,
        _key: &StorageKey,
        _value: &[u8],
        _excluded_bucket_choice: Option<u8>,
    ) -> Option<u8> {
        todo!("insert into the least-filled candidate Bucket")
    }

    pub(super) fn replace(&mut self, _key: &StorageKey, _bucket_choice: u8, _value: &[u8]) -> bool {
        todo!("replace one full-key item in a Mutable SG")
    }

    pub(super) fn remove(&mut self, _key: &StorageKey, _bucket_choice: u8) -> bool {
        todo!("remove one full-key item from a Mutable SG")
    }

    pub(super) fn as_bytes(&self) -> &[u8] {
        todo!("return the initialized Segment+Blob record bytes")
    }

    pub(super) fn clear(&mut self) {
        todo!("reset authoritative SG metadata while retaining its allocation")
    }
}

pub(super) fn value_fits_in_empty_sg(_value: &[u8]) -> bool {
    todo!("check the maximum value size against an empty SG")
}

pub(super) async fn read_candidate(
    _storage_file: Rc<File>,
    _start: u64,
    _end: u64,
    _key: &StorageKey,
    _bucket_choice: u8,
) -> io::Result<CandidateLookup> {
    todo!("read and validate one SSD candidate")
}

pub(super) async fn read_record_routes(
    _storage_file: Rc<File>,
    _start: u64,
    _end: u64,
) -> io::Result<Box<[StoredRoute]>> {
    todo!("read every full key and Bucket choice from an SSD SG record")
}
