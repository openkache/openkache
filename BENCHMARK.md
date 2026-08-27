# OpenKache Benchmark

This document defines the public benchmark environment and comparison methodology for OpenKache. Benchmark results and graphs will be updated here as measurements are finalized.

## Environment

- Host: ServerOptima KVM VPS (AMD EPYC 7763 Plan 2)
- CPU / Memory: 6 vCPU / 20 GB
- Storage: 300 GB NVMe SSD RAID 10
- OS: Ubuntu 24.04 LTS
- Network path: loopback (`127.0.0.1`)

Loopback is used to remove external network variance and focus the comparison on request processing, memory use, and storage behavior.

## Systems

- OpenKache
- Redis
- Dragonfly
- KeyDB
- Apache Kvrocks
- PostgreSQL
- MySQL
- SQLite
- DuckDB

Redis-compatible systems are exercised through RESP. Other systems use an equivalent benchmark adapter with the same logical key-value operations and workload parameters where applicable.

## Workloads

Comparisons keep the following parameters aligned as closely as each system allows:

- key size
- value size
- dataset size
- GET / SET ratio
- concurrency
- warm-up and measurement duration

## Metrics

- throughput (operations/sec)
- average latency
- p99 latency
- memory usage
- storage write volume

## Results

Benchmark tables and graphs will be added here as the final measurement set is completed.
