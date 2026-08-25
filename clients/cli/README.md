# OpenKache CLI

`openkache-cli` is a standalone command-line client for Bash scripts,
administration, and interactive cache inspection. It uses the shared Rust
client engine for QUIC, TLS, key derivation, value codecs, retries, and
terminal-safe output.

## Purpose

The CLI provides one-shot commands that compose naturally with Unix pipelines
and an interactive shell that keeps one connection open for multiple commands.
It accepts text application keys and supports text values, exact bytes from
stdin, and Base64 or raw output.

The default `gate0` profile is deliberately the same profile as the
maintained Rust client:

- `NamespaceHash` Item IDs with the public Gate 0 root and the server-assigned
  default namespace `1` (resolved lazily, like the Rust client);
- `StructuredValue-CBOR-v1` values, uncompressed and unprotected inside TLS;
- `openkache/1` over QUIC with the local development TLS trust policy.

That fixed profile is also the native QUIC contract exposed by
`my-ideal-prototype`, whose UDP frontend forwards requests to its RESP
storage path. A CLI `set` can therefore be read by the Rust client, and vice
versa, when both use the default profile.

The `configured` profile retains the compatibility path for existing CLI
data: text keys use the legacy byte-key mapping and values use the raw-value
codec. Select it when preserving that data matters or when a server requires
custom TLS, value protection, conditional writes, or expiration.

## Commands

From the public repository root:

```bash
cargo build --release -p openkache-cli
cargo run -p openkache-cli -- --help
```

The default `quic-compio` feature is intended for Linux deployments with
io_uring. Build the optional Tokio/Quinn variant on platforms where io_uring is
unavailable:

```bash
cargo build --release -p openkache-cli \
  --no-default-features --features quic-quinn
```

The `quic-quinn` variant should be paired with a server built and selected with
its `quic-quinn` backend. Check formatting before publishing a build:

```bash
cargo fmt --check -p openkache-cli
```

The resulting binary is `openkache-cli`. Install it into Cargo's binary
directory with:

```bash
cargo install --path clients/cli
```

## Usage

```bash
openkache-cli ping
openkache-cli set greeting "hello OpenKache"
openkache-cli get greeting
printf 'binary value' | openkache-cli set payload --value-stdin
openkache-cli get payload --output base64
openkache-cli delete greeting
openkache-cli shell
```

Normal `VALUE` arguments become structured text values. `--value-stdin`
preserves the input as a structured byte value, including embedded NULs and
newlines. `get --output raw` writes the logical text/byte value without a
trailing newline; `text` is the default and replaces invalid UTF-8 lossily;
`base64` is safe for shell pipelines. Structured values written by another
client that are neither text nor bytes are emitted as canonical CBOR bytes.

`experimental_stats` and `experimental_sync` remain available for a full
OpenKache server when `enable_experimental_api = true` and the exact revision
`draft-2026-08-19.4` have been coordinated out of band. The prototype does
not implement these operations. They are not part of the stable-v1 data
operation set.

The Gate 0 profile supports unconditional writes only. Use the configurable
profile for the compatibility-only conditional and expiration options:

```bash
openkache-cli --profile configured set lease value --if-absent --ttl-ms 5000
openkache-cli --profile configured set lease value --if-present
```

When attached to a terminal, statistics render as a readable table and
maintenance waits show a spinner on stderr. Piped statistics remain plain
JSON. `shell` uses an editable prompt with history and Tab completion; set
`NO_COLOR=1` to disable terminal styling.

## Prototype quick start

Start the prototype with a port shared by its RESP/TCP and native QUIC/UDP
frontends, then use the default CLI profile:

```bash
cargo run --manifest-path my-ideal-prototype/Cargo.toml -- \
  127.0.0.1:4433 0 1
openkache-cli --address 127.0.0.1:4433 set greeting "from cli"
openkache-cli --address 127.0.0.1:4433 get greeting
```

The prototype creates a self-signed `localhost` certificate, so the default
Gate 0 profile intentionally does not authenticate that certificate. This is
for local development only; do not use the Gate 0 trust policy for production
traffic.

## Configuration

Use `--profile configured` for production trust roots, mutual TLS, client-side
value protection, legacy CLI data, or the extended write options. Its
data-protection key may come from an environment variable or a file so secrets
do not appear in the process list:

```bash
export OPENKACHE_PROFILE=configured
export OPENKACHE_DATA_PROTECTION_KEY='base64-encoded-32-byte-key'
export OPENKACHE_ADDRESS='cache.example.com:4433'
openkache-cli get greeting
```

Available environment variables are `OPENKACHE_PROFILE`,
`OPENKACHE_ADDRESS`, `OPENKACHE_SERVER_NAME`, `OPENKACHE_CERTIFICATE`,
`OPENKACHE_CLIENT_CERTIFICATE`, `OPENKACHE_CLIENT_KEY`,
`OPENKACHE_DATA_PROTECTION_KEY`, and
`OPENKACHE_DATA_PROTECTION_KEY_FILE`. Command-line options take precedence
over environment variables. `--certificate` accepts one DER certificate or a
PEM certificate chain; configured mode uses the operating-system trust store
when it is omitted.

Use `--client-certificate` and `--client-key` (or their environment variables)
when the server requires mutual TLS. The certificate file may contain the leaf
and intermediate chain.

For a local server whose certificate name is `localhost`:

```bash
openkache-cli --profile configured \
  --address 127.0.0.1:4433 \
  --server-name localhost \
  --certificate target/openkache-local/certificate.local.der \
  get greeting
```

## Components

- `src/main.rs` starts the selected async runtime and reports process-level errors.
- `src/lib.rs` owns argument parsing, profiles, connection configuration,
  one-shot operations, value conversion/output, and the interactive command loop.
- `openkache-client-core` supplies the shared QUIC, key, value, and TLS
  behavior as an internal implementation crate.

The CLI speaks OpenKache protocol v1 over QUIC. It is not a Redis RESP client
and cannot be used with `redis-cli`; only the prototype's native QUIC frontend
is compatible with this binary.
