# OpenKache CLI

`openkache-cli` is a standalone command-line client for Bash scripts,
administration, and interactive cache inspection. It uses the shared Rust
client for QUIC, TLS, retries, application-key derivation, and value
protection.

## Purpose

The CLI provides one-shot commands that compose naturally with Unix pipelines
and an interactive shell that keeps one connection open for multiple commands.
It uses text application keys. Supply a data-protection key to protect values;
when omitted, it uses the unprotected formatted profile while retaining the
same Item ID derivation as other text-key clients.

## Commands

From the public repository root:

```bash
cargo build --release -p openkache-cli
cargo run -p openkache-cli -- --help
```

The default `quic-compio` feature matches the server's default `noq` backend
and is intended for Linux deployments with io_uring. Build the optional
Tokio/Quinn variant for platforms where io_uring is unavailable:

```bash
cargo build --release -p openkache-cli \
  --no-default-features --features quic-quinn
```

The `quic-quinn` variant should be paired with a server built and selected with
its `quic-quinn` backend.

The resulting binary is `openkache-cli`. Install it into Cargo's binary
directory with:

```bash
cargo install --path clients/cli
```

## Usage

```bash
openkache-cli ping
openkache-cli get greeting
openkache-cli set greeting "hello OpenKache"
printf 'binary value' | openkache-cli set payload --value-stdin
openkache-cli get payload --output base64
openkache-cli delete greeting
openkache-cli stats
openkache-cli sync
openkache-cli shell
```

`get --output raw` writes exact stored bytes without a newline. `text` is the
default and replaces invalid UTF-8 bytes lossily; `base64` is safe for
binary values in shell pipelines.

When attached to a terminal, `stats` renders a readable table and connection
or durability waits show a spinner on stderr. Piped `stats` output remains
plain JSON, while the other commands keep their existing plain or raw stdout
contracts, so scripts do not receive terminal control sequences. `shell` uses
an editable prompt with history and Tab completion; set `NO_COLOR=1` to
disable terminal styling.

Set conditions and expiration are available for one-shot writes:

```bash
openkache-cli set lease value --if-absent --ttl-ms 5000
openkache-cli set lease value --if-present
```

## Configuration

The data-protection key is optional. Supply it through an environment variable
or a file so secrets do not appear in the process list when protection is
wanted:

```bash
export OPENKACHE_DATA_PROTECTION_KEY='base64-encoded-32-byte-key'
export OPENKACHE_ADDRESS='cache.example.com:4433'
openkache-cli get greeting
```

Available environment variables are `OPENKACHE_ADDRESS`,
`OPENKACHE_SERVER_NAME`, `OPENKACHE_CERTIFICATE`,
`OPENKACHE_CLIENT_CERTIFICATE`, `OPENKACHE_CLIENT_KEY`,
`OPENKACHE_DATA_PROTECTION_KEY`, and
`OPENKACHE_DATA_PROTECTION_KEY_FILE`. Command-line options take precedence
over environment variables. `--certificate` accepts one DER certificate or a
PEM certificate chain; when it is omitted, the operating system trust store is
used.

Use `--client-certificate` and `--client-key` (or their environment variables)
when the server requires mutual TLS. The certificate file may contain the leaf
and intermediate chain.

For a local server whose certificate name is `localhost`:

```bash
openkache-cli \
  --address 127.0.0.1:4433 \
  --server-name localhost \
  --certificate target/openkache-local/certificate.local.der \
  --data-protection-key "$OPENKACHE_DATA_PROTECTION_KEY" \
  get greeting
```

## Components

- `src/main.rs` starts the selected async runtime and reports process-level errors.
- `src/lib.rs` owns argument parsing, connection configuration, one-shot
  operations, value output, and the interactive command loop.
- `openkache-client` supplies the shared QUIC and value-protection behavior.

The CLI speaks OpenKache protocol v1 over QUIC. It is not a Redis RESP client
and cannot be used with `redis-cli`.
