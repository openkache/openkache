# OpenKache server

`openkache-server` is the production cache-server package for OpenKache. It
stores cache data on SSD-backed shards and exposes the versioned OpenKache
protocol over QUIC and TLS-over-TCP. Every connection on those production
profiles uses TLS 1.3 and the required `X25519MLKEM768` hybrid key exchange;
this package change adds no plaintext transport. The existing loopback-only
RESP development profile remains separate from the production protocol.

The package publishes the `openkache-server` binary and keeps the established
`openkache` library crate name for applications that embed the server runtime.
The package build uses the checked-in operation-contract snapshot when the
repository-level protocol generator is unavailable, so `cargo install`,
docs.rs, and other isolated builds do not require Bun or Smithy.

## Usage

Build or install the server:

```bash
# From an OpenKache checkout
cargo build --manifest-path server/Cargo.toml --bin openkache-server

# From crates.io
cargo install openkache-server
```

Start a local server:

```bash
cargo run --manifest-path server/Cargo.toml --bin openkache-server
```

The default loopback endpoint is `127.0.0.1:4433`. It creates an ephemeral
server certificate and writes its DER form to
`target/openkache-local/certificate.local.der`. The connection remains
encrypted with TLS 1.3, but local development does not require a client
certificate or a trusted server certificate.

### Certificate-free client-authentication development

The following is an explicitly labeled development mode. It omits client
certificates while retaining TLS 1.3 encryption and the mandatory hybrid key
exchange:

```bash
cargo run --manifest-path server/Cargo.toml --bin openkache-server -- \
  --insecure-development \
  --listen 127.0.0.1:4433
```

`--insecure-development` is required before binding this mode to a
non-loopback address. It disables peer authentication and grants administrative
operations to every connected peer, so use it only on an isolated development
network. It never enables plaintext sockets.

For a deployable endpoint, provide a server certificate and private key. Client
certificate authentication is optional for ordinary operations:

```toml
[tls]
certificate_chain = "/etc/openkache/tls/server-chain.pem"
private_key = "/etc/openkache/tls/server-key.pem"

# Optional mTLS and administrative authorization:
client_ca = "/etc/openkache/tls/client-ca-bundle.pem"
admin_client_certificates = [
  "/etc/openkache/tls/operators/admin-2026.pem",
]
```

When `client_ca` is omitted, the server still requires its own certificate and
key but does not request client certificates. Supplying `client_ca` enables
mTLS; administrative operations additionally require the client's exact leaf
certificate to appear in `admin_client_certificates`. A configured administrator
allowlist must have a matching `client_ca`.

The `pki` subcommands can create a small internal CA and a deployable mTLS
bundle without OpenSSL:

```bash
openkache-server pki init
openkache-server pki issue-server --dns cache.example.com --ip 10.0.0.10
openkache-server pki issue-client application-01
openkache-server pki issue-admin operator-01
openkache-server --pki-directory /etc/openkache/pki
```

## Configuration

Pass `--config <path>` to load a TOML cache configuration. Without a file, the
server derives worker, memory, and storage sizing from process limits and the
selected storage directory. `--cpus`, `--memory-gib`, `--storage-gb`,
`--directory`, and `--plan` provide explicit sizing or a plan-only preview.

The QUIC backend is selected by the compiled feature set or
`--quic-backend`. The TLS-over-TCP listener reuses the QUIC address unless
`[tcp].listen` sets another address. Both profiles share the same TLS 1.3
security boundary and application protocol.

## Components

- `src/lib.rs` exposes the server library and compile-time runtime selections.
- `src/server/` owns lifecycle, authorization, namespace control, and worker
  composition.
- `src/transport/` adapts the QUIC backends and TLS-over-TCP lanes.
- `src/bin/openkache_server.rs` contains the `openkache-server` CLI, sizing,
  diagnostics, and development PKI commands.
- `src/contract_snapshot/` stores the generated server-visible wire contract
  used by isolated package builds; checkout builds regenerate it from the
  canonical protocol model.

## Verification

The public package intentionally contains production code only. Run the
repository's private validation before publishing changes:

```bash
cargo fmt --check
cargo check --manifest-path server/Cargo.toml --all-features
cargo test --manifest-path server/Cargo.toml --all-features
```
