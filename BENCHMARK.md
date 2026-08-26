# OpenKache SSD GET-Throughput Benchmark

## Summary

| System | GET throughput | Load tool |
|---|---:|---|
| OpenKache | 97,887 ops/s | memtier (RESP) |
| PostgreSQL 17.10 | 17,421 ops/s | pgbench |
| MySQL 8.4.11 | 16,295 ops/s | sysbench |

OpenKache is 5.6× faster than PostgreSQL and 6.0× faster than MySQL. On the
same machine, the single-core fio 4 KiB random-read ceiling is 128,820 IOPS,
and OpenKache reaches 76% of that ceiling with a single storage core.

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

## Measurement Method

memtier speaks only the RESP protocol, so measuring every system with memtier
would be unfair to the SQL databases. Each system was therefore driven by a
tool that speaks its own native protocol — memtier (RESP) for OpenKache,
pgbench for PostgreSQL, and sysbench for MySQL. In every run the database was
pinned to CPU cores 0–1 and the load generator to cores 2–5.

For PostgreSQL and MySQL, parameters were swept to find the configuration with
the highest throughput.

PostgreSQL: swept shared_buffers over 128 / 256 / 512 MB and client count over
16 / 24 / 32 / 48, selecting shared_buffers 256 MB with 24 clients. Prepared
statements (`-M prepared`) were used, jit and autovacuum were disabled, and
ANALYZE was run right after prefill.

MySQL: swept innodb_buffer_pool_size over 128 / 256 / 512 MB and threads over
8 / 16 / 32 / 64, selecting a 512 MB buffer pool with 16 threads. Prepared
statements were used.
