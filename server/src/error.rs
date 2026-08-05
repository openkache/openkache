//! Error types for the KV cache. Defines [`KvError`] (a `thiserror`-derived enum covering
//! I/O, config, Table-full, timeout, worker, and usage errors) and a
//! [`Result`] type alias.

use std::io;

use thiserror::Error;

pub type Result<T> = std::result::Result<T, KvError>;

#[derive(Debug, Error)]
pub enum KvError {
    #[error("{0}")]
    Io(#[from] io::Error),
    #[error("invalid configuration: {0}")]
    InvalidConfig(String),
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    #[error("lookup Table is full")]
    TableFull,
    #[error("Item requires {bytes} bytes but one empty Bucket has {capacity} bytes")]
    ItemTooLarge { bytes: usize, capacity: usize },
    #[error(
        "Blob Item requires {required_bytes} bytes but the Blob Segment has {remaining_bytes} bytes remaining"
    )]
    BlobSegmentFull {
        required_bytes: u64,
        remaining_bytes: u64,
    },
    #[error("{resource} capacity is exhausted; writes are temporarily stopped")]
    CapacityExhausted { resource: &'static str },
    #[error("write cannot be admitted without evicting protected items")]
    NoCapacity,
    #[error("{0} timed out")]
    Timeout(&'static str),
    #[error("worker error: {0}")]
    Worker(String),
    #[error("{0}")]
    Usage(String),
}
