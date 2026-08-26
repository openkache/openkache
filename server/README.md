# `openkache-server`

OpenKache is a super-fast, open-source SSD cache server. `openkache-server`
is its Linux server binary: one process exposes a Redis-compatible RESP/TCP
endpoint and the native OpenKache/QUIC endpoint.

[crates.io](https://crates.io/crates/openkache-server) ·
[docs.rs](https://docs.rs/openkache-server) ·
[GitHub](https://github.com/openkache/openkache/tree/main/server)

This is a preview release for local development and evaluation. It creates a
16 GiB `openkache.data` file and truncates it when the server starts. It does
not provide restart recovery, client authentication, or production durability
guarantees.

## Install

Install the published binary with Cargo:

```bash
# Compile and install from crates.io
cargo install openkache-server

# Or use cargo-binstall if it is already part of your toolchain
cargo binstall openkache-server
```

To build from a checkout instead:

```bash
git clone https://github.com/openkache/openkache.git
cd openkache
cargo install --path server --locked
```

The published archive contains the protocol code required by the server, so
registry builds do not need the OpenKache repository, Bun, or the Smithy CLI.

## Usage

Run the server on the default local endpoint:

```bash
openkache-server
# openkache-server listening on 127.0.0.1:4433 over RESP/TCP and native QUIC/UDP
```

The default configuration uses network CPU `0` and storage CPU `1`. Choose a
different address and CPU pair with positional arguments:

```bash
openkache-server 0.0.0.0:4433 2 3
```

The server uses the same numeric address for TCP and UDP. The native endpoint
uses TLS 1.3 with an ephemeral self-signed certificate for local development;
use the Rust, Python, or TypeScript client examples against a local server only.

## Reference

The command syntax is:

```text
openkache-server [address] [network-cpu storage-cpu]
```

`address` defaults to `127.0.0.1:4433`. `network-cpu` defaults to `0`, and
`storage-cpu` defaults to `1`. The two CPU values must be different.

The RESP/TCP endpoint currently supports:

- `GET key`
- `SET key value`
- `DEL key`

The native OpenKache/QUIC endpoint currently supports `PING`, `GET`, `SET`, and
`DELETE`. TTL overrides, conditional writes, namespace administration,
statistics, synchronization, clustering, and restart recovery are not
implemented by this preview.

## Requirements

- Linux with `io_uring`
- two distinct CPUs available to the process
- a Rust toolchain and native C linker when installing from source

The server is intended for Linux `x86_64` and `aarch64` hosts. See the
[repository README](https://github.com/openkache/openkache) for the container
image and deployment notes.
