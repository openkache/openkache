use std::sync::Arc;

pub(crate) const STORAGE_QUEUE_SLOTS: usize = 4096;
pub(crate) const STORAGE_KEY_BYTES: usize = 32;

/// Fixed-size key passed from a network worker to a storage shard.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct StorageKey([u8; STORAGE_KEY_BYTES]);

impl StorageKey {
    /// This conversion runs on the network worker before the request enters an
    /// SPSC queue.
    pub(crate) fn from_client_key(key: &[u8]) -> Self {
        Self(*blake3::hash(key).as_bytes())
    }

    pub(crate) const fn as_bytes(&self) -> &[u8; STORAGE_KEY_BYTES] {
        &self.0
    }

    pub(crate) fn table_hash(&self) -> u128 {
        u128::from_le_bytes(self.0[8..24].try_into().unwrap())
    }
}

#[derive(Clone, Copy)]
pub(crate) struct ClientId(pub(crate) usize);

pub(crate) struct StorageRequest {
    pub client_id: ClientId,
    pub sequence: u64,
    pub command: Command,
}

pub(crate) enum Command {
    Get {
        key: StorageKey,
    },
    Set {
        key: StorageKey,
        value: Arc<[u8]>,
    },
    Delete {
        key: StorageKey,
    },
    /// Benchmark-only: force every current Mutable SG to flush to SSD now.
    Flush,
}

pub(crate) struct StorageResponse {
    pub client_id: ClientId,
    pub sequence: u64,
    pub reply: Reply,
}

pub(crate) enum Reply {
    Get(Option<Arc<[u8]>>),
    SetOk,
    Delete(bool),
    /// `Ok` when every current Mutable SG was flushed; `Err` carries a human-readable reason
    /// (e.g. SSD capacity reached, a flush already in flight).
    Flush(Result<(), &'static str>),
}
