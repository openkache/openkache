# OpenKache Rust Client

`openkache-client` is the production QUIC client shared by native Rust callers
and the TypeScript SDK. It owns the transport, binary framing, transparent
Zstandard compression, and XChaCha20-Poly1305 value encryption.

## Purpose

Keeping value transformation and protocol logic in one crate prevents
language-specific SDKs from drifting. Secure values are compressed when
beneficial, encrypted in place, and authenticated against their cache-key
digest before transmission. The server stores these bytes without parsing or
decompressing them.

## Commands

From `clients/rust`:

```bash
cargo build
cargo check
cargo fmt --check
```

Build the helper process used by TypeScript with:

```bash
cargo build --bin openkache-client-helper --release
```

## Usage

```rust
use openkache_client::value::{Compression, ValueCodec, ZstandardOptions};
use openkache_client::{Client, ClientOptions};

let client = Client::connect_with_options(
    "127.0.0.1:4433".parse()?,
    "localhost",
    &certificate_der,
    ClientOptions {
        value_codec: ValueCodec::encrypted(
            encryption_key,
            Compression::Zstandard(ZstandardOptions::default()),
        )?,
        ..ClientOptions::default()
    },
)
.await?;

client.set(b"greeting", b"hello").await?;
let value = client.get(b"greeting").await?;
```

`Client::connect` remains available for unwrapped plaintext values. Prefer
`connect_with_options` with an application-managed 32-byte encryption key when
the server must not observe value plaintext.

Production servers also require `ClientOptions.identity` containing a
`ClientIdentity` certificate chain and private key. Every CA-authenticated
identity can use data operations, while `STATS` and `SYNC` additionally require
the exact leaf certificate to appear in the server administrator allowlist.

One client owns one QUIC connection. Operations reuse a lazily grown pool of
bidirectional stream lanes, with one request in flight per lane and at most 256
lanes per connection.

The transport-independent `value_envelope` module is the reference framing
implementation for cross-language object values:

```rust
use openkache_client::value_envelope;

let stored = value_envelope::encode(
    "protobuf",
    "acme.Profile",
    &encoded_message,
)?;
let decoded = value_envelope::decode(&stored)?;
assert_eq!(decoded.encoding, "protobuf");
assert_eq!(decoded.type_name, "acme.Profile");
```

The server treats these bytes as opaque. Language clients remain responsible
for converting regular language objects with JSON or a registered generated
codec; the envelope carries codec and type identity so `get` and `set` do not
need positional schema arguments.

The default `quic-quinn` feature uses Quinn on Tokio. Call client futures from
an active Tokio runtime. Builds may instead enable `quic-compio`; those futures
require an active Compio runtime. A single-backend build selects that backend
automatically. When both backends are compiled, the client selects the active
Compio or Tokio runtime, or callers can set `QuicOptions.backend` explicitly.
Missing runtime support is returned as `Error::Runtime`.

Connection setup has a 5-second deadline and complete request exchanges have a
2-second deadline by default. Configure both with `ClientTimeouts`. `PING`,
`GET`, and `STATS` retry once after reconnectable transport failures by
default. `SET`, `DELETE`, and `SYNC` are never retried automatically because
the server may already have applied them. Configure safe-operation attempts
with `RetryPolicy`.

## Configuration

`ZstandardOptions` defaults to level 1, skips values below 1 KiB, and requires
at least 64 bytes of savings. The codec uses a fresh 24-byte nonce for every
encrypted value. Clients that share cached values must use the same encryption
key.

The stored representation has no magic, version, fixed original-length field,
or padding:

```text
encrypted: nonce[24] | ciphertext[N] | authentication_tag[16]
```

The existing request and response length fields carry `compressed` and
`encrypted` in their two unused high bits. The server preserves those bits in
unused bits of its existing stored-value tag and never adds them to the value
bytes. Encryption therefore adds exactly 40 bytes. Compression-only values use
the Zstandard frame directly when compression is beneficial and otherwise
remain exact plaintext bytes.

The flags are authenticated with the cache-key digest whenever encryption is
enabled. Clients can distinguish all four plain, compressed, encrypted, and
compressed-encrypted representations without inspecting value contents.

Encoded values are limited to the protocol's 64 MiB wire ceiling. Servers may
configure a smaller operational item limit; the default is 16 MiB.

Owned buffers are reused across value transformation and protocol framing when
possible. Uncompressed decryptions compact in place. Compressed reads allocate
one output buffer from the authenticated Zstandard frame content size after
decryption.
