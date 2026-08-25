# OpenKache server

The current OpenKache server is an SSD-backed cache preview for Linux. One
process exposes Redis-compatible `GET`, `SET`, and `DEL` over RESP/TCP and the
same operations through the OpenKache Gate 0 protocol over QUIC/UDP.

The preview is intended for development and performance work. It generates an
ephemeral self-signed certificate, does not authenticate clients, and recreates
its cache file on every start.

## Requirements

- Linux with `io_uring`
- Two distinct CPUs available to the process
- Rust and the C toolchain required by the workspace dependencies

## Commands

Build the server from the repository root:

```bash
cargo server-build
```

Run it on the default address with the network thread on CPU 0 and the storage
thread on CPU 1:

```bash
cargo run --locked --package openkache-server --bin openkache-server
```

Select a different address and CPU pair with positional arguments:

```bash
cargo run --locked --package openkache-server --bin openkache-server -- \
  0.0.0.0:4433 2 3
```

Verify the crate:

```bash
cargo test --locked --package openkache-server
```

## Runtime behavior

TCP and UDP use the same numeric address. TCP accepts RESP while UDP accepts
the native OpenKache QUIC protocol. The native adapter currently supports
`PING`, `GET`, `SET`, `DELETE`, and the synthetic Gate 0 namespace descriptor.
TTL overrides, conditional writes, namespace administration, statistics, and
sync operations are not implemented.

The server creates `openkache.data` in its current working directory. The file
is fixed at 16 GiB and is truncated on startup, so this preview does not provide
restart recovery.

## Components

- `network.rs`: RESP/TCP connection handling on `io_uring`
- `resp_proxy/`: OpenKache/QUIC-to-RESP compatibility frontend
- `storage.rs`: SSD segment-group and lookup runtime
- `spsc.rs`: queues between the network and storage threads
