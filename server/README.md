# `openkache-server`

OpenKache is a super-fast, open-source SSD cache server. `openkache-server`
is the server binary for Linux and Apple Silicon macOS, exposing RESP/TCP and
the native OpenKache/QUIC endpoint.

[crates.io](https://crates.io/crates/openkache-server) ·
[docs.rs](https://docs.rs/openkache-server) ·
[GitHub](https://github.com/openkache/openkache/tree/main/server) ·
[Releases](https://github.com/openkache/openkache/releases)

This preview is intended for local development and evaluation. It creates an
`openkache.data` file and truncates it at startup, uses an ephemeral
self-signed certificate, and does not authenticate clients or provide restart
recovery.

## Install

Install the crate from crates.io, or use `cargo-binstall` when it is available:

```bash
# Build and install from crates.io
cargo install --locked openkache-server

# Install with cargo-binstall
cargo binstall --locked openkache-server
```

To build from a checkout instead:

```bash
git clone https://github.com/openkache/openkache.git
cd openkache
cargo install --path server --locked
```

Prebuilt Linux and Apple Silicon macOS archives are available from
[GitHub Releases](https://github.com/openkache/openkache/releases).

## Quick start

Run the server on its default local endpoint:

```bash
openkache-server
# listening on 127.0.0.1:4433 over RESP/TCP and native QUIC/UDP
```

On Linux, choose a different address and network/storage CPU pair:

```bash
openkache-server 0.0.0.0:4433 2 3
```

On Apple Silicon macOS, thread placement is delegated to the scheduler, so
only the address is accepted:

```bash
openkache-server 0.0.0.0:4433
```

Pass a TOML configuration file with `--config`:

```bash
openkache-server --config openkache.toml
```

The configuration file can set `table_max_entries`, `sg_size_mib`,
`storage_sg_count`, `storage_file_path`, `io_queue_entries`, and
`preallocate_file`. Omitted values use the development defaults.

## Reference

### Command line

```text
# Linux
openkache-server [--config <path>] [address] [network-cpu storage-cpu]

# Apple Silicon macOS
openkache-server [--config <path>] [address]
```

- `address` defaults to `127.0.0.1:4433`; TCP and UDP use the same address.
- Linux CPU arguments default to `0` and `1` and must be different.
- `--config <path>` loads the optional TOML storage configuration.

### Operations

The RESP/TCP endpoint supports:

- `GET key`
- `SET key value`
- `DEL key`

The native OpenKache/QUIC endpoint supports `PING`, `GET`, `SET`, and
`DELETE`. TTL overrides, conditional writes, namespace administration,
statistics, synchronization, clustering, and restart recovery are not
implemented by this preview.

The native endpoint uses TLS 1.3 with a certificate generated at startup.
Linux uses `io_uring` and direct I/O; Apple Silicon macOS uses a portable
polling and buffered-I/O path.
