//! kvbench — native multi-protocol GET-throughput load generator.
//!
//! Measures GET-only throughput and latency against a KV/DB server, using each
//! target's own protocol so the comparison isn't skewed by a foreign wire
//! format. Prefill uses a fixed key count; the measured window is warmup +
//! measure seconds. Memory footprint is fixed (keys generated on the fly).

mod backend;
mod keygen;
mod stats;
mod workload;

use std::process::ExitCode;
use std::time::Duration;

use std::future::Future;

use backend::mysql::MyConn;
use backend::postgres::PgConn;
use backend::resp::RespConn;
use backend::{BackendKind, Conn};
use workload::Config;

struct Args {
    backend: BackendKind,
    addr: String,
    keys: u64,
    value_len: usize,
    connections: usize,
    pipeline: usize,
    warmup_ms: u64,
    measure_ms: u64,
    phase: Phase,
    flush_after_prefill: bool,
}

#[derive(PartialEq)]
enum Phase {
    Prefill,
    Measure,
    Both,
}

fn usage() -> String {
    "\
kvbench --backend <resp|postgres|mysql> --addr <host:port> --keys <N> [options]
  --value-len <bytes>     value size for prefill (default 100)
  --connections <N>       concurrent connections (default 50)
  --pipeline <N>          pipelined ops per batch (default 32)
  --warmup-ms <ms>        warmup before measuring (default 1000)
  --measure-ms <ms>       measurement window (default 10000)
  --phase <prefill|measure|both>   (default both)
  --flush-after-prefill   send RESP FLUSH after prefill (OpenKache SSD)"
        .to_string()
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args {
        backend: BackendKind::Resp,
        addr: "127.0.0.1:7711".to_string(),
        keys: 0,
        value_len: 100,
        connections: 50,
        pipeline: 32,
        warmup_ms: 1000,
        measure_ms: 10000,
        phase: Phase::Both,
        flush_after_prefill: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut val = || it.next().ok_or(format!("missing value for {flag}"));
        match flag.as_str() {
            "--backend" => {
                let v = val()?;
                a.backend = BackendKind::parse(&v).ok_or(format!("bad backend: {v}"))?;
            }
            "--addr" => a.addr = val()?,
            "--keys" => a.keys = val()?.parse().map_err(|_| "bad --keys")?,
            "--value-len" => a.value_len = val()?.parse().map_err(|_| "bad --value-len")?,
            "--connections" => a.connections = val()?.parse().map_err(|_| "bad --connections")?,
            "--pipeline" => a.pipeline = val()?.parse().map_err(|_| "bad --pipeline")?,
            "--warmup-ms" => a.warmup_ms = val()?.parse().map_err(|_| "bad --warmup-ms")?,
            "--measure-ms" => a.measure_ms = val()?.parse().map_err(|_| "bad --measure-ms")?,
            "--phase" => {
                a.phase = match val()?.as_str() {
                    "prefill" => Phase::Prefill,
                    "measure" => Phase::Measure,
                    "both" => Phase::Both,
                    other => return Err(format!("bad --phase: {other}")),
                }
            }
            "--flush-after-prefill" => a.flush_after_prefill = true,
            "-h" | "--help" => return Err(usage()),
            other => return Err(format!("unknown flag: {other}\n{}", usage())),
        }
    }
    if a.keys == 0 {
        return Err(format!("--keys is required\n{}", usage()));
    }
    Ok(a)
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("tokio runtime");

    let cfg = Config {
        keys: args.keys,
        value_len: args.value_len,
        connections: args.connections,
        pipeline: args.pipeline,
        warmup: Duration::from_millis(args.warmup_ms),
        measure: Duration::from_millis(args.measure_ms),
    };

    let result = rt.block_on(async { run(&args, &cfg).await });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

async fn run(args: &Args, cfg: &Config) -> std::io::Result<()> {
    // For SQL backends, --addr is parsed as host:port; the db/user/password are
    // fixed (postgres: dbname=kvbench user=kimseojin111 trust; mysql: the
    // provisioned kvbench/kvbench/kvbench identity). Table DDL runs on the first
    // connection only — connections are built sequentially, so no DDL race.
    let addr = args.addr.clone();
    match args.backend {
        BackendKind::Resp => {
            let flush = args.flush_after_prefill;
            let make = move |_c: usize| {
                let addr = addr.clone();
                async move { RespConn::connect(&addr, flush).await }
            };
            run_phases(args, cfg, make).await
        }
        BackendKind::Postgres => {
            let make = move |c: usize| {
                let addr = addr.clone();
                async move { PgConn::connect(&addr, c == 0).await }
            };
            run_phases(args, cfg, make).await
        }
        BackendKind::Mysql => {
            let make = move |c: usize| {
                let addr = addr.clone();
                async move { MyConn::connect(&addr, c == 0).await }
            };
            run_phases(args, cfg, make).await
        }
    }
}

/// Run the prefill and/or measure phases against a backend, given a connection
/// factory. Generic so all backends share the phase orchestration.
async fn run_phases<C, F, Fut>(args: &Args, cfg: &Config, make: F) -> std::io::Result<()>
where
    C: Conn + Send + 'static,
    F: Fn(usize) -> Fut,
    Fut: Future<Output = std::io::Result<C>>,
{
    if args.phase != Phase::Measure {
        eprintln!(
            "prefilling {} keys x {}B over {} connections...",
            cfg.keys, cfg.value_len, cfg.connections
        );
        let t = std::time::Instant::now();
        workload::prefill(cfg, &make).await?;
        eprintln!("prefill done in {:.1}s", t.elapsed().as_secs_f64());
    }
    if args.phase != Phase::Prefill {
        let report = workload::measure(cfg, &make).await?;
        print_report(cfg, &report);
    }
    Ok(())
}

fn print_report(cfg: &Config, r: &workload::Report) {
    let ops = r.hist.total();
    let secs = r.elapsed.as_secs_f64();
    let tput = ops as f64 / secs;
    println!("--- kvbench GET results ---");
    println!("connections   {}", cfg.connections);
    println!("pipeline      {}", cfg.pipeline);
    println!("measured_ops  {ops}");
    println!("elapsed_s     {secs:.3}");
    println!("throughput    {tput:.0} ops/sec");
    println!("hits          {}", r.hits);
    println!("mean_us       {:.1}", r.hist.mean_us());
    println!("p50_us        {}", r.hist.percentile(50.0));
    println!("p99_us        {}", r.hist.percentile(99.0));
    println!("p99.9_us      {}", r.hist.percentile(99.9));
    println!("max_us        {}", r.hist.max_us());
}
