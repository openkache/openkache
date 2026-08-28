# OpenKache Benchmark

## Throughput (GET)

| System | GET throughput | Load tool |
|---|---:|---|
| OpenKache | 97,887 ops/s (1×) | kvbench (RESP) |
| PostgreSQL 17.10 | 17,421 ops/s (0.18×) | kvbench (PostgreSQL wire) |
| MySQL 8.4.11 | 16,295 ops/s (0.17×) | kvbench (MySQL wire) |

OpenKache is 5.6× faster than PostgreSQL and 6.0× faster than MySQL. OpenKache
reaches 76% of the hardware limit (128,820 IOPS, measured with fio).

## Latency (GET, single request at a time)

| System | avg | p50 | p99 | p99.9 |
|---|---:|---:|---:|---:|
| OpenKache | 238.7 µs (1×) | 229 µs (1×) | 386 µs (1×) | 1376 µs (1×) |
| MySQL 8.4.11 | 385.7 µs (1.6×) | 410 µs (1.8×) | 1169 µs (3.0×) | 2207 µs (1.6×) |
| PostgreSQL 17.10 | 558.0 µs (2.3×) | 510 µs (2.2×) | 1263 µs (3.3×) | 3342 µs (2.4×) |

## Test Environment

serveroptima1:

- CPU: AMD EPYC 7773X, 6 vCPU
- RAM: 19.5 GiB
- Storage: /dev/sda1, ext4, SSD
- Kernel: 6.8
- Date: 2026-08-26

## Workload

- Record: 32-byte key + 100-byte random value
- GET-only point lookups over the prefilled key range
- Throughput: many concurrent pipelined requests, driven to saturation
- Latency: one connection issuing a single request at a time (no pipelining,
  no concurrency), measuring the end-to-end round trip of one query

## Measurement Method

### Throughput

All three systems were driven by kvbench speaking each system's native
protocol: RESP for OpenKache, the PostgreSQL wire protocol, and the MySQL wire
protocol. In every run the database was pinned to CPU cores 0–1 and the load
generator to cores 2–5.

For PostgreSQL and MySQL, parameters were swept to find the configuration with
the highest throughput.

PostgreSQL: swept shared_buffers over 128 / 256 / 512 MB and client count over
16 / 24 / 32 / 48, selecting shared_buffers 256 MB with 24 clients. Jit and
autovacuum were disabled, and ANALYZE was run right after prefill.

MySQL: swept innodb_buffer_pool_size over 128 / 256 / 512 MB and threads over
8 / 16 / 32 / 64, selecting a 512 MB buffer pool with 16 threads.

### Latency

All three systems were driven by one load generator (kvbench) speaking each
system's native protocol — RESP for OpenKache, the PostgreSQL wire protocol,
and the MySQL wire protocol. The database was pinned to CPU cores 0–1 and the
load generator to cores 2–5. The generator used a single connection and sent
one request at a time, so each sample is the full end-to-end latency of one
query with no queueing.
