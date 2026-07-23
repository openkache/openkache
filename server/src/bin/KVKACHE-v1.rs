//! KVKACHE v1: a persistent NVMe-backed KV cache with a RAM location filter.
//!
//! The logical global index is partitioned by key across thread-per-core
//! workers. Each worker exclusively owns a Breadcrumb-style
//! front-yard/backyard filter, its independent io_uring, and its circular SG
//! queue, so the hot path has no shared mutable index. Unlike an ordinary
//! approximate-membership filter, every fingerprint carries a compact physical
//! location: an SG region and the one-bit page-hash choice. A single mutable SG
//! per worker balances every record between two candidate pages.
//!
//! Public semantics intentionally follow common cache APIs:
//! - `get` returns the current value or `None`.
//! - `set` is an in-memory-acknowledged upsert that compacts mutable records.
//! - `delete` is acknowledged after appending an in-memory tombstone.
//!
//! A mutable SG stays open across request batches. It is persisted only when
//! page placement fails because it is full, or when `sync`/graceful shutdown
//! explicitly flushes it. Reopening the same paths restores the checkpoint and
//! validates every SG generation. If no checkpoint exists, the data SGs are
//! scanned to build an initial index.

use std::cell::Cell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::env;
use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::time::Duration;

use compio::BufResult;
use compio::driver::ProactorBuilder;
use compio::fs::{File, OpenOptions};
use compio::io::{AsyncReadAtExt, AsyncWriteAtExt};
use compio::runtime::RuntimeBuilder;
use futures_util::stream::{FuturesUnordered, StreamExt};
use openkache::Key;
use serde::Deserialize;

const BUCKET_BYTES: usize = 64;
const PAGE_MAGIC: u32 = 0x4b56_5031; // "KVP1"
const PAGE_VERSION: u16 = 1;
const PAGE_HEADER: usize = 32;
const RECORD_HEADER: usize = 16;
const RECORD_SET: u8 = 1;
const RECORD_DELETE: u8 = 2;
const CHECKPOINT_MAGIC: &[u8; 8] = b"KVKIDX01";
const CHECKPOINT_VERSION: u32 = 1;
const NONE_GENERATION: u64 = u64::MAX;

type Result<T> = std::result::Result<T, KvError>;

#[derive(Debug)]
pub(crate) enum KvError {
    Io(io::Error),
    InvalidConfig(String),
    Corrupt(String),
    IndexFull,
    RecordTooLarge { bytes: usize, capacity: usize },
    Timeout(&'static str),
    Worker(String),
    Usage(String),
}

impl fmt::Display for KvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "{error}"),
            Self::InvalidConfig(message) => write!(f, "invalid configuration: {message}"),
            Self::Corrupt(message) => write!(f, "corrupt KVKACHE data: {message}"),
            Self::IndexFull => f.write_str("global Breadcrumb location index is full"),
            Self::RecordTooLarge { bytes, capacity } => write!(
                f,
                "record requires {bytes} bytes but one empty page has {capacity} bytes"
            ),
            Self::Timeout(operation) => write!(f, "{operation} timed out"),
            Self::Worker(message) => write!(f, "worker error: {message}"),
            Self::Usage(message) => f.write_str(message),
        }
    }
}

impl Error for KvError {}

impl From<io::Error> for KvError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

include!("kvkache_v1/config.rs");
include!("kvkache_v1/cli.rs");
include!("kvkache_v1/packed_bucket.rs");
include!("kvkache_v1/breadcrumb.rs");
include!("kvkache_v1/page.rs");
include!("kvkache_v1/engine.rs");
include!("kvkache_v1/persistence.rs");
include!("kvkache_v1/worker.rs");
include!("kvkache_v1/threaded.rs");
include!("kvkache_v1/codec.rs");
