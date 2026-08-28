# OpenKache Benchmark

## Throughput (GET)

| System | GET throughput | Load tool |
|---|---:|---|
| OpenKache | 97,887 ops/s | kvbench (RESP) |
| PostgreSQL 17.10 | 17,421 ops/s | kvbench (PostgreSQL wire) |
| MySQL 8.4.11 | 16,295 ops/s | kvbench (MySQL wire) |

OpenKache is 5.6× faster than PostgreSQL and 6.0× faster than MySQL. On the
same machine, the single-core fio 4 KiB random-read ceiling is 128,820 IOPS,
and OpenKache reaches 76% of that ceiling with a single storage core.

## Latency (GET, single request at a time)

| System | avg | p50 | p99 | p99.9 |
|---|---:|---:|---:|---:|
| OpenKache | 238.7 µs | 229 µs | 386 µs | 1376 µs |
| MySQL 8.4.11 | 385.7 µs | 410 µs | 1169 µs | 2207 µs |
| PostgreSQL 17.10 | 558.0 µs | 510 µs | 1263 µs | 3342 µs |

OpenKache's average GET latency is 1.6× lower than MySQL and 2.3× lower than
PostgreSQL; at p99 it is 3.0× and 3.3× lower.

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

All three systems were driven by kvbench, a custom load generator that speaks
each system's native protocol: RESP for OpenKache, the PostgreSQL wire protocol
for PostgreSQL, and the MySQL wire protocol for MySQL. Using a single tool
across all three systems keeps the client-side code path, key distribution, and
measurement logic identical — only the wire protocol changes per backend. In
every run the database was pinned to CPU cores 0–1 and the load generator to
cores 2–5. kvbench source is under `bench/kvbench/`.

For PostgreSQL and MySQL, server parameters were swept to find the configuration
with the highest throughput.

PostgreSQL: swept shared_buffers over 128 / 256 / 512 MB and client count over
16 / 24 / 32 / 48, selecting shared_buffers 256 MB with 24 clients. Prepared
statements were used, jit and autovacuum were disabled, and ANALYZE was run
right after prefill.

MySQL: swept innodb_buffer_pool_size over 128 / 256 / 512 MB and threads over
8 / 16 / 32 / 64, selecting a 512 MB buffer pool with 16 threads. Prepared
statements were used.

### Latency

All three systems were driven by kvbench speaking each system's native protocol
— RESP for OpenKache, the PostgreSQL wire protocol for PostgreSQL, and the
MySQL wire protocol for MySQL. The database was pinned to CPU cores 0–1 and the
load generator to cores 2–5. The generator used a single connection and sent one
request at a time, so each sample is the full end-to-end latency of one query
with no queueing. kvbench source is under `bench/kvbench/`.
