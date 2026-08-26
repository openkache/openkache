# FAQ

## General

### What is OpenKache?

OpenKache is a high-performance cache server designed from the ground up for
modern SSDs. The current Linux preview exposes Redis-compatible `GET`, `SET`,
and `DEL` over RESP/TCP and the Gate 0 `PING`, `GET`, `SET`, and `DELETE`
operations over OpenKache/QUIC.

### How is it different from Redis?

The current prototype uses a fixed 16 GiB file as its primary value store and
keeps a lookup table in memory. It also exposes a QUIC endpoint for OpenKache
clients. It is an implementation preview, not a production-ready or
benchmark-backed Redis replacement.

### Is it production-ready?

No. The server truncates its cache file at startup, uses an ephemeral
self-signed certificate, and has no client authentication. Restart recovery,
clustering, namespace administration, and the broader target API remain
unimplemented.

## Runtime

### What does the server require?

The server requires Linux with `io_uring` and two distinct CPUs. One thread is
pinned to the network CPU and one to the storage CPU.

### Which ports and protocols does it use?

TCP and UDP share the same numeric address, `127.0.0.1:4433` by default. TCP
accepts RESP while UDP accepts the OpenKache Gate 0 protocol over QUIC.

### Is data persistent?

No. The server creates `openkache.data` in its working directory, fixes it at
16 GiB, and truncates it every time the process starts.

### Is QUIC certificate verification enabled?

Not in the current Gate 0 development SDK. The connection uses TLS 1.3 over
QUIC, but the SDK accepts the server's ephemeral self-signed certificate. Do
not expose this preview as a trusted production service.

## Clients and contracts

### What client languages are available?

See [the client status](../clients/README.md). Several packages are available
or scaffolded, but the current server implements only the Gate 0 subset listed
above.

### Why do some design documents describe more features?

The protocol, client-format, security, and storage design documents are target
contracts. Their implementations may temporarily lag during the migration.

### Where do I start?

Follow [Getting Started](getting-started.md) or read the
[server README](../server/README.md).
