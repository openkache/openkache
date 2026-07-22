//! Standalone benchmark for the reusable breadcrumb-filter library module.

use std::time::{Duration, Instant};

use openkache::breadcrumb::BreadcrumbFilter;
use openkache::{HashedKey, Key};

/// Runs insertion and positive/negative membership-query benchmarks.
fn main() {
    let count = benchmark_item_count();
    let mut filter = BreadcrumbFilter::with_capacity(count);
    let keys: Vec<HashedKey> = (0..count as u64).map(hashed_key).collect();
    let negative_keys: Vec<HashedKey> = (count as u64..count as u64 * 2).map(hashed_key).collect();

    println!(
        "BCF53: items={count}, memory={} bytes, backend={}",
        filter.memory_bytes(),
        filter.simd_backend()
    );

    let started = Instant::now();
    for key in &keys {
        filter.insert(key).expect("filter reached capacity");
    }
    let insert_time = started.elapsed();

    let started = Instant::now();
    let positives = keys.iter().filter(|key| filter.contains(key)).count();
    let positive_time = started.elapsed();

    let started = Instant::now();
    let false_positives = negative_keys
        .iter()
        .filter(|key| filter.contains(key))
        .count();
    let negative_time = started.elapsed();

    println!(
        "insert={:.2} Mops/s, positive={:.2} Mops/s, negative={:.2} Mops/s",
        mops(count, insert_time),
        mops(count, positive_time),
        mops(count, negative_time),
    );
    println!(
        "positive hits={positives}/{count}, false-positive rate={:.4}%",
        false_positives as f64 * 100.0 / count as f64
    );
}

/// Parses the optional number of benchmark items from the first CLI argument.
fn benchmark_item_count() -> usize {
    std::env::args()
        .nth(1)
        .map(|arg| arg.parse::<usize>().expect("item count must be an integer"))
        .unwrap_or(1_000_000)
}

/// Converts an operation count and elapsed time into millions of operations per second.
fn mops(operations: usize, elapsed: Duration) -> f64 {
    operations as f64 / elapsed.as_secs_f64() / 1_000_000.0
}

/// Hashes a deterministic numeric source key before the measured filter operations.
fn hashed_key(value: u64) -> HashedKey {
    Key::new(value.to_le_bytes().to_vec()).hashed_key()
}
