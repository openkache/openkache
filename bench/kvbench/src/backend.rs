//! Backend abstraction: one connection to a target DB, spoken in its own
//! protocol. Each backend implements prefill (SET) and the measured GET.
//!
//! GET-only is the measured path; `set` is used solely during prefill.

use std::io;

pub mod mysql;
pub mod postgres;
pub mod resp;

/// The wire protocol / product to drive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackendKind {
    /// RESP over TCP: OpenKache, Redis.
    Resp,
    /// PostgreSQL wire protocol (tokio-postgres, NoTls).
    Postgres,
    /// MySQL/MariaDB protocol (mysql_async, no TLS).
    Mysql,
}

impl BackendKind {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "resp" | "openkache" | "redis" => Some(Self::Resp),
            "postgres" | "postgresql" | "pg" => Some(Self::Postgres),
            "mysql" | "mariadb" => Some(Self::Mysql),
            _ => None,
        }
    }
}

use std::future::Future;

/// A single connection. Each async task owns one; futures must be `Send` so the
/// multi-thread runtime can drive connections across its 4 worker threads.
pub trait Conn {
    /// Prefill: store `keys.len()` records. `vals` is a flat buffer of
    /// `keys.len() * vlen` bytes; record i uses `vals[i*vlen..(i+1)*vlen]`.
    /// Pipelined / batched per protocol for speed.
    fn set_batch(
        &mut self,
        keys: &[[u8; 32]],
        vals: &[u8],
        vlen: usize,
    ) -> impl Future<Output = io::Result<()>> + Send;

    /// Measured path: issue `keys.len()` GETs (pipelined where the protocol
    /// allows), pushing one latency sample (microseconds) per key into `lat_us`.
    /// Returns the number of hits. Latency for a pipelined op is measured from
    /// batch start to that op's reply — matching how memtier reports it.
    fn get_batch(
        &mut self,
        keys: &[[u8; 32]],
        lat_us: &mut Vec<u64>,
    ) -> impl Future<Output = io::Result<u32>> + Send;

    /// Optional product-specific step run once after prefill (e.g. OpenKache
    /// FLUSH to force data to SSD). Default: no-op.
    fn post_prefill(&mut self) -> impl Future<Output = io::Result<()>> + Send {
        async { Ok(()) }
    }
}
