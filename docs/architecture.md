# OpenKache architecture

OpenKache is an SSD-first cache server. Instead of treating the SSD as a swap
target behind an in-memory index, it makes the SSD the primary capacity tier and
keeps only a compact index in RAM. The design goal is simple: keep every core
busy with useful work and let the SSD run at its sequential bandwidth.

<div align="center">

<img src="./assets/openkache-architecture.png" alt="OpenKache Architecture"/>

</div>

## Thread-per-core, shared-nothing

A conventional server hands requests to a thread pool whose threads migrate
across cores. Every migration has a cost: lock contention, mutex ownership
transfer, context switches, and cache lines bouncing between cores, each of
which forces synchronization and copying. Under load, that coordination
overhead — not the actual work — becomes the bottleneck.

OpenKache pins each worker to a single core. A worker owns its data and shares
no mutable state with other workers, so there are no locks on the hot path. This
shared-nothing, thread-per-core model is the same approach TigerBeetle,
ScyllaDB, and Redis adopted to extract maximum performance from modern hardware.

Redis executes commands on a single core. OpenKache keeps the shared-nothing
principle but shards work across cores, so throughput scales with the hardware
rather than stopping at a single-core ceiling. Because workers share no locks,
adding a core adds no contention.

## Network and storage as separate workers

The two hot paths run on their own cores:

- The **network worker** accepts connections, parses the wire protocol, and
  turns each request into a storage message.
- The **storage worker** owns the SSD data file and the in-RAM key index, and
  services lookups and writes.

They communicate over a single **lock-free SPSC (single-producer,
single-consumer) queue** in each direction. There is no shared mutable state
between them and no lock on the request path, so protocol parsing on the network
core never blocks disk I/O on the storage core, and vice versa.

## Storage: keys in RAM, values on SSD

Values live on the SSD; only a compact index lives in RAM, mapping a compressed
key to the segment offset where its value is stored. This keeps the memory
footprint proportional to the number of keys, not the size of the data, so the
working set can far exceed available RAM.

### Segment-group write aggregation

Writing one key at a time to an SSD wastes the drive: small random writes run
far below its sequential ceiling. OpenKache instead batches writes from many
keys into a single sequential **segment-group** flush — like a subway moving
many riders in one trip rather than each in a separate car. The SSD sees large
sequential writes and runs near its bandwidth limit.

On Linux, the storage worker submits this I/O through **`io_uring`** with direct
I/O, which removes per-operation system-call overhead and bypasses the kernel
page cache so the cache manages its own memory.

## Dual protocol on one address

The server exposes the same numeric address over two transports:

- **RESP over TCP** — Redis-compatible `GET`/`SET`/`DEL`, so existing Redis
  tooling and clients work unchanged.
- **OpenKache Gate 0 over QUIC/UDP** — the native protocol, running TLS 1.3 over
  QUIC. A compatibility frontend bridges QUIC to the same storage engine.

## Why Rust

The entire server is written in Rust. There is no garbage collector, so no GC
pause can interrupt the fast path. The ownership and borrow model rules out data
races at compile time — which matters most precisely because the design leans on
core-local ownership and lock-free queues. And Rust keeps C-level control over
memory layout and system calls, so the hardware-focused design above is
expressible without giving up safety.

## Platform support

Linux is the primary platform: the `io_uring` network frontend and the
direct-I/O storage path depend on it, and that is where peak throughput lives.
Portable macOS and Windows/WSL builds are on the [roadmap](../ROADMAP.md),
targeting developer ergonomics and correctness rather than peak throughput.

## Related documents

- [ROADMAP.md](../ROADMAP.md) — where this design is headed
- [server/README.md](../server/README.md) — the server implementation and
  operation subset
- [protocol/README.md](../protocol/README.md) — the wire protocol and generated
  contract
