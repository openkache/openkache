//! TOML configuration for benchmark-tunable storage parameters.
//!
//! Only parameters that are safe to vary at runtime are exposed. Values that are
//! baked into the on-disk format or index encoding (`BUCKET_BYTES`,
//! `STORAGE_KEY_BYTES`, `BUCKET_CHOICE_COUNT`, `MUTABLE_SG_COUNT`) stay fixed in
//! `storage_legacy.rs`. `table_value_bits` is not exposed: it is derived from
//! `storage_sg_count` so the SG index can never overflow the table value.

use std::fs;
use std::io;
use std::path::Path;

use serde::Deserialize;

use crate::storage_legacy::{BUCKET_BYTES, BUCKET_CHOICE_BITS, MUTABLE_SG_COUNT};

/// The largest table value width the index supports (`u32`-backed).
const MAX_TABLE_VALUE_BITS: u32 = 32;

/// One mebibyte, used to convert the human-facing `sg_size_mib` into bytes.
const MIB: u64 = 1024 * 1024;

/// Raw TOML shape. Every field defaults so a partial file (or none) still loads.
#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Config {
    /// Maximum number of live keys the index table can hold.
    pub table_max_entries: usize,
    /// Size of one segment group in mebibytes (must be a multiple of 4 KiB).
    pub sg_size_mib: u64,
    /// Total number of segment groups, i.e. the on-disk capacity in SGs.
    pub storage_sg_count: usize,
    /// Path to the backing storage file.
    pub storage_file_path: String,
    /// io_uring submission-queue capacity for the storage worker.
    pub io_queue_entries: u32,
    /// Reserve physical blocks with `fallocate` instead of a sparse `ftruncate`.
    pub preallocate_file: bool,
}

impl Default for Config {
    fn default() -> Self {
        // Benchmark-oriented defaults, sized for ~100-byte values:
        //   - 16 MiB SGs (4,096 Buckets each) keep flushes cheap for experiments.
        //   - 16 SGs -> 256 MiB backing file; 12 land on SSD after the 3 mutable
        //     plus 1 spare, so a full fill cycle exercises SSD reads.
        //   - The first flush triggers after ~330k inserts (3 mutable SGs), and
        //     the disk fills (SsdCapacityReached) at ~1.97M items — below
        //     table_max_entries, so the table never fills first at >=100-byte
        //     values. Shrink values below ~97 bytes and you must raise this.
        Self {
            table_max_entries: 2_200_000,
            sg_size_mib: 16,
            storage_sg_count: 16,
            storage_file_path: "openkache.data".to_owned(),
            io_queue_entries: 4_096,
            preallocate_file: false,
        }
    }
}

/// Validated configuration with derived fields the storage engine consumes.
#[derive(Debug, Clone)]
pub(crate) struct StorageConfig {
    pub table_max_entries: usize,
    /// Buckets per SG, derived from `sg_size_mib` / `BUCKET_BYTES`.
    pub buckets_per_sg: usize,
    pub storage_sg_count: usize,
    /// Bytes per SG (`buckets_per_sg * BUCKET_BYTES`).
    pub sg_bytes: u64,
    /// Total backing-file size (`storage_sg_count * sg_bytes`).
    pub storage_file_bytes: u64,
    /// Table value width, derived from `storage_sg_count` (never user-set).
    pub table_value_bits: u8,
    pub io_queue_entries: u32,
    pub storage_file_path: String,
    pub preallocate_file: bool,
}

/// Minimum number of bits needed to hold indices `0..count`.
fn index_bits(count: usize) -> u32 {
    // `count - 1` is the largest index; its bit length is the width required.
    // `count == 1` needs a single index (0), which still needs one bit slot.
    match count {
        0 | 1 => 1,
        n => usize::BITS - (n - 1).leading_zeros(),
    }
}

fn invalid(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

impl Config {
    /// Loads a config file, or returns defaults when `path` is `None`.
    pub(crate) fn load(path: Option<&Path>) -> io::Result<StorageConfig> {
        let config = match path {
            Some(path) => {
                let text = fs::read_to_string(path)?;
                toml::from_str::<Self>(&text).map_err(|error| {
                    invalid(format!("invalid config {}: {error}", path.display()))
                })?
            }
            None => Self::default(),
        };
        config.validate()
    }

    fn validate(self) -> io::Result<StorageConfig> {
        if self.table_max_entries == 0 {
            return Err(invalid("table_max_entries must be non-zero".to_owned()));
        }
        if self.sg_size_mib == 0 {
            return Err(invalid("sg_size_mib must be non-zero".to_owned()));
        }
        let sg_bytes = self.sg_size_mib * MIB;
        if sg_bytes % BUCKET_BYTES as u64 != 0 {
            return Err(invalid(format!(
                "sg_size_mib ({} MiB) must be a multiple of the {}-byte Bucket size",
                self.sg_size_mib, BUCKET_BYTES
            )));
        }
        let buckets_per_sg = (sg_bytes / BUCKET_BYTES as u64) as usize;

        // Need at least the mutable working set plus one spare target SG so the
        // first rotate does not immediately hit SsdCapacityReached.
        let min_sg_count = MUTABLE_SG_COUNT + 2;
        if self.storage_sg_count < min_sg_count {
            return Err(invalid(format!(
                "storage_sg_count ({}) must be at least mutable_sg_count + 2 = {}",
                self.storage_sg_count, min_sg_count
            )));
        }

        // Derive the table value width: SG index bits + bucket-choice bits.
        let table_value_bits = index_bits(self.storage_sg_count) + BUCKET_CHOICE_BITS;
        if table_value_bits > MAX_TABLE_VALUE_BITS {
            return Err(invalid(format!(
                "storage_sg_count ({}) needs {table_value_bits} table value bits, exceeding the {MAX_TABLE_VALUE_BITS}-bit limit",
                self.storage_sg_count
            )));
        }
        if self.io_queue_entries == 0 {
            return Err(invalid("io_queue_entries must be non-zero".to_owned()));
        }

        let storage_file_bytes = self.storage_sg_count as u64 * sg_bytes;
        Ok(StorageConfig {
            table_max_entries: self.table_max_entries,
            buckets_per_sg,
            storage_sg_count: self.storage_sg_count,
            sg_bytes,
            storage_file_bytes,
            table_value_bits: table_value_bits as u8,
            io_queue_entries: self.io_queue_entries,
            storage_file_path: self.storage_file_path,
            preallocate_file: self.preallocate_file,
        })
    }
}
