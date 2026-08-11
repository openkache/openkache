//! Benchmark request vocabulary and aggregate statistics.
//!
//! Benchmark orchestration stays on the runtime facade, while these public
//! values remain independent from the worker scheduler and its command types.

use crate::protocol::ItemId;

#[derive(Debug)]
pub enum BenchmarkOperation {
    Get(ItemId),
    Set(ItemId, Vec<u8>),
    Delete(ItemId),
}

impl BenchmarkOperation {
    pub(crate) fn item_id(&self) -> ItemId {
        match self {
            Self::Get(item_id) | Self::Delete(item_id) | Self::Set(item_id, _) => *item_id,
        }
    }
}

#[derive(Debug, Default)]
pub struct BenchmarkBatchStats {
    pub operations: usize,
    pub gets: usize,
    pub hits: usize,
    pub sets: usize,
    pub creates: usize,
    pub replaces: usize,
    pub deletes: usize,
    pub deleted: usize,
    pub latency_ns: Vec<u64>,
}

impl BenchmarkBatchStats {
    pub fn merge(&mut self, mut other: Self) {
        self.operations += other.operations;
        self.gets += other.gets;
        self.hits += other.hits;
        self.sets += other.sets;
        self.creates += other.creates;
        self.replaces += other.replaces;
        self.deletes += other.deletes;
        self.deleted += other.deleted;
        self.latency_ns.append(&mut other.latency_ns);
    }
}
