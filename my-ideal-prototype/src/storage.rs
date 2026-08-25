use std::io;
use std::sync::Arc;
pub(crate) mod bucket;
pub(crate) mod sg;
pub(crate) mod table;

use crate::spsc::{Consumer, Producer};
use crate::storage_message::{
    Command, Reply, STORAGE_QUEUE_SLOTS, StorageRequest, StorageResponse,
};
use bucket::BucketValue;
use sg::MutableSg;
use table::{Table, TableConfig, TableError, TableLocation};

const STORAGE_KEY_BYTES: usize = 32;
const MUTABLE_SG_COUNT: usize = 3;
const MUTABLE_SG_BUCKET_COUNT: usize = 65_536;

/// Storage 안에서 key를 식별할 때 사용하는 고정 크기 key다.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct StorageKey([u8; STORAGE_KEY_BYTES]);

impl StorageKey {
    /// client key를 BLAKE3로 고정 크기 StorageKey로 바꾼다.
    pub(crate) fn from_key(key: &[u8]) -> Self {
        Self(*blake3::hash(key).as_bytes())
    }

    /// 이미 계산된 32바이트 key를 감싼다.
    pub(crate) const fn new(bytes: [u8; STORAGE_KEY_BYTES]) -> Self {
        Self(bytes)
    }

    /// SG가 전체 key를 비교할 때 사용할 32바이트를 반환한다.
    pub(crate) const fn as_bytes(&self) -> &[u8; STORAGE_KEY_BYTES] {
        &self.0
    }

    /// Table의 Subtable, unary index, fingerprint를 계산할 128비트를 반환한다.
    pub(crate) fn table_hash(&self) -> u128 {
        u128::from_le_bytes(self.0[8..24].try_into().unwrap())
    }
}

/// 1차 mutable storage에서 SET이 실패한 이유다.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StorageSetError {
    /// 세 mutable SG 어디에도 Item을 넣을 공간이 없다.
    MutableSgsFull,
    /// Table 삽입이나 할당이 실패했다.
    Table(TableError),
    /// Bucket에서 확인한 기존 위치를 Table에서 갱신하지 못했다.
    TableLocationMissing,
}

/// Table 하나와 동시에 쓰기 가능한 mutable SG 세 개를 연결한 1차 storage다.
pub(crate) struct Storage {
    table: Table,
    mutable_sgs: [MutableSg; MUTABLE_SG_COUNT],
}

impl Storage {
    /// SG 0, 1, 2와 이 위치들을 가리킬 Table을 만든다.
    pub(crate) fn new(
        table_config: TableConfig,
        buckets_per_sg: usize,
    ) -> Result<Self, TableError> {
        if table_config.sg_index_bits < 2 {
            return Err(TableError::InvalidConfig(
                "three mutable SGs require at least two SG index bits",
            ));
        }

        let table = Table::new(table_config)?;
        let mutable_sgs = std::array::from_fn(|sg_index| {
            MutableSg::new(
                sg_index as u32,
                buckets_per_sg,
                table_config.bucket_choice_count,
            )
        });
        Ok(Self { table, mutable_sgs })
    }

    /// Table 후보를 따라가 full StorageKey가 일치하는 live value를 반환한다.
    pub(crate) fn get(&self, storage_key: &StorageKey) -> Option<&[u8]> {
        let candidates = self.table.candidate_locations(storage_key);
        for &location in candidates.as_slice() {
            let Some(sg_position) = self.mutable_sg_position(location.sg_index) else {
                continue;
            };
            match self.mutable_sgs[sg_position].get(location, storage_key) {
                Some(BucketValue::Value(value)) => return Some(value),
                Some(BucketValue::Tombstone) => return None,
                None => {}
            }
        }
        None
    }

    /// 같은 mutable SG에서 교체하거나, 다른 mutable SG에 추가하고 Table을 갱신한다.
    pub(crate) fn set(
        &mut self,
        storage_key: &StorageKey,
        value: &[u8],
    ) -> Result<(), StorageSetError> {
        let previous = self.find_location(storage_key);
        if let Some(previous) = previous {
            let sg_position = self
                .mutable_sg_position(previous.sg_index)
                .expect("a mutable TableLocation must select one of the three mutable SGs");
            if self.mutable_sgs[sg_position].replace(
                previous,
                storage_key,
                BucketValue::Value(value),
            ) {
                return Ok(());
            }
        }

        let replacement = self
            .append_to_mutable_sg(storage_key, BucketValue::Value(value), previous)
            .ok_or(StorageSetError::MutableSgsFull)?;

        match previous {
            Some(previous) => {
                if !self
                    .table
                    .replace_location(storage_key, previous, replacement)
                {
                    self.rollback_append(replacement, storage_key);
                    return Err(StorageSetError::TableLocationMissing);
                }
                self.remove_from_mutable_sg(previous, storage_key);
            }
            None => {
                if let Err(error) = self.table.insert(storage_key, replacement) {
                    self.rollback_append(replacement, storage_key);
                    return Err(StorageSetError::Table(error));
                }
            }
        }
        Ok(())
    }

    /// mutable SG에 있는 Item을 제거하고 그 Table Entry도 제거한다.
    pub(crate) fn remove(&mut self, storage_key: &StorageKey) -> bool {
        let Some(location) = self.find_location(storage_key) else {
            return false;
        };
        if !self.table.remove(storage_key, location) {
            return false;
        }
        self.remove_from_mutable_sg(location, storage_key)
    }

    fn find_location(&self, storage_key: &StorageKey) -> Option<TableLocation> {
        let candidates = self.table.candidate_locations(storage_key);
        candidates.as_slice().iter().copied().find(|location| {
            self.mutable_sg_position(location.sg_index)
                .is_some_and(|sg_position| {
                    self.mutable_sgs[sg_position]
                        .get(*location, storage_key)
                        .is_some()
                })
        })
    }

    fn append_to_mutable_sg(
        &mut self,
        storage_key: &StorageKey,
        value: BucketValue<'_>,
        previous: Option<TableLocation>,
    ) -> Option<TableLocation> {
        for sg in &mut self.mutable_sgs {
            let Some(location) = sg.append(storage_key, value) else {
                continue;
            };

            let same_physical_bucket = previous.is_some_and(|previous| {
                previous.sg_index == location.sg_index
                    && sg.bucket_index_for_choice(storage_key, previous.bucket_hash_index)
                        == sg.bucket_index_for_choice(storage_key, location.bucket_hash_index)
            });
            if same_physical_bucket {
                let removed = sg.remove(location, storage_key);
                debug_assert!(removed);
                continue;
            }
            return Some(location);
        }
        None
    }

    fn rollback_append(&mut self, location: TableLocation, storage_key: &StorageKey) {
        let removed = self.remove_from_mutable_sg(location, storage_key);
        debug_assert!(removed);
    }

    fn remove_from_mutable_sg(
        &mut self,
        location: TableLocation,
        storage_key: &StorageKey,
    ) -> bool {
        let Some(sg_position) = self.mutable_sg_position(location.sg_index) else {
            return false;
        };
        self.mutable_sgs[sg_position].remove(location, storage_key)
    }

    fn mutable_sg_position(&self, sg_index: u32) -> Option<usize> {
        self.mutable_sgs
            .iter()
            .position(|sg| sg.sg_index() == sg_index)
    }
}

pub(crate) fn run(
    mut request_receiver: Consumer<StorageRequest, STORAGE_QUEUE_SLOTS>,
    mut response_sender: Producer<StorageResponse, STORAGE_QUEUE_SLOTS>,
) -> io::Result<()> {
    let mut storage = Storage::new(
        TableConfig {
            capacity: 625_000,
            target_load_percent: 88,
            fingerprint_bits: 8,
            unary_count: 32,
            front_back_ratio: 8,
            sg_index_bits: 2,
            bucket_choice_count: 4,
            fingerprint_hash_offset_bits: 64,
        },
        MUTABLE_SG_BUCKET_COUNT,
    )
    .map_err(|error| io::Error::other(format!("failed to create storage: {error:?}")))?;

    loop {
        while !response_sender.has_capacity() {
            std::hint::spin_loop();
        }

        let request = loop {
            if let Some(request) = request_receiver.pop() {
                break request;
            }

            std::hint::spin_loop();
        };

        let reply = match request.command {
            Command::Get { key } => {
                let storage_key = StorageKey::from_key(&key);
                Reply::Get(storage.get(&storage_key).map(Arc::from))
            }
            Command::Set { key, value } => {
                let storage_key = StorageKey::from_key(&key);
                storage
                    .set(&storage_key, &value)
                    .map_err(|error| io::Error::other(format!("storage SET failed: {error:?}")))?;
                Reply::SetOk
            }
        };

        let response = StorageResponse {
            client_id: request.client_id,
            reply,
        };

        let Ok(()) = response_sender.push(response) else {
            unreachable!("response queue had capacity and storage is its sole producer");
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table_config() -> TableConfig {
        TableConfig {
            capacity: 1_024,
            target_load_percent: 88,
            fingerprint_bits: 8,
            unary_count: 32,
            front_back_ratio: 8,
            sg_index_bits: 2,
            bucket_choice_count: 4,
            fingerprint_hash_offset_bits: 64,
        }
    }

    #[test]
    fn storage_updates_table_while_mutating_three_sgs() {
        let mut storage = Storage::new(table_config(), 8).unwrap();
        let key = StorageKey::new([7; STORAGE_KEY_BYTES]);

        assert_eq!(storage.mutable_sgs.len(), MUTABLE_SG_COUNT);
        assert_eq!(storage.get(&key), None);
        assert_eq!(storage.set(&key, b"first"), Ok(()));
        assert_eq!(storage.get(&key), Some(b"first".as_slice()));
        assert_eq!(storage.set(&key, b"replacement"), Ok(()));
        assert_eq!(storage.get(&key), Some(b"replacement".as_slice()));
        assert!(storage.remove(&key));
        assert_eq!(storage.get(&key), None);
    }
}
