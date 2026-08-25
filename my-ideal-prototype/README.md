# RESP-backed native client prototype

This standalone prototype exposes one numeric port through two transports:

- TCP accepts the existing RESP `GET` and `SET` commands.
- UDP accepts the OpenKache `openkache/1` protocol over QUIC and translates
  native requests into RESP calls to the TCP listener.

It exists so the maintained Rust, TypeScript, and Python clients can exercise
the native protocol against the cache prototype without changing its RESP
network path.

## Commands

Run these commands from `my-ideal-prototype/` on Linux. The network and storage
CPU numbers must be different and available to the process.

```bash
cargo build --locked --release
cargo test --locked
cargo run --locked --release -- 127.0.0.1:4433 0 1
```

Clients use the same address regardless of language:

```text
127.0.0.1:4433
```

RESP clients connect over TCP. Maintained OpenKache clients connect over
QUIC/UDP automatically; no separate proxy address or process is required.
See the sibling client READMEs under `../clients/` for language-specific setup.

## Request flow

```text
Rust / TypeScript / Python client
              │ openkache/1 over QUIC/UDP
              ▼
       src/resp_proxy/
              │ RESP over loopback TCP
              ▼
      existing RESP listener
              │
              ▼
 temporary in-memory compatibility storage
```

`src/main.rs` owns both sockets and starts the QUIC frontend.
`src/resp_proxy/quic.rs` handles TLS, ALPN, streams, and native frames.
`src/resp_proxy/mapping.rs` maps native operations to the temporary contract.
`src/resp_proxy/resp_backend.rs` sends RESP commands to the TCP listener.

## Configuration and limitations

The executable accepts only `[address] [network-cpu storage-cpu]`; it has no
environment-variable or configuration-file interface.

This is a development-only compatibility path:

- the QUIC frontend creates an ephemeral self-signed certificate, matching the
  maintained clients' development trust profile;
- native `PING`, namespace open, `GET`, `SET`, and `DELETE` are available;
- namespace ID and revision are temporarily fixed to `1`;
- native `DELETE` uses process-local tombstones because the RESP prototype has
  no delete command;
- conditional writes, TTL overrides, policy updates, statistics, and sync are
  unsupported;
- data and tombstones are volatile and disappear when the process exits.

Do not expose this prototype to untrusted networks or use it for production
credentials.
