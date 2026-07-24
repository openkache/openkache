//! Error types for the KV cache. Defines [`KvError`] (a `thiserror`-derived enum covering
//! I/O, config, corruption, index-full, timeout, worker, and usage errors) and a
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
    #[error("corrupt KVKACHE data: {0}")]
    Corrupt(String),
    #[error("global Breadcrumb location index is full")]
    IndexFull,
    #[error("record requires {bytes} bytes but one empty page has {capacity} bytes")]
    RecordTooLarge { bytes: usize, capacity: usize },
    #[error(
        "blob item requires {required_bytes} bytes but the Segment has {remaining_bytes} bytes remaining"
    )]
    BlobSegmentFull {
        required_bytes: u64,
        remaining_bytes: u64,
    },
    #[error("{0} timed out")]
    Timeout(&'static str),
    #[error("worker error: {0}")]
    Worker(String),
    #[error("{0}")]
    Usage(String),
}
