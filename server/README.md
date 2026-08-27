# OpenKache server

The current OpenKache server is an SSD-backed cache preview for Linux and
Apple Silicon macOS. One process exposes Redis-compatible `GET`, `SET`, and
`DEL` over RESP/TCP and the same operations through the OpenKache Gate 0
protocol over QUIC/UDP.

The preview is intended for development and performance work. It generates an
ephemeral self-signed certificate, does not authenticate clients, and recreates
its cache file on every start.

## Requirements

- Linux with `io_uring`, or Apple Silicon macOS
- Linux: two distinct CPU IDs available to the process; macOS: thread
  placement is delegated to the scheduler
- Rust and the C toolchain required by the workspace dependencies when building
  from source

Prebuilt archives for Linux x86_64, Linux aarch64, and Apple Silicon macOS are
listed in the [repository README](../README.md#download-server-binaries).

## Commands

Build the server from the repository root:

```bash
cargo server-build
```

For a release archive, extract it and run the included `openkache-server`
executable directly. Linux archives are static musl binaries; the macOS
archive is an arm64 Mach-O binary:

Linux:

```bash
./openkache-server 127.0.0.1:4433 0 1
```

macOS:

```bash
./openkache-server 127.0.0.1:4433
```

Run it on the default address. Linux pins the network and storage workers to
CPU 0 and CPU 1; macOS delegates placement to the scheduler:

```bash
cargo run --locked --package openkache-server --bin openkache-server
```

On Linux, select a different address and CPU pair with positional arguments:

```bash
cargo run --locked --package openkache-server --bin openkache-server -- \
  0.0.0.0:4433 2 3
```

On macOS, select a different address without CPU arguments:

```bash
cargo run --locked --package openkache-server --bin openkache-server -- \
  0.0.0.0:4433
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

Linux uses the `io_uring` network frontend and direct-I/O storage path. Apple
Silicon macOS uses Tokio's native polling frontend and buffered file I/O; this
fallback keeps the protocol contract but is not a Linux throughput comparison.

The server creates `openkache.data` in its current working directory. The file
is fixed at 16 GiB and is truncated on startup, so this preview does not provide
restart recovery.

## Components

- `network.rs`: platform dispatcher for RESP/TCP handling
- `network_linux.rs`: Linux `io_uring` frontend
- `network_macos.rs`: Apple Silicon Tokio polling frontend
- `resp_proxy/`: OpenKache/QUIC-to-RESP compatibility frontend
- `storage.rs`: SSD segment-group and lookup runtime
- `spsc.rs`: queues between the network and storage threads
