//! Latency histogram and throughput aggregation.
//!
//! Two-tier microsecond buckets so a single scheme covers sub-millisecond SSD
//! reads and the tens-of-milliseconds queueing latency seen under deep
//! pipelining, without a huge array:
//!   - fine:   [0, 8192) us at 1 us resolution
//!   - coarse: [8192, ~2 s) at 128 us resolution
//! Each connection owns one; the driver merges after the measurement window.

const FINE_LIMIT_US: u64 = 8_192;
const COARSE_STEP_US: u64 = 128;
const COARSE_MAX_US: u64 = 2_000_000;
const COARSE_BUCKETS: usize = ((COARSE_MAX_US - FINE_LIMIT_US) / COARSE_STEP_US) as usize;
const NUM_BUCKETS: usize = FINE_LIMIT_US as usize + COARSE_BUCKETS + 1;

#[derive(Clone)]
pub struct Histogram {
    counts: Vec<u64>,
    total: u64,
    sum_us: u128,
    max_us: u64,
}

#[inline]
fn bucket_of(us: u64) -> usize {
    if us < FINE_LIMIT_US {
        us as usize
    } else {
        let c = ((us - FINE_LIMIT_US) / COARSE_STEP_US) as usize;
        (FINE_LIMIT_US as usize + c).min(NUM_BUCKETS - 1)
    }
}

/// Representative microsecond value for a bucket index (its lower edge).
#[inline]
fn us_of(bucket: usize) -> u64 {
    if (bucket as u64) < FINE_LIMIT_US {
        bucket as u64
    } else {
        FINE_LIMIT_US + (bucket as u64 - FINE_LIMIT_US) * COARSE_STEP_US
    }
}

impl Histogram {
    pub fn new() -> Self {
        Self {
            counts: vec![0; NUM_BUCKETS],
            total: 0,
            sum_us: 0,
            max_us: 0,
        }
    }

    #[inline]
    pub fn record_us(&mut self, us: u64) {
        self.counts[bucket_of(us)] += 1;
        self.total += 1;
        self.sum_us += us as u128;
        if us > self.max_us {
            self.max_us = us;
        }
    }

    pub fn merge(&mut self, other: &Histogram) {
        for (slot, add) in self.counts.iter_mut().zip(&other.counts) {
            *slot += add;
        }
        self.total += other.total;
        self.sum_us += other.sum_us;
        self.max_us = self.max_us.max(other.max_us);
    }

    pub fn total(&self) -> u64 {
        self.total
    }

    pub fn mean_us(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            self.sum_us as f64 / self.total as f64
        }
    }

    pub fn max_us(&self) -> u64 {
        self.max_us
    }

    /// Microsecond value at the given percentile (0..100).
    pub fn percentile(&self, p: f64) -> u64 {
        if self.total == 0 {
            return 0;
        }
        let target = (self.total as f64 * p / 100.0).ceil() as u64;
        let mut cumulative = 0u64;
        for (bucket, &count) in self.counts.iter().enumerate() {
            cumulative += count;
            if cumulative >= target {
                return us_of(bucket);
            }
        }
        us_of(NUM_BUCKETS - 1)
    }
}
