//! Prefill and measurement orchestration across N concurrent connections.

use std::io;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::backend::Conn;
use crate::keygen::{write_key, write_value, Rng, KEY_LEN};
use crate::stats::Histogram;

#[derive(Clone)]
pub struct Config {
    pub keys: u64,
    pub value_len: usize,
    pub connections: usize,
    pub pipeline: usize,
    pub warmup: Duration,
    pub measure: Duration,
}

/// Result of a measurement run.
pub struct Report {
    pub hist: Histogram,
    pub hits: u64,
    pub elapsed: Duration,
}

/// Prefill all `cfg.keys` records, split evenly across `conns` connections,
/// then run each connection's `post_prefill`. Connections are provided by a
/// factory so the caller controls backend construction.
pub async fn prefill<C, F, Fut>(cfg: &Config, make_conn: F) -> io::Result<()>
where
    C: Conn + Send + 'static,
    F: Fn(usize) -> Fut,
    Fut: std::future::Future<Output = io::Result<C>>,
{
    let batch = 256usize;
    let per_conn = cfg.keys.div_ceil(cfg.connections as u64);
    let mut tasks = Vec::new();
    for c in 0..cfg.connections {
        let start = c as u64 * per_conn;
        let end = (start + per_conn).min(cfg.keys);
        let conn = make_conn(c).await?;
        let vlen = cfg.value_len;
        tasks.push(tokio::spawn(async move {
            let mut conn = conn;
            let mut keybuf = vec![[0u8; KEY_LEN]; batch];
            let mut valbuf = vec![0u8; batch * vlen];
            let mut idx = start;
            while idx < end {
                let n = ((end - idx) as usize).min(batch);
                for j in 0..n {
                    write_key(idx + j as u64, &mut keybuf[j]);
                    write_value(idx + j as u64, &mut valbuf[j * vlen..(j + 1) * vlen]);
                }
                conn.set_batch(&keybuf[..n], &valbuf[..n * vlen], vlen).await?;
                idx += n as u64;
            }
            conn.post_prefill().await?;
            io::Result::Ok(())
        }));
    }
    for t in tasks {
        t.await.expect("prefill task panicked")?;
    }
    Ok(())
}

/// GET-only measurement. Each connection loops issuing pipelined batches of
/// random GETs until the measurement window ends; samples are only recorded
/// after warmup.
pub async fn measure<C, F, Fut>(cfg: &Config, make_conn: F) -> io::Result<Report>
where
    C: Conn + Send + 'static,
    F: Fn(usize) -> Fut,
    Fut: std::future::Future<Output = io::Result<C>>,
{
    let start_flag = Arc::new(AtomicBool::new(false));
    let stop_flag = Arc::new(AtomicBool::new(false));
    let record_flag = Arc::new(AtomicBool::new(false));
    let total_hits = Arc::new(AtomicU64::new(0));

    let mut tasks = Vec::new();
    for c in 0..cfg.connections {
        let conn = make_conn(c).await?;
        let keys = cfg.keys;
        let pipeline = cfg.pipeline;
        let start_flag = start_flag.clone();
        let stop_flag = stop_flag.clone();
        let record_flag = record_flag.clone();
        let total_hits = total_hits.clone();
        tasks.push(tokio::spawn(async move {
            let mut conn = conn;
            let mut rng = Rng::new(0xABCD_0000 ^ (c as u64 + 1));
            let mut hist = Histogram::new();
            let mut keybuf = vec![[0u8; KEY_LEN]; pipeline];
            let mut lat = Vec::with_capacity(pipeline);
            let mut hits = 0u64;

            while !start_flag.load(Ordering::Relaxed) {
                tokio::task::yield_now().await;
            }
            while !stop_flag.load(Ordering::Relaxed) {
                for k in keybuf.iter_mut() {
                    write_key(rng.index(keys), k);
                }
                lat.clear();
                let batch_hits = conn.get_batch(&keybuf, &mut lat).await?;
                if record_flag.load(Ordering::Relaxed) {
                    for &us in &lat {
                        hist.record_us(us);
                    }
                    hits += batch_hits as u64;
                }
            }
            total_hits.fetch_add(hits, Ordering::Relaxed);
            io::Result::Ok(hist)
        }));
    }

    // Timeline: start -> warmup -> record window -> stop.
    start_flag.store(true, Ordering::Relaxed);
    tokio::time::sleep(cfg.warmup).await;
    let measure_start = Instant::now();
    record_flag.store(true, Ordering::Relaxed);
    tokio::time::sleep(cfg.measure).await;
    record_flag.store(false, Ordering::Relaxed);
    let elapsed = measure_start.elapsed();
    stop_flag.store(true, Ordering::Relaxed);

    let mut hist = Histogram::new();
    for t in tasks {
        let h = t.await.expect("measure task panicked")?;
        hist.merge(&h);
    }
    Ok(Report {
        hist,
        hits: total_hits.load(Ordering::Relaxed),
        elapsed,
    })
}
