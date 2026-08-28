//! RESP (Redis serialization protocol) over TCP: OpenKache, Redis.
//!
//! Hand-rolled, minimal: only the SET / GET / FLUSH replies we issue. One
//! `TcpStream` per connection with a reusable read buffer; no per-op allocation.

use std::io;
use std::time::Instant;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use super::Conn;

pub struct RespConn {
    stream: TcpStream,
    /// Reusable scratch for building request frames.
    out: Vec<u8>,
    /// Reusable read buffer for replies.
    inbuf: Vec<u8>,
    /// Whether to send a FLUSH after prefill (OpenKache SSD residency).
    flush_after_prefill: bool,
}

impl RespConn {
    pub async fn connect(addr: &str, flush_after_prefill: bool) -> io::Result<Self> {
        let stream = TcpStream::connect(addr).await?;
        stream.set_nodelay(true)?;
        Ok(Self {
            stream,
            out: Vec::with_capacity(256),
            inbuf: vec![0u8; 8192],
            flush_after_prefill,
        })
    }

    /// Reads one RESP reply, returning (found, value_len_if_bulk).
    /// Handles: `+OK`, `$-1` (nil), `$<len>\r\n<data>\r\n`, `-ERR ...`, `:<n>`.
    async fn read_reply(&mut self) -> io::Result<ReplyKind> {
        let first = self.read_line().await?;
        match first.first() {
            Some(b'+') | Some(b':') => Ok(ReplyKind::Simple),
            Some(b'-') => {
                let msg = String::from_utf8_lossy(&first[1..]).into_owned();
                Err(io::Error::new(io::ErrorKind::Other, msg))
            }
            Some(b'$') => {
                let len: i64 = parse_int(&first[1..]);
                if len < 0 {
                    return Ok(ReplyKind::Nil);
                }
                // Consume <len> bytes + trailing CRLF.
                self.consume_exact(len as usize + 2).await?;
                Ok(ReplyKind::Bulk(len as usize))
            }
            other => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unexpected RESP reply byte: {other:?}"),
            )),
        }
    }

    /// Reads up to and including CRLF, returning the line without CRLF.
    async fn read_line(&mut self) -> io::Result<Vec<u8>> {
        let mut line = Vec::with_capacity(32);
        loop {
            let mut byte = [0u8; 1];
            let n = self.stream.read(&mut byte).await?;
            if n == 0 {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            if byte[0] == b'\r' {
                // Expect the \n.
                let mut lf = [0u8; 1];
                self.stream.read_exact(&mut lf).await?;
                break;
            }
            line.push(byte[0]);
        }
        Ok(line)
    }

    async fn consume_exact(&mut self, n: usize) -> io::Result<()> {
        if self.inbuf.len() < n {
            self.inbuf.resize(n, 0);
        }
        self.stream.read_exact(&mut self.inbuf[..n]).await?;
        Ok(())
    }
}

enum ReplyKind {
    Simple,
    Nil,
    Bulk(usize),
}

fn parse_int(bytes: &[u8]) -> i64 {
    let mut neg = false;
    let mut val: i64 = 0;
    for &b in bytes {
        if b == b'-' {
            neg = true;
        } else if b.is_ascii_digit() {
            val = val * 10 + (b - b'0') as i64;
        }
    }
    if neg { -val } else { val }
}

/// Appends a RESP array of bulk-string arguments to `out`.
fn encode(out: &mut Vec<u8>, args: &[&[u8]]) {
    out.clear();
    out.extend_from_slice(format!("*{}\r\n", args.len()).as_bytes());
    for arg in args {
        out.extend_from_slice(format!("${}\r\n", arg.len()).as_bytes());
        out.extend_from_slice(arg);
        out.extend_from_slice(b"\r\n");
    }
}

impl Conn for RespConn {
    async fn set_batch(&mut self, keys: &[[u8; 32]], vals: &[u8], vlen: usize) -> io::Result<()> {
        self.out.clear();
        let vlen_hdr = format!("${vlen}\r\n");
        for (i, key) in keys.iter().enumerate() {
            self.out.extend_from_slice(b"*3\r\n$3\r\nSET\r\n$32\r\n");
            self.out.extend_from_slice(key);
            self.out.extend_from_slice(b"\r\n");
            self.out.extend_from_slice(vlen_hdr.as_bytes());
            self.out.extend_from_slice(&vals[i * vlen..(i + 1) * vlen]);
            self.out.extend_from_slice(b"\r\n");
        }
        self.stream.write_all(&self.out).await?;
        for _ in 0..keys.len() {
            self.read_reply().await?;
        }
        Ok(())
    }

    async fn get_batch(&mut self, keys: &[[u8; 32]], lat_us: &mut Vec<u64>) -> io::Result<u32> {
        // Pipeline: write all GET requests, then read all replies.
        self.out.clear();
        for key in keys {
            self.out.extend_from_slice(b"*2\r\n$3\r\nGET\r\n$32\r\n");
            self.out.extend_from_slice(key);
            self.out.extend_from_slice(b"\r\n");
        }
        let start = Instant::now();
        self.stream.write_all(&self.out).await?;
        let mut hits = 0u32;
        for _ in 0..keys.len() {
            match self.read_reply().await? {
                ReplyKind::Bulk(_) | ReplyKind::Simple => hits += 1,
                ReplyKind::Nil => {}
            }
            lat_us.push(start.elapsed().as_micros() as u64);
        }
        Ok(hits)
    }

    async fn post_prefill(&mut self) -> io::Result<()> {
        if !self.flush_after_prefill {
            return Ok(());
        }
        // Drain all mutable SGs to SSD. Each FLUSH rotates one SG; loop until it
        // reports the capacity/nothing state or a bounded number of rounds.
        for _ in 0..512 {
            encode(&mut self.out, &[b"FLUSH"]);
            self.stream.write_all(&self.out).await?;
            match self.read_reply().await {
                Ok(_) => continue,
                // An -ERR (e.g. "SSD capacity reached" / nothing to flush) ends it.
                Err(_) => break,
            }
        }
        Ok(())
    }
}
