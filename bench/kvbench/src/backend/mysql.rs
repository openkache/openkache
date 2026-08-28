//! MySQL/MariaDB backend via `mysql_async` (no TLS; pure-Rust flate2).
//!
//! One `mysql_async::Conn` per connection. Keys are CHAR(32), values
//! VARBINARY(100). Prefill uses a prepared multi-row batch insert
//! (`exec_batch`, INSERT IGNORE). The measured GET path is sequential prepared
//! `SELECT v FROM kv WHERE k = ?` — the MySQL protocol has no client-side
//! pipelining, so this reflects its real request/response behavior.
//!
//! Connection identity is fixed to the provisioned values (see
//! provision/mysql/env.sh): db `kvbench`, user `kvbench`, pass `kvbench`. Host
//! and port come from `--addr` (default 127.0.0.1:33061).

use std::io;
use std::time::Instant;

use mysql_async::prelude::*;
use mysql_async::{Conn as MyClient, OptsBuilder, Row, Statement};

use super::Conn;

pub struct MyConn {
    client: MyClient,
    set_stmt: Statement,
    get_stmt: Statement,
}

fn err<E: std::error::Error + Send + Sync + 'static>(e: E) -> io::Error {
    io::Error::new(io::ErrorKind::Other, e)
}

/// Parse `host:port`; missing/invalid port falls back to `default_port`.
fn split_host_port(addr: &str, default_port: u16) -> (String, u16) {
    match addr.rsplit_once(':') {
        Some((h, p)) => (h.to_string(), p.parse().unwrap_or(default_port)),
        None => (addr.to_string(), default_port),
    }
}

impl MyConn {
    /// Connect to `host:port` using the provisioned kvbench/kvbench/kvbench
    /// identity. `first` connection creates the table.
    pub async fn connect(addr: &str, first: bool) -> io::Result<Self> {
        let (host, port) = split_host_port(addr, 33061);
        let opts = OptsBuilder::default()
            .ip_or_hostname(host)
            .tcp_port(port)
            .user(Some("kvbench"))
            .pass(Some("kvbench"))
            .db_name(Some("kvbench"));
        let mut client = MyClient::new(opts).await.map_err(err)?;

        if first {
            client
                .query_drop(
                    "CREATE TABLE IF NOT EXISTS kv \
                     (k CHAR(32) PRIMARY KEY, v VARBINARY(100))",
                )
                .await
                .map_err(err)?;
        }

        let set_stmt = client
            .prep("INSERT IGNORE INTO kv (k, v) VALUES (?, ?)")
            .await
            .map_err(err)?;
        let get_stmt = client
            .prep("SELECT v FROM kv WHERE k = ?")
            .await
            .map_err(err)?;

        Ok(Self {
            client,
            set_stmt,
            get_stmt,
        })
    }
}

impl Conn for MyConn {
    async fn set_batch(&mut self, keys: &[[u8; 32]], vals: &[u8], vlen: usize) -> io::Result<()> {
        // Batched prepared insert: one prepare, N executions server-side.
        // Params own their bytes; the closure yields (key, value) per row.
        let stmt = self.set_stmt.clone();
        let params = keys.iter().enumerate().map(|(i, key)| {
            let k = std::str::from_utf8(key).expect("keys are ASCII");
            let v = &vals[i * vlen..(i + 1) * vlen];
            (k, v)
        });
        self.client.exec_batch(stmt, params).await.map_err(err)?;
        Ok(())
    }

    async fn get_batch(&mut self, keys: &[[u8; 32]], lat_us: &mut Vec<u64>) -> io::Result<u32> {
        // MySQL has no pipelining: issue prepared SELECTs sequentially,
        // recording each op's latency from batch start.
        let stmt = self.get_stmt.clone();
        let start = Instant::now();
        let mut hits = 0u32;
        for key in keys {
            let k = std::str::from_utf8(key)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            let row: Option<Row> = self
                .client
                .exec_first(&stmt, (k,))
                .await
                .map_err(err)?;
            if row.is_some() {
                hits += 1;
            }
            lat_us.push(start.elapsed().as_micros() as u64);
        }
        Ok(hits)
    }
}
