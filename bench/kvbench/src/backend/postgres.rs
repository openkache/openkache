//! PostgreSQL backend via `tokio-postgres` (NoTls).
//!
//! One `Client` per connection; the `Connection` driver future is spawned onto
//! the tokio runtime. Keys are stored as TEXT (`k TEXT COLLATE "C"`), values as
//! BYTEA. Prepared statements are built once at connect and reused (no per-op
//! allocation of SQL). Prefill inserts and measured GETs are both pipelined:
//! tokio-postgres allows many concurrent in-flight queries on a single
//! connection, so we launch all N ops of a batch and drain them as they land.

use std::io;
use std::time::Instant;

use futures_util::stream::{FuturesUnordered, StreamExt};
use tokio_postgres::{Client, NoTls, Statement};

use super::Conn;

pub struct PgConn {
    client: Client,
    set_stmt: Statement,
    get_stmt: Statement,
}

fn err<E: std::error::Error + Send + Sync + 'static>(e: E) -> io::Error {
    io::Error::new(io::ErrorKind::Other, e)
}

impl PgConn {
    /// Connect to `host:port`, fixed dbname `kvbench` / user `kimseojin111`, no
    /// password (trust auth). `first` connection creates the table (DDL is not
    /// run concurrently — prefill/measure build connections sequentially).
    pub async fn connect(addr: &str, first: bool) -> io::Result<Self> {
        let (host, port) = split_host_port(addr, 55432);
        let conn_str = format!(
            "host={host} port={port} dbname=kvbench user=kimseojin111 \
             connect_timeout=10 application_name=kvbench",
        );
        let (client, connection) = tokio_postgres::connect(&conn_str, NoTls)
            .await
            .map_err(err)?;
        // Drive the connection: it must be polled for the client to make progress.
        tokio::spawn(async move {
            let _ = connection.await;
        });

        if first {
            client
                .batch_execute(
                    "CREATE TABLE IF NOT EXISTS kv \
                     (k TEXT COLLATE \"C\" PRIMARY KEY, v BYTEA NOT NULL)",
                )
                .await
                .map_err(err)?;
        }

        let set_stmt = client
            .prepare("INSERT INTO kv (k, v) VALUES ($1, $2) ON CONFLICT DO NOTHING")
            .await
            .map_err(err)?;
        let get_stmt = client
            .prepare("SELECT v FROM kv WHERE k = $1")
            .await
            .map_err(err)?;

        Ok(Self {
            client,
            set_stmt,
            get_stmt,
        })
    }
}

/// Parse `host:port`; missing/invalid port falls back to `default_port`.
fn split_host_port(addr: &str, default_port: u16) -> (String, u16) {
    match addr.rsplit_once(':') {
        Some((h, p)) => (h.to_string(), p.parse().unwrap_or(default_port)),
        None => (addr.to_string(), default_port),
    }
}

/// One pipelined INSERT. Borrows client/stmt/key; the returned future is Send.
async fn one_set(
    client: &Client,
    stmt: &Statement,
    key: &str,
    val: &[u8],
) -> io::Result<()> {
    client.execute(stmt, &[&key, &val]).await.map_err(err)?;
    Ok(())
}

/// One pipelined SELECT; returns true if the row exists (hit).
async fn one_get(client: &Client, stmt: &Statement, key: &str) -> io::Result<bool> {
    let rows = client.query(stmt, &[&key]).await.map_err(err)?;
    Ok(!rows.is_empty())
}

impl Conn for PgConn {
    async fn set_batch(&mut self, keys: &[[u8; 32]], vals: &[u8], vlen: usize) -> io::Result<()> {
        // Launch all inserts concurrently on the one connection (pipelined),
        // then drain. Keys are ASCII (prefix + digits) so from_utf8 is safe.
        let futs = FuturesUnordered::new();
        for (i, key) in keys.iter().enumerate() {
            let k = std::str::from_utf8(key)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            let v = &vals[i * vlen..(i + 1) * vlen];
            futs.push(one_set(&self.client, &self.set_stmt, k, v));
        }
        let mut futs = futs;
        while let Some(res) = futs.next().await {
            res?;
        }
        Ok(())
    }

    async fn get_batch(&mut self, keys: &[[u8; 32]], lat_us: &mut Vec<u64>) -> io::Result<u32> {
        // Pipeline all GETs; record each op's latency from batch start as it
        // completes (order-independent, matching memtier-style per-op latency).
        let futs = FuturesUnordered::new();
        for key in keys {
            let k = std::str::from_utf8(key)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
            futs.push(one_get(&self.client, &self.get_stmt, k));
        }
        let start = Instant::now();
        let mut futs = futs;
        let mut hits = 0u32;
        while let Some(res) = futs.next().await {
            if res? {
                hits += 1;
            }
            lat_us.push(start.elapsed().as_micros() as u64);
        }
        Ok(hits)
    }
}
